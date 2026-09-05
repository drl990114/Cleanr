use super::*;

pub(crate) fn render_command(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let content_area = fluid_content_rect(area, 220, area.height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::horizontal(1));
    let block = if app.scan_view.search_open {
        block.title(app.i18n.t("scan_search_title"))
    } else {
        block
    };
    let inner = block.inner(content_area);
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

    frame.render_widget(Paragraph::new(content).block(block), content_area);

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
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.surface)),
        area,
    );
    let list_len = app.list_len();
    let mut right = Vec::new();
    if let Some(plan) = &app.plan
        && !app.is_scan_running()
        && app.view == View::Home
    {
        right.extend([
            Span::styled(
                plan.summary.selected_count.to_string(),
                Style::default()
                    .fg(app.theme.ok)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                app.i18n.format(
                    "selection_progress_total",
                    &[("total", plan.summary.candidate_count.to_string())],
                ),
                Style::default().fg(app.theme.fg_dim),
            ),
        ]);
    } else if list_len > 0 && !app.is_scan_running() {
        let current = app.list_state.selected().map_or(0, |index| index + 1);
        right.push(Span::styled(
            format!("{current} / {list_len} "),
            Style::default().fg(app.theme.fg_dim),
        ));
    }

    let right_width = spans_width(&right).min(area.width as usize);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(u16::try_from(right_width).unwrap_or(area.width)),
        ])
        .split(area);
    let hint_budget = chunks[0].width as usize;
    let mut hints = Vec::new();
    match app.mode {
        Mode::Command => {
            hints.extend([
                Span::styled(
                    format!("  {}", app.i18n.t("label_mode_command")),
                    Style::default()
                        .fg(app.theme.magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ·  ", Style::default().fg(app.theme.border)),
            ]);
            push_hint_if_fits(
                &mut hints,
                key_hint("↑↓", app.i18n.t("hint_choose"), app.theme),
                hint_budget,
            );
            push_hint_if_fits(
                &mut hints,
                key_hint("enter", app.i18n.t("hint_run"), app.theme),
                hint_budget,
            );
            push_hint_if_fits(
                &mut hints,
                key_hint("esc", app.i18n.t("hint_close"), app.theme),
                hint_budget,
            );
        }
        Mode::Normal => {
            if app.is_scan_running() {
                push_hint_if_fits(
                    &mut hints,
                    key_hint("esc/x", app.i18n.t("hint_cancel"), app.theme),
                    hint_budget,
                );
            } else if app.view == View::Home {
                push_hint_if_fits(
                    &mut hints,
                    key_hint("/", app.i18n.t("hint_commands"), app.theme),
                    hint_budget,
                );
                push_hint_if_fits(
                    &mut hints,
                    key_hint("?", app.i18n.t("hint_help"), app.theme),
                    hint_budget,
                );
                push_hint_if_fits(
                    &mut hints,
                    key_hint("q", app.i18n.t("hint_quit"), app.theme),
                    hint_budget,
                );
            } else if app.view == View::Scan {
                let hints_for_scan = if app.scan_view.details_focused {
                    vec![("↑↓", "hint_move"), ("Tab/Esc", "hint_close")]
                } else {
                    let mut keys = Vec::new();
                    if app.plan.is_some() && !app.has_background_task() {
                        if list_len > 0 {
                            keys.push(("space", "hint_select"));
                        }
                        keys.push(("c", "hint_clean"));
                    }
                    keys.extend([
                        ("Tab", "label_details"),
                        ("?", "hint_help"),
                        ("p", "hint_find_path"),
                        ("f", "hint_filter"),
                        ("o", "hint_sort"),
                        ("v", "scan_selected_only"),
                    ]);
                    if app.plan.is_some() && !app.has_background_task() {
                        keys.extend([("a", "hint_all_filtered"), ("A", "hint_all_global")]);
                    }
                    keys
                };
                for (key, label) in hints_for_scan {
                    push_hint_if_fits(
                        &mut hints,
                        key_hint(key, app.i18n.t(label), app.theme),
                        hint_budget,
                    );
                }
            } else if list_len > 0 {
                push_hint_if_fits(
                    &mut hints,
                    key_hint("j/k", app.i18n.t("hint_move"), app.theme),
                    hint_budget,
                );
                if matches!(app.view, View::Languages | View::Restore) {
                    push_hint_if_fits(
                        &mut hints,
                        key_hint("enter", app.i18n.t("hint_select"), app.theme),
                        hint_budget,
                    );
                }
                push_hint_if_fits(
                    &mut hints,
                    key_hint("/", app.i18n.t("hint_commands"), app.theme),
                    hint_budget,
                );
            } else {
                push_hint_if_fits(
                    &mut hints,
                    key_hint("/", app.i18n.t("hint_commands"), app.theme),
                    hint_budget,
                );
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(hints)), chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(ratatui::layout::Alignment::Right),
        chunks[1],
    );
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
    let available_width = (area.width as usize).saturating_sub(6);
    let command_width = commands
        .iter()
        .map(|command| display_width(command.name))
        .max()
        .unwrap_or(0)
        .min(28)
        .min(available_width);
    let description_width = available_width.saturating_sub(command_width.saturating_add(2));
    let items = if commands.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            app.i18n.t("palette_no_matches"),
            Style::default().fg(app.theme.fg_dim),
        )))]
    } else {
        commands
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

                let command_padding = " ".repeat(
                    command_width
                        .saturating_add(2)
                        .saturating_sub(display_width(&command_name)),
                );
                let mut spans = vec![
                    Span::styled(command_name.clone(), Style::default().fg(app.theme.accent)),
                    Span::raw(command_padding.clone()),
                    Span::styled(description.clone(), Style::default().fg(app.theme.fg_dim)),
                ];

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
                                    .fg(app.theme.warn)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(after.to_string()),
                            Span::raw(command_padding),
                            Span::styled(
                                description.clone(),
                                Style::default().fg(app.theme.fg_dim),
                            ),
                        ];
                    }
                }

                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(app.theme.border))
                .padding(Padding::horizontal(1))
                .style(Style::default().bg(app.theme.surface))
                .title(format!(" {} ", app.i18n.t("label_slash_commands")))
                .title_style(
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    frame.render_stateful_widget(list, area, &mut app.palette_state);
}

pub(crate) fn render_help(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(vec![Span::styled(
            app.i18n.t("help_title"),
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
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(app.theme.border))
            .padding(Padding::horizontal(2))
            .style(Style::default().bg(app.theme.surface))
            .title(format!(" {} ", app.i18n.t("label_help")))
            .title_style(
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    app.help_max_scroll = u16::try_from(
        paragraph
            .line_count(area.width)
            .saturating_sub(area.height as usize),
    )
    .unwrap_or(u16::MAX);
    app.help_scroll = app.help_scroll.min(app.help_max_scroll);
    frame.render_widget(paragraph.scroll((app.help_scroll, 0)), area);
}

pub(crate) fn render_confirm(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    frame.render_widget(Clear, area);
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(app.theme.surface))
        .title(format!(" {title} "))
        .title_style(
            Style::default()
                .fg(action_color)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let hint_height = if inner.width < 54 { 2 } else { 1 };
    // Reserve buttons independently so wrapped scope information never pushes them off screen.
    let rows = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(hint_height),
    ])
    .split(inner);
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
        .alignment(ratatui::layout::Alignment::Center);
    app.confirm_content_visible = rows[0].width >= 20
        && rows[1].height == 1
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
            .bg(selected_color)
            .fg(theme.highlight_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };
    let shortcut = if selected {
        format!("[{shortcut}]")
    } else {
        format!("({shortcut})")
    };
    Span::styled(format!("  {shortcut} {label}  "), style)
}

pub(crate) fn render_ime_guard(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    if area.is_empty() {
        return;
    }
    let position = ime_guard_position(area);
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
