use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use cleanr_config::{Config, default_config_path, default_state_dir};
use cleanr_core::{
    AnalysisReport, CandidateId, CleanupPlan, ExecutionManifest, ExecutionStatus,
    GlobalScanEvidence, RecommendationPolicy, RecommendationPolicyError, RecommendationState,
    SafetyPolicy, ScanBudgetExceeded, ScanEntry, ScanIssue, ScanRequest, ScanSummary,
    UserSelection,
};
use cleanr_fs::ScanOptions;
use cleanr_i18n::I18n;
use cleanr_plugin_api::PluginDiagnostic;
use cleanr_rules::RuleRegistry;
#[cfg(test)]
use cleanr_tasks::CleanupExecutor;
use cleanr_tasks::{
    build_workflow_analysis_from_parts, build_workflow_plan, restored_run_ids,
    safety_policy_for_config,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

#[cfg(test)]
use crate::effects::execute_cleanup;
use crate::{
    commands::{
        ActionRequest, CleanupIntent, command_name_for_status, filtered_palette_commands,
        localized_command_error, palette_command_invocation, parse_slash_command,
    },
    effects::{
        OperationEvent, OperationKind, PreparedPlanning, PreparedScan, ScanDiagnostics,
        ScanFailure, ScanPreparation, ScanSample, ScanStage, ScanTaskProgress, TaskEvent,
        build_usage_projection, export_cleanup_plan, load_history, save_config, spawn_cleanup,
        spawn_restore, spawn_scan,
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

const DURATION_SAMPLE_CAPACITY: usize = 128;

#[derive(Clone, Debug, Default)]
pub(crate) struct DurationRecorder {
    samples: VecDeque<Duration>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DurationSummary {
    pub p95: Duration,
    pub max: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CleanupResult {
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) cleaned_size_bytes: u64,
    pub(crate) first_path: Option<PathBuf>,
}

impl DurationRecorder {
    pub(crate) fn record(&mut self, duration: Duration) {
        if self.samples.len() == DURATION_SAMPLE_CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(duration);
    }

    pub(crate) fn clear(&mut self) {
        self.samples.clear();
    }

    pub(crate) fn summary(&self) -> DurationSummary {
        if self.samples.is_empty() {
            return DurationSummary::default();
        }
        let mut sorted = self.samples.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();
        let p95_index = sorted.len().saturating_mul(95).div_ceil(100) - 1;
        DurationSummary {
            p95: sorted[p95_index],
            max: *sorted.last().expect("non-empty duration samples"),
        }
    }
}

pub struct Workbench {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) config: Config,
    pub(crate) config_path: Option<PathBuf>,
    /// One-process override supplied at TUI startup; never written back to `Config`.
    pub(crate) session_inactive_days: Option<u16>,
    pub(crate) registry: Arc<RuleRegistry>,
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
    pub(crate) scan_budget_exceeded: Vec<ScanBudgetExceeded>,
    pub(crate) scan_explicit_roots: Vec<PathBuf>,
    pub(crate) scan_global_evidence: GlobalScanEvidence,
    pub(crate) candidate_count: usize,
    pub(crate) candidate_entry_indices: Vec<usize>,
    pub(crate) candidate_projection_entries_len: usize,
    /// One immutable report per completed scan. Candidate IDs remain stable while the user edits
    /// selection and rebuilds a plan.
    pub(crate) analysis: Option<AnalysisReport>,
    pub(crate) candidate_ids_by_path: HashMap<PathBuf, CandidateId>,
    pub(crate) selection: UserSelection,
    pub(crate) plan: Option<CleanupPlan>,
    pub(crate) task_log: Vec<String>,
    /// Result of the latest cleanup completed in this process. Sizes come from the reviewed plan;
    /// persisted execution manifests intentionally remain backward-compatible.
    pub(crate) last_cleanup_result: Option<CleanupResult>,
    pub(crate) execution_manifests: Vec<cleanr_core::ExecutionManifest>,
    pub(crate) restore_manifests: Vec<cleanr_core::RestoreManifest>,
    pub(crate) scan_rx: Option<Receiver<TaskEvent>>,
    pub(crate) scan_sample_rx: Option<Receiver<ScanSample>>,
    pub(crate) scan_cancel: Option<Arc<AtomicBool>>,
    pub(crate) scan_job_id: Option<u64>,
    pub(crate) next_scan_job_id: u64,
    pub(crate) scan_cancel_requested: bool,
    pub(crate) scan_progress: Option<ScanTaskProgress>,
    pub(crate) scan_started_at: Option<Instant>,
    pub(crate) scan_phase_started_at: Option<Instant>,
    pub(crate) scan_last_progress_at: Option<Instant>,
    pub(crate) scan_stall_reported_seconds: Option<u64>,
    pub(crate) scan_diagnostics: Option<ScanDiagnostics>,
    pub(crate) frame_durations: DurationRecorder,
    pub(crate) input_durations: DurationRecorder,
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
