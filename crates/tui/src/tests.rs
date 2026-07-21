use super::*;
use crate::{
    app::{ConfirmChoice, Mode, View},
    commands::{ActionRequest, CleanupIntent, palette_command_invocation},
    views::{
        bottom_bounded_rect, centered_bounded_rect, command_cursor_position, command_input_view,
        display_width, fluid_content_rect, ime_guard_position, render, scan_loading_bar_sample,
        truncate_text, usage_descendant_count, visible_list_window,
    },
};
use cleanr_config::Config;
use cleanr_core::{
    Confidence, EXECUTION_SCHEMA_VERSION, EntryKind, ExecutionItem, ExecutionManifest,
    ExecutionStatus, ExecutionSummary, GlobalScanKind, PlannedAction, RollbackReceipt, RuleHit,
    RuleTrust, ScanEntry, ScanRequest,
};
use cleanr_fs::{ScanOptions, ScanPhase, ScanProgress, scan_paths};
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
use std::{collections::BTreeMap, fs, path::PathBuf, thread, time::Duration};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    }
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
    assert!(screen.contains("Scan & analyze"));
    assert!(screen.contains("Every item is reviewed first"));
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
    assert!(compact.contains("扫描分析"));
    assert!(compact.contains("所有清理项都会先审阅"));
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
    app.scan_progress = Some(ScanProgress {
        phase: ScanPhase::Scanning,
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
    app.dispatch(ActionRequest::Review);
    assert_eq!(app.plan().expect("plan").summary.selected_count, 0);
}

#[test]
fn global_scan_request_replaces_current_roots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = app(temp.path().to_path_buf());
    let expected_temp = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());

    app.dispatch(ActionRequest::Scan(ScanRequest::global(vec![
        GlobalScanKind::TempFiles,
    ])));

    assert!(app.is_scan_running());
    assert_eq!(app.roots, vec![expected_temp]);
    app.cancel_scan();
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
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
    let expected_temp = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());

    app.handle_key(key(KeyCode::Char('/')));
    for ch in "global".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert!(!app.palette_open());
    assert!(app.is_scan_running());
    assert!(app.roots.contains(&expected_temp));
    assert!(!app.roots.contains(&original_root));
    app.cancel_scan();
    for _ in 0..50 {
        app.poll_tasks();
        if !app.is_scan_running() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
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
