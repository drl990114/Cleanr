use super::*;

pub(crate) fn render_scan_workspace(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    if app.is_scan_running() {
        render_scan_progress(frame, area, app);
        return;
    }

    let wide = area.width >= 88;
    let workspace = fluid_content_rect(area, 220, area.height);
    let result_height = if app.last_cleanup_result.is_some() {
        if workspace.width >= 72 { 3 } else { 4 }
    } else {
        0
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(result_height), Constraint::Fill(1)])
        .split(workspace);
    if result_height > 0 {
        render_cleanup_result(frame, rows[0], app);
    }
    let has_candidates = app.plan.as_ref().map_or_else(
        || app.candidate_count_cached() > 0,
        |plan| plan.summary.candidate_count > 0,
    );
    if app
        .last_cleanup_result
        .as_ref()
        .is_some_and(|result| result.succeeded > 0 && result.failed == 0 && !has_candidates)
    {
        app.viewport_height = 1;
        return;
    }
    let columns = responsive_workspace(rows[1], 62);

    render_candidates(frame, columns[0], app, wide);
    render_preview(frame, columns[1], app);
    app.viewport_height = columns[0].height.saturating_sub(1).max(1);
}

fn render_cleanup_result(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let Some(result) = &app.last_cleanup_result else {
        return;
    };
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
    let path = result.first_path.as_ref().map_or_else(
        || app.i18n.t("cleanup_result_no_items"),
        |path| {
            let first = compact_path(path, &app.roots);
            if result.succeeded == 1 {
                first
            } else {
                app.i18n.format(
                    "cleanup_result_paths_more",
                    &[
                        ("path", first),
                        ("count", result.succeeded.saturating_sub(1).to_string()),
                    ],
                )
            }
        },
    );
    let path_width = area.width.saturating_sub(4) as usize;
    let title = Line::from(vec![
        Span::styled(
            "✓ ",
            Style::default()
                .fg(app.theme.ok)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.i18n.t("cleanup_result_title"),
            Style::default()
                .fg(app.theme.fg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let path = Line::from(Span::styled(
        truncate_text(&path, path_width),
        Style::default().fg(app.theme.fg_dim),
    ));
    let lines = if area.width >= 72 {
        vec![
            Line::from(vec![
                Span::styled(
                    "✓ ",
                    Style::default()
                        .fg(app.theme.ok)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    app.i18n.t("cleanup_result_title"),
                    Style::default()
                        .fg(app.theme.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ·  ", Style::default().fg(app.theme.border)),
                Span::styled(summary, Style::default().fg(app.theme.cyan)),
            ]),
            path,
        ]
    } else {
        vec![
            title,
            Line::from(Span::styled(summary, Style::default().fg(app.theme.cyan))),
            path,
        ]
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(app.theme.border))
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

pub(crate) fn render_scan_progress(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let mut panel_area = fluid_content_rect(area, 220, area.height);
    if area.height > panel_area.height {
        panel_area.y = panel_area.y.saturating_add(1);
    }
    let panel = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::horizontal(2));
    let inner = panel.inner(panel_area);
    frame.render_widget(panel, panel_area);

    let progress = app.scan_progress.as_ref();
    let stage = progress.map_or(ScanStage::Resolving, |value| value.stage);
    let summary = scan_progress_summary(progress, app);
    let is_wide = inner.width >= 96;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if is_wide {
            vec![Constraint::Length(1), Constraint::Length(1)]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
        })
        .split(inner);
    let phase_line = Line::from(vec![
        Span::styled(
            format!("{}  ", scan_spinner_frame(app.animation_tick)),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if app.scan_cancel_requested {
                app.i18n.t("status_scan_cancelling")
            } else {
                app.scan_stage_label(stage)
            },
            Style::default()
                .fg(app.theme.fg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let path_row = if is_wide {
        let heading = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(rows[0]);
        frame.render_widget(Paragraph::new(phase_line), heading[0]);
        frame.render_widget(
            Paragraph::new(summary)
                .style(Style::default().fg(app.theme.fg_dim))
                .alignment(ratatui::layout::Alignment::Right),
            heading[1],
        );
        rows[1]
    } else {
        frame.render_widget(Paragraph::new(phase_line), rows[0]);
        frame.render_widget(
            Paragraph::new(summary).style(Style::default().fg(app.theme.fg_dim)),
            rows[1],
        );
        rows[2]
    };

    let Some(current_path) = progress.and_then(|value| value.current_path.as_ref()) else {
        return;
    };
    let current_path_label = format!("{}  ", app.i18n.t("scan_current_path"));
    let current_path_width = path_row
        .width
        .saturating_sub(u16::try_from(display_width(&current_path_label)).unwrap_or(u16::MAX))
        as usize;
    let current_path = compact_path_for_width(current_path, &app.roots, current_path_width);
    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::styled(current_path_label, Style::default().fg(app.theme.fg_dim)),
            Span::styled(current_path, Style::default().fg(app.theme.fg_dim)),
        ])])
        .alignment(ratatui::layout::Alignment::Left)
        .wrap(Wrap { trim: true }),
        path_row,
    );
}

fn scan_progress_summary(progress: Option<&ScanTaskProgress>, app: &Workbench) -> String {
    let Some(value) = progress else {
        return app.i18n.t("scan_preparing");
    };
    let progress = if value.stage == ScanStage::Scanning {
        if value.entries_total == 0 {
            app.i18n.format(
                "scan_progress_unbounded",
                &[("scanned", value.entries_scanned.to_string())],
            )
        } else {
            app.i18n.format(
                "scan_progress_count",
                &[
                    ("scanned", value.entries_scanned.to_string()),
                    ("total", value.entries_total.to_string()),
                ],
            )
        }
    } else if value.entries_scanned > 0 {
        app.i18n.format(
            "scan_progress_discovered",
            &[("total", value.entries_scanned.to_string())],
        )
    } else {
        app.i18n.t("scan_preparing")
    };
    let key = if value.errors == 0 {
        "scan_progress_summary"
    } else {
        "scan_progress_summary_with_errors"
    };
    app.i18n.format(
        key,
        &[
            ("progress", progress),
            ("size", format_bytes(value.bytes_scanned)),
            ("elapsed", app.scan_elapsed_label()),
            ("errors", value.errors.to_string()),
        ],
    )
}

fn scan_spinner_frame(animation_tick: u64) -> &'static str {
    spinner_frame(animation_tick / 2)
}

#[cfg(test)]
pub(crate) fn scan_loading_indicator_sample(animation_tick: u64) -> &'static str {
    scan_spinner_frame(animation_tick)
}

pub(crate) fn render_candidates(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut Workbench,
    wide: bool,
) {
    app.rebuild_candidate_projection_if_stale();
    let item_count = app
        .plan
        .as_ref()
        .map_or_else(|| app.candidate_count, |plan| plan.items.len());
    let viewport_height = area.height.saturating_sub(1).max(1) as usize;
    let window = visible_list_window(&mut app.list_state, item_count, viewport_height);
    let has_scrollbar = item_count > viewport_height;
    let content_width = candidate_content_width(area, wide, has_scrollbar);
    let items: Vec<ListItem<'static>> = if let Some(plan) = &app.plan {
        plan.items
            .iter()
            .skip(window.start)
            .take(window.len())
            .map(|item| {
                ListItem::new(plan_candidate_line(
                    item,
                    &app.roots,
                    app.theme,
                    content_width,
                ))
            })
            .collect()
    } else {
        app.candidate_entry_indices
            .iter()
            .skip(window.start)
            .take(window.len())
            .filter_map(|entry_index| app.entries.get(*entry_index))
            .map(|entry| {
                ListItem::new(scan_candidate_line(
                    entry,
                    &app.roots,
                    app.theme,
                    content_width,
                ))
            })
            .collect()
    };
    let mut local_state = local_list_state(&app.list_state, &window);

    let mut list_block = Block::default()
        .borders(if wide {
            Borders::TOP | Borders::RIGHT
        } else {
            Borders::TOP
        })
        .border_style(Style::default().fg(app.theme.border))
        .title(format!(
            " {} ",
            app.i18n
                .format("scan_candidate_title", &[("count", item_count.to_string())],)
        ))
        .title_style(
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    if has_scrollbar && !wide {
        list_block = list_block.padding(Padding::new(0, 1, 0, 0));
    }

    let list = List::new(items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .fg(app.theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    frame.render_stateful_widget(list, area, &mut local_state);
    render_list_scrollbar(
        frame,
        area,
        item_count,
        viewport_height,
        app.list_state.selected().unwrap_or(window.start),
        app.theme,
    );
}

fn candidate_content_width(area: Rect, wide: bool, has_scrollbar: bool) -> usize {
    let right_border: u16 = if wide { 1 } else { 0 };
    let scrollbar_gutter = if has_scrollbar && !wide { 1 } else { 0 };
    usize::from(area.width.saturating_sub(right_border).saturating_sub(2))
        .saturating_sub(scrollbar_gutter)
}

fn plan_candidate_line(
    item: &CleanupItem,
    roots: &[PathBuf],
    theme: Theme,
    content_width: usize,
) -> Line<'static> {
    let check_text = if item.selected { "[✓]" } else { "[ ]" };
    let check = if item.selected {
        Span::styled(check_text, Style::default().fg(theme.ok))
    } else {
        Span::styled(check_text, Style::default().fg(theme.fg_dim))
    };
    let size_text = size_cell(item.size_bytes);
    let fixed_width = display_width(check_text) + 1 + display_width(&size_text);
    let path_width = content_width.saturating_sub(fixed_width);

    Line::from(vec![
        check,
        Span::raw(" "),
        Span::styled(size_text, Style::default().fg(theme.cyan)),
        Span::raw(compact_path_for_width(&item.path, roots, path_width)),
    ])
}

fn scan_candidate_line(
    entry: &ScanEntry,
    roots: &[PathBuf],
    theme: Theme,
    content_width: usize,
) -> Line<'static> {
    let size_text = size_cell(entry.size_bytes);
    let fixed_width = 2 + display_width(&size_text);
    let path_width = content_width.saturating_sub(fixed_width);

    Line::from(vec![
        Span::raw("  "),
        Span::styled(size_text, Style::default().fg(theme.cyan)),
        Span::raw(compact_path_for_width(&entry.path, roots, path_width)),
    ])
}

fn size_cell(bytes: u64) -> String {
    format!("{:>10} ", format_bytes(bytes))
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

pub(crate) fn render_preview(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let mut lines: Vec<Line> = Vec::new();
    let inner_width = area.width.saturating_sub(2) as usize;

    if let Some(plan) = &app.plan {
        lines.push(Line::from(Span::styled(
            app.i18n.format(
                "plan_selection_summary",
                &[
                    ("selected", plan.summary.selected_count.to_string()),
                    ("total", plan.summary.candidate_count.to_string()),
                    ("size", format_bytes(plan.summary.selected_size_bytes)),
                ],
            ),
            Style::default()
                .fg(app.theme.cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        if let Some(idx) = app.list_state.selected()
            && let Some(item) = plan.items.get(idx)
        {
            lines.push(Line::from(vec![Span::styled(
                app.i18n.t("plan_current_item"),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )]));
            let path_label = app.i18n.t("detail_path");
            let path_width = inner_width
                .saturating_sub(display_width(&path_label))
                .saturating_sub(2);
            lines.push(preview_field(
                path_label,
                truncate_text(&item.path.display().to_string(), path_width),
                app.theme.fg,
                app.theme,
            ));
            lines.push(Line::from(Span::styled(
                app.i18n.format(
                    "plan_item_meta",
                    &[
                        ("size", format_bytes(item.size_bytes)),
                        ("category", item.category.clone()),
                        ("confidence", confidence_label(item.confidence).to_string()),
                    ],
                ),
                Style::default().fg(confidence_color(item.confidence, app.theme)),
            )));
            lines.push(preview_field(
                app.i18n.t("detail_reason"),
                preview_rule_text(item, |rule| &rule.reason, &item.reason),
                app.theme.fg,
                app.theme,
            ));
            lines.push(preview_field(
                app.i18n.t("detail_risk"),
                preview_rule_text(item, |rule| &rule.risk_note, &item.risk_note),
                app.theme.warn,
                app.theme,
            ));
        }
    } else if app.is_scan_running() {
        lines.push(Line::from(app.i18n.t("plan_scanning")));
        lines.push(Line::from(app.i18n.t("plan_keep_typing")));
    } else {
        lines.push(Line::from(app.i18n.t("plan_empty")));
        lines.push(Line::from(app.i18n.t("plan_empty_hint")));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(app.theme.border))
            .padding(Padding::horizontal(1))
            .title(format!(" {} ", app.i18n.t("label_details")))
            .title_style(
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    frame.render_widget(paragraph, area);
}

fn preview_rule_text(
    item: &CleanupItem,
    field: impl for<'a> Fn(&'a cleanr_core::RuleEvidence) -> &'a str,
    fallback: &str,
) -> String {
    let Some(evidence) = &item.evidence else {
        return fallback.to_string();
    };
    let mut values = Vec::new();
    for rule in &evidence.matched_rules {
        let value = field(rule);
        if !values.contains(&value) {
            values.push(value);
        }
    }
    if values.is_empty() {
        fallback.to_string()
    } else {
        values.join(" | ")
    }
}

fn preview_field(label: String, value: String, value_color: Color, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": "),
        Span::styled(value, Style::default().fg(value_color)),
    ])
}
