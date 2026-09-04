use super::*;
use crate::app::CategoryKey;

fn category_app(root: PathBuf, categories: &[&str]) -> Workbench {
    let mut app = app(root.clone());
    app.config.cleanup.require_confirm = true;
    app.entries = categories
        .iter()
        .enumerate()
        .map(|(index, category)| {
            let mut hit = test_rule_hit(&format!("category-{index}"));
            hit.label = format!("Cache rule {index}");
            hit.category = (*category).into();
            hit.default_selected = false;
            ScanEntry {
                path: root.join(format!("cache-{index:05}")),
                kind: EntryKind::File,
                size_bytes: (index as u64 + 1) * 1024,
                modified_at: Some(app.scan_as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit],
            }
        })
        .collect();
    app.build_plan();
    app.view = View::Scan;
    app.ensure_scan_view_projection();
    assert_eq!(app.plan().expect("plan").items.len(), categories.len());
    assert_eq!(app.plan().expect("plan").summary.selected_count, 0);
    app
}

/// Exercise the same key path as a user without assuming localized category ordering.
fn filter_category(app: &mut Workbench, category: Option<&str>) {
    app.handle_key(key(KeyCode::Char('f')));
    assert!(app.scan_view.filter_open);
    let target = category.map_or(0, |category| {
        app.scan_view
            .groups
            .iter()
            .position(|group| group.key == CategoryKey::Named(category.into()))
            .expect("category is offered in the filter")
            + 1
    });
    let selected = app.scan_view.filter_state.selected().expect("filter focus");
    let code = if selected > target {
        KeyCode::Up
    } else {
        KeyCode::Down
    };
    for _ in 0..selected.abs_diff(target) {
        app.handle_key(key(code));
    }
    app.handle_key(key(KeyCode::Enter));
    assert!(!app.scan_view.filter_open);
    assert_eq!(
        app.scan_view.filter,
        category.map(|category| CategoryKey::Named(category.into()))
    );
}

fn selected_categories(app: &Workbench) -> Vec<&str> {
    app.plan()
        .expect("plan")
        .items
        .iter()
        .filter(|item| item.selected)
        .map(|item| item.category.as_str())
        .collect()
}

fn assert_selection_summary(app: &Workbench, count: usize) {
    let plan = app.plan().expect("plan");
    assert_eq!(
        plan.items.iter().filter(|item| item.selected).count(),
        count
    );
    assert_eq!(plan.summary.selected_count, count);
    assert_eq!(app.selection.candidate_ids.len(), count);
    assert_eq!(
        plan.summary.selected_size_bytes,
        plan.items
            .iter()
            .filter(|item| item.selected)
            .map(|item| item.size_bytes)
            .sum::<u64>()
    );
}

#[test]
fn scan_category_filter_maps_focus_and_accumulates_selection_across_categories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = category_app(
        temp.path().to_path_buf(),
        &[
            "build-cache",
            "logs",
            "browser-cache",
            "build-cache",
            "logs",
            "browser-cache",
        ],
    );
    let focused = app
        .plan()
        .expect("plan")
        .items
        .iter()
        .position(|item| item.path.ends_with("cache-00002"))
        .expect("browser candidate");
    app.list_state.select(Some(focused));
    filter_category(&mut app, Some("browser-cache"));
    assert_eq!(app.list_len(), 2);
    app.handle_key(key(KeyCode::Char(' ')));
    assert_selection_summary(&app, 1);
    let selected = app
        .plan()
        .expect("plan")
        .items
        .iter()
        .find(|item| item.selected)
        .unwrap();
    assert!(
        selected.path.ends_with("cache-00002"),
        "filter must retain the focused candidate"
    );

    app.handle_key(key(KeyCode::Char('a')));
    assert_selection_summary(&app, 2);
    assert!(
        selected_categories(&app)
            .iter()
            .all(|category| *category == "browser-cache")
    );
    let browser_size = app.plan().expect("plan").summary.selected_size_bytes;
    filter_category(&mut app, Some("logs"));
    assert_eq!(app.scan_view.hidden_selected_count, 2);
    assert_eq!(app.scan_view.hidden_selected_bytes, browser_size);
    assert_eq!(app.list_state.selected(), Some(0));
    app.handle_key(key(KeyCode::Char('a')));
    assert_selection_summary(&app, 4);
    app.handle_key(key(KeyCode::Char('%')));
    assert_selection_summary(&app, 2);
    assert!(
        selected_categories(&app)
            .iter()
            .all(|category| *category == "browser-cache")
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert_selection_summary(&app, 6);
    assert_eq!(app.scan_view.hidden_selected_count, 4);
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert_selection_summary(&app, 0);
    assert_eq!(app.scan_view.hidden_selected_count, 0);
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.view, View::Home);
    app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(app.view, View::Scan);
    assert_eq!(
        app.scan_view.filter,
        Some(CategoryKey::Named("logs".into()))
    );
    assert_eq!(app.list_len(), 2);
}

#[test]
fn scan_category_popup_is_modal_and_control_f_keeps_page_navigation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = category_app(temp.path().to_path_buf(), &["build-cache"; 40]);
    app.viewport_height = 8;
    app.list_state.select(Some(0));
    app.handle_key(ctrl(KeyCode::Char('f')));
    assert!(!app.scan_view.filter_open);
    assert_eq!(app.list_state.selected(), Some(8));
    app.handle_key(key(KeyCode::Char('f')));
    let previous_focus = app.list_state.selected();
    for code in ['a', 'A', '%', 'c', 's', 'q', '?', '/', ' '] {
        app.handle_key(key(KeyCode::Char(code)));
    }
    assert!(app.scan_view.filter_open);
    assert_selection_summary(&app, 0);
    assert!(!app.clean_waiting_for_confirmation);
    assert!(!app.should_quit);
    assert!(!app.is_scan_running());
    assert!(!app.help_open);
    assert!(!app.palette_open());
    assert_eq!(app.list_state.selected(), previous_focus);
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.scan_view.filter_state.selected(), Some(1));
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.scan_view.filter_state.selected(), Some(0));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.view, View::Scan);
    assert!(!app.scan_view.filter_open);
    assert_eq!(app.scan_view.filter, None);
    assert_eq!(app.list_state.selected(), previous_focus);
}

#[test]
fn scan_category_bulk_selection_covers_all_filtered_pages_in_ten_thousand_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    let categories = (0..10_000)
        .map(|index| {
            if index % 2 == 0 {
                "build-cache"
            } else {
                "logs"
            }
        })
        .collect::<Vec<_>>();
    let mut app = category_app(temp.path().to_path_buf(), &categories);
    filter_category(&mut app, Some("logs"));
    assert_eq!(app.list_len(), 5_000);
    app.handle_key(key(KeyCode::Char('G')));
    assert_eq!(app.list_state.selected(), Some(4_999));
    let selected_row = app.scan_view.visible[4_999];
    let expected_name = app.plan().expect("plan").items[selected_row]
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let screen = render_text(&mut app, 120, 24);
    assert!(screen.contains(&expected_name), "{screen}");
    assert!(screen.contains("[Logs]"), "{screen}");
    assert!(app.list_state.offset() > 0);
    app.handle_key(key(KeyCode::Char('a')));
    assert_selection_summary(&app, 5_000);
    assert!(
        selected_categories(&app)
            .iter()
            .all(|category| *category == "logs")
    );
    app.handle_key(key(KeyCode::Char('%')));
    assert_selection_summary(&app, 0);
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert_selection_summary(&app, 10_000);
    assert_eq!(app.scan_view.hidden_selected_count, 5_000);
}

#[test]
fn scan_category_read_only_results_show_categories_without_enabling_cleanup() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = category_app(temp.path().to_path_buf(), &["build-cache", "logs"]);
    app.plan = None;
    app.analysis = None;
    app.selection = Default::default();
    app.ensure_scan_view_projection();
    filter_category(&mut app, Some("logs"));
    let screen = render_text(&mut app, 100, 28);
    assert!(screen.contains("[Logs]"), "{screen}");
    for code in [
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Char('a'),
        KeyCode::Char('%'),
        KeyCode::Char('A'),
        KeyCode::Char('c'),
    ] {
        app.handle_key(key(code));
        assert!(
            app.plan().is_none(),
            "read-only key unexpectedly created a plan: {code:?}"
        );
        assert!(!app.clean_waiting_for_confirmation);
        assert!(!app.is_operation_running());
    }
}

#[test]
fn scan_category_nine_labels_remain_visible_with_size_in_both_languages_and_themes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let labels = [
        ("developer-cache", "Dev", "开发"),
        ("build-cache", "Build", "构建"),
        ("package-cache", "Packages", "包缓存"),
        ("browser-cache", "Browser", "浏览器"),
        ("application-cache", "App", "应用"),
        ("temporary-files", "Temp", "临时"),
        ("logs", "Logs", "日志"),
        ("diagnostics", "Diagnostics", "诊断"),
        ("downloads", "Downloads", "下载"),
    ];
    for locale in ["en-US", "zh-CN"] {
        for theme in [Theme::dark(), Theme::light()] {
            for (category, english, chinese) in labels {
                let mut app = category_app(temp.path().to_path_buf(), &[category]);
                app.i18n = I18n::new(locale, BTreeMap::new(), builtin_language_packs());
                app.theme = theme;
                let screen = render_text(&mut app, 72, 26);
                let compact = screen
                    .chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect::<String>();
                let short = if locale == "en-US" { english } else { chinese };
                assert!(
                    compact.contains(&format!("[{short}]")),
                    "missing {category} / {locale}:\n{screen}"
                );
                assert!(screen.contains("1.00 KiB"), "{screen}");
                assert!(screen.contains("[ ]"), "{screen}");
                assert!(
                    compact.contains(category),
                    "raw category is absent from details:\n{screen}"
                );
                assert!(
                    screen.contains("Cache rule 0"),
                    "matched rule label is absent from details:\n{screen}"
                );
            }
        }
    }
}

#[test]
fn scan_category_filter_displays_counts_bytes_and_only_existing_groups() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = category_app(
        temp.path().to_path_buf(),
        &["build-cache", "logs", "build-cache"],
    );
    app.handle_key(key(KeyCode::Char('f')));
    let screen = render_text(&mut app, 120, 30);
    let build = screen
        .lines()
        .find(|line| line.contains("Build caches") && line.contains("4.00 KiB"))
        .expect("build filter option");
    let logs = screen
        .lines()
        .find(|line| line.contains("Logs") && !line.contains("[Logs]"))
        .expect("logs filter option");
    assert!(
        build.contains('2') && build.contains("4.00 KiB"),
        "{screen}"
    );
    assert!(logs.contains('1') && logs.contains("2.00 KiB"), "{screen}");
    assert!(
        screen
            .lines()
            .any(|line| line.contains("All") && line.contains('3') && line.contains("6.00 KiB")),
        "{screen}"
    );
    assert!(
        !screen.contains("Browser caches"),
        "absent category was offered:\n{screen}"
    );
    assert!(
        !screen.contains("Package caches"),
        "absent category was offered:\n{screen}"
    );
    app.handle_key(key(KeyCode::Esc));
    filter_category(&mut app, Some("build-cache"));
    let screen = render_text(&mut app, 120, 30);
    let compact = screen
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    assert!(
        compact.contains("2/3"),
        "filtered count should show visible / total:\n{screen}"
    );
}

#[test]
fn scan_category_hidden_selection_remains_visible_and_is_included_in_confirmation() {
    let temp = tempfile::tempdir().expect("tempdir");
    for locale in ["en-US", "zh-CN"] {
        let mut app = category_app(temp.path().to_path_buf(), &["build-cache", "logs", "logs"]);
        app.i18n = I18n::new(locale, BTreeMap::new(), builtin_language_packs());
        filter_category(&mut app, Some("logs"));
        app.handle_key(key(KeyCode::Char('a')));
        filter_category(&mut app, Some("build-cache"));
        let expected_hidden = app.i18n.format(
            "scan_selection_hidden",
            &[("count", "2".into()), ("size", "5.00 KiB".into())],
        );
        let expected_confirm = app.i18n.format(
            "confirm_hidden_selection",
            &[("count", "2".into()), ("size", "5.00 KiB".into())],
        );
        let compact = |text: &str| {
            text.chars()
                .filter(|ch| !ch.is_whitespace() && !('\u{2500}'..='\u{257f}').contains(ch))
                .collect::<String>()
        };
        let screen = render_text(&mut app, 120, 28);
        assert!(
            compact(&screen).contains(&compact(&expected_hidden)),
            "{screen}"
        );
        app.handle_key(key(KeyCode::Char('c')));
        assert!(app.clean_waiting_for_confirmation);
        assert_eq!(app.confirm_choice, ConfirmChoice::No);
        for (width, height) in [(40, 10), (60, 24), (120, 24)] {
            let screen = render_text(&mut app, width, height);
            assert!(
                compact(&screen).contains(&compact(&expected_confirm)),
                "{screen}"
            );
            assert!(
                compact(&screen).contains(&compact(&app.i18n.t("confirm_yes"))),
                "{screen}"
            );
            assert!(
                compact(&screen).contains(&compact(&app.i18n.t("confirm_no"))),
                "{screen}"
            );
        }
        app.handle_key(key(KeyCode::Esc));
        assert!(!app.clean_waiting_for_confirmation);
        assert!(!app.is_operation_running());
        assert_selection_summary(&app, 2);
    }
}

#[test]
fn scan_category_plugin_names_and_conflicting_categories_are_explained_in_details() {
    let temp = tempfile::tempdir().expect("tempdir");
    let custom_category = "vendor-custom-rendering-cache";
    let mut custom = category_app(temp.path().to_path_buf(), &[custom_category]);
    let screen = render_text(&mut custom, 120, 30);
    assert!(
        screen.contains(custom_category),
        "custom category should be preserved in details:\n{screen}"
    );
    custom.handle_key(key(KeyCode::Char('f')));
    let screen = render_text(&mut custom, 120, 30);
    assert!(
        screen.contains(custom_category),
        "custom category should be offered in filter:\n{screen}"
    );

    let mut app = category_app(temp.path().to_path_buf(), &["build-cache"]);
    let mut conflict = app.entries[0].rule_hits[0].clone();
    conflict.rule_id = "conflicting-cache".into();
    conflict.label = "Conflicting log rule".into();
    conflict.category = "logs".into();
    app.entries[0].rule_hits.push(conflict);
    app.analysis = None;
    app.plan = None;
    app.build_plan();
    let screen = render_text(&mut app, 120, 36);
    assert!(screen.contains("[Multiple]"), "{screen}");
    assert!(screen.contains("Conflicting rules"), "{screen}");
    assert!(screen.contains("build-cache"), "{screen}");
    assert!(screen.contains("logs"), "{screen}");
    assert!(screen.contains("Cache rule 0"), "{screen}");
    assert!(screen.contains("Conflicting log rule"), "{screen}");
    assert_eq!(app.scan_view.groups.len(), 1);
    assert_eq!(app.scan_view.groups[0].key, CategoryKey::Multiple);
    assert_eq!(app.scan_view.groups[0].count, 1);
    assert_eq!(app.scan_view.groups[0].size_bytes, 1024);
}
