use super::*;

pub(crate) fn render_scan_workspace(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    if app.is_scan_running() {
        render_scan_progress(frame, area, app);
        return;
    }
    app.ensure_scan_view_projection();

    let wide = area.width >= 88;
    let workspace = fluid_content_rect(area, 220, area.height);
    let result_height = if app.last_cleanup_result.is_some() {
        if workspace.width >= 72 { 3 } else { 4 }
    } else {
        0
    };
    let selection_height = if app.plan.is_some() {
        let line_height = if workspace.width < 64 { 2 } else { 1 };
        line_height
            * if app.scan_view.hidden_selected_count > 0 {
                2
            } else {
                1
            }
    } else {
        0
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(result_height),
            Constraint::Fill(1),
            Constraint::Length(selection_height),
        ])
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
    render_scan_selection(frame, rows[2], app);
}

fn render_scan_selection(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let Some(plan) = &app.plan else {
        return;
    };
    let mut lines = vec![Line::from(Span::styled(
        app.i18n.format(
            "scan_selection_global",
            &[
                ("count", plan.summary.selected_count.to_string()),
                ("size", format_bytes(plan.summary.selected_size_bytes)),
            ],
        ),
        Style::default()
            .fg(app.theme.fg)
            .add_modifier(Modifier::BOLD),
    ))];
    if app.scan_view.hidden_selected_count > 0 {
        lines.push(Line::from(Span::styled(
            app.i18n.format(
                "scan_selection_hidden",
                &[
                    ("count", app.scan_view.hidden_selected_count.to_string()),
                    ("size", format_bytes(app.scan_view.hidden_selected_bytes)),
                ],
            ),
            Style::default().fg(app.theme.warn),
        )));
    }
    let line_height = if area.width < 64 { 2 } else { 1 };
    for (index, line) in lines.into_iter().enumerate() {
        let y = area.y.saturating_add(index as u16 * line_height);
        let row = Rect::new(
            area.x,
            y,
            area.width,
            line_height.min(area.bottom().saturating_sub(y)),
        );
        frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), row);
    }
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
    app.ensure_scan_view_projection();
    let filter_label = app.scan_view.filter.as_ref().map_or_else(
        || app.i18n.t("scan_filter_all"),
        |category| category.label(&app.i18n, false),
    );
    let heading = app.i18n.format(
        "scan_candidate_filtered_title",
        &[
            ("visible", app.scan_view.visible.len().to_string()),
            ("total", app.scan_total_count().to_string()),
        ],
    );
    let filter_text = app
        .i18n
        .format("scan_filter_active", &[("category", filter_label)]);
    let inline_filter =
        display_width(&heading) + display_width(&filter_text) + 6 <= area.width as usize;
    let area = if inline_filter {
        area
    } else {
        let filter_bar = Rect::new(area.x, area.y, area.width, area.height.min(1));
        frame.render_widget(
            Paragraph::new(truncate_text(&filter_text, area.width as usize))
                .style(Style::default().fg(app.theme.fg)),
            filter_bar,
        );
        Rect::new(
            area.x,
            area.y.saturating_add(filter_bar.height),
            area.width,
            area.height.saturating_sub(filter_bar.height),
        )
    };
    let item_count = app.scan_view.visible.len();
    let viewport_height = area.height.saturating_sub(1).max(1) as usize;
    app.viewport_height = u16::try_from(viewport_height).unwrap_or(u16::MAX);
    let window = visible_list_window(&mut app.list_state, item_count, viewport_height);
    let has_scrollbar = item_count > viewport_height;
    let content_width = candidate_content_width(area, wide, has_scrollbar);
    // The localized column width is constant for all rows, including custom plugin categories.
    let category_width = if app.i18n.locale().starts_with("zh") {
        9
    } else {
        14
    };
    let items: Vec<ListItem<'static>> = app.scan_view.visible[window.clone()]
        .iter()
        .filter_map(|row_index| {
            let row = &app.scan_view.rows[*row_index];
            let (path, size, selected) = if let Some(plan) = &app.plan {
                let item = plan.items.get(row.source_index)?;
                (&item.path, item.size_bytes, Some(item.selected))
            } else {
                let entry = app.entries.get(row.source_index)?;
                (&entry.path, entry.size_bytes, None)
            };
            Some(ListItem::new(candidate_line(
                path,
                size,
                selected,
                &row.category,
                app,
                content_width,
                category_width,
            )))
        })
        .collect();
    let mut local_state = local_list_state(&app.list_state, &window);

    let mut list_block = Block::default()
        .borders(if wide {
            Borders::TOP | Borders::RIGHT
        } else {
            Borders::TOP
        })
        .border_style(Style::default().fg(app.theme.border))
        .title(format!(" {heading} "))
        .title_style(
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    if inline_filter {
        list_block = list_block.title(
            Line::from(Span::styled(
                format!(" {filter_text} "),
                Style::default().fg(app.theme.fg),
            ))
            .alignment(ratatui::layout::Alignment::Right),
        );
    }
    if has_scrollbar && !wide {
        list_block = list_block.padding(Padding::new(0, 1, 0, 0));
    }
    if item_count == 0 {
        frame.render_widget(
            Paragraph::new(app.i18n.t("scan_filter_empty"))
                .style(Style::default().fg(app.theme.fg_dim))
                .wrap(Wrap { trim: true })
                .block(list_block),
            area,
        );
        return;
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

fn candidate_line(
    path: &std::path::Path,
    size: u64,
    selected: Option<bool>,
    category: &CandidateCategory,
    app: &Workbench,
    content_width: usize,
    category_width: usize,
) -> Line<'static> {
    let check_text = match selected {
        Some(true) => "[✓]",
        Some(false) => "[ ]",
        None => "   ",
    };
    let check_color = if selected == Some(true) {
        app.theme.ok
    } else {
        app.theme.fg_dim
    };
    let size_text = size_cell(size);
    let label = truncate_text(
        &category.key.label(&app.i18n, true),
        category_width.saturating_sub(3),
    );
    let marker = if category.conflict {
        "!"
    } else if category.tentative {
        "?"
    } else {
        ""
    };
    let badge = format!("[{label}]{marker}");
    let padding = " ".repeat(category_width.saturating_sub(display_width(&badge)) + 1);
    let path_width = content_width.saturating_sub(
        display_width(check_text) + 1 + display_width(&size_text) + category_width + 1,
    );
    Line::from(vec![
        Span::styled(check_text, Style::default().fg(check_color)),
        Span::raw(" "),
        Span::styled(size_text, Style::default().fg(app.theme.cyan)),
        Span::styled(badge, Style::default().fg(app.theme.fg)),
        Span::raw(padding),
        Span::raw(compact_path_for_width(path, &app.roots, path_width)),
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
    let path_label = app.i18n.t("detail_path");
    let path_width = (area.width as usize).saturating_sub(4 + display_width(&path_label));

    if let Some(row) = app.selected_scan_row() {
        lines.push(preview_field(
            app.i18n.t("detail_category"),
            category_detail(&row.category, app),
            app.theme.fg,
            app.theme,
        ));
        if row.category.conflict {
            lines.push(Line::from(Span::styled(
                app.i18n.t("scan_category_conflict"),
                Style::default().fg(app.theme.warn),
            )));
        }
        if row.category.tentative {
            lines.push(Line::from(Span::styled(
                app.i18n.t("scan_category_tentative"),
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        if let Some(item) = app
            .plan
            .as_ref()
            .and_then(|plan| plan.items.get(row.source_index))
        {
            lines.push(preview_field(
                app.i18n.t("detail_path"),
                truncate_text(&item.path.display().to_string(), path_width),
                app.theme.fg,
                app.theme,
            ));
            lines.push(Line::from(Span::styled(
                format!(
                    "{}  ·  {}",
                    format_bytes(item.size_bytes),
                    confidence_label(item.confidence)
                ),
                Style::default().fg(confidence_color(item.confidence, app.theme)),
            )));
            lines.push(preview_field(
                app.i18n.t("detail_rule"),
                preview_rule_text(item, |rule| &rule.label, &item.rule_id),
                app.theme.fg,
                app.theme,
            ));
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
        } else if let Some(entry) = app.entries.get(row.source_index) {
            lines.push(preview_field(
                app.i18n.t("detail_path"),
                truncate_text(&entry.path.display().to_string(), path_width),
                app.theme.fg,
                app.theme,
            ));
            let labels = entry
                .rule_hits
                .iter()
                .map(|hit| hit.label.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | ");
            lines.push(preview_field(
                app.i18n.t("detail_rule"),
                labels,
                app.theme.fg,
                app.theme,
            ));
            lines.push(Line::from(app.i18n.t("scan_read_only")));
        }
    } else if app.is_scan_running() {
        lines.push(Line::from(app.i18n.t("plan_scanning")));
        lines.push(Line::from(app.i18n.t("plan_keep_typing")));
    } else if app.scan_total_count() > 0 {
        lines.push(Line::from(app.i18n.t("scan_filter_empty")));
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

fn category_detail(category: &CandidateCategory, app: &Workbench) -> String {
    if category.categories.is_empty() {
        return category.key.label(&app.i18n, false);
    }
    category
        .categories
        .iter()
        .map(|slug| {
            let name = CategoryKey::Named(slug.clone()).label(&app.i18n, false);
            if name == *slug {
                name
            } else {
                format!("{name} ({slug})")
            }
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(crate) fn render_category_filter(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(app.theme.surface))
        .title(format!(" {} ", app.i18n.t("scan_filter_title")))
        .title_style(
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let hint_height = if inner.width < 54 { 2 } else { 1 };
    let rows =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(hint_height)]).split(inner);
    let item_count = app.scan_view.groups.len() + 1;
    let window = visible_list_window(
        &mut app.scan_view.filter_state,
        item_count,
        rows[0].height.max(1) as usize,
    );
    let items = window
        .clone()
        .map(|index| {
            let (label, count, bytes, active) = if index == 0 {
                (
                    app.i18n.t("scan_filter_all"),
                    app.scan_total_count(),
                    app.scan_view.total_size_bytes,
                    app.scan_view.filter.is_none(),
                )
            } else {
                let group = &app.scan_view.groups[index - 1];
                (
                    group.key.label(&app.i18n, false),
                    group.count,
                    group.size_bytes,
                    app.scan_view.filter.as_ref() == Some(&group.key),
                )
            };
            let metrics = format!("{count:>4}  {:>10}", format_bytes(bytes));
            let name_width = (rows[0].width as usize).saturating_sub(display_width(&metrics) + 5);
            let label = truncate_text(&label, name_width);
            let padding = " ".repeat(name_width.saturating_sub(display_width(&label)) + 1);
            ListItem::new(Line::from(vec![
                Span::styled(
                    if active { "• " } else { "  " },
                    Style::default().fg(app.theme.accent),
                ),
                Span::styled(label, Style::default().fg(app.theme.fg)),
                Span::raw(padding),
                Span::styled(metrics, Style::default().fg(app.theme.fg_dim)),
            ]))
        })
        .collect::<Vec<_>>();
    let mut local_state = local_list_state(&app.scan_view.filter_state, &window);
    frame.render_stateful_widget(
        List::new(items).highlight_symbol("› ").highlight_style(
            Style::default()
                .fg(app.theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
        &mut local_state,
    );
    frame.render_widget(
        Paragraph::new(app.i18n.t("scan_filter_hint"))
            .style(Style::default().fg(app.theme.fg_dim))
            .wrap(Wrap { trim: true }),
        rows[1],
    );
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
