use super::*;
use crate::app::CleanupResult;

pub(crate) fn cleanup_result_summary(app: &Workbench, result: &CleanupResult) -> String {
    if result.interruption.is_some() {
        return app.i18n.t("cleanup_result_unconfirmed");
    }
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
    summary
}

pub(crate) fn render_cleanup_result(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let Some(result) = &app.last_cleanup_result else {
        return;
    };
    let (marker, color) = if result.interruption.is_some() {
        ("! ", app.theme.warn)
    } else if result.failed == 0 {
        ("✓ ", app.theme.ok)
    } else if result.succeeded == 0 {
        ("× ", app.theme.danger)
    } else {
        ("! ", app.theme.warn)
    };
    let summary = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(marker, Style::default().fg(color)),
            Span::styled(
                app.i18n.t(result.title_key()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            cleanup_result_summary(app, result),
            Style::default().fg(app.theme.fg),
        )),
        Line::from(""),
    ])
    .wrap(Wrap { trim: true });
    let summary_height = summary.line_count(area.width).min(u16::MAX as usize) as u16;
    let rows =
        Layout::vertical([Constraint::Length(summary_height), Constraint::Fill(1)]).split(area);
    frame.render_widget(summary, rows[0]);

    let mut details = Vec::new();
    if let Some(error) = &result.interruption {
        details.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(app.theme.warn),
        )));
    } else {
        if let Some(path) = &result.first_path {
            details.push(home_detail_line(
                app.i18n.t("cleanup_result_items"),
                if result.succeeded == 1 {
                    compact_path(path, &app.roots)
                } else {
                    app.i18n.format(
                        "cleanup_result_paths_more",
                        &[
                            ("path", compact_path(path, &app.roots)),
                            ("count", result.succeeded.saturating_sub(1).to_string()),
                        ],
                    )
                },
                app.theme.fg_dim,
                app.theme,
            ));
        } else {
            details.push(Line::from(app.i18n.t("cleanup_result_no_items")));
        }
        if let Some((path, error)) = &result.first_failure {
            details.push(Line::from(""));
            details.push(home_detail_line(
                app.i18n.t("cleanup_result_first_failure"),
                compact_path(path, &app.roots),
                app.theme.warn,
                app.theme,
            ));
            details.push(Line::from(Span::styled(
                error.clone(),
                Style::default().fg(app.theme.warn),
            )));
        }
    }
    details.push(Line::from(""));
    details.push(Line::from(Span::styled(
        app.i18n.t("cleanup_result_rescan_note"),
        Style::default().fg(app.theme.fg_dim),
    )));
    let details_area = rows[1];
    let paragraph = Paragraph::new(details).wrap(Wrap { trim: true });
    app.details.viewport_height = details_area.height;
    app.details.max_scroll = u16::try_from(
        paragraph
            .line_count(details_area.width)
            .saturating_sub(details_area.height as usize),
    )
    .unwrap_or(u16::MAX);
    app.details.scroll = app.details.scroll.min(app.details.max_scroll);
    frame.render_widget(paragraph.scroll((app.details.scroll, 0)), details_area);
}
