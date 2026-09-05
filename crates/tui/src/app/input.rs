use std::hash::{DefaultHasher, Hash, Hasher};
use unicode_segmentation::UnicodeSegmentation;

use super::*;

impl Workbench {
    pub fn handle_key(&mut self, key: KeyEvent) {
        self.handle_key_changed(key);
    }

    pub(crate) fn ui_stamp(&self) -> u64 {
        let mut stamp = DefaultHasher::new();
        self.view.hash(&mut stamp);
        matches!(self.mode, Mode::Command).hash(&mut stamp);
        self.input.hash(&mut stamp);
        self.input_cursor.hash(&mut stamp);
        self.status.hash(&mut stamp);
        self.i18n.locale().hash(&mut stamp);
        self.list_state.selected().hash(&mut stamp);
        self.list_state.offset().hash(&mut stamp);
        self.palette_state.selected().hash(&mut stamp);
        self.palette_open.hash(&mut stamp);
        self.help_open.hash(&mut stamp);
        self.help_scroll.hash(&mut stamp);
        self.scan_view.filter.hash(&mut stamp);
        self.scan_view.filter_open.hash(&mut stamp);
        self.scan_view.filter_state.selected().hash(&mut stamp);
        self.scan_view.query.hash(&mut stamp);
        self.scan_view.search_open.hash(&mut stamp);
        self.scan_view.sort.hash(&mut stamp);
        self.scan_view.sort_open.hash(&mut stamp);
        self.scan_view.sort_state.selected().hash(&mut stamp);
        self.scan_view.only_selected.hash(&mut stamp);
        self.scan_view.details_focused.hash(&mut stamp);
        self.scan_view.details_scroll.hash(&mut stamp);
        self.has_background_task().hash(&mut stamp);
        self.confirmation_pending().hash(&mut stamp);
        (self.confirm_choice as u8).hash(&mut stamp);
        self.scan_cancel_requested.hash(&mut stamp);
        self.should_quit.hash(&mut stamp);
        self.count_buffer.hash(&mut stamp);
        self.pending_key.hash(&mut stamp);
        self.scan_data_revision.hash(&mut stamp);
        if let Some(plan) = &self.plan {
            plan.summary.selected_count.hash(&mut stamp);
            plan.summary.selected_size_bytes.hash(&mut stamp);
        }
        stamp.finish()
    }

    pub(crate) fn handle_key_changed(&mut self, key: KeyEvent) -> bool {
        let before = self.ui_stamp();
        let focus_before = self.list_state.selected();
        self.handle_key_inner(key);
        if focus_before != self.list_state.selected() && !self.scan_view.details_focused {
            self.scan_view.details_scroll = 0;
        }
        let ime_input = key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Char(ch) if !ch.is_ascii());
        let changed = self.ui_stamp() != before || ime_input;
        if changed {
            self.ime_guard_phase = !self.ime_guard_phase;
        }
        changed
    }

    pub(crate) fn can_batch_navigation(&self, key: KeyEvent) -> bool {
        matches!(self.mode, Mode::Normal)
            && !self.help_open
            && !self.confirmation_pending()
            && !self.scan_view.filter_open
            && !self.scan_view.sort_open
            && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
            && matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageDown
                    | KeyCode::PageUp
                    | KeyCode::Char('j' | 'k')
            )
    }

    fn handle_key_inner(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press
            && (key.kind != KeyEventKind::Repeat || !self.is_repeatable_key(key))
        {
            return;
        }

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.scan_view.search_open {
                self.close_scan_search(false);
            } else if self.scan_view.sort_open {
                self.scan_view.sort_open = false;
            } else if self.scan_view.filter_open {
                self.scan_view.filter_open = false;
                self.clear_pending();
            } else if matches!(self.mode, Mode::Command) {
                self.close_command();
            } else if self.confirmation_pending() {
                self.cancel_confirmation();
            } else if self.is_operation_running() {
                self.status = self.i18n.t("status_operation_running");
            } else {
                self.should_quit = true;
                self.clear_pending();
            }
            return;
        }

        if self.help_open {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?' | 'h' | 'H')) {
                self.help_open = false;
            }
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = self.help_scroll.saturating_add(1).min(self.help_max_scroll)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => {
                    self.help_scroll = self.help_scroll.saturating_add(8).min(self.help_max_scroll)
                }
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(8),
                KeyCode::Home => self.help_scroll = 0,
                KeyCode::End => self.help_scroll = self.help_max_scroll,
                _ => {}
            }
            return;
        }

        if self.confirmation_pending() {
            match key.code {
                KeyCode::Left | KeyCode::Char('h' | 'H' | 'y' | 'Y') => {
                    self.confirm_choice = ConfirmChoice::Yes;
                }
                KeyCode::Right | KeyCode::Char('l' | 'L' | 'n' | 'N') => {
                    self.confirm_choice = ConfirmChoice::No;
                }
                KeyCode::Enter | KeyCode::Char(' ') => self.submit_confirmation(),
                KeyCode::Esc => self.cancel_confirmation(),
                KeyCode::Char('v') if self.clean_waiting_for_confirmation => {
                    self.review_all_selected()
                }
                _ => {}
            }
            return;
        }

        if self.view == View::Scan {
            self.ensure_scan_view_projection();
        }
        if self.scan_view.filter_open {
            self.handle_scan_filter_key(key);
            return;
        }
        if self.scan_view.sort_open {
            self.handle_scan_sort_key(key);
            return;
        }
        if self.scan_view.search_open {
            self.handle_scan_search_key(key);
            return;
        }
        if self.handle_details_key(key) {
            return;
        }

        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Command => self.handle_command_key(key),
        }
    }

    pub(crate) fn handle_normal_key(&mut self, key: KeyEvent) {
        if self.pending_key == Some('g') {
            if let KeyCode::Char('g') = key.code {
                let count = self.take_count_or(1);
                self.select_line(count);
                self.clear_pending();
                return;
            }
            self.clear_pending();
        }

        if let KeyCode::Char(d) = key.code
            && d.is_ascii_digit()
        {
            self.count_buffer.push(d);
            return;
        }

        match key.code {
            KeyCode::Char('p') if key.modifiers.is_empty() => self.open_scan_search(),
            KeyCode::Char('o') if key.modifiers.is_empty() => self.open_scan_sort(),
            KeyCode::Char('v') if key.modifiers.is_empty() => self.toggle_selected_view(),
            KeyCode::Tab | KeyCode::BackTab
                if self.view == View::Scan
                    && !self.is_scan_running()
                    && !self.is_operation_running() =>
            {
                self.scan_view.details_focused = true;
            }
            KeyCode::Char('z') if self.last_cleanup_result.is_some() => self.show_restore(),
            KeyCode::Char('q') => {
                if self.is_operation_running() {
                    self.status = self.i18n.t("status_operation_running");
                } else {
                    self.should_quit = true;
                }
                self.clear_pending();
            }
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('H') => {
                if self.is_scan_running() {
                    self.cancel_scan();
                } else {
                    self.go_home();
                }
                self.clear_pending();
            }
            KeyCode::Char('x') | KeyCode::Char('X') if self.is_scan_running() => {
                self.cancel_scan();
                self.clear_pending();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let count = self.take_count();
                self.select_next_n(count);
                self.clear_pending();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let count = self.take_count();
                self.select_previous_n(count);
                self.clear_pending();
            }
            KeyCode::Char('g') => {
                self.pending_key = Some('g');
            }
            KeyCode::Char('G') => {
                if self.count_buffer.is_empty() {
                    self.select_last();
                } else {
                    let count = self.take_count();
                    self.select_line(count);
                }
                self.clear_pending();
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                self.toggle_selected();
                self.clear_pending();
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.review();
                self.clear_pending();
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_scan(ScanRequest::default());
                self.clear_pending();
            }
            KeyCode::Char('u') | KeyCode::Char('U')
                if !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.open_current_usage();
                self.clear_pending();
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.dispatch(ActionRequest::Clean {
                    intent: CleanupIntent::UserRequest,
                });
                self.clear_pending();
            }
            KeyCode::Char('/') => {
                self.open_command('/');
                self.clear_pending();
            }
            KeyCode::Char('?') => {
                self.help_open = true;
                self.clear_pending();
            }
            KeyCode::Char('a') | KeyCode::Char('%') => {
                self.toggle_all_scan_selection();
                self.clear_pending();
            }
            KeyCode::Char('A') => {
                self.toggle_global_scan_selection();
                self.clear_pending();
            }
            KeyCode::Char('f')
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.open_scan_filter();
                self.clear_pending();
            }
            KeyCode::PageDown => {
                self.page_down();
                self.clear_pending();
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.page_down();
                self.clear_pending();
            }
            KeyCode::PageUp => {
                self.page_up();
                self.clear_pending();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.page_up();
                self.clear_pending();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.page_forward();
                self.clear_pending();
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.page_back();
                self.clear_pending();
            }
            _ => {
                self.clear_pending();
            }
        }
    }

    pub(crate) fn handle_command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_command(),
            KeyCode::Enter => {
                self.submit_input();
            }
            KeyCode::Backspace => {
                if self.input.len() <= self.command_prefix_len() {
                    self.close_command();
                } else {
                    self.backspace_command_char();
                }
            }
            KeyCode::Delete => self.delete_command_char(),
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_word_back();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_to_command_start();
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_to_command_end();
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input_cursor = self.command_prefix_len();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input_cursor = self.input.len();
            }
            KeyCode::Left => self.move_command_cursor_left(),
            KeyCode::Right => self.move_command_cursor_right(),
            KeyCode::Home => self.input_cursor = self.command_prefix_len(),
            KeyCode::End => self.input_cursor = self.input.len(),
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert_command_text(&ch.to_string());
            }
            KeyCode::Up => {
                if self.palette_open {
                    self.palette_previous();
                } else {
                    self.select_previous();
                }
            }
            KeyCode::Down => {
                if self.palette_open {
                    self.palette_next();
                } else {
                    self.select_next();
                }
            }
            KeyCode::Tab | KeyCode::Char('n')
                if self.palette_open
                    && (key.code == KeyCode::Tab
                        || key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                self.palette_next();
            }
            KeyCode::BackTab | KeyCode::Char('p')
                if self.palette_open
                    && (key.code == KeyCode::BackTab
                        || key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                self.palette_previous();
            }
            _ => {}
        }
    }

    pub(crate) fn open_command(&mut self, prefix: char) {
        self.mode = Mode::Command;
        self.input = String::from(prefix);
        self.input_cursor = self.input.len();
        self.palette_open = prefix == '/';
        self.palette_state.select(Some(0));
    }

    pub(crate) fn delete_word_back(&mut self) {
        let prefix_len = self.command_prefix_len();
        if self.input_cursor <= prefix_len {
            return;
        }

        let end = self.input_cursor;
        let mut start = end;
        while let Some(previous) = previous_grapheme_boundary(&self.input, start, prefix_len) {
            let grapheme = &self.input[previous..start];
            if !grapheme.chars().all(char::is_whitespace) {
                break;
            }
            start = previous;
        }
        while let Some(previous) = previous_grapheme_boundary(&self.input, start, prefix_len) {
            let grapheme = &self.input[previous..start];
            if grapheme.chars().all(char::is_whitespace) {
                break;
            }
            start = previous;
        }
        self.input.replace_range(start..end, "");
        self.input_cursor = start;
        self.refresh_palette_query();
    }

    pub(crate) fn close_command(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
        self.mode = Mode::Normal;
        self.palette_open = false;
        self.palette_state.select(None);
        self.clear_pending();
    }

    pub(crate) fn go_home(&mut self) {
        self.switch_view(View::Home);
        self.status = self.i18n.t("status_home");
    }

    pub fn submit_input(&mut self) {
        let input = std::mem::take(&mut self.input);
        self.input_cursor = 0;
        self.mode = Mode::Normal;
        self.palette_open = false;
        self.clear_pending();

        if input.trim().is_empty() {
            self.palette_state.select(None);
            return;
        }

        if input.starts_with('/') {
            self.submit_slash_input(&input);
            self.palette_state.select(None);
            return;
        }

        self.palette_state.select(None);
        self.status = self.i18n.t("status_help");
    }

    pub(crate) fn submit_slash_input(&mut self, input: &str) {
        let parse_error = match parse_slash_command(input) {
            Ok(action) => {
                self.status = self.i18n.format(
                    "status_queued",
                    &[("command", command_name_for_status(&action).to_string())],
                );
                self.dispatch(action);
                return;
            }
            Err(error) => error,
        };

        // Palette fallback is only for fuzzy command-name completion. Once the user supplied an
        // argument, silently falling back could execute a command after rejecting its options.
        if input.split_whitespace().nth(1).is_some() {
            self.status = localized_command_error(&parse_error, &self.i18n);
            return;
        }

        let filtered = self.filtered_palette_commands_for(input);
        if let Some(cmd) = filtered
            .get(self.palette_state.selected().unwrap_or(0))
            .or(filtered.first())
            && let Ok(action) = parse_slash_command(&palette_command_invocation(cmd.name))
        {
            self.status = self.i18n.format(
                "status_queued",
                &[("command", command_name_for_status(&action).to_string())],
            );
            self.dispatch(action);
            return;
        }

        self.status = self.i18n.t("status_help");
    }

    /// Insert bracketed-paste content into the single-line command editor. Newlines and tabs are
    /// folded to spaces so pasted shell paths cannot accidentally submit a second command.
    pub(crate) fn handle_paste(&mut self, value: &str) {
        if !matches!(self.mode, Mode::Command) {
            return;
        }

        let mut sanitized = String::with_capacity(value.len());
        let mut last_was_space = false;
        for ch in value.chars() {
            let ch = if matches!(ch, '\r' | '\n' | '\t') {
                ' '
            } else {
                ch
            };
            if ch.is_control() {
                continue;
            }
            if ch == ' ' && last_was_space {
                continue;
            }
            last_was_space = ch == ' ';
            sanitized.push(ch);
        }
        if !sanitized.is_empty() {
            self.insert_command_text(&sanitized);
            self.scan_search_changed();
        }
    }

    fn command_prefix_len(&self) -> usize {
        self.input.chars().next().map_or(0, char::len_utf8)
    }

    fn insert_command_text(&mut self, value: &str) {
        self.input_cursor = self.input_cursor.min(self.input.len());
        while !self.input.is_char_boundary(self.input_cursor) {
            self.input_cursor = self.input_cursor.saturating_sub(1);
        }
        self.input.insert_str(self.input_cursor, value);
        self.input_cursor = self.input_cursor.saturating_add(value.len());
        self.refresh_palette_query();
    }

    fn backspace_command_char(&mut self) {
        let prefix_len = self.command_prefix_len();
        let Some(previous) = previous_grapheme_boundary(&self.input, self.input_cursor, prefix_len)
        else {
            return;
        };
        self.input.replace_range(previous..self.input_cursor, "");
        self.input_cursor = previous;
        self.refresh_palette_query();
    }

    fn delete_command_char(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        let next = self.input[self.input_cursor..]
            .graphemes(true)
            .next()
            .map_or(self.input.len(), |grapheme| {
                self.input_cursor + grapheme.len()
            });
        self.input.replace_range(self.input_cursor..next, "");
        self.refresh_palette_query();
    }

    fn delete_to_command_start(&mut self) {
        let prefix_len = self.command_prefix_len();
        if self.input_cursor <= prefix_len {
            return;
        }
        self.input.replace_range(prefix_len..self.input_cursor, "");
        self.input_cursor = prefix_len;
        self.refresh_palette_query();
    }

    fn delete_to_command_end(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        self.input.truncate(self.input_cursor);
        self.refresh_palette_query();
    }

    fn move_command_cursor_left(&mut self) {
        let prefix_len = self.command_prefix_len();
        if let Some(previous) =
            previous_grapheme_boundary(&self.input, self.input_cursor, prefix_len)
        {
            self.input_cursor = previous;
        }
    }

    fn move_command_cursor_right(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        self.input_cursor += self.input[self.input_cursor..]
            .graphemes(true)
            .next()
            .map_or(0, str::len);
    }

    fn refresh_palette_query(&mut self) {
        self.palette_open = self.input.starts_with('/');
        if self.palette_open && !self.filtered_palette_commands().is_empty() {
            self.palette_state.select(Some(0));
        } else {
            self.palette_state.select(None);
        }
    }

    fn is_repeatable_key(&self, key: KeyEvent) -> bool {
        if self.help_open {
            return matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Char('j' | 'k')
            );
        }
        if self.scan_view.filter_open || self.scan_view.sort_open {
            return matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k')
            );
        }
        if self.confirmation_pending() {
            return matches!(key.code, KeyCode::Left | KeyCode::Right);
        }
        if matches!(self.mode, Mode::Command) {
            return matches!(
                key.code,
                KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
            ) || matches!(key.code, KeyCode::Char(_))
                && !key.modifiers.contains(KeyModifiers::ALT);
        }
        matches!(
            key.code,
            KeyCode::Char('j' | 'k')
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
        ) || matches!(key.code, KeyCode::Char('d' | 'u' | 'f' | 'b'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
    }
}

fn previous_grapheme_boundary(value: &str, index: usize, floor: usize) -> Option<usize> {
    if index <= floor {
        return None;
    }
    value[..index]
        .grapheme_indices(true)
        .next_back()
        .map(|(offset, _)| offset)
        .filter(|offset| *offset >= floor)
}
