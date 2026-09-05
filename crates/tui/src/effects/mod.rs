use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use cleanr_config::Config;
use cleanr_core::{
    AnalysisReport, CandidateId, CleanupPlan, EntryKind, ExecutionManifest, GlobalScanEvidence,
    GlobalScanKind, RecommendationPolicy, RestoreManifest, SafetyPolicy, ScanEntry, ScanRequest,
    UserSelection,
};
use cleanr_fs::{ScanOptions, ScanPhase, ScanProgress, ScanReport};
use cleanr_i18n::I18n;
use cleanr_plugin_api::discover_bundles;
use cleanr_rules::RuleRegistry;
#[cfg(test)]
use cleanr_tasks::{CleanupExecutor, execute_locally_confirmed_plan_with_executor};
use cleanr_tasks::{
    ManifestRepository, ScanPreparationMode as WorkflowPreparationMode, ScanWorkflowError,
    ScanWorkflowInput, ScanWorkflowObserver, ScanWorkflowStage, SystemRestoreExecutor,
    run_scan_workflow, write_cleanup_plan,
};

pub(crate) enum TaskEvent {
    ScanProgress {
        job_id: u64,
        progress: ScanTaskProgress,
    },
    ScanFinished {
        job_id: u64,
        result: std::result::Result<Box<PreparedScan>, ScanFailure>,
        diagnostics: ScanDiagnostics,
    },
}

pub(crate) struct ScanSample {
    pub job_id: u64,
    pub progress: ScanTaskProgress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScanStage {
    Resolving,
    Scanning,
    Aggregating,
    Rules,
    Evidence,
    Plan,
    Usage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanTaskProgress {
    pub stage: ScanStage,
    pub entries_total: usize,
    pub entries_scanned: usize,
    pub bytes_scanned: u64,
    pub errors: usize,
    pub current_path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) enum ScanFailure {
    Cancelled,
    NoGlobalRoots,
    Message(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanPhaseTiming {
    pub stage: ScanStage,
    pub duration: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanDiagnostics {
    pub phases: Vec<ScanPhaseTiming>,
    pub total: Duration,
    pub longest_progress_gap: Duration,
    pub entries_scanned: usize,
    pub bytes_scanned: u64,
    pub errors: usize,
}

pub(crate) struct ScanPreparation {
    pub registry: Arc<RuleRegistry>,
    pub safety_policy: SafetyPolicy,
    pub preselect_after_days: u16,
    pub prepare_usage: bool,
}

pub(crate) struct PreparedScan {
    pub report: ScanReport,
    pub explicit_roots: Vec<PathBuf>,
    pub global_scan: GlobalScanEvidence,
    pub candidate_count: usize,
    pub candidate_entry_indices: Vec<usize>,
    pub usage: Option<UsageProjection>,
    pub planning: std::result::Result<Option<PreparedPlanning>, String>,
    pub index: crate::projection::ScanIndex,
}

pub(crate) struct PreparedPlanning {
    pub analysis: AnalysisReport,
    pub candidate_ids_by_path: HashMap<PathBuf, CandidateId>,
    pub selection: UserSelection,
    pub plan: CleanupPlan,
}

#[derive(Default)]
pub(crate) struct UsageProjection {
    pub order: Vec<usize>,
    pub max_size: u64,
    pub descendant_counts: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Cleanup,
    Restore,
}

pub(crate) enum OperationEvent {
    CleanupFinished(std::result::Result<ExecutionManifest, String>),
    RestoreFinished(std::result::Result<RestoreManifest, String>),
}

pub(crate) struct ScanEffect {
    pub receiver: Receiver<TaskEvent>,
    pub sample_receiver: Receiver<ScanSample>,
    pub cancellation: Arc<AtomicBool>,
}

pub(crate) struct OperationEffect {
    pub kind: OperationKind,
    pub receiver: Receiver<OperationEvent>,
    pub sample_receiver: Receiver<cleanr_tasks::OperationProgress>,
}

#[derive(Debug)]
pub(crate) struct ProjectedScan {
    pub data_revision: u64,
    pub query_revision: u64,
    pub visible: Arc<Vec<usize>>,
}

pub(crate) struct PlanPreparation {
    pub source_revision: u64,
    pub entries: Arc<Vec<ScanEntry>>,
    pub analysis: Arc<AnalysisReport>,
    pub selection: UserSelection,
    pub roots: Vec<PathBuf>,
    pub registry: Arc<RuleRegistry>,
    pub safety: SafetyPolicy,
    pub explicit_roots: Vec<PathBuf>,
    pub global_scan: GlobalScanEvidence,
}

pub(crate) struct PreparedPlan {
    pub source_revision: u64,
    pub result: std::result::Result<(CleanupPlan, crate::projection::ScanIndex), String>,
}

pub(crate) fn spawn_plan(
    input: PlanPreparation,
) -> Result<(Receiver<PreparedPlan>, Arc<AtomicBool>)> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("cleanr-plan".into())
        .spawn(move || {
            let result = cleanr_tasks::build_workflow_plan_cancellable(
                input.roots,
                input.registry.versions(),
                &input.entries,
                &input.analysis,
                &input.selection,
                &input.safety,
                &input.explicit_roots,
                &input.global_scan,
                &|| worker_cancel.load(Ordering::Relaxed),
            )
            .map(|plan| {
                let index = crate::projection::prepare_scan_index(Some(&plan), &input.entries);
                (plan, index)
            })
            .map_err(|error| error.to_string());
            if !worker_cancel.load(Ordering::Relaxed) {
                let _ = sender.send(PreparedPlan {
                    source_revision: input.source_revision,
                    result,
                });
            }
        })
        .context("failed to spawn plan worker")?;
    Ok((receiver, cancel))
}

pub(crate) type HistoryResult =
    std::result::Result<(Vec<ExecutionManifest>, Vec<RestoreManifest>), String>;

pub(crate) fn spawn_history(state_dir: PathBuf) -> Result<Receiver<HistoryResult>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-history".into())
        .spawn(move || {
            let _ = sender.send(load_history(&state_dir).map_err(|error| error.to_string()));
        })
        .context("failed to spawn history worker")?;
    Ok(receiver)
}

pub(crate) fn spawn_projection(
    index: Arc<crate::projection::ScanIndex>,
    query: crate::projection::ScanQuery,
    selected: Arc<Vec<bool>>,
    data_revision: u64,
    query_revision: u64,
) -> Result<(Receiver<ProjectedScan>, Arc<AtomicBool>)> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("cleanr-filter".into())
        .spawn(move || {
            let visible =
                crate::projection::project_scan(&index, &query, &selected, &worker_cancel);
            drop(selected);
            drop(index);
            if let Some(visible) = visible {
                let _ = sender.send(ProjectedScan {
                    data_revision,
                    query_revision,
                    visible,
                });
            }
        })
        .context("failed to spawn candidate projection worker")?;
    Ok((receiver, cancel))
}

pub(crate) fn load_runtime(config: &Config) -> Result<(RuleRegistry, I18n)> {
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    Ok((
        RuleRegistry::load_with_discovery(config, &discovery)?,
        I18n::load_with_discovery(config, &discovery)?,
    ))
}

pub(crate) fn spawn_scan(
    job_id: u64,
    request: ScanRequest,
    configured_global_kinds: Vec<GlobalScanKind>,
    options: ScanOptions,
    preparation: ScanPreparation,
) -> Result<ScanEffect> {
    // Lifecycle events are few and reliable. Path samples use a separate one-item lossy channel,
    // so a sample can neither delay a phase transition nor inflate phase timing with UI backpressure.
    let (sender, receiver) = mpsc::channel();
    let (sample_sender, sample_receiver) = mpsc::sync_channel(1);
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    std::thread::Builder::new()
        .name("cleanr-scan".to_string())
        .spawn(move || {
            let mut progress = ScanProgressRecorder::new(job_id, &sender, &sample_sender);
            let result = run_scan_worker(
                request,
                configured_global_kinds,
                options,
                preparation,
                &worker_cancellation,
                &mut progress,
            )
            .map(Box::new);
            let diagnostics = progress.finish();
            let _ = sender.send(TaskEvent::ScanFinished {
                job_id,
                result,
                diagnostics,
            });
        })
        .context("failed to spawn scan worker")?;
    Ok(ScanEffect {
        receiver,
        sample_receiver,
        cancellation,
    })
}

fn run_scan_worker(
    request: ScanRequest,
    configured_global_kinds: Vec<GlobalScanKind>,
    options: ScanOptions,
    preparation: ScanPreparation,
    cancellation: &AtomicBool,
    progress: &mut ScanProgressRecorder<'_>,
) -> std::result::Result<PreparedScan, ScanFailure> {
    let recommendation_policy = RecommendationPolicy::new(preparation.preselect_after_days)
        .map_err(|error| ScanFailure::Message(error.to_string()))?;
    let prepared = run_scan_workflow(
        ScanWorkflowInput {
            request,
            configured_global_kinds,
            options,
            registry: Arc::clone(&preparation.registry),
            safety_policy: preparation.safety_policy,
            recommendation_policy,
            preparation_mode: WorkflowPreparationMode::Interactive,
        },
        Some(cancellation),
        progress,
    )
    .map_err(workflow_scan_failure)?;

    let planning = match (prepared.analysis, prepared.plan) {
        (Some(analysis), Some(plan)) => {
            let candidate_ids_by_path = analysis
                .candidates
                .iter()
                .map(|candidate| (candidate.local_path.clone(), candidate.id.clone()))
                .collect();
            Ok(Some(PreparedPlanning {
                analysis,
                candidate_ids_by_path,
                selection: prepared.selection,
                plan,
            }))
        }
        (None, None) => Ok(None),
        _ => Err("scan workflow returned incomplete planning state".to_string()),
    };
    let usage = if preparation.prepare_usage {
        progress.emit_stage(ScanStage::Usage);
        ensure_scan_active(cancellation)?;
        Some(build_usage_projection(
            &prepared.report.entries,
            &prepared.report.summary.roots,
        ))
    } else {
        None
    };

    let index = crate::projection::prepare_scan_index(
        planning
            .as_ref()
            .ok()
            .and_then(|p| p.as_ref())
            .map(|p| &p.plan),
        &prepared.report.entries,
    );
    ensure_scan_active(cancellation)?;
    Ok(PreparedScan {
        report: prepared.report,
        explicit_roots: prepared.explicit_roots,
        global_scan: prepared.global_scan,
        candidate_count: prepared.candidate_count,
        candidate_entry_indices: prepared.candidate_entry_indices,
        usage,
        planning,
        index,
    })
}

fn workflow_scan_failure(error: ScanWorkflowError) -> ScanFailure {
    match error {
        ScanWorkflowError::Cancelled => ScanFailure::Cancelled,
        ScanWorkflowError::NoRoots => ScanFailure::NoGlobalRoots,
        ScanWorkflowError::Message(message) => ScanFailure::Message(message),
    }
}

fn ensure_scan_active(cancellation: &AtomicBool) -> std::result::Result<(), ScanFailure> {
    if cancellation.load(Ordering::Relaxed) {
        Err(ScanFailure::Cancelled)
    } else {
        Ok(())
    }
}

struct ScanProgressRecorder<'a> {
    job_id: u64,
    sender: &'a Sender<TaskEvent>,
    sample_sender: &'a SyncSender<ScanSample>,
    started_at: Instant,
    phase_started_at: Instant,
    last_progress_at: Instant,
    stage: ScanStage,
    phases: Vec<ScanPhaseTiming>,
    longest_progress_gap: Duration,
    latest: ScanTaskProgress,
}

impl ScanWorkflowObserver for ScanProgressRecorder<'_> {
    fn stage_changed(&mut self, stage: ScanWorkflowStage) {
        let stage = match stage {
            ScanWorkflowStage::Resolving => ScanStage::Resolving,
            ScanWorkflowStage::Scanning => ScanStage::Scanning,
            ScanWorkflowStage::Rules => ScanStage::Rules,
            ScanWorkflowStage::Evidence => ScanStage::Evidence,
            ScanWorkflowStage::Plan => ScanStage::Plan,
        };
        self.emit_stage(stage);
    }

    fn filesystem_progress(&mut self, progress: &ScanProgress) {
        self.emit_filesystem_progress(progress.clone());
    }
}

impl<'a> ScanProgressRecorder<'a> {
    fn new(
        job_id: u64,
        sender: &'a Sender<TaskEvent>,
        sample_sender: &'a SyncSender<ScanSample>,
    ) -> Self {
        let now = Instant::now();
        Self {
            job_id,
            sender,
            sample_sender,
            started_at: now,
            phase_started_at: now,
            last_progress_at: now,
            stage: ScanStage::Resolving,
            phases: Vec::new(),
            longest_progress_gap: Duration::ZERO,
            latest: ScanTaskProgress {
                stage: ScanStage::Resolving,
                entries_total: 0,
                entries_scanned: 0,
                bytes_scanned: 0,
                errors: 0,
                current_path: None,
            },
        }
    }

    fn emit_stage(&mut self, stage: ScanStage) {
        let mut progress = self.latest.clone();
        progress.stage = stage;
        progress.current_path = None;
        self.observe(&progress);
        let _ = self.sender.send(TaskEvent::ScanProgress {
            job_id: self.job_id,
            progress,
        });
    }

    fn emit_filesystem_progress(&mut self, progress: ScanProgress) {
        let stage = match progress.phase {
            ScanPhase::Discovering | ScanPhase::Scanning => ScanStage::Scanning,
            ScanPhase::Aggregating => ScanStage::Aggregating,
        };
        let progress = ScanTaskProgress {
            stage,
            entries_total: progress.entries_total,
            entries_scanned: progress.entries_scanned,
            bytes_scanned: progress.bytes_scanned,
            errors: progress.errors,
            current_path: progress.current_path,
        };
        let reliable = progress.current_path.is_none();
        self.observe(&progress);
        if reliable {
            let _ = self.sender.send(TaskEvent::ScanProgress {
                job_id: self.job_id,
                progress,
            });
        } else {
            let _ = self.sample_sender.try_send(ScanSample {
                job_id: self.job_id,
                progress,
            });
        }
    }

    fn observe(&mut self, progress: &ScanTaskProgress) {
        let now = Instant::now();
        self.longest_progress_gap = self
            .longest_progress_gap
            .max(now.saturating_duration_since(self.last_progress_at));
        self.last_progress_at = now;
        if progress.stage != self.stage {
            self.phases.push(ScanPhaseTiming {
                stage: self.stage,
                duration: now.saturating_duration_since(self.phase_started_at),
            });
            self.stage = progress.stage;
            self.phase_started_at = now;
        }
        self.latest = progress.clone();
    }

    fn finish(mut self) -> ScanDiagnostics {
        let now = Instant::now();
        self.longest_progress_gap = self
            .longest_progress_gap
            .max(now.saturating_duration_since(self.last_progress_at));
        self.phases.push(ScanPhaseTiming {
            stage: self.stage,
            duration: now.saturating_duration_since(self.phase_started_at),
        });
        ScanDiagnostics {
            phases: self.phases,
            total: now.saturating_duration_since(self.started_at),
            longest_progress_gap: self.longest_progress_gap,
            entries_scanned: self.latest.entries_scanned,
            bytes_scanned: self.latest.bytes_scanned,
            errors: self.latest.errors,
        }
    }
}

pub(crate) fn build_usage_projection(entries: &[ScanEntry], roots: &[PathBuf]) -> UsageProjection {
    let mut order = Vec::new();

    for root in roots {
        let root_path = root.as_path();
        let mut children = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.path.parent() == Some(root_path))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        children.sort_by_key(|index| std::cmp::Reverse(entries[*index].size_bytes));
        order.extend(children);
    }

    if order.is_empty() {
        order.extend(0..entries.len());
        order.sort_by_key(|index| std::cmp::Reverse(entries[*index].size_bytes));
        order.truncate(100);
    }

    let max_size = order
        .iter()
        .map(|index| entries[*index].size_bytes)
        .max()
        .unwrap_or(0);
    // Link each entry only to its immediate scanned parent, then fold leaf totals upward. This is
    // O(N) instead of walking every path ancestor and deliberately stops at gaps in the scan.
    let entry_index_by_path = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.path.as_path(), index))
        .collect::<HashMap<_, _>>();
    let parent_indices = entries
        .iter()
        .map(|entry| {
            entry
                .path
                .parent()
                .and_then(|parent| entry_index_by_path.get(parent).copied())
        })
        .collect::<Vec<_>>();
    let mut pending_children = vec![0usize; entries.len()];
    for parent in parent_indices.iter().flatten() {
        pending_children[*parent] = pending_children[*parent].saturating_add(1);
    }

    let mut descendant_totals = vec![0usize; entries.len()];
    let mut ready = pending_children
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect::<Vec<_>>();
    while let Some(index) = ready.pop() {
        let Some(parent) = parent_indices[index] else {
            continue;
        };
        descendant_totals[parent] =
            descendant_totals[parent].saturating_add(descendant_totals[index].saturating_add(1));
        pending_children[parent] = pending_children[parent].saturating_sub(1);
        if pending_children[parent] == 0 {
            ready.push(parent);
        }
    }

    let descendant_counts = order
        .iter()
        .map(|index| {
            if entries[*index].kind == EntryKind::Directory {
                descendant_totals[*index]
            } else {
                0
            }
        })
        .collect();

    UsageProjection {
        order,
        max_size,
        descendant_counts,
    }
}

pub(crate) fn spawn_usage(
    entries: Arc<Vec<ScanEntry>>,
    roots: Vec<PathBuf>,
) -> Result<Receiver<UsageProjection>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-usage".into())
        .spawn(move || {
            let _ = sender.send(build_usage_projection(&entries, &roots));
        })
        .context("failed to spawn usage worker")?;
    Ok(receiver)
}

pub(crate) fn spawn_cleanup(plan: Arc<CleanupPlan>, state_dir: PathBuf) -> Result<OperationEffect> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let (samples, sample_receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-cleanup".to_string())
        .spawn(move || {
            let executor = cleanr_tasks::TrashExecutor;
            let mut last_sample = None;
            let result = cleanr_tasks::execute_locally_confirmed_plan_with_progress(
                &plan,
                &executor,
                &state_dir,
                &mut |sample| {
                    if last_sample
                        .is_none_or(|at: Instant| at.elapsed() >= Duration::from_millis(100))
                        && samples.try_send(sample).is_ok()
                    {
                        last_sample = Some(Instant::now());
                    }
                },
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(OperationEvent::CleanupFinished(result));
        })
        .context("failed to spawn cleanup worker")?;
    Ok(OperationEffect {
        kind: OperationKind::Cleanup,
        receiver,
        sample_receiver,
    })
}

pub(crate) fn spawn_restore(
    manifest: ExecutionManifest,
    state_dir: PathBuf,
) -> Result<OperationEffect> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let (samples, sample_receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-restore".to_string())
        .spawn(move || {
            let mut last_sample = None;
            let result = cleanr_tasks::restore_execution_manifest_with_progress(
                &manifest,
                &SystemRestoreExecutor,
                &state_dir,
                &mut |sample| {
                    if last_sample
                        .is_none_or(|at: Instant| at.elapsed() >= Duration::from_millis(100))
                        && samples.try_send(sample).is_ok()
                    {
                        last_sample = Some(Instant::now());
                    }
                },
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(OperationEvent::RestoreFinished(result));
        })
        .context("failed to spawn restore worker")?;
    Ok(OperationEffect {
        kind: OperationKind::Restore,
        receiver,
        sample_receiver,
    })
}

pub(crate) fn load_history(
    state_dir: &Path,
) -> Result<(Vec<ExecutionManifest>, Vec<RestoreManifest>)> {
    ManifestRepository::new(state_dir).history()
}

#[cfg(test)]
pub(crate) fn execute_cleanup(
    plan: &CleanupPlan,
    executor: &impl CleanupExecutor,
    state_dir: &Path,
    user_authorized: bool,
) -> Result<ExecutionManifest> {
    if !user_authorized {
        anyhow::bail!("cleanup requires explicit local user authorization");
    }
    execute_locally_confirmed_plan_with_executor(plan, executor, state_dir)
}

pub(crate) fn export_cleanup_plan(plan: &CleanupPlan, path: &Path) -> Result<()> {
    write_cleanup_plan(plan, path)
}

pub(crate) fn save_config(config: &Config, path: &Path) -> Result<()> {
    config.save_to(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_failure_distinguishes_typed_cancellation_from_matching_text() {
        assert!(matches!(
            workflow_scan_failure(ScanWorkflowError::Cancelled),
            ScanFailure::Cancelled
        ));

        let failure = workflow_scan_failure(ScanWorkflowError::Message(
            cleanr_fs::SCAN_CANCELLED.to_string(),
        ));
        assert!(matches!(
            failure,
            ScanFailure::Message(message) if message == cleanr_fs::SCAN_CANCELLED
        ));
    }

    #[test]
    fn budget_limited_scan_skips_analysis_and_plan_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("one"), b"one").expect("first entry");
        std::fs::write(temp.path().join("two"), b"two").expect("second entry");
        let (sender, _receiver) = mpsc::channel();
        let (sample_sender, _sample_receiver) = mpsc::sync_channel(1);
        let mut progress = ScanProgressRecorder::new(9, &sender, &sample_sender);

        let prepared = run_scan_worker(
            ScanRequest::paths(vec![temp.path().to_path_buf()]),
            Vec::new(),
            ScanOptions {
                budgets: cleanr_core::ScanBudgetLimits {
                    entries: 1,
                    ..cleanr_core::ScanBudgetLimits::default()
                },
                ..ScanOptions::default()
            },
            ScanPreparation {
                registry: Arc::new(RuleRegistry::builtin().expect("builtin rules")),
                safety_policy: SafetyPolicy::new(Vec::new(), true),
                preselect_after_days: 90,
                prepare_usage: false,
            },
            &AtomicBool::new(false),
            &mut progress,
        )
        .expect("budget-limited preparation");

        assert!(matches!(prepared.planning, Ok(None)));
    }

    #[test]
    fn empty_resolved_scope_never_falls_back_to_current_directory() {
        let (sender, _receiver) = mpsc::channel();
        let (sample_sender, _sample_receiver) = mpsc::sync_channel(1);
        let mut progress = ScanProgressRecorder::new(3, &sender, &sample_sender);
        let result = run_scan_worker(
            ScanRequest {
                include_global: true,
                ..ScanRequest::default()
            },
            Vec::new(),
            ScanOptions::default(),
            ScanPreparation {
                registry: Arc::new(RuleRegistry::builtin().expect("builtin rules")),
                safety_policy: SafetyPolicy::new(Vec::new(), true),
                preselect_after_days: 90,
                prepare_usage: false,
            },
            &AtomicBool::new(false),
            &mut progress,
        );
        assert!(matches!(result, Err(ScanFailure::NoGlobalRoots)));
    }
}
