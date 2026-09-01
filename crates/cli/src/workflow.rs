use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use cleanr_config::{Config, default_config_path, default_state_dir};
use cleanr_core::{
    AnalysisReport, AnalysisScanContext, CleanupPlan, CleanupPlanBuildError, GlobalScanEvidence,
    RecommendationPolicy, RecommendationState, SafetyPolicy, ScanRequest, UserSelection,
    build_analysis_report_with_scan_context, build_cleanup_plan_from_analysis,
    suppress_unrequested_global_candidates,
};
use cleanr_fs::{
    ScanOptions, ScanReport, global_scan_evidence, resolve_scan_roots,
    scan_resolved_paths_started_at,
};
use cleanr_rules::RuleRegistry;
use cleanr_tasks::{
    CleanupAuthorization, ManifestRepository, SystemRestoreExecutor, TrashExecutor,
    execute_cleanup_plan, restore_execution_manifest, restored_run_ids, write_cleanup_plan,
};
use serde_json::json;
use sha2::{Digest, Sha256};

pub struct ScanCommand {
    pub config_path: Option<PathBuf>,
    pub request: ScanRequest,
    pub json: bool,
}

/// A read-only local analysis request intended for scripts and external local agents.
pub struct AnalyzeCommand {
    pub config_path: Option<PathBuf>,
    pub request: ScanRequest,
}

pub struct PlanCommand {
    pub config_path: Option<PathBuf>,
    pub request: ScanRequest,
    pub selection: SelectionOverrides,
    pub output: Option<PathBuf>,
}

pub struct DryRunCommand {
    pub config_path: Option<PathBuf>,
    pub request: ScanRequest,
    pub selection: SelectionOverrides,
    pub json: bool,
    pub output: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct SelectionOverrides {
    pub select_paths: Vec<PathBuf>,
    pub deselect_paths: Vec<PathBuf>,
}

pub struct CleanCommand {
    pub config_path: Option<PathBuf>,
    pub plan_path: PathBuf,
    pub plan_sha256: String,
    pub authorized_by_user: bool,
}

struct WorkflowScan {
    config: Config,
    config_path: Option<PathBuf>,
    registry: RuleRegistry,
    roots: Vec<PathBuf>,
    explicit_roots: Vec<PathBuf>,
    global_scan: GlobalScanEvidence,
    report: ScanReport,
}

pub fn scan(command: ScanCommand) -> Result<()> {
    let scan = run_scan(
        command.config_path,
        command.request,
        ScanPreparationMode::Evidence,
    )?;
    if command.json {
        print_scan_json(&scan.report)?;
    } else {
        let candidates = scan
            .report
            .entries
            .iter()
            .filter(|entry| !entry.rule_hits.is_empty())
            .count();
        println!(
            "Scanned {} entrie(s), found {} candidate(s), total size {}.",
            scan.report.summary.entries_seen,
            candidates,
            format_bytes(scan.report.summary.total_size_bytes)
        );
        if scan.report.summary.errors > 0 {
            println!("Scan errors: {}", scan.report.summary.errors);
        }
        println!("Traversal workers used: {}", scan.report.workers_used);
        if !scan.report.budget_exceeded.is_empty() {
            println!(
                "Scan budgets were reached; this partial result is read-only and cannot produce a cleanup plan."
            );
        }
    }
    Ok(())
}

/// Print a versioned evidence report. This command never creates a cleanup plan or mutates files.
pub fn analyze(command: AnalyzeCommand) -> Result<()> {
    let scan = run_scan(
        command.config_path,
        command.request,
        ScanPreparationMode::Evidence,
    )?;
    let safety = safety_policy(&scan.config, scan.config_path.clone());
    let report = build_analysis_report(&scan, recommendation_policy(&scan.config)?, &safety)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub fn plan(command: PlanCommand) -> Result<()> {
    let scan = run_scan(
        command.config_path,
        command.request,
        ScanPreparationMode::Planning,
    )?;
    let plan = build_plan(&scan, PlanSelection::Recommendations(command.selection))?;
    write_or_print_plan(&plan, command.output)
}

pub fn dry_run(command: DryRunCommand) -> Result<()> {
    let scan = run_scan(
        command.config_path,
        command.request,
        ScanPreparationMode::Planning,
    )?;
    let plan = build_plan(&scan, PlanSelection::Recommendations(command.selection))?;
    if let Some(path) = command.output {
        write_cleanup_plan(&plan, &path)?;
        println!("Dry run wrote {}", path.display());
    }
    if command.json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        println!(
            "Dry run: {} candidate(s), {} selected, {} selected bytes. No files were changed.",
            plan.summary.candidate_count,
            plan.summary.selected_count,
            format_bytes(plan.summary.selected_size_bytes)
        );
    }
    Ok(())
}

pub fn clean(command: CleanCommand) -> Result<()> {
    if !command.authorized_by_user {
        bail!("cleanup requires --authorized-by-user for the exact reviewed plan");
    }
    validate_sha256(&command.plan_sha256)?;
    let plan_bytes = std::fs::read(&command.plan_path).with_context(|| {
        format!(
            "failed to read cleanup plan {}",
            command.plan_path.display()
        )
    })?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&plan_bytes));
    if !actual_sha256.eq_ignore_ascii_case(&command.plan_sha256) {
        bail!(
            "cleanup plan SHA-256 changed after authorization: expected {}, found {}",
            command.plan_sha256,
            actual_sha256
        );
    }
    let expected_plan: CleanupPlan = serde_json::from_slice(&plan_bytes).with_context(|| {
        format!(
            "failed to parse cleanup plan {}",
            command.plan_path.display()
        )
    })?;
    ensure_plan_source_scan_is_executable(&expected_plan)?;
    if expected_plan.summary.selected_count == 0 {
        bail!("cleanup plan has no selected items");
    }

    let scan = run_scan(
        command.config_path,
        ScanRequest::paths(expected_plan.scan_roots.clone()),
        ScanPreparationMode::Planning,
    )?;
    let selected_paths = expected_plan
        .items
        .iter()
        .filter(|item| item.selected)
        .map(|item| item.path.clone())
        .collect();
    let current_plan = build_plan(&scan, PlanSelection::ExactPaths(selected_paths))?;
    ensure_plan_unchanged(&expected_plan, &current_plan)?;

    let authorization = CleanupAuthorization::explicit_user_delegation();
    let state_dir = default_state_dir();
    let manifest = execute_cleanup_plan(
        &current_plan,
        &TrashExecutor,
        &state_dir,
        Some(&authorization),
    )?;
    println!(
        "Cleanup run {} moved {} item(s) to trash; {} failed. Restore with `cleanr restore run {} --confirm`.",
        manifest.run_id, manifest.summary.succeeded, manifest.summary.failed, manifest.run_id
    );
    Ok(())
}

pub fn restore_list() -> Result<()> {
    let repository = ManifestRepository::new(default_state_dir());
    let (runs, restores) = repository.history()?;
    let restored = restored_run_ids(&restores);
    if runs.is_empty() {
        println!("No cleanup runs found");
        return Ok(());
    }
    for run in runs {
        let state = if restored.contains(run.run_id.as_str()) {
            "restored"
        } else {
            "available"
        };
        println!(
            "{} {} succeeded={} failed={} {}",
            run.run_id,
            run.created_at.to_rfc3339(),
            run.summary.succeeded,
            run.summary.failed,
            state
        );
    }
    Ok(())
}

pub fn restore_run(run_id: &str, confirm: bool) -> Result<()> {
    if !confirm {
        bail!("restore run requires --confirm");
    }
    let repository = ManifestRepository::new(default_state_dir());
    let manifest = repository
        .find_execution(run_id)?
        .with_context(|| format!("cleanup run {run_id} was not found"))?;
    let restore =
        restore_execution_manifest(&manifest, &SystemRestoreExecutor, repository.state_dir())?;
    println!(
        "Restored {} item(s), failed {} item(s), restore id {}.",
        restore.summary.succeeded, restore.summary.failed, restore.restore_id
    );
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanPreparationMode {
    /// Preserve rule-derived candidates for read-only scan and analysis output, including partial
    /// budget evidence.
    Evidence,
    /// Planning can never consume budget-limited evidence, so avoid rule annotation in that case.
    Planning,
}

fn run_scan(
    config_path: Option<PathBuf>,
    request: ScanRequest,
    preparation_mode: ScanPreparationMode,
) -> Result<WorkflowScan> {
    let config_path_for_policy = config_path.clone().or_else(default_config_path);
    let config = load_config(config_path)?;
    let registry = RuleRegistry::load(&config)?;
    let scan_started_at = Instant::now();
    let explicit_roots = request
        .paths
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect();
    let resolved = resolve_scan_roots(&request, &config.scan.global_kinds)?;
    let options = ScanOptions {
        stay_on_filesystem: config.scan.stay_on_filesystem,
        ignore_dirs: config.scan.ignore_dirs.clone(),
        ignore_patterns: config.scan.ignore_patterns.clone(),
        budgets: config.scan.budgets.limits()?,
        ..ScanOptions::default()
    };
    let mut report = scan_resolved_paths_started_at(&resolved, &options, scan_started_at)?;
    if preparation_mode == ScanPreparationMode::Evidence || report.budget_exceeded.is_empty() {
        registry.annotate_entries_at(&mut report.entries, report.as_of);
    }
    let roots = report.summary.roots.clone();
    let global_scan = global_scan_evidence(
        &request,
        &config.scan.global_kinds,
        &resolved,
        &report.completed_roots,
    );
    Ok(WorkflowScan {
        config,
        config_path: config_path_for_policy,
        registry,
        roots,
        explicit_roots,
        global_scan,
        report,
    })
}

fn build_analysis_report(
    scan: &WorkflowScan,
    policy: RecommendationPolicy,
    safety: &SafetyPolicy,
) -> Result<AnalysisReport> {
    let mut report = build_analysis_report_with_scan_context(
        scan.report.as_of,
        Utc::now(),
        scan.roots.clone(),
        &scan.report.entries,
        &scan.report.issues,
        policy,
        AnalysisScanContext {
            budget_exceeded: &scan.report.budget_exceeded,
            safety_policy: Some(safety),
        },
    )?;
    report.scan.global = scan.global_scan.clone();
    suppress_unrequested_global_candidates(&mut report, &scan.explicit_roots);
    Ok(report)
}

enum PlanSelection {
    Recommendations(SelectionOverrides),
    ExactPaths(Vec<PathBuf>),
}

fn build_plan(scan: &WorkflowScan, requested_selection: PlanSelection) -> Result<CleanupPlan> {
    if !scan.report.budget_exceeded.is_empty() {
        return Err(CleanupPlanBuildError::ScanBudgetExceeded.into());
    }
    let safety = safety_policy(&scan.config, scan.config_path.clone());
    let analysis = build_analysis_report(scan, recommendation_policy(&scan.config)?, &safety)?;
    let selection = match requested_selection {
        PlanSelection::Recommendations(overrides) => {
            selection_with_overrides(&analysis, overrides)?
        }
        PlanSelection::ExactPaths(paths) => exact_selection(&analysis, &paths)?,
    };
    build_cleanup_plan_from_analysis(
        scan.roots.clone(),
        scan.registry.versions(),
        &scan.report.entries,
        &analysis,
        &selection,
        &safety,
    )
    .context("failed to build cleanup plan from analysis")
}

fn selection_with_overrides(
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

    let mut selection = UserSelection::from_recommendations(analysis);
    for path in deselect_paths {
        let candidate = candidate_for_path(analysis, &path)?;
        selection.deselect(&candidate.id);
    }
    for path in select_paths {
        let candidate = selectable_candidate_for_path(analysis, &path)?;
        selection.select(candidate.id.clone());
    }
    Ok(selection)
}

fn exact_selection(analysis: &AnalysisReport, paths: &[PathBuf]) -> Result<UserSelection> {
    let mut selection = UserSelection::default();
    for path in normalize_selection_paths(paths)? {
        let candidate = selectable_candidate_for_path(analysis, &path)?;
        selection.select(candidate.id.clone());
    }
    Ok(selection)
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

fn ensure_plan_unchanged(expected: &CleanupPlan, current: &CleanupPlan) -> Result<()> {
    let unchanged = expected.schema_version == current.schema_version
        && expected.scan_roots == current.scan_roots
        && expected.ruleset_versions == current.ruleset_versions
        && source_scan_provenance_unchanged(expected, current)
        && expected.summary == current.summary
        && expected.items == current.items
        && expected.safety == current.safety;
    if !unchanged {
        bail!(
            "cleanup plan changed after authorization; generate, review, and authorize a new plan"
        );
    }
    Ok(())
}

fn ensure_plan_source_scan_is_executable(plan: &CleanupPlan) -> Result<()> {
    if plan
        .source_scan
        .as_ref()
        .is_some_and(|source| !source.budget_exceeded.is_empty())
    {
        bail!(
            "cleanup plan came from a scan that exceeded a budget and is read-only; generate, review, and authorize a new complete plan"
        );
    }
    Ok(())
}

fn source_scan_provenance_unchanged(expected: &CleanupPlan, current: &CleanupPlan) -> bool {
    match (&expected.source_scan, &current.source_scan) {
        (None, None) => true,
        (Some(expected), Some(current)) => {
            expected.integrity == current.integrity
                && expected.budget_exceeded == current.budget_exceeded
        }
        // Plans written before source-scan provenance was added remain executable only when the
        // fresh scan has no budget ledger. The current builder already refuses budget-limited
        // analysis, while the rest of `ensure_plan_unchanged` still compares every plan item and
        // safety field. A new plan losing provenance remains a downgrade and is rejected.
        (None, Some(current)) => current.budget_exceeded.is_empty(),
        (Some(_), None) => false,
    }
}

fn recommendation_policy(config: &Config) -> Result<RecommendationPolicy> {
    Ok(RecommendationPolicy::new(
        config.recommendations.preselect_after_days,
    )?)
}

fn write_or_print_plan(plan: &CleanupPlan, output: Option<PathBuf>) -> Result<()> {
    if let Some(path) = output {
        write_cleanup_plan(plan, &path)?;
        let bytes = std::fs::read(&path)
            .with_context(|| format!("failed to read cleanup plan {}", path.display()))?;
        println!(
            "Wrote {} sha256={:x}",
            path.display(),
            Sha256::digest(bytes)
        );
    } else {
        println!("{}", serde_json::to_string_pretty(plan)?);
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("--plan-sha256 must be exactly 64 hexadecimal characters");
    }
    Ok(())
}

fn print_scan_json(report: &ScanReport) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&scan_json_value(report))?
    );
    Ok(())
}

fn scan_json_value(report: &ScanReport) -> serde_json::Value {
    let errors = report
        .errors
        .iter()
        .map(|error| {
            json!({
                "path": error.path.as_ref().map(|path| path.display().to_string()),
                "message": error.message,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "as_of": &report.as_of,
        "completeness": report.completeness(),
        "workers_used": report.workers_used,
        "completed_roots": &report.completed_roots,
        "budget_exceeded": &report.budget_exceeded,
        "summary": report.summary,
        "entries": report.entries,
        "issues": &report.issues,
        "errors": errors,
    })
}

fn load_config(path: Option<PathBuf>) -> Result<Config> {
    match path {
        Some(path) => Config::load_from(path),
        None => Config::load(),
    }
}

fn safety_policy(config: &Config, config_path: Option<PathBuf>) -> SafetyPolicy {
    let mut protected = Vec::new();
    protected.extend(cleanr_config::home_dir());
    protected.extend(config_path);
    if let Ok(executable) = std::env::current_exe() {
        protected.push(executable);
    }
    let mut protected_subtrees = vec![default_state_dir()];
    protected_subtrees.extend(config.plugins.dirs.iter().cloned());
    protected_subtrees.extend(config.i18n.dirs.iter().cloned());
    SafetyPolicy::new(protected, config.cleanup.require_confirm)
        .with_protected_subtrees(protected_subtrees)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanr_core::{
        Confidence, DecisionCode, EntryKind, GlobalScanKind, ReportIntegrity, RuleHit,
        RuleMatchRole, RuleTrust, ScanBudgetExceeded, ScanEntry, ScanIssue, ScanIssueCode,
    };
    use cleanr_fs::{GlobalScanRoot, ResolvedScanRoots, ScanError};

    #[test]
    fn scan_json_separates_structured_issues_from_local_diagnostics() {
        let report = ScanReport {
            issues: vec![ScanIssue {
                code: ScanIssueCode::MetadataUnavailable,
                path: Some(PathBuf::from("scope")),
            }],
            errors: vec![ScanError {
                path: Some(PathBuf::from("scope")),
                message: "local diagnostic text".to_string(),
            }],
            budget_exceeded: vec![ScanBudgetExceeded::IssueDetails {
                limit: 1,
                observed: 2,
            }],
            workers_used: 1,
            ..ScanReport::default()
        };

        let value = scan_json_value(&report);

        assert_eq!(value["completeness"], "partial");
        assert_eq!(
            value["issues"],
            json!([{
                "code": "metadata-unavailable",
                "path": "scope",
            }])
        );
        assert_eq!(
            value["errors"],
            json!([{
                "path": "scope",
                "message": "local diagnostic text",
            }])
        );
        assert!(value["issues"][0].get("message").is_none());
        assert_eq!(value["workers_used"], 1);
        assert_eq!(value["budget_exceeded"][0]["kind"], "issue-details");

        let pathless_report = ScanReport {
            issues: vec![ScanIssue {
                code: ScanIssueCode::Unknown,
                path: None,
            }],
            ..ScanReport::default()
        };
        let pathless_value = scan_json_value(&pathless_report);
        let pathless_issue = &pathless_value["issues"][0];
        assert_eq!(pathless_issue["code"], "unknown");
        assert!(pathless_issue.get("path").is_none());
    }

    #[test]
    fn budget_limited_plan_fails_without_creating_output() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::write(temp.path().join("entry"), b"data")?;
        let config_path = temp.path().join("config.toml");
        let mut config = Config::default();
        config.scan.budgets.max_entries = 1;
        config.save_to(&config_path)?;
        let output = temp.path().join("must-not-exist.json");

        let error = plan(PlanCommand {
            config_path: Some(config_path),
            request: ScanRequest::paths(vec![temp.path().to_path_buf()]),
            selection: SelectionOverrides::default(),
            output: Some(output.clone()),
        })
        .expect_err("budget evidence must not produce a plan");

        assert!(format!("{error:#}").contains("scan budget was exceeded"));
        assert!(!output.exists());

        let dry_run_output = temp.path().join("dry-run-must-not-exist.json");
        let error = dry_run(DryRunCommand {
            config_path: Some(temp.path().join("config.toml")),
            request: ScanRequest::paths(vec![temp.path().to_path_buf()]),
            selection: SelectionOverrides::default(),
            json: false,
            output: Some(dry_run_output.clone()),
        })
        .expect_err("budget evidence must not produce a dry-run plan");

        assert!(format!("{error:#}").contains("scan budget was exceeded"));
        assert!(!dry_run_output.exists());
        Ok(())
    }

    #[test]
    fn analysis_json_preserves_requested_global_kinds_and_named_locations() {
        let scan_root = PathBuf::from("/tmp/cleanr-cache");
        let pnpm = scan_root.join("pnpm");
        let request = ScanRequest::global(vec![
            GlobalScanKind::AppCaches,
            GlobalScanKind::DeveloperCaches,
            GlobalScanKind::BrowserCaches,
            GlobalScanKind::DeveloperCaches,
        ]);
        let resolved = ResolvedScanRoots {
            roots: vec![scan_root.clone()],
            global_roots: vec![GlobalScanRoot {
                path: scan_root.clone(),
                kind: GlobalScanKind::AppCaches,
                label: "Application caches".to_string(),
            }],
            global_locations: vec![
                GlobalScanRoot {
                    path: scan_root.clone(),
                    kind: GlobalScanKind::AppCaches,
                    label: "Application caches".to_string(),
                },
                GlobalScanRoot {
                    path: pnpm.clone(),
                    kind: GlobalScanKind::DeveloperCaches,
                    label: "pnpm cache".to_string(),
                },
            ],
        };
        let global_scan = global_scan_evidence(&request, &[], &resolved, &resolved.roots);
        let mut reversed_request = request.clone();
        reversed_request.global_kinds.reverse();
        let mut reversed_resolved = resolved.clone();
        reversed_resolved.global_locations.reverse();
        let reversed_global_scan = global_scan_evidence(
            &reversed_request,
            &[],
            &reversed_resolved,
            &reversed_resolved.roots,
        );
        assert_eq!(global_scan, reversed_global_scan);
        assert_eq!(
            serde_json::to_value(&global_scan).expect("coverage JSON"),
            serde_json::to_value(&reversed_global_scan).expect("reversed coverage JSON")
        );
        let config = Config::default();
        let scan = WorkflowScan {
            config: config.clone(),
            config_path: None,
            registry: RuleRegistry::builtin().expect("builtin registry"),
            roots: resolved.roots,
            explicit_roots: Vec::new(),
            global_scan,
            report: ScanReport::default(),
        };

        let report = build_analysis_report(
            &scan,
            RecommendationPolicy::default(),
            &safety_policy(&config, None),
        )
        .expect("analysis report");
        let value = serde_json::to_value(report).expect("analysis JSON");

        assert_eq!(
            value["scan"]["global"]["requested_kinds"],
            json!(["developer-caches", "browser-caches", "app-caches"])
        );
        assert_eq!(
            value["scan"]["global"]["locations"][0]["label"],
            "pnpm cache"
        );
        assert_eq!(
            value["scan"]["global"]["locations"][0]["local_path"],
            pnpm.to_string_lossy().as_ref()
        );
        assert_eq!(
            value["scan"]["global"]["locations"][0]["scan_root"],
            scan_root.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn app_cache_scope_suppresses_nested_unrequested_developer_candidates() {
        let scan_root = PathBuf::from("/tmp/cleanr-cache");
        let pnpm = scan_root.join("pnpm");
        let request = ScanRequest::global(vec![GlobalScanKind::AppCaches]);
        let resolved = ResolvedScanRoots {
            roots: vec![scan_root.clone()],
            global_roots: vec![GlobalScanRoot {
                path: scan_root.clone(),
                kind: GlobalScanKind::AppCaches,
                label: "Application caches".to_string(),
            }],
            global_locations: vec![
                GlobalScanRoot {
                    path: scan_root.clone(),
                    kind: GlobalScanKind::AppCaches,
                    label: "Application caches".to_string(),
                },
                GlobalScanRoot {
                    path: pnpm.clone(),
                    kind: GlobalScanKind::DeveloperCaches,
                    label: "pnpm cache".to_string(),
                },
            ],
        };
        let global_scan = global_scan_evidence(&request, &[], &resolved, &resolved.roots);
        let as_of = Utc::now();
        let config = Config::default();
        let scan = WorkflowScan {
            config: config.clone(),
            config_path: None,
            registry: RuleRegistry::builtin().expect("builtin registry"),
            roots: resolved.roots,
            explicit_roots: Vec::new(),
            global_scan,
            report: ScanReport {
                as_of,
                entries: vec![ScanEntry {
                    path: pnpm,
                    kind: EntryKind::Directory,
                    size_bytes: 1024,
                    modified_at: Some(as_of - chrono::Duration::days(100)),
                    rule_hits: vec![RuleHit {
                        rule_pack_id: "builtin-dev".to_string(),
                        rule_id: "pnpm-cache".to_string(),
                        label: "pnpm cache".to_string(),
                        category: "developer-cache".to_string(),
                        confidence: Confidence::High,
                        reason: "rebuildable".to_string(),
                        risk_note: "dependencies may be downloaded again".to_string(),
                        default_selected: true,
                        trust: RuleTrust::Builtin,
                        match_role: RuleMatchRole::Primary,
                    }],
                }],
                ..ScanReport::default()
            },
        };

        let analysis = build_analysis_report(
            &scan,
            RecommendationPolicy::default(),
            &safety_policy(&config, None),
        )
        .expect("analysis report");

        assert_eq!(
            analysis.candidates[0].recommendation.state,
            RecommendationState::Suppressed
        );
        assert!(
            analysis.candidates[0]
                .recommendation
                .codes
                .contains(&DecisionCode::GlobalKindNotRequested)
        );
        let plan = build_plan(
            &scan,
            PlanSelection::Recommendations(SelectionOverrides::default()),
        )
        .expect("cleanup plan");
        assert_eq!(plan.summary.candidate_count, 0);
        assert_eq!(plan.summary.selected_count, 0);
    }

    #[test]
    fn explicit_path_analysis_does_not_invent_global_coverage() {
        let request = ScanRequest::paths(vec![PathBuf::from("/tmp/project")]);
        let roots = request.paths.clone();
        let evidence = global_scan_evidence(
            &request,
            &GlobalScanKind::ALL,
            &ResolvedScanRoots::default(),
            &roots,
        );
        let config = Config::default();
        let scan = WorkflowScan {
            config: config.clone(),
            config_path: None,
            registry: RuleRegistry::builtin().expect("builtin registry"),
            roots,
            explicit_roots: vec![PathBuf::from("/tmp/project")],
            global_scan: evidence.clone(),
            report: ScanReport::default(),
        };
        let report = build_analysis_report(
            &scan,
            RecommendationPolicy::default(),
            &safety_policy(&config, None),
        )
        .expect("analysis report");
        let value = serde_json::to_value(report).expect("analysis JSON");

        assert_eq!(evidence, GlobalScanEvidence::default());
        assert!(value["scan"].get("global").is_none());
    }

    #[test]
    fn global_analysis_preserves_requested_kind_when_no_location_exists() {
        let request = ScanRequest::global(vec![GlobalScanKind::BrowserCaches]);
        let resolved = ResolvedScanRoots::default();
        let global_scan = global_scan_evidence(&request, &[], &resolved, &[]);
        let config = Config::default();
        let scan = WorkflowScan {
            config: config.clone(),
            config_path: None,
            registry: RuleRegistry::builtin().expect("builtin registry"),
            roots: Vec::new(),
            explicit_roots: Vec::new(),
            global_scan,
            report: ScanReport::default(),
        };

        let report = build_analysis_report(
            &scan,
            RecommendationPolicy::default(),
            &safety_policy(&config, None),
        )
        .expect("analysis report");
        let value = serde_json::to_value(report).expect("analysis JSON");

        assert_eq!(
            value["scan"]["global"]["requested_kinds"],
            json!(["browser-caches"])
        );
        assert_eq!(value["scan"]["global"]["locations"], json!([]));
        assert_eq!(value["scan"]["roots"], json!([]));
    }

    #[test]
    fn authorized_plan_must_match_content_and_scan_provenance() -> Result<()> {
        let expected = empty_analysis_backed_plan()?;
        let mut current = empty_analysis_backed_plan()?;
        let expected_analysis_id = expected
            .source_scan
            .as_ref()
            .context("expected plan should retain source scan provenance")?
            .analysis_id
            .clone();
        let current_analysis_id = current
            .source_scan
            .as_ref()
            .context("current plan should retain source scan provenance")?
            .analysis_id
            .clone();
        assert_ne!(expected_analysis_id, current_analysis_id);
        current.created_at += chrono::Duration::seconds(1);

        ensure_plan_unchanged(&expected, &current)?;

        let mut legacy_expected = expected.clone();
        legacy_expected.source_scan = None;
        ensure_plan_unchanged(&legacy_expected, &current)?;

        let mut budgeted_current_for_legacy = current.clone();
        budgeted_current_for_legacy
            .source_scan
            .as_mut()
            .context("current plan should retain source scan provenance")?
            .budget_exceeded
            .push(ScanBudgetExceeded::EntryCount {
                limit: 100,
                observed: 101,
            });
        assert!(ensure_plan_unchanged(&legacy_expected, &budgeted_current_for_legacy).is_err());

        let mut missing_source = current.clone();
        missing_source.source_scan = None;
        assert!(ensure_plan_unchanged(&expected, &missing_source).is_err());

        let mut changed_integrity = current.clone();
        changed_integrity
            .source_scan
            .as_mut()
            .context("current plan should retain source scan provenance")?
            .integrity = ReportIntegrity::Partial;
        assert!(ensure_plan_unchanged(&expected, &changed_integrity).is_err());

        let mut changed_budget = current.clone();
        changed_budget
            .source_scan
            .as_mut()
            .context("current plan should retain source scan provenance")?
            .budget_exceeded
            .push(ScanBudgetExceeded::EntryCount {
                limit: 100,
                observed: 101,
            });
        assert!(ensure_plan_unchanged(&expected, &changed_budget).is_err());

        current.summary.selected_count = 1;
        let error = ensure_plan_unchanged(&expected, &current)
            .err()
            .context("changed plan must be rejected")?;
        assert!(error.to_string().contains("changed after authorization"));
        Ok(())
    }

    #[test]
    fn clean_rejects_budget_exhausted_plan_before_selection_or_rescan() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let mut plan = empty_analysis_backed_plan()?;
        plan.source_scan
            .as_mut()
            .context("plan should retain source scan provenance")?
            .budget_exceeded
            .push(ScanBudgetExceeded::ElapsedTime {
                limit_millis: 500,
                observed_millis: 501,
            });
        assert_eq!(plan.summary.selected_count, 0);

        let plan_path = temp.path().join("budget-exhausted-plan.json");
        let plan_bytes = serde_json::to_vec(&plan)?;
        std::fs::write(&plan_path, &plan_bytes)?;
        let plan_sha256 = format!("{:x}", Sha256::digest(&plan_bytes));

        let error = clean(CleanCommand {
            config_path: Some(temp.path().join("must-not-be-read.toml")),
            plan_path,
            plan_sha256,
            authorized_by_user: true,
        })
        .err()
        .context("budget-exhausted plan must be rejected")?;

        assert!(error.to_string().contains("scan that exceeded a budget"));
        Ok(())
    }

    #[test]
    fn plan_hash_must_be_a_sha256_hex_digest() {
        validate_sha256(&"a".repeat(64)).expect("valid SHA-256");
        assert!(validate_sha256("abc").is_err());
        assert!(validate_sha256(&"z".repeat(64)).is_err());
    }

    #[test]
    fn exact_path_overrides_can_select_review_items_and_deselect_recommendations() {
        let temp = tempfile::tempdir().expect("tempdir");
        let preselected_path = temp.path().join("preselected");
        let review_path = temp.path().join("review");
        std::fs::create_dir(&preselected_path).expect("preselected directory");
        std::fs::create_dir(&review_path).expect("review directory");
        let as_of = Utc::now();
        let hit = |rule_id: &str, confidence, default_selected| RuleHit {
            rule_pack_id: "builtin-test".to_string(),
            rule_id: rule_id.to_string(),
            label: rule_id.to_string(),
            category: "cache".to_string(),
            confidence,
            reason: "candidate".to_string(),
            risk_note: "review".to_string(),
            default_selected,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
        };
        let entries = vec![
            ScanEntry {
                path: preselected_path.clone(),
                kind: EntryKind::Directory,
                size_bytes: 10,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit("safe", Confidence::High, true)],
            },
            ScanEntry {
                path: review_path.clone(),
                kind: EntryKind::Directory,
                size_bytes: 20,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit("review", Confidence::Medium, false)],
            },
        ];
        let analysis = cleanr_core::build_analysis_report(
            as_of,
            as_of,
            vec![temp.path().to_path_buf()],
            &entries,
            &[],
            RecommendationPolicy::default(),
        )
        .expect("analysis");

        let selection = selection_with_overrides(
            &analysis,
            SelectionOverrides {
                select_paths: vec![review_path.clone()],
                deselect_paths: vec![preselected_path.clone()],
            },
        )
        .expect("selection overrides");
        let selected_paths = analysis
            .candidates
            .iter()
            .filter(|candidate| selection.candidate_ids.contains(&candidate.id))
            .map(|candidate| candidate.local_path.clone())
            .collect::<Vec<_>>();

        assert_eq!(selected_paths, vec![review_path.clone()]);
        assert!(
            selection_with_overrides(
                &analysis,
                SelectionOverrides {
                    select_paths: vec![review_path.clone()],
                    deselect_paths: vec![review_path],
                },
            )
            .expect_err("conflicting override")
            .to_string()
            .contains("both selected and deselected")
        );
    }

    fn empty_analysis_backed_plan() -> Result<CleanupPlan> {
        let config = Config::default();
        let scan = WorkflowScan {
            config,
            config_path: None,
            registry: RuleRegistry::builtin()?,
            roots: vec![PathBuf::from("/repo")],
            explicit_roots: vec![PathBuf::from("/repo")],
            global_scan: GlobalScanEvidence::default(),
            report: ScanReport::default(),
        };
        build_plan(
            &scan,
            PlanSelection::Recommendations(SelectionOverrides::default()),
        )
    }
}
