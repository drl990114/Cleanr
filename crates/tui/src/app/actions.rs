use super::*;

impl Workbench {
    pub(crate) fn dispatch(&mut self, action: ActionRequest) {
        match action {
            ActionRequest::Scan(request) => self.start_scan(request),
            ActionRequest::Review => self.review(),
            ActionRequest::Plan => self.request_plan(),
            ActionRequest::Clean { intent } => self.request_cleanup(intent),
            ActionRequest::Restore => self.show_restore(),
            ActionRequest::Rules => self.show_rules(),
            ActionRequest::Plugins => self.show_plugins(),
            ActionRequest::Languages => self.show_languages(),
            ActionRequest::Tasks => self.show_tasks(),
            ActionRequest::Usage(request) => self.start_usage_scan(request),
            ActionRequest::ExportPlan(path) => self.export_plan(path),
            ActionRequest::Help => self.show_help(),
            ActionRequest::Quit => {
                if self.is_operation_running() {
                    self.status = self.i18n.t("status_operation_running");
                } else {
                    self.should_quit = true;
                }
            }
        }
    }

    /// Validate and authorize cleanup on the UI thread, then move the filesystem work to a
    /// background worker. The generic synchronous variant below remains available for deterministic
    /// executor tests.
    pub(crate) fn request_cleanup(&mut self, intent: CleanupIntent) {
        if self.has_background_task() {
            self.status = self.i18n.t("status_operation_running");
            return;
        }
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        if self.plan.is_none() {
            self.status = self.i18n.t("scan_read_only");
            return;
        }
        self.ensure_scan_view_projection();
        let Some(plan) = self.plan.clone() else {
            return;
        };
        if plan.summary.selected_count == 0 {
            self.status = self.i18n.t("status_no_selected_items");
            return;
        }
        let confirmed = intent == CleanupIntent::ExplicitUserConfirmation
            || (intent == CleanupIntent::UserRequest && !plan.safety.requires_confirmation);
        if plan.safety.requires_confirmation && !confirmed {
            self.clean_waiting_for_confirmation = true;
            self.restore_waiting_for_confirmation = None;
            self.confirm_choice = ConfirmChoice::No;
            self.status = self.i18n.format(
                "status_clean_confirm",
                &[
                    ("count", plan.summary.selected_count.to_string()),
                    ("size", format_bytes(plan.summary.selected_size_bytes)),
                ],
            );
            return;
        }

        let count = plan.summary.selected_count;
        let size = format_bytes(plan.summary.selected_size_bytes);
        self.last_cleanup_result = None;
        match spawn_cleanup(plan, self.state_dir.clone()) {
            Ok(effect) => {
                self.clean_waiting_for_confirmation = false;
                self.operation_kind = Some(effect.kind);
                self.operation_rx = Some(effect.receiver);
                self.operation_sample_rx = Some(effect.sample_receiver);
                self.operation_progress = None;
                self.status = self.i18n.format(
                    "status_cleaning",
                    &[("count", count.to_string()), ("size", size)],
                );
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn clean_with_executor(
        &mut self,
        intent: CleanupIntent,
        executor: &impl CleanupExecutor,
    ) {
        if self.plan.is_none() {
            self.build_plan();
        }
        let Some(plan) = self.plan.clone() else {
            return;
        };
        if plan.summary.selected_count == 0 {
            self.status = self.i18n.t("status_no_selected_items");
            return;
        }
        let confirmed = intent == CleanupIntent::ExplicitUserConfirmation
            || (intent == CleanupIntent::UserRequest && !plan.safety.requires_confirmation);
        let needs_confirmation = plan.safety.requires_confirmation;
        if needs_confirmation && !confirmed {
            self.clean_waiting_for_confirmation = true;
            self.restore_waiting_for_confirmation = None;
            self.confirm_choice = ConfirmChoice::No;
            self.status = self.i18n.format(
                "status_clean_confirm",
                &[
                    ("count", plan.summary.selected_count.to_string()),
                    ("size", format_bytes(plan.summary.selected_size_bytes)),
                ],
            );
            return;
        }

        self.last_cleanup_result = None;
        match execute_cleanup(&plan, executor, &self.state_dir, true) {
            Ok(manifest) => self.finish_cleanup_manifest(manifest),
            Err(err) => self.status = err.to_string(),
        }
    }

    pub(crate) fn finish_cleanup_manifest(&mut self, manifest: ExecutionManifest) {
        self.clean_waiting_for_confirmation = false;
        let result = self.cleanup_result(&manifest);
        let status = self.cleanup_manifest_status(&manifest, result.cleaned_size_bytes);
        let succeeded = manifest.summary.succeeded;
        self.last_cleanup_result = Some(result);
        self.task_log.push(status.clone());
        if !self
            .execution_manifests
            .iter()
            .any(|existing| existing.run_id == manifest.run_id)
        {
            self.execution_manifests.insert(0, manifest);
        }
        if succeeded > 0 {
            self.refresh_roots_after_mutation(status);
        } else {
            self.status = status;
        }
    }

    fn cleanup_result(&self, manifest: &ExecutionManifest) -> CleanupResult {
        let sizes_by_path = self.plan.as_ref().map_or_else(HashMap::new, |plan| {
            plan.items
                .iter()
                .map(|item| (&item.path, item.size_bytes))
                .collect::<HashMap<_, _>>()
        });
        let mut first_path = None;
        let mut cleaned_size_bytes = 0u64;
        for item in &manifest.items {
            if item.status != ExecutionStatus::Trashed {
                continue;
            }
            cleaned_size_bytes = cleaned_size_bytes
                .saturating_add(sizes_by_path.get(&item.path).copied().unwrap_or_default());
            if first_path.is_none() {
                first_path = Some(item.path.clone());
            }
        }

        CleanupResult {
            succeeded: manifest.summary.succeeded,
            failed: manifest.summary.failed,
            cleaned_size_bytes,
            first_path,
        }
    }

    fn cleanup_manifest_status(
        &self,
        manifest: &ExecutionManifest,
        cleaned_size_bytes: u64,
    ) -> String {
        if manifest.summary.failed == 0 {
            return self.i18n.format(
                "status_cleaned",
                &[
                    ("succeeded", manifest.summary.succeeded.to_string()),
                    ("size", format_bytes(cleaned_size_bytes)),
                ],
            );
        }

        let error = manifest
            .items
            .iter()
            .find_map(|item| item.error.as_deref())
            .map(|error| error.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_else(|| self.i18n.t("status_cleanup_unknown_error"));
        let key = if manifest.summary.succeeded == 0 {
            "status_cleanup_failed"
        } else {
            "status_cleanup_partial"
        };
        self.i18n.format(
            key,
            &[
                ("succeeded", manifest.summary.succeeded.to_string()),
                ("failed", manifest.summary.failed.to_string()),
                ("size", format_bytes(cleaned_size_bytes)),
                ("error", error),
            ],
        )
    }

    pub(crate) fn submit_confirmation(&mut self) {
        if !self.confirm_content_visible {
            self.status = self.i18n.t("confirm_resize");
            return;
        }
        let confirmed = self.confirm_choice == ConfirmChoice::Yes;
        let restore_run = self.restore_waiting_for_confirmation.take();
        let was_restore = restore_run.is_some();
        let was_clean = self.clean_waiting_for_confirmation;
        self.clean_waiting_for_confirmation = false;
        if confirmed {
            if let Some(run_id) = restore_run {
                self.restore_run(&run_id);
            } else if was_clean {
                self.dispatch(ActionRequest::Clean {
                    intent: CleanupIntent::ExplicitUserConfirmation,
                });
            }
        } else {
            self.status = if was_restore {
                self.i18n.t("status_restore_cancelled")
            } else {
                self.i18n.t("status_clean_cancelled")
            };
        }
    }

    pub(crate) fn cancel_confirmation(&mut self) {
        let was_restore = self.restore_waiting_for_confirmation.take().is_some();
        self.confirm_choice = ConfirmChoice::No;
        self.clean_waiting_for_confirmation = false;
        self.status = if was_restore {
            self.i18n.t("status_restore_cancelled")
        } else {
            self.i18n.t("status_clean_cancelled")
        };
    }

    pub(crate) fn confirmation_pending(&self) -> bool {
        self.clean_waiting_for_confirmation || self.restore_waiting_for_confirmation.is_some()
    }

    pub(crate) fn request_restore_selected(&mut self) {
        if self.is_operation_running() || self.history_rx.is_some() {
            self.status = self.i18n.t("status_operation_running");
            return;
        }
        let idx = self.list_state.selected().unwrap_or(0);
        let Some(manifest) = self.execution_manifests.get(idx) else {
            self.status = self.i18n.t("status_no_manifests");
            return;
        };
        let restored = restored_run_ids(&self.restore_manifests).contains(manifest.run_id.as_str());
        if restored {
            self.status = self.i18n.t("status_restore_already_done");
            return;
        }
        self.clean_waiting_for_confirmation = false;
        self.restore_waiting_for_confirmation = Some(manifest.run_id.clone());
        self.confirm_choice = ConfirmChoice::No;
        self.status = self.i18n.format(
            "status_restore_confirm",
            &[
                ("run_id", manifest.run_id.clone()),
                ("count", manifest.summary.succeeded.to_string()),
            ],
        );
    }

    pub(crate) fn restore_run(&mut self, run_id: &str) {
        if self.is_operation_running() {
            self.status = self.i18n.t("status_operation_running");
            return;
        }
        let Some(manifest) = self
            .execution_manifests
            .iter()
            .find(|manifest| manifest.run_id == run_id)
            .cloned()
        else {
            self.status = "cleanup run manifest was not found".to_string();
            return;
        };
        match spawn_restore(manifest, self.state_dir.clone()) {
            Ok(effect) => {
                self.operation_kind = Some(effect.kind);
                self.operation_rx = Some(effect.receiver);
                self.operation_sample_rx = Some(effect.sample_receiver);
                self.operation_progress = None;
                self.status = self
                    .i18n
                    .format("status_restoring", &[("run_id", run_id.to_string())]);
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(crate) fn review(&mut self) {
        if self.scan_rx.is_some() {
            self.review_after_scan = true;
            self.status = self.i18n.t("status_review_after_scan");
            return;
        }
        if self.plan.is_some() {
            self.switch_view(View::Scan);
            self.ensure_scan_view_projection();
            return;
        }
        self.request_plan();
    }

    pub(crate) fn request_plan(&mut self) {
        if self.has_background_task() {
            return;
        }
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        let Some(analysis) = self.analysis.as_ref().map(Arc::clone) else {
            self.status = self.i18n.t("status_no_scan_results");
            return;
        };
        self.switch_view(View::Scan);
        let input = crate::effects::PlanPreparation {
            source_revision: self.scan_data_revision,
            entries: Arc::clone(&self.entries),
            analysis,
            selection: self.selection.clone(),
            roots: self.roots.clone(),
            registry: Arc::clone(&self.registry),
            safety: self.safety_policy(),
            explicit_roots: self.scan_explicit_roots.clone(),
            global_scan: self.scan_global_evidence.clone(),
        };
        match crate::effects::spawn_plan(input) {
            Ok((receiver, cancel)) => {
                self.plan_rx = Some(receiver);
                self.plan_cancel = Some(cancel);
                self.status = self.i18n.t("status_plan_preparing");
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn build_plan(&mut self) {
        self.build_plan_for_view(true);
    }

    /// Create exactly one evidence report for a completed scan. Its candidate IDs stay stable
    /// while the user toggles items and rebuilds the cleanup plan.
    #[cfg(test)]
    pub(crate) fn ensure_analysis_report(
        &mut self,
    ) -> std::result::Result<(), RecommendationPolicyError> {
        self.rebuild_candidate_projection_if_stale();
        if self.analysis.is_some() {
            return Ok(());
        }
        let safety = self.safety_policy();
        let analysis = build_workflow_analysis_from_parts(
            self.scan_as_of,
            self.roots.clone(),
            &self.entries,
            &self.scan_issues,
            &self.scan_budget_exceeded,
            RecommendationPolicy::new(self.effective_inactive_days(None))?,
            &safety,
            &self.scan_explicit_roots,
            &self.scan_global_evidence,
        )?;
        self.candidate_ids_by_path = analysis
            .candidates
            .iter()
            .map(|candidate| (candidate.local_path.clone(), candidate.id.clone()))
            .collect();
        self.selection = UserSelection::from_recommendations(&analysis);
        self.analysis = Some(Arc::new(analysis));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn build_plan_for_view(&mut self, activate_scan: bool) {
        let entering_scan = activate_scan && self.view != View::Scan;
        if activate_scan {
            self.view = View::Scan;
        }
        if self.scan_is_budget_limited() {
            self.plan = None;
            self.invalidate_scan_view_projection();
            self.reject_budget_limited_action();
            return;
        }
        if self.entries.is_empty() {
            self.status = self.i18n.t("status_no_scan_results");
            return;
        }
        if let Err(error) = self.ensure_analysis_report() {
            self.status = error.to_string();
            return;
        }
        let policy = self.safety_policy();
        let Some(analysis) = &self.analysis else {
            return;
        };
        let inactive_days = analysis.policy.preselect_after_days;
        let plan = match build_workflow_plan(
            self.roots.clone(),
            self.registry.versions(),
            &self.entries,
            analysis,
            &self.selection,
            &policy,
            &self.scan_explicit_roots,
            &self.scan_global_evidence,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.plan = None;
                self.invalidate_scan_view_projection();
                self.status = error.to_string();
                return;
            }
        };
        self.status = self.plan_ready_status(&plan, inactive_days);
        self.plan = Some(Arc::new(plan));
        self.invalidate_scan_view_projection();
        self.ensure_scan_view_projection();
        if entering_scan {
            self.select_first();
        }
    }

    pub(crate) fn plan_ready_status(&self, plan: &CleanupPlan, inactive_days: u16) -> String {
        let key = if inactive_days == 0 {
            "status_plan_ready"
        } else {
            "status_plan_ready_filtered"
        };
        self.i18n.format(
            key,
            &[
                ("candidates", plan.summary.candidate_count.to_string()),
                ("selected", plan.summary.selected_count.to_string()),
                ("size", format_bytes(plan.summary.selected_size_bytes)),
                ("days", inactive_days.to_string()),
            ],
        )
    }

    pub(crate) fn effective_inactive_days(&self, scan_override: Option<u16>) -> u16 {
        scan_override
            .or(self.session_inactive_days)
            .unwrap_or(self.config.recommendations.preselect_after_days)
    }

    pub(crate) fn safety_policy(&self) -> SafetyPolicy {
        safety_policy_for_config(
            &self.config,
            self.config_path.clone(),
            self.state_dir.clone(),
        )
    }

    pub(crate) fn export_plan(&mut self, path: Option<PathBuf>) {
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        if self.plan.is_none() {
            self.status = self.i18n.t("scan_read_only");
            return;
        }
        let Some(plan) = &self.plan else {
            return;
        };
        let path = path.unwrap_or_else(|| PathBuf::from("cleanr-plan.json"));
        match export_cleanup_plan(plan, &path) {
            Ok(()) => {
                self.status = self.i18n.format(
                    "status_exported_plan",
                    &[("path", path.display().to_string())],
                );
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    pub(crate) fn scan_is_budget_limited(&self) -> bool {
        !self.scan_budget_exceeded.is_empty()
    }

    pub(crate) fn reject_budget_limited_action(&mut self) {
        self.plan = None;
        self.invalidate_scan_view_projection();
        self.status = self.i18n.t("status_scan_budget_read_only");
    }

    pub(crate) fn show_restore(&mut self) {
        self.switch_view(View::Restore);
        if self.history_rx.is_some() || self.is_operation_running() {
            return;
        }
        match crate::effects::spawn_history(self.state_dir.clone()) {
            Ok(receiver) => {
                self.history_rx = Some(receiver);
                self.status = self.i18n.t("status_history_loading");
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(crate) fn show_rules(&mut self) {
        self.switch_view(View::Rules);
        let count = self
            .registry
            .packs()
            .iter()
            .map(|pack| pack.definition.rules.len())
            .sum::<usize>();
        self.status = self.i18n.format(
            "status_rules",
            &[
                ("packs", self.registry.packs().len().to_string()),
                ("rules", count.to_string()),
            ],
        );
        self.reset_list_selection();
    }

    pub(crate) fn show_plugins(&mut self) {
        self.switch_view(View::Plugins);
        let packs = self
            .registry
            .packs()
            .iter()
            .map(|pack| format!("{}@{}", pack.definition.id, pack.definition.version))
            .collect::<Vec<_>>()
            .join(", ");
        self.status = self.i18n.format("status_plugins", &[("packs", packs)]);
        self.reset_list_selection();
    }

    pub(crate) fn show_languages(&mut self) {
        self.switch_view(View::Languages);
        let packs = self
            .i18n
            .packs()
            .iter()
            .map(|pack| format!("{}@{} ({})", pack.id, pack.version, pack.locale))
            .collect::<Vec<_>>()
            .join(", ");
        self.status = self.i18n.format(
            "status_languages",
            &[("packs", packs), ("locale", self.i18n.locale().to_string())],
        );
        self.reset_list_selection();
    }

    pub(crate) fn show_tasks(&mut self) {
        self.switch_view(View::Tasks);
        self.status = if self.task_log.is_empty() {
            self.i18n.t("status_no_tasks")
        } else {
            self.task_log.join(" | ")
        };
        self.reset_list_selection();
    }

    pub(crate) fn show_usage(&mut self) {
        self.switch_view(View::Usage);
        let candidates = self.plan.as_ref().map_or_else(
            || self.candidate_count_cached(),
            |plan| plan.summary.candidate_count,
        );
        let (selected, selected_size) = self.plan.as_ref().map_or((0, 0), |plan| {
            (
                plan.summary.selected_count,
                plan.summary.selected_size_bytes,
            )
        });
        self.status = self.i18n.format(
            "status_usage",
            &[
                ("entries", self.scan_summary.entries_seen.to_string()),
                ("total", format_bytes(self.scan_summary.total_size_bytes)),
                ("candidates", candidates.to_string()),
                ("selected", selected.to_string()),
                ("size", format_bytes(selected_size)),
            ],
        );
        if self.list_state.selected().is_none() && self.list_len() > 0 {
            self.select_first();
        }
    }

    pub(crate) fn open_current_usage(&mut self) {
        if self.scan_rx.is_some() || self.is_operation_running() {
            return;
        }
        if !self.has_scan_results() {
            self.start_usage_scan(ScanRequest::default());
            return;
        }
        self.show_usage();
        if !self.usage_ready && self.usage_rx.is_none() {
            match crate::effects::spawn_usage(Arc::clone(&self.entries), self.roots.clone()) {
                Ok(receiver) => {
                    self.usage_rx = Some(receiver);
                    self.status = self.i18n.t("scan_phase_usage");
                }
                Err(error) => self.status = error.to_string(),
            }
        }
    }

    pub(crate) fn show_help(&mut self) {
        self.help_open = true;
        self.status = self.i18n.t("status_help");
    }

    pub(crate) fn refresh_roots_after_mutation(&mut self, completed_status: String) {
        if self.scan_rx.is_some() || self.roots.is_empty() {
            self.status = completed_status;
            return;
        }
        let view = self.view;
        let inactive_days = self
            .analysis
            .as_ref()
            .map(|analysis| analysis.policy.preselect_after_days);
        let mut request = ScanRequest {
            inactive_days,
            ..ScanRequest::default()
        };
        self.reuse_scan_scope_if_unspecified(&mut request);
        self.start_scan_for_view(request, view);
        if self.scan_rx.is_some() {
            self.status_after_scan = Some(completed_status);
        }
    }
}
