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

        let mut progress_events = Vec::new();
        if let Some(sample_rx) = self.scan_sample_rx.take() {
            let mut latest_sample = None;
            loop {
                match sample_rx.try_recv() {
                    Ok(sample) if self.scan_job_id == Some(sample.job_id) => {
                        latest_sample = Some(sample.progress);
                    }
                    Ok(_) => continue,
                    Err(mpsc::TryRecvError::Empty) => {
                        self.scan_sample_rx = Some(sample_rx);
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }
            if let Some(progress) = latest_sample {
                progress_events.push(progress);
            }
        }
        let mut finished = None;
        let mut disconnected = false;
        let active_job_id = self.scan_job_id;
        loop {
            match rx.try_recv() {
                Ok(TaskEvent::ScanProgress { job_id, progress })
                    if active_job_id == Some(job_id) =>
                {
                    progress_events.push(progress);
                }
                Ok(TaskEvent::ScanFinished {
                    job_id,
                    result,
                    diagnostics,
                }) if active_job_id == Some(job_id) => {
                    finished = Some((result, diagnostics));
                    break;
                }
                // A cancelled job is invalidated immediately, so even a completed snapshot that
                // was already queued cannot replace the current UI state.
                Ok(_) => continue,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if let Some((result, diagnostics)) = finished {
            self.scan_sample_rx = None;
            let result = if self.scan_cancel_requested {
                Err(ScanFailure::Cancelled)
            } else {
                result
            };
            self.finish_scan(result.map(|prepared| *prepared), diagnostics);
            return true;
        }
        if disconnected {
            self.scan_sample_rx = None;
            self.scan_cancel = None;
            self.scan_job_id = None;
            self.status = if self.scan_cancel_requested {
                self.i18n.t("status_scan_cancelled")
            } else {
                self.i18n.t("status_scan_disconnected")
            };
            self.scan_cancel_requested = false;
            self.status_after_scan = None;
            self.scan_progress = None;
            self.clear_scan_watchdog();
            self.review_after_scan = false;
            self.usage_after_scan = false;
            return true;
        }
        self.scan_rx = Some(rx);
        if !progress_events.is_empty() {
            for progress in progress_events {
                self.accept_scan_progress(progress, Instant::now());
            }
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
        self.update_scan_stall_at(Instant::now());
        true
    }

    fn finish_scan(
        &mut self,
        result: std::result::Result<PreparedScan, ScanFailure>,
        diagnostics: ScanDiagnostics,
    ) {
        self.scan_cancel = None;
        self.scan_job_id = None;
        self.scan_cancel_requested = false;
        self.scan_diagnostics = Some(diagnostics.clone());
        self.clear_scan_watchdog();
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.status = match error {
                    ScanFailure::Cancelled => self.i18n.t("status_scan_cancelled"),
                    ScanFailure::NoGlobalRoots => self.i18n.t("status_no_global_caches"),
                    ScanFailure::Message(error) => error,
                };
                self.push_scan_diagnostics(&diagnostics);
                self.status_after_scan = None;
                self.scan_progress = None;
                self.review_after_scan = false;
                self.usage_after_scan = false;
                return;
            }
        };

        let PreparedScan {
            report,
            explicit_roots,
            global_scan,
            candidate_count,
            candidate_entry_indices,
            usage,
            planning,
        } = prepared;
        let workers_used = report.workers_used;
        self.scan_as_of = report.as_of;
        self.scan_issues = report.issues;
        self.scan_budget_exceeded = report.budget_exceeded;
        self.scan_summary = report.summary;
        self.roots = self.scan_summary.roots.clone();
        self.scan_explicit_roots = explicit_roots;
        self.scan_global_evidence = global_scan;
        self.candidate_count = candidate_count;
        self.candidate_entry_indices = candidate_entry_indices;
        self.entries = report.entries;
        self.candidate_projection_entries_len = self.entries.len();
        self.analysis = None;
        self.candidate_ids_by_path.clear();
        self.selection = UserSelection::default();
        self.plan = None;
        self.scan_progress = None;
        if let Some(usage) = usage {
            self.usage_order = usage.order;
            self.usage_max_size = usage.max_size;
            self.usage_descendant_counts = usage.descendant_counts;
        }
        self.task_log.push(self.i18n.format(
            "status_scan_log",
            &[
                ("entries", self.scan_summary.entries_seen.to_string()),
                ("errors", self.scan_summary.errors.to_string()),
            ],
        ));
        self.task_log.push(self.i18n.format(
            "status_scan_workers",
            &[("workers", workers_used.to_string())],
        ));
        self.push_scan_diagnostics(&diagnostics);
        self.status = self.i18n.format(
            "status_scan_finished",
            &[
                ("entries", self.scan_summary.entries_seen.to_string()),
                ("candidates", candidate_count.to_string()),
            ],
        );
        let activate_scan = self.view == View::Scan || self.review_after_scan;
        self.review_after_scan = false;
        let usage_after_scan = self.usage_after_scan;
        self.usage_after_scan = false;
        if activate_scan && !self.entries.is_empty() {
            self.view = View::Scan;
        }
        match planning {
            Ok(Some(PreparedPlanning {
                analysis,
                candidate_ids_by_path,
                selection,
                plan,
            })) => {
                let inactive_days = analysis.policy.preselect_after_days;
                self.analysis = Some(analysis);
                self.candidate_ids_by_path = candidate_ids_by_path;
                self.selection = selection;
                self.status = self.plan_ready_status(&plan, inactive_days);
                self.plan = Some(plan);
                if self.view == View::Scan {
                    self.select_first();
                }
            }
            Ok(None) => {}
            Err(error) => self.status = error,
        }
        if usage_after_scan {
            self.show_usage();
        } else if self.view == View::Scan {
            self.select_first();
        }
        if let Some(status) = self.status_after_scan.take() {
            self.status = status;
        }
        if !self.scan_budget_exceeded.is_empty() {
            self.plan = None;
            self.status = self.i18n.t("status_scan_budget_read_only");
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
        self.reuse_scan_scope_if_unspecified(&mut request);
        let budgets = match self.config.scan.budgets.limits() {
            Ok(budgets) => budgets,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let options = ScanOptions {
            stay_on_filesystem: self.config.scan.stay_on_filesystem,
            ignore_dirs: self.config.scan.ignore_dirs.clone(),
            ignore_patterns: self.config.scan.ignore_patterns.clone(),
            budgets,
            ..ScanOptions::default()
        };
        let preselect_after_days = self.effective_inactive_days(request.inactive_days);
        if let Err(error) = RecommendationPolicy::new(preselect_after_days) {
            self.status = error.to_string();
            return;
        }
        self.next_scan_job_id = self.next_scan_job_id.wrapping_add(1);
        let job_id = self.next_scan_job_id;
        let preparation = ScanPreparation {
            registry: Arc::clone(&self.registry),
            safety_policy: self.safety_policy(),
            preselect_after_days,
            // A normal scan needs evidence and a cleanup plan, but usage remains view-demanded.
            prepare_usage: view == View::Usage,
        };
        let effect = match spawn_scan(
            job_id,
            request,
            self.config.scan.global_kinds.clone(),
            options,
            preparation,
        ) {
            Ok(effect) => effect,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.scan_rx = Some(effect.receiver);
        self.scan_sample_rx = Some(effect.sample_receiver);
        self.scan_cancel = Some(effect.cancellation);
        self.scan_job_id = Some(job_id);
        self.scan_cancel_requested = false;
        self.scan_progress = Some(ScanTaskProgress {
            stage: ScanStage::Resolving,
            entries_total: 0,
            entries_scanned: 0,
            bytes_scanned: 0,
            errors: 0,
            current_path: None,
        });
        let now = Instant::now();
        self.scan_started_at = Some(now);
        self.scan_phase_started_at = Some(now);
        self.scan_last_progress_at = Some(now);
        self.scan_stall_reported_seconds = None;
        self.scan_diagnostics = None;
        self.frame_durations.clear();
        self.input_durations.clear();
        self.entries.clear();
        self.scan_budget_exceeded.clear();
        self.candidate_count = 0;
        self.candidate_entry_indices.clear();
        self.candidate_projection_entries_len = 0;
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
        self.status = self.i18n.t("status_scan_resolving");
        self.task_log.push(self.i18n.t("status_scan_started"));
    }

    pub(crate) fn reuse_scan_scope_if_unspecified(&self, request: &mut ScanRequest) {
        if !request.paths.is_empty() || request.include_global {
            return;
        }
        if self.scan_explicit_roots.is_empty()
            && self.scan_global_evidence.requested_kinds.is_empty()
        {
            request.paths = self.roots.clone();
            return;
        }
        request.paths = self.scan_explicit_roots.clone();
        request.global_kinds = self.scan_global_evidence.requested_kinds.clone();
        request.include_global = !request.global_kinds.is_empty();
    }

    pub(crate) fn cancel_scan(&mut self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Relaxed);
            self.scan_job_id = None;
            self.scan_cancel_requested = true;
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

    pub(crate) fn scan_progress_status(&self, progress: &ScanTaskProgress) -> String {
        let phase = self.scan_stage_label(progress.stage);
        if progress.stage == ScanStage::Scanning && progress.entries_total == 0 {
            return self.i18n.format(
                "status_scan_progress_unbounded",
                &[
                    ("phase", phase),
                    ("scanned", progress.entries_scanned.to_string()),
                    ("size", format_bytes(progress.bytes_scanned)),
                ],
            );
        }
        if progress.stage != ScanStage::Scanning {
            return self.i18n.format(
                "status_scan_progress_stage",
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

    pub(crate) fn scan_stage_label(&self, stage: ScanStage) -> String {
        let key = match stage {
            ScanStage::Resolving => "scan_phase_resolving",
            ScanStage::Scanning => "scan_phase_scanning",
            ScanStage::Aggregating => "scan_phase_aggregating",
            ScanStage::Rules => "scan_phase_rules",
            ScanStage::Evidence => "scan_phase_evidence",
            ScanStage::Plan => "scan_phase_plan",
            ScanStage::Usage => "scan_phase_usage",
        };
        self.i18n.t(key)
    }

    pub(crate) fn scan_elapsed_label(&self) -> String {
        self.scan_started_at.map_or_else(
            || format_duration(Duration::ZERO),
            |started_at| format_duration(Instant::now().saturating_duration_since(started_at)),
        )
    }

    fn accept_scan_progress(&mut self, progress: ScanTaskProgress, now: Instant) {
        if self
            .scan_progress
            .as_ref()
            .is_some_and(|current| current.stage > progress.stage)
        {
            return;
        }
        if self
            .scan_progress
            .as_ref()
            .is_none_or(|current| current.stage != progress.stage)
        {
            self.scan_phase_started_at = Some(now);
        }
        self.scan_last_progress_at = Some(now);
        self.scan_stall_reported_seconds = None;
        self.status = self.scan_progress_status(&progress);
        self.scan_progress = Some(progress);
    }

    fn clear_scan_watchdog(&mut self) {
        self.scan_started_at = None;
        self.scan_phase_started_at = None;
        self.scan_last_progress_at = None;
        self.scan_stall_reported_seconds = None;
    }

    pub(crate) fn update_scan_stall_at(&mut self, now: Instant) -> bool {
        if self.scan_job_id.is_none() {
            return false;
        }
        let Some(last_progress_at) = self.scan_last_progress_at else {
            return false;
        };
        let gap = now.saturating_duration_since(last_progress_at);
        if gap < Duration::from_secs(2) {
            return false;
        }
        let seconds = gap.as_secs();
        if self.scan_stall_reported_seconds == Some(seconds) {
            return false;
        }
        let progress = self.scan_progress.clone().unwrap_or(ScanTaskProgress {
            stage: ScanStage::Resolving,
            entries_total: 0,
            entries_scanned: 0,
            bytes_scanned: 0,
            errors: 0,
            current_path: None,
        });
        self.status = self.i18n.format(
            "status_scan_stalled",
            &[
                ("phase", self.scan_stage_label(progress.stage)),
                ("seconds", seconds.to_string()),
                ("scanned", progress.entries_scanned.to_string()),
                ("size", format_bytes(progress.bytes_scanned)),
                ("errors", progress.errors.to_string()),
            ],
        );
        self.scan_stall_reported_seconds = Some(seconds);
        true
    }

    fn push_scan_diagnostics(&mut self, diagnostics: &ScanDiagnostics) {
        let frame = self.frame_durations.summary();
        let input = self.input_durations.summary();
        let phases = diagnostics
            .phases
            .iter()
            .map(|timing| {
                format!(
                    "{}={}",
                    self.scan_stage_label(timing.stage),
                    format_duration(timing.duration)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.task_log.push(self.i18n.format(
            "status_scan_diagnostics",
            &[
                ("phases", phases),
                ("total", format_duration(diagnostics.total)),
                ("gap", format_duration(diagnostics.longest_progress_gap)),
                ("entries", diagnostics.entries_scanned.to_string()),
                ("size", format_bytes(diagnostics.bytes_scanned)),
                ("errors", diagnostics.errors.to_string()),
                ("frame_p95", format_duration(frame.p95)),
                ("frame_max", format_duration(frame.max)),
                ("input_p95", format_duration(input.p95)),
                ("input_max", format_duration(input.max)),
            ],
        ));
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

fn format_duration(duration: Duration) -> String {
    if duration.is_zero() {
        "0ms".to_string()
    } else if duration < Duration::from_millis(1) {
        "<1ms".to_string()
    } else if duration < Duration::from_secs(1) {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}
