use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

use chrono::{DateTime, Utc};
use cleanr_config::{Config, default_config_path, default_state_dir};
use cleanr_core::{
    AnalysisReport, CandidateId, CleanupPlan, EntryKind, ExecutionManifest, RecommendationPolicy,
    RecommendationPolicyError, RecommendationState, SafetyPolicy, ScanEntry, ScanIssue,
    ScanRequest, ScanSummary, UserSelection, build_analysis_report_with_safety_policy,
    build_cleanup_plan_from_analysis,
};
use cleanr_fs::{
    NO_GLOBAL_SCAN_ROOTS, SCAN_CANCELLED, ScanOptions, ScanPhase, ScanProgress, resolve_scan_roots,
};
use cleanr_i18n::I18n;
use cleanr_plugin_api::PluginDiagnostic;
use cleanr_rules::RuleRegistry;
#[cfg(test)]
use cleanr_tasks::CleanupExecutor;
use cleanr_tasks::restored_run_ids;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

#[cfg(test)]
use crate::effects::execute_cleanup;
use crate::{
    commands::{
        ActionRequest, CleanupIntent, command_name_for_status, filtered_palette_commands,
        palette_command_invocation, parse_slash_command,
    },
    effects::{
        OperationEvent, OperationKind, TaskEvent, export_cleanup_plan, load_history, save_config,
        spawn_cleanup, spawn_restore, spawn_scan,
    },
    theme::Theme,
    views::format_bytes,
};

// -------------------------------------------------------------------------
// Application state
// -------------------------------------------------------------------------
pub(crate) enum Mode {
    Normal,
    Command,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ConfirmChoice {
    Yes,
    #[default]
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Home,
    Scan,
    Languages,
    Rules,
    Plugins,
    Tasks,
    Usage,
    Restore,
}

pub struct Workbench {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) config: Config,
    pub(crate) registry: RuleRegistry,
    pub(crate) i18n: I18n,
    pub(crate) theme: Theme,
    pub(crate) state_dir: PathBuf,
    pub(crate) input: String,
    /// Byte offset of the command cursor. It is always kept on a UTF-8 boundary and never moves
    /// before the command prefix.
    pub(crate) input_cursor: usize,
    pub(crate) mode: Mode,
    pub(crate) view: View,
    pub(crate) palette_open: bool,
    pub(crate) help_open: bool,
    pub(crate) status: String,
    /// Operation result restored after an automatic post-mutation refresh scan finishes.
    pub(crate) status_after_scan: Option<String>,
    pub(crate) entries: Vec<ScanEntry>,
    pub(crate) scan_summary: ScanSummary,
    pub(crate) scan_as_of: DateTime<Utc>,
    pub(crate) scan_issues: Vec<ScanIssue>,
    /// One immutable report per completed scan. Candidate IDs remain stable while the user edits
    /// selection and rebuilds a plan.
    pub(crate) analysis: Option<AnalysisReport>,
    pub(crate) candidate_ids_by_path: HashMap<PathBuf, CandidateId>,
    pub(crate) selection: UserSelection,
    pub(crate) plan: Option<CleanupPlan>,
    pub(crate) task_log: Vec<String>,
    pub(crate) execution_manifests: Vec<cleanr_core::ExecutionManifest>,
    pub(crate) restore_manifests: Vec<cleanr_core::RestoreManifest>,
    pub(crate) scan_rx: Option<Receiver<TaskEvent>>,
    pub(crate) scan_cancel: Option<Arc<AtomicBool>>,
    pub(crate) scan_progress: Option<ScanProgress>,
    pub(crate) operation_rx: Option<Receiver<OperationEvent>>,
    pub(crate) operation_kind: Option<OperationKind>,
    /// Stable, size-sorted indices used by the usage view. Keeping this outside the renderer
    /// avoids sorting the full scan result on every frame and every navigation key.
    pub(crate) usage_order: Vec<usize>,
    pub(crate) usage_max_size: u64,
    pub(crate) usage_descendant_counts: Vec<usize>,
    pub(crate) review_after_scan: bool,
    pub(crate) usage_after_scan: bool,
    pub(crate) clean_waiting_for_confirmation: bool,
    pub(crate) restore_waiting_for_confirmation: Option<String>,
    pub(crate) confirm_choice: ConfirmChoice,
    pub(crate) should_quit: bool,
    pub(crate) list_state: ListState,
    pub(crate) palette_state: ListState,
    pub(crate) count_buffer: String,
    pub(crate) pending_key: Option<char>,
    pub(crate) viewport_height: u16,
    pub(crate) animation_tick: u64,
    pub(crate) ime_guard_phase: bool,
}

mod actions;
mod core;
mod input;
mod navigation;
mod tasks;
