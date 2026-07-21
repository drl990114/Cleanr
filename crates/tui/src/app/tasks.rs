use super::*;

impl Workbench {
    /// Drain worker events and report whether observable UI state changed. Progress events are
    /// coalesced so a fast filesystem walk causes one status allocation per UI poll, not one per
    /// channel message.
    pub fn poll_tasks(&mut self) -> bool {
        let operation_changed = self.poll_operation();
        let Some(rx) = self.scan_rx.take() else {
            return operation_changed;
        };

        let mut latest_progress = None;
        let mut finished = None;
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(TaskEvent::ScanProgress(progress)) => latest_progress = Some(progress),
                Ok(TaskEvent::ScanFinished(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.scan_rx = Some(rx);
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if let Some(result) = finished {
            self.finish_scan(result);
            return true;
        }
        if disconnected {
            self.scan_cancel = None;
            self.status = self.i18n.t("status_scan_disconnected");
            self.status_after_scan = None;
            self.scan_progress = None;
            self.review_after_scan = false;
            self.usage_after_scan = false;
            return true;
        }
        if let Some(progress) = latest_progress {
            self.status = self.scan_progress_status(&progress);
            self.scan_progress = Some(progress);
            return true;
        }
        operation_changed
    }

    fn poll_operation(&mut self) -> bool {
        let Some(receiver) = self.operation_rx.take() else {
            return false;
        };
        match receiver.try_recv() {
            Ok(event) => {
                self.operation_kind = None;
                self.finish_operation(event);
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.operation_rx = Some(receiver);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.operation_kind = None;
                self.status = self.i18n.t("status_operation_disconnected");
                true
            }
        }
    }

    fn finish_operation(&mut self, event: OperationEvent) {
        match event {
            OperationEvent::CleanupFinished(Ok(manifest)) => {
                self.finish_cleanup_manifest(manifest);
            }
            OperationEvent::RestoreFinished(Ok(restored)) => {
                let status = self.i18n.format(
                    "status_restored",
                    &[
                        ("succeeded", restored.summary.succeeded.to_string()),
                        ("failed", restored.summary.failed.to_string()),
                        ("restore_id", restored.restore_id.clone()),
                    ],
                );
                self.task_log
                    .push(format!("restore {}", restored.summary.succeeded));
                let succeeded = restored.summary.succeeded;
                if !self
                    .restore_manifests
                    .iter()
                    .any(|existing| existing.restore_id == restored.restore_id)
                {
                    self.restore_manifests.insert(0, restored);
                }
                if succeeded > 0 {
                    self.refresh_roots_after_mutation(status);
                } else {
                    self.status = status;
                }
            }
            OperationEvent::CleanupFinished(Err(error))
            | OperationEvent::RestoreFinished(Err(error)) => self.status = error,
        }
    }

    pub(crate) fn advance_animation(&mut self) -> bool {
        if !self.has_background_task() {
            return false;
        }
        self.animation_tick = self.animation_tick.wrapping_add(1);
        true
    }

    fn finish_scan(&mut self, result: std::result::Result<cleanr_fs::ScanReport, String>) {
        self.scan_cancel = None;
        let mut report = match result {
            Ok(report) => report,
            Err(error) => {
                self.status = if error.contains(SCAN_CANCELLED) {
                    self.i18n.t("status_scan_cancelled")
                } else {
                    error
                };
                self.status_after_scan = None;
                self.scan_progress = None;
                self.review_after_scan = false;
                self.usage_after_scan = false;
                return;
            }
        };

        self.registry
            .annotate_entries_at(&mut report.entries, report.as_of);
        self.scan_as_of = report.as_of;
        self.scan_issues = report.issues;
        self.scan_summary = report.summary;
        self.entries = report.entries;
        self.analysis = None;
        self.candidate_ids_by_path.clear();
        self.selection = UserSelection::default();
        self.plan = None;
        self.scan_progress = None;
        self.rebuild_usage_order();
        self.task_log.push(self.i18n.format(
            "status_scan_log",
            &[
                ("entries", self.scan_summary.entries_seen.to_string()),
                ("errors", self.scan_summary.errors.to_string()),
            ],
        ));
        self.status = self.i18n.format(
            "status_scan_finished",
            &[
                ("entries", self.scan_summary.entries_seen.to_string()),
                (
                    "candidates",
                    self.entries
                        .iter()
                        .filter(|entry| !entry.rule_hits.is_empty())
                        .count()
                        .to_string(),
                ),
            ],
        );
        let activate_scan = self.view == View::Scan || self.review_after_scan;
        self.review_after_scan = false;
        let usage_after_scan = self.usage_after_scan;
        self.usage_after_scan = false;
        if !self.entries.is_empty() {
            self.build_plan_for_view(activate_scan);
        }
        if usage_after_scan {
            self.show_usage();
        } else if self.view == View::Scan {
            self.select_first();
        }
        if let Some(status) = self.status_after_scan.take() {
            self.status = status;
        }
    }

    pub(crate) fn start_scan(&mut self, request: ScanRequest) {
        self.start_scan_for_view(request, View::Scan);
    }

    pub(crate) fn start_scan_for_view(&mut self, mut request: ScanRequest, view: View) {
        if self.is_operation_running() {
            self.status = self.i18n.t("status_operation_running");
            return;
        }
        if self.scan_rx.is_some() {
            self.status = self.i18n.t("status_scan_already_running");
            return;
        }
        self.status_after_scan = None;
        if request.paths.is_empty() && !request.include_global {
            request.paths = self.roots.clone();
        }
        let resolved = match resolve_scan_roots(&request, &self.config.scan.global_kinds) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.status = if error.to_string().contains(NO_GLOBAL_SCAN_ROOTS) {
                    self.i18n.t("status_no_global_caches")
                } else {
                    error.to_string()
                };
                return;
            }
        };
        if resolved.roots.is_empty() {
            self.status = self.i18n.t("status_no_global_caches");
            return;
        }
        self.roots = resolved.roots;

        let roots = self.roots.clone();
        let options = ScanOptions {
            stay_on_filesystem: self.config.scan.stay_on_filesystem,
            ignore_dirs: self.config.scan.ignore_dirs.clone(),
            ignore_patterns: self.config.scan.ignore_patterns.clone(),
        };
        let effect = match spawn_scan(roots, options) {
            Ok(effect) => effect,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.scan_rx = Some(effect.receiver);
        self.scan_cancel = Some(effect.cancellation);
        self.scan_progress = Some(ScanProgress {
            phase: ScanPhase::Scanning,
            entries_total: 0,
            entries_scanned: 0,
            bytes_scanned: 0,
            errors: 0,
            current_path: self.roots.first().cloned(),
        });
        self.entries.clear();
        self.usage_order.clear();
        self.usage_max_size = 0;
        self.usage_descendant_counts.clear();
        self.scan_summary = ScanSummary::default();
        self.scan_as_of = Utc::now();
        self.scan_issues.clear();
        self.analysis = None;
        self.candidate_ids_by_path.clear();
        self.selection = UserSelection::default();
        self.plan = None;
        self.view = view;
        self.list_state.select(None);
        self.status = self.i18n.format(
            "status_scanning",
            &[("roots", self.roots.len().to_string())],
        );
        self.task_log.push(self.i18n.t("status_scan_started"));
    }

    pub(crate) fn cancel_scan(&mut self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Relaxed);
            self.status = self.i18n.t("status_scan_cancelling");
        }
    }

    pub(crate) fn start_usage_scan(&mut self, request: ScanRequest) {
        if self.is_operation_running() {
            self.status = self.i18n.t("status_operation_running");
            return;
        }
        if self.scan_rx.is_some() {
            self.status = self.i18n.t("status_scan_already_running");
            return;
        }
        self.usage_after_scan = true;
        self.start_scan_for_view(request, View::Usage);
    }

    pub(crate) fn scan_progress_status(&self, progress: &ScanProgress) -> String {
        let phase = self.scan_phase_label(progress.phase);
        if progress.phase == ScanPhase::Scanning && progress.entries_total == 0 {
            return self.i18n.format(
                "status_scan_progress_unbounded",
                &[
                    ("phase", phase),
                    ("scanned", progress.entries_scanned.to_string()),
                    ("size", format_bytes(progress.bytes_scanned)),
                ],
            );
        }
        self.i18n.format(
            "status_scan_progress",
            &[
                ("phase", phase),
                ("scanned", progress.entries_scanned.to_string()),
                ("total", progress.entries_total.to_string()),
                ("size", format_bytes(progress.bytes_scanned)),
            ],
        )
    }

    pub(crate) fn scan_phase_label(&self, phase: ScanPhase) -> String {
        let key = match phase {
            ScanPhase::Discovering => "scan_phase_discovering",
            ScanPhase::Scanning => "scan_phase_scanning",
            ScanPhase::Aggregating => "scan_phase_aggregating",
        };
        self.i18n.t(key)
    }

    pub(crate) fn refresh_history(&mut self) {
        match load_history(&self.state_dir) {
            Ok((execution_manifests, restore_manifests)) => {
                self.execution_manifests = execution_manifests;
                self.restore_manifests = restore_manifests;
            }
            Err(error) => {
                self.execution_manifests.clear();
                self.restore_manifests.clear();
                self.status = error.to_string();
            }
        }
    }
}
