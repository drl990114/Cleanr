use super::*;

impl Workbench {
    pub(crate) fn switch_view(&mut self, view: View) {
        if self.view != view {
            self.saved_list_states
                .insert(self.view, self.list_state.clone());
            self.view = view;
            self.list_state = self.saved_list_states.remove(&view).unwrap_or_default();
            if self.list_state.selected().is_none() && self.list_len() > 0 {
                self.list_state.select(Some(0));
            }
        }
    }
    pub(crate) fn list_len(&self) -> usize {
        match self.view {
            View::Home => 0,
            View::Scan => self.scan_visible_count(),
            View::Languages => self.i18n.packs().len(),
            View::Rules => self
                .registry
                .packs()
                .iter()
                .map(|pack| 1 + pack.definition.rules.len())
                .sum(),
            View::Plugins => self.registry.packs().len() + self.plugin_diagnostics().len(),
            View::Tasks => self.task_log.len(),
            View::Usage => self.usage_order.len(),
            View::Restore => self.execution_manifests.len(),
        }
    }

    #[cfg(test)]
    pub(crate) fn rebuild_usage_order(&mut self) {
        let projection = build_usage_projection(&self.entries, &self.roots);
        self.usage_order = projection.order;
        self.usage_max_size = projection.max_size;
        self.usage_descendant_counts = projection.descendant_counts;
        self.usage_ready = true;
    }

    pub(crate) fn candidate_count_cached(&self) -> usize {
        if self.candidate_projection_entries_len == self.entries.len() {
            self.candidate_count
        } else {
            self.entries
                .iter()
                .filter(|entry| !entry.rule_hits.is_empty())
                .count()
        }
    }

    #[cfg(test)]
    pub(crate) fn rebuild_candidate_projection_if_stale(&mut self) {
        if self.candidate_projection_entries_len == self.entries.len() {
            return;
        }
        self.candidate_entry_indices = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| (!entry.rule_hits.is_empty()).then_some(index))
            .collect();
        self.candidate_count = self.candidate_entry_indices.len();
        self.candidate_projection_entries_len = self.entries.len();
    }

    pub(crate) fn reset_list_selection(&mut self) {
        if self.view == View::Scan {
            self.ensure_scan_view_projection();
        }
        if self.list_len() > 0 {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    pub(crate) fn plugin_diagnostics(&self) -> Vec<&PluginDiagnostic> {
        let mut seen = BTreeSet::new();
        self.registry
            .diagnostics()
            .iter()
            .chain(self.i18n.diagnostics())
            .filter(|diagnostic| {
                seen.insert(format!(
                    "{}\0{}\0{}",
                    diagnostic.code,
                    diagnostic.message,
                    diagnostic
                        .path
                        .as_ref()
                        .map_or_else(String::new, |path| path.display().to_string())
                ))
            })
            .collect()
    }

    pub(crate) fn select_next(&mut self) {
        self.select_next_n(1);
    }

    pub(crate) fn select_previous(&mut self) {
        self.select_previous_n(1);
    }

    pub(crate) fn select_next_n(&mut self, n: usize) {
        if self.view == View::Scan {
            self.ensure_scan_view_projection();
        }
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let next = self
            .list_state
            .selected()
            .map_or(0usize, |i: usize| (i + n).min(len - 1));
        self.list_state.select(Some(next));
    }

    pub(crate) fn select_previous_n(&mut self, n: usize) {
        if self.view == View::Scan {
            self.ensure_scan_view_projection();
        }
        if self.list_len() == 0 {
            self.list_state.select(None);
            return;
        }
        let prev = self
            .list_state
            .selected()
            .map_or(0usize, |i: usize| i.saturating_sub(n));
        self.list_state.select(Some(prev));
    }

    pub(crate) fn select_first(&mut self) {
        self.select_line(1);
    }

    pub(crate) fn select_last(&mut self) {
        if self.view == View::Scan {
            self.ensure_scan_view_projection();
        }
        let len = self.list_len();
        if len > 0 {
            self.list_state.select(Some(len - 1));
        }
    }

    pub(crate) fn select_line(&mut self, line: usize) {
        if self.view == View::Scan {
            self.ensure_scan_view_projection();
        }
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let idx = line.saturating_sub(1).min(len - 1);
        self.list_state.select(Some(idx));
    }

    pub(crate) fn page_down(&mut self) {
        let step = (self.viewport_height / 2).max(1) as usize;
        self.select_next_n(step);
    }

    pub(crate) fn page_up(&mut self) {
        let step = (self.viewport_height / 2).max(1) as usize;
        self.select_previous_n(step);
    }

    pub(crate) fn page_forward(&mut self) {
        let step = self.viewport_height.max(1) as usize;
        self.select_next_n(step);
    }

    pub(crate) fn page_back(&mut self) {
        let step = self.viewport_height.max(1) as usize;
        self.select_previous_n(step);
    }

    pub(crate) fn clear_pending(&mut self) {
        self.count_buffer.clear();
        self.pending_key = None;
    }

    pub(crate) fn take_count(&mut self) -> usize {
        self.take_count_or(1)
    }

    pub(crate) fn take_count_or(&mut self, default: usize) -> usize {
        if self.count_buffer.is_empty() {
            default
        } else {
            let count = self.count_buffer.parse().unwrap_or(default);
            self.count_buffer.clear();
            count.max(1)
        }
    }

    pub(crate) fn toggle_selected(&mut self) {
        match self.view {
            View::Scan => self.toggle_scan_selection(),
            View::Languages => self.switch_language(),
            View::Restore => self.request_restore_selected(),
            _ => self.status = self.i18n.t("status_select_scan_only"),
        }
    }

    pub(crate) fn toggle_scan_selection(&mut self) {
        if self.has_background_task() {
            return;
        }
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        self.ensure_scan_view_projection();
        if self.plan.is_none() {
            self.status = self.i18n.t("scan_read_only");
            return;
        }
        let Some(idx) = self.selected_scan_row().map(|row| row.source_index) else {
            return;
        };
        let (path, selected, review) = {
            let Some(plan) = self.plan.as_mut().map(Arc::make_mut) else {
                self.status = self.i18n.t("status_no_scan_results");
                return;
            };
            let Some(item) = plan.items.get_mut(idx) else {
                return;
            };
            item.selected = !item.selected;
            if item.selected {
                plan.summary.selected_count += 1;
                plan.summary.selected_size_bytes += item.size_bytes;
            } else {
                plan.summary.selected_count -= 1;
                plan.summary.selected_size_bytes -= item.size_bytes;
            }
            (
                item.path.clone(),
                item.selected,
                item.evidence
                    .as_ref()
                    .is_some_and(|e| e.recommendation_state == RecommendationState::Review),
            )
        };
        self.update_scan_selection_flag(idx, selected, review);
        self.set_analysis_selection_for_path(&path, selected);
        self.finish_scan_selection_change(false);
        let state = if selected {
            self.i18n.t("state_selected")
        } else {
            self.i18n.t("state_deselected")
        };
        self.status = self.i18n.format(
            "status_item_toggled",
            &[
                ("path", path.display().to_string()),
                ("state", state.to_string()),
            ],
        );
    }

    pub(crate) fn toggle_all_scan_selection(&mut self) {
        self.toggle_scan_selection_scope(false);
    }

    pub(crate) fn toggle_global_scan_selection(&mut self) {
        self.toggle_scan_selection_scope(true);
    }

    fn toggle_scan_selection_scope(&mut self, global: bool) {
        if self.has_background_task() {
            return;
        }
        if self.view != View::Scan {
            self.status = self.i18n.t("status_select_scan_only");
            return;
        }
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        self.ensure_scan_view_projection();
        let Some(plan) = self.plan.as_mut().map(Arc::make_mut) else {
            self.status = self.i18n.t("scan_read_only");
            return;
        };
        let indices = if global {
            (0..plan.items.len()).collect::<Vec<_>>()
        } else {
            self.scan_view
                .visible
                .iter()
                .map(|index| self.scan_view.rows[*index].source_index)
                .collect()
        };
        if indices.is_empty() {
            return;
        }
        let target = indices.iter().any(|index| !plan.items[*index].selected);
        let item_count = indices.len();
        let mut review_count = 0;
        let mut updates = Vec::with_capacity(item_count);
        for index in indices {
            let item = &mut plan.items[index];
            if item.evidence.as_ref().is_some_and(|evidence| {
                evidence.recommendation_state == RecommendationState::Review
            }) {
                review_count += 1;
            }
            if item.selected == target {
                continue;
            }
            item.selected = target;
            if target {
                plan.summary.selected_count += 1;
                plan.summary.selected_size_bytes += item.size_bytes;
            } else {
                plan.summary.selected_count -= 1;
                plan.summary.selected_size_bytes -= item.size_bytes;
            }
            updates.push((
                item.path.clone(),
                target,
                index,
                item.evidence
                    .as_ref()
                    .is_some_and(|e| e.recommendation_state == RecommendationState::Review),
            ));
        }
        for (path, selected, index, review) in updates {
            self.set_analysis_selection_for_path(&path, selected);
            self.update_scan_selection_flag(index, selected, review);
        }
        self.finish_scan_selection_change(global);
        self.status = if target && review_count > 0 {
            self.i18n.format(
                if global {
                    "status_all_toggled_selected_review"
                } else {
                    "status_filtered_toggled_selected_review"
                },
                &[
                    ("count", item_count.to_string()),
                    ("review", review_count.to_string()),
                ],
            )
        } else if target {
            self.i18n.format(
                if global {
                    "status_all_toggled_selected"
                } else {
                    "status_filtered_toggled_selected"
                },
                &[("count", item_count.to_string())],
            )
        } else {
            self.i18n.format(
                if global {
                    "status_all_toggled_deselected"
                } else {
                    "status_filtered_toggled_deselected"
                },
                &[("count", item_count.to_string())],
            )
        };
    }

    fn set_analysis_selection_for_path(&mut self, path: &std::path::Path, selected: bool) {
        let Some(candidate_id) = self.candidate_ids_by_path.get(path).cloned() else {
            return;
        };
        if selected {
            self.selection.select(candidate_id);
        } else {
            self.selection.deselect(&candidate_id);
        }
    }

    pub(crate) fn switch_language(&mut self) {
        let packs = self.i18n.packs();
        if packs.is_empty() {
            return;
        }
        let idx = self.list_state.selected().unwrap_or(0);
        let Some(pack) = packs.get(idx) else {
            return;
        };
        let new_locale = pack.locale.clone();
        self.i18n.set_locale(&new_locale);
        self.config.i18n.locale = Some(new_locale);
        if let Some(path) = self.config_path.clone()
            && let Err(error) = save_config(&self.config, &path)
        {
            self.status = error.to_string();
            return;
        }
        self.status = self.i18n.format(
            "status_language_switched",
            &[("locale", self.i18n.locale().to_string())],
        );
    }

    // ------------------------------------------------------------------
    // Command palette
    // ------------------------------------------------------------------

    pub(crate) fn filtered_palette_commands(&self) -> Vec<crate::commands::CommandInfo> {
        self.filtered_palette_commands_for(&self.input)
    }

    pub(crate) fn has_scan_results(&self) -> bool {
        self.scan_rx.is_none() && (!self.entries.is_empty() || !self.scan_summary.roots.is_empty())
    }

    pub(crate) fn filtered_palette_commands_for(
        &self,
        input: &str,
    ) -> Vec<crate::commands::CommandInfo> {
        filtered_palette_commands(self.has_scan_results(), input, &self.i18n)
    }

    pub(crate) fn palette_next(&mut self) {
        let len = self.filtered_palette_commands().len();
        if len == 0 {
            return;
        }
        let next = self
            .palette_state
            .selected()
            .map_or(0usize, |i: usize| (i + 1) % len);
        self.palette_state.select(Some(next));
    }

    pub(crate) fn palette_previous(&mut self) {
        let len = self.filtered_palette_commands().len();
        if len == 0 {
            return;
        }
        let prev = self
            .palette_state
            .selected()
            .map_or(0usize, |i: usize| if i == 0 { len - 1 } else { i - 1 });
        self.palette_state.select(Some(prev));
    }
}
