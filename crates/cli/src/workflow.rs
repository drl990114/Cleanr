use std::{borrow::Cow, path::PathBuf};

use anyhow::{Context, Result, bail};
#[cfg(test)]
use cleanr_config::Config;
use cleanr_config::default_state_dir;
use cleanr_core::{
    AnalysisReport, CleanupPlan, GlobalScanEvidence, RecommendationPolicy, RecommendationState,
    RulesetVersion, SafetyPolicy, ScanRequest,
};
use cleanr_fs::ScanReport;
use cleanr_tasks::{
    DelegatedCleanupRequest, ManifestRepository, ScanPreparationMode, SystemRestoreExecutor,
    build_workflow_analysis, build_workflow_plan, execute_delegated_cleanup,
    restore_execution_manifest, restored_run_ids, run_configured_scan, selection_with_overrides,
    write_cleanup_plan,
};
#[cfg(test)]
use cleanr_tasks::{
    exact_selection, recommendation_policy, recommendation_policy_for_plan_rescan,
    validate_plan_sha256 as validate_sha256, verify_plan_unchanged as ensure_plan_unchanged,
};
use serde_json::json;
use sha2::{Digest, Sha256};

pub use cleanr_tasks::SelectionOverrides;

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

pub struct CleanCommand {
    pub config_path: Option<PathBuf>,
    pub plan_path: PathBuf,
    pub plan_sha256: String,
    pub authorized_by_user: bool,
}

struct WorkflowScan {
    #[cfg(test)]
    config: Config,
    recommendation_policy: RecommendationPolicy,
    ruleset_versions: Vec<RulesetVersion>,
    roots: Vec<PathBuf>,
    explicit_roots: Vec<PathBuf>,
    global_scan: GlobalScanEvidence,
    analysis: Option<AnalysisReport>,
    safety_policy: SafetyPolicy,
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
        let candidates = inactivity_qualified_candidate_count(&scan)?;
        println!(
            "Scanned {} entrie(s), found {} modification-age-qualified candidate(s), total size {}.",
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
    let report = build_analysis_report(&scan, &scan.safety_policy)?;
    println!("{}", serde_json::to_string_pretty(report.as_ref())?);
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
    let manifest = execute_delegated_cleanup(DelegatedCleanupRequest {
        config_path: command.config_path,
        plan_path: command.plan_path,
        plan_sha256: command.plan_sha256,
        authorized_by_user: command.authorized_by_user,
    })?;
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

fn run_scan(
    config_path: Option<PathBuf>,
    request: ScanRequest,
    preparation_mode: ScanPreparationMode,
) -> Result<WorkflowScan> {
    run_scan_with_recommendation_policy(config_path, request, preparation_mode, None)
}

fn run_scan_with_recommendation_policy(
    config_path: Option<PathBuf>,
    request: ScanRequest,
    preparation_mode: ScanPreparationMode,
    policy_override: Option<RecommendationPolicy>,
) -> Result<WorkflowScan> {
    let mut observer = ();
    let configured = run_configured_scan(
        config_path,
        request,
        preparation_mode,
        policy_override,
        None,
        &mut observer,
    )
    .map_err(anyhow::Error::new)?;
    let prepared = configured.prepared;
    let roots = prepared.report.summary.roots.clone();
    Ok(WorkflowScan {
        #[cfg(test)]
        config: configured.config,
        recommendation_policy: prepared.recommendation_policy,
        ruleset_versions: prepared.ruleset_versions,
        roots,
        explicit_roots: prepared.explicit_roots,
        global_scan: prepared.global_scan,
        analysis: prepared.analysis,
        safety_policy: prepared.safety_policy,
        report: prepared.report,
    })
}

fn build_analysis_report<'a>(
    scan: &'a WorkflowScan,
    safety: &SafetyPolicy,
) -> Result<Cow<'a, AnalysisReport>> {
    if let Some(analysis) = &scan.analysis {
        return Ok(Cow::Borrowed(analysis));
    }
    Ok(Cow::Owned(build_workflow_analysis(
        &scan.report,
        scan.recommendation_policy.clone(),
        safety,
        &scan.explicit_roots,
        &scan.global_scan,
    )?))
}

fn inactivity_qualified_candidate_count(scan: &WorkflowScan) -> Result<usize> {
    let analysis = build_analysis_report(scan, &scan.safety_policy)?;
    Ok(analysis
        .candidates
        .iter()
        .filter(|candidate| {
            !matches!(
                candidate.recommendation.state,
                RecommendationState::Suppressed | RecommendationState::Excluded
            ) && analysis
                .policy
                .activity_meets_inactivity_threshold(&candidate.activity)
        })
        .count())
}

enum PlanSelection {
    Recommendations(SelectionOverrides),
    #[cfg(test)]
    ExactPaths(Vec<PathBuf>),
}

fn build_plan(scan: &WorkflowScan, requested_selection: PlanSelection) -> Result<CleanupPlan> {
    if !scan.report.budget_exceeded.is_empty() {
        bail!("scan budget was exceeded; partial evidence cannot produce a cleanup plan");
    }
    let analysis = build_analysis_report(scan, &scan.safety_policy)?;
    let selection = match requested_selection {
        PlanSelection::Recommendations(overrides) => {
            selection_with_overrides(analysis.as_ref(), overrides)?
        }
        #[cfg(test)]
        PlanSelection::ExactPaths(paths) => exact_selection(analysis.as_ref(), &paths)?,
    };
    let plan = build_workflow_plan(
        scan.roots.clone(),
        scan.ruleset_versions.clone(),
        &scan.report.entries,
        analysis.as_ref(),
        &selection,
        &scan.safety_policy,
        &scan.explicit_roots,
        &scan.global_scan,
    )
    .context("failed to build cleanup plan from analysis")?;
    Ok(plan)
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

#[cfg(test)]
fn safety_policy(config: &Config, config_path: Option<PathBuf>) -> SafetyPolicy {
    cleanr_tasks::safety_policy_for_config(config, config_path, default_state_dir())
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
    use chrono::Utc;
    use cleanr_core::{
        CleanupItem, CleanupItemFingerprint, CleanupPlanScanScope, Confidence, DecisionCode,
        EntryKind, GlobalScanKind, PlanSummary, PlannedAction, ReportIntegrity, RuleHit,
        RuleMatchRole, RuleTrust, ScanBudgetExceeded, ScanEntry, ScanIssue, ScanIssueCode,
    };
    use cleanr_fs::{GlobalScanRoot, ResolvedScanRoots, ScanError, global_scan_evidence};
    use cleanr_rules::RuleRegistry;
    use cleanr_tasks::semantic_explicit_roots;

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
    fn request_inactivity_override_wins_without_mutating_config() {
        let mut config = Config::default();
        config.recommendations.preselect_after_days = 180;

        let configured = recommendation_policy(&config, None).expect("configured policy");
        let overridden = recommendation_policy(&config, Some(30)).expect("one-scan override");

        assert_eq!(configured.preselect_after_days, 180);
        assert_eq!(overridden.preselect_after_days, 30);
        assert_eq!(config.recommendations.preselect_after_days, 180);
        assert!(recommendation_policy(&config, Some(3651)).is_err());
    }

    #[test]
    fn default_local_scope_materializes_the_resolved_working_directory() {
        let original_working_directory = PathBuf::from("/original/working-directory");
        let request = ScanRequest::default();
        let resolved = ResolvedScanRoots {
            roots: vec![original_working_directory.clone()],
            ..ResolvedScanRoots::default()
        };

        let scope = CleanupPlanScanScope::new(
            semantic_explicit_roots(&request, &resolved.roots),
            Vec::new(),
        );
        let rescan_request = scope.to_scan_request();

        assert_eq!(scope.explicit_roots, vec![original_working_directory]);
        assert_eq!(rescan_request.paths, scope.explicit_roots);
        assert!(!rescan_request.include_global);
    }

    #[test]
    fn effective_policy_filters_summary_and_plan_but_preserves_raw_evidence() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let inactive_path = temp.path().join("inactive-cache");
        let recent_path = temp.path().join("recent-cache");
        std::fs::create_dir(&inactive_path)?;
        std::fs::create_dir(&recent_path)?;
        let as_of = Utc::now();
        let hit = |rule_id: &str| RuleHit {
            rule_pack_id: "builtin-test".to_string(),
            rule_id: rule_id.to_string(),
            label: rule_id.to_string(),
            category: "cache".to_string(),
            confidence: Confidence::High,
            reason: "rebuildable".to_string(),
            risk_note: "rebuild".to_string(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
        };
        let mut config = Config::default();
        config.recommendations.preselect_after_days = 180;
        let scan = WorkflowScan {
            config: config.clone(),
            recommendation_policy: RecommendationPolicy::new(30)?,
            ruleset_versions: RuleRegistry::builtin()?.versions(),
            roots: vec![temp.path().to_path_buf()],
            explicit_roots: vec![temp.path().to_path_buf()],
            global_scan: GlobalScanEvidence::default(),
            analysis: None,
            safety_policy: SafetyPolicy::default(),
            report: ScanReport {
                as_of,
                entries: vec![
                    ScanEntry {
                        path: inactive_path.clone(),
                        kind: EntryKind::Directory,
                        size_bytes: 10,
                        modified_at: Some(as_of - chrono::Duration::days(31)),
                        rule_hits: vec![hit("inactive")],
                    },
                    ScanEntry {
                        path: recent_path.clone(),
                        kind: EntryKind::Directory,
                        size_bytes: 20,
                        modified_at: Some(as_of - chrono::Duration::days(29)),
                        rule_hits: vec![hit("recent")],
                    },
                ],
                ..ScanReport::default()
            },
        };

        let safety = safety_policy(&config, None);
        let analysis = build_analysis_report(&scan, &safety)?;
        assert_eq!(analysis.policy.preselect_after_days, 30);
        assert_eq!(analysis.candidates.len(), 2);
        assert_eq!(
            scan_json_value(&scan.report)["entries"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(inactivity_qualified_candidate_count(&scan)?, 1);

        let recommended = build_plan(
            &scan,
            PlanSelection::Recommendations(SelectionOverrides::default()),
        )?;
        assert_eq!(recommended.summary.candidate_count, 1);
        assert_eq!(recommended.items[0].path, inactive_path);
        assert_eq!(
            recommended
                .source_scan
                .as_ref()
                .and_then(|source| source.recommendation_policy.as_ref()),
            Some(&scan.recommendation_policy)
        );

        let explicitly_selected = build_plan(
            &scan,
            PlanSelection::Recommendations(SelectionOverrides {
                select_paths: vec![recent_path.clone()],
                deselect_paths: Vec::new(),
            }),
        )?;
        assert_eq!(explicitly_selected.summary.candidate_count, 2);
        assert!(
            explicitly_selected
                .items
                .iter()
                .any(|item| item.path == recent_path && item.selected)
        );
        Ok(())
    }

    #[test]
    fn saved_policy_override_is_reused_exactly_for_rescan() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config_path = temp.path().join("config.toml");
        let mut config = Config::default();
        config.recommendations.preselect_after_days = 180;
        config.save_to(&config_path)?;
        let saved_policy = RecommendationPolicy {
            version: "v1".to_string(),
            preselect_after_days: 30,
            strict_report_integrity: false,
        };

        let scan = run_scan_with_recommendation_policy(
            Some(config_path.clone()),
            ScanRequest::paths(vec![temp.path().to_path_buf()]),
            ScanPreparationMode::Planning,
            Some(saved_policy.clone()),
        )?;

        assert_eq!(scan.recommendation_policy, saved_policy);
        assert_eq!(scan.config.recommendations.preselect_after_days, 180);
        let unknown_policy = RecommendationPolicy {
            version: "future-v99".to_string(),
            ..RecommendationPolicy::default()
        };
        let error = run_scan_with_recommendation_policy(
            Some(config_path),
            ScanRequest::paths(vec![temp.path().to_path_buf()]),
            ScanPreparationMode::Planning,
            Some(unknown_policy),
        )
        .err()
        .context("unknown saved policy must fail closed")?;
        assert!(
            error
                .to_string()
                .contains("unsupported recommendation policy")
        );
        Ok(())
    }

    #[test]
    fn policyless_v1_plan_rebuild_keeps_recent_unselected_candidates() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let recent_path = root.join("recent-cache");
        let inactive_path = root.join("inactive-cache");
        std::fs::create_dir(&recent_path)?;
        std::fs::create_dir(&inactive_path)?;
        let as_of = Utc::now();
        let hit = RuleHit {
            rule_pack_id: "builtin-test".to_string(),
            rule_id: "cache".to_string(),
            label: "Cache".to_string(),
            category: "cache".to_string(),
            confidence: Confidence::High,
            reason: "rebuildable".to_string(),
            risk_note: "rebuild".to_string(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
        };
        let entries = vec![
            ScanEntry {
                path: recent_path.clone(),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: Some(as_of - chrono::Duration::days(1)),
                rule_hits: vec![hit.clone()],
            },
            ScanEntry {
                path: inactive_path.clone(),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit],
            },
        ];
        let make_scan = |recommendation_policy: RecommendationPolicy| -> Result<WorkflowScan> {
            Ok(WorkflowScan {
                config: Config::default(),
                recommendation_policy,
                ruleset_versions: RuleRegistry::builtin()?.versions(),
                roots: vec![root.clone()],
                explicit_roots: vec![root.clone()],
                global_scan: GlobalScanEvidence::default(),
                analysis: None,
                safety_policy: SafetyPolicy::default(),
                report: ScanReport {
                    as_of,
                    entries: entries.clone(),
                    ..ScanReport::default()
                },
            })
        };
        let legacy_policy = RecommendationPolicy {
            version: "v1".to_string(),
            ..RecommendationPolicy::default()
        };
        let legacy_scan = make_scan(legacy_policy)?;
        let mut expected = build_plan(
            &legacy_scan,
            PlanSelection::Recommendations(SelectionOverrides::default()),
        )?;
        assert_eq!(expected.summary.candidate_count, 2);
        assert_eq!(expected.summary.selected_count, 1);
        let source = expected
            .source_scan
            .as_mut()
            .context("analysis-backed plan")?;
        source.recommendation_policy = None;
        source.scope = None;

        let mut current_scan = make_scan(RecommendationPolicy::default())?;
        current_scan.recommendation_policy =
            recommendation_policy_for_plan_rescan(&current_scan.config, &expected)?;
        assert_eq!(current_scan.recommendation_policy.version, "v1");
        let current = build_plan(
            &current_scan,
            PlanSelection::ExactPaths(vec![inactive_path]),
        )?;

        ensure_plan_unchanged(&expected, &current)?;
        assert!(
            current
                .items
                .iter()
                .any(|item| item.path == recent_path && !item.selected)
        );
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
            os_managed: Vec::new(),
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
            recommendation_policy: RecommendationPolicy::default(),
            ruleset_versions: RuleRegistry::builtin()
                .expect("builtin registry")
                .versions(),
            roots: resolved.roots,
            explicit_roots: Vec::new(),
            global_scan,
            analysis: None,
            safety_policy: SafetyPolicy::default(),
            report: ScanReport::default(),
        };

        let report =
            build_analysis_report(&scan, &safety_policy(&config, None)).expect("analysis report");
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
            os_managed: Vec::new(),
        };
        let global_scan = global_scan_evidence(&request, &[], &resolved, &resolved.roots);
        let as_of = Utc::now();
        let config = Config::default();
        let scan = WorkflowScan {
            config: config.clone(),
            recommendation_policy: RecommendationPolicy::default(),
            ruleset_versions: RuleRegistry::builtin()
                .expect("builtin registry")
                .versions(),
            roots: resolved.roots,
            explicit_roots: Vec::new(),
            global_scan,
            analysis: None,
            safety_policy: SafetyPolicy::default(),
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
                        sources: Vec::new(),
                    }],
                }],
                ..ScanReport::default()
            },
        };

        let analysis =
            build_analysis_report(&scan, &safety_policy(&config, None)).expect("analysis report");

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
        let scope = plan
            .source_scan
            .as_ref()
            .and_then(|source| source.scope.as_ref())
            .expect("plan retains semantic global scope");
        assert!(scope.explicit_roots.is_empty());
        assert_eq!(scope.global_kinds, vec![GlobalScanKind::AppCaches]);
        let rescan_request = scope.to_scan_request();
        assert!(rescan_request.include_global);
        assert!(rescan_request.paths.is_empty());
        assert_eq!(rescan_request.global_kinds, vec![GlobalScanKind::AppCaches]);
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
            recommendation_policy: RecommendationPolicy::default(),
            ruleset_versions: RuleRegistry::builtin()
                .expect("builtin registry")
                .versions(),
            roots,
            explicit_roots: vec![PathBuf::from("/tmp/project")],
            global_scan: evidence.clone(),
            analysis: None,
            safety_policy: SafetyPolicy::default(),
            report: ScanReport::default(),
        };
        let report =
            build_analysis_report(&scan, &safety_policy(&config, None)).expect("analysis report");
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
            recommendation_policy: RecommendationPolicy::default(),
            ruleset_versions: RuleRegistry::builtin()
                .expect("builtin registry")
                .versions(),
            roots: Vec::new(),
            explicit_roots: Vec::new(),
            global_scan,
            analysis: None,
            safety_policy: SafetyPolicy::default(),
            report: ScanReport::default(),
        };

        let report =
            build_analysis_report(&scan, &safety_policy(&config, None)).expect("analysis report");
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

        let mut changed_policy = current.clone();
        changed_policy
            .source_scan
            .as_mut()
            .context("current plan should retain source scan provenance")?
            .recommendation_policy = Some(RecommendationPolicy::new(30)?);
        assert!(ensure_plan_unchanged(&expected, &changed_policy).is_err());

        let mut changed_scope = current.clone();
        changed_scope
            .source_scan
            .as_mut()
            .context("current plan should retain source scan provenance")?
            .scope = Some(CleanupPlanScanScope::new(
            Vec::new(),
            vec![GlobalScanKind::BrowserCaches],
        ));
        assert!(ensure_plan_unchanged(&expected, &changed_scope).is_err());

        let mut policyless_expected = expected.clone();
        policyless_expected
            .source_scan
            .as_mut()
            .context("expected plan should retain source scan provenance")?
            .recommendation_policy = None;
        ensure_plan_unchanged(&policyless_expected, &current)?;

        current.summary.selected_count = 1;
        let error = ensure_plan_unchanged(&expected, &current)
            .err()
            .context("changed plan must be rejected")?;
        assert!(error.to_string().contains("changed after authorization"));
        Ok(())
    }

    #[test]
    fn plan_revalidation_ignores_unselected_candidate_drift() -> Result<()> {
        let mut expected = empty_analysis_backed_plan()?;
        let selected = CleanupItem {
            path: PathBuf::from("/repo/selected-cache"),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: None,
            tree_fingerprint: Some(CleanupItemFingerprint {
                descendants: 1,
                total_size_bytes: 10,
                latest_modified_at: None,
            }),
            rule_id: "builtin-test:selected".to_string(),
            category: "cache".to_string(),
            confidence: Confidence::High,
            reason: "selected candidate".to_string(),
            risk_note: "rebuildable".to_string(),
            evidence: None,
            selected: true,
            planned_action: PlannedAction::Trash,
            rollback_method: "system-trash+manifest".to_string(),
        };
        let mut unselected = selected.clone();
        unselected.path = PathBuf::from("/repo/unselected-cache");
        unselected.size_bytes = 20;
        unselected.tree_fingerprint = Some(CleanupItemFingerprint {
            descendants: 2,
            total_size_bytes: 20,
            latest_modified_at: None,
        });
        unselected.reason = "unselected candidate".to_string();
        unselected.selected = false;
        expected.items = vec![selected, unselected];
        expected.summary = PlanSummary {
            candidate_count: 2,
            selected_count: 1,
            selected_size_bytes: 10,
            total_candidate_size_bytes: 30,
        };

        let mut changed_unselected = expected.clone();
        changed_unselected.items[1].size_bytes = 25;
        changed_unselected.items[1].reason = "updated unrelated evidence".to_string();
        changed_unselected.summary.total_candidate_size_bytes = 35;
        ensure_plan_unchanged(&expected, &changed_unselected)?;

        let mut added_unselected = changed_unselected.clone();
        let mut new_candidate = added_unselected.items[1].clone();
        new_candidate.path = PathBuf::from("/repo/new-unselected-cache");
        new_candidate.size_bytes = 5;
        added_unselected.items.push(new_candidate);
        added_unselected.summary.candidate_count = 3;
        added_unselected.summary.total_candidate_size_bytes = 40;
        ensure_plan_unchanged(&expected, &added_unselected)?;

        let mut removed_unselected = expected.clone();
        removed_unselected.items.retain(|item| item.selected);
        removed_unselected.summary.candidate_count = 1;
        removed_unselected.summary.total_candidate_size_bytes = 10;
        ensure_plan_unchanged(&expected, &removed_unselected)?;
        Ok(())
    }

    #[test]
    fn plan_revalidation_rejects_selected_target_drift() -> Result<()> {
        let mut expected = empty_analysis_backed_plan()?;
        expected.items.push(CleanupItem {
            path: PathBuf::from("/repo/selected-cache"),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: None,
            tree_fingerprint: Some(CleanupItemFingerprint {
                descendants: 1,
                total_size_bytes: 10,
                latest_modified_at: None,
            }),
            rule_id: "builtin-test:selected".to_string(),
            category: "cache".to_string(),
            confidence: Confidence::High,
            reason: "selected candidate".to_string(),
            risk_note: "rebuildable".to_string(),
            evidence: None,
            selected: true,
            planned_action: PlannedAction::Trash,
            rollback_method: "system-trash+manifest".to_string(),
        });
        expected.summary = PlanSummary {
            candidate_count: 1,
            selected_count: 1,
            selected_size_bytes: 10,
            total_candidate_size_bytes: 10,
        };

        let mut changed_target = expected.clone();
        changed_target.items[0]
            .tree_fingerprint
            .as_mut()
            .context("directory fingerprint")?
            .descendants = 2;
        assert!(ensure_plan_unchanged(&expected, &changed_target).is_err());

        let mut changed_selected_summary = expected.clone();
        changed_selected_summary.summary.selected_size_bytes = 11;
        assert!(ensure_plan_unchanged(&expected, &changed_selected_summary).is_err());
        Ok(())
    }

    #[test]
    fn rescan_allows_unselected_filesystem_drift_but_rejects_selected_tree_drift() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let selected_path = root.join("selected-project/node_modules");
        let unselected_path = root.join("unselected-project/node_modules");
        std::fs::create_dir_all(&selected_path)?;
        std::fs::create_dir_all(&unselected_path)?;
        std::fs::File::create(selected_path.join("payload"))?.set_len(1_048_576)?;
        std::fs::File::create(unselected_path.join("payload"))?.set_len(1_048_576)?;
        let request = ScanRequest {
            inactive_days: Some(0),
            ..ScanRequest::paths(vec![root])
        };

        let initial_scan = run_scan(None, request.clone(), ScanPreparationMode::Planning)?;
        let expected = build_plan(
            &initial_scan,
            PlanSelection::Recommendations(SelectionOverrides {
                select_paths: Vec::new(),
                deselect_paths: vec![unselected_path.clone()],
            }),
        )?;
        assert!(
            expected
                .items
                .iter()
                .any(|item| item.path == selected_path && item.selected)
        );
        assert!(
            expected
                .items
                .iter()
                .any(|item| item.path == unselected_path && !item.selected)
        );

        std::fs::OpenOptions::new()
            .write(true)
            .open(unselected_path.join("payload"))?
            .set_len(1_048_577)?;
        let unselected_drift_scan = run_scan(None, request.clone(), ScanPreparationMode::Planning)?;
        let unselected_drift_plan = build_plan(
            &unselected_drift_scan,
            PlanSelection::ExactPaths(vec![selected_path.clone()]),
        )?;
        ensure_plan_unchanged(&expected, &unselected_drift_plan)?;

        std::fs::File::create(selected_path.join("new-payload"))?.set_len(1)?;
        let selected_drift_scan = run_scan(None, request, ScanPreparationMode::Planning)?;
        let selected_drift_plan = build_plan(
            &selected_drift_scan,
            PlanSelection::ExactPaths(vec![selected_path]),
        )?;
        assert!(ensure_plan_unchanged(&expected, &selected_drift_plan).is_err());
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
            sources: Vec::new(),
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
            recommendation_policy: RecommendationPolicy::default(),
            ruleset_versions: RuleRegistry::builtin()?.versions(),
            roots: vec![PathBuf::from("/repo")],
            explicit_roots: vec![PathBuf::from("/repo")],
            global_scan: GlobalScanEvidence::default(),
            analysis: None,
            safety_policy: SafetyPolicy::default(),
            report: ScanReport::default(),
        };
        build_plan(
            &scan,
            PlanSelection::Recommendations(SelectionOverrides::default()),
        )
    }
}
