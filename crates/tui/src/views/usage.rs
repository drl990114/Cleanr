use super::*;

pub(crate) fn render_usage(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    if app.is_scan_running() {
        render_scan_progress(frame, area, app);
        return;
    }
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{}  ", app.i18n.t("usage_metric_total")),
                Style::default().fg(app.theme.fg_dim),
            ),
            Span::styled(
                format_bytes(app.scan_summary.total_size_bytes),
                Style::default()
                    .fg(app.theme.fg)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[0],
    );
    let mut details = Vec::new();
    let mut more = vec![
        detail_line(
            &app.i18n.t("home_detail_scope"),
            join_paths(&app.roots),
            app.theme.fg_dim,
            app.theme,
        ),
        detail_line(
            &app.i18n.t("usage_metric_entries"),
            app.scan_summary.entries_seen.to_string(),
            app.theme.fg_dim,
            app.theme,
        ),
    ];
    if let Some((index, entry)) = app.list_state.selected().and_then(|index| {
        app.usage_order
            .get(index)
            .and_then(|entry_index| app.entries.get(*entry_index))
            .map(|entry| (index, entry))
    }) {
        details.push(home_title(
            entry.path.file_name().map_or_else(
                || entry.path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            app.theme,
        ));
        details.push(Line::from(format_bytes(entry.size_bytes)));
        detail_section(
            &mut details,
            app.i18n.t("detail_path"),
            entry.path.display().to_string(),
            app.theme.fg,
            app.theme,
        );
        more.extend([
            detail_line(
                &app.i18n.t("detail_kind"),
                app.i18n.t(&format!("kind_{}", kind_label(entry.kind))),
                app.theme.fg_dim,
                app.theme,
            ),
            detail_line(
                &app.i18n.t("detail_contained"),
                app.usage_descendant_counts
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .to_string(),
                app.theme.fg_dim,
                app.theme,
            ),
            detail_line(
                &app.i18n.t("detail_matched_rules"),
                entry.rule_hits.len().to_string(),
                app.theme.fg_dim,
                app.theme,
            ),
        ]);
    } else {
        details.push(Line::from(app.i18n.t("status_no_scan_results")));
    }
    let list_width = responsive_workspace(area, frame.area().width >= 88)[0].width;
    let bar_width = match list_width {
        0..=42 => 4,
        43..=64 => 8,
        _ => 12,
    };
    let size_width = display_width(&format_bytes(app.scan_summary.total_size_bytes)).max(10) as u16;
    let content = ContextContent {
        title: app.i18n.t("label_usage"),
        count: app.usage_order.len(),
        columns: vec![
            Constraint::Fill(1),
            Constraint::Length(bar_width),
            Constraint::Length(size_width),
        ],
        empty: app.i18n.t(if app.usage_rx.is_some() {
            "scan_phase_usage"
        } else {
            "status_no_scan_results"
        }),
        details,
        more,
    };
    render_context_workspace(frame, rows[1], app, content, |app, window, widths| {
        app.usage_order[window]
            .iter()
            .filter_map(|index| app.entries.get(*index))
            .map(|entry| {
                let name = compact_path(&entry.path, &app.roots);
                let name = if entry.kind == EntryKind::Directory {
                    format!("{name}/")
                } else {
                    name
                };
                let filled = if app.usage_max_size == 0 {
                    0
                } else {
                    ((u128::from(entry.size_bytes) * u128::from(widths[1]))
                        .div_ceil(u128::from(app.usage_max_size)) as usize)
                        .min(widths[1] as usize)
                };
                Row::new(vec![
                    text_cell(name, widths[0], app.theme.fg),
                    Cell::from(Line::from(vec![
                        Span::styled("━".repeat(filled), Style::default().fg(app.theme.accent)),
                        Span::styled(
                            "─".repeat(widths[1] as usize - filled),
                            Style::default().fg(app.theme.border),
                        ),
                    ])),
                    right_cell(format_bytes(entry.size_bytes), app.theme.fg),
                ])
            })
            .collect()
    });
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
