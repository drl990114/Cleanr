#![forbid(unsafe_code)]

#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
use chrono::Utc;

mod evidence;
mod manifests;
mod model;
mod planning;
mod safety;

pub use model::{
    CLEANUP_PLAN_SCHEMA_VERSION, CleanupItem, CleanupItemEvidence, CleanupItemFingerprint,
    CleanupPlan, CleanupPlanBuildError, CleanupPlanScanScope, CleanupPlanSourceScan, Confidence,
    EXECUTION_SCHEMA_VERSION, EntryKind, GlobalScanKind, PlanSafety, PlanSummary, PlannedAction,
    RESTORE_SCHEMA_VERSION, RuleHit, RuleMatchRole, RulePlatform, RuleSource, RuleSourceRelation,
    RuleTrust, RulesetVersion, ScanEntry, ScanLocationBase, ScanLocationDefinition,
    ScanLocationExpansion, ScanLocationMode, ScanLocationPack, ScanRequest, ScanSummary,
    default_global_scan_kinds,
};

pub use evidence::{
    ANALYSIS_REPORT_SCHEMA_VERSION, ActivityEvidence, ActivitySource, ActivityStatus, AnalysisId,
    AnalysisReport, AnalysisScanContext, CandidateCoverage, CandidateEvidence, CandidateId,
    DecisionCode, GlobalManagedLocationEvidence, GlobalScanEvidence, GlobalScanLocationEvidence,
    MAX_RECOMMENDATION_AGE_DAYS, OverlapEvidence, RecommendationDecision, RecommendationPolicy,
    RecommendationPolicyError, RecommendationState, ReportIntegrity, RuleEvidence, RuleKey,
    RuleResolution, RuleResolutionState, RuntimeGuardEvidence, RuntimeGuardState,
    ScanBudgetExceeded, ScanBudgetLimits, ScanEvidence, ScanIssue, ScanIssueCode, UserSelection,
    build_analysis_report, build_analysis_report_with_budget,
    build_analysis_report_with_safety_policy, build_analysis_report_with_scan_context,
    suppress_unrequested_global_candidates,
};

pub use manifests::{
    CleanupAuthorizationSource, ExecutionAuthorization, ExecutionItem, ExecutionManifest,
    ExecutionStatus, ExecutionSummary, RestoreItem, RestoreManifest, RestoreStatus, RestoreSummary,
    RollbackReceipt,
};

pub use safety::SafetyPolicy;

#[allow(deprecated)]
pub use planning::{
    build_cleanup_plan, build_cleanup_plan_from_analysis, build_cleanup_plan_with_policy,
};

pub(crate) use planning::merge_path_forest;

#[cfg(test)]
use planning::{TreeFingerprintFacts, tree_fingerprints};

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn plan_selects_only_high_confidence_defaults() {
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("node_modules"),
                kind: EntryKind::Directory,
                size_bytes: 10,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "builtin-dev".into(),
                    rule_id: "node-modules".into(),
                    label: "Node modules".into(),
                    category: "developer-cache".into(),
                    confidence: Confidence::High,
                    reason: "reinstallable dependency cache".into(),
                    risk_note: "can be restored by package manager".into(),
                    default_selected: true,
                    trust: RuleTrust::Builtin,
                    match_role: RuleMatchRole::Primary,
                    sources: Vec::new(),
                    runtime_guard: None,
                }],
            },
            ScanEntry {
                path: PathBuf::from("Downloads/big.zip"),
                kind: EntryKind::File,
                size_bytes: 20,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "builtin-general".into(),
                    rule_id: "large-download".into(),
                    label: "Large download".into(),
                    category: "downloads".into(),
                    confidence: Confidence::Low,
                    reason: "large download".into(),
                    risk_note: "user data review required".into(),
                    default_selected: false,
                    trust: RuleTrust::Builtin,
                    match_role: RuleMatchRole::Primary,
                    sources: Vec::new(),
                    runtime_guard: None,
                }],
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from(".")], vec![], &entries);
        assert_eq!(plan.summary.candidate_count, 2);
        assert_eq!(plan.summary.selected_count, 1);
        assert!(plan.items.iter().any(|item| item.selected));
        assert!(plan.source_scan.is_none());
    }

    #[test]
    fn analysis_plan_uses_the_same_ninety_day_candidate_boundary() {
        let as_of = Utc::now();
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "rebuildable".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let entries = [89_i64, 90, 91, 1]
            .into_iter()
            .map(|days| ScanEntry {
                path: PathBuf::from(format!("/repo/cache-{days}")),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: Some(as_of - chrono::Duration::days(days)),
                rule_hits: vec![hit.clone()],
            })
            .collect::<Vec<_>>();
        let safety = SafetyPolicy::default();
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            RecommendationPolicy::default(),
            &safety,
        )
        .expect("default policy is valid");
        let selection = UserSelection::from_recommendations(&analysis);
        let plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &selection,
            &safety,
        )
        .expect("supported complete analysis");

        let source_scan = plan
            .source_scan
            .as_ref()
            .expect("analysis plan retains scan provenance");
        assert_eq!(source_scan.analysis_id, analysis.analysis_id);
        assert_eq!(source_scan.integrity, analysis.scan.integrity);
        assert!(source_scan.budget_exceeded.is_empty());
        assert_eq!(
            source_scan.recommendation_policy.as_ref(),
            Some(&analysis.policy)
        );

        let item = |path: &str| {
            plan.items
                .iter()
                .find(|item| item.path == Path::new(path))
                .expect("inactive candidate appears in plan")
        };
        assert_eq!(plan.summary.candidate_count, 2);
        assert!(plan.items.iter().all(|item| item.selected));
        assert!(
            !plan
                .items
                .iter()
                .any(|item| item.path == Path::new("/repo/cache-89"))
        );
        assert!(item("/repo/cache-90").selected);
        assert!(item("/repo/cache-91").selected);
        assert!(
            !plan
                .items
                .iter()
                .any(|item| item.path == Path::new("/repo/cache-1"))
        );
        let evidence = plan.items[0]
            .evidence
            .as_ref()
            .expect("analysis plan keeps recommendation evidence");
        assert_eq!(evidence.matched_rules.len(), 1);
        assert_eq!(evidence.rule_resolution_state, RuleResolutionState::Single);
        assert!(!evidence.decision_codes.is_empty());
    }

    #[test]
    fn v1_analysis_plan_keeps_below_threshold_candidates_for_legacy_validation() {
        let as_of = Utc::now();
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "rebuildable".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let entries = [1_i64, 100]
            .into_iter()
            .map(|days| ScanEntry {
                path: PathBuf::from(format!("/repo/cache-{days}")),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: Some(as_of - chrono::Duration::days(days)),
                rule_hits: vec![hit.clone()],
            })
            .collect::<Vec<_>>();
        let safety = SafetyPolicy::default();
        let legacy_policy = RecommendationPolicy {
            version: "v1".to_string(),
            ..RecommendationPolicy::default()
        };
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            legacy_policy,
            &safety,
        )
        .expect("v1 policy remains supported");
        let plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            Vec::new(),
            &entries,
            &analysis,
            &UserSelection::from_recommendations(&analysis),
            &safety,
        )
        .expect("v1 plan projection");

        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.summary.selected_count, 1);
        assert!(
            plan.items
                .iter()
                .any(|item| item.path == Path::new("/repo/cache-1") && !item.selected)
        );
    }

    #[test]
    fn explicit_selection_can_include_a_recent_review_candidate() {
        let as_of = Utc::now();
        let recent_path = PathBuf::from("/repo/recent-cache");
        let entries = vec![ScanEntry {
            path: recent_path.clone(),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: Some(as_of - chrono::Duration::days(1)),
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "cache".into(),
                label: "Cache".into(),
                category: "developer-cache".into(),
                confidence: Confidence::High,
                reason: "rebuildable".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: RuleTrust::Builtin,
                match_role: RuleMatchRole::Primary,
                sources: Vec::new(),
                runtime_guard: None,
            }],
        }];
        let safety = SafetyPolicy::default();
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            RecommendationPolicy::default(),
            &safety,
        )
        .expect("default policy is valid");

        let default_plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &UserSelection::from_recommendations(&analysis),
            &safety,
        )
        .expect("default plan");
        assert!(default_plan.items.is_empty());

        let mut explicit_selection = UserSelection::default();
        explicit_selection.select(analysis.candidates[0].id.clone());
        let explicit_plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &explicit_selection,
            &safety,
        )
        .expect("explicit plan");
        assert_eq!(explicit_plan.items.len(), 1);
        assert!(explicit_plan.items[0].selected);
        assert_eq!(explicit_plan.items[0].path, recent_path);
    }

    #[test]
    fn analysis_plan_builder_rejects_budget_exhaustion_and_unknown_schemas() {
        let as_of = Utc::now();
        let entries = vec![ScanEntry {
            path: PathBuf::from("/repo/cache"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: Some(as_of - chrono::Duration::days(100)),
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "cache".into(),
                label: "Cache".into(),
                category: "developer-cache".into(),
                confidence: Confidence::High,
                reason: "rebuildable".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: RuleTrust::Builtin,
                match_role: RuleMatchRole::Primary,
                sources: Vec::new(),
                runtime_guard: None,
            }],
        }];
        let safety = SafetyPolicy::default();
        let budget = [ScanBudgetExceeded::EntryCount {
            limit: 1,
            observed: 2,
        }];
        let analysis = build_analysis_report_with_budget(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            &budget,
            RecommendationPolicy::default(),
        )
        .expect("valid policy");

        let error = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &UserSelection::default(),
            &safety,
        )
        .expect_err("budget analysis must not create a plan");
        assert_eq!(error, CleanupPlanBuildError::ScanBudgetExceeded);

        let mut future_analysis = analysis;
        future_analysis.scan.budget_exceeded.clear();
        future_analysis.schema_version = "cleanr.analysis.v999".to_string();
        let error = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &future_analysis,
            &UserSelection::default(),
            &safety,
        )
        .expect_err("unknown analysis schemas must fail closed");
        assert_eq!(
            error,
            CleanupPlanBuildError::UnsupportedAnalysisSchema {
                found: "cleanr.analysis.v999".to_string(),
            }
        );
    }

    #[test]
    fn analysis_plan_does_not_let_a_safety_excluded_parent_hide_a_safe_child() {
        let as_of = Utc::now();
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "rebuildable".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 10,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit.clone()],
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/child"),
                kind: EntryKind::Directory,
                size_bytes: 5,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit],
            },
        ];
        let safety = SafetyPolicy::new(vec![PathBuf::from("/repo/cache/protected")], true);
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            RecommendationPolicy::default(),
            &safety,
        )
        .expect("default policy is valid");
        let parent = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache"))
            .expect("parent candidate");
        let child = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache/child"))
            .expect("child candidate");
        assert_eq!(parent.recommendation.state, RecommendationState::Excluded);
        assert_ne!(child.recommendation.state, RecommendationState::Suppressed);

        let selection = UserSelection::from_recommendations(&analysis);
        let plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &selection,
            &safety,
        )
        .expect("supported complete analysis");
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].path, PathBuf::from("/repo/cache/child"));
        assert!(plan.items[0].selected);
    }

    #[test]
    fn plan_rule_ties_use_a_stable_rule_key_not_input_order() {
        let hit = |rule_id: &str| RuleHit {
            rule_pack_id: "builtin-dev".to_string(),
            rule_id: rule_id.to_string(),
            label: rule_id.to_string(),
            category: "developer-cache".to_string(),
            confidence: Confidence::High,
            reason: "equivalent generated cache".to_string(),
            risk_note: "rebuild".to_string(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let make_entry = |rule_hits| ScanEntry {
            path: PathBuf::from("/repo/cache"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: None,
            rule_hits,
        };

        let forward = build_cleanup_plan(
            vec![PathBuf::from("/repo")],
            vec![],
            &[make_entry(vec![hit("z-rule"), hit("a-rule")])],
        );
        let reverse = build_cleanup_plan(
            vec![PathBuf::from("/repo")],
            vec![],
            &[make_entry(vec![hit("a-rule"), hit("z-rule")])],
        );

        assert_eq!(forward.items[0].rule_id, "builtin-dev:a-rule");
        assert_eq!(forward.items[0].rule_id, reverse.items[0].rule_id);
    }

    #[test]
    fn plan_serializes_as_json_manifest() {
        let plan = build_cleanup_plan(vec![PathBuf::from(".")], vec![], &[]);
        let json = serde_json::to_string(&plan).expect("plan serializes");
        assert!(json.contains(CLEANUP_PLAN_SCHEMA_VERSION));
        assert!(json.contains("system-trash+manifest"));
        assert!(!json.contains("source_scan"));

        let restored: CleanupPlan = serde_json::from_str(&json).expect("legacy plan deserializes");
        assert!(restored.source_scan.is_none());
    }

    #[test]
    fn plan_removes_overlapping_parent_and_child_candidates() {
        let hit = |rule_id: &str| RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: rule_id.into(),
            label: rule_id.into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "generated".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/node_modules"),
                kind: EntryKind::Directory,
                size_bytes: 100,
                modified_at: None,
                rule_hits: vec![hit("node-modules")],
            },
            ScanEntry {
                path: PathBuf::from("/repo/node_modules/pkg/node_modules"),
                kind: EntryKind::Directory,
                size_bytes: 40,
                modified_at: None,
                rule_hits: vec![hit("node-modules")],
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &entries);
        assert_eq!(plan.summary.candidate_count, 1);
        assert_eq!(plan.summary.selected_size_bytes, 100);
        assert_eq!(plan.items[0].path, PathBuf::from("/repo/node_modules"));
    }

    #[test]
    fn directory_candidates_include_tree_fingerprints() {
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "generated".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let modified_at = Utc::now();
        let newer_modified_at = modified_at + chrono::Duration::seconds(1);
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 7,
                modified_at: Some(modified_at),
                rule_hits: vec![hit],
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/file"),
                kind: EntryKind::File,
                size_bytes: 3,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/nested"),
                kind: EntryKind::Directory,
                size_bytes: 4,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/nested/file"),
                kind: EntryKind::File,
                size_bytes: 4,
                modified_at: Some(newer_modified_at),
                rule_hits: Vec::new(),
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &entries);

        assert_eq!(
            plan.items[0].tree_fingerprint,
            Some(CleanupItemFingerprint {
                descendants: 3,
                total_size_bytes: 7,
                latest_modified_at: Some(newer_modified_at),
            })
        );
    }

    #[test]
    fn tree_fingerprints_are_order_independent_across_multiple_roots() {
        let modified_at = Utc::now();
        let newer_modified_at = modified_at + chrono::Duration::seconds(1);
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/first"),
                kind: EntryKind::Directory,
                size_bytes: 5,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/first/nested"),
                kind: EntryKind::Directory,
                size_bytes: 5,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/first/nested/file"),
                kind: EntryKind::File,
                size_bytes: 5,
                modified_at: Some(newer_modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/second"),
                kind: EntryKind::Directory,
                size_bytes: 7,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/second/file"),
                kind: EntryKind::File,
                size_bytes: 7,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
        ];
        let reversed = entries.iter().cloned().rev().collect::<Vec<_>>();

        let forward = tree_fingerprints(&entries);
        let backward = tree_fingerprints(&reversed);

        assert_eq!(forward, backward);
        assert_eq!(
            forward.get(Path::new("/first")),
            Some(&CleanupItemFingerprint {
                descendants: 2,
                total_size_bytes: 5,
                latest_modified_at: Some(newer_modified_at),
            })
        );
        assert_eq!(
            forward.get(Path::new("/second")),
            Some(&CleanupItemFingerprint {
                descendants: 1,
                total_size_bytes: 7,
                latest_modified_at: Some(modified_at),
            })
        );
    }

    #[test]
    fn tree_fingerprints_stop_at_a_missing_immediate_parent() {
        let root_modified_at = Utc::now();
        let descendant_modified_at = root_modified_at + chrono::Duration::seconds(1);
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 5,
                modified_at: Some(root_modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/missing/file"),
                kind: EntryKind::File,
                size_bytes: 5,
                modified_at: Some(descendant_modified_at),
                rule_hits: Vec::new(),
            },
        ];

        let fingerprints = tree_fingerprints(&entries);

        assert_eq!(
            fingerprints.get(Path::new("/repo/cache")),
            Some(&CleanupItemFingerprint {
                descendants: 0,
                total_size_bytes: 5,
                latest_modified_at: Some(root_modified_at),
            })
        );
    }

    #[test]
    fn tree_fingerprint_descendant_count_saturates() {
        let mut parent = TreeFingerprintFacts {
            descendants: usize::MAX - 1,
            latest_modified_at: None,
        };
        parent.absorb_descendant(&TreeFingerprintFacts {
            descendants: 1,
            latest_modified_at: None,
        });

        assert_eq!(parent.descendants, usize::MAX);
    }

    #[test]
    fn selected_child_wins_over_unselected_parent() {
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 100,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "custom".into(),
                    rule_id: "review-parent".into(),
                    label: "review".into(),
                    category: "cache".into(),
                    confidence: Confidence::Medium,
                    reason: "review".into(),
                    risk_note: "review".into(),
                    default_selected: false,
                    trust: RuleTrust::Trusted,
                    match_role: RuleMatchRole::Primary,
                    sources: Vec::new(),
                    runtime_guard: None,
                }],
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/generated"),
                kind: EntryKind::Directory,
                size_bytes: 40,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "custom".into(),
                    rule_id: "safe-child".into(),
                    label: "safe".into(),
                    category: "cache".into(),
                    confidence: Confidence::High,
                    reason: "generated".into(),
                    risk_note: "rebuild".into(),
                    default_selected: true,
                    trust: RuleTrust::Trusted,
                    match_role: RuleMatchRole::Primary,
                    sources: Vec::new(),
                    runtime_guard: None,
                }],
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &entries);
        assert_eq!(plan.summary.candidate_count, 1);
        assert_eq!(plan.items[0].path, PathBuf::from("/repo/cache/generated"));
        assert!(plan.items[0].selected);
    }

    #[test]
    fn protected_subtrees_reject_their_children_and_parents() {
        let policy = SafetyPolicy::new(vec![], true)
            .with_protected_subtrees(vec![PathBuf::from("/repo/.cleanr")]);

        assert!(!policy.allows_candidate(std::path::Path::new("/repo")));
        assert!(!policy.allows_candidate(std::path::Path::new("/repo/.cleanr/history")));
        assert!(policy.allows_candidate(std::path::Path::new("/repo/target")));
    }

    #[test]
    fn scan_root_itself_is_never_a_cleanup_candidate() {
        for scan_root in [
            PathBuf::from("/repo"),
            PathBuf::from("cleanr-core-missing-scan-root-fixture"),
        ] {
            let entry = ScanEntry {
                path: scan_root.clone(),
                kind: EntryKind::Directory,
                size_bytes: 100,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "trusted".into(),
                    rule_id: "broad".into(),
                    label: "Broad".into(),
                    category: "cache".into(),
                    confidence: Confidence::High,
                    reason: "generated".into(),
                    risk_note: "dangerous".into(),
                    default_selected: true,
                    trust: RuleTrust::Trusted,
                    match_role: RuleMatchRole::Primary,
                    sources: Vec::new(),
                    runtime_guard: None,
                }],
            };

            let plan = build_cleanup_plan(vec![scan_root.clone()], vec![], &[entry]);

            assert!(
                plan.items.is_empty(),
                "scan root became a cleanup candidate: {}",
                scan_root.display()
            );
        }
    }

    #[test]
    fn legacy_plan_omits_unresolved_rule_conflicts() {
        let hit = |rule_id: &str,
                   trust: RuleTrust,
                   confidence: Confidence,
                   default_selected: bool| RuleHit {
            rule_pack_id: "pack".into(),
            rule_id: rule_id.into(),
            label: rule_id.into(),
            category: "cache".into(),
            confidence,
            reason: rule_id.into(),
            risk_note: "review".into(),
            default_selected,
            trust,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        };
        let entry = ScanEntry {
            path: PathBuf::from("/repo/cache"),
            kind: EntryKind::Directory,
            size_bytes: 42,
            modified_at: None,
            rule_hits: vec![
                hit("untrusted", RuleTrust::Untrusted, Confidence::High, true),
                hit("trusted", RuleTrust::Trusted, Confidence::Medium, false),
                hit("builtin", RuleTrust::Builtin, Confidence::Low, false),
            ],
        };

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &[entry]);

        assert!(plan.items.is_empty());
    }

    #[test]
    fn protected_paths_are_normalized_sorted_and_deduplicated() {
        let temp = tempfile::tempdir().expect("tempdir");
        let protected = temp.path().join("protected");
        std::fs::create_dir(&protected).expect("protected dir");
        let policy = SafetyPolicy::new(
            vec![
                protected.clone(),
                temp.path().join(".").join("protected"),
                temp.path().join("another"),
            ],
            false,
        );

        assert_eq!(policy.protected_paths().len(), 2);
        assert!(!policy.allows_candidate(&protected));
        assert!(!policy.allows_candidate(temp.path()));
        assert!(!policy.requires_confirmation());
    }

    #[test]
    fn nonexistent_relative_protected_paths_are_made_absolute() {
        let relative = PathBuf::from("cleanr-missing-config-for-safety-test.toml");
        let expected = std::path::absolute(&relative).expect("absolute path");
        let policy = SafetyPolicy::new(vec![relative.clone()], true);

        assert_eq!(policy.protected_paths(), &[expected]);
        assert!(!policy.allows_candidate(&relative));
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_root_is_never_allowed() {
        assert!(!SafetyPolicy::default().allows_candidate(std::path::Path::new("/")));
    }
}
