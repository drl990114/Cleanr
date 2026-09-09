use super::*;

pub(crate) fn render_scan_workspace(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    if app.is_scan_running() {
        render_scan_progress(frame, area, app);
        return;
    }
    app.ensure_scan_view_projection();

    let wide = frame.area().width >= 88;
    let workspace = area;
    let result_height = if app.last_cleanup_result.is_some() {
        if workspace.width >= 72 { 3 } else { 4 }
    } else {
        0
    };
    let selection_height = Paragraph::new(scan_selection_lines(app))
        .wrap(Wrap { trim: true })
        .line_count(workspace.width)
        .min(u16::MAX as usize) as u16;
    let days = app.analysis.as_ref().map_or_else(
        || app.effective_inactive_days(None),
        |a| a.policy.preselect_after_days,
    );
    let age = app.i18n.format(
        if days == 0 {
            "scope_all_ages"
        } else {
            "scope_age"
        },
        &[("days", days.to_string())],
    );
    let extra_roots = if app.roots.len() > 1 {
        app.i18n
            .format("scope_roots", &[("count", app.roots.len().to_string())])
    } else {
        String::new()
    };
    let suffix = format!(" · {age}{extra_roots}");
    let path_width = (workspace.width as usize).saturating_sub(display_width(&suffix));
    let scope = format!(
        "{}{suffix}",
        compact_path_for_width(
            &app.roots.first().cloned().unwrap_or_default(),
            &[],
            path_width
        )
    );
    let scope = Paragraph::new(scope)
        .style(Style::default().fg(app.theme.fg_dim))
        .wrap(Wrap { trim: true });
    let scope_height = scope.line_count(workspace.width).min(u16::MAX as usize) as u16;
    let rows = Layout::vertical([
        Constraint::Length(result_height),
        Constraint::Length(scope_height),
        Constraint::Fill(1),
        Constraint::Length(selection_height),
    ])
    .split(workspace);
    if result_height > 0 {
        render_cleanup_result(frame, rows[0], app);
    }
    frame.render_widget(scope, rows[1]);
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
    // The details overlay must cover the selection footer in narrow terminals.
    render_scan_selection(frame, rows[3], app);
    if wide {
        let columns = responsive_workspace(rows[2], true);
        render_candidates(frame, columns[0], app, true);
        render_preview(frame, columns[1], app);
    } else {
        render_candidates(frame, rows[2], app, false);
        if app.details.focused {
            render_preview(frame, area, app);
        }
    }
}

fn scan_selection_lines(app: &Workbench) -> Vec<Line<'static>> {
    let Some(plan) = &app.plan else {
        return Vec::new();
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
    if app.scan_view.selected_review_count > 0 {
        lines.push(Line::from(Span::styled(
            app.i18n.format(
                "confirm_review_count",
                &[("count", app.scan_view.selected_review_count.to_string())],
            ),
            Style::default().fg(app.theme.warn),
        )));
    }
    lines
}

fn render_scan_selection(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    frame.render_widget(
        Paragraph::new(scan_selection_lines(app)).wrap(Wrap { trim: true }),
        area,
    );
}

fn render_cleanup_result(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let Some(result) = &app.last_cleanup_result else {
        return;
    };
    let (marker, result_color) = if result.failed == 0 {
        ("✓ ", app.theme.ok)
    } else if result.succeeded == 0 {
        ("× ", app.theme.danger)
    } else {
        ("! ", app.theme.warn)
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
            marker,
            Style::default()
                .fg(result_color)
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
                    marker,
                    Style::default()
                        .fg(result_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    app.i18n.t("cleanup_result_title"),
                    Style::default()
                        .fg(app.theme.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ·  ", Style::default().fg(app.theme.border)),
                Span::styled(summary, Style::default().fg(app.theme.fg)),
            ]),
            path,
        ]
    } else {
        vec![
            title,
            Line::from(Span::styled(summary, Style::default().fg(app.theme.fg))),
            path,
        ]
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(app.theme.border))
                .padding(Padding::horizontal(0)),
        ),
        area,
    );
}

pub(crate) fn render_scan_progress(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let panel_area = area;
    let panel = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::horizontal(0));
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
    _wide: bool,
) {
    app.ensure_scan_view_projection();
    let mut filters = Vec::new();
    if let Some(category) = &app.scan_view.filter {
        filters.push(format!("{} [f]", category.label(&app.i18n, false)));
    }
    if !app.scan_view.query.is_empty() && !app.scan_view.search_open {
        filters.push(format!("{} [p]", app.scan_view.query));
    }
    if app.scan_view.only_selected {
        filters.push(format!("{} [v]", app.i18n.t("scan_selected_only")));
    }
    if app.scan_view.sort != crate::projection::ScanSort::Plan {
        filters.push(format!(
            "{} [o]",
            app.i18n.t(app.scan_view.sort.label_key())
        ));
    }
    let bar_height = u16::from(app.scan_view.search_open || !filters.is_empty()).min(area.height);
    let rows = Layout::vertical([Constraint::Length(bar_height), Constraint::Fill(1)]).split(area);
    if app.scan_view.search_open {
        render_scan_search(frame, rows[0], app);
    } else if bar_height > 0 {
        frame.render_widget(
            Paragraph::new(truncate_text(&filters.join(" · "), area.width as usize))
                .style(Style::default().fg(app.theme.fg_dim)),
            rows[0],
        );
    }
    let heading = if app.scan_projection_pending() {
        app.i18n.t("scan_filter_processing")
    } else if app.scan_view.visible.len() == app.scan_total_count() {
        app.i18n.format(
            "scan_candidate_title",
            &[("count", app.scan_total_count().to_string())],
        )
    } else {
        app.i18n.format(
            "scan_candidate_filtered_title",
            &[
                ("visible", app.scan_view.visible.len().to_string()),
                ("total", app.scan_total_count().to_string()),
            ],
        )
    };
    let category_width = if app.i18n.locale().starts_with("zh") {
        8
    } else {
        12
    };
    let total_bytes = app
        .plan
        .as_ref()
        .map_or(app.scan_summary.total_size_bytes, |plan| {
            plan.summary.total_candidate_size_bytes
        });
    let size_width = display_width(&format_bytes(total_bytes)).max(10) as u16;
    let show_category = rows[1].width >= size_width + category_width + 28;
    let mut constraints = vec![
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
    ];
    if show_category {
        constraints.push(Constraint::Length(category_width));
    }
    constraints.push(Constraint::Length(size_width));
    render_table_list(
        frame,
        rows[1],
        app,
        heading,
        app.scan_view.visible.len(),
        &constraints,
        scan_empty_text(app),
        |app, window, widths| {
            app.scan_view.visible[window.clone()]
                .iter()
                .enumerate()
                .filter_map(|(offset, row_index)| {
                    let row = &app.scan_view.rows[*row_index];
                    let (path, size, selected, review) = if let Some(plan) = &app.plan {
                        let item = plan.items.get(row.source_index)?;
                        (
                            &item.path,
                            item.size_bytes,
                            Some(item.selected),
                            item.evidence.as_ref().is_some_and(|e| {
                                e.recommendation_state == cleanr_core::RecommendationState::Review
                            }),
                        )
                    } else {
                        let entry = app.entries.get(row.source_index)?;
                        (&entry.path, entry.size_bytes, None, false)
                    };
                    let checked = match selected {
                        Some(true) => "[✓]",
                        Some(false) => "[ ]",
                        None => "",
                    };
                    let marker = if review || row.category.conflict {
                        "!"
                    } else if row.category.tentative {
                        "?"
                    } else {
                        ""
                    };
                    let focused = app.list_state.selected() == Some(window.start + offset)
                        && !app.details.focused;
                    let mut cells = vec![
                        text_cell(
                            checked,
                            widths[0],
                            if selected == Some(true) {
                                app.theme.ok
                            } else {
                                app.theme.fg_dim
                            },
                        ),
                        text_cell(marker, widths[1], app.theme.warn),
                        text_cell(
                            compact_path_for_width(path, &app.roots, widths[2] as usize),
                            widths[2],
                            if focused {
                                app.theme.accent
                            } else {
                                app.theme.fg
                            },
                        ),
                    ];
                    if show_category {
                        cells.push(text_cell(
                            row.category.key.label(&app.i18n, true),
                            widths[3],
                            app.theme.fg_dim,
                        ));
                    }
                    cells.push(right_cell(format_bytes(size), app.theme.fg));
                    Some(Row::new(cells))
                })
                .collect()
        },
    );
}

fn render_scan_search(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    if area.is_empty() {
        return;
    }
    let prefix = format!("{}: ", app.i18n.t("hint_find_path"));
    let prefix_width = display_width(&prefix).min(area.width.saturating_sub(1) as usize);
    let (text, cursor) = command_input_view(
        &app.input,
        app.input_cursor,
        (area.width as usize).saturating_sub(prefix_width + 1),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                truncate_text(&prefix, prefix_width),
                Style::default().fg(app.theme.accent),
            ),
            Span::raw(text),
        ])),
        area,
    );
    if !app.help_open && !app.confirmation_pending() {
        frame.set_cursor_position(Position::new(
            area.x + (prefix_width + cursor).min(area.width.saturating_sub(1) as usize) as u16,
            area.y,
        ));
    }
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

pub(crate) fn render_preview(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let mut lines = Vec::new();
    let mut more = Vec::new();
    if let Some(row) = app.selected_scan_row() {
        let name = row.path.file_name().map_or_else(
            || row.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        lines.push(home_title(name, app.theme));
        lines.push(Line::from(format!(
            "{} · {}",
            format_bytes(row.size_bytes),
            row.category.key.label(&app.i18n, false)
        )));
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
        more.push(detail_line(
            &app.i18n.t("detail_category"),
            category_detail(&row.category, app),
            app.theme.fg_dim,
            app.theme,
        ));
        if let Some(item) = app
            .plan
            .as_ref()
            .and_then(|plan| plan.items.get(row.source_index))
        {
            if let Some(evidence) = &item.evidence {
                let review =
                    evidence.recommendation_state == cleanr_core::RecommendationState::Review;
                lines.push(detail_line(
                    &app.i18n.t("detail_recommendation"),
                    app.i18n
                        .t(&format!("recommendation_{}", evidence.recommendation_state)),
                    if review { app.theme.warn } else { app.theme.fg },
                    app.theme,
                ));
            }
            detail_section(
                &mut lines,
                app.i18n.t("detail_risk"),
                preview_rule_text(item, |rule| &rule.risk_note, &item.risk_note),
                app.theme.warn,
                app.theme,
            );
            detail_section(
                &mut lines,
                app.i18n.t("detail_reason"),
                preview_rule_text(item, |rule| &rule.reason, &item.reason),
                app.theme.fg,
                app.theme,
            );
            detail_section(
                &mut lines,
                app.i18n.t("detail_path"),
                item.path.display().to_string(),
                app.theme.fg,
                app.theme,
            );
            more.push(Line::from(
                app.i18n
                    .t(&format!("confidence_{}", confidence_label(item.confidence))),
            ));
            more.push(detail_line(
                &app.i18n.t("detail_rule"),
                preview_rule_text(item, |rule| &rule.label, &item.rule_id),
                app.theme.fg_dim,
                app.theme,
            ));
            more.push(detail_line(
                &app.i18n.t("detail_id"),
                item.rule_id.clone(),
                app.theme.fg_dim,
                app.theme,
            ));
        } else if let Some(entry) = app.entries.get(row.source_index) {
            lines.push(Line::from(Span::styled(
                app.i18n.t("scan_read_only"),
                Style::default().fg(app.theme.warn),
            )));
            detail_section(
                &mut lines,
                app.i18n.t("detail_path"),
                entry.path.display().to_string(),
                app.theme.fg,
                app.theme,
            );
            let labels = entry
                .rule_hits
                .iter()
                .map(|hit| hit.label.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | ");
            more.push(detail_line(
                &app.i18n.t("detail_rule"),
                labels,
                app.theme.fg_dim,
                app.theme,
            ));
        }
        more.push(detail_line(
            &app.i18n.t("home_detail_scope"),
            join_paths(&app.roots),
            app.theme.fg_dim,
            app.theme,
        ));
    } else {
        lines.push(Line::from(scan_empty_text(app)));
    }
    render_details(frame, area, app, lines, more);
}

pub(crate) fn scan_empty_text(app: &Workbench) -> String {
    if app.scan_projection_pending() {
        return app.i18n.t("scan_filter_processing");
    }
    if app.scan_total_count() > 0 {
        return app.i18n.t("scan_query_empty");
    }
    if app.scan_is_budget_limited() || (app.plan.is_none() && !app.entries.is_empty()) {
        return app.i18n.t("scan_read_only");
    }
    if let Some(analysis) = &app.analysis {
        if app.scan_view.age_excluded_candidates {
            return app.i18n.format(
                "scan_age_empty",
                &[("days", analysis.policy.preselect_after_days.to_string())],
            );
        }
        return app.i18n.t("scan_candidates_empty");
    }
    app.i18n.t("plan_empty_hint")
}

pub(crate) fn render_operation_progress(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let content = Rect::new(area.x, area.y, area.width.min(100), area.height);
    let (phase, completed, total, path) = if let Some(progress) = &app.operation_progress {
        let key = match progress.phase {
            cleanr_tasks::OperationPhase::Validating => "operation_validating",
            cleanr_tasks::OperationPhase::Trashing => "operation_trashing",
            cleanr_tasks::OperationPhase::Restoring => "operation_restoring",
        };
        (
            key,
            progress.completed,
            progress.total,
            progress.current_path.as_ref(),
        )
    } else {
        (
            "operation_validating",
            0,
            app.plan.as_ref().map_or(0, |p| p.summary.selected_count),
            None,
        )
    };
    let mut lines = vec![
        home_title(app.i18n.t(phase), app.theme),
        Line::from(app.i18n.format(
            "operation_count",
            &[
                ("done", completed.to_string()),
                ("total", total.to_string()),
            ],
        )),
        Line::from(""),
    ];
    if let Some(path) = path {
        lines.push(Line::from(compact_path_for_width(
            path,
            &app.roots,
            content.width as usize,
        )));
    }
    lines.push(Line::from(app.i18n.t("operation_selection_frozen")));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), content);
}

pub(crate) fn render_scan_sort(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    frame.render_widget(Clear, area);
    let items = crate::projection::ScanSort::ALL
        .iter()
        .map(|sort| ListItem::new(app.i18n.t(sort.label_key())))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(popup_block(app.i18n.t("scan_sort_title"), app.theme))
        .highlight_symbol("› ")
        .highlight_style(
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, area, &mut app.scan_view.sort_state);
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
    let block = popup_block(app.i18n.t("scan_filter_title"), app.theme);
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
    let count_width = app.scan_total_count().to_string().len().max(4) as u16;
    let size_width = display_width(&format_bytes(app.scan_view.total_size_bytes)).max(10) as u16;
    let name_width = rows[0].width.saturating_sub(count_width + size_width + 6);
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
            Row::new(vec![
                text_cell(if active { "•" } else { " " }, 1, app.theme.accent),
                text_cell(label, name_width, app.theme.fg),
                right_cell(count.to_string(), app.theme.fg_dim),
                right_cell(format_bytes(bytes), app.theme.fg_dim),
            ])
        })
        .collect::<Vec<_>>();
    let local_state = local_list_state(&app.scan_view.filter_state, &window);
    let mut table_state = TableState::default().with_selected(local_state.selected());
    frame.render_stateful_widget(
        Table::new(
            items,
            [
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(count_width),
                Constraint::Length(size_width),
            ],
        )
        .column_spacing(1)
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_symbol("› ")
        .row_highlight_style(
            Style::default()
                .fg(app.theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
        &mut table_state,
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
