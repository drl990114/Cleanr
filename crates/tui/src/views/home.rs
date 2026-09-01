use super::*;

pub(crate) fn render_home(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let content = fluid_content_rect(area, 160, 14);

    let candidate_count = app.plan.as_ref().map_or_else(
        || app.candidate_count_cached(),
        |plan| plan.summary.candidate_count,
    );
    let (selected_count, selected_size) = app.plan.as_ref().map_or((0, 0), |plan| {
        (
            plan.summary.selected_count,
            plan.summary.selected_size_bytes,
        )
    });

    let (title, subtitle, subtitle_color, actions, details) = if app.scan_summary.entries_seen == 0
    {
        (
            app.i18n.t("home_welcome"),
            app.i18n.t("home_subtitle"),
            app.theme.fg_dim,
            vec![
                home_action_line(app.theme, "s", app.i18n.t("home_action_scan"), true),
                home_action_line(app.theme, "u", app.i18n.t("home_action_usage"), false),
                home_action_line(app.theme, "/", app.i18n.t("home_action_more"), false),
            ],
            vec![
                home_detail_line(
                    app.i18n.t("home_detail_scope"),
                    truncate_text(&join_paths(&app.roots), 34),
                    app.theme.fg,
                    app.theme,
                ),
                home_detail_line(
                    app.i18n.t("home_detail_state"),
                    app.i18n.t("home_state_ready"),
                    app.theme.ok,
                    app.theme,
                ),
                home_detail_line(
                    app.i18n.t("home_detail_policy"),
                    app.i18n.t("home_policy_review"),
                    app.theme.fg_dim,
                    app.theme,
                ),
            ],
        )
    } else if candidate_count == 0 {
        (
            app.i18n.t("home_result_title"),
            app.i18n.t("home_result_empty"),
            app.theme.ok,
            vec![
                home_action_line(app.theme, "s", app.i18n.t("home_action_rescan"), true),
                home_action_line(app.theme, "u", app.i18n.t("home_action_usage"), false),
                home_action_line(app.theme, "/", app.i18n.t("home_action_more"), false),
            ],
            vec![
                home_detail_line(
                    app.i18n.t("home_detail_scanned"),
                    app.i18n.format(
                        "home_result_scanned",
                        &[("size", format_bytes(app.scan_summary.total_size_bytes))],
                    ),
                    app.theme.cyan,
                    app.theme,
                ),
                home_detail_line(
                    app.i18n.t("home_detail_state"),
                    app.i18n.t("home_result_empty"),
                    app.theme.ok,
                    app.theme,
                ),
                home_detail_line(
                    app.i18n.t("home_detail_policy"),
                    app.i18n.t("home_policy_review"),
                    app.theme.fg_dim,
                    app.theme,
                ),
            ],
        )
    } else {
        let reclaimable = format_bytes(
            app.plan
                .as_ref()
                .map_or(0, |plan| plan.summary.total_candidate_size_bytes),
        );
        (
            app.i18n.t("home_result_title"),
            format!(
                "{}{}{}",
                app.i18n.t("home_result_reclaimable"),
                reclaimable,
                app.i18n.format(
                    "home_result_candidates",
                    &[("count", candidate_count.to_string())],
                )
            ),
            app.theme.fg_dim,
            vec![
                home_action_line(app.theme, "r", app.i18n.t("home_action_review"), true),
                home_action_line(app.theme, "u", app.i18n.t("home_action_usage"), false),
                home_action_line(app.theme, "s", app.i18n.t("home_action_rescan"), false),
            ],
            vec![
                home_detail_line(
                    app.i18n.t("home_detail_reclaimable"),
                    reclaimable,
                    app.theme.cyan,
                    app.theme,
                ),
                home_detail_line(
                    app.i18n.t("home_detail_selected"),
                    app.i18n.format(
                        "home_result_selected",
                        &[
                            ("count", selected_count.to_string()),
                            ("size", format_bytes(selected_size)),
                        ],
                    ),
                    app.theme.ok,
                    app.theme,
                ),
                home_detail_line(
                    app.i18n.t("home_detail_scanned"),
                    app.i18n.format(
                        "home_last_scan",
                        &[
                            ("entries", app.scan_summary.entries_seen.to_string()),
                            ("candidates", candidate_count.to_string()),
                            ("size", format_bytes(app.scan_summary.total_size_bytes)),
                        ],
                    ),
                    app.theme.fg_dim,
                    app.theme,
                ),
            ],
        )
    };

    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(content);
    frame.render_widget(block, content);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(Paragraph::new(home_title(title, app.theme)), rows[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(subtitle, Style::default().fg(subtitle_color)))
            .wrap(Wrap { trim: true }),
        rows[1],
    );

    let columns = if rows[3].width >= 72 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[3])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100), Constraint::Percentage(0)])
            .split(rows[3])
    };
    render_home_section(
        frame,
        columns[0],
        app.theme,
        app.i18n.t("home_section_start"),
        actions,
    );
    if columns[1].width > 0 {
        render_home_section(
            frame,
            columns[1],
            app.theme,
            app.i18n.t("home_section_workspace"),
            details,
        );
    }
    frame.render_widget(Paragraph::new(home_safety_line(app)), rows[4]);
}

pub(crate) fn home_title(title: String, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(
        title,
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    ))
}

fn render_home_section(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    heading: String,
    mut lines: Vec<Line<'static>>,
) {
    let mut content = vec![Line::from(Span::styled(
        heading,
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ))];
    content.append(&mut lines);
    frame.render_widget(Paragraph::new(content).wrap(Wrap { trim: true }), area);
}

pub(crate) fn home_detail_line(
    label: String,
    value: String,
    value_color: Color,
    theme: Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}  "), Style::default().fg(theme.fg_dim)),
        Span::styled(value, Style::default().fg(value_color)),
    ])
}

pub(crate) fn home_action_line(
    theme: Theme,
    key: &'static str,
    description: String,
    primary: bool,
) -> Line<'static> {
    let key_style = if primary {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::BOLD)
    };
    let description_style = if primary {
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };

    Line::from(vec![
        Span::styled(format!("[{key}]  "), key_style),
        Span::styled(description, description_style),
    ])
}

pub(crate) fn home_safety_line(app: &Workbench) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "✓ ",
            Style::default()
                .fg(app.theme.ok)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.i18n.t("home_safety_note"),
            Style::default().fg(app.theme.fg_dim),
        ),
    ])
}
