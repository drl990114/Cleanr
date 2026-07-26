use super::*;

pub(crate) fn render(frame: &mut Frame<'_>, app: &mut Workbench) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.bg)),
        area,
    );

    let header_height = area.height.min(2);
    let status_height = u16::from(area.height >= 3);
    let command_height = area
        .height
        .saturating_sub(header_height + status_height)
        .min(3);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Fill(1),
            Constraint::Length(command_height),
            Constraint::Length(status_height),
        ])
        .split(area);

    render_header(frame, layout[0], app);
    render_body(frame, layout[1], app);
    render_command(frame, layout[2], app);
    render_status(frame, layout[3], app);

    if app.palette_open {
        let commands = app.filtered_palette_commands().len().min(8);
        let popup = bottom_bounded_rect(
            layout[1],
            layout[1].width.saturating_sub(4),
            (commands as u16).saturating_add(2),
            112,
        );
        render_palette(frame, popup, app);
    }
    if app.help_open {
        render_help(frame, centered_bounded_rect(area, 72, 16, 88), app);
    }
    if app.confirmation_pending() {
        render_confirm(frame, centered_bounded_rect(area, 68, 9, 84), app);
    }
    if matches!(app.mode, Mode::Normal) {
        render_ime_guard(frame, area, app);
    }
}

pub(crate) fn render_header(frame: &mut Frame<'_>, area: Rect, app: &Workbench) {
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.surface)),
        area,
    );

    let status = if app.operation_kind.is_some() {
        format!("{} {}", spinner_frame(app.animation_tick), app.status)
    } else {
        app.status.clone()
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if rows[0].width >= 72 {
            [Constraint::Percentage(46), Constraint::Percentage(54)]
        } else {
            [Constraint::Percentage(68), Constraint::Percentage(32)]
        })
        .split(rows[0]);
    let roots_label = app.i18n.t("label_roots");
    let roots_budget = (top[1].width as usize)
        .saturating_sub(display_width(&roots_label))
        .saturating_sub(4);
    let roots = truncate_text(&join_paths(&app.roots), roots_budget);
    let brand = Line::from(vec![
        Span::styled(
            "  cleanr",
            Style::default()
                .fg(app.theme.magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(app.theme.fg_dim),
        ),
        Span::styled("  /  ", Style::default().fg(app.theme.border)),
        Span::styled(view_title(app), Style::default().fg(app.theme.fg)),
    ]);
    frame.render_widget(Paragraph::new(brand), top[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{roots_label}  "),
                Style::default().fg(app.theme.fg_dim),
            ),
            Span::styled(roots, Style::default().fg(app.theme.fg_dim)),
            Span::raw("  "),
        ]))
        .alignment(ratatui::layout::Alignment::Right),
        top[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  •  ", Style::default().fg(app.theme.accent)),
            Span::styled(
                status,
                Style::default().fg(if app.has_background_task() {
                    app.theme.fg
                } else {
                    app.theme.fg_dim
                }),
            ),
        ])),
        rows[1],
    );
}

pub(crate) fn render_body(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    app.viewport_height = area.height.max(1);
    match app.view {
        View::Home => render_home(frame, area, app),
        View::Scan => render_scan_workspace(frame, area, app),
        View::Languages => render_languages(frame, area, app),
        View::Rules => render_rules(frame, area, app),
        View::Plugins => render_plugins(frame, area, app),
        View::Tasks => render_tasks(frame, area, app),
        View::Usage => render_usage(frame, area, app),
        View::Restore => render_restore(frame, area, app),
    }
}
