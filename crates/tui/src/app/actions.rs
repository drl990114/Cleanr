use super::*;

impl Workbench {
    pub(crate) fn dispatch(&mut self, action: ActionRequest) {
        match action {
            ActionRequest::Scan(request) => self.start_scan(request),
            ActionRequest::Review => self.review(),
            ActionRequest::Plan => self.build_plan(),
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
        if self.is_operation_running() {
            self.status = self.i18n.t("status_operation_running");
            return;
        }
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        if self.plan.is_none() {
            self.build_plan();
        }
        let Some(plan) = &self.plan else {
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
        match spawn_cleanup(plan.clone(), self.state_dir.clone()) {
            Ok(effect) => {
                self.clean_waiting_for_confirmation = false;
                self.operation_kind = Some(effect.kind);
                self.operation_rx = Some(effect.receiver);
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
        let Some(plan) = &self.plan else {
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

        match execute_cleanup(plan, executor, &self.state_dir, true) {
            Ok(manifest) => self.finish_cleanup_manifest(manifest),
            Err(err) => self.status = err.to_string(),
        }
    }

    pub(crate) fn finish_cleanup_manifest(&mut self, manifest: ExecutionManifest) {
        self.clean_waiting_for_confirmation = false;
        let status = self.cleanup_manifest_status(&manifest);
        let succeeded = manifest.summary.succeeded;
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

    fn cleanup_manifest_status(&self, manifest: &ExecutionManifest) -> String {
        if manifest.summary.failed == 0 {
            return self.i18n.format(
                "status_cleaned",
                &[
                    ("succeeded", manifest.summary.succeeded.to_string()),
                    ("failed", manifest.summary.failed.to_string()),
                    ("run_id", manifest.run_id.clone()),
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
                ("error", error),
                ("run_id", manifest.run_id.clone()),
            ],
        )
    }

    pub(crate) fn submit_confirmation(&mut self) {
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
        if self.is_operation_running() {
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
        self.build_plan();
    }

    pub(crate) fn build_plan(&mut self) {
        self.build_plan_for_view(true);
    }

    /// Create exactly one evidence report for a completed scan. Its candidate IDs stay stable
    /// while the user toggles items and rebuilds the cleanup plan.
    pub(crate) fn ensure_analysis_report(
        &mut self,
    ) -> std::result::Result<(), RecommendationPolicyError> {
        self.rebuild_candidate_projection_if_stale();
        if self.analysis.is_some() {
            return Ok(());
        }
        let safety = self.safety_policy();
        let analysis = build_analysis_report_with_scan_context(
            self.scan_as_of,
            Utc::now(),
            self.roots.clone(),
            &self.entries,
            &self.scan_issues,
            RecommendationPolicy::new(self.effective_inactive_days(None))?,
            AnalysisScanContext {
                budget_exceeded: &self.scan_budget_exceeded,
                safety_policy: Some(&safety),
                global: Some(&self.scan_global_evidence),
                explicit_roots: &self.scan_explicit_roots,
            },
        )?;
        self.candidate_ids_by_path = analysis
            .candidates
            .iter()
            .map(|candidate| (candidate.local_path.clone(), candidate.id.clone()))
            .collect();
        self.selection = UserSelection::from_recommendations(&analysis);
        self.analysis = Some(analysis);
        Ok(())
    }

    pub(crate) fn build_plan_for_view(&mut self, activate_scan: bool) {
        if activate_scan {
            self.view = View::Scan;
        }
        if self.scan_is_budget_limited() {
            self.plan = None;
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
        let mut plan = match build_cleanup_plan_from_analysis(
            self.roots.clone(),
            self.registry.versions(),
            &self.entries,
            analysis,
            &self.selection,
            &policy,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.plan = None;
                self.status = error.to_string();
                return;
            }
        };
        if let Some(source) = plan.source_scan.as_mut() {
            source.scope = Some(CleanupPlanScanScope::new(
                self.scan_explicit_roots.clone(),
                analysis.scan.global.requested_kinds.clone(),
            ));
        }
        self.status = self.plan_ready_status(&plan, inactive_days);
        self.plan = Some(plan);
        if self.view == View::Scan {
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
        let mut protected = Vec::new();
        protected.extend(cleanr_config::home_dir());
        protected.extend(default_config_path());
        if let Ok(executable) = std::env::current_exe() {
            protected.push(executable);
        }
        let mut protected_subtrees = vec![self.state_dir.clone()];
        protected_subtrees.extend(self.config.plugins.dirs.iter().cloned());
        protected_subtrees.extend(self.config.i18n.dirs.iter().cloned());
        SafetyPolicy::new(protected, self.config.cleanup.require_confirm)
            .with_protected_subtrees(protected_subtrees)
    }

    pub(crate) fn export_plan(&mut self, path: Option<PathBuf>) {
        if self.scan_is_budget_limited() {
            self.reject_budget_limited_action();
            return;
        }
        if self.plan.is_none() {
            self.build_plan();
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
        self.status = self.i18n.t("status_scan_budget_read_only");
    }

    pub(crate) fn show_restore(&mut self) {
        self.view = View::Restore;
        self.refresh_history();
        match self.execution_manifests.first() {
            None => {
                self.status = self.i18n.t("status_no_manifests");
            }
            Some(manifest) => {
                self.status = self.i18n.format(
                    "status_latest_run",
                    &[
                        ("run_id", manifest.run_id.clone()),
                        ("count", manifest.summary.succeeded.to_string()),
                        ("message", self.i18n.t("restore_select_hint")),
                    ],
                );
            }
        }
        self.reset_list_selection();
    }

    pub(crate) fn show_rules(&mut self) {
        self.view = View::Rules;
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
        self.view = View::Plugins;
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
        self.view = View::Languages;
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
        self.view = View::Tasks;
        self.status = if self.task_log.is_empty() {
            self.i18n.t("status_no_tasks")
        } else {
            self.task_log.join(" | ")
        };
        self.reset_list_selection();
    }

    pub(crate) fn show_usage(&mut self) {
        if self.usage_order.is_empty() && !self.entries.is_empty() {
            self.rebuild_usage_order();
        }
        self.view = View::Usage;
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
        self.reset_list_selection();
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
