use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_context_workspace(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut ListState,
    theme: Theme,
    title: String,
    items: Vec<ListItem<'static>>,
    detail_title: String,
    details: Vec<Line<'static>>,
) -> u16 {
    let item_count = items.len();
    render_context_workspace_virtualized(
        frame,
        area,
        state,
        theme,
        title,
        item_count,
        move |window| {
            items
                .into_iter()
                .skip(window.start)
                .take(window.len())
                .collect()
        },
        detail_title,
        details,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_context_workspace_virtualized<F>(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut ListState,
    theme: Theme,
    title: String,
    item_count: usize,
    items_for_window: F,
    detail_title: String,
    details: Vec<Line<'static>>,
) -> u16
where
    F: FnOnce(Range<usize>) -> Vec<ListItem<'static>>,
{
    let wide = area.width >= 88;
    let workspace = fluid_content_rect(area, 220, area.height);
    let columns = responsive_workspace(workspace, 56);
    let viewport_height = columns[0].height.saturating_sub(1).max(1) as usize;
    let window = visible_list_window(state, item_count, viewport_height);
    let mut local_state = local_list_state(state, &window);
    let has_scrollbar = item_count > viewport_height;
    let list_borders = if wide {
        Borders::TOP | Borders::RIGHT
    } else {
        Borders::TOP
    };

    let mut list_block = Block::default()
        .borders(list_borders)
        .border_style(Style::default().fg(theme.border))
        .title(format!(" {title} "))
        .title_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    if has_scrollbar && !wide {
        list_block = list_block.padding(Padding::new(0, 1, 0, 0));
    }

    let list = List::new(items_for_window(window.clone()))
        .block(list_block)
        .highlight_style(
            Style::default()
                .fg(theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, columns[0], &mut local_state);
    render_list_scrollbar(
        frame,
        columns[0],
        item_count,
        viewport_height,
        state.selected().unwrap_or(window.start),
        theme,
    );

    let details = Paragraph::new(details).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme.border))
            .padding(Padding::horizontal(1))
            .title(format!(" {detail_title} "))
            .title_style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    frame.render_widget(details, columns[1]);
    u16::try_from(viewport_height).unwrap_or(u16::MAX)
}

pub(crate) fn render_languages(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let items = app
        .i18n
        .packs()
        .iter()
        .map(|pack| {
            let marker = if pack.locale == app.i18n.locale() {
                Span::styled(
                    ">",
                    Style::default()
                        .fg(app.theme.ok)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(" ", Style::default().fg(app.theme.fg_dim))
            };
            ListItem::new(Line::from(vec![
                marker,
                Span::raw(" "),
                Span::styled(pack.locale.clone(), Style::default().fg(app.theme.cyan)),
                Span::raw("  "),
                Span::raw(pack.name.clone()),
                Span::styled(
                    format!("  {}@{}", language_source_label(&pack.source), pack.version),
                    Style::default().fg(app.theme.fg_dim),
                ),
            ]))
        })
        .collect::<Vec<_>>();

    let dirs = join_paths(&app.config.i18n.dirs);
    let details = vec![
        detail_line(
            "Active locale",
            app.i18n.locale().to_string(),
            app.theme.ok,
            app.theme,
        ),
        detail_line(
            "Loaded packs",
            app.i18n.packs().len().to_string(),
            app.theme.cyan,
            app.theme,
        ),
        detail_line("Language dirs", dirs, app.theme.fg_dim, app.theme),
        Line::from(""),
        Line::from(vec![Span::styled(
            app.i18n.t("language_home_hint"),
            Style::default().fg(app.theme.fg_dim),
        )]),
    ];
    let title = app.i18n.t("label_languages");
    let detail_title = app.i18n.t("label_details");

    app.viewport_height = render_context_workspace(
        frame,
        area,
        &mut app.list_state,
        app.theme,
        title,
        items,
        detail_title,
        details,
    );
}

pub(crate) fn render_rules(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let mut items = Vec::new();
    let mut categories = BTreeSet::new();
    let mut rule_count = 0usize;
    for pack in app.registry.packs() {
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                pack.definition.id.clone(),
                Style::default().fg(app.theme.cyan),
            ),
            Span::styled(
                format!(" @{}", pack.definition.version),
                Style::default().fg(app.theme.fg_dim),
            ),
        ])));
        for rule in &pack.definition.rules {
            rule_count += 1;
            categories.insert(rule.category.clone());
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  - ", Style::default().fg(app.theme.fg_dim)),
                Span::styled(rule.id.clone(), Style::default().fg(app.theme.ok)),
                Span::raw("  "),
                Span::raw(rule.label.clone()),
                Span::styled(
                    format!("  {}", rule.category),
                    Style::default().fg(app.theme.fg_dim),
                ),
            ])));
        }
    }

    let details = vec![
        detail_line(
            "Rule packs",
            app.registry.packs().len().to_string(),
            app.theme.cyan,
            app.theme,
        ),
        detail_line("Rules", rule_count.to_string(), app.theme.ok, app.theme),
        detail_line(
            "Categories",
            categories.into_iter().collect::<Vec<_>>().join(", "),
            app.theme.fg_dim,
            app.theme,
        ),
    ];
    let title = app.i18n.t("label_rules");
    let detail_title = app.i18n.t("label_details");

    app.viewport_height = render_context_workspace(
        frame,
        area,
        &mut app.list_state,
        app.theme,
        title,
        items,
        detail_title,
        details,
    );
}

pub(crate) fn render_plugins(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let diagnostics = app.plugin_diagnostics();
    let diagnostic_count = diagnostics.len();
    let mut items = app
        .registry
        .packs()
        .iter()
        .map(|pack| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    pack.definition.id.clone(),
                    Style::default().fg(app.theme.cyan),
                ),
                Span::styled(
                    format!(" @{}", pack.definition.version),
                    Style::default().fg(app.theme.fg_dim),
                ),
                Span::raw("  "),
                Span::raw(pack.definition.name.clone()),
                Span::styled(
                    format!("  [{} / {:?}]", pack.source.label(), pack.trust),
                    Style::default().fg(app.theme.fg_dim),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    items.extend(diagnostics.iter().map(|diagnostic| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("! {}", diagnostic.code),
                Style::default().fg(app.theme.warn),
            ),
            Span::raw("  "),
            Span::raw(diagnostic.message.clone()),
        ]))
    }));
    drop(diagnostics);

    let details = vec![
        detail_line(
            "Plugin dirs",
            join_paths(&app.config.plugins.dirs),
            app.theme.fg_dim,
            app.theme,
        ),
        detail_line(
            "Loaded packs",
            app.registry.packs().len().to_string(),
            app.theme.cyan,
            app.theme,
        ),
        detail_line(
            "Diagnostics",
            diagnostic_count.to_string(),
            app.theme.warn,
            app.theme,
        ),
        Line::from(""),
        Line::from(vec![Span::styled(
            app.i18n.t("plugins_context_hint"),
            Style::default().fg(app.theme.fg_dim),
        )]),
    ];
    let title = app.i18n.t("label_plugins");
    let detail_title = app.i18n.t("label_details");

    app.viewport_height = render_context_workspace(
        frame,
        area,
        &mut app.list_state,
        app.theme,
        title,
        items,
        detail_title,
        details,
    );
}

pub(crate) fn render_tasks(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let items = if app.task_log.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            app.i18n.t("status_no_tasks"),
            Style::default().fg(app.theme.fg_dim),
        )]))]
    } else {
        app.task_log
            .iter()
            .rev()
            .map(|task| ListItem::new(Line::from(task.clone())))
            .collect::<Vec<_>>()
    };

    let mut details = vec![
        detail_line(
            "Task count",
            app.task_log.len().to_string(),
            app.theme.cyan,
            app.theme,
        ),
        detail_line(
            "Scan running",
            if app.is_scan_running() { "yes" } else { "no" }.to_string(),
            if app.is_scan_running() {
                app.theme.warn
            } else {
                app.theme.fg_dim
            },
            app.theme,
        ),
        detail_line(
            "Status",
            app.status().to_string(),
            app.theme.fg_dim,
            app.theme,
        ),
    ];
    for (label, recorder) in [
        ("metrics_handler", &app.input_durations),
        ("metrics_frame", &app.frame_durations),
        ("metrics_input_frame", &app.input_to_frame_durations),
        ("metrics_commit", &app.task_commit_durations),
    ] {
        let summary = recorder.summary();
        details.push(detail_line(
            &app.i18n.t(label),
            format!(
                "P95 {:.2} ms · max {:.2} ms",
                summary.p95.as_secs_f64() * 1000.0,
                summary.max.as_secs_f64() * 1000.0
            ),
            app.theme.fg_dim,
            app.theme,
        ));
    }
    let title = app.i18n.t("label_tasks");
    let detail_title = app.i18n.t("label_details");

    app.viewport_height = render_context_workspace(
        frame,
        area,
        &mut app.list_state,
        app.theme,
        title,
        items,
        detail_title,
        details,
    );
}
