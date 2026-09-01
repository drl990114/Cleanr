use super::*;

pub(crate) fn render_command(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    let content_area = fluid_content_rect(area, 220, area.height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(app.theme.border))
        .padding(Padding::horizontal(1));
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
    if app.is_scan_running() {
        let progress = app.scan_progress.as_ref();
        let compact = progress.map_or_else(
            || app.i18n.t("scan_preparing"),
            |value| match value.stage {
                ScanStage::Scanning if value.entries_total == 0 => app.i18n.format(
                    "scan_progress_unbounded",
                    &[("scanned", value.entries_scanned.to_string())],
                ),
                ScanStage::Scanning => app.i18n.format(
                    "scan_progress_count",
                    &[
                        ("scanned", value.entries_scanned.to_string()),
                        ("total", value.entries_total.to_string()),
                    ],
                ),
                ScanStage::Aggregating => app.i18n.t("scan_progress_aggregating"),
                stage => app.scan_stage_label(stage),
            },
        );
        right.push(Span::styled(
            format!("{compact} "),
            Style::default().fg(app.theme.accent),
        ));
    } else if let Some(plan) = &app.plan {
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
    } else if list_len > 0 {
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
    let mode = match app.mode {
        Mode::Normal => app.i18n.t("label_mode_normal"),
        Mode::Command => app.i18n.t("label_mode_command"),
    };
    let mode_color = match app.mode {
        Mode::Normal => app.theme.accent,
        Mode::Command => app.theme.magenta,
    };
    let mut hints = vec![
        Span::styled(
            format!("  {mode}"),
            Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(app.theme.border)),
    ];
    match app.mode {
        Mode::Command => {
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
                    key_hint("s", app.i18n.t("hint_scan"), app.theme),
                    hint_budget,
                );
                push_hint_if_fits(
                    &mut hints,
                    key_hint("u", app.i18n.t("hint_usage"), app.theme),
                    hint_budget,
                );
            } else if app.view == View::Scan {
                if list_len > 0 {
                    push_hint_if_fits(
                        &mut hints,
                        key_hint("j/k", app.i18n.t("hint_move"), app.theme),
                        hint_budget,
                    );
                    push_hint_if_fits(
                        &mut hints,
                        key_hint("space", app.i18n.t("hint_select"), app.theme),
                        hint_budget,
                    );
                    if app.plan.is_some() {
                        push_hint_if_fits(
                            &mut hints,
                            key_hint("c", app.i18n.t("hint_clean"), app.theme),
                            hint_budget,
                        );
                        push_hint_if_fits(
                            &mut hints,
                            key_hint("a", app.i18n.t("hint_all"), app.theme),
                            hint_budget,
                        );
                    }
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
            }
            push_hint_if_fits(
                &mut hints,
                key_hint("/", app.i18n.t("hint_commands"), app.theme),
                hint_budget,
            );
            push_hint_if_fits(
                &mut hints,
                key_hint("q", app.i18n.t("hint_quit"), app.theme),
                hint_budget,
            );
            push_hint_if_fits(
                &mut hints,
                key_hint("?", app.i18n.t("hint_help"), app.theme),
                hint_budget,
            );
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

pub(crate) fn render_help(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
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
    let paragraph = Paragraph::new(lines).block(
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
    frame.render_widget(paragraph, area);
}

pub(crate) fn render_confirm(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
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

    let lines = vec![
        Line::from(""),
        Line::from(body).alignment(ratatui::layout::Alignment::Center),
        Line::from(""),
        Line::from(vec![
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
        ])
        .alignment(ratatui::layout::Alignment::Center),
        Line::from(Span::styled(
            app.i18n.t("confirm_hint"),
            Style::default().fg(app.theme.fg_dim),
        ))
        .alignment(ratatui::layout::Alignment::Center),
    ];
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(app.theme.border))
            .padding(Padding::horizontal(2))
            .style(Style::default().bg(app.theme.surface))
            .title(format!(" {title} "))
            .title_style(
                Style::default()
                    .fg(action_color)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    frame.render_widget(paragraph, area);
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
