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
    let head_width = budget / 2;
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

pub(crate) fn kind_icon(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "▣ ",
        EntryKind::Symlink => "↗ ",
        EntryKind::File => "· ",
        EntryKind::Other => "? ",
    }
}

pub(crate) fn confidence_color(confidence: cleanr_core::Confidence, theme: Theme) -> Color {
    match confidence {
        cleanr_core::Confidence::High => theme.ok,
        cleanr_core::Confidence::Medium => theme.warn,
        cleanr_core::Confidence::Low => theme.danger,
    }
}

pub(crate) fn metric_span(label: String, value: String, value_color: Color) -> Span<'static> {
    Span::styled(
        format!("{label} {value}"),
        Style::default()
            .fg(value_color)
            .add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn responsive_workspace(area: Rect, list_percent: u16) -> [Rect; 2] {
    let chunks = if area.width >= 88 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(list_percent),
                Constraint::Percentage(100u16.saturating_sub(list_percent)),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(area)
    };
    [chunks[0], chunks[1]]
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
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

pub(crate) fn fluid_content_rect(area: Rect, max_width: u16, desired_height: u16) -> Rect {
    let side_margin: u16 = match area.width {
        0..=95 => 0,
        96..=159 => 2,
        _ => 4,
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
    Position::new(
        area.right().saturating_sub(2),
        area.bottom().saturating_sub(2),
    )
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
