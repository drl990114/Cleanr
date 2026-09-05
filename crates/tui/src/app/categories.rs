use super::*;
pub(crate) use crate::projection::{
    CandidateCategory, CategoryKey, ScanIndex, ScanQuery, ScanSort,
};
use crate::projection::{ScanCandidateRow, prepare_scan_index, project_scan};
#[cfg(test)]
use cleanr_core::{RuleHit, RuleMatchRole, RuleTrust};
#[cfg(test)]
use std::collections::BTreeMap;
use std::ops::Deref;

#[derive(Debug, Default)]
pub(crate) struct ScanViewState {
    pub(crate) index: Arc<ScanIndex>,
    pub(crate) visible: Arc<Vec<usize>>,
    pub(crate) filter: Option<CategoryKey>,
    pub(crate) filter_open: bool,
    pub(crate) filter_state: ListState,
    pub(crate) query: String,
    pub(crate) only_selected: bool,
    pub(crate) sort: ScanSort,
    pub(crate) sort_open: bool,
    pub(crate) sort_state: ListState,
    pub(crate) search_open: bool,
    pub(crate) search_before: String,
    pub(crate) search_due: Option<Instant>,
    pub(crate) details_focused: bool,
    pub(crate) details_scroll: u16,
    pub(crate) details_max_scroll: u16,
    pub(crate) hidden_selected_count: usize,
    pub(crate) hidden_selected_bytes: u64,
    pub(crate) selected_review_count: usize,
    pub(crate) age_excluded_candidates: bool,
    pub(crate) selection_flags: Arc<Vec<bool>>,
    pub(crate) projection_rx: Option<Receiver<crate::effects::ProjectedScan>>,
    pub(crate) projection_cancel: Option<Arc<AtomicBool>>,
    query_revision: u64,
    source: Option<(u64, bool, usize)>,
    pub(super) projected_query: Option<ScanQuery>,
    focused_path: Option<PathBuf>,
    keep_row_position: bool,
    selected_summary: (usize, u64),
    selection_revision: u64,
    projected_selection_revision: u64,
}

impl Deref for ScanViewState {
    type Target = ScanIndex;
    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

impl Drop for ScanViewState {
    fn drop(&mut self) {
        if let Some(cancel) = &self.projection_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Workbench {
    fn scan_projection_source(&self) -> (u64, bool, usize) {
        (
            self.scan_data_revision,
            self.plan.is_some(),
            self.plan
                .as_ref()
                .map_or(self.entries.len(), |p| p.items.len()),
        )
    }

    pub(crate) fn invalidate_scan_view_projection(&mut self) {
        self.scan_data_revision = self.scan_data_revision.wrapping_add(1);
        self.scan_view.source = None;
    }

    pub(crate) fn install_scan_index(&mut self, index: ScanIndex) {
        let focused = self.selected_scan_row().map(|row| row.path.clone());
        self.scan_view.index = Arc::new(index);
        self.scan_view.selection_flags =
            Arc::new(self.plan.as_ref().map_or_else(Vec::new, |plan| {
                plan.items.iter().map(|item| item.selected).collect()
            }));
        self.scan_view.selected_review_count = self.plan.as_ref().map_or(0, |plan| {
            plan.items
                .iter()
                .filter(|item| {
                    item.selected
                        && item
                            .evidence
                            .as_ref()
                            .is_some_and(|e| e.recommendation_state == RecommendationState::Review)
                })
                .count()
        });
        self.scan_view.age_excluded_candidates = self.analysis.as_ref().is_some_and(|analysis| {
            analysis.candidates.iter().any(|candidate| {
                !matches!(
                    candidate.recommendation.state,
                    RecommendationState::Excluded | RecommendationState::Suppressed
                ) && analysis.policy.filters_candidate_projection_by_inactivity()
                    && !analysis
                        .policy
                        .activity_meets_inactivity_threshold(&candidate.activity)
            })
        });
        self.scan_view.source = Some(self.scan_projection_source());
        self.scan_view.projected_query = None;
        self.scan_view.focused_path = focused;
        self.ensure_scan_view_projection();
    }

    fn scan_query(&self) -> ScanQuery {
        ScanQuery {
            category: self.scan_view.filter.clone(),
            text: self.scan_view.query.replace('\\', "/").to_lowercase(),
            only_selected: self.scan_view.only_selected,
            sort: self.scan_view.sort,
        }
    }

    pub(crate) fn scan_projection_pending(&self) -> bool {
        self.scan_view.projection_rx.is_some() || self.scan_view.search_due.is_some()
    }

    pub(crate) fn ensure_scan_view_projection(&mut self) {
        if self.scan_view.source != Some(self.scan_projection_source()) {
            // Normal worker results carry their index. This fallback serves explicit in-process
            // callers replacing a plan or constructing read-only evidence.
            let index = prepare_scan_index(self.plan.as_deref(), &self.entries);
            self.install_scan_index(index);
            return;
        }
        if self.scan_view.search_due.is_some() {
            return;
        }
        let query = self.scan_query();
        if self.scan_view.projected_query.as_ref() == Some(&query)
            && (!query.only_selected
                || self.scan_view.projected_selection_revision == self.scan_view.selection_revision)
        {
            return;
        }
        let focus = self
            .scan_view
            .focused_path
            .take()
            .or_else(|| self.selected_scan_row().map(|r| r.path.clone()));
        self.scan_view.focused_path = focus;
        self.scan_view.query_revision = self.scan_view.query_revision.wrapping_add(1);
        if let Some(cancel) = self.scan_view.projection_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.scan_view.projection_rx = None;
        self.scan_view.projected_query = Some(query.clone());
        self.scan_view.projected_selection_revision = self.scan_view.selection_revision;
        let all_rows = query.category.is_none() && query.text.is_empty() && !query.only_selected;
        if all_rows || self.scan_view.rows.len() <= 2048 {
            let visible = project_scan(
                &self.scan_view.index,
                &query,
                &self.scan_view.selection_flags,
                &AtomicBool::new(false),
            )
            .unwrap_or_default();
            self.commit_scan_projection(visible);
        } else {
            match crate::effects::spawn_projection(
                Arc::clone(&self.scan_view.index),
                query,
                Arc::clone(&self.scan_view.selection_flags),
                self.scan_data_revision,
                self.scan_view.query_revision,
            ) {
                Ok((receiver, cancel)) => {
                    self.scan_view.projection_rx = Some(receiver);
                    self.scan_view.projection_cancel = Some(cancel);
                }
                Err(error) => {
                    self.scan_view.projected_query = None;
                    self.scan_view.visible = Arc::new(Vec::new());
                    self.clear_scan_focus();
                    self.status = error.to_string();
                }
            }
        }
    }

    fn commit_scan_projection(&mut self, visible: Arc<Vec<usize>>) {
        let focus = self.scan_view.focused_path.take();
        let focused = focus.as_ref().and_then(|path| {
            visible
                .iter()
                .position(|i| self.scan_view.rows[*i].path == *path)
        });
        let state = if self.view == View::Scan {
            &mut self.list_state
        } else {
            self.saved_list_states.entry(View::Scan).or_default()
        };
        // Selected-only removal keeps the next row (or the last remaining row).
        let initial = state
            .selected()
            .filter(|_| {
                !visible.is_empty() && (focus.is_none() || self.scan_view.keep_row_position)
            })
            .map(|i| i.min(visible.len() - 1));
        self.scan_view.keep_row_position = false;
        self.scan_view.visible = visible;
        state.select(
            focused
                .or(initial)
                .or_else(|| (!self.scan_view.visible.is_empty()).then_some(0)),
        );
        self.scan_view.details_scroll = 0;
        self.refresh_hidden_scan_selection();
    }

    fn clear_scan_focus(&mut self) {
        if self.view == View::Scan {
            self.list_state.select(None);
        } else {
            self.saved_list_states
                .entry(View::Scan)
                .or_default()
                .select(None);
        }
    }

    pub(crate) fn poll_scan_projection(&mut self) -> bool {
        let mut changed = false;
        if self
            .scan_view
            .search_due
            .is_some_and(|due| Instant::now() >= due)
        {
            self.scan_view.search_due = None;
            self.ensure_scan_view_projection();
            changed = true;
        }
        let Some(receiver) = self.scan_view.projection_rx.take() else {
            return changed;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.scan_view.projection_cancel = None;
                if result.data_revision == self.scan_data_revision
                    && result.query_revision == self.scan_view.query_revision
                {
                    self.commit_scan_projection(result.visible);
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.scan_view.projection_rx = Some(receiver);
                changed
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.scan_view.projection_cancel = None;
                self.scan_view.projected_query = None;
                self.scan_view.visible = Arc::new(Vec::new());
                self.clear_scan_focus();
                self.status = self.i18n.t("status_operation_disconnected");
                true
            }
        }
    }

    pub(crate) fn selected_scan_row(&self) -> Option<&ScanCandidateRow> {
        let state = if self.view == View::Scan {
            &self.list_state
        } else {
            self.saved_list_states.get(&View::Scan)?
        };
        let visible = state.selected()?;
        self.scan_view
            .rows
            .get(*self.scan_view.visible.get(visible)?)
    }
    pub(crate) fn scan_total_count(&self) -> usize {
        self.scan_view.rows.len()
    }
    pub(crate) fn scan_visible_count(&self) -> usize {
        self.scan_view.visible.len()
    }

    pub(crate) fn cache_scan_selection_summary(&mut self) {
        self.scan_view.selected_summary = self.plan.as_ref().map_or((0, 0), |p| {
            (p.summary.selected_count, p.summary.selected_size_bytes)
        });
    }

    pub(crate) fn refresh_hidden_scan_selection(&mut self) {
        self.cache_scan_selection_summary();
        let mut visible_count = 0usize;
        let mut visible_bytes = 0u64;
        if let Some(plan) = &self.plan {
            for index in self.scan_view.visible.iter() {
                if let Some(item) = plan.items.get(self.scan_view.rows[*index].source_index)
                    && item.selected
                {
                    visible_count += 1;
                    visible_bytes = visible_bytes.saturating_add(item.size_bytes);
                }
            }
        }
        self.scan_view.hidden_selected_count = self
            .scan_view
            .selected_summary
            .0
            .saturating_sub(visible_count);
        self.scan_view.hidden_selected_bytes = self
            .scan_view
            .selected_summary
            .1
            .saturating_sub(visible_bytes);
    }

    pub(crate) fn update_scan_selection_flag(
        &mut self,
        index: usize,
        selected: bool,
        review: bool,
    ) {
        self.scan_view.selection_revision = self.scan_view.selection_revision.wrapping_add(1);
        if let Some(flag) = Arc::make_mut(&mut self.scan_view.selection_flags).get_mut(index) {
            *flag = selected;
        }
        if review {
            if selected {
                self.scan_view.selected_review_count += 1;
            } else {
                self.scan_view.selected_review_count =
                    self.scan_view.selected_review_count.saturating_sub(1);
            }
        }
    }

    pub(crate) fn finish_scan_selection_change(&mut self, global: bool) {
        if self.scan_view.only_selected {
            self.scan_view.keep_row_position = true;
            self.scan_view.projected_query = None;
            self.ensure_scan_view_projection();
        } else if global {
            self.refresh_hidden_scan_selection();
        } else {
            self.cache_scan_selection_summary();
        }
    }

    pub(crate) fn open_scan_filter(&mut self) {
        if self.view != View::Scan || self.has_background_task() {
            return;
        }
        self.ensure_scan_view_projection();
        let selected = self
            .scan_view
            .filter
            .as_ref()
            .and_then(|filter| {
                self.scan_view
                    .groups
                    .iter()
                    .position(|g| &g.key == filter)
                    .map(|i| i + 1)
            })
            .unwrap_or(0);
        self.scan_view.filter_state.select(Some(selected));
        self.scan_view.filter_open = true;
        self.clear_pending();
    }

    pub(crate) fn handle_scan_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let index = self.scan_view.filter_state.selected().unwrap_or(0);
                self.scan_view
                    .filter_state
                    .select(Some((index + 1).min(self.scan_view.groups.len())));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let index = self.scan_view.filter_state.selected().unwrap_or(0);
                self.scan_view
                    .filter_state
                    .select(Some(index.saturating_sub(1)));
            }
            KeyCode::Enter => {
                self.scan_view.filter = self
                    .scan_view
                    .filter_state
                    .selected()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| self.scan_view.groups.get(i))
                    .map(|g| g.key.clone());
                self.scan_view.filter_open = false;
                self.ensure_scan_view_projection();
            }
            KeyCode::Esc => self.scan_view.filter_open = false,
            _ => {}
        }
        self.clear_pending();
    }
}

#[cfg(test)]
mod tests {
    use cleanr_core::{Confidence, EntryKind};
    use cleanr_i18n::builtin_language_packs;

    use super::*;

    fn hit(category: &str, id: &str) -> RuleHit {
        RuleHit {
            rule_pack_id: "category-test".to_string(),
            rule_id: id.to_string(),
            label: id.to_string(),
            category: category.to_string(),
            confidence: Confidence::High,
            reason: "generated data".to_string(),
            risk_note: "can be regenerated".to_string(),
            default_selected: false,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
            sources: Vec::new(),
            runtime_guard: None,
        }
    }

    fn fixture() -> (tempfile::TempDir, Workbench) {
        let root = tempfile::tempdir().expect("temporary scan root");
        let mut config = Config::default();
        config.recommendations.preselect_after_days = 0;
        let app = Workbench::new_with_config_path(
            vec![root.path().to_path_buf()],
            config,
            None,
            RuleRegistry::builtin().expect("builtin rules"),
            I18n::new("en-US", BTreeMap::new(), builtin_language_packs()),
            Theme::dark(),
        );
        (root, app)
    }

    fn entry(app: &Workbench, path: &str, size: u64, hits: Vec<RuleHit>) -> ScanEntry {
        ScanEntry {
            path: app.roots[0].join(path),
            kind: EntryKind::Directory,
            size_bytes: size,
            modified_at: Some(Utc::now() - chrono::Duration::days(180)),
            rule_hits: hits,
        }
    }

    fn mixed_plan() -> (tempfile::TempDir, Workbench) {
        let (root, mut app) = fixture();
        app.entries = Arc::new(vec![
            entry(&app, "alpha", 100, vec![hit("build-cache", "build")]),
            entry(&app, "beta", 200, vec![hit("logs", "logs")]),
            entry(&app, "gamma", 300, vec![hit("build-cache", "build")]),
        ]);
        app.build_plan();
        app.ensure_scan_view_projection();
        assert_eq!(app.scan_view.rows.len(), 3);
        (root, app)
    }

    #[test]
    fn category_conflicts_use_effective_rules_and_count_each_candidate_once() {
        let (_root, mut app) = fixture();
        let mut fallback = hit("logs", "a-fallback");
        fallback.match_role = RuleMatchRole::Fallback;
        let mut different_reason = hit("build-cache", "build-other");
        different_reason.reason = "another reason".to_string();
        app.entries = Arc::new(vec![
            entry(
                &app,
                "shadowed",
                100,
                vec![fallback, hit("build-cache", "build")],
            ),
            entry(
                &app,
                "multiple",
                200,
                vec![hit("build-cache", "build"), hit("logs", "logs")],
            ),
            entry(
                &app,
                "same-category",
                300,
                vec![hit("build-cache", "build"), different_reason],
            ),
        ]);
        app.build_plan();
        let rows = &app.scan_view.rows;
        let shadowed = rows
            .iter()
            .find(|row| row.path.ends_with("shadowed"))
            .unwrap();
        assert_eq!(
            shadowed.category.key,
            CategoryKey::Named("build-cache".into())
        );
        assert!(!shadowed.category.conflict);
        let multiple = rows
            .iter()
            .find(|row| row.path.ends_with("multiple"))
            .unwrap();
        assert_eq!(multiple.category.key, CategoryKey::Multiple);
        assert_eq!(multiple.category.categories, ["build-cache", "logs"]);
        assert!(multiple.category.conflict);
        let same = rows
            .iter()
            .find(|row| row.path.ends_with("same-category"))
            .unwrap();
        assert_eq!(same.category.key, CategoryKey::Named("build-cache".into()));
        assert!(same.category.conflict);
        assert_eq!(
            app.scan_view
                .groups
                .iter()
                .map(|group| group.count)
                .sum::<usize>(),
            3
        );
        assert_eq!(
            app.scan_view
                .groups
                .iter()
                .map(|group| group.size_bytes)
                .sum::<u64>(),
            600
        );
        assert_eq!(app.scan_view.total_size_bytes, 600);
    }

    #[test]
    fn category_projection_refreshes_equal_length_replacement_and_explicit_metadata_change() {
        let (_root, mut app) = mixed_plan();
        app.list_state.select(Some(1));
        let focused = app.selected_scan_row().unwrap().path.clone();
        let mut replacement = app.plan.clone().unwrap();
        Arc::make_mut(&mut replacement).items.reverse();
        app.plan = Some(replacement);
        app.invalidate_scan_view_projection();
        app.ensure_scan_view_projection();
        assert_eq!(app.selected_scan_row().unwrap().path, focused);
        for row in &app.scan_view.rows {
            assert_eq!(
                row.path,
                app.plan.as_ref().unwrap().items[row.source_index].path
            );
        }
        let old_address = app.scan_view.rows.as_ptr();
        app.ensure_scan_view_projection();
        assert_eq!(
            old_address,
            app.scan_view.rows.as_ptr(),
            "unchanged frames reuse rows"
        );
        app.list_state.select(Some(2));
        let focused_before_review = app.selected_scan_row().unwrap().path.clone();
        app.build_plan();
        assert_eq!(
            app.selected_scan_row().unwrap().path,
            focused_before_review,
            "review preserves scan focus"
        );
        let item = &mut Arc::make_mut(app.plan.as_mut().unwrap()).items[0];
        item.category = "custom-data".to_string();
        item.evidence.as_mut().unwrap().matched_rules[0].category = "custom-data".to_string();
        app.invalidate_scan_view_projection();
        app.ensure_scan_view_projection();
        assert_eq!(
            app.scan_view.rows[0].category.key,
            CategoryKey::Named("custom-data".into())
        );
    }

    #[test]
    fn category_empty_filter_clears_focus_and_selection_keys_are_noops() {
        let (_root, mut app) = mixed_plan();
        app.scan_view.filter = Some(CategoryKey::Named("absent-category".into()));
        app.ensure_scan_view_projection();
        let before = app.plan.clone();
        assert_eq!(app.list_state.selected(), None);
        assert_eq!(app.list_len(), 0);
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Char('a'),
            KeyCode::Char('%'),
        ] {
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
            assert_eq!(app.list_state.selected(), None);
            assert_eq!(app.plan, before);
        }
    }

    #[test]
    fn category_raw_projection_tracks_source_indices_and_same_length_replacement() {
        let (_root, mut app) = fixture();
        let mut fallback = hit("temporary-files", "fallback");
        fallback.match_role = RuleMatchRole::Fallback;
        app.entries = Arc::new(vec![
            entry(&app, "plain", 50, vec![]),
            entry(
                &app,
                "cache",
                100,
                vec![fallback, hit("build-cache", "primary")],
            ),
        ]);
        app.view = View::Scan;
        app.ensure_scan_view_projection();
        assert_eq!(app.scan_view.rows[0].source_index, 1);
        assert_eq!(
            app.scan_view.rows[0].category.key,
            CategoryKey::Named("build-cache".into())
        );
        assert!(app.scan_view.rows[0].category.tentative);
        app.entries = Arc::new(vec![
            entry(&app, "logs", 75, vec![hit("logs", "logs")]),
            entry(&app, "plain", 50, vec![]),
        ]);
        app.invalidate_scan_view_projection();
        app.ensure_scan_view_projection();
        assert_eq!(app.scan_view.rows[0].source_index, 0);
        assert_eq!(
            app.scan_view.rows[0].category.key,
            CategoryKey::Named("logs".into())
        );
        assert_eq!(app.scan_view.total_size_bytes, 75);
    }

    #[test]
    fn category_new_scan_resets_filter_and_open_popup() {
        let (_root, mut app) = mixed_plan();
        app.scan_view.filter = Some(CategoryKey::Named("logs".into()));
        app.ensure_scan_view_projection();
        app.open_scan_filter();
        app.start_scan(ScanRequest::default());
        assert!(app.scan_rx.is_some(), "a new scan was accepted");
        assert!(app.scan_view.filter.is_none());
        assert!(!app.scan_view.filter_open);
        assert!(app.scan_view.rows.is_empty());
        assert!(app.scan_view.visible.is_empty());
        app.cancel_scan();
    }
}
