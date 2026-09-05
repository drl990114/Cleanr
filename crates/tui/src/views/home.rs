use super::*;

pub(crate) fn render_home(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let height = if area.height > 10 {
        area.height - 1
    } else {
        area.height
    };
    let mut content = fluid_content_rect(area, 120, height);
    if area.height > content.height {
        content.y = content.y.saturating_add(1);
    }

    let candidate_count = app.plan.as_ref().map_or_else(
        || app.candidate_count_cached(),
        |plan| plan.summary.candidate_count,
    );

    let (title, summary, primary, secondary, detail) =
        if let Some(result) = &app.last_cleanup_result {
            let mut summary = app.i18n.format(
                "cleanup_result_summary",
                &[
                    ("count", result.succeeded.to_string()),
                    ("size", format_bytes(result.cleaned_size_bytes)),
                ],
            );
            if result.failed > 0 {
                summary.push_str("  ·  ");
                summary.push_str(&app.i18n.format(
                    "cleanup_result_failed",
                    &[("count", result.failed.to_string())],
                ));
            }
            (
                app.i18n.t("cleanup_result_title"),
                summary,
                home_action_line(app.theme, "s", app.i18n.t("home_action_rescan"), true),
                home_secondary_actions(
                    app.theme,
                    "z",
                    app.i18n.t("hint_restore_result"),
                    "u",
                    app.i18n.t("home_action_usage"),
                ),
                cleanup_result_path_line(app, 76),
            )
        } else if !app.has_scan_results() {
            (
                app.i18n.t("home_welcome"),
                app.i18n.t("home_subtitle"),
                home_action_line(app.theme, "s", app.i18n.t("home_action_scan"), true),
                home_secondary_actions(
                    app.theme,
                    "u",
                    app.i18n.t("home_action_usage"),
                    "/",
                    app.i18n.t("home_action_more"),
                ),
                home_detail_line(
                    app.i18n.t("home_detail_scope"),
                    truncate_text(&join_paths(&app.roots), 76),
                    app.theme.fg_dim,
                    app.theme,
                ),
            )
        } else if candidate_count == 0 {
            (
                app.i18n.t("home_result_title"),
                scan_empty_text(app),
                home_action_line(app.theme, "s", app.i18n.t("home_action_rescan"), true),
                home_secondary_actions(
                    app.theme,
                    "u",
                    app.i18n.t("home_action_usage"),
                    "/",
                    app.i18n.t("home_action_more"),
                ),
                home_detail_line(
                    app.i18n.t("home_detail_scanned"),
                    format_bytes(app.scan_summary.total_size_bytes),
                    app.theme.fg_dim,
                    app.theme,
                ),
            )
        } else {
            let reclaimable = app
                .plan
                .as_ref()
                .map_or(0, |plan| plan.summary.total_candidate_size_bytes);
            (
                app.i18n.t("home_result_title"),
                app.i18n.format(
                    "home_result_summary",
                    &[
                        ("size", format_bytes(reclaimable)),
                        ("count", candidate_count.to_string()),
                    ],
                ),
                home_action_line(app.theme, "r", app.i18n.t("home_action_review"), true),
                home_secondary_actions(
                    app.theme,
                    "s",
                    app.i18n.t("home_action_rescan"),
                    "/",
                    app.i18n.t("home_action_more"),
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
            )
        };

    let mut lines = vec![
        home_title(title, app.theme),
        Line::from(Span::styled(summary, Style::default().fg(app.theme.fg_dim))),
        Line::from(""),
        primary,
        secondary,
        Line::from(""),
        detail,
        home_safety_line(app),
    ];
    if let Some(notice) = &app.update_notice {
        lines.push(Line::from(Span::styled(
            app.i18n.format(
                "status_update_available",
                &[
                    ("version", notice.version.clone()),
                    ("url", notice.release_url.clone()),
                ],
            ),
            Style::default().fg(app.theme.fg_dim),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().padding(Padding::horizontal(2))),
        content,
    );
}

pub(crate) fn home_title(title: String, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(
        title,
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    ))
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

fn home_secondary_actions(
    theme: Theme,
    first_key: &'static str,
    first_label: String,
    second_key: &'static str,
    second_label: String,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("[{first_key}]  "),
            Style::default()
                .fg(theme.fg_dim)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(first_label, Style::default().fg(theme.fg_dim)),
        Span::raw("    "),
        Span::styled(
            format!("[{second_key}]  "),
            Style::default()
                .fg(theme.fg_dim)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(second_label, Style::default().fg(theme.fg_dim)),
    ])
}

fn cleanup_result_path_line(app: &Workbench, max_width: usize) -> Line<'static> {
    let Some(result) = &app.last_cleanup_result else {
        return Line::from("");
    };
    let Some(first) = result.first_path.as_ref() else {
        return home_detail_line(
            app.i18n.t("home_detail_state"),
            app.i18n.format(
                "cleanup_result_failed",
                &[("count", result.failed.to_string())],
            ),
            app.theme.warn,
            app.theme,
        );
    };
    let first = compact_path(first, &app.roots);
    let value = if result.succeeded == 1 {
        first
    } else {
        app.i18n.format(
            "cleanup_result_paths_more",
            &[
                ("path", first),
                ("count", result.succeeded.saturating_sub(1).to_string()),
            ],
        )
    };
    home_detail_line(
        app.i18n.t("cleanup_result_items"),
        truncate_text(&value, max_width),
        app.theme.fg,
        app.theme,
    )
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
