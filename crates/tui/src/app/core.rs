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
            status,
            status_after_scan: None,
            entries: Vec::new(),
            scan_summary: ScanSummary::default(),
            scan_as_of: Utc::now(),
            scan_issues: Vec::new(),
            scan_budget_exceeded: Vec::new(),
            scan_explicit_roots: Vec::new(),
            scan_global_evidence: GlobalScanEvidence::default(),
            candidate_count: 0,
            candidate_entry_indices: Vec::new(),
            candidate_projection_entries_len: 0,
            analysis: None,
            candidate_ids_by_path: HashMap::new(),
            selection: UserSelection::default(),
            plan: None,
            task_log: Vec::new(),
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
            operation_rx: None,
            operation_kind: None,
            usage_order: Vec::new(),
            usage_max_size: 0,
            usage_descendant_counts: Vec::new(),
            review_after_scan: false,
            usage_after_scan: false,
            clean_waiting_for_confirmation: false,
            restore_waiting_for_confirmation: None,
            confirm_choice: ConfirmChoice::default(),
            should_quit: false,
            list_state: ListState::default(),
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
        self.is_scan_running() || self.is_operation_running()
    }

    #[must_use]
    pub fn plan(&self) -> Option<&CleanupPlan> {
        self.plan.as_ref()
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
