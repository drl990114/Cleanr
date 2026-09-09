use super::*;

pub(crate) struct ContextContent {
    pub(crate) title: String,
    pub(crate) count: usize,
    pub(crate) columns: Vec<Constraint>,
    pub(crate) empty: String,
    pub(crate) details: Vec<Line<'static>>,
    pub(crate) more: Vec<Line<'static>>,
}

pub(crate) fn render_context_workspace<F>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut Workbench,
    content: ContextContent,
    rows: F,
) where
    F: FnOnce(&Workbench, Range<usize>, &[u16]) -> Vec<Row<'static>>,
{
    let columns = responsive_workspace(area, frame.area().width >= 88);
    render_table_list(
        frame,
        columns[0],
        app,
        content.title,
        content.count,
        &content.columns,
        content.empty,
        rows,
    );
    render_details(frame, columns[1], app, content.details, content.more);
}

pub(crate) fn render_languages(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let mut details = Vec::new();
    let mut more = vec![detail_line(
        &app.i18n.t("detail_language_dirs"),
        join_paths(&app.config.i18n.dirs),
        app.theme.fg_dim,
        app.theme,
    )];
    if let Some(pack) = app
        .list_state
        .selected()
        .and_then(|i| app.i18n.packs().get(i))
    {
        details.push(home_title(pack.name.clone(), app.theme));
        details.push(Line::from(pack.locale.clone()));
        details.push(Line::from(app.i18n.t(
            if pack.locale == app.i18n.locale() {
                "language_current"
            } else {
                "language_select"
            },
        )));
        more.extend([
            detail_line(
                &app.i18n.t("detail_id"),
                pack.id.clone(),
                app.theme.fg_dim,
                app.theme,
            ),
            detail_line(
                &app.i18n.t("detail_version"),
                pack.version.clone(),
                app.theme.fg_dim,
                app.theme,
            ),
            detail_line(
                &app.i18n.t("detail_source"),
                app.i18n
                    .t(&format!("source_{}", language_source_label(&pack.source))),
                app.theme.fg_dim,
                app.theme,
            ),
        ]);
        let path = match &pack.source {
            LanguagePackSource::Builtin => None,
            LanguagePackSource::UserFile(path) | LanguagePackSource::Plugin { path, .. } => {
                Some(path)
            }
        };
        if let Some(path) = path {
            more.push(Line::from(path.display().to_string()));
        }
    }
    let content = ContextContent {
        title: app.i18n.t("label_languages"),
        count: app.i18n.packs().len(),
        columns: vec![
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(9),
        ],
        empty: app.i18n.t("ui_empty"),
        details,
        more,
    };
    render_context_workspace(frame, area, app, content, |app, window, widths| {
        app.i18n.packs()[window]
            .iter()
            .map(|pack| {
                Row::new(vec![
                    text_cell(
                        if pack.locale == app.i18n.locale() {
                            "✓"
                        } else {
                            ""
                        },
                        widths[0],
                        app.theme.ok,
                    ),
                    text_cell(&pack.name, widths[1], app.theme.fg),
                    text_cell(&pack.locale, widths[2], app.theme.fg_dim),
                ])
            })
            .collect()
    });
}

pub(crate) fn render_rules(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let selected = app.list_state.selected();
    let mut details = Vec::new();
    let mut more = Vec::new();
    let mut offset = 0;
    for pack in app.registry.packs() {
        if selected == Some(offset) {
            details.push(home_title(pack.definition.name.clone(), app.theme));
            details.push(Line::from(pack.definition.description.clone()));
            more.push(detail_line(
                &app.i18n.t("detail_id"),
                pack.definition.id.clone(),
                app.theme.fg_dim,
                app.theme,
            ));
            more.push(detail_line(
                &app.i18n.t("detail_version"),
                pack.definition.version.clone(),
                app.theme.fg_dim,
                app.theme,
            ));
            break;
        }
        offset += 1;
        if let Some(rule) = selected
            .and_then(|i| i.checked_sub(offset))
            .and_then(|i| pack.definition.rules.get(i))
        {
            details.push(home_title(rule.label.clone(), app.theme));
            details.push(Line::from(
                CategoryKey::Named(rule.category.clone()).label(&app.i18n, false),
            ));
            detail_section(
                &mut details,
                app.i18n.t("detail_risk"),
                rule.risk_note.clone(),
                app.theme.warn,
                app.theme,
            );
            detail_section(
                &mut details,
                app.i18n.t("detail_reason"),
                rule.reason.clone(),
                app.theme.fg,
                app.theme,
            );
            more.push(detail_line(
                &app.i18n.t("detail_id"),
                rule.id.clone(),
                app.theme.fg_dim,
                app.theme,
            ));
            more.push(detail_line(
                &app.i18n.t("detail_category"),
                rule.category.clone(),
                app.theme.fg_dim,
                app.theme,
            ));
            more.push(detail_line(
                &app.i18n.t("detail_source"),
                pack.definition.id.clone(),
                app.theme.fg_dim,
                app.theme,
            ));
            break;
        }
        offset += pack.definition.rules.len();
    }
    let content = ContextContent {
        title: app.i18n.t("label_rules"),
        count: app.list_len(),
        columns: vec![Constraint::Fill(1), Constraint::Length(12)],
        empty: app.i18n.t("ui_empty"),
        details,
        more,
    };
    render_context_workspace(frame, area, app, content, |app, window, widths| {
        app.registry
            .packs()
            .iter()
            .flat_map(|pack| {
                std::iter::once((pack, None)).chain(
                    pack.definition
                        .rules
                        .iter()
                        .map(move |rule| (pack, Some(rule))),
                )
            })
            .skip(window.start)
            .take(window.len())
            .map(|(pack, rule)| {
                let (name, category) = rule.map_or_else(
                    || (pack.definition.name.clone(), String::new()),
                    |rule| {
                        (
                            format!("  {}", rule.label),
                            CategoryKey::Named(rule.category.clone()).label(&app.i18n, true),
                        )
                    },
                );
                Row::new(vec![
                    text_cell(name, widths[0], app.theme.fg),
                    text_cell(category, widths[1], app.theme.fg_dim),
                ])
            })
            .collect()
    });
}

fn trust_label(trust: cleanr_plugin_api::TrustLevel, app: &Workbench) -> String {
    app.i18n.t(match trust {
        cleanr_plugin_api::TrustLevel::Builtin => "trust_builtin",
        cleanr_plugin_api::TrustLevel::Trusted => "trust_trusted",
        cleanr_plugin_api::TrustLevel::Untrusted => "trust_untrusted",
    })
}

fn diagnostic_color(diagnostic: &cleanr_plugin_api::PluginDiagnostic, theme: Theme) -> Color {
    match diagnostic.severity {
        cleanr_plugin_api::DiagnosticSeverity::Warning => theme.warn,
        cleanr_plugin_api::DiagnosticSeverity::Error => theme.danger,
    }
}

pub(crate) fn render_plugins(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let diagnostics = app.plugin_diagnostics();
    let diagnostic_count = diagnostics.len();
    let pack_count = app.registry.packs().len();
    let mut details = Vec::new();
    let mut more = vec![detail_line(
        &app.i18n.t("detail_plugin_dirs"),
        join_paths(&app.config.plugins.dirs),
        app.theme.fg_dim,
        app.theme,
    )];
    if let Some(index) = app.list_state.selected() {
        if let Some(pack) = app.registry.packs().get(index) {
            details.push(home_title(pack.definition.name.clone(), app.theme));
            details.push(Line::from(trust_label(pack.trust, app)));
            details.push(Line::from(pack.definition.description.clone()));
            more.extend([
                detail_line(
                    &app.i18n.t("detail_id"),
                    pack.definition.id.clone(),
                    app.theme.fg_dim,
                    app.theme,
                ),
                detail_line(
                    &app.i18n.t("detail_version"),
                    pack.definition.version.clone(),
                    app.theme.fg_dim,
                    app.theme,
                ),
                detail_line(
                    &app.i18n.t("detail_source"),
                    app.i18n.t(&format!("source_{}", pack.source.label())),
                    app.theme.fg_dim,
                    app.theme,
                ),
            ]);
            if let Some(path) = pack.source.path() {
                more.push(Line::from(path.display().to_string()));
            }
        } else if let Some(diagnostic) = index
            .checked_sub(pack_count)
            .and_then(|i| diagnostics.get(i))
        {
            details.push(home_title(app.i18n.t("detail_diagnostics"), app.theme));
            details.push(Line::from(Span::styled(
                diagnostic.message.clone(),
                Style::default().fg(diagnostic_color(diagnostic, app.theme)),
            )));
            if let Some(path) = &diagnostic.path {
                detail_section(
                    &mut details,
                    app.i18n.t("detail_path"),
                    path.display().to_string(),
                    app.theme.fg,
                    app.theme,
                );
            }
            more.push(detail_line(
                &app.i18n.t("detail_id"),
                diagnostic.code.to_string(),
                app.theme.fg_dim,
                app.theme,
            ));
        }
    }
    drop(diagnostics);
    let title = if diagnostic_count > 0 {
        format!(
            "{} · {}",
            app.i18n.t("label_plugins"),
            app.i18n
                .format("plugin_issues", &[("count", diagnostic_count.to_string())])
        )
    } else {
        app.i18n.t("label_plugins")
    };
    let content = ContextContent {
        title,
        count: pack_count + diagnostic_count,
        columns: vec![Constraint::Fill(1), Constraint::Length(11)],
        empty: app.i18n.t("ui_empty"),
        details,
        more,
    };
    render_context_workspace(frame, area, app, content, |app, window, widths| {
        let diagnostics = app.plugin_diagnostics();
        window
            .map(|index| {
                if let Some(pack) = app.registry.packs().get(index) {
                    Row::new(vec![
                        text_cell(&pack.definition.name, widths[0], app.theme.fg),
                        text_cell(
                            trust_label(pack.trust, app),
                            widths[1],
                            if pack.trust == cleanr_plugin_api::TrustLevel::Untrusted {
                                app.theme.warn
                            } else {
                                app.theme.fg_dim
                            },
                        ),
                    ])
                } else {
                    let diagnostic = diagnostics[index - app.registry.packs().len()];
                    Row::new(vec![
                        text_cell(
                            format!("! {}", diagnostic.message),
                            widths[0],
                            diagnostic_color(diagnostic, app.theme),
                        ),
                        Cell::default(),
                    ])
                }
            })
            .collect()
    });
}

pub(crate) fn render_tasks(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let details = app
        .list_state
        .selected()
        .and_then(|i| app.task_log.iter().rev().nth(i))
        .map_or_else(
            || vec![Line::from(app.i18n.t("status_no_tasks"))],
            |task| vec![Line::from(task.clone())],
        );
    let mut more = vec![detail_line(
        &app.i18n.t("detail_task_count"),
        app.task_log.len().to_string(),
        app.theme.fg_dim,
        app.theme,
    )];
    if app.details.expanded {
        more.push(detail_line(
            &app.i18n.t("detail_status"),
            app.status().to_string(),
            app.theme.fg_dim,
            app.theme,
        ));
        for (label, recorder) in [
            ("metrics_handler", &app.input_durations),
            ("metrics_frame", &app.frame_durations),
            ("metrics_input_frame", &app.input_to_frame_durations),
            ("metrics_commit", &app.task_commit_durations),
        ] {
            let summary = recorder.summary();
            more.push(detail_line(
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
    }
    let content = ContextContent {
        title: app.i18n.t("label_tasks"),
        count: app.task_log.len(),
        columns: vec![Constraint::Fill(1)],
        empty: app.i18n.t("status_no_tasks"),
        details,
        more,
    };
    render_context_workspace(frame, area, app, content, |app, window, widths| {
        app.task_log
            .iter()
            .rev()
            .skip(window.start)
            .take(window.len())
            .map(|task| Row::new(vec![text_cell(task, widths[0], app.theme.fg)]))
            .collect()
    });
}
