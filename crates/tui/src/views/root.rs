use super::*;

pub(crate) fn render(frame: &mut Frame<'_>, app: &mut Workbench) {
    if app.view == View::Scan || app.confirmation_pending() {
        app.ensure_scan_view_projection();
    }
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.bg)),
        area,
    );

    let header_height = area.height.min(1);
    let status_height = u16::from(area.height >= 3);
    let command_height = if matches!(app.mode, Mode::Command) {
        area.height
            .saturating_sub(header_height + status_height)
            .min(3)
    } else {
        0
    };
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
    if command_height > 0 {
        render_command(frame, layout[2], app);
    }
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
        render_help(frame, centered_bounded_rect(area, 72, 18, 88), app);
    }
    if app.scan_view.filter_open {
        let height = u16::try_from(app.scan_view.groups.len())
            .unwrap_or(u16::MAX)
            .saturating_add(5)
            .min(18);
        render_category_filter(frame, centered_bounded_rect(area, 64, height, 76), app);
    }
    if app.confirmation_pending() {
        render_confirm(frame, centered_bounded_rect(area, 68, 11, 84), app);
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
    } else if app.is_scan_running() {
        if app.scan_stall_reported_seconds.is_some() {
            app.status.clone()
        } else {
            String::new()
        }
    } else if app.last_cleanup_result.is_some() {
        String::new()
    } else {
        app.status.clone()
    };
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if area.width >= 72 {
            [Constraint::Percentage(40), Constraint::Percentage(60)]
        } else {
            [Constraint::Percentage(62), Constraint::Percentage(38)]
        })
        .split(area);
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
    let status_budget = top[1].width.saturating_sub(2) as usize;
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_text(&status, status_budget),
            Style::default().fg(if app.has_background_task() {
                app.theme.fg
            } else {
                app.theme.fg_dim
            }),
        ))
        .alignment(ratatui::layout::Alignment::Right),
        top[1],
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
