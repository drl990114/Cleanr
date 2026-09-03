use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use cleanr_config::{Config, default_config_path, default_state_dir};
use cleanr_core::{
    AnalysisReport, AnalysisScanContext, CleanupPlan, CleanupPlanBuildError, CleanupPlanScanScope,
    GlobalScanEvidence, GlobalScanKind, RecommendationPolicy, RecommendationPolicyError,
    RecommendationState, RulesetVersion, SafetyPolicy, ScanBudgetExceeded, ScanEntry, ScanIssue,
    ScanRequest, UserSelection, build_analysis_report_with_scan_context,
    build_cleanup_plan_from_analysis,
};
use cleanr_fs::{
    ScanOptions, ScanProgress, ScanReport, global_scan_evidence, resolve_scan_roots_with_locations,
    scan_resolved_paths_with_progress_cancellable_started_at,
};
use cleanr_rules::RuleRegistry;
use sha2::{Digest, Sha256};

use crate::{CleanupAuthorization, TrashExecutor, execute_cleanup_plan, validate_recoverable_plan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanPreparationMode {
    /// Preserve complete or partial evidence for read-only reporting.
    Evidence,
    /// Skip rule and analysis work when a partial scan cannot produce a plan.
    Planning,
    /// Preserve rule hits for partial interactive scans, but build plans only from complete scans.
    Interactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanWorkflowStage {
    Resolving,
    Scanning,
    Rules,
    Evidence,
    Plan,
}

pub trait ScanWorkflowObserver {
    fn stage_changed(&mut self, _stage: ScanWorkflowStage) {}

    fn filesystem_progress(&mut self, _progress: &ScanProgress) {}
}

impl ScanWorkflowObserver for () {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanWorkflowError {
    Cancelled,
    NoRoots,
    Message(String),
}

impl fmt::Display for ScanWorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("scan cancelled"),
            Self::NoRoots => formatter.write_str("no scan roots were resolved"),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl Error for ScanWorkflowError {}

pub struct ScanWorkflowInput {
    pub request: ScanRequest,
    pub configured_global_kinds: Vec<GlobalScanKind>,
    pub options: ScanOptions,
    pub registry: Arc<RuleRegistry>,
    pub safety_policy: SafetyPolicy,
    pub recommendation_policy: RecommendationPolicy,
    pub preparation_mode: ScanPreparationMode,
}

pub struct PreparedWorkflowScan {
    pub report: ScanReport,
    pub explicit_roots: Vec<PathBuf>,
    pub global_scan: GlobalScanEvidence,
    pub candidate_count: usize,
    pub candidate_entry_indices: Vec<usize>,
    pub analysis: Option<AnalysisReport>,
    pub selection: UserSelection,
    pub plan: Option<CleanupPlan>,
    pub ruleset_versions: Vec<RulesetVersion>,
    pub safety_policy: SafetyPolicy,
    pub recommendation_policy: RecommendationPolicy,
}

pub struct ConfiguredWorkflowScan {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub prepared: PreparedWorkflowScan,
}

pub struct DelegatedCleanupRequest {
    pub config_path: Option<PathBuf>,
    pub plan_path: PathBuf,
    pub plan_sha256: String,
    pub authorized_by_user: bool,
}

#[derive(Debug, Default)]
pub struct SelectionOverrides {
    pub select_paths: Vec<PathBuf>,
    pub deselect_paths: Vec<PathBuf>,
}

pub fn run_configured_scan(
    config_path: Option<PathBuf>,
    request: ScanRequest,
    preparation_mode: ScanPreparationMode,
    policy_override: Option<RecommendationPolicy>,
    cancellation: Option<&AtomicBool>,
    observer: &mut impl ScanWorkflowObserver,
) -> std::result::Result<ConfiguredWorkflowScan, ScanWorkflowError> {
    let effective_config_path = config_path.clone().or_else(default_config_path);
    let config = match config_path {
        Some(path) => Config::load_from(path),
        None => Config::load(),
    }
    .map_err(scan_message)?;
    let recommendation_policy = match policy_override {
        Some(policy) => {
            policy.validate().map_err(scan_message)?;
            policy
        }
        None => recommendation_policy(&config, request.inactive_days).map_err(scan_message)?,
    };
    let registry = Arc::new(RuleRegistry::load(&config).map_err(scan_message)?);
    let options = ScanOptions {
        stay_on_filesystem: config.scan.stay_on_filesystem,
        ignore_dirs: config.scan.ignore_dirs.clone(),
        ignore_patterns: config.scan.ignore_patterns.clone(),
        budgets: config.scan.budgets.limits().map_err(scan_message)?,
        ..ScanOptions::default()
    };
    let safety_policy =
        safety_policy_for_config(&config, effective_config_path.clone(), default_state_dir());
    let prepared = run_scan_workflow(
        ScanWorkflowInput {
            request,
            configured_global_kinds: config.scan.global_kinds.clone(),
            options,
            registry,
            safety_policy,
            recommendation_policy,
            preparation_mode,
        },
        cancellation,
        observer,
    )?;
    Ok(ConfiguredWorkflowScan {
        config,
        config_path: effective_config_path,
        prepared,
    })
}

pub fn execute_delegated_cleanup(
    request: DelegatedCleanupRequest,
) -> Result<cleanr_core::ExecutionManifest> {
    if !request.authorized_by_user {
        bail!("cleanup requires --authorized-by-user for the exact reviewed plan");
    }
    validate_plan_sha256(&request.plan_sha256)?;
    let plan_bytes = std::fs::read(&request.plan_path).with_context(|| {
        format!(
            "failed to read cleanup plan {}",
            request.plan_path.display()
        )
    })?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&plan_bytes));
    if !actual_sha256.eq_ignore_ascii_case(&request.plan_sha256) {
        bail!(
            "cleanup plan SHA-256 changed after authorization: expected {}, found {}",
            request.plan_sha256,
            actual_sha256
        );
    }
    let expected_plan: CleanupPlan = serde_json::from_slice(&plan_bytes).with_context(|| {
        format!(
            "failed to parse cleanup plan {}",
            request.plan_path.display()
        )
    })?;
    validate_recoverable_plan(&expected_plan)?;
    if expected_plan.summary.selected_count == 0 {
        bail!("cleanup plan has no selected items");
    }

    let effective_config_path = request.config_path.clone().or_else(default_config_path);
    let config = match request.config_path {
        Some(path) => Config::load_from(path),
        None => Config::load(),
    }?;
    let saved_policy = recommendation_policy_for_plan_rescan(&config, &expected_plan)?;
    let rescan_request = expected_plan
        .source_scan
        .as_ref()
        .and_then(|source| source.scope.as_ref())
        .map_or_else(
            || ScanRequest::paths(expected_plan.scan_roots.clone()),
            CleanupPlanScanScope::to_scan_request,
        );
    let registry = Arc::new(RuleRegistry::load(&config)?);
    let options = ScanOptions {
        stay_on_filesystem: config.scan.stay_on_filesystem,
        ignore_dirs: config.scan.ignore_dirs.clone(),
        ignore_patterns: config.scan.ignore_patterns.clone(),
        budgets: config.scan.budgets.limits()?,
        ..ScanOptions::default()
    };
    let state_dir = default_state_dir();
    let safety_policy = safety_policy_for_config(&config, effective_config_path, state_dir.clone());
    let mut observer = ();
    let current_scan = run_scan_workflow(
        ScanWorkflowInput {
            request: rescan_request,
            configured_global_kinds: config.scan.global_kinds.clone(),
            options,
            registry,
            safety_policy,
            recommendation_policy: saved_policy,
            preparation_mode: ScanPreparationMode::Planning,
        },
        None,
        &mut observer,
    )
    .map_err(anyhow::Error::new)?;
    let analysis = current_scan
        .analysis
        .as_ref()
        .context("complete cleanup re-scan did not produce analysis evidence")?;
    let selected_paths = expected_plan
        .items
        .iter()
        .filter(|item| item.selected)
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    let selection = exact_selection(analysis, &selected_paths)?;
    let current_plan = build_workflow_plan(
        current_scan.report.summary.roots.clone(),
        current_scan.ruleset_versions.clone(),
        &current_scan.report.entries,
        analysis,
        &selection,
        &current_scan.safety_policy,
        &current_scan.explicit_roots,
        &current_scan.global_scan,
    )?;
    verify_plan_unchanged(&expected_plan, &current_plan)?;

    let authorization = CleanupAuthorization::explicit_user_delegation();
    execute_cleanup_plan(
        &current_plan,
        &TrashExecutor,
        state_dir,
        Some(&authorization),
    )
}

pub fn run_scan_workflow(
    input: ScanWorkflowInput,
    cancellation: Option<&AtomicBool>,
    observer: &mut impl ScanWorkflowObserver,
) -> std::result::Result<PreparedWorkflowScan, ScanWorkflowError> {
    let workflow_started_at = Instant::now();
    let never_cancelled = AtomicBool::new(false);
    let cancellation = cancellation.unwrap_or(&never_cancelled);
    ensure_scan_active(cancellation)?;
    input
        .recommendation_policy
        .validate()
        .map_err(scan_message)?;
    observer.stage_changed(ScanWorkflowStage::Resolving);

    let resolved = resolve_scan_roots_with_locations(
        &input.request,
        &input.configured_global_kinds,
        input.registry.scan_locations(),
    )
    .map_err(|error| scan_failure(error, cancellation))?;
    ensure_scan_active(cancellation)?;
    if resolved.roots.is_empty() && input.preparation_mode == ScanPreparationMode::Interactive {
        return Err(ScanWorkflowError::NoRoots);
    }
    let explicit_roots = semantic_explicit_roots(&input.request, &resolved.roots);

    observer.stage_changed(ScanWorkflowStage::Scanning);
    let mut report = scan_resolved_paths_with_progress_cancellable_started_at(
        &resolved,
        &input.options,
        cancellation,
        workflow_started_at,
        |progress| observer.filesystem_progress(&progress),
    )
    .map_err(|error| scan_failure(error, cancellation))?;
    ensure_scan_active(cancellation)?;

    let global_scan = global_scan_evidence(
        &input.request,
        &input.configured_global_kinds,
        &resolved,
        &report.completed_roots,
    );
    let scan_is_complete = report.budget_exceeded.is_empty();
    let annotate_entries =
        scan_is_complete || input.preparation_mode != ScanPreparationMode::Planning;
    if annotate_entries {
        observer.stage_changed(ScanWorkflowStage::Rules);
        input
            .registry
            .annotate_entries_at(&mut report.entries, report.as_of);
        ensure_scan_active(cancellation)?;
    }

    let candidate_entry_indices = report
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (!entry.rule_hits.is_empty()).then_some(index))
        .collect::<Vec<_>>();
    let candidate_count = candidate_entry_indices.len();
    let should_build_analysis = match input.preparation_mode {
        ScanPreparationMode::Evidence => true,
        ScanPreparationMode::Planning => scan_is_complete,
        ScanPreparationMode::Interactive => scan_is_complete && !report.entries.is_empty(),
    };
    let analysis = if should_build_analysis {
        observer.stage_changed(ScanWorkflowStage::Evidence);
        let analysis = build_workflow_analysis(
            &report,
            input.recommendation_policy.clone(),
            &input.safety_policy,
            &explicit_roots,
            &global_scan,
        )
        .map_err(scan_message)?;
        ensure_scan_active(cancellation)?;
        Some(analysis)
    } else {
        None
    };
    let selection = analysis
        .as_ref()
        .map(UserSelection::from_recommendations)
        .unwrap_or_default();
    let ruleset_versions = input.registry.versions();
    let plan = if input.preparation_mode != ScanPreparationMode::Evidence {
        analysis
            .as_ref()
            .map(|analysis| {
                observer.stage_changed(ScanWorkflowStage::Plan);
                build_workflow_plan(
                    report.summary.roots.clone(),
                    ruleset_versions.clone(),
                    &report.entries,
                    analysis,
                    &selection,
                    &input.safety_policy,
                    &explicit_roots,
                    &global_scan,
                )
                .map_err(scan_message)
            })
            .transpose()?
    } else {
        None
    };
    ensure_scan_active(cancellation)?;

    Ok(PreparedWorkflowScan {
        report,
        explicit_roots,
        global_scan,
        candidate_count,
        candidate_entry_indices,
        analysis,
        selection,
        plan,
        ruleset_versions,
        safety_policy: input.safety_policy,
        recommendation_policy: input.recommendation_policy,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_workflow_plan(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    entries: &[ScanEntry],
    analysis: &AnalysisReport,
    selection: &UserSelection,
    policy: &SafetyPolicy,
    explicit_roots: &[PathBuf],
    global_scan: &GlobalScanEvidence,
) -> std::result::Result<CleanupPlan, CleanupPlanBuildError> {
    let mut plan = build_cleanup_plan_from_analysis(
        scan_roots,
        ruleset_versions,
        entries,
        analysis,
        selection,
        policy,
    )?;
    if let Some(source) = plan.source_scan.as_mut() {
        source.scope = Some(CleanupPlanScanScope::new(
            explicit_roots.to_vec(),
            global_scan.requested_kinds.clone(),
        ));
    }
    Ok(plan)
}

pub fn build_workflow_analysis(
    report: &ScanReport,
    recommendation_policy: RecommendationPolicy,
    safety_policy: &SafetyPolicy,
    explicit_roots: &[PathBuf],
    global_scan: &GlobalScanEvidence,
) -> std::result::Result<AnalysisReport, RecommendationPolicyError> {
    build_workflow_analysis_from_parts(
        report.as_of,
        report.summary.roots.clone(),
        &report.entries,
        &report.issues,
        &report.budget_exceeded,
        recommendation_policy,
        safety_policy,
        explicit_roots,
        global_scan,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_workflow_analysis_from_parts(
    scan_as_of: chrono::DateTime<Utc>,
    scan_roots: Vec<PathBuf>,
    entries: &[ScanEntry],
    issues: &[ScanIssue],
    budget_exceeded: &[ScanBudgetExceeded],
    recommendation_policy: RecommendationPolicy,
    safety_policy: &SafetyPolicy,
    explicit_roots: &[PathBuf],
    global_scan: &GlobalScanEvidence,
) -> std::result::Result<AnalysisReport, RecommendationPolicyError> {
    build_analysis_report_with_scan_context(
        scan_as_of,
        Utc::now(),
        scan_roots,
        entries,
        issues,
        recommendation_policy,
        AnalysisScanContext {
            budget_exceeded,
            safety_policy: Some(safety_policy),
            global: Some(global_scan),
            explicit_roots,
        },
    )
}

pub fn selection_with_overrides(
    analysis: &AnalysisReport,
    overrides: SelectionOverrides,
) -> Result<UserSelection> {
    let select_paths = normalize_selection_paths(&overrides.select_paths)?;
    let deselect_paths = normalize_selection_paths(&overrides.deselect_paths)?;
    if let Some(path) = select_paths.intersection(&deselect_paths).next() {
        bail!(
            "candidate path cannot be both selected and deselected: {}",
            path.display()
        );
    }
    reject_overlapping_explicit_selections(&select_paths)?;

    let mut selection = UserSelection::from_recommendations(analysis);
    for path in deselect_paths {
        let candidate = candidate_for_path(analysis, &path)?;
        selection.deselect(&candidate.id);
    }
    for path in select_paths {
        let candidate = selectable_candidate_for_path(analysis, &path)?;
        let overlapping_ids = analysis
            .candidates
            .iter()
            .filter(|overlapping| {
                overlapping.id != candidate.id
                    && selection.candidate_ids.contains(&overlapping.id)
                    && cleanup_paths_overlap(&candidate.local_path, &overlapping.local_path)
            })
            .map(|overlapping| overlapping.id.clone())
            .collect::<Vec<_>>();
        for overlapping_id in overlapping_ids {
            selection.deselect(&overlapping_id);
        }
        selection.select(candidate.id.clone());
    }
    Ok(selection)
}

pub fn exact_selection(analysis: &AnalysisReport, paths: &[PathBuf]) -> Result<UserSelection> {
    let mut selection = UserSelection::default();
    for path in normalize_selection_paths(paths)? {
        let candidate = selectable_candidate_for_path(analysis, &path)?;
        selection.select(candidate.id.clone());
    }
    Ok(selection)
}

pub fn recommendation_policy(
    config: &Config,
    inactive_days: Option<u16>,
) -> std::result::Result<RecommendationPolicy, RecommendationPolicyError> {
    RecommendationPolicy::new(inactive_days.unwrap_or(config.recommendations.preselect_after_days))
}

pub fn recommendation_policy_for_plan_rescan(
    config: &Config,
    plan: &CleanupPlan,
) -> std::result::Result<RecommendationPolicy, RecommendationPolicyError> {
    let mut policy = plan
        .source_scan
        .as_ref()
        .and_then(|source| source.recommendation_policy.clone())
        .map_or_else(|| recommendation_policy(config, None), Ok)?;
    if plan
        .source_scan
        .as_ref()
        .is_none_or(|source| source.recommendation_policy.is_none())
    {
        // Plans produced before recommendation policy provenance used the v1 projection rules.
        policy.version = "v1".to_string();
    }
    policy.validate()?;
    Ok(policy)
}

pub fn safety_policy_for_config(
    config: &Config,
    config_path: Option<PathBuf>,
    state_dir: PathBuf,
) -> SafetyPolicy {
    let mut protected = Vec::new();
    protected.extend(cleanr_config::home_dir());
    protected.extend(config_path);
    if let Ok(executable) = std::env::current_exe() {
        protected.push(executable);
    }
    let mut protected_subtrees = vec![state_dir];
    protected_subtrees.extend(config.plugins.dirs.iter().cloned());
    protected_subtrees.extend(config.i18n.dirs.iter().cloned());
    SafetyPolicy::new(protected, config.cleanup.require_confirm)
        .with_protected_subtrees(protected_subtrees)
}

fn reject_overlapping_explicit_selections(paths: &BTreeSet<PathBuf>) -> Result<()> {
    let paths = paths.iter().collect::<Vec<_>>();
    for (index, left) in paths.iter().enumerate() {
        if let Some(right) = paths
            .iter()
            .skip(index + 1)
            .find(|right| cleanup_paths_overlap(left, right))
        {
            bail!(
                "explicitly selected candidate paths overlap: {} and {}",
                left.display(),
                right.display()
            );
        }
    }
    Ok(())
}

fn cleanup_paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn normalize_selection_paths(paths: &[PathBuf]) -> Result<BTreeSet<PathBuf>> {
    paths
        .iter()
        .map(|path| normalize_selection_path(path))
        .collect()
}

fn normalize_selection_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve the current directory for candidate selection")?
            .join(path)
    };
    absolute
        .canonicalize()
        .with_context(|| format!("selected candidate path does not exist: {}", path.display()))
}

fn candidate_for_path<'a>(
    analysis: &'a AnalysisReport,
    normalized_path: &Path,
) -> Result<&'a cleanr_core::CandidateEvidence> {
    analysis
        .candidates
        .iter()
        .find(|candidate| {
            candidate
                .local_path
                .canonicalize()
                .unwrap_or_else(|_| candidate.local_path.clone())
                == normalized_path
        })
        .with_context(|| {
            format!(
                "path is not a cleanup candidate in this scan: {}",
                normalized_path.display()
            )
        })
}

fn selectable_candidate_for_path<'a>(
    analysis: &'a AnalysisReport,
    normalized_path: &Path,
) -> Result<&'a cleanr_core::CandidateEvidence> {
    let candidate = candidate_for_path(analysis, normalized_path)?;
    if matches!(
        candidate.recommendation.state,
        RecommendationState::Suppressed | RecommendationState::Excluded
    ) {
        bail!(
            "candidate cannot be selected because its recommendation state is {}: {}",
            candidate.recommendation.state,
            normalized_path.display()
        );
    }
    Ok(candidate)
}

pub fn validate_plan_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("--plan-sha256 must be exactly 64 hexadecimal characters");
    }
    Ok(())
}

pub fn verify_plan_unchanged(expected: &CleanupPlan, current: &CleanupPlan) -> Result<()> {
    let unchanged = expected.schema_version == current.schema_version
        && expected.scan_roots == current.scan_roots
        && expected.ruleset_versions == current.ruleset_versions
        && source_scan_provenance_unchanged(expected, current)
        && selected_plan_projection_unchanged(expected, current)
        && expected.safety == current.safety;
    if !unchanged {
        bail!(
            "selected cleanup targets or safety provenance changed after authorization; generate, review, and authorize a new plan"
        );
    }
    Ok(())
}

fn selected_plan_projection_unchanged(expected: &CleanupPlan, current: &CleanupPlan) -> bool {
    expected.summary.selected_count == current.summary.selected_count
        && expected.summary.selected_size_bytes == current.summary.selected_size_bytes
        && expected
            .items
            .iter()
            .filter(|item| item.selected)
            .eq(current.items.iter().filter(|item| item.selected))
}

fn source_scan_provenance_unchanged(expected: &CleanupPlan, current: &CleanupPlan) -> bool {
    match (&expected.source_scan, &current.source_scan) {
        (None, None) => true,
        (Some(expected), Some(current)) => {
            expected.integrity == current.integrity
                && expected.budget_exceeded == current.budget_exceeded
                && expected
                    .recommendation_policy
                    .as_ref()
                    .is_none_or(|policy| current.recommendation_policy.as_ref() == Some(policy))
                && expected
                    .scope
                    .as_ref()
                    .is_none_or(|scope| current.scope.as_ref() == Some(scope))
        }
        (None, Some(current)) => current.budget_exceeded.is_empty(),
        (Some(_), None) => false,
    }
}

pub fn semantic_explicit_roots(request: &ScanRequest, resolved_roots: &[PathBuf]) -> Vec<PathBuf> {
    if request.paths.is_empty() && !request.include_global {
        return resolved_roots.to_vec();
    }
    request
        .paths
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect()
}

fn ensure_scan_active(cancellation: &AtomicBool) -> std::result::Result<(), ScanWorkflowError> {
    if cancellation.load(Ordering::Relaxed) {
        Err(ScanWorkflowError::Cancelled)
    } else {
        Ok(())
    }
}

fn scan_failure(error: anyhow::Error, cancellation: &AtomicBool) -> ScanWorkflowError {
    if cleanr_fs::is_scan_cancelled(&error) || cancellation.load(Ordering::Relaxed) {
        ScanWorkflowError::Cancelled
    } else {
        ScanWorkflowError::Message(format!("{error:#}"))
    }
}

fn scan_message(error: impl fmt::Display) -> ScanWorkflowError {
    ScanWorkflowError::Message(format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanr_core::GlobalScanKind;

    #[derive(Default)]
    struct StageRecorder {
        stages: Vec<ScanWorkflowStage>,
    }

    impl ScanWorkflowObserver for StageRecorder {
        fn stage_changed(&mut self, stage: ScanWorkflowStage) {
            self.stages.push(stage);
        }
    }

    #[test]
    fn default_scope_records_the_resolved_working_directory() {
        let root = std::env::current_dir().expect("current directory");
        assert_eq!(
            semantic_explicit_roots(&ScanRequest::default(), std::slice::from_ref(&root)),
            vec![root]
        );
    }

    #[test]
    fn cancellation_is_typed() {
        let cancellation = AtomicBool::new(true);
        assert_eq!(
            ensure_scan_active(&cancellation),
            Err(ScanWorkflowError::Cancelled)
        );
    }

    #[test]
    fn recommendation_override_is_validated() {
        let config = Config::default();
        assert!(recommendation_policy(&config, Some(30)).is_ok());
        assert!(recommendation_policy(&config, Some(3651)).is_err());
    }

    #[test]
    fn explicit_global_scope_does_not_invent_local_roots() {
        let request = ScanRequest::global(vec![GlobalScanKind::AppCaches]);
        assert!(semantic_explicit_roots(&request, &[PathBuf::from("/tmp")]).is_empty());
    }

    #[test]
    fn read_only_empty_global_scope_preserves_cli_evidence_behavior() {
        let mut observer = StageRecorder::default();
        let prepared = run_scan_workflow(
            ScanWorkflowInput {
                request: ScanRequest {
                    include_global: true,
                    ..ScanRequest::default()
                },
                configured_global_kinds: Vec::new(),
                options: ScanOptions::default(),
                registry: Arc::new(RuleRegistry::builtin().expect("builtin rules")),
                safety_policy: SafetyPolicy::new(Vec::new(), true),
                recommendation_policy: RecommendationPolicy::default(),
                preparation_mode: ScanPreparationMode::Evidence,
            },
            None,
            &mut observer,
        )
        .expect("read-only CLI evidence may describe an empty global scope");

        assert!(prepared.report.summary.roots.is_empty());
        assert!(prepared.analysis.is_some());
        assert_eq!(
            observer.stages,
            vec![
                ScanWorkflowStage::Resolving,
                ScanWorkflowStage::Scanning,
                ScanWorkflowStage::Rules,
                ScanWorkflowStage::Evidence,
            ]
        );
    }

    #[test]
    fn progress_observers_do_not_change_workflow_results() {
        let temp = tempfile::tempdir().expect("tempdir");
        let candidate = temp.path().join("node_modules");
        std::fs::create_dir(&candidate).expect("candidate directory");
        std::fs::write(candidate.join("artifact.bin"), vec![0_u8; 2 * 1024 * 1024])
            .expect("candidate contents");
        let input = || ScanWorkflowInput {
            request: ScanRequest::paths(vec![temp.path().to_path_buf()]),
            configured_global_kinds: Vec::new(),
            options: ScanOptions::default(),
            registry: Arc::new(RuleRegistry::builtin().expect("builtin rules")),
            safety_policy: SafetyPolicy::new(Vec::new(), true),
            recommendation_policy: RecommendationPolicy::new(0).expect("policy"),
            preparation_mode: ScanPreparationMode::Interactive,
        };

        let mut no_observer = ();
        let baseline = run_scan_workflow(input(), None, &mut no_observer).expect("baseline scan");
        let mut recorder = StageRecorder::default();
        let observed = run_scan_workflow(input(), None, &mut recorder).expect("observed scan");

        assert_eq!(baseline.report.entries, observed.report.entries);
        assert_eq!(
            baseline.candidate_entry_indices,
            observed.candidate_entry_indices
        );
        assert_eq!(baseline.candidate_count, observed.candidate_count);
        let selected_paths = |scan: &PreparedWorkflowScan| {
            scan.analysis
                .as_ref()
                .expect("analysis")
                .candidates
                .iter()
                .filter(|candidate| scan.selection.candidate_ids.contains(&candidate.id))
                .map(|candidate| candidate.local_path.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(selected_paths(&baseline), selected_paths(&observed));
        assert_eq!(
            baseline.plan.as_ref().map(|plan| plan
                .items
                .iter()
                .map(|item| &item.path)
                .collect::<Vec<_>>()),
            observed.plan.as_ref().map(|plan| plan
                .items
                .iter()
                .map(|item| &item.path)
                .collect::<Vec<_>>())
        );
        assert_eq!(
            recorder.stages,
            vec![
                ScanWorkflowStage::Resolving,
                ScanWorkflowStage::Scanning,
                ScanWorkflowStage::Rules,
                ScanWorkflowStage::Evidence,
                ScanWorkflowStage::Plan,
            ]
        );
    }

    #[test]
    fn delegated_cleanup_rejects_missing_or_malformed_authorization_before_io() {
        let request = |authorized_by_user, plan_sha256: &str| DelegatedCleanupRequest {
            config_path: None,
            plan_path: PathBuf::from("missing-plan.json"),
            plan_sha256: plan_sha256.to_string(),
            authorized_by_user,
        };

        let unauthorized = execute_delegated_cleanup(request(false, &"0".repeat(64)))
            .expect_err("authorization is mandatory");
        assert!(unauthorized.to_string().contains("--authorized-by-user"));

        let malformed = execute_delegated_cleanup(request(true, "not-a-sha256"))
            .expect_err("digest shape is validated before reading the plan");
        assert!(malformed.to_string().contains("64 hexadecimal"));
    }
}
