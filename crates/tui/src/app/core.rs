use super::*;

impl Workbench {
    pub fn new(
        roots: Vec<PathBuf>,
        config: Config,
        registry: RuleRegistry,
        i18n: I18n,
        theme: Theme,
    ) -> Self {
        let status = i18n.t("status_ready");
        Self {
            roots,
            config,
            registry,
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
            analysis: None,
            candidate_ids_by_path: HashMap::new(),
            selection: UserSelection::default(),
            plan: None,
            task_log: Vec::new(),
            execution_manifests: Vec::new(),
            restore_manifests: Vec::new(),
            scan_rx: None,
            scan_cancel: None,
            scan_progress: None,
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
}
