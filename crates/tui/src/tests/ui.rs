use super::*;
use ratatui::{buffer::Buffer, style::Modifier};

fn fixture(root: PathBuf) -> Workbench {
    let mut app = app(root.clone());
    let sizes = [7, 1024, 4 * 1024 * 1024, 12 * 1024 * 1024 * 1024];
    app.entries = Arc::new(
        (0..40)
            .map(|i| ScanEntry {
                path: root.join(format!(
                    "中文📦-cache-{i:02}-e\u{301}-long-directory/artifact-{i:02}"
                )),
                kind: EntryKind::Directory,
                size_bytes: sizes[i % sizes.len()],
                modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
                rule_hits: vec![test_rule_hit("generated")],
            })
            .collect(),
    );
    app.config.cleanup.require_confirm = true;
    app.build_plan();
    app.scan_view.sort = crate::projection::ScanSort::Path;
    app.ensure_scan_view_projection();
    app
}

fn buffer(app: &mut Workbench, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// Search terminal cells, not UTF-8 byte offsets or whitespace-stripped screenshots.
fn find_ascii(buffer: &Buffer, text: &str) -> Vec<(u16, u16)> {
    let mut positions = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        for x in buffer.area.x..buffer.area.right() {
            if x + text.len() as u16 > buffer.area.right() {
                break;
            }
            if text
                .chars()
                .enumerate()
                .all(|(i, ch)| buffer[(x + i as u16, y)].symbol() == ch.to_string())
            {
                positions.push((x, y));
            }
        }
    }
    positions
}

fn compact(buffer: &Buffer) -> String {
    buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

#[test]
fn ui_scan_columns_align_in_terminal_cells_and_focus_does_not_reflow() {
    let temp = tempfile::tempdir().unwrap();
    for locale in ["en-US", "zh-CN"] {
        for theme in [Theme::dark(), Theme::light()] {
            for (width, height) in [
                (40, 12),
                (60, 20),
                (80, 24),
                (87, 24),
                (88, 24),
                (120, 32),
                (180, 48),
            ] {
                let mut app = fixture(temp.path().into());
                app.i18n = I18n::new(locale, BTreeMap::new(), builtin_language_packs());
                app.theme = theme;
                let position = app.list_state.selected();
                let before = buffer(&mut app, width, height);
                let checks = find_ascii(&before, "[")
                    .into_iter()
                    .filter(|(x, y)| *y >= 3 && *x < 8 && before[(*x + 2, *y)].symbol() == "]")
                    .collect::<Vec<_>>();
                assert!(checks.len() >= 2, "{width}x{height}: {before:?}");
                assert!(checks.iter().all(|(x, _)| *x == checks[0].0));
                let path_x = checks[0].0 + 6;
                assert!(
                    checks
                        .iter()
                        .all(|(_, y)| before[(path_x, *y)].symbol() == "中")
                );
                let list_end = if width >= 88 {
                    (0..width)
                        .rev()
                        .find(|x| before[(*x, checks[0].1)].symbol() == "│")
                        .expect("detail separator")
                } else {
                    width
                };
                let mut size_end_by_row = BTreeMap::new();
                for size in ["7 B", "1.00 KiB", "4.00 MiB", "12.00 GiB"] {
                    for (x, y) in find_ascii(&before, size).into_iter().filter(|(x, y)| {
                        checks.iter().any(|(_, row_y)| row_y == y) && *x < list_end
                    }) {
                        // The rightmost match is the size, even if a path ends in "7 B…".
                        let end = x + size.len() as u16;
                        let current = size_end_by_row.entry(y).or_insert(end);
                        *current = (*current).max(end);
                    }
                }
                let size_ends = size_end_by_row.into_values().collect::<Vec<_>>();
                assert!(
                    size_ends.len() == checks.len(),
                    "size cells must survive truncation: {before:?}"
                );
                assert!(
                    size_ends.iter().all(|x| *x == size_ends[0]),
                    "unaligned sizes: {size_ends:?}"
                );
                app.handle_key(key(KeyCode::Tab));
                let after = buffer(&mut app, width, height);
                if width >= 88 {
                    for y in 1..height - 1 {
                        for x in 0..width {
                            assert_eq!(
                                before[(x, y)].symbol(),
                                after[(x, y)].symbol(),
                                "focus reflow at {x},{y} ({width}x{height}, {locale})"
                            );
                        }
                    }
                } else {
                    assert_eq!(after[(0, 1)].symbol(), "╭");
                    assert_eq!(after[(width - 1, height - 2)].symbol(), "╯");
                    assert_eq!(
                        after[(width - 2, height - 2)].symbol(),
                        "─",
                        "IME guard erased the overlay"
                    );
                }
                app.handle_key(key(KeyCode::Esc));
                assert!(!app.details.focused);
                assert_eq!(app.list_state.selected(), position);
                app.handle_key(key(KeyCode::Char('G')));
                let last = buffer(&mut app, width, height);
                assert!(compact(&last).contains("-39"), "last row lost: {last:?}");
            }
        }
    }
}

#[test]
fn ui_header_body_and_footer_share_gutters_and_keep_errors_visible() {
    let temp = tempfile::tempdir().unwrap();
    for (width, margin) in [(40, 1), (80, 2), (120, 2), (240, 10)] {
        let mut app = app(temp.path().into());
        let screen = buffer(&mut app, width, 24);
        assert_eq!(find_ascii(&screen, "cleanr")[0].0, margin);
        assert_eq!(find_ascii(&screen, "Review disk cleanup")[0].0, margin);
        assert_eq!(find_ascii(&screen, "/ commands")[0].0, margin);
        assert!(find_ascii(&screen, env!("CARGO_PKG_VERSION")).is_empty());
        assert!(find_ascii(&screen, "Ready").is_empty());
        app.status = "Permission denied".into();
        assert!(!find_ascii(&buffer(&mut app, width, 24), "Permission denied").is_empty());
    }
}

#[test]
fn ui_narrow_paths_keep_distinguishing_parent_names() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path().into());
    app.entries = Arc::new(
        (0..4)
            .map(|i| ScanEntry {
                path: temp.path().join(format!("project-{i}/node_modules")),
                kind: EntryKind::Directory,
                size_bytes: 2 * 1024 * 1024,
                modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
                rule_hits: vec![test_rule_hit("generated")],
            })
            .collect(),
    );
    app.analysis = None;
    app.build_plan();
    let screen = buffer(&mut app, 40, 12);
    for i in 0..4 {
        assert!(
            !find_ascii(&screen, &format!("project-{i}")).is_empty(),
            "parent names disappeared: {screen:?}"
        );
    }
}

fn manifest() -> ExecutionManifest {
    ExecutionManifest {
        schema_version: EXECUTION_SCHEMA_VERSION.into(),
        run_id: "exact-reviewed-run-123".into(),
        created_at: chrono::DateTime::parse_from_rfc3339("2026-09-08T01:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        plan_schema_version: "plan".into(),
        authorization: None,
        summary: ExecutionSummary {
            attempted: 2,
            succeeded: 2,
            failed: 0,
        },
        items: Vec::new(),
    }
}

#[test]
fn ui_all_list_views_offer_modal_scrollable_details_without_actions_leaking() {
    let temp = tempfile::tempdir().unwrap();
    for view in [
        View::Scan,
        View::Usage,
        View::Restore,
        View::Rules,
        View::Plugins,
        View::Languages,
        View::Tasks,
    ] {
        for locale in ["en-US", "zh-CN"] {
            for theme in [Theme::dark(), Theme::light()] {
                for (width, height) in [
                    (40, 12),
                    (60, 20),
                    (80, 24),
                    (87, 24),
                    (88, 24),
                    (120, 32),
                    (180, 48),
                ] {
                    let mut app = fixture(temp.path().into());
                    app.i18n = I18n::new(locale, BTreeMap::new(), builtin_language_packs());
                    app.theme = theme;
                    app.rebuild_usage_order();
                    app.execution_manifests = vec![manifest()];
                    app.task_log = vec!["A long task message. 长消息。 ".repeat(60)];
                    app.switch_view(view);
                    let before = buffer(&mut app, width, height);
                    let selected = app.plan().unwrap().summary.selected_count;
                    let locale_before = app.i18n.locale().to_string();
                    let position = app.list_state.selected();
                    app.handle_key(key(KeyCode::Tab));
                    let after = buffer(&mut app, width, height);
                    if width >= 88 {
                        for y in 1..height - 1 {
                            for x in 0..width {
                                assert_eq!(
                                    before[(x, y)].symbol(),
                                    after[(x, y)].symbol(),
                                    "{view:?}: focus moved content at {x},{y}"
                                );
                            }
                        }
                    }
                    assert!(app.details.focused, "{view:?}");
                    assert!(!app.details.expanded);
                    assert!(app.handle_key_changed(key(KeyCode::Char('i'))));
                    app.handle_key(repeat(KeyCode::Char('i')));
                    buffer(&mut app, width, height);
                    assert!(app.details.expanded);
                    app.handle_key(key(KeyCode::End));
                    assert_eq!(app.details.scroll, app.details.max_scroll);
                    app.handle_key(key(KeyCode::Enter));
                    app.handle_key(key(KeyCode::Char(' ')));
                    for code in ['a', 'A', '%', 'c', 'f', 'o', 'v', 'p', 's', 'r', 'u'] {
                        app.handle_key(key(KeyCode::Char(code)));
                    }
                    assert_eq!(app.plan().unwrap().summary.selected_count, selected);
                    assert_eq!(app.i18n.locale(), locale_before);
                    assert!(!app.confirmation_pending());
                    assert!(!app.is_operation_running());
                    assert!(!app.is_scan_running());
                    assert_eq!(app.view, view);
                    assert!(
                        !app.scan_view.filter_open
                            && !app.scan_view.sort_open
                            && !app.scan_view.search_open
                    );
                    app.handle_key(key(KeyCode::Char('?')));
                    let scroll = app.details.scroll;
                    app.handle_key(key(KeyCode::Home));
                    assert_eq!(app.details.scroll, scroll, "help must consume navigation");
                    app.handle_key(key(KeyCode::Esc));
                    assert!(app.details.focused);
                    app.handle_key(key(KeyCode::Esc));
                    assert!(!app.details.focused);
                    assert_eq!(app.list_state.selected(), position);
                    app.handle_key(key(KeyCode::BackTab));
                    assert!(app.details.focused);
                    app.handle_key(key(KeyCode::BackTab));
                    assert!(!app.details.focused);
                }
            }
        }
    }
}

#[test]
fn ui_details_state_is_per_view_and_new_item_resets_scroll() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path().into());
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Char('i')));
    buffer(&mut app, 80, 12);
    app.handle_key(key(KeyCode::End));
    assert!(app.details.scroll > 0);
    let scroll = app.details.scroll;
    app.switch_view(View::Languages);
    assert!(!app.details.focused && !app.details.expanded);
    app.switch_view(View::Scan);
    assert!(!app.details.focused && app.details.expanded);
    assert_eq!(app.details.scroll, scroll);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.details.scroll, 0);
    let selected = app.plan().unwrap().summary.selected_count;
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(
        app.plan().unwrap().summary.selected_count,
        selected,
        "Enter and Space still toggle the same item"
    );
    app.show_rules();
    app.select_line(3);
    let rule_position = app.list_state.selected();
    app.show_languages();
    app.show_rules();
    assert_eq!(
        app.list_state.selected(),
        rule_position,
        "opening a page must preserve its list position"
    );
    app.go_home();
    app.start_scan(ScanRequest::default());
    assert!(
        app.details.expanded,
        "a new scan must retain the Scan disclosure choice for this session"
    );
    app.cancel_scan();
}

#[test]
fn ui_inline_search_keeps_cursor_at_the_list_and_escape_restores_query() {
    let temp = tempfile::tempdir().unwrap();
    for (width, height) in [(40, 12), (80, 24), (120, 32)] {
        let mut app = fixture(temp.path().into());
        app.scan_view.query = "cache".into();
        app.open_scan_search();
        app.handle_key(ctrl(KeyCode::Char('u')));
        app.handle_paste("中文📦-cache-01");
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        assert!(
            cursor.y <= 3 && cursor.x < width,
            "search cursor: {cursor:?}"
        );
        assert!(
            find_ascii(terminal.backend().buffer(), "find:")
                .iter()
                .any(|(_, y)| *y <= 3)
        );
        assert!(!app.palette_open);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.scan_view.query, "cache");
        assert!(!app.scan_view.search_open);
        assert!(matches!(app.mode, Mode::Normal));
    }
}

#[test]
fn ui_confirmation_has_readable_focus_and_still_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    for theme in [Theme::dark(), Theme::light()] {
        let mut app = fixture(temp.path().into());
        app.theme = theme;
        app.handle_key(key(KeyCode::Char('c')));
        let screen = buffer(&mut app, 120, 32);
        assert_eq!(app.confirm_choice, ConfirmChoice::No);
        let (x, y) = find_ascii(&screen, "Cancel")[0];
        let cell = &screen[(x, y)];
        assert_ne!(cell.fg, cell.bg);
        assert!(cell.modifier.contains(Modifier::BOLD));
        buffer(&mut app, 20, 8);
        assert!(!app.confirm_content_visible);
        app.handle_key(key(KeyCode::Char('y')));
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.is_operation_running());
        app.handle_key(key(KeyCode::Esc));
        assert!(!app.confirmation_pending());
    }
}
