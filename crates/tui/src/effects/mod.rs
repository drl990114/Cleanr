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
    AnalysisReport, AnalysisScanContext, CandidateId, CleanupPlan, EntryKind, ExecutionManifest,
    GlobalScanEvidence, GlobalScanKind, RecommendationPolicy, RestoreManifest, SafetyPolicy,
    ScanEntry, ScanRequest, UserSelection, build_analysis_report_with_scan_context,
    build_cleanup_plan_from_analysis, suppress_unrequested_global_candidates,
};
use cleanr_fs::{
    ResolvedScanRoots, ScanOptions, ScanPhase, ScanProgress, ScanReport, global_scan_evidence,
    resolve_scan_roots_with_locations, scan_resolved_paths_with_progress_cancellable_started_at,
};
use cleanr_i18n::I18n;
use cleanr_plugin_api::discover_bundles;
use cleanr_rules::RuleRegistry;
use cleanr_tasks::{
    CleanupAuthorization, CleanupExecutor, ManifestRepository, SystemRestoreExecutor,
    execute_cleanup_plan, restore_execution_manifest, write_cleanup_plan,
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
    progress.emit_stage(ScanStage::Resolving);
    ensure_scan_active(cancellation)?;
    let mut explicit_roots = Vec::with_capacity(request.paths.len());
    for path in &request.paths {
        ensure_scan_active(cancellation)?;
        explicit_roots.push(path.canonicalize().unwrap_or_else(|_| path.clone()));
    }
    let resolved = resolve_scan_roots_with_locations(
        &request,
        &configured_global_kinds,
        preparation.registry.scan_locations(),
    )
    .map_err(|error| scan_failure(error, cancellation))?;
    ensure_scan_active(cancellation)?;
    if resolved.roots.is_empty() {
        return Err(ScanFailure::NoGlobalRoots);
    }
    progress.emit_stage(ScanStage::Scanning);
    ensure_scan_active(cancellation)?;
    let report = scan_resolved_scope(&resolved, &options, cancellation, progress)?;
    ensure_scan_active(cancellation)?;
    let global_scan = global_scan_evidence(
        &request,
        &configured_global_kinds,
        &resolved,
        &report.completed_roots,
    );

    prepare_scan(
        report,
        explicit_roots,
        global_scan,
        preparation,
        cancellation,
        progress,
    )
}

fn scan_resolved_scope(
    resolved: &ResolvedScanRoots,
    options: &ScanOptions,
    cancellation: &AtomicBool,
    progress: &mut ScanProgressRecorder<'_>,
) -> std::result::Result<ScanReport, ScanFailure> {
    scan_resolved_paths_with_progress_cancellable_started_at(
        resolved,
        options,
        cancellation,
        progress.started_at,
        |sample| progress.emit_filesystem_progress(sample),
    )
    .map_err(|error| scan_failure(error, cancellation))
}

fn prepare_scan(
    mut report: ScanReport,
    explicit_roots: Vec<PathBuf>,
    global_scan: GlobalScanEvidence,
    preparation: ScanPreparation,
    cancellation: &AtomicBool,
    progress: &mut ScanProgressRecorder<'_>,
) -> std::result::Result<PreparedScan, ScanFailure> {
    progress.emit_stage(ScanStage::Rules);
    ensure_scan_active(cancellation)?;
    preparation
        .registry
        .annotate_entries_at(&mut report.entries, report.as_of);
    ensure_scan_active(cancellation)?;

    let candidate_entry_indices = report
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (!entry.rule_hits.is_empty()).then_some(index))
        .collect::<Vec<_>>();
    let candidate_count = candidate_entry_indices.len();

    let planning = if report.budget_exceeded.is_empty() {
        progress.emit_stage(ScanStage::Evidence);
        ensure_scan_active(cancellation)?;
        let evidence = if report.entries.is_empty() {
            Ok(None)
        } else {
            prepare_evidence(&report, &explicit_roots, &global_scan, &preparation).map(Some)
        };
        ensure_scan_active(cancellation)?;

        progress.emit_stage(ScanStage::Plan);
        ensure_scan_active(cancellation)?;
        let planning = match evidence {
            Ok(Some(evidence)) => prepare_plan(&report, evidence, &preparation).map(Some),
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        };
        ensure_scan_active(cancellation)?;
        planning
    } else {
        // Budget-limited evidence is useful for review but can never produce a plan. Avoid the
        // otherwise O(N) analysis/selection pass that would immediately be discarded.
        Ok(None)
    };

    let usage = if preparation.prepare_usage {
        progress.emit_stage(ScanStage::Usage);
        ensure_scan_active(cancellation)?;
        let usage = build_usage_projection(&report.entries, &report.summary.roots);
        ensure_scan_active(cancellation)?;
        Some(usage)
    } else {
        None
    };

    Ok(PreparedScan {
        report,
        explicit_roots,
        global_scan,
        candidate_count,
        candidate_entry_indices,
        usage,
        planning,
    })
}

struct PreparedEvidence {
    analysis: AnalysisReport,
    candidate_ids_by_path: HashMap<PathBuf, CandidateId>,
    selection: UserSelection,
}

fn prepare_evidence(
    report: &ScanReport,
    explicit_roots: &[PathBuf],
    global_scan: &GlobalScanEvidence,
    preparation: &ScanPreparation,
) -> std::result::Result<PreparedEvidence, String> {
    let recommendation_policy = RecommendationPolicy::new(preparation.preselect_after_days)
        .map_err(|error| error.to_string())?;
    let scan_roots = report.summary.roots.clone();
    let mut analysis = build_analysis_report_with_scan_context(
        report.as_of,
        chrono::Utc::now(),
        scan_roots.clone(),
        &report.entries,
        &report.issues,
        recommendation_policy,
        AnalysisScanContext {
            budget_exceeded: &report.budget_exceeded,
            safety_policy: Some(&preparation.safety_policy),
        },
    )
    .map_err(|error| error.to_string())?;
    analysis.scan.global = global_scan.clone();
    suppress_unrequested_global_candidates(&mut analysis, explicit_roots);

    let candidate_ids_by_path = analysis
        .candidates
        .iter()
        .map(|candidate| (candidate.local_path.clone(), candidate.id.clone()))
        .collect();
    let selection = UserSelection::from_recommendations(&analysis);

    Ok(PreparedEvidence {
        analysis,
        candidate_ids_by_path,
        selection,
    })
}

fn prepare_plan(
    report: &ScanReport,
    evidence: PreparedEvidence,
    preparation: &ScanPreparation,
) -> std::result::Result<PreparedPlanning, String> {
    let PreparedEvidence {
        analysis,
        candidate_ids_by_path,
        selection,
    } = evidence;
    let plan = build_cleanup_plan_from_analysis(
        report.summary.roots.clone(),
        preparation.registry.versions(),
        &report.entries,
        &analysis,
        &selection,
        &preparation.safety_policy,
    )
    .map_err(|error| error.to_string())?;

    Ok(PreparedPlanning {
        analysis,
        candidate_ids_by_path,
        selection,
        plan,
    })
}

fn ensure_scan_active(cancellation: &AtomicBool) -> std::result::Result<(), ScanFailure> {
    if cancellation.load(Ordering::Relaxed) {
        Err(ScanFailure::Cancelled)
    } else {
        Ok(())
    }
}

fn scan_failure(error: anyhow::Error, cancellation: &AtomicBool) -> ScanFailure {
    if cleanr_fs::is_scan_cancelled(&error) || cancellation.load(Ordering::Relaxed) {
        ScanFailure::Cancelled
    } else {
        ScanFailure::Message(error.to_string())
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

pub(crate) fn spawn_cleanup(plan: CleanupPlan, state_dir: PathBuf) -> Result<OperationEffect> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-cleanup".to_string())
        .spawn(move || {
            let executor = cleanr_tasks::TrashExecutor;
            let result = execute_cleanup(&plan, &executor, &state_dir, true)
                .map_err(|error| error.to_string());
            let _ = sender.send(OperationEvent::CleanupFinished(result));
        })
        .context("failed to spawn cleanup worker")?;
    Ok(OperationEffect {
        kind: OperationKind::Cleanup,
        receiver,
    })
}

pub(crate) fn spawn_restore(
    manifest: ExecutionManifest,
    state_dir: PathBuf,
) -> Result<OperationEffect> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-restore".to_string())
        .spawn(move || {
            let result = restore_cleanup(&manifest, &state_dir).map_err(|error| error.to_string());
            let _ = sender.send(OperationEvent::RestoreFinished(result));
        })
        .context("failed to spawn restore worker")?;
    Ok(OperationEffect {
        kind: OperationKind::Restore,
        receiver,
    })
}

pub(crate) fn load_history(
    state_dir: &Path,
) -> Result<(Vec<ExecutionManifest>, Vec<RestoreManifest>)> {
    ManifestRepository::new(state_dir).history()
}

pub(crate) fn execute_cleanup(
    plan: &CleanupPlan,
    executor: &impl CleanupExecutor,
    state_dir: &Path,
    user_authorized: bool,
) -> Result<ExecutionManifest> {
    let authorization = user_authorized.then(CleanupAuthorization::explicit_user_confirmation);
    execute_cleanup_plan(plan, executor, state_dir, authorization.as_ref())
}

pub(crate) fn restore_cleanup(
    manifest: &ExecutionManifest,
    state_dir: &Path,
) -> Result<RestoreManifest> {
    restore_execution_manifest(manifest, &SystemRestoreExecutor, state_dir)
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
        let cancellation = AtomicBool::new(false);
        assert!(matches!(
            scan_failure(anyhow::Error::new(cleanr_fs::ScanCancelled), &cancellation),
            ScanFailure::Cancelled
        ));

        let failure = scan_failure(anyhow::anyhow!(cleanr_fs::SCAN_CANCELLED), &cancellation);
        assert!(matches!(
            failure,
            ScanFailure::Message(message) if message == cleanr_fs::SCAN_CANCELLED
        ));
    }

    #[test]
    fn budget_limited_scan_skips_analysis_and_plan_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let report = ScanReport {
            entries: vec![ScanEntry {
                path: temp.path().join("candidate"),
                kind: EntryKind::Directory,
                size_bytes: 1024,
                modified_at: None,
                rule_hits: Vec::new(),
            }],
            budget_exceeded: vec![cleanr_core::ScanBudgetExceeded::EntryCount {
                limit: 1,
                observed: 2,
            }],
            ..ScanReport::default()
        };
        let (sender, _receiver) = mpsc::channel();
        let (sample_sender, _sample_receiver) = mpsc::sync_channel(1);
        let mut progress = ScanProgressRecorder::new(9, &sender, &sample_sender);

        let prepared = prepare_scan(
            report,
            Vec::new(),
            GlobalScanEvidence::default(),
            ScanPreparation {
                registry: Arc::new(RuleRegistry::builtin().expect("builtin rules")),
                safety_policy: SafetyPolicy::new(Vec::new(), true),
                // This would fail RecommendationPolicy validation if evidence preparation ran.
                preselect_after_days: u16::MAX,
                prepare_usage: false,
            },
            &AtomicBool::new(false),
            &mut progress,
        )
        .expect("budget-limited preparation");

        assert!(matches!(prepared.planning, Ok(None)));
    }

    #[test]
    fn resolved_scan_preserves_prepared_roots_without_second_normalization() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("entry"), b"data").expect("write");
        let prepared_root = temp.path().join(".");
        let resolved = ResolvedScanRoots {
            // A user-provided scan entry would canonicalize and deduplicate this list. The
            // resolved entry point must preserve it exactly because resolution already happened.
            roots: vec![prepared_root.clone(), prepared_root],
            ..ResolvedScanRoots::default()
        };
        let (sender, _receiver) = mpsc::channel();
        let (sample_sender, _sample_receiver) = mpsc::sync_channel(1);
        let mut progress = ScanProgressRecorder::new(1, &sender, &sample_sender);

        let report = scan_resolved_scope(
            &resolved,
            &ScanOptions::default(),
            &AtomicBool::new(false),
            &mut progress,
        )
        .expect("resolved scan");

        assert_eq!(report.summary.roots, resolved.roots);
        assert_eq!(report.summary.roots.len(), 2);
    }

    #[test]
    fn empty_resolved_scope_never_falls_back_to_current_directory() {
        let empty = ResolvedScanRoots::default();
        let (sender, _receiver) = mpsc::channel();
        let (sample_sender, _sample_receiver) = mpsc::sync_channel(1);
        let mut progress = ScanProgressRecorder::new(2, &sender, &sample_sender);
        let report = scan_resolved_scope(
            &empty,
            &ScanOptions::default(),
            &AtomicBool::new(false),
            &mut progress,
        )
        .expect("empty resolved scan");
        assert!(report.summary.roots.is_empty());
        assert!(report.entries.is_empty());

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
