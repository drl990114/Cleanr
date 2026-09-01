use super::*;

pub(crate) fn render_usage(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    if app.is_scan_running() {
        render_scan_progress(frame, area, app, app.i18n.t("label_usage"));
        return;
    }

    let item_count = app.usage_order.len();
    let bar_width = usage_bar_width(area.width);

    let details = app
        .list_state
        .selected()
        .and_then(|index| {
            app.usage_order
                .get(index)
                .and_then(|entry_index| app.entries.get(*entry_index))
                .map(|entry| (index, entry))
        })
        .map_or_else(
            || {
                vec![
                    detail_line("Roots", join_paths(&app.roots), app.theme.fg_dim, app.theme),
                    detail_line(
                        "Total",
                        format_bytes(app.scan_summary.total_size_bytes),
                        app.theme.cyan,
                        app.theme,
                    ),
                    detail_line(
                        "Entries",
                        app.scan_summary.entries_seen.to_string(),
                        app.theme.fg,
                        app.theme,
                    ),
                    Line::from(vec![Span::styled(
                        app.i18n.t("usage_context_hint"),
                        Style::default().fg(app.theme.fg_dim),
                    )]),
                ]
            },
            |(index, entry)| {
                vec![
                    detail_line(
                        "Path",
                        entry.path.display().to_string(),
                        app.theme.fg_dim,
                        app.theme,
                    ),
                    detail_line(
                        "Size",
                        format_bytes(entry.size_bytes),
                        app.theme.cyan,
                        app.theme,
                    ),
                    detail_line(
                        "Kind",
                        kind_label(entry.kind).to_string(),
                        app.theme.fg,
                        app.theme,
                    ),
                    detail_line(
                        "Contained",
                        app.usage_descendant_counts
                            .get(index)
                            .copied()
                            .unwrap_or(0)
                            .to_string(),
                        app.theme.ok,
                        app.theme,
                    ),
                    detail_line(
                        "Rule hits",
                        entry.rule_hits.len().to_string(),
                        app.theme.warn,
                        app.theme,
                    ),
                ]
            },
        );

    let title = app.i18n.t("label_usage");
    let detail_title = app.i18n.t("label_details");

    let candidates = app.plan.as_ref().map_or_else(
        || app.candidate_count_cached(),
        |plan| plan.summary.candidate_count,
    );
    let selected = app
        .plan
        .as_ref()
        .map_or(0, |plan| plan.summary.selected_count);
    let overview = Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            app.i18n.t("usage_overview"),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(app.theme.border)),
        metric_span(
            app.i18n.t("usage_metric_total"),
            format_bytes(app.scan_summary.total_size_bytes),
            app.theme.cyan,
        ),
        Span::styled("  ·  ", Style::default().fg(app.theme.border)),
        metric_span(
            app.i18n.t("usage_metric_entries"),
            app.scan_summary.entries_seen.to_string(),
            app.theme.fg,
        ),
        Span::styled("  ·  ", Style::default().fg(app.theme.border)),
        metric_span(
            app.i18n.t("usage_metric_candidates"),
            candidates.to_string(),
            app.theme.warn,
        ),
        Span::styled("  ·  ", Style::default().fg(app.theme.border)),
        metric_span(
            app.i18n.t("usage_metric_selected"),
            selected.to_string(),
            app.theme.ok,
        ),
    ]))
    .wrap(Wrap { trim: true })
    .style(Style::default().bg(app.theme.surface));
    let summary_area = fluid_content_rect(area, 220, 1);
    frame.render_widget(overview, summary_area);

    let content_area = Rect::new(
        area.x,
        summary_area.y.saturating_add(summary_area.height),
        area.width,
        area.bottom()
            .saturating_sub(summary_area.y.saturating_add(summary_area.height)),
    );
    let empty_message = app.i18n.t("status_no_scan_results");
    let entries = &app.entries;
    let usage_order = &app.usage_order;
    let max_size = app.usage_max_size;
    let theme = app.theme;
    app.viewport_height = render_context_workspace_virtualized(
        frame,
        content_area,
        &mut app.list_state,
        theme,
        title,
        item_count,
        move |window| {
            if item_count == 0 {
                return vec![ListItem::new(Line::from(vec![Span::styled(
                    empty_message,
                    Style::default().fg(theme.fg_dim),
                )]))];
            }
            usage_order[window]
                .iter()
                .filter_map(|entry_index| entries.get(*entry_index))
                .map(|entry| ListItem::new(usage_bar_line(entry, max_size, bar_width, theme)))
                .collect()
        },
        detail_title,
        details,
    );
}

#[cfg(test)]
pub(crate) fn usage_descendant_count(entries: &[ScanEntry], parent: &ScanEntry) -> usize {
    if parent.kind != EntryKind::Directory {
        return 0;
    }
    entries
        .iter()
        .filter(|entry| entry.path != parent.path && entry.path.starts_with(&parent.path))
        .count()
}

pub(crate) fn usage_bar_line(
    entry: &ScanEntry,
    max_size: u64,
    bar_width: usize,
    theme: Theme,
) -> Line<'static> {
    let size_str = format_bytes(entry.size_bytes);
    let filled = if max_size == 0 {
        0
    } else {
        usize::try_from(
            (u128::from(entry.size_bytes) * bar_width as u128).div_ceil(u128::from(max_size)),
        )
        .unwrap_or(bar_width)
        .min(bar_width)
    };
    let empty = bar_width.saturating_sub(filled);

    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.path.display().to_string());
    let icon = kind_icon(entry.kind);
    let suffix = if entry.kind == EntryKind::Directory {
        "/"
    } else {
        ""
    };

    Line::from(vec![
        Span::styled(format!("{size_str:>10}"), Style::default().fg(theme.cyan)),
        Span::raw("  "),
        Span::styled("━".repeat(filled), Style::default().fg(theme.accent)),
        Span::styled("─".repeat(empty), Style::default().fg(theme.border)),
        Span::raw("  "),
        Span::styled(
            format!("{icon}{name}{suffix}"),
            Style::default().fg(theme.fg),
        ),
    ])
}

pub(crate) fn usage_bar_width(area_width: u16) -> usize {
    match area_width {
        0..=54 => 4,
        55..=84 => 8,
        85..=124 => 12,
        _ => 18,
    }
}
