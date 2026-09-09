use super::*;

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------
pub(crate) fn detail_line(
    label: &str,
    value: String,
    value_color: Color,
    theme: Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(theme.fg_dim)),
        Span::styled(value, Style::default().fg(value_color)),
    ])
}

pub(crate) fn view_title(app: &Workbench) -> String {
    let key = match app.view {
        View::Home => "label_home",
        View::Scan => "label_scan_tree",
        View::Languages => "label_languages",
        View::Rules => "label_rules",
        View::Plugins => "label_plugins",
        View::Tasks => "label_tasks",
        View::Usage => "label_usage",
        View::Restore => "label_restore",
    };
    app.i18n.t(key)
}

pub(crate) fn key_hint(key: &'static str, label: String, theme: Theme) -> [Span<'static>; 2] {
    [
        Span::styled(
            key,
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}   "), Style::default().fg(theme.fg_dim)),
    ]
}

pub(crate) fn compact_path(path: &std::path::Path, roots: &[PathBuf]) -> String {
    roots
        .iter()
        .find_map(|root| path.strip_prefix(root).ok())
        .filter(|relative| !relative.as_os_str().is_empty())
        .map_or_else(
            || path.display().to_string(),
            |relative| relative.display().to_string(),
        )
}

pub(crate) fn compact_path_for_width(
    path: &std::path::Path,
    roots: &[PathBuf],
    max_width: usize,
) -> String {
    truncate_text(&compact_path(path, roots), max_width)
}

pub(crate) fn truncate_text(text: &str, max_width: usize) -> String {
    let current_width = display_width(text);
    if current_width <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }

    let marker = "…";
    let marker_width = display_width(marker);
    if max_width <= marker_width {
        return marker.to_string();
    }

    let budget = max_width.saturating_sub(marker_width);
    let head_width = budget.div_ceil(2);
    let tail_width = budget.saturating_sub(head_width);
    format!(
        "{}{marker}{}",
        take_width_from_start(text, head_width),
        take_width_from_end(text, tail_width)
    )
}

pub(crate) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn take_width_from_start(text: &str, max_width: usize) -> String {
    text.unicode_truncate(max_width).0.to_string()
}

fn take_width_from_end(text: &str, max_width: usize) -> String {
    text.unicode_truncate_start(max_width).0.to_string()
}

pub(crate) fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "directory",
        EntryKind::File => "file",
        EntryKind::Symlink => "symlink",
        EntryKind::Other => "other",
    }
}

pub(crate) fn language_source_label(source: &LanguagePackSource) -> &'static str {
    match source {
        LanguagePackSource::Builtin => "builtin",
        LanguagePackSource::UserFile(_) => "user",
        LanguagePackSource::Plugin { .. } => "plugin",
    }
}

pub(crate) fn join_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "-".to_string();
    }
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The breakpoint is measured once against the terminal, never against an already inset pane.
pub(crate) fn responsive_workspace(area: Rect, wide: bool) -> [Rect; 2] {
    if !wide {
        return [area, area];
    }
    let detail_width = ((u32::from(area.width) * 38 / 100) as u16)
        .clamp(32, 56)
        .min(area.width);
    let columns = Layout::horizontal([Constraint::Fill(1), Constraint::Length(detail_width)])
        .spacing(1)
        .split(area);
    [columns[0], columns[1]]
}

pub(crate) fn panel_block(title: String, theme: Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border))
        .title(format!("{title} "))
        .title_style(Style::default().fg(theme.fg).add_modifier(Modifier::BOLD))
        // Keep a stable gutter even when the scrollbar disappears.
        .padding(Padding::new(0, 1, 0, 0))
}

pub(crate) fn popup_block(title: String, theme: Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .title(format!(" {title} "))
        .title_style(Style::default().fg(theme.fg).add_modifier(Modifier::BOLD))
}

/// Render only the visible slice; application navigation keeps its existing absolute ListState.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_table_list<F>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut Workbench,
    title: String,
    item_count: usize,
    constraints: &[Constraint],
    empty_message: String,
    rows_for_window: F,
) where
    F: FnOnce(&Workbench, Range<usize>, &[u16]) -> Vec<Row<'static>>,
{
    let block = panel_block(title, app.theme);
    let inner = block.inner(area);
    let viewport_height = usize::from(inner.height.max(1));
    app.viewport_height = inner.height.max(1);
    let window = visible_list_window(&mut app.list_state, item_count, viewport_height);
    let widths = Layout::horizontal(constraints.iter().copied())
        .spacing(1)
        .split(Rect::new(0, 0, inner.width.saturating_sub(2), 1))
        .iter()
        .map(|column| column.width)
        .collect::<Vec<_>>();
    if item_count == 0 {
        frame.render_widget(
            Paragraph::new(empty_message)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(app.theme.fg_dim))
                .block(block),
            area,
        );
        return;
    }
    let rows = rows_for_window(app, window.clone(), &widths);
    let local = local_list_state(&app.list_state, &window);
    let mut state = TableState::default().with_selected(local.selected());
    let table = Table::new(rows, widths.iter().copied().map(Constraint::Length))
        .block(block)
        .column_spacing(1)
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_symbol("› ")
        .row_highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(table, area, &mut state);
    render_list_scrollbar(
        frame,
        area,
        item_count,
        viewport_height,
        app.list_state.selected().unwrap_or(window.start),
        app.theme,
    );
}

pub(crate) fn text_cell(text: impl AsRef<str>, width: u16, color: Color) -> Cell<'static> {
    Cell::from(truncate_text(text.as_ref(), width as usize)).style(Style::default().fg(color))
}

pub(crate) fn right_cell(text: impl Into<String>, color: Color) -> Cell<'static> {
    Cell::from(Line::from(text.into()).alignment(ratatui::layout::Alignment::Right))
        .style(Style::default().fg(color))
}

pub(crate) fn detail_section(
    lines: &mut Vec<Line<'static>>,
    label: String,
    value: String,
    color: Color,
    theme: Theme,
) {
    if value.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        label,
        Style::default().fg(theme.fg_dim),
    )));
    lines.push(Line::from(Span::styled(value, Style::default().fg(color))));
}

pub(crate) fn render_details(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut Workbench,
    mut lines: Vec<Line<'static>>,
    more: Vec<Line<'static>>,
) {
    let wide = frame.area().width >= 88;
    if !wide && !app.details.focused {
        return;
    }
    let area = if wide {
        area
    } else {
        // Cover the full terminal width so underlying CJK continuation cells cannot damage the border.
        Rect::new(frame.area().x, area.y, frame.area().width, area.height)
    };
    if !wide {
        frame.render_widget(Clear, area);
    }
    let title = format!("{} [Tab]", app.i18n.t("label_details"));
    let block = if wide {
        Block::default()
            .borders(Borders::TOP | Borders::LEFT)
            .padding(Padding::horizontal(1))
            .title(format!(" {title} "))
    } else {
        popup_block(title, app.theme)
    }
    .style(Style::default().bg(app.theme.bg).fg(app.theme.fg))
    .border_style(Style::default().fg(if app.details.focused {
        app.theme.accent
    } else {
        app.theme.border
    }))
    .title_style(
        Style::default()
            .fg(if app.details.focused {
                app.theme.accent
            } else {
                app.theme.fg
            })
            .add_modifier(Modifier::BOLD),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if !more.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} [i]",
                if app.details.expanded { "▾" } else { "▸" },
                app.i18n.t("detail_more")
            ),
            Style::default().fg(app.theme.fg_dim),
        )));
        if app.details.expanded {
            lines.extend(more);
        }
    }
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    app.details.viewport_height = inner.height;
    app.details.max_scroll = u16::try_from(
        paragraph
            .line_count(inner.width)
            .saturating_sub(inner.height as usize),
    )
    .unwrap_or(u16::MAX);
    app.details.scroll = app.details.scroll.min(app.details.max_scroll);
    frame.render_widget(paragraph.scroll((app.details.scroll, 0)), inner);
    if app.details.max_scroll > 0 && !inner.is_empty() {
        render_scrollbar(
            frame,
            Rect::new(inner.right(), inner.y, 1, inner.height),
            usize::from(app.details.max_scroll) + usize::from(inner.height),
            usize::from(inner.height),
            usize::from(app.details.scroll),
            app.theme,
        );
    }
}

/// Keep the absolute selection visible and return only the rows needed for this frame.
pub(crate) fn visible_list_window(
    state: &mut ListState,
    content_len: usize,
    viewport_len: usize,
) -> Range<usize> {
    if content_len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return 0..0;
    }

    let viewport_len = viewport_len.max(1).min(content_len);
    let selected = state.selected().map(|index| index.min(content_len - 1));
    if selected != state.selected() {
        state.select(selected);
    }

    let max_start = content_len.saturating_sub(viewport_len);
    let mut start = state.offset().min(max_start);
    if let Some(selected) = selected {
        if selected < start {
            start = selected;
        } else if selected >= start.saturating_add(viewport_len) {
            start = selected.saturating_add(1).saturating_sub(viewport_len);
        }
    }
    start = start.min(max_start);
    *state.offset_mut() = start;

    start..start.saturating_add(viewport_len).min(content_len)
}

pub(crate) fn local_list_state(state: &ListState, window: &Range<usize>) -> ListState {
    let selected = state.selected().and_then(|selected| {
        selected
            .checked_sub(window.start)
            .filter(|local| *local < window.len())
    });
    ListState::default().with_selected(selected)
}

pub(crate) fn render_list_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    content_len: usize,
    viewport_len: usize,
    position: usize,
    theme: Theme,
) {
    if content_len <= viewport_len || area.width == 0 || area.height <= 1 {
        return;
    }

    let scrollbar_area = Rect::new(
        area.right().saturating_sub(1),
        area.y.saturating_add(1),
        1,
        area.height.saturating_sub(1),
    );
    render_scrollbar(
        frame,
        scrollbar_area,
        content_len,
        viewport_len,
        position,
        theme,
    );
}

fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    content_len: usize,
    viewport_len: usize,
    position: usize,
    theme: Theme,
) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some("│"))
        .thumb_symbol("┃")
        .track_style(Style::default().fg(theme.border))
        .thumb_style(Style::default().fg(theme.accent));
    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(viewport_len);
    frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
}

pub(crate) fn fluid_content_rect(area: Rect, max_width: u16, desired_height: u16) -> Rect {
    let side_margin: u16 = match area.width {
        0..=31 => 0,
        32..=63 => 1,
        _ => 2,
    };
    let available_width = area.width.saturating_sub(side_margin.saturating_mul(2));
    let width = available_width.min(max_width);
    let height = area.height.min(desired_height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y,
        width,
        height,
    )
}

pub(crate) fn ime_guard_position(area: Rect) -> Position {
    Position::new(area.right().saturating_sub(1).max(area.x), area.y)
}

#[cfg(test)]
pub(crate) fn command_cursor_position(area: Rect, input: &str) -> Option<Position> {
    command_cursor_position_at(area, input, input.len())
}

#[cfg(test)]
pub(crate) fn command_cursor_position_at(
    area: Rect,
    input: &str,
    cursor: usize,
) -> Option<Position> {
    if area.is_empty() {
        return None;
    }
    let prefix_width = 3usize;
    let prefix_bytes = input
        .chars()
        .next()
        .map_or(0, char::len_utf8)
        .min(input.len());
    let mut cursor = cursor.min(input.len());
    while cursor > prefix_bytes && !input.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    let rest_width = display_width(&input[prefix_bytes..cursor]);
    let offset = u16::try_from(prefix_width.saturating_add(rest_width)).unwrap_or(u16::MAX);
    Some(Position::new(
        area.x
            .saturating_add(offset)
            .min(area.right().saturating_sub(1)),
        area.y,
    ))
}

/// Return a single-line viewport that keeps the command cursor visible without splitting a
/// grapheme cluster. The cursor column is relative to the returned text.
pub(crate) fn command_input_view(input: &str, cursor: usize, max_width: usize) -> (String, usize) {
    if max_width == 0 {
        return (String::new(), 0);
    }
    let prefix_bytes = input
        .chars()
        .next()
        .map_or(0, char::len_utf8)
        .min(input.len());
    let mut cursor = cursor.clamp(prefix_bytes, input.len());
    while cursor > prefix_bytes && !input.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }

    let before = &input[prefix_bytes..cursor];
    let after = &input[cursor..];
    let before_width = display_width(before);
    let after_width = display_width(after);
    if before_width.saturating_add(after_width) <= max_width {
        return (format!("{before}{after}"), before_width);
    }

    if before_width < max_width {
        let (visible_after, _) = after.unicode_truncate(max_width - before_width);
        return (format!("{before}{visible_after}"), before_width);
    }

    let marker = "…";
    let marker_width = display_width(marker).min(max_width);
    let after_budget = usize::from(!after.is_empty() && max_width > marker_width);
    let before_budget = max_width.saturating_sub(marker_width + after_budget);
    let (visible_before, visible_before_width) = before.unicode_truncate_start(before_budget);
    let (visible_after, _) = after.unicode_truncate(after_budget);
    (
        format!("{marker}{visible_before}{visible_after}"),
        marker_width + visible_before_width,
    )
}

pub(crate) fn centered_bounded_rect(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    max_width: u16,
) -> Rect {
    let width = area.width.min(desired_width.max(24).min(max_width));
    let height = area.height.min(desired_height.max(3));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

pub(crate) fn bottom_bounded_rect(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    max_width: u16,
) -> Rect {
    let width = area.width.min(desired_width.max(24).min(max_width));
    let height = area.height.min(desired_height.max(3));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.bottom().saturating_sub(height),
        width,
        height,
    )
}

pub(crate) fn spinner_frame(tick: u64) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    FRAMES[tick as usize % FRAMES.len()]
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GB {
        format!("{:.2} GiB", bytes_f / GB)
    } else if bytes_f >= MB {
        format!("{:.2} MiB", bytes_f / MB)
    } else if bytes_f >= KB {
        format!("{:.2} KiB", bytes_f / KB)
    } else {
        format!("{bytes} B")
    }
}
