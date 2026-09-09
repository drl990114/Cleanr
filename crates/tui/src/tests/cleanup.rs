use super::*;
use crate::effects::{OperationEvent, OperationKind};

fn manifest_for(app: &Workbench, statuses: &[ExecutionStatus]) -> ExecutionManifest {
    let plan = app.plan().expect("reviewed plan");
    ExecutionManifest {
        schema_version: EXECUTION_SCHEMA_VERSION.into(),
        run_id: "cleanup-result-fixture".into(),
        created_at: chrono::Utc::now(),
        plan_schema_version: plan.schema_version.clone(),
        authorization: None,
        summary: ExecutionSummary {
            attempted: statuses.len(),
            succeeded: statuses
                .iter()
                .filter(|s| **s == ExecutionStatus::Trashed)
                .count(),
            failed: statuses
                .iter()
                .filter(|s| **s == ExecutionStatus::Failed)
                .count(),
        },
        items: plan
            .items
            .iter()
            .zip(statuses)
            .map(|(item, status)| ExecutionItem {
                path: item.path.clone(),
                planned_action: item.planned_action,
                status: *status,
                rule_id: item.rule_id.clone(),
                rollback_receipt: None,
                error: (*status == ExecutionStatus::Failed)
                    .then(|| "simulated permission denied".into()),
            })
            .collect(),
    }
}

fn deliver(app: &mut Workbench, result: Result<ExecutionManifest, String>) {
    let (sender, receiver) = mpsc::channel();
    app.operation_kind = Some(OperationKind::Cleanup);
    app.operation_rx = Some(receiver);
    sender
        .send(OperationEvent::CleanupFinished(result))
        .unwrap();
    assert!(app.poll_tasks());
}

#[test]
fn cleanup_async_completion_invalidates_snapshot_and_never_reuses_old_plan() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = scan_category::category_app(temp.path().into(), &["logs"; 3]);
    app.state_dir = temp.path().join("state");
    app.toggle_global_scan_selection();
    app.rebuild_usage_order();
    app.scan_explicit_roots = app.roots.clone();
    let scope = app.scan_explicit_roots.clone();
    let revision = app.scan_data_revision;
    let manifest = manifest_for(&app, &[ExecutionStatus::Trashed; 3]);
    let expected_size = app.plan().unwrap().summary.selected_size_bytes;
    app.show_tasks();
    app.open_command('/');

    deliver(&mut app, Ok(manifest));

    assert_eq!(app.view, View::CleanupResult);
    assert!(matches!(app.mode, Mode::Normal));
    assert!(!app.has_background_task());
    assert!(app.scan_data_revision > revision);
    assert!(app.entries.is_empty());
    assert!(app.analysis.is_none());
    assert!(app.plan().is_none());
    assert!(app.selection.candidate_ids.is_empty());
    assert!(app.usage_order.is_empty());
    assert!(!app.usage_ready);
    assert_eq!(app.scan_explicit_roots, scope);
    assert_eq!(
        app.last_cleanup_result.as_ref().unwrap().cleaned_size_bytes,
        expected_size
    );
    assert_eq!(app.execution_manifests.len(), 1);
    let screen = render_text(&mut app, 120, 24);
    assert!(screen.contains("Scan again"), "{screen}");
    assert!(screen.contains("Restore history"), "{screen}");

    // Candidate shortcuts and explicit commands cannot revive or execute the consumed plan.
    for code in [
        KeyCode::Tab,
        KeyCode::Char('p'),
        KeyCode::Char('f'),
        KeyCode::Char('a'),
        KeyCode::Char('c'),
    ] {
        app.handle_key(key(code));
    }
    app.dispatch(ActionRequest::Plan);
    app.dispatch(ActionRequest::Clean {
        intent: CleanupIntent::ExplicitUserConfirmation,
    });
    assert!(!app.has_background_task());
    assert!(!app.confirmation_pending());
    assert!(app.plan().is_none());
    assert_eq!(app.execution_manifests.len(), 1);

    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.view, View::Home);
    app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(app.view, View::CleanupResult);
    app.handle_key(key(KeyCode::Char('z')));
    assert_eq!(app.view, View::Restore);
    assert!(!app.is_scan_running());
}

#[test]
fn cleanup_partial_result_counts_only_trashed_bytes_and_shows_failure_in_both_locales() {
    let temp = tempfile::tempdir().unwrap();
    for locale in ["en-US", "zh-CN"] {
        for theme in [Theme::dark(), Theme::light()] {
            let mut app = scan_category::category_app(temp.path().into(), &["logs"; 3]);
            app.i18n = I18n::new(locale, BTreeMap::new(), builtin_language_packs());
            app.theme = theme;
            let manifest = manifest_for(&app, &[ExecutionStatus::Trashed, ExecutionStatus::Failed]);
            let expected_size = app.plan().unwrap().items[0].size_bytes;
            let failed_path = manifest.items[1].path.clone();

            deliver(&mut app, Ok(manifest));

            assert!(!app.has_background_task());
            let result = app.last_cleanup_result.as_ref().unwrap();
            assert_eq!((result.succeeded, result.failed), (1, 1));
            assert_eq!(result.cleaned_size_bytes, expected_size);
            assert_eq!(result.first_failure.as_ref().unwrap().0, failed_path);
            let expected_title = app.i18n.t("cleanup_result_partial_title");
            let expected_failed = app
                .i18n
                .format("cleanup_result_failed", &[("count", "1".into())]);
            for (width, height) in [(120, 24), (60, 16), (44, 14)] {
                let screen = render_text(&mut app, width, height);
                let compact = screen.split_whitespace().collect::<String>();
                assert!(
                    compact.contains(&expected_title.split_whitespace().collect::<String>()),
                    "{screen}"
                );
                assert!(
                    compact.contains(&expected_failed.split_whitespace().collect::<String>()),
                    "{screen}"
                );
                assert!(
                    compact.contains(
                        &crate::views::format_bytes(expected_size)
                            .split_whitespace()
                            .collect::<String>()
                    ),
                    "{screen}"
                );
                assert!(compact.contains("simulatedpermissiondenied"), "{screen}");
                assert!(!screen.contains("No cleanup plan"), "{screen}");
            }
            app.go_home();
            let home = render_text(&mut app, 120, 24);
            assert!(
                home.split_whitespace()
                    .collect::<String>()
                    .contains(&expected_title.split_whitespace().collect::<String>()),
                "{home}"
            );
        }
    }
}

#[test]
fn cleanup_worker_errors_and_disconnects_show_unconfirmed_results_without_rescanning() {
    let temp = tempfile::tempdir().unwrap();
    for disconnected in [false, true] {
        let mut app = scan_category::category_app(temp.path().into(), &["logs"]);
        let error = if disconnected {
            let (sender, receiver) = mpsc::channel();
            app.operation_kind = Some(OperationKind::Cleanup);
            app.operation_rx = Some(receiver);
            drop(sender);
            assert!(app.poll_tasks());
            app.i18n.t("status_operation_disconnected")
        } else {
            let error = "could not record the outcome for cache-00000".to_string();
            deliver(&mut app, Err(error.clone()));
            error
        };
        assert_eq!(app.view, View::CleanupResult);
        assert!(!app.has_background_task());
        assert!(app.plan().is_none());
        assert_eq!(
            app.last_cleanup_result
                .as_ref()
                .unwrap()
                .interruption
                .as_deref(),
            Some(error.as_str())
        );
        let screen = render_text(&mut app, 120, 24);
        assert!(screen.contains("Cleanup interrupted"), "{screen}");
        assert!(
            screen.contains("Final counts and size could not be confirmed"),
            "{screen}"
        );
        assert!(screen.contains(&error), "{screen}");
        assert!(!screen.contains("0 B moved to Trash"), "{screen}");
        assert!(!screen.contains("Cleanup complete"), "{screen}");
    }
}

#[test]
fn cleanup_long_failure_scrolls_without_hiding_summary_or_intercepting_rescan() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path().into());
    app.last_cleanup_result = Some(CleanupResult {
        failed: 1,
        first_failure: Some((
            temp.path().join("cache"),
            format!("{}failure detail end", "long error ".repeat(150)),
        )),
        ..CleanupResult::default()
    });
    app.view = View::CleanupResult;
    render_text(&mut app, 60, 12);
    assert!(app.details.max_scroll > 0);
    app.handle_key(key(KeyCode::End));
    let screen = render_text(&mut app, 60, 12);
    assert!(screen.contains("Cleanup failed"), "{screen}");
    assert!(screen.contains("failure detail end"), "{screen}");
    assert!(screen.contains("Scan again"), "{screen}");
    app.handle_key(key(KeyCode::Char('s')));
    assert!(app.is_scan_running());
    assert!(app.last_cleanup_result.is_none());
    wait_for_scan(&mut app);
}

#[test]
fn cleanup_result_survives_a_rejected_rescan_request() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path().into());
    app.last_cleanup_result = Some(CleanupResult {
        succeeded: 1,
        ..CleanupResult::default()
    });
    app.view = View::CleanupResult;
    app.start_scan(ScanRequest {
        inactive_days: Some(u16::MAX),
        ..ScanRequest::default()
    });
    assert!(!app.is_scan_running());
    assert!(app.last_cleanup_result.is_some());
    assert_eq!(app.view, View::CleanupResult);
}
