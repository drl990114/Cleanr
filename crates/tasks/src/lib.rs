#![forbid(unsafe_code)]

mod cleanup;
mod platform;
mod restore;
mod storage;
mod workflow;

pub use cleanup::{
    CleanupExecutor, FakeTrashExecutor, TrashExecutor, execute_locally_confirmed_plan,
    execute_locally_confirmed_plan_with_executor,
};

pub(crate) use cleanup::{CleanupAuthorization, execute_cleanup_plan, validate_recoverable_plan};

pub use restore::{
    FakeRestoreExecutor, RestoreExecutor, SystemRestoreExecutor, restore_execution_manifest,
    restored_run_ids,
};

pub use storage::{
    ManifestRepository, list_execution_manifests, list_restore_manifests, write_cleanup_plan,
    write_execution_manifest, write_restore_manifest,
};

pub use workflow::{
    ConfiguredWorkflowScan, DelegatedCleanupRequest, PreparedWorkflowScan, ScanPreparationMode,
    ScanWorkflowError, ScanWorkflowInput, ScanWorkflowObserver, ScanWorkflowStage,
    SelectionOverrides, build_workflow_analysis, build_workflow_analysis_from_parts,
    build_workflow_plan, exact_selection, execute_delegated_cleanup, recommendation_policy,
    recommendation_policy_for_plan_rescan, run_configured_scan, run_scan_workflow,
    safety_policy_for_config, selection_with_overrides, semantic_explicit_roots,
    validate_plan_sha256, verify_plan_unchanged,
};

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        fs,
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use anyhow::Result;
    use chrono::{DateTime, Utc};
    use cleanr_core::{
        AnalysisScanContext, CleanupAuthorizationSource, CleanupPlanSourceScan, Confidence,
        EXECUTION_SCHEMA_VERSION, EntryKind, ExecutionAuthorization, ExecutionItem,
        ExecutionManifest, ExecutionStatus, ExecutionSummary, PlannedAction,
        RESTORE_SCHEMA_VERSION, RecommendationPolicy, RecommendationState, ReportIntegrity,
        RestoreManifest, RestoreStatus, RestoreSummary, RollbackReceipt, RuleTrust, SafetyPolicy,
        ScanBudgetExceeded, ScanIssue, ScanIssueCode, UserSelection,
        build_analysis_report_with_scan_context, build_cleanup_plan,
        build_cleanup_plan_from_analysis, build_cleanup_plan_with_policy,
    };
    use cleanr_core::{RuleHit, ScanEntry};

    #[cfg(target_os = "macos")]
    use crate::platform::{restore_from_system_trash, trash_with_receipt};

    fn cleanup_entry(path: PathBuf, kind: EntryKind, size_bytes: u64) -> ScanEntry {
        ScanEntry {
            path,
            kind,
            size_bytes,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "generated".into(),
                label: "Generated".into(),
                category: "build-cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: RuleTrust::Builtin,
                match_role: cleanr_core::RuleMatchRole::Primary,
                sources: Vec::new(),
            }],
        }
    }

    fn restorable_manifest(run_id: &str, path: PathBuf) -> ExecutionManifest {
        ExecutionManifest {
            schema_version: EXECUTION_SCHEMA_VERSION.to_string(),
            run_id: run_id.to_string(),
            created_at: Utc::now(),
            plan_schema_version: "plan".to_string(),
            authorization: None,
            summary: ExecutionSummary {
                attempted: 1,
                succeeded: 1,
                failed: 0,
            },
            items: vec![ExecutionItem {
                path,
                planned_action: PlannedAction::Trash,
                status: ExecutionStatus::Trashed,
                rule_id: "test".to_string(),
                rollback_receipt: Some(RollbackReceipt {
                    method: "fake-trash".to_string(),
                    note: "test".to_string(),
                    locator: Some("fake".to_string()),
                }),
                error: None,
            }],
        }
    }

    #[test]
    fn fake_clean_writes_execution_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("create target");
        let entry = ScanEntry {
            path: temp.path().join("target"),
            kind: EntryKind::Directory,
            size_bytes: 0,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "rust-target".into(),
                label: "Rust target".into(),
                category: "build-cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: cleanr_core::RuleTrust::Builtin,
                match_role: cleanr_core::RuleMatchRole::Primary,
                sources: Vec::new(),
            }],
        };
        let plan = build_cleanup_plan(vec![temp.path().to_path_buf()], vec![], &[entry]);
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_delegation();
        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.succeeded, 1);
        assert_eq!(manifest.items[0].planned_action, PlannedAction::Trash);
        assert_eq!(
            manifest.authorization,
            Some(ExecutionAuthorization {
                source: CleanupAuthorizationSource::ExplicitUserDelegation,
            })
        );
        assert_eq!(fake.trashed_paths().len(), 1);
        assert_eq!(
            list_execution_manifests(temp.path()).expect("list").len(),
            1
        );
    }

    #[test]
    fn manifest_repository_round_trips_history_and_finds_runs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repository = ManifestRepository::new(temp.path());
        let mut older = restorable_manifest("run-old", temp.path().join("old"));
        older.created_at = Utc::now() - chrono::Duration::seconds(60);
        let newer = restorable_manifest("run-new", temp.path().join("new"));
        repository
            .write_execution(&older)
            .expect("write older execution");
        repository
            .write_execution(&newer)
            .expect("write newer execution");
        let restore = RestoreManifest {
            schema_version: RESTORE_SCHEMA_VERSION.to_string(),
            restore_id: "restore-1".to_string(),
            source_run_id: "run-old".to_string(),
            created_at: Utc::now(),
            summary: RestoreSummary {
                attempted: 0,
                succeeded: 0,
                failed: 0,
            },
            items: Vec::new(),
        };
        repository.write_restore(&restore).expect("write restore");

        let (executions, restores) = repository.history().expect("history");

        assert_eq!(executions.len(), 2);
        assert_eq!(executions[0].run_id, "run-new");
        assert_eq!(executions[1].run_id, "run-old");
        assert_eq!(restores.len(), 1);
        assert_eq!(restores[0].restore_id, "restore-1");
        assert_eq!(
            repository
                .find_execution("run-old")
                .expect("find")
                .expect("run")
                .run_id,
            "run-old"
        );
        assert!(
            repository
                .find_execution("missing")
                .expect("find")
                .is_none()
        );
    }

    #[test]
    fn execution_manifest_is_journaled_before_each_cleanup_item() {
        struct JournalInspectingExecutor {
            state_dir: PathBuf,
            calls: Mutex<usize>,
        }

        impl CleanupExecutor for JournalInspectingExecutor {
            fn trash(&self, path: &Path) -> Result<RollbackReceipt> {
                let mut calls = self.calls.lock().expect("calls mutex");
                *calls += 1;
                let manifests = list_execution_manifests(&self.state_dir).expect("journal");
                assert_eq!(manifests.len(), 1);
                let manifest = &manifests[0];
                match *calls {
                    1 => {
                        assert_eq!(manifest.summary.attempted, 0);
                        assert!(
                            manifest
                                .items
                                .iter()
                                .all(|item| item.status == ExecutionStatus::Pending)
                        );
                    }
                    2 => {
                        assert_eq!(manifest.summary.attempted, 1);
                        assert_eq!(manifest.summary.succeeded, 1);
                        assert_eq!(manifest.items[0].status, ExecutionStatus::Trashed);
                        assert_eq!(manifest.items[1].status, ExecutionStatus::Pending);
                    }
                    _ => unreachable!("test only creates two cleanup items"),
                }

                Ok(RollbackReceipt {
                    method: "fake-trash".to_string(),
                    note: "journal test".to_string(),
                    locator: Some(format!("fake:{}", path.display())),
                })
            }
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first.bin");
        let second = temp.path().join("second.bin");
        fs::write(&first, b"one").expect("first");
        fs::write(&second, b"two").expect("second");
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[
                cleanup_entry(first, EntryKind::File, 3),
                cleanup_entry(second, EntryKind::File, 3),
            ],
        );
        let executor = JournalInspectingExecutor {
            state_dir: temp.path().to_path_buf(),
            calls: Mutex::new(0),
        };
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest = execute_cleanup_plan(&plan, &executor, temp.path(), Some(&authorization))
            .expect("execute");

        assert_eq!(manifest.summary.attempted, 2);
        assert_eq!(manifest.summary.succeeded, 2);
        assert!(
            manifest
                .items
                .iter()
                .all(|item| item.status == ExecutionStatus::Trashed)
        );
    }

    #[test]
    fn cleanup_continues_after_an_individual_trash_failure() {
        struct FailFirstTrashExecutor {
            calls: Mutex<Vec<PathBuf>>,
        }

        impl CleanupExecutor for FailFirstTrashExecutor {
            fn trash(&self, path: &Path) -> Result<RollbackReceipt> {
                self.calls
                    .lock()
                    .expect("calls mutex")
                    .push(path.to_path_buf());
                if path.file_name().is_some_and(|name| name == "first.bin") {
                    anyhow::bail!("simulated trash failure");
                }
                Ok(RollbackReceipt {
                    method: "fake-trash".to_string(),
                    note: "mixed-result test".to_string(),
                    locator: Some(format!("fake:{}", path.display())),
                })
            }
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first.bin");
        let second = temp.path().join("second.bin");
        fs::write(&first, b"one").expect("first");
        fs::write(&second, b"two").expect("second");
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[
                cleanup_entry(first.clone(), EntryKind::File, 3),
                cleanup_entry(second.clone(), EntryKind::File, 3),
            ],
        );
        let executor = FailFirstTrashExecutor {
            calls: Mutex::new(Vec::new()),
        };
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest = execute_cleanup_plan(
            &plan,
            &executor,
            temp.path().join("state"),
            Some(&authorization),
        )
        .expect("execute");

        assert_eq!(manifest.summary.attempted, 2);
        assert_eq!(manifest.summary.succeeded, 1);
        assert_eq!(manifest.summary.failed, 1);
        assert_eq!(
            executor.calls.lock().expect("calls mutex").as_slice(),
            [first, second]
        );
        assert_eq!(manifest.items[0].status, ExecutionStatus::Failed);
        assert_eq!(manifest.items[1].status, ExecutionStatus::Trashed);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_trash_round_trip_records_exact_locator() {
        // Exercise a caller-selected volume while only trashing a test-owned directory.
        let temp = std::env::var_os("CLEANR_MACOS_TRASH_TEST_PARENT").map_or_else(
            || tempfile::tempdir().expect("tempdir"),
            |parent| {
                tempfile::Builder::new()
                    .prefix(".cleanr-trash-round-trip-")
                    .tempdir_in(parent)
                    .expect("tempdir in requested parent")
            },
        );
        let target = temp.path().join("旧 macOS 兼容 cache");
        let nested_file = target.join("nested folder").join("artifact.bin");
        fs::create_dir_all(nested_file.parent().expect("nested parent"))
            .expect("create test directory");
        fs::write(&nested_file, b"round trip").expect("seed file");

        let receipt = trash_with_receipt(&target).expect("move test file to macOS Trash");
        let trashed_path = receipt
            .locator
            .as_deref()
            .and_then(|locator| locator.strip_prefix("mac-path:"))
            .map(PathBuf::from)
            .expect("exact macOS Trash locator");
        let source_was_removed = !target.try_exists().expect("source existence");
        let trash_item_existed = trashed_path.try_exists().expect("trash item existence");

        restore_from_system_trash(&target, &receipt, Utc::now().timestamp())
            .expect("restore test file from macOS Trash");

        assert!(source_was_removed);
        assert!(trash_item_existed);
        assert!(target.is_dir());
        assert_eq!(
            fs::read(&nested_file).expect("restored contents"),
            b"round trip"
        );
    }

    #[test]
    fn cleanup_requires_user_authorization_without_a_confirmation_dialog() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("create target");
        let entry = ScanEntry {
            path: temp.path().join("target"),
            kind: EntryKind::Directory,
            size_bytes: 0,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "rust-target".into(),
                label: "Rust target".into(),
                category: "build-cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: cleanr_core::RuleTrust::Builtin,
                match_role: cleanr_core::RuleMatchRole::Primary,
                sources: Vec::new(),
            }],
        };
        let policy = cleanr_core::SafetyPolicy::new(vec![], false);
        let plan = cleanr_core::build_cleanup_plan_with_policy(
            vec![temp.path().to_path_buf()],
            vec![],
            &[entry],
            &policy,
        );
        let fake = FakeTrashExecutor::default();

        let error = execute_cleanup_plan(&plan, &fake, temp.path(), None)
            .expect_err("cleanup without local authorization must be denied");
        assert!(error.to_string().contains("user authorization"));
        assert!(fake.trashed_paths().is_empty());
    }

    #[test]
    fn cleanup_rejects_budget_exhausted_plan_before_manifest_or_trash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("create target");
        let entries = vec![cleanup_entry(target, EntryKind::Directory, 0)];
        let policy = SafetyPolicy::new(vec![], false);
        let budgets = [ScanBudgetExceeded::EntryCount {
            limit: 1,
            observed: 2,
        }];
        let analysis = build_analysis_report_with_scan_context(
            Utc::now(),
            Utc::now(),
            vec![temp.path().to_path_buf()],
            &entries,
            &[],
            RecommendationPolicy::default(),
            AnalysisScanContext {
                budget_exceeded: &budgets,
                safety_policy: Some(&policy),
                ..AnalysisScanContext::default()
            },
        )
        .expect("analysis");
        let mut plan = build_cleanup_plan_with_policy(
            vec![temp.path().to_path_buf()],
            vec![],
            &entries,
            &policy,
        );
        plan.source_scan = Some(CleanupPlanSourceScan {
            analysis_id: analysis.analysis_id,
            integrity: analysis.scan.integrity,
            budget_exceeded: analysis.scan.budget_exceeded,
            recommendation_policy: Some(analysis.policy),
            scope: None,
        });
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let error = execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization))
            .expect_err("budget-exhausted plan must be denied");

        assert!(error.to_string().contains("read-only"));
        assert!(fake.trashed_paths().is_empty());
        assert!(
            list_execution_manifests(temp.path())
                .expect("list manifests")
                .is_empty()
        );
    }

    #[test]
    fn cleanup_allows_explicit_selection_from_an_ordinary_partial_scan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("create target");
        let entries = vec![cleanup_entry(target.clone(), EntryKind::Directory, 0)];
        let policy = SafetyPolicy::new(vec![], false);
        let analysis = build_analysis_report_with_scan_context(
            Utc::now(),
            Utc::now(),
            vec![temp.path().to_path_buf()],
            &entries,
            &[ScanIssue {
                code: ScanIssueCode::MetadataUnavailable,
                path: Some(target.join("unreadable")),
            }],
            RecommendationPolicy::default(),
            AnalysisScanContext {
                budget_exceeded: &[],
                safety_policy: Some(&policy),
                ..AnalysisScanContext::default()
            },
        )
        .expect("ordinary partial analysis");
        assert_eq!(analysis.scan.integrity, ReportIntegrity::Partial);
        assert!(analysis.scan.budget_exceeded.is_empty());
        assert_eq!(
            analysis.candidates[0].recommendation.state,
            RecommendationState::Review
        );
        let mut selection = UserSelection::default();
        selection.select(analysis.candidates[0].id.clone());
        let plan = build_cleanup_plan_from_analysis(
            vec![temp.path().to_path_buf()],
            vec![],
            &entries,
            &analysis,
            &selection,
            &policy,
        )
        .expect("ordinary partial analysis may be explicitly selected");
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest = execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization))
            .expect("ordinary partial plan remains executable after explicit selection");

        assert_eq!(manifest.summary.succeeded, 1);
        assert_eq!(fake.trashed_paths(), vec![target]);
    }

    #[test]
    fn fake_restore_writes_restore_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("target");
        let manifest = restorable_manifest("run-1", source.clone());
        let fake = FakeRestoreExecutor::default();
        let restored = restore_execution_manifest(&manifest, &fake, temp.path()).expect("restore");

        assert_eq!(restored.summary.succeeded, 1);
        assert_eq!(fake.restored_paths(), vec![source]);
        assert_eq!(list_restore_manifests(temp.path()).expect("list").len(), 1);
    }

    #[test]
    fn changed_file_is_recorded_as_failure_without_calling_executor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("artifact");
        fs::write(&target, b"old").expect("seed file");
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[cleanup_entry(target.clone(), EntryKind::File, 3)],
        );
        fs::write(&target, b"changed").expect("change file");
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.failed, 1);
        assert!(
            manifest.items[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("size changed"))
        );
        assert!(fake.trashed_paths().is_empty());
    }

    #[test]
    fn changed_directory_contents_are_rejected_before_trash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let child = target.join("artifact");
        fs::create_dir(&target).expect("target");
        fs::write(&child, b"old").expect("seed child");
        let target_metadata = target.symlink_metadata().expect("target metadata");
        let child_metadata = child.symlink_metadata().expect("child metadata");
        let mut target_entry = cleanup_entry(target.clone(), EntryKind::Directory, 3);
        target_entry.modified_at = target_metadata.modified().ok().map(DateTime::<Utc>::from);
        let child_entry = ScanEntry {
            path: child,
            kind: EntryKind::File,
            size_bytes: 3,
            modified_at: child_metadata.modified().ok().map(DateTime::<Utc>::from),
            rule_hits: Vec::new(),
        };
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[target_entry, child_entry],
        );
        fs::write(target.join("new-artifact"), b"new").expect("new child");
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.failed, 1);
        assert!(
            manifest.items[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("contents changed"))
        );
        assert!(fake.trashed_paths().is_empty());
    }

    #[test]
    fn protected_target_is_revalidated_at_execution_time() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("target");
        let mut plan = build_cleanup_plan_with_policy(
            vec![temp.path().to_path_buf()],
            vec![],
            &[cleanup_entry(target.clone(), EntryKind::Directory, 0)],
            &SafetyPolicy::new(vec![], true),
        );
        plan.safety.protected_subtrees.push(target);
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.failed, 1);
        assert!(
            manifest.items[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("protected subtree"))
        );
    }

    #[test]
    fn repeated_restore_skips_items_already_restored() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("target");
        let manifest = restorable_manifest("run-repeat", source.clone());
        let fake = FakeRestoreExecutor::default();

        let first =
            restore_execution_manifest(&manifest, &fake, temp.path()).expect("first restore");
        let second =
            restore_execution_manifest(&manifest, &fake, temp.path()).expect("second restore");

        assert_eq!(first.summary.succeeded, 1);
        assert_eq!(second.summary.attempted, 0);
        assert_eq!(second.items[0].status, RestoreStatus::Skipped);
        assert_eq!(fake.restored_paths(), vec![source]);
    }

    #[test]
    fn restore_reports_missing_receipts_and_skips_failed_cleanup_items() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut manifest = restorable_manifest("run-invalid", temp.path().join("missing-receipt"));
        manifest.items[0].rollback_receipt = None;
        manifest.items.push(ExecutionItem {
            path: temp.path().join("never-trashed"),
            planned_action: PlannedAction::Trash,
            status: ExecutionStatus::Failed,
            rule_id: "test".to_string(),
            rollback_receipt: None,
            error: Some("cleanup failed".to_string()),
        });
        let fake = FakeRestoreExecutor::default();

        let restore =
            restore_execution_manifest(&manifest, &fake, temp.path()).expect("restore manifest");

        assert_eq!(restore.summary.attempted, 1);
        assert_eq!(restore.summary.failed, 1);
        assert_eq!(restore.items[0].status, RestoreStatus::Failed);
        assert_eq!(restore.items[1].status, RestoreStatus::Skipped);
        assert!(fake.restored_paths().is_empty());
    }

    #[test]
    fn restored_run_ids_require_at_least_one_success_and_no_failures() {
        let restore = |run_id: &str, succeeded: usize, failed: usize| RestoreManifest {
            schema_version: RESTORE_SCHEMA_VERSION.to_string(),
            restore_id: format!("restore-{run_id}"),
            source_run_id: run_id.to_string(),
            created_at: Utc::now(),
            summary: RestoreSummary {
                attempted: succeeded + failed,
                succeeded,
                failed,
            },
            items: vec![],
        };
        let manifests = vec![
            restore("complete", 1, 0),
            restore("partial", 1, 1),
            restore("empty", 0, 0),
        ];

        assert_eq!(restored_run_ids(&manifests), HashSet::from(["complete"]));
    }
}
