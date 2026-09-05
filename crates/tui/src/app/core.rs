use super::*;

impl Workbench {
    pub fn new(
        roots: Vec<PathBuf>,
        config: Config,
        registry: RuleRegistry,
        i18n: I18n,
        theme: Theme,
    ) -> Self {
        Self::new_with_config_path(roots, config, default_config_path(), registry, i18n, theme)
    }

    pub fn new_with_config_path(
        roots: Vec<PathBuf>,
        config: Config,
        config_path: Option<PathBuf>,
        registry: RuleRegistry,
        i18n: I18n,
        theme: Theme,
    ) -> Self {
        let status = i18n.t("status_ready");
        Self {
            roots,
            config,
            config_path,
            session_inactive_days: None,
            registry: Arc::new(registry),
            i18n,
            theme,
            state_dir: default_state_dir(),
            input: String::new(),
            input_cursor: 0,
            mode: Mode::Normal,
            view: View::Home,
            palette_open: false,
            help_open: false,
            help_scroll: 0,
            help_max_scroll: 0,
            status,
            update_notice: None,
            update_notice_rx: None,
            status_after_scan: None,
            entries: Arc::new(Vec::new()),
            scan_summary: ScanSummary::default(),
            scan_as_of: Utc::now(),
            scan_issues: Vec::new(),
            scan_budget_exceeded: Vec::new(),
            scan_explicit_roots: Vec::new(),
            scan_global_evidence: GlobalScanEvidence::default(),
            scan_view: ScanViewState::default(),
            scan_data_revision: 0,
            candidate_count: 0,
            candidate_entry_indices: Vec::new(),
            candidate_projection_entries_len: 0,
            analysis: None,
            candidate_ids_by_path: HashMap::new(),
            selection: UserSelection::default(),
            plan: None,
            task_log: Vec::new(),
            last_cleanup_result: None,
            execution_manifests: Vec::new(),
            restore_manifests: Vec::new(),
            scan_rx: None,
            scan_sample_rx: None,
            scan_cancel: None,
            scan_job_id: None,
            next_scan_job_id: 0,
            scan_cancel_requested: false,
            scan_progress: None,
            scan_started_at: None,
            scan_phase_started_at: None,
            scan_last_progress_at: None,
            scan_stall_reported_seconds: None,
            scan_diagnostics: None,
            frame_durations: DurationRecorder::default(),
            input_durations: DurationRecorder::default(),
            input_to_frame_durations: DurationRecorder::default(),
            task_commit_durations: DurationRecorder::default(),
            operation_rx: None,
            operation_kind: None,
            operation_sample_rx: None,
            operation_progress: None,
            usage_order: Vec::new(),
            usage_max_size: 0,
            usage_descendant_counts: Vec::new(),
            review_after_scan: false,
            usage_after_scan: false,
            clean_waiting_for_confirmation: false,
            restore_waiting_for_confirmation: None,
            confirm_choice: ConfirmChoice::default(),
            confirm_content_visible: true,
            should_quit: false,
            list_state: ListState::default(),
            saved_list_states: HashMap::new(),
            usage_ready: false,
            usage_rx: None,
            plan_rx: None,
            plan_cancel: None,
            history_rx: None,
            palette_state: ListState::default(),
            count_buffer: String::new(),
            pending_key: None,
            viewport_height: 10,
            animation_tick: 0,
            ime_guard_phase: false,
        }
    }

    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    #[must_use]
    pub fn palette_open(&self) -> bool {
        self.palette_open
    }

    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    #[must_use]
    pub fn is_home(&self) -> bool {
        self.view == View::Home
    }

    #[must_use]
    pub fn is_scan_running(&self) -> bool {
        self.scan_rx.is_some()
    }

    #[must_use]
    pub(crate) fn is_operation_running(&self) -> bool {
        self.operation_rx.is_some()
    }

    #[must_use]
    pub(crate) fn has_background_task(&self) -> bool {
        self.is_scan_running()
            || self.is_operation_running()
            || self.usage_rx.is_some()
            || self.plan_rx.is_some()
            || self.history_rx.is_some()
            || self.scan_projection_pending()
    }

    #[must_use]
    pub fn plan(&self) -> Option<&CleanupPlan> {
        self.plan.as_deref()
    }

    #[must_use]
    pub fn entries(&self) -> &[ScanEntry] {
        &self.entries
    }

    pub(crate) fn record_frame_duration(&mut self, duration: Duration) {
        self.frame_durations.record(duration);
    }

    pub(crate) fn record_input_duration(&mut self, duration: Duration) {
        self.input_durations.record(duration);
    }
}

impl Drop for Workbench {
    fn drop(&mut self) {
        for cancel in [&self.scan_cancel, &self.plan_cancel].into_iter().flatten() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
