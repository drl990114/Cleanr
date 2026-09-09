use super::*;

pub(crate) fn render_command(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let block = popup_block(String::new(), app.theme);
    let inner = block.inner(area);
    let mut cursor_column = None;
    let content = match app.mode {
        Mode::Command => {
            let prefix = app.input.chars().next().unwrap_or('>');
            let max_input_width = (inner.width as usize).saturating_sub(3);
            let (rest, column) = command_input_view(&app.input, app.input_cursor, max_input_width);
            cursor_column = Some(column);
            Line::from(vec![
                Span::styled(
                    format!(" {prefix} "),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest, Style::default().fg(app.theme.fg)),
            ])
        }
        Mode::Normal => Line::from(vec![
            Span::styled(" / ", Style::default().fg(app.theme.accent)),
            Span::styled(
                app.i18n.t("command_placeholder"),
                Style::default().fg(app.theme.fg_dim),
            ),
        ]),
    };

    frame.render_widget(Paragraph::new(content).block(block), area);

    if let Some(column) = cursor_column
        && !inner.is_empty()
    {
        let offset = u16::try_from(3usize.saturating_add(column)).unwrap_or(u16::MAX);
        frame.set_cursor_position(Position::new(
            inner
                .x
                .saturating_add(offset)
                .min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}

pub(crate) fn render_status(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let keys = if app.help_open {
        vec![("↑↓", "hint_scroll"), ("Esc", "hint_close")]
    } else if app.confirmation_pending() {
        vec![
            ("←→", "hint_choose"),
            ("Enter", "hint_apply"),
            ("Esc", "hint_cancel"),
        ]
    } else if app.scan_view.filter_open || app.scan_view.sort_open {
        vec![
            ("↑↓", "hint_choose"),
            ("Enter", "hint_apply"),
            ("Esc", "hint_close"),
        ]
    } else if app.scan_view.search_open {
        vec![("Enter", "hint_apply"), ("Esc", "hint_revert")]
    } else if matches!(app.mode, Mode::Command) {
        vec![
            ("↑↓", "hint_choose"),
            ("Enter", "hint_run"),
            ("Esc", "hint_close"),
        ]
    } else if app.is_scan_running() {
        vec![("Esc/x", "hint_cancel")]
    } else if app.is_operation_running() {
        Vec::new()
    } else if app.view == View::CleanupResult {
        let mut keys = vec![
            ("s", "home_action_rescan"),
            ("z", "hint_restore_result"),
            ("q", "hint_quit"),
        ];
        if app.details.max_scroll > 0 {
            keys.push(("↑↓", "hint_scroll"));
        }
        keys
    } else if app.details.focused {
        vec![
            ("Tab", "hint_back"),
            ("i", "detail_more"),
            ("↑↓", "hint_scroll"),
            ("?", "hint_help"),
        ]
    } else if app.view == View::Home {
        vec![
            ("/", "hint_commands"),
            ("?", "hint_help"),
            ("q", "hint_quit"),
        ]
    } else if app.view == View::Scan {
        let mut keys = Vec::new();
        if app.plan.is_some() && !app.has_background_task() {
            if app.list_len() > 0 {
                keys.push(("space", "hint_select"));
            }
            if app
                .plan
                .as_ref()
                .is_some_and(|plan| plan.summary.selected_count > 0)
            {
                keys.push(("c", "hint_clean"));
            }
        }
        keys.extend([
            ("Tab", "label_details"),
            ("p", "hint_find_path"),
            ("?", "hint_help"),
        ]);
        keys
    } else {
        let mut keys = vec![("↑↓", "hint_move")];
        if matches!(app.view, View::Languages | View::Restore) && app.list_len() > 0 {
            keys.push(("Enter", "hint_select"));
        }
        keys.extend([
            ("Tab", "label_details"),
            ("/", "hint_commands"),
            ("?", "hint_help"),
        ]);
        keys
    };
    let mut hints = Vec::new();
    let help_width = if keys.iter().any(|(key, _)| *key == "?") {
        spans_width(&key_hint("?", app.i18n.t("hint_help"), app.theme))
    } else {
        0
    };
    for (key, label) in keys {
        let budget = if key == "?" {
            area.width as usize
        } else {
            (area.width as usize).saturating_sub(help_width)
        };
        push_hint_if_fits(
            &mut hints,
            key_hint(key, app.i18n.t(label), app.theme),
            budget,
        );
    }
    frame.render_widget(Paragraph::new(Line::from(hints)), area);
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn push_hint_if_fits(hints: &mut Vec<Span<'static>>, hint: [Span<'static>; 2], max_width: usize) {
    if spans_width(hints).saturating_add(spans_width(&hint)) <= max_width {
        hints.extend(hint);
    }
}

pub(crate) fn render_palette(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    frame.render_widget(Clear, area);
    let filter = app
        .input
        .strip_prefix('/')
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();

    let commands = app.filtered_palette_commands();
    let block = popup_block(app.i18n.t("label_slash_commands"), app.theme);
    let inner = block.inner(area);
    let window = visible_list_window(
        &mut app.palette_state,
        commands.len(),
        inner.height.max(1) as usize,
    );
    if commands.is_empty() {
        frame.render_widget(
            Paragraph::new(app.i18n.t("palette_no_matches"))
                .style(Style::default().fg(app.theme.fg_dim))
                .block(block),
            area,
        );
        return;
    }
    let available_width = (inner.width as usize).saturating_sub(2);
    let command_width = commands
        .iter()
        .map(|command| display_width(command.name))
        .max()
        .unwrap_or(0)
        .min(28)
        .min(available_width);
    let description_width = available_width.saturating_sub(command_width.saturating_add(2));
    let items = commands[window.clone()]
        .iter()
        .map(|command| {
            let translated = app.i18n.t(command.description_key);
            let description = if translated == command.description_key {
                command.description.to_string()
            } else {
                translated
            };
            let description = truncate_text(&description, description_width);
            let command_name = truncate_text(command.name, command_width);

            let mut spans = vec![Span::styled(
                command_name.clone(),
                Style::default().fg(app.theme.fg),
            )];

            // Highlight matching characters in the command name.
            if !filter.is_empty() {
                let name_lower = command_name.to_lowercase();
                if let Some(start) = name_lower.find(&filter) {
                    let end = start + filter.len();
                    let before = &command_name[..start];
                    let matched = &command_name[start..end];
                    let after = &command_name[end..];
                    spans = vec![
                        Span::raw(before.to_string()),
                        Span::styled(
                            matched.to_string(),
                            Style::default()
                                .fg(app.theme.accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(after.to_string()),
                    ];
                }
            }

            Row::new(vec![
                Cell::from(Line::from(spans)),
                Cell::from(description).style(Style::default().fg(app.theme.fg_dim)),
            ])
        })
        .collect::<Vec<_>>();

    let local = local_list_state(&app.palette_state, &window);
    let mut state = TableState::default().with_selected(local.selected());
    let table = Table::new(
        items,
        [
            Constraint::Length(command_width as u16),
            Constraint::Fill(1),
        ],
    )
    .block(block)
    .column_spacing(2)
    .highlight_spacing(HighlightSpacing::Always)
    .row_highlight_style(
        Style::default()
            .fg(app.theme.highlight_fg)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("› ");

    frame.render_stateful_widget(table, area, &mut state);
}

pub(crate) fn render_help(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(vec![Span::styled(
            format!("cleanr {}", env!("CARGO_PKG_VERSION")),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(app.i18n.t("help_move")),
        Line::from(app.i18n.t("help_select_all")),
        Line::from(app.i18n.t("help_select_global")),
        Line::from(app.i18n.t("help_categories")),
        Line::from(app.i18n.t("help_query_sort")),
        Line::from(app.i18n.t("help_details")),
        Line::from(app.i18n.t("help_more")),
        Line::from(app.i18n.t("help_restore_result")),
        Line::from(app.i18n.t("help_toggle")),
        Line::from(app.i18n.t("help_actions")),
        Line::from(app.i18n.t("help_command")),
        Line::from(app.i18n.t("help_palette")),
        Line::from(app.i18n.t("help_command_edit")),
        Line::from(app.i18n.t("help_page")),
        Line::from(app.i18n.t("help_home")),
        Line::from(app.i18n.t("help_confirm_yes")),
        Line::from(app.i18n.t("help_confirm_no")),
        Line::from(app.i18n.t("help_quit")),
    ];
    let block = popup_block(app.i18n.t("label_help"), app.theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    app.help_max_scroll = u16::try_from(
        paragraph
            .line_count(inner.width)
            .saturating_sub(inner.height as usize),
    )
    .unwrap_or(u16::MAX);
    app.help_scroll = app.help_scroll.min(app.help_max_scroll);
    frame.render_widget(paragraph.scroll((app.help_scroll, 0)), inner);
}

pub(crate) fn render_confirm(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let restoring = app.restore_waiting_for_confirmation.is_some();
    let (title, body, action_color) = if restoring {
        let run_id = app
            .restore_waiting_for_confirmation
            .as_deref()
            .unwrap_or_default();
        let count = app
            .execution_manifests
            .iter()
            .find(|manifest| manifest.run_id == run_id)
            .map_or(0, |manifest| manifest.summary.succeeded);
        (
            app.i18n.t("confirm_restore_title"),
            app.i18n.format(
                "confirm_restore_body",
                &[("count", count.to_string()), ("run_id", run_id.to_string())],
            ),
            app.theme.ok,
        )
    } else {
        let (count, size) = app.plan.as_ref().map_or((0, String::from("-")), |plan| {
            (
                plan.summary.selected_count,
                format_bytes(plan.summary.selected_size_bytes),
            )
        });
        (
            app.i18n.t("confirm_title"),
            app.i18n.format(
                "confirm_body",
                &[("count", count.to_string()), ("size", size)],
            ),
            app.theme.danger,
        )
    };

    let block = popup_block(title, app.theme).title_style(
        Style::default()
            .fg(action_color)
            .add_modifier(Modifier::BOLD),
    );
    let inner_width = block.inner(area).width;
    let hint_height = if inner_width < 54 { 2 } else { 1 };
    let mut body_lines = vec![Line::from(body)];
    if !restoring {
        if app.scan_view.selected_review_count > 0 {
            body_lines.push(Line::from(app.i18n.format(
                "confirm_review_count",
                &[("count", app.scan_view.selected_review_count.to_string())],
            )));
        }
        body_lines.push(Line::from(app.i18n.t("confirm_review_selected")));
    }
    if !restoring && app.scan_view.hidden_selected_count > 0 {
        body_lines.push(Line::from(Span::styled(
            app.i18n.format(
                "confirm_hidden_selection",
                &[
                    ("count", app.scan_view.hidden_selected_count.to_string()),
                    ("size", format_bytes(app.scan_view.hidden_selected_bytes)),
                ],
            ),
            Style::default().fg(app.theme.warn),
        )));
    }
    let body_paragraph = Paragraph::new(body_lines)
        .wrap(Wrap { trim: true })
        .alignment(ratatui::layout::Alignment::Left);
    let desired_height = body_paragraph
        .line_count(inner_width)
        .saturating_add(4 + hint_height as usize)
        .min(u16::MAX as usize) as u16;
    let area = centered_bounded_rect(area, area.width, desired_height, area.width);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    // Reserve buttons independently so wrapped scope information never pushes them off screen.
    let rows = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(hint_height),
    ])
    .split(inner);
    let buttons = Line::from(vec![
        confirm_button(
            "Y",
            app.i18n.t("confirm_yes"),
            app.confirm_choice == ConfirmChoice::Yes,
            action_color,
            app.theme,
        ),
        Span::raw("   "),
        confirm_button(
            "N",
            app.i18n.t("confirm_no"),
            app.confirm_choice == ConfirmChoice::No,
            app.theme.accent,
            app.theme,
        ),
    ]);
    app.confirm_content_visible = rows[0].width >= 20
        && rows[1].height == 1
        && buttons.width() <= rows[1].width as usize
        && body_paragraph.line_count(rows[0].width) <= rows[0].height as usize;
    if app.confirm_content_visible {
        frame.render_widget(body_paragraph, rows[0]);
    } else {
        frame.render_widget(
            Paragraph::new(app.i18n.t("confirm_resize"))
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(app.theme.warn)),
            rows[0],
        );
    }
    frame.render_widget(
        Paragraph::new(buttons).alignment(ratatui::layout::Alignment::Center),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(app.i18n.t("confirm_hint"))
            .style(Style::default().fg(app.theme.fg_dim))
            .wrap(Wrap { trim: true })
            .alignment(ratatui::layout::Alignment::Center),
        rows[2],
    );
}

pub(crate) fn confirm_button(
    shortcut: &'static str,
    label: String,
    selected: bool,
    selected_color: Color,
    theme: Theme,
) -> Span<'static> {
    let style = if selected {
        Style::default()
            .fg(selected_color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };
    let marker = if selected { "›" } else { " " };
    Span::styled(format!("{marker} [{shortcut}] {label}"), style)
}

pub(crate) fn render_ime_guard(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    if area.is_empty() {
        return;
    }
    let position = ime_guard_position(area);
    if frame.buffer_mut()[(position.x, position.y)].symbol() != " " {
        return;
    }
    let style = if app.ime_guard_phase {
        Style::default().bg(app.theme.bg)
    } else {
        Style::default()
            .bg(app.theme.bg)
            .add_modifier(Modifier::DIM)
    };
    frame.render_widget(
        Paragraph::new(" ").style(style),
        Rect::new(position.x, position.y, 1, 1),
    );
}
