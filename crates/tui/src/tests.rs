use super::*;
use crate::{
    app::{ConfirmChoice, DurationRecorder, Mode, View},
    commands::{ActionRequest, CleanupIntent, palette_command_invocation},
    effects::{
        PreparedScan, ScanDiagnostics, ScanFailure, ScanStage, ScanTaskProgress, TaskEvent,
        build_usage_projection,
    },
    views::{
        bottom_bounded_rect, centered_bounded_rect, command_cursor_position, command_input_view,
        display_width, fluid_content_rect, ime_guard_position, render, scan_loading_bar_sample,
        truncate_text, usage_descendant_count, visible_list_window,
    },
};
use cleanr_config::Config;
use cleanr_core::{
    Confidence, DecisionCode, EXECUTION_SCHEMA_VERSION, EntryKind, ExecutionItem,
    ExecutionManifest, ExecutionStatus, ExecutionSummary, GlobalScanKind, PlannedAction,
    RecommendationState, RollbackReceipt, RuleHit, RuleTrust, ScanBudgetExceeded, ScanEntry,
    ScanRequest,
};
use cleanr_fs::{GlobalScanRoot, ResolvedScanRoots, ScanOptions, global_scan_evidence, scan_paths};
use cleanr_i18n::{I18n, builtin_language_packs};
use cleanr_rules::RuleRegistry;
use cleanr_tasks::{FakeTrashExecutor, write_execution_manifest};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::{Position, Rect},
    style::Color,
    widgets::ListState,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    }
}

#[test]
fn duration_recorder_keeps_latest_128_samples_and_reports_p95_and_max() {
    let mut recorder = DurationRecorder::default();
    for millis in 1..=130 {
        recorder.record(Duration::from_millis(millis));
    }

    let summary = recorder.summary();
    assert_eq!(summary.p95, Duration::from_millis(124));
    assert_eq!(summary.max, Duration::from_millis(130));

    recorder.clear();
    assert_eq!(recorder.summary(), Default::default());
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    }
}

fn repeat(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Repeat,
        state: crossterm::event::KeyEventState::empty(),
    }
}

fn app(root: PathBuf) -> Workbench {
    Workbench::new(
        vec![root],
        Config::default(),
        RuleRegistry::builtin().expect("builtin rules"),
        I18n::new("en-US", BTreeMap::new(), builtin_language_packs()),
        Theme::dark(),
    )
}

fn render_text(app: &mut Workbench, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(frame, app))
        .expect("render frame");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn theme_has_extended_color(theme: Theme) -> bool {
    [
        theme.bg,
        theme.surface,
        theme.surface_alt,
        theme.fg,
        theme.fg_dim,
        theme.border,
        theme.accent,
        theme.ok,
        theme.warn,
        theme.danger,
        theme.cyan,
        theme.magenta,
        theme.highlight_bg,
        theme.highlight_fg,
    ]
    .iter()
    .any(|color| matches!(color, Color::Rgb(_, _, _) | Color::Indexed(_)))
}

fn test_rule_hit(rule_id: &str) -> RuleHit {
    RuleHit {
        rule_pack_id: "builtin-dev".into(),
        rule_id: rule_id.into(),
        label: "Generated".into(),
        category: "build-cache".into(),
        confidence: Confidence::High,
        reason: "generated".into(),
        risk_note: "rebuild".into(),
        default_selected: true,
        trust: RuleTrust::Builtin,
        match_role: cleanr_core::RuleMatchRole::Primary,
        sources: Vec::new(),
    }
}

#[test]
fn built_in_themes_use_only_portable_ansi_colors() {
    assert!(!theme_has_extended_color(Theme::dark()));
    assert!(!theme_has_extended_color(Theme::light()));
    assert!(matches!(Theme::dark().bg, Color::Reset));
    assert!(matches!(Theme::light().bg, Color::Reset));
}

#[test]
fn starts_in_workbench_with_empty_command_input() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = app(temp.path().to_path_buf());
    assert_eq!(app.input(), "");
    assert!(!app.palette_open());
    assert!(app.status().contains("Ready"));
}

#[test]
fn home_layout_has_one_clear_primary_action() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let screen = render_text(&mut app, 100, 28);
    println!("{screen}");

    assert!(screen.contains("Safe intelligent disk organization"));
    assert!(screen.contains("[s]  Scan & analyze"));
    assert!(screen.contains("Every item is reviewed first"));
    assert!(!screen.contains('›'));
    assert!(!screen.contains("Recent activity"));
    assert!(!screen.contains("No scan yet"));
}

#[test]
fn home_layout_starts_near_the_top_on_tall_terminals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let screen = render_text(&mut app, 111, 58);
    let title_line = screen
        .lines()
        .position(|line| line.contains("Safe intelligent disk organization"))
        .expect("home title should render");

    assert!(
        title_line <= 6,
        "home title rendered too low at line {title_line}\n{screen}"
    );
}

#[test]
fn home_layout_switches_to_a_concise_scan_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("node_modules")).expect("mkdir");
    fs::write(
        temp.path().join("node_modules").join("index.js"),
        vec![0; 2 * 1024 * 1024],
    )
    .expect("write");

    let mut app = app(temp.path().to_path_buf());
    let mut report =
        scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
    app.registry.annotate_entries(&mut report.entries);
    app.entries = report.entries;
    app.scan_summary = report.summary;
    app.build_plan_for_view(false);

    let screen = render_text(&mut app, 100, 28);
    println!("{screen}");

    assert!(screen.contains("Scan result"));
    assert!(screen.contains("Reclaimable"));
    assert!(screen.contains("Review cleanup items"));
    assert!(!screen.contains("Recent activity"));
}

#[test]
fn chinese_home_matches_the_primary_terminal_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = Workbench::new(
        vec![temp.path().to_path_buf()],
        Config::default(),
        RuleRegistry::builtin().expect("builtin rules"),
        I18n::new("zh-CN", BTreeMap::new(), builtin_language_packs()),
        Theme::dark(),
    );

    let screen = render_text(&mut app, 143, 41);
    let compact = screen
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();

    assert!(compact.contains("安全智能磁盘整理"));
    assert!(compact.contains("[s]扫描分析"));
    assert!(compact.contains("所有清理项都会先审阅"));
    assert!(!compact.contains('›'));
    assert!(!compact.contains("最近活动"));
    assert!(!compact.contains("尚未扫描"));
}

#[test]
fn single_key_shortcuts_start_primary_actions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());

    app.handle_key(key(KeyCode::Char('s')));
    assert!(app.is_scan_running());
    assert_eq!(app.view, View::Scan);
}

#[test]
fn scan_layout_keeps_selection_and_details_distinct() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("node_modules")).expect("mkdir");
    fs::write(
        temp.path().join("node_modules").join("index.js"),
        vec![0; 2 * 1024 * 1024],
    )
    .expect("write");

    let mut app = app(temp.path().to_path_buf());
    let mut report =
        scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
    app.registry.annotate_entries(&mut report.entries);
    app.entries = report.entries;
    app.scan_summary = report.summary;
    app.build_plan();

    let screen = render_text(&mut app, 120, 30);
    println!("{screen}");

    assert!(screen.contains("[ ]"));
    assert!(screen.contains("Preview"));
    assert!(screen.contains("space select"));
    assert!(screen.contains("Current item"));
    assert!(screen.contains("Matched rules"));
    assert!(screen.contains("Rule resolution"));
    assert!(!screen.contains("i inspect"));
}

#[test]
fn bulk_selection_leaves_review_items_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut review_hit = test_rule_hit("review");
    review_hit.confidence = Confidence::Medium;
    review_hit.default_selected = false;
    let as_of = chrono::Utc::now();
    let mut app = app(temp.path().to_path_buf());
    app.scan_as_of = as_of;
    app.entries = vec![
        ScanEntry {
            path: temp.path().join("eligible-cache"),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: Some(as_of - chrono::Duration::days(100)),
            rule_hits: vec![test_rule_hit("eligible")],
        },
        ScanEntry {
            path: temp.path().join("review-cache"),
            kind: EntryKind::Directory,
            size_bytes: 20,
            modified_at: Some(as_of - chrono::Duration::days(100)),
            rule_hits: vec![review_hit],
        },
    ];
    app.build_plan();

    let review_index = app
        .plan()
        .expect("plan")
        .items
        .iter()
        .position(|item| item.path.ends_with("review-cache"))
        .expect("review item");
    app.list_state.select(Some(review_index));
    app.toggle_scan_selection();
    let eligible_index = app
        .plan()
        .expect("plan")
        .items
        .iter()
        .position(|item| item.path.ends_with("eligible-cache"))
        .expect("eligible item");
    app.list_state.select(Some(eligible_index));
    app.toggle_scan_selection();

    app.toggle_all_scan_selection();
    assert!(
        app.plan()
            .expect("plan")
            .items
            .iter()
            .all(|item| item.selected)
    );
    assert!(app.status().contains("review item(s) unchanged"));

    app.toggle_all_scan_selection();
    let plan = app.plan().expect("plan");
    assert!(!plan.items[eligible_index].selected);
    assert!(plan.items[review_index].selected);
    assert_eq!(plan.summary.selected_count, 1);
}

#[test]
fn scan_layout_truncates_long_paths_without_hiding_decision_columns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.entries = vec![ScanEntry {
        path: temp
            .path()
            .join("very-long-generated-directory-name")
            .join("nested-cache")
            .join("another-long-segment")
            .join("artifact.bin"),
        kind: EntryKind::Directory,
        size_bytes: 12 * 1024 * 1024 * 1024,
        modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
        rule_hits: vec![test_rule_hit("generated")],
    }];
    app.build_plan();

    let screen = render_text(&mut app, 76, 22);
    println!("{screen}");

    assert!(screen.contains("[✓]"));
    assert!(screen.contains("12.00 GiB"));
    assert!(screen.contains("high"));
    assert!(screen.contains("…"));
}

#[test]
fn chinese_scan_layout_uses_translations_for_preview_labels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = Workbench::new(
        vec![temp.path().to_path_buf()],
        Config::default(),
        RuleRegistry::builtin().expect("builtin rules"),
        I18n::new("zh-CN", BTreeMap::new(), builtin_language_packs()),
        Theme::dark(),
    );
    app.entries = vec![ScanEntry {
        path: temp.path().join("target"),
        kind: EntryKind::Directory,
        size_bytes: 1024 * 1024,
        modified_at: None,
        rule_hits: vec![test_rule_hit("generated")],
    }];
    app.build_plan();

    let screen = render_text(&mut app, 120, 24);
    println!("{screen}");
    let compact = screen
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();

    assert!(compact.contains("预览"));
    assert!(compact.contains("当前项"));
    assert!(compact.contains("路径"));
}

#[test]
fn chinese_scan_progress_uses_refined_thin_rail_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = Workbench::new(
        vec![temp.path().to_path_buf()],
        Config::default(),
        RuleRegistry::builtin().expect("builtin rules"),
        I18n::new("zh-CN", BTreeMap::new(), builtin_language_packs()),
        Theme::light(),
    );
    app.dispatch(ActionRequest::Scan(ScanRequest::default()));
    app.scan_progress = Some(ScanTaskProgress {
        stage: ScanStage::Scanning,
        entries_total: 0,
        entries_scanned: 155_840,
        bytes_scanned: (12.74 * 1024.0 * 1024.0 * 1024.0) as u64,
        errors: 0,
        current_path: Some(
            temp.path()
                .join("node_modules")
                .join("@babel")
                .join("traverse")
                .join("lib")
                .join("context.js"),
        ),
    });

    let screen = render_text(&mut app, 111, 33);
    println!("{screen}");
    let compact = screen
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();

    assert!(compact.contains("正在扫描文件"));
    assert!(compact.contains("已扫描155840个条目"));
    assert!(compact.contains("已读取12.74GiB"));
    assert!(compact.contains("当前路径"));
    assert!(screen.contains('━'));
    assert!(screen.contains('─'));
    assert!(!screen.contains('█'));
    assert!(!screen.contains("155840 / 0"));
}

#[test]
fn slash_opens_command_palette() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));
    assert!(app.palette_open());
    assert_eq!(app.input(), "/");
}

#[test]
fn command_palette_tabs_wrap_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));
    let len = app.filtered_palette_commands().len();

    app.handle_key(key(KeyCode::BackTab));
    assert_eq!(app.palette_state.selected(), Some(len - 1));

    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.palette_state.selected(), Some(0));
}

#[test]
fn confirmation_supports_arrows_and_y_n_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.clean_waiting_for_confirmation = true;
    app.confirm_choice = ConfirmChoice::No;

    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.confirm_choice, ConfirmChoice::Yes);
    assert!(app.clean_waiting_for_confirmation);

    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.confirm_choice, ConfirmChoice::No);

    app.handle_key(key(KeyCode::Char('y')));
    assert_eq!(app.confirm_choice, ConfirmChoice::Yes);

    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.confirm_choice, ConfirmChoice::No);
}

#[test]
fn confirmation_enter_submits_current_choice() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.clean_waiting_for_confirmation = true;
    app.confirm_choice = ConfirmChoice::No;

    app.handle_key(key(KeyCode::Enter));

    assert!(!app.clean_waiting_for_confirmation);
    assert!(app.status().contains("cancelled"));
}

#[test]
fn confirmation_escape_always_cancels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.clean_waiting_for_confirmation = true;
    app.confirm_choice = ConfirmChoice::Yes;

    app.handle_key(key(KeyCode::Esc));

    assert!(!app.clean_waiting_for_confirmation);
    assert_eq!(app.confirm_choice, ConfirmChoice::No);
}

#[test]
fn confirmation_dialog_renders_in_small_terminals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.clean_waiting_for_confirmation = true;
    app.confirm_choice = ConfirmChoice::No;

    for (width, height) in [(40, 10), (80, 24), (194, 64)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
    }
}

#[test]
fn restore_view_requests_confirmation_for_selected_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.state_dir = temp.path().to_path_buf();
    write_execution_manifest(
        &ExecutionManifest {
            schema_version: EXECUTION_SCHEMA_VERSION.to_string(),
            run_id: "restore-test".to_string(),
            created_at: chrono::Utc::now(),
            plan_schema_version: "plan".to_string(),
            authorization: None,
            summary: ExecutionSummary {
                attempted: 1,
                succeeded: 1,
                failed: 0,
            },
            items: vec![ExecutionItem {
                path: temp.path().join("target"),
                planned_action: PlannedAction::Trash,
                status: ExecutionStatus::Trashed,
                rule_id: "test".to_string(),
                rollback_receipt: Some(RollbackReceipt {
                    method: "fake".to_string(),
                    note: "test".to_string(),
                    locator: Some("fake".to_string()),
                }),
                error: None,
            }],
        },
        &app.state_dir,
    )
    .expect("write manifest");

    app.dispatch(ActionRequest::Restore);
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(
        app.restore_waiting_for_confirmation.as_deref(),
        Some("restore-test")
    );
}

#[test]
fn scan_command_runs_in_background_and_finds_candidates() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("node_modules")).expect("mkdir");
    fs::write(
        temp.path().join("node_modules").join("a.js"),
        vec![0; 2 * 1024 * 1024],
    )
    .expect("write");

    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Scan(ScanRequest::default()));
    assert!(app.is_scan_running());
    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.input(), "/");

    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(!app.is_scan_running());
    assert!(app.usage_order.is_empty());
    assert_eq!(app.candidate_projection_entries_len, app.entries.len());
    let stages = app
        .scan_diagnostics
        .as_ref()
        .expect("scan diagnostics")
        .phases
        .iter()
        .map(|phase| phase.stage)
        .collect::<Vec<_>>();
    assert_eq!(
        stages,
        vec![
            ScanStage::Resolving,
            ScanStage::Scanning,
            ScanStage::Aggregating,
            ScanStage::Rules,
            ScanStage::Evidence,
            ScanStage::Plan,
        ]
    );
    assert!(!stages.contains(&ScanStage::Usage));
    assert_eq!(
        app.scan_explicit_roots,
        vec![
            temp.path()
                .canonicalize()
                .unwrap_or_else(|_| temp.path().to_path_buf())
        ]
    );
    app.dispatch(ActionRequest::Review);
    assert_eq!(app.plan().expect("plan").summary.selected_count, 0);
}

#[test]
fn real_budget_limited_scan_commits_read_only_results_without_planning() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("first"), b"first").expect("first");
    fs::write(temp.path().join("second"), b"second").expect("second");

    let mut app = app(temp.path().to_path_buf());
    app.config.scan.budgets.max_entries = 1;
    app.dispatch(ActionRequest::Scan(ScanRequest::default()));
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(!app.is_scan_running());
    assert_eq!(app.entries.len(), 1);
    assert!(!app.scan_budget_exceeded.is_empty());
    assert!(app.plan().is_none());
    assert!(app.status().contains("read-only"), "{}", app.status());
    let stages = app
        .scan_diagnostics
        .as_ref()
        .expect("scan diagnostics")
        .phases
        .iter()
        .map(|phase| phase.stage)
        .collect::<Vec<_>>();
    assert!(stages.contains(&ScanStage::Rules));
    assert!(!stages.contains(&ScanStage::Evidence));
    assert!(!stages.contains(&ScanStage::Plan));
}

#[test]
fn cancelled_scan_rejects_a_queued_prepared_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("queued-result"), b"result").expect("write");
    let report = scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
    assert!(!report.entries.is_empty());

    let (sender, receiver) = mpsc::channel();
    sender
        .send(TaskEvent::ScanFinished {
            job_id: 7,
            result: Ok(Box::new(PreparedScan {
                report,
                explicit_roots: vec![temp.path().to_path_buf()],
                global_scan: Default::default(),
                candidate_count: 0,
                candidate_entry_indices: Vec::new(),
                usage: None,
                planning: Ok(None),
            })),
            diagnostics: ScanDiagnostics::default(),
        })
        .expect("queue prepared scan");
    drop(sender);

    let mut app = app(temp.path().to_path_buf());
    app.scan_rx = Some(receiver);
    app.scan_cancel = Some(Arc::new(AtomicBool::new(false)));
    app.scan_job_id = Some(7);

    app.cancel_scan();
    app.poll_tasks();

    assert!(!app.is_scan_running());
    assert!(app.entries().is_empty());
    assert!(app.status().contains("cancelled"), "{}", app.status());
}

#[test]
fn stale_scan_job_cannot_replace_current_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("stale-result"), b"result").expect("write");
    let report = scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
    assert!(!report.entries.is_empty());

    let (sender, receiver) = mpsc::channel();
    sender
        .send(TaskEvent::ScanFinished {
            job_id: 7,
            result: Ok(Box::new(PreparedScan {
                report,
                explicit_roots: vec![temp.path().to_path_buf()],
                global_scan: Default::default(),
                candidate_count: 0,
                candidate_entry_indices: Vec::new(),
                usage: None,
                planning: Ok(None),
            })),
            diagnostics: ScanDiagnostics::default(),
        })
        .expect("queue stale scan");
    drop(sender);

    let mut app = app(temp.path().to_path_buf());
    app.scan_rx = Some(receiver);
    app.scan_cancel = Some(Arc::new(AtomicBool::new(false)));
    app.scan_job_id = Some(8);

    app.poll_tasks();

    assert!(!app.is_scan_running());
    assert!(app.entries().is_empty());
    assert_eq!(app.scan_job_id, None);
}

#[test]
fn completed_scan_atomically_commits_worker_resolved_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("resolved-result"), b"result").expect("write");
    let report = scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
    let expected_roots = report.summary.roots.clone();
    let explicit_root = temp
        .path()
        .canonicalize()
        .unwrap_or_else(|_| temp.path().to_path_buf());
    let global_scan = cleanr_core::GlobalScanEvidence {
        requested_kinds: vec![GlobalScanKind::TempFiles],
        locations: Vec::new(),
        os_managed: Vec::new(),
    };
    let (sender, receiver) = mpsc::channel();
    sender
        .send(TaskEvent::ScanFinished {
            job_id: 17,
            result: Ok(Box::new(PreparedScan {
                report,
                explicit_roots: vec![explicit_root.clone()],
                global_scan: global_scan.clone(),
                candidate_count: 0,
                candidate_entry_indices: Vec::new(),
                usage: None,
                planning: Ok(None),
            })),
            diagnostics: ScanDiagnostics::default(),
        })
        .expect("queue prepared scan");

    let mut app = app(temp.path().join("old-scope"));
    app.scan_rx = Some(receiver);
    app.scan_cancel = Some(Arc::new(AtomicBool::new(false)));
    app.scan_job_id = Some(17);
    app.poll_tasks();

    assert_eq!(app.roots, expected_roots);
    assert_eq!(app.scan_explicit_roots, vec![explicit_root]);
    assert_eq!(app.scan_global_evidence, global_scan);
    let diagnostics = app.task_log.last().expect("diagnostics log");
    assert!(diagnostics.contains("frame p95/max"), "{diagnostics}");
    assert!(!diagnostics.contains(&temp.path().display().to_string()));
}

#[test]
fn structured_scan_failure_preserves_previous_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let previous_root = temp.path().join("previous");
    let previous_explicit = temp.path().join("explicit");
    let previous_global = cleanr_core::GlobalScanEvidence {
        requested_kinds: vec![GlobalScanKind::AppCaches],
        locations: Vec::new(),
        os_managed: Vec::new(),
    };
    let (sender, receiver) = mpsc::channel();
    sender
        .send(TaskEvent::ScanFinished {
            job_id: 18,
            result: Err(ScanFailure::NoGlobalRoots),
            diagnostics: ScanDiagnostics::default(),
        })
        .expect("queue failed scan");

    let mut app = app(previous_root.clone());
    app.scan_explicit_roots = vec![previous_explicit.clone()];
    app.scan_global_evidence = previous_global.clone();
    app.scan_rx = Some(receiver);
    app.scan_cancel = Some(Arc::new(AtomicBool::new(false)));
    app.scan_job_id = Some(18);
    app.poll_tasks();

    assert_eq!(app.roots, vec![previous_root]);
    assert_eq!(app.scan_explicit_roots, vec![previous_explicit]);
    assert_eq!(app.scan_global_evidence, previous_global);
    assert!(app.status().contains("No known system cleanup locations"));
}

#[test]
fn scan_stall_watchdog_updates_once_per_second_and_progress_resets_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let (sender, receiver) = mpsc::channel();
    app.scan_rx = Some(receiver);
    app.scan_job_id = Some(19);
    let started_at = Instant::now();
    app.scan_started_at = Some(started_at);
    app.scan_phase_started_at = Some(started_at);
    app.scan_last_progress_at = Some(started_at);
    app.scan_progress = Some(ScanTaskProgress {
        stage: ScanStage::Scanning,
        entries_total: 0,
        entries_scanned: 12,
        bytes_scanned: 4096,
        errors: 1,
        current_path: Some(PathBuf::from("/private/secret/cache")),
    });

    assert!(!app.update_scan_stall_at(started_at + Duration::from_secs(1)));
    assert!(app.update_scan_stall_at(started_at + Duration::from_secs(2)));
    assert!(app.status().contains("2s"), "{}", app.status());
    assert!(
        !app.status().contains("/private/secret"),
        "{}",
        app.status()
    );
    assert!(!app.update_scan_stall_at(started_at + Duration::from_millis(2_900)));
    assert!(app.update_scan_stall_at(started_at + Duration::from_secs(3)));

    sender
        .send(TaskEvent::ScanProgress {
            job_id: 19,
            progress: ScanTaskProgress {
                stage: ScanStage::Evidence,
                entries_total: 12,
                entries_scanned: 12,
                bytes_scanned: 4096,
                errors: 1,
                current_path: None,
            },
        })
        .expect("progress");
    assert!(app.poll_tasks());
    assert_eq!(app.scan_stall_reported_seconds, None);
    assert_eq!(
        app.scan_progress.as_ref().map(|progress| progress.stage),
        Some(ScanStage::Evidence)
    );

    sender
        .send(TaskEvent::ScanProgress {
            job_id: 19,
            progress: ScanTaskProgress {
                stage: ScanStage::Scanning,
                entries_total: 0,
                entries_scanned: 13,
                bytes_scanned: 8192,
                errors: 1,
                current_path: Some(PathBuf::from("/private/secret/stale")),
            },
        })
        .expect("stale progress");
    app.poll_tasks();
    assert_eq!(
        app.scan_progress.as_ref().map(|progress| progress.stage),
        Some(ScanStage::Evidence)
    );
}

#[test]
fn global_scan_request_preserves_current_scope_until_worker_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let original_roots = app.roots.clone();
    let original_evidence = app.scan_global_evidence.clone();

    app.dispatch(ActionRequest::Scan(ScanRequest::global(vec![
        GlobalScanKind::TempFiles,
    ])));

    assert!(app.is_scan_running());
    assert_eq!(app.roots, original_roots);
    assert_eq!(app.scan_global_evidence, original_evidence);
    app.cancel_scan();
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!app.is_scan_running());
    assert_eq!(app.roots, original_roots);
    assert_eq!(app.scan_global_evidence, original_evidence);
}

#[test]
fn tui_analysis_suppresses_candidates_from_unrequested_global_kinds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let scan_root = temp.path().join("cache");
    let pnpm = scan_root.join("pnpm");
    fs::create_dir_all(&pnpm).expect("global locations");
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

    let mut app = app(scan_root.clone());
    app.roots = resolved.roots.clone();
    app.scan_global_evidence = global_scan_evidence(&request, &[], &resolved, &app.roots);
    app.entries = vec![ScanEntry {
        path: pnpm,
        kind: EntryKind::Directory,
        size_bytes: 1024,
        modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
        rule_hits: vec![test_rule_hit("pnpm-cache")],
    }];

    app.build_plan();

    let analysis = app.analysis.as_ref().expect("analysis");
    assert_eq!(
        analysis.scan.global.requested_kinds,
        vec![GlobalScanKind::AppCaches]
    );
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
    assert_eq!(app.plan().expect("plan").summary.candidate_count, 0);
}

#[test]
fn scan_view_can_render_selection_beyond_old_candidate_cap() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.entries = (0..501)
        .map(|index| ScanEntry {
            path: temp.path().join(format!("candidate-{index:03}")),
            kind: EntryKind::File,
            size_bytes: 1,
            modified_at: None,
            rule_hits: vec![test_rule_hit("generated")],
        })
        .collect();
    app.build_plan();
    app.list_state.select(Some(500));
    let selected_name = app.plan().expect("plan").items[500]
        .path
        .file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned();

    let screen = render_text(&mut app, 120, 24);

    assert!(screen.contains(&selected_name), "{screen}");
}

#[test]
fn scan_view_virtualizes_ten_thousand_candidates_and_keeps_last_selected_visible() {
    let root = PathBuf::from("/workspace");
    let mut app = app(root.clone());
    let candidate_count = 10_000usize;
    app.entries = (0..candidate_count)
        .map(|index| ScanEntry {
            path: root.join(format!("candidate-{index:05}")),
            kind: EntryKind::File,
            size_bytes: u64::try_from(index).expect("candidate index fits in u64"),
            modified_at: None,
            rule_hits: vec![test_rule_hit("generated")],
        })
        .collect();
    app.view = View::Scan;
    app.list_state.select(Some(candidate_count - 1));

    let screen = render_text(&mut app, 120, 24);

    assert!(screen.contains("candidate-09999"), "{screen}");
    assert_eq!(app.list_state.selected(), Some(candidate_count - 1));
    assert!(app.list_state.offset() > 0);
}

#[test]
#[ignore = "manual local render evidence; set CLEANR_BENCH_CANDIDATES/CLEANR_BENCH_FRAMES"]
fn scan_view_render_performance_keeps_large_candidate_sets_off_the_frame_path() {
    let candidate_count = std::env::var("CLEANR_BENCH_CANDIDATES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10_000)
        .max(1);
    let frames = std::env::var("CLEANR_BENCH_FRAMES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(200)
        .max(50);
    let root = PathBuf::from("/cleanr-render-fixture");
    let mut app = app(root.clone());
    app.entries = (0..candidate_count)
        .map(|index| ScanEntry {
            path: root.join(format!("candidate-{index:08}")),
            kind: EntryKind::File,
            size_bytes: u64::try_from(index).unwrap_or(u64::MAX),
            modified_at: None,
            rule_hits: vec![test_rule_hit("render-benchmark")],
        })
        .collect();
    app.build_plan();
    app.view = View::Scan;
    app.list_state.select(Some(candidate_count - 1));

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    for _ in 0..10 {
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("warm render frame");
    }

    let mut samples = Vec::with_capacity(frames);
    for frame_index in 0..frames {
        let selected = candidate_count - 1 - frame_index % candidate_count.min(16);
        app.list_state.select(Some(selected));
        let started = Instant::now();
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render frame");
        samples.push(started.elapsed());
    }

    samples.sort_unstable();
    let p95_rank = (samples.len() * 95).div_ceil(100);
    let p95 = samples[p95_rank.saturating_sub(1)];
    let max = *samples.last().expect("render samples");
    let total = samples.iter().copied().sum::<Duration>();
    let mean = total / u32::try_from(samples.len()).expect("frame count fits u32");
    eprintln!(
        "cleanr-render-benchmark candidates={candidate_count} frames={} width=120 height=40 mean_us={} p95_us={} max_us={}",
        samples.len(),
        mean.as_micros(),
        p95.as_micros(),
        max.as_micros(),
    );

    let final_offset = (frames - 1) % candidate_count.min(16);
    assert_eq!(
        app.list_state.selected(),
        Some(candidate_count - 1 - final_offset)
    );
    assert!(app.list_state.offset() > 0 || candidate_count <= 40);
}

#[test]
fn restore_view_can_render_selection_beyond_old_history_cap() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.execution_manifests = (0..21)
        .map(|index| ExecutionManifest {
            schema_version: EXECUTION_SCHEMA_VERSION.to_string(),
            run_id: format!("run-{index:02}"),
            created_at: chrono::Utc::now(),
            plan_schema_version: "plan".to_string(),
            authorization: None,
            summary: ExecutionSummary {
                attempted: 1,
                succeeded: 1,
                failed: 0,
            },
            items: vec![],
        })
        .collect();
    app.view = View::Restore;
    app.list_state.select(Some(20));

    let screen = render_text(&mut app, 120, 24);

    assert!(screen.contains("run-20"), "{screen}");
}

#[test]
fn cleanup_success_starts_background_refresh_scan() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_dir = temp.path().join("state");
    fs::create_dir(temp.path().join("node_modules")).expect("mkdir");
    fs::write(
        temp.path().join("node_modules").join("index.js"),
        vec![0; 2 * 1024 * 1024],
    )
    .expect("write");
    let mut app = app(temp.path().to_path_buf());
    app.state_dir = state_dir;
    let report = scan_paths(&app.roots, &ScanOptions::default()).expect("scan");
    app.entries = report.entries;
    app.registry.annotate_entries(&mut app.entries);
    app.build_plan();
    app.toggle_all_scan_selection();
    let executor = FakeTrashExecutor::default();

    app.clean_with_executor(CleanupIntent::ExplicitUserConfirmation, &executor);

    assert!(app.is_scan_running());
    assert!(app.status_after_scan.is_some());
}

#[test]
fn cleanup_failure_surfaces_item_error_without_starting_refresh_scan() {
    struct FailingTrashExecutor;

    impl cleanr_tasks::CleanupExecutor for FailingTrashExecutor {
        fn trash(&self, _path: &std::path::Path) -> anyhow::Result<RollbackReceipt> {
            anyhow::bail!("simulated trash failure");
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let state_dir = temp.path().join("state");
    let target = temp.path().join("node_modules");
    fs::create_dir(&target).expect("mkdir");
    fs::write(target.join("index.js"), vec![0; 2 * 1024 * 1024]).expect("write");
    let mut app = app(temp.path().to_path_buf());
    app.state_dir = state_dir;
    let report = scan_paths(&app.roots, &ScanOptions::default()).expect("scan");
    app.entries = report.entries;
    app.registry.annotate_entries(&mut app.entries);
    app.build_plan();
    app.toggle_all_scan_selection();

    app.clean_with_executor(
        CleanupIntent::ExplicitUserConfirmation,
        &FailingTrashExecutor,
    );

    assert!(!app.is_scan_running());
    assert!(target.exists());
    assert!(app.status().contains("simulated trash failure"));
    assert!(app.status().contains("Nothing was moved to trash"));
    assert_eq!(app.execution_manifests[0].summary.succeeded, 0);
    assert_eq!(app.execution_manifests[0].summary.failed, 1);
}

#[test]
fn usage_scan_exposes_live_progress() {
    let temp = tempfile::tempdir().expect("tempdir");
    for index in 0..128 {
        fs::write(temp.path().join(format!("file-{index}")), b"1234").expect("write");
    }

    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Usage(ScanRequest::default()));
    assert_eq!(app.view, View::Usage);

    for _ in 0..50 {
        app.poll_tasks();
        if app
            .scan_progress
            .as_ref()
            .is_some_and(|progress| progress.entries_total > 0)
            || !app.is_scan_running()
        {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }

    assert!(
        app.scan_progress
            .as_ref()
            .is_some_and(|progress| progress.entries_total > 0)
            || !app.is_scan_running()
    );
}

#[test]
fn usage_scan_prepares_the_usage_projection_on_demand() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("artifact"), b"1234").expect("write");
    let mut app = app(temp.path().to_path_buf());

    app.dispatch(ActionRequest::Usage(ScanRequest::default()));
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(!app.is_scan_running());
    assert!(!app.usage_order.is_empty());
    assert_eq!(app.usage_order.len(), app.usage_descendant_counts.len());
    assert!(
        app.scan_diagnostics
            .as_ref()
            .expect("scan diagnostics")
            .phases
            .iter()
            .any(|phase| phase.stage == ScanStage::Usage)
    );
}

#[test]
fn adaptive_rects_never_exceed_terminal_area() {
    let area = Rect::new(3, 5, 40, 10);
    let content = fluid_content_rect(area, 220, 30);
    let popup = centered_bounded_rect(area, 100, 40, 120);
    let bottom = bottom_bounded_rect(area, 100, 40, 120);

    for rect in [content, popup, bottom] {
        assert!(rect.x >= area.x);
        assert!(rect.y >= area.y);
        assert!(rect.right() <= area.right());
        assert!(rect.bottom() <= area.bottom());
    }
}

#[test]
fn visible_list_window_preserves_visible_offset_and_clamps_selection() {
    let mut state = ListState::default().with_offset(4).with_selected(Some(6));

    assert_eq!(visible_list_window(&mut state, 10, 3), 4..7);
    assert_eq!(state.offset(), 4);
    assert_eq!(state.selected(), Some(6));

    state.select(Some(8));
    assert_eq!(visible_list_window(&mut state, 10, 3), 6..9);
    assert_eq!(state.offset(), 6);

    state.select(Some(1));
    assert_eq!(visible_list_window(&mut state, 10, 3), 1..4);
    assert_eq!(state.offset(), 1);

    *state.offset_mut() = usize::MAX;
    state.select(Some(usize::MAX));
    assert_eq!(visible_list_window(&mut state, 10, 4), 6..10);
    assert_eq!(state.offset(), 6);
    assert_eq!(state.selected(), Some(9));

    assert_eq!(visible_list_window(&mut state, 0, 4), 0..0);
    assert_eq!(state.offset(), 0);
    assert_eq!(state.selected(), None);
}

#[test]
fn usage_rebuild_caches_sorted_root_children_and_list_length() {
    let root = PathBuf::from("/workspace");
    let mut app = app(root.clone());
    app.entries = vec![
        ScanEntry {
            path: root.join("small"),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: root.join("small/nested-but-larger"),
            kind: EntryKind::File,
            size_bytes: 999,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: root.join("large"),
            kind: EntryKind::Directory,
            size_bytes: 30,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: root.join("medium"),
            kind: EntryKind::File,
            size_bytes: 20,
            modified_at: None,
            rule_hits: vec![],
        },
    ];

    app.rebuild_usage_order();
    app.view = View::Usage;

    assert_eq!(app.usage_order, vec![2, 3, 0]);
    assert_eq!(app.usage_max_size, 30);
    assert_eq!(app.list_len(), 3);
}

#[test]
fn usage_renders_at_small_and_large_terminal_sizes() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("target")).expect("mkdir");
    fs::write(temp.path().join("target").join("artifact"), vec![0; 4096]).expect("write");
    let mut app = app(temp.path().to_path_buf());
    let report = scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
    app.entries = report.entries;
    app.scan_summary = report.summary;
    app.show_usage();

    for (width, height) in [(40, 10), (80, 24), (194, 64)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
    }
}

#[test]
fn usage_details_count_recursive_directory_entries() {
    let root = PathBuf::from("/workspace/target");
    let entries = vec![
        ScanEntry {
            path: root.clone(),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: root.join("debug"),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: root.join("debug/app"),
            kind: EntryKind::File,
            size_bytes: 10,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: PathBuf::from("/workspace/README.md"),
            kind: EntryKind::File,
            size_bytes: 1,
            modified_at: None,
            rule_hits: vec![],
        },
    ];

    assert_eq!(usage_descendant_count(&entries, &entries[0]), 2);
    assert_eq!(usage_descendant_count(&entries, &entries[2]), 0);
}

#[test]
fn usage_projection_merges_unordered_multi_root_descendants_and_stops_at_gaps() {
    let first_root = PathBuf::from("/workspace-a");
    let second_root = PathBuf::from("/workspace-b");
    let entries = vec![
        ScanEntry {
            path: first_root.join("top/child/artifact"),
            kind: EntryKind::File,
            size_bytes: 3,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: second_root.join("other/artifact"),
            kind: EntryKind::File,
            size_bytes: 2,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: first_root.join("top"),
            kind: EntryKind::Directory,
            size_bytes: 20,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: first_root.join("top/orphan/artifact"),
            kind: EntryKind::File,
            size_bytes: 8,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: second_root.join("other"),
            kind: EntryKind::Directory,
            size_bytes: 10,
            modified_at: None,
            rule_hits: vec![],
        },
        ScanEntry {
            path: first_root.join("top/child"),
            kind: EntryKind::Directory,
            size_bytes: 3,
            modified_at: None,
            rule_hits: vec![],
        },
    ];

    let projection = build_usage_projection(&entries, &[first_root, second_root]);
    let top_position = projection
        .order
        .iter()
        .position(|index| entries[*index].path == std::path::Path::new("/workspace-a/top"))
        .expect("first root child");
    let other_position = projection
        .order
        .iter()
        .position(|index| entries[*index].path == std::path::Path::new("/workspace-b/other"))
        .expect("second root child");

    assert_eq!(projection.descendant_counts[top_position], 2);
    assert_eq!(projection.descendant_counts[other_position], 1);
}

#[test]
fn ime_guard_stays_inside_terminal_and_off_the_last_row() {
    for area in [
        Rect::new(0, 0, 80, 24),
        Rect::new(3, 5, 40, 10),
        Rect::new(0, 0, 1, 1),
    ] {
        let position = ime_guard_position(area);
        assert!(position.x >= area.x);
        assert!(position.y >= area.y);
        assert!(position.x < area.right().max(area.x + 1));
        assert!(position.y < area.bottom().max(area.y + 1));
        if area.height > 1 {
            assert!(position.y < area.bottom().saturating_sub(1));
        }
    }
}

#[test]
fn command_cursor_accounts_for_wide_chinese_input() {
    let area = Rect::new(1, 20, 40, 1);
    assert_eq!(
        command_cursor_position(area, ":中文"),
        Some(Position::new(8, 20))
    );
}

#[test]
fn command_input_view_keeps_long_and_wide_character_cursors_visible() {
    let long_input = "/scan /a/very/long/path/that/ends/here";
    let (long_view, long_cursor) = command_input_view(long_input, long_input.len(), 12);
    assert!(long_view.starts_with('…'));
    assert!(display_width(&long_view) <= 12);
    assert_eq!(long_cursor, display_width(&long_view));

    let wide_input = "/scan 项目/非常长的缓存目录/后缀";
    let wide_cursor = wide_input.find("/后缀").expect("wide cursor boundary");
    let (wide_view, wide_column) = command_input_view(wide_input, wide_cursor, 12);
    assert!(wide_view.starts_with('…'));
    assert!(wide_view.contains("缓存目录"));
    assert!(display_width(&wide_view) <= 12);
    assert!(wide_column < display_width(&wide_view));
    assert!(wide_column < 12);
}

#[test]
fn text_truncation_respects_terminal_display_width() {
    let text = "缓存/very-long-directory-name/target";
    let truncated = truncate_text(text, 12);

    assert!(truncated.contains('…'));
    assert!(display_width(&truncated) <= 12);
}

#[test]
fn scan_loading_bar_is_continuous_activity_not_percent_progress() {
    let samples = [
        scan_loading_bar_sample(40, 0, Theme::dark()),
        scan_loading_bar_sample(40, 4, Theme::dark()),
        scan_loading_bar_sample(40, 16, Theme::dark()),
    ];

    assert!(samples.iter().all(|sample| display_width(sample) == 40));
    assert!(samples.iter().all(|sample| sample.contains('─')));
    assert!(samples.iter().all(|sample| sample.contains('━')));
    assert!(samples.iter().all(|sample| !sample.contains('█')));
    assert_ne!(samples[0], samples[1]);
}

#[test]
fn languages_command_reports_loaded_packs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Languages);
    assert_eq!(app.view, View::Languages);
    assert_eq!(app.list_state.selected(), Some(0));
    assert!(app.status().contains("Active locale"));
}

#[test]
fn context_views_support_arrow_navigation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());

    app.dispatch(ActionRequest::Languages);
    assert_eq!(app.list_state.selected(), Some(0));

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.list_state.selected(), Some(1));

    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn home_shortcuts_hide_context_views() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());

    assert!(app.is_home());

    app.dispatch(ActionRequest::Languages);
    assert_eq!(app.view, View::Languages);

    app.handle_key(key(KeyCode::Char('h')));
    assert!(app.is_home());

    app.dispatch(ActionRequest::Usage(ScanRequest::default()));
    assert_eq!(app.view, View::Usage);

    app.handle_key(key(KeyCode::Esc));
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    app.handle_key(key(KeyCode::Esc));
    assert!(app.is_home());
}

#[test]
fn toggling_selection_updates_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("node_modules")).expect("mkdir");
    fs::write(
        temp.path().join("node_modules").join("a.js"),
        vec![0; 2 * 1024 * 1024],
    )
    .expect("write");

    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Scan(ScanRequest::default()));
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    let plan = app.plan().expect("plan");
    let initial = plan.summary.selected_count;
    app.handle_key(key(KeyCode::Char(' ')));
    let plan = app.plan().expect("plan");
    assert_ne!(plan.summary.selected_count, initial);
}

#[test]
fn rebuilding_a_plan_preserves_the_user_selection_from_one_analysis_report() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let old_cache = temp.path().join("old-cache");
    let recent_cache = temp.path().join("recent-cache");
    app.entries = vec![
        ScanEntry {
            path: old_cache.clone(),
            kind: EntryKind::Directory,
            size_bytes: 1024,
            modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
            rule_hits: vec![test_rule_hit("generated")],
        },
        ScanEntry {
            path: recent_cache.clone(),
            kind: EntryKind::Directory,
            size_bytes: 1024,
            modified_at: Some(app.scan_as_of - chrono::Duration::days(1)),
            rule_hits: vec![test_rule_hit("generated")],
        },
    ];

    app.build_plan();
    let old_candidate_id = app
        .analysis
        .as_ref()
        .expect("analysis")
        .candidates
        .iter()
        .find(|candidate| candidate.local_path == old_cache)
        .expect("old candidate")
        .id
        .clone();
    let recent_candidate_id = app
        .analysis
        .as_ref()
        .expect("analysis")
        .candidates
        .iter()
        .find(|candidate| candidate.local_path == recent_cache)
        .expect("recent candidate")
        .id
        .clone();
    assert!(
        app.plan()
            .expect("plan")
            .items
            .iter()
            .find(|item| item.path == old_cache)
            .expect("old item")
            .selected
    );
    assert!(
        !app.plan()
            .expect("plan")
            .items
            .iter()
            .find(|item| item.path == recent_cache)
            .expect("recent item")
            .selected
    );

    app.list_state.select(Some(0));
    app.toggle_scan_selection();
    app.list_state.select(Some(1));
    app.toggle_scan_selection();
    assert!(!app.selection.candidate_ids.contains(&old_candidate_id));
    assert!(app.selection.candidate_ids.contains(&recent_candidate_id));

    app.build_plan();
    let rebuilt = app.plan().expect("rebuilt plan");
    assert!(
        !rebuilt
            .items
            .iter()
            .find(|item| item.path == old_cache)
            .expect("old item")
            .selected
    );
    assert!(
        rebuilt
            .items
            .iter()
            .find(|item| item.path == recent_cache)
            .expect("recent item")
            .selected
    );
    assert_eq!(
        app.analysis
            .as_ref()
            .expect("analysis remains")
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == old_cache)
            .expect("old candidate")
            .id,
        old_candidate_id
    );
}

#[test]
fn plan_build_errors_clear_stale_plan_and_surface_the_reason() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.entries = vec![ScanEntry {
        path: temp.path().join("old-cache"),
        kind: EntryKind::Directory,
        size_bytes: 1024,
        modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
        rule_hits: vec![test_rule_hit("generated")],
    }];
    app.build_plan();
    assert!(app.plan().is_some());

    app.analysis
        .as_mut()
        .expect("analysis")
        .scan
        .budget_exceeded
        .push(ScanBudgetExceeded::EntryCount {
            limit: 1,
            observed: 2,
        });
    app.build_plan();
    assert!(app.plan().is_none());
    assert!(app.status().contains("scan budget was exceeded"));

    let analysis = app.analysis.as_mut().expect("analysis");
    analysis.scan.budget_exceeded.clear();
    analysis.schema_version = "cleanr.analysis.v999".to_string();
    app.build_plan();
    assert!(app.plan().is_none());
    assert!(app.status().contains("unsupported analysis report schema"));
}

#[test]
fn budget_limited_scan_rejects_plan_export_and_cleanup_with_read_only_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.entries = vec![ScanEntry {
        path: temp.path().join("old-cache"),
        kind: EntryKind::Directory,
        size_bytes: 1024,
        modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
        rule_hits: vec![test_rule_hit("generated")],
    }];
    app.scan_budget_exceeded = vec![ScanBudgetExceeded::EntryCount {
        limit: 1,
        observed: 2,
    }];
    let output = temp.path().join("must-not-exist.json");

    app.build_plan();
    assert!(app.plan().is_none());
    assert!(app.status().contains("read-only"), "{}", app.status());
    app.export_plan(Some(output.clone()));
    assert!(!output.exists());
    app.request_cleanup(CleanupIntent::ExplicitUserConfirmation);
    assert!(app.operation_rx.is_none());
    assert!(app.status().contains("read-only"), "{}", app.status());

    app.toggle_scan_selection();
    assert!(app.status().contains("read-only"), "{}", app.status());
    app.toggle_all_scan_selection();
    assert!(app.status().contains("read-only"), "{}", app.status());

    app.entries.clear();
    app.build_plan();
    assert!(app.status().contains("read-only"), "{}", app.status());
}

#[test]
fn palette_selection_dispatches_non_scan_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());

    app.handle_key(key(KeyCode::Char('/')));
    assert!(app.palette_open());

    for ch in "langu".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert!(!app.palette_open());
    assert!(app.status().contains("Active locale"));
}

#[test]
fn palette_enter_dispatches_filtered_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());

    app.handle_key(key(KeyCode::Char('/')));
    for ch in "langu".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.view, View::Languages);
    assert!(app.status().contains("Active locale"));
}

#[test]
fn palette_global_filter_dispatches_global_scan() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let original_root = temp.path().to_path_buf();

    app.handle_key(key(KeyCode::Char('/')));
    for ch in "global".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert!(!app.palette_open());
    assert!(app.is_scan_running());
    assert_eq!(app.roots, vec![original_root.clone()]);
    app.cancel_scan();
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.roots, vec![original_root]);
}

#[test]
fn palette_invocation_keeps_flags_and_drops_placeholders() {
    assert_eq!(
        palette_command_invocation("/clean --confirm"),
        "/clean --confirm"
    );
    assert_eq!(palette_command_invocation("/scan [path...]"), "/scan");
    assert_eq!(
        palette_command_invocation("/scan --global"),
        "/scan --global"
    );
    assert_eq!(
        palette_command_invocation("/export-plan [path]"),
        "/export-plan"
    );
}

#[test]
fn command_mode_ctrl_w_deletes_word_and_ctrl_u_clears_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));
    for ch in "scan /tmp".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.input(), "/scan /tmp");

    app.handle_key(ctrl(KeyCode::Char('w')));
    assert_eq!(app.input(), "/scan ");

    app.handle_key(ctrl(KeyCode::Char('u')));
    assert_eq!(app.input(), "/");
}

#[test]
fn command_editor_supports_middle_insertion_cursor_movement_and_delete() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));
    for ch in "scan".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }

    assert_eq!(app.input(), "/scan");
    assert_eq!(app.input_cursor, app.input().len());

    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.input_cursor, 4);
    app.handle_key(key(KeyCode::Char('X')));
    assert_eq!(app.input(), "/scaXn");

    app.handle_key(key(KeyCode::Home));
    assert_eq!(app.input_cursor, 1, "home must preserve the command prefix");
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.input_cursor, 2);
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(app.input(), "/saXn");
    assert_eq!(app.input_cursor, 2);

    app.handle_key(key(KeyCode::End));
    assert_eq!(app.input_cursor, app.input().len());
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.input_cursor, app.input().len());
}

#[test]
fn command_editor_backspace_respects_unicode_boundaries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));
    for ch in "扫描🧹".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }

    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.input(), "/扫描");
    assert!(app.input().is_char_boundary(app.input_cursor));

    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.input(), "/描");
    assert_eq!(app.input_cursor, 1);
}

#[test]
fn bracketed_paste_is_folded_into_one_safe_command_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));

    app.handle_paste("scan\t/tmp\r\n/next\u{7}  path");

    assert_eq!(app.input(), "/scan /tmp /next path");
    assert!(!app.input().chars().any(char::is_control));
    assert!(matches!(app.mode, Mode::Command));
    assert_eq!(app.input_cursor, app.input().len());
}

#[test]
fn command_mode_ctrl_c_closes_before_ctrl_c_quits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Char('s')));

    app.handle_key(ctrl(KeyCode::Char('c')));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.input(), "");
    assert!(!app.palette_open());
    assert!(!app.should_quit);

    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(app.should_quit);
}

#[test]
fn repeat_navigation_works_but_repeat_clean_and_quit_are_ignored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut rules_app = app(temp.path().to_path_buf());
    rules_app.dispatch(ActionRequest::Rules);
    assert!(rules_app.list_len() >= 3);

    rules_app.handle_key(repeat(KeyCode::Down));
    rules_app.handle_key(repeat(KeyCode::Char('j')));
    assert_eq!(rules_app.list_state.selected(), Some(2));
    rules_app.handle_key(repeat(KeyCode::Char('q')));
    assert!(!rules_app.should_quit);

    let mut cleanup_app = app(temp.path().to_path_buf());
    cleanup_app.config.cleanup.require_confirm = true;
    cleanup_app.entries = vec![ScanEntry {
        path: temp.path().join("old-cache"),
        kind: EntryKind::Directory,
        size_bytes: 1024,
        modified_at: Some(cleanup_app.scan_as_of - chrono::Duration::days(100)),
        rule_hits: vec![test_rule_hit("generated")],
    }];
    cleanup_app.build_plan();
    assert!(cleanup_app.plan().expect("plan").summary.selected_count > 0);

    cleanup_app.handle_key(repeat(KeyCode::Char('c')));
    cleanup_app.handle_key(repeat(KeyCode::Char('q')));

    assert!(!cleanup_app.clean_waiting_for_confirmation);
    assert!(!cleanup_app.should_quit);
}

#[test]
fn gg_goto_first_and_goto_last() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Rules);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.list_state.selected(), Some(2));

    app.handle_key(key(KeyCode::Char('g')));
    app.handle_key(key(KeyCode::Char('g')));
    assert_eq!(app.list_state.selected(), Some(0));

    app.handle_key(key(KeyCode::Char('G')));
    assert_eq!(app.list_state.selected(), Some(app.list_len() - 1));
}

#[test]
fn invalid_gg_second_key_is_processed_normally() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Rules);
    assert_eq!(app.list_state.selected(), Some(0));

    app.handle_key(key(KeyCode::Char('g')));
    assert_eq!(app.pending_key, Some('g'));
    app.handle_key(key(KeyCode::Char('j')));

    assert_eq!(app.pending_key, None);
    assert_eq!(app.list_state.selected(), Some(1));
}

#[test]
fn count_prefix_moves_multiple_lines() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Rules);
    assert!(app.list_len() >= 3);

    for ch in "2".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.list_state.selected(), Some(2));
}

#[test]
fn count_prefix_goto_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Rules);

    for ch in "2".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Char('G')));
    assert_eq!(app.list_state.selected(), Some(1));
}

#[test]
fn toggle_all_selects_and_deselects_scan_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir(temp.path().join("node_modules")).expect("mkdir");
    fs::write(
        temp.path().join("node_modules").join("a.js"),
        vec![0; 2 * 1024 * 1024],
    )
    .expect("write");

    let mut app = app(temp.path().to_path_buf());
    app.dispatch(ActionRequest::Scan(ScanRequest::default()));
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    let plan = app.plan().expect("plan");
    let initial = plan.summary.selected_count;

    app.handle_key(key(KeyCode::Char('a')));
    let plan = app.plan().expect("plan");
    assert_ne!(plan.summary.selected_count, initial);

    app.handle_key(key(KeyCode::Char('%')));
    let plan = app.plan().expect("plan");
    assert_eq!(plan.summary.selected_count, initial);
}
