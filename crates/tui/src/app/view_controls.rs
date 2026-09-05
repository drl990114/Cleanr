use super::*;
use crate::projection::ScanSort;

impl Workbench {
    pub(crate) fn open_scan_search(&mut self) {
        if self.view != View::Scan || self.is_scan_running() || self.is_operation_running() {
            return;
        }
        self.scan_view.search_before = self.scan_view.query.clone();
        self.open_command('>');
        self.input.push_str(&self.scan_view.query);
        self.input_cursor = self.input.len();
        self.scan_view.search_open = true;
    }

    pub(crate) fn scan_search_changed(&mut self) {
        if self.scan_view.search_open {
            self.scan_view.query = self.input.get(1..).unwrap_or_default().to_string();
            self.scan_view.search_due = Some(Instant::now() + Duration::from_millis(100));
            if let Some(cancel) = self.scan_view.projection_cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.scan_view.projection_rx = None;
            self.scan_view.projected_query = None;
        }
    }

    pub(crate) fn close_scan_search(&mut self, apply: bool) {
        if !apply {
            self.scan_view.query = self.scan_view.search_before.clone();
        }
        self.scan_view.search_open = false;
        self.scan_view.search_due = None;
        self.close_command();
        self.ensure_scan_view_projection();
    }

    pub(crate) fn handle_scan_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_scan_search(false),
            KeyCode::Enter => self.close_scan_search(true),
            KeyCode::Backspace if self.input.len() <= 1 => {}
            _ => {
                let previous = self.input.clone();
                self.handle_command_key(key);
                if self.input != previous {
                    self.scan_search_changed();
                }
            }
        }
    }

    pub(crate) fn open_scan_sort(&mut self) {
        if self.view != View::Scan || self.is_scan_running() || self.is_operation_running() {
            return;
        }
        self.scan_view.sort_open = true;
        self.scan_view.sort_state.select(
            ScanSort::ALL
                .iter()
                .position(|sort| *sort == self.scan_view.sort),
        );
    }

    pub(crate) fn handle_scan_sort_key(&mut self, key: KeyEvent) {
        let index = self.scan_view.sort_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.scan_view.sort_state.select(Some((index + 1).min(2)))
            }
            KeyCode::Up | KeyCode::Char('k') => self
                .scan_view
                .sort_state
                .select(Some(index.saturating_sub(1))),
            KeyCode::Enter => {
                self.scan_view.sort = ScanSort::ALL[index.min(2)];
                self.scan_view.sort_open = false;
                self.ensure_scan_view_projection();
            }
            KeyCode::Esc => self.scan_view.sort_open = false,
            _ => {}
        }
    }

    pub(crate) fn toggle_selected_view(&mut self) {
        if self.view != View::Scan || self.is_scan_running() || self.is_operation_running() {
            return;
        }
        self.scan_view.only_selected = !self.scan_view.only_selected;
        self.ensure_scan_view_projection();
    }

    pub(crate) fn review_all_selected(&mut self) {
        self.cancel_confirmation();
        self.switch_view(View::Scan);
        self.scan_view.only_selected = true;
        self.scan_view.query.clear();
        self.scan_view.filter = None;
        self.ensure_scan_view_projection();
    }

    pub(crate) fn handle_details_key(&mut self, key: KeyEvent) -> bool {
        if self.view != View::Scan
            || !self.scan_view.details_focused
            || !matches!(self.mode, Mode::Normal)
        {
            return false;
        }
        let amount = match key.code {
            KeyCode::Down | KeyCode::Char('j') => 1i32,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => i32::from(self.viewport_height.max(1)),
            KeyCode::PageUp => -i32::from(self.viewport_height.max(1)),
            KeyCode::Home => -i32::from(u16::MAX),
            KeyCode::End => i32::from(u16::MAX),
            KeyCode::Enter => return true,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => {
                self.scan_view.details_focused = false;
                return true;
            }
            _ => return false,
        };
        self.scan_view.details_scroll = (i32::from(self.scan_view.details_scroll) + amount)
            .clamp(0, i32::from(self.scan_view.details_max_scroll))
            as u16;
        true
    }
}
