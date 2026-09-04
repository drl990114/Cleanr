use std::collections::BTreeMap;

use cleanr_core::{CleanupItem, RuleHit, RuleMatchRole, RuleResolutionState, RuleTrust};

use super::*;

const BUILTIN_CATEGORIES: [&str; 9] = [
    "developer-cache",
    "build-cache",
    "package-cache",
    "browser-cache",
    "application-cache",
    "temporary-files",
    "logs",
    "diagnostics",
    "downloads",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CategoryKey {
    Named(String),
    Multiple,
    Unknown,
}

impl CategoryKey {
    pub(crate) fn label(&self, i18n: &I18n, short: bool) -> String {
        let name = match self {
            Self::Named(name) if BUILTIN_CATEGORIES.contains(&name.as_str()) => {
                name.replace('-', "_")
            }
            Self::Named(name) => return name.clone(),
            Self::Multiple => "multiple".to_string(),
            Self::Unknown => "unknown".to_string(),
        };
        let suffix = if short { "_short" } else { "" };
        i18n.t(&format!("category_{name}{suffix}"))
    }

    fn order(&self) -> usize {
        match self {
            Self::Named(name) => BUILTIN_CATEGORIES
                .iter()
                .position(|builtin| *builtin == name)
                .unwrap_or(BUILTIN_CATEGORIES.len()),
            Self::Multiple => BUILTIN_CATEGORIES.len() + 1,
            Self::Unknown => BUILTIN_CATEGORIES.len() + 2,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CandidateCategory {
    pub(crate) key: CategoryKey,
    pub(crate) categories: Vec<String>,
    pub(crate) conflict: bool,
    pub(crate) tentative: bool,
}

impl CandidateCategory {
    fn new(categories: impl Iterator<Item = String>, conflict: bool, tentative: bool) -> Self {
        let categories = categories
            .filter(|category| !category.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let key = match categories.as_slice() {
            [] => CategoryKey::Unknown,
            [category] => CategoryKey::Named(category.clone()),
            _ => CategoryKey::Multiple,
        };
        Self {
            key,
            categories,
            conflict,
            tentative,
        }
    }

    fn from_plan_item(item: &CleanupItem) -> Self {
        if let Some(evidence) = &item.evidence {
            return Self::new(
                evidence
                    .matched_rules
                    .iter()
                    .filter(|rule| !evidence.shadowed_rules.contains(&rule.key))
                    .map(|rule| rule.category.clone()),
                evidence.rule_resolution_state == RuleResolutionState::UnresolvedConflict,
                false,
            );
        }
        Self::new(std::iter::once(item.category.clone()), false, false)
    }

    /// A failed or budget-limited scan can have rule hits without a usable plan. This display
    /// projection mirrors only rule shadowing and never grants cleanup or selection authority.
    fn from_rule_hits(hits: &[RuleHit]) -> Self {
        let has_trusted_primary = hits.iter().any(|hit| {
            hit.match_role == RuleMatchRole::Primary && hit.trust != RuleTrust::Untrusted
        });
        let effective = hits
            .iter()
            .filter(|hit| !(has_trusted_primary && hit.match_role == RuleMatchRole::Fallback))
            .collect::<Vec<_>>();
        let mut category = Self::new(
            effective.iter().map(|hit| hit.category.clone()),
            false,
            true,
        );
        // Only plan evidence authoritatively resolves same-category safety conflicts. Raw rows
        // can identify differing categories without duplicating the domain's risk semantics.
        category.conflict = category.key == CategoryKey::Multiple;
        category
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScanCandidateRow {
    pub(crate) source_index: usize,
    pub(crate) category: CandidateCategory,
    path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct CategorySummary {
    pub(crate) key: CategoryKey,
    pub(crate) count: usize,
    pub(crate) size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProjectionSource {
    Plan {
        address: usize,
        len: usize,
        created_at: DateTime<Utc>,
    },
    Entries {
        address: usize,
        len: usize,
        as_of: DateTime<Utc>,
    },
}

#[derive(Debug, Default)]
pub(crate) struct ScanViewState {
    pub(crate) rows: Vec<ScanCandidateRow>,
    pub(crate) visible: Vec<usize>,
    pub(crate) groups: Vec<CategorySummary>,
    pub(crate) total_size_bytes: u64,
    pub(crate) filter: Option<CategoryKey>,
    pub(crate) filter_open: bool,
    pub(crate) filter_state: ListState,
    pub(crate) hidden_selected_count: usize,
    pub(crate) hidden_selected_bytes: u64,
    source: Option<ProjectionSource>,
    projected_filter: Option<CategoryKey>,
    selected_summary: (usize, u64),
}

impl Workbench {
    fn scan_projection_source(&self) -> ProjectionSource {
        if let Some(plan) = &self.plan {
            ProjectionSource::Plan {
                address: plan.items.as_ptr() as usize,
                len: plan.items.len(),
                created_at: plan.created_at,
            }
        } else {
            ProjectionSource::Entries {
                address: self.entries.as_ptr() as usize,
                len: self.entries.len(),
                as_of: self.scan_as_of,
            }
        }
    }

    /// Call when replacing a plan or changing its candidate metadata in place. Selection changes
    /// have a separate cache refresh, so moving the cursor never walks all candidates.
    pub(crate) fn invalidate_scan_view_projection(&mut self) {
        self.scan_view.source = None;
    }

    pub(crate) fn ensure_scan_view_projection(&mut self) {
        let source = self.scan_projection_source();
        let source_changed = self.scan_view.source.as_ref() != Some(&source);
        let filter_changed = self.scan_view.projected_filter != self.scan_view.filter;
        if source_changed || filter_changed {
            let focused_path = self.selected_scan_row().map(|row| row.path.clone());
            if source_changed {
                let mut groups: BTreeMap<CategoryKey, (usize, u64)> = BTreeMap::new();
                let mut total_size_bytes = 0u64;
                let rows = if let Some(plan) = &self.plan {
                    plan.items
                        .iter()
                        .enumerate()
                        .map(|(source_index, item)| {
                            let category = CandidateCategory::from_plan_item(item);
                            let group = groups.entry(category.key.clone()).or_default();
                            group.0 += 1;
                            group.1 = group.1.saturating_add(item.size_bytes);
                            total_size_bytes = total_size_bytes.saturating_add(item.size_bytes);
                            ScanCandidateRow {
                                source_index,
                                category,
                                path: item.path.clone(),
                            }
                        })
                        .collect()
                } else {
                    self.entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| !entry.rule_hits.is_empty())
                        .map(|(source_index, entry)| {
                            let category = CandidateCategory::from_rule_hits(&entry.rule_hits);
                            let group = groups.entry(category.key.clone()).or_default();
                            group.0 += 1;
                            group.1 = group.1.saturating_add(entry.size_bytes);
                            total_size_bytes = total_size_bytes.saturating_add(entry.size_bytes);
                            ScanCandidateRow {
                                source_index,
                                category,
                                path: entry.path.clone(),
                            }
                        })
                        .collect()
                };
                self.scan_view.rows = rows;
                if self.plan.is_none() {
                    self.candidate_entry_indices = self
                        .scan_view
                        .rows
                        .iter()
                        .map(|row| row.source_index)
                        .collect();
                    self.candidate_count = self.scan_view.rows.len();
                    self.candidate_projection_entries_len = self.entries.len();
                }
                self.scan_view.total_size_bytes = total_size_bytes;
                self.scan_view.groups = groups
                    .into_iter()
                    .map(|(key, (count, size_bytes))| CategorySummary {
                        key,
                        count,
                        size_bytes,
                    })
                    .collect();
                self.scan_view.groups.sort_by(|left, right| {
                    left.key
                        .order()
                        .cmp(&right.key.order())
                        .then(left.key.cmp(&right.key))
                });
                self.scan_view.source = Some(source);
            }
            self.scan_view.visible = self
                .scan_view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    self.scan_view
                        .filter
                        .as_ref()
                        .is_none_or(|filter| filter == &row.category.key)
                })
                .map(|(index, _)| index)
                .collect();
            self.scan_view.projected_filter = self.scan_view.filter.clone();
            if self.view == View::Scan {
                let focused = focused_path.and_then(|path| {
                    self.scan_view
                        .visible
                        .iter()
                        .position(|index| self.scan_view.rows[*index].path == path)
                });
                // On the first projection, preserve a valid caller-provided focus (e.g. a restored
                // list state); changing an existing filter instead falls back to the first row.
                let initial = (!filter_changed && focused.is_none())
                    .then(|| self.list_state.selected())
                    .flatten();
                let selected = focused
                    .or(initial.filter(|index| *index < self.scan_view.visible.len()))
                    .or_else(|| (!self.scan_view.visible.is_empty()).then_some(0));
                self.list_state.select(selected);
                if filter_changed {
                    *self.list_state.offset_mut() = 0;
                }
            }
            self.refresh_hidden_scan_selection();
        } else if self.plan.as_ref().map_or((0, 0), |plan| {
            (
                plan.summary.selected_count,
                plan.summary.selected_size_bytes,
            )
        }) != self.scan_view.selected_summary
        {
            self.refresh_hidden_scan_selection();
        }
    }

    pub(crate) fn selected_scan_row(&self) -> Option<&ScanCandidateRow> {
        let visible_index = self.list_state.selected()?;
        let row_index = *self.scan_view.visible.get(visible_index)?;
        self.scan_view.rows.get(row_index)
    }

    pub(crate) fn scan_total_count(&self) -> usize {
        self.scan_view.rows.len()
    }

    pub(crate) fn scan_visible_count(&self) -> usize {
        if self.scan_view.source.as_ref() == Some(&self.scan_projection_source())
            && self.scan_view.projected_filter == self.scan_view.filter
        {
            self.scan_view.visible.len()
        } else if self.scan_view.filter.is_none() {
            self.plan
                .as_ref()
                .map_or_else(|| self.candidate_count_cached(), |plan| plan.items.len())
        } else {
            0
        }
    }

    /// A visible-only selection change cannot affect hidden selections.
    pub(crate) fn cache_scan_selection_summary(&mut self) {
        self.scan_view.selected_summary = self.plan.as_ref().map_or((0, 0), |plan| {
            (
                plan.summary.selected_count,
                plan.summary.selected_size_bytes,
            )
        });
    }

    pub(crate) fn refresh_hidden_scan_selection(&mut self) {
        self.scan_view.hidden_selected_count = 0;
        self.scan_view.hidden_selected_bytes = 0;
        let Some(plan) = &self.plan else {
            self.scan_view.selected_summary = (0, 0);
            return;
        };
        self.scan_view.selected_summary = (
            plan.summary.selected_count,
            plan.summary.selected_size_bytes,
        );
        let Some(filter) = &self.scan_view.filter else {
            return;
        };
        for row in &self.scan_view.rows {
            if row.category.key != *filter
                && let Some(item) = plan.items.get(row.source_index)
                && item.selected
            {
                self.scan_view.hidden_selected_count += 1;
                self.scan_view.hidden_selected_bytes = self
                    .scan_view
                    .hidden_selected_bytes
                    .saturating_add(item.size_bytes);
            }
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
                    .position(|group| &group.key == filter)
                    .map(|index| index + 1)
            })
            .unwrap_or(0);
        self.scan_view.filter_state.select(Some(selected));
        self.scan_view.filter_open = true;
        self.clear_pending();
    }

    pub(crate) fn handle_scan_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let selected = self.scan_view.filter_state.selected().unwrap_or(0);
                self.scan_view
                    .filter_state
                    .select(Some((selected + 1).min(self.scan_view.groups.len())));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let selected = self.scan_view.filter_state.selected().unwrap_or(0);
                self.scan_view
                    .filter_state
                    .select(Some(selected.saturating_sub(1)));
            }
            KeyCode::Enter => {
                self.scan_view.filter = self
                    .scan_view
                    .filter_state
                    .selected()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| self.scan_view.groups.get(index))
                    .map(|group| group.key.clone());
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
        app.entries = vec![
            entry(&app, "alpha", 100, vec![hit("build-cache", "build")]),
            entry(&app, "beta", 200, vec![hit("logs", "logs")]),
            entry(&app, "gamma", 300, vec![hit("build-cache", "build")]),
        ];
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
        app.entries = vec![
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
        ];
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
        replacement.items.reverse();
        app.plan = Some(replacement);
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
        let item = &mut app.plan.as_mut().unwrap().items[0];
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
        app.entries = vec![
            entry(&app, "plain", 50, vec![]),
            entry(
                &app,
                "cache",
                100,
                vec![fallback, hit("build-cache", "primary")],
            ),
        ];
        app.view = View::Scan;
        app.ensure_scan_view_projection();
        assert_eq!(app.scan_view.rows[0].source_index, 1);
        assert_eq!(
            app.scan_view.rows[0].category.key,
            CategoryKey::Named("build-cache".into())
        );
        assert!(app.scan_view.rows[0].category.tentative);
        app.entries = vec![
            entry(&app, "logs", 75, vec![hit("logs", "logs")]),
            entry(&app, "plain", 50, vec![]),
        ];
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
