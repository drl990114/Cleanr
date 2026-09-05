use super::scan_category::{category_app, settle_projection};
use super::*;
use crate::projection::{CategoryKey, ScanSort};

fn find(app: &mut Workbench, text: &str) {
    app.handle_key(key(KeyCode::Char('p')));
    while app.input.len() > 1 {
        app.handle_key(key(KeyCode::Backspace));
    }
    app.handle_paste(text);
    app.handle_key(key(KeyCode::Enter));
    settle_projection(app);
}

fn settle_tasks(app: &mut Workbench) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.has_background_task() && Instant::now() < deadline {
        app.poll_tasks();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !app.has_background_task(),
        "background task did not finish: {}",
        app.status()
    );
}

#[test]
fn interaction_search_normalizes_case_and_separators_and_escape_restores_query() {
    let mut app = app(PathBuf::from("/fixture"));
    app.entries = Arc::new(
        ["C:\\Build\\缓存\\ÄBC", "C:\\logs\\second"]
            .iter()
            .map(|path| ScanEntry {
                path: PathBuf::from(path),
                kind: EntryKind::File,
                size_bytes: 10,
                modified_at: None,
                rule_hits: vec![test_rule_hit("search")],
            })
            .collect(),
    );
    app.view = View::Scan;
    app.ensure_scan_view_projection();
    find(&mut app, "build/缓存/äbc");
    assert_eq!(app.list_len(), 1);
    let original = app.selected_scan_row().unwrap().path.clone();
    app.handle_key(key(KeyCode::Char('p')));
    app.handle_paste("missing");
    app.scan_view.search_due = Some(Instant::now());
    app.poll_tasks();
    assert_eq!(app.list_len(), 0);
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.selected_scan_row().unwrap().path, original);
    assert!(
        app.plan().is_none(),
        "read-only queries never create a plan"
    );
}

#[test]
fn interaction_selected_only_removal_keeps_neighbor_and_selection_scope() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &["logs"; 6]);
    app.handle_key(key(KeyCode::Char('A')));
    app.handle_key(key(KeyCode::Char('v')));
    app.list_state.select(Some(3));
    let neighbor = app.scan_view.rows[app.scan_view.visible[4]].path.clone();
    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(app.selected_scan_row().unwrap().path, neighbor);
    assert_eq!(app.list_state.selected(), Some(3));
    assert_eq!(app.plan().unwrap().summary.selected_count, 5);
    find(&mut app, "missing");
    app.handle_key(key(KeyCode::Char('a')));
    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(app.plan().unwrap().summary.selected_count, 5);
    app.handle_key(key(KeyCode::Char('A')));
    assert_eq!(app.plan().unwrap().summary.selected_count, 6);
}

#[test]
fn interaction_sort_preserves_path_and_plan_order() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &["logs"; 6]);
    let plan_ptr = Arc::as_ptr(app.plan.as_ref().unwrap());
    let before = app
        .plan()
        .unwrap()
        .items
        .iter()
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    let focus = app.selected_scan_row().unwrap().path.clone();
    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.scan_view.sort, ScanSort::Path);
    assert_eq!(app.selected_scan_row().unwrap().path, focus);
    let paths = app
        .scan_view
        .visible
        .iter()
        .map(|i| app.scan_view.rows[*i].path.clone())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(Arc::as_ptr(app.plan.as_ref().unwrap()), plan_ptr);
    assert_eq!(
        app.plan()
            .unwrap()
            .items
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn interaction_view_switching_reuses_snapshot_and_restores_focus_and_offset() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &["logs"; 200]);
    app.list_state.select(Some(150));
    render_text(&mut app, 120, 24);
    let state = app.list_state.clone();
    let plan = Arc::as_ptr(app.plan.as_ref().unwrap());
    let entries = Arc::as_ptr(&app.entries);
    app.handle_key(key(KeyCode::Char('u')));
    settle_tasks(&mut app);
    app.list_state.select(Some(120));
    render_text(&mut app, 120, 24);
    let usage_state = app.list_state.clone();
    for _ in 0..20 {
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('r')));
        assert_eq!(app.list_state, state);
        app.show_rules();
        app.handle_key(key(KeyCode::Char('r')));
        assert_eq!(app.list_state, state);
        app.handle_key(key(KeyCode::Char('u')));
        assert_eq!(app.list_state, usage_state);
        assert_eq!(Arc::as_ptr(app.plan.as_ref().unwrap()), plan);
        assert_eq!(Arc::as_ptr(&app.entries), entries);
        assert!(!app.has_background_task());
    }
}

#[test]
fn interaction_large_projection_ignores_stale_result_and_cancelled_search_can_restart() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &["logs"; 10_000]);
    find(&mut app, "cache-000");
    assert_eq!(app.list_len(), 100);
    app.handle_key(key(KeyCode::Char('p')));
    app.handle_paste("missing");
    app.scan_view.search_due = Some(Instant::now());
    app.poll_tasks();
    // Cancel even if the worker has already queued its result; receiver ownership fences it.
    app.handle_key(key(KeyCode::Esc));
    settle_projection(&mut app);
    assert_eq!(app.list_len(), 100);
    find(&mut app, "cache-001");
    assert_eq!(app.list_len(), 100);
    let previous = Arc::clone(&app.scan_view.visible);
    let (sender, receiver) = mpsc::channel();
    app.scan_view.projection_rx = Some(receiver);
    sender
        .send(crate::effects::ProjectedScan {
            data_revision: app.scan_data_revision.wrapping_sub(1),
            query_revision: 0,
            visible: Arc::new(vec![]),
        })
        .unwrap();
    app.poll_tasks();
    assert_eq!(*app.scan_view.visible, *previous);
}

#[test]
fn interaction_confirm_review_and_detail_focus_do_not_authorize_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    for locale in ["en-US", "zh-CN"] {
        for theme in [Theme::dark(), Theme::light()] {
            let mut app = category_app(temp.path().into(), &["logs", "build-cache"]);
            app.i18n = I18n::new(locale, BTreeMap::new(), builtin_language_packs());
            app.theme = theme;
            app.handle_key(key(KeyCode::Char('A')));
            app.scan_view.filter = Some(CategoryKey::Named("logs".into()));
            app.ensure_scan_view_projection();
            app.handle_key(key(KeyCode::Char('c')));
            app.handle_key(key(KeyCode::Char('v')));
            assert!(!app.confirmation_pending());
            assert!(app.scan_view.filter.is_none());
            assert_eq!(app.list_len(), 2);
            app.handle_key(key(KeyCode::Char('c')));
            assert_eq!(app.confirm_choice, ConfirmChoice::No);
            app.handle_key(key(KeyCode::Esc));
            let focused = app.selected_scan_row().unwrap().source_index;
            let item = &mut Arc::make_mut(app.plan.as_mut().unwrap()).items[focused];
            item.risk_note = "Review this risk. 请检查风险。 ".repeat(80);
            if let Some(evidence) = &mut item.evidence {
                for rule in &mut evidence.matched_rules {
                    rule.risk_note = item.risk_note.clone();
                }
            }
            for (width, height) in [(120, 40), (80, 24), (60, 20), (40, 12)] {
                render_text(&mut app, width, height);
                app.handle_key(key(KeyCode::Tab));
                let details = render_text(&mut app, width, height);
                assert!(
                    details.contains('└') && details.contains('┘'),
                    "the complete details border must remain visible: {details}"
                );
                assert!(app.scan_view.details_focused);
                app.handle_key(key(KeyCode::End));
                assert!(app.scan_view.details_scroll > 0);
                app.handle_key(key(KeyCode::Enter));
                app.handle_key(key(KeyCode::Char(' ')));
                assert_eq!(app.plan().unwrap().summary.selected_count, 2);
                app.handle_key(key(KeyCode::Esc));
                assert!(!app.scan_view.details_focused);
                app.handle_key(key(KeyCode::Char('?')));
                render_text(&mut app, width, height);
                app.handle_key(key(KeyCode::End));
                if width <= 60 {
                    assert!(app.help_scroll > 0);
                }
                app.handle_key(key(KeyCode::Esc));
            }
            assert!(!app.is_operation_running());
        }
    }
}

#[test]
fn interaction_projection_completed_in_usage_preserves_scan_path_focus() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &["logs"; 10_000]);
    let index = app
        .plan()
        .unwrap()
        .items
        .iter()
        .position(|item| item.path.ends_with("cache-00042"))
        .unwrap();
    app.list_state.select(Some(index));
    let focused = app.selected_scan_row().unwrap().path.clone();
    app.handle_key(key(KeyCode::Char('p')));
    app.handle_paste("cache-000");
    app.handle_key(key(KeyCode::Enter));
    app.open_current_usage();
    settle_tasks(&mut app);
    app.review();
    assert_eq!(app.selected_scan_row().unwrap().path, focused);
    assert!(app.list_state.selected().unwrap() < 100);
}

#[test]
fn interaction_operation_freezes_selection_and_cancel_feedback_is_immediate() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &["logs"; 100]);
    let frozen = Arc::clone(app.plan.as_ref().unwrap());
    let (_sender, receiver) = mpsc::channel();
    app.operation_rx = Some(receiver);
    for code in [
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Char('a'),
        KeyCode::Char('A'),
    ] {
        app.handle_key(key(code));
    }
    assert!(Arc::ptr_eq(app.plan.as_ref().unwrap(), &frozen));
    assert_eq!(app.plan().unwrap().summary.selected_count, 0);
    app.operation_rx = None;
    let (_sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    app.scan_rx = Some(receiver);
    app.scan_cancel = Some(Arc::clone(&cancel));
    let started = Instant::now();
    app.handle_key(key(KeyCode::Char('x')));
    let text = render_text(&mut app, 120, 40);
    assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    assert!(app.scan_cancel_requested);
    assert!(text.to_lowercase().contains("cancell"), "{text}");
    assert!(started.elapsed() < Duration::from_millis(100));
}

#[test]
fn interaction_update_notice_preserves_error_status_and_ignored_keys_are_clean() {
    let mut app = app(PathBuf::from("/fixture"));
    let (sender, receiver) = mpsc::channel();
    app.update_notice_rx = Some(receiver);
    app.status = "an existing task error".into();
    sender
        .send(crate::UpdateNotice {
            version: "99.0.0".into(),
            release_url: "https://example.org/release".into(),
        })
        .unwrap();
    assert!(app.poll_tasks());
    assert_eq!(app.status, "an existing task error");
    assert!(render_text(&mut app, 120, 40).contains("99.0.0"));
    assert!(!app.handle_key_changed(key(KeyCode::F(12))));
    assert!(!app.can_batch_navigation(key(KeyCode::Char(' '))));
    assert!(!app.can_batch_navigation(key(KeyCode::Char('c'))));
    assert!(app.can_batch_navigation(key(KeyCode::Down)));
}

#[test]
#[ignore = "local interaction latency and bounded snapshot retention evidence"]
fn interaction_performance_large_snapshots() {
    let candidates = std::env::var("CLEANR_BENCH_CANDIDATES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10_000);
    let temp = tempfile::tempdir().unwrap();
    let mut app = category_app(temp.path().into(), &vec!["logs"; candidates]);
    let plan_ptr = Arc::as_ptr(app.plan.as_ref().unwrap());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    app.list_state.select(Some(candidates / 2));
    for _ in 0..10 {
        terminal.draw(|f| render(f, &mut app)).unwrap();
    }
    for action in [KeyCode::Down, KeyCode::Char(' '), KeyCode::Char('c')] {
        let mut samples = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            app.handle_key_changed(key(action));
            terminal.draw(|f| render(f, &mut app)).unwrap();
            samples.push(start.elapsed());
            if app.confirmation_pending() {
                app.handle_key(key(KeyCode::Esc));
            }
        }
        samples.sort_unstable();
        eprintln!(
            "interaction candidates={candidates} action={action:?} input_to_test_frame_p95_us={} max_us={}",
            samples[94].as_micros(),
            samples[99].as_micros()
        );
        assert!(samples[94] < Duration::from_millis(50));
        // Leave one selected item so the confirmation path is actually measured.
        if action == KeyCode::Char(' ') && app.plan().unwrap().summary.selected_count == 0 {
            app.handle_key(key(KeyCode::Char(' ')));
        }
    }
    app.open_current_usage();
    settle_tasks(&mut app);
    app.review();
    let rss = || {
        #[cfg(unix)]
        {
            let output = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u64>()
                .unwrap()
        }
        #[cfg(not(unix))]
        {
            0u64
        }
    };
    let initial_rss = rss();
    let mut peak_rss = initial_rss;
    for round in 0..20 {
        find(
            &mut app,
            if round % 2 == 0 {
                "cache-00"
            } else {
                "cache-01"
            },
        );
        app.go_home();
        app.open_current_usage();
        app.review();
        assert_eq!(Arc::strong_count(&app.scan_view.index), 1);
        assert_eq!(Arc::as_ptr(app.plan.as_ref().unwrap()), plan_ptr);
        peak_rss = peak_rss.max(rss());
    }
    eprintln!(
        "interaction retention candidates={candidates} rounds=20 rss_start_kib={initial_rss} rss_end_kib={} rss_peak_kib={peak_rss}",
        rss()
    );
}
