#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    fs::Metadata,
    mem::size_of,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use cleanr_core::{
    EntryKind, GlobalScanEvidence, GlobalScanKind, GlobalScanLocationEvidence, ReportIntegrity,
    ScanBudgetExceeded, ScanBudgetLimits, ScanEntry, ScanIssue, ScanIssueCode, ScanRequest,
    ScanSummary,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{
    DirEntry as ParallelDirEntry, Error as ParallelWalkError, ParallelVisitor,
    ParallelVisitorBuilder, WalkBuilder, WalkState,
};
use walkdir::WalkDir;

pub const SCAN_CANCELLED: &str = "scan cancelled";
pub const NO_GLOBAL_SCAN_ROOTS: &str = "no system cleanup locations were found";
pub const MAX_SCAN_WORKERS: usize = 4;
const PARALLEL_SCAN_FAILED: &str = "parallel scan worker failed";
const PARALLEL_BATCH_SIZE: usize = 256;
const PARALLEL_BATCH_QUEUE_DEPTH: usize = MAX_SCAN_WORKERS * 2;

/// A caller-requested scan cancellation. Budget exhaustion is not cancellation: it returns an
/// `Ok` partial report instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCancelled;

impl fmt::Display for ScanCancelled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(SCAN_CANCELLED)
    }
}

impl Error for ScanCancelled {}

/// Returns whether an anyhow error chain represents a caller-requested scan cancellation.
///
/// Display text is deliberately not part of the control-flow contract: an unrelated error may
/// use the same wording without being a cancellation.
#[must_use]
pub fn is_scan_cancelled(error: &anyhow::Error) -> bool {
    error.is::<ScanCancelled>()
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Experimental traversal worker count. Values are clamped to `1..=4`.
    pub workers: usize,
    pub stay_on_filesystem: bool,
    pub ignore_dirs: Vec<PathBuf>,
    pub ignore_patterns: Vec<String>,
    /// Optional soft limits. Any nonzero limit forces serial traversal so cutoffs can be enforced
    /// before retention; the retained subset is not promised to be stable across runs.
    pub budgets: ScanBudgetLimits,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            workers: 1,
            stay_on_filesystem: false,
            ignore_dirs: Vec::new(),
            ignore_patterns: Vec::new(),
            budgets: ScanBudgetLimits::default(),
        }
    }
}

impl ScanOptions {
    #[must_use]
    pub fn effective_workers(&self) -> usize {
        if self.budgets.is_unlimited() {
            self.workers.clamp(1, MAX_SCAN_WORKERS)
        } else {
            1
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalScanRoot {
    pub path: PathBuf,
    pub kind: GlobalScanKind,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalScanEnvironment {
    pub home_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub data_local_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub temp_dir: Option<PathBuf>,
    pub download_dir: Option<PathBuf>,
}

impl GlobalScanEnvironment {
    #[must_use]
    pub fn current() -> Self {
        Self {
            home_dir: dirs::home_dir(),
            cache_dir: dirs::cache_dir(),
            data_local_dir: dirs::data_local_dir(),
            data_dir: dirs::data_dir(),
            temp_dir: Some(std::env::temp_dir()),
            download_dir: dirs::download_dir(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedScanRoots {
    pub roots: Vec<PathBuf>,
    pub global_roots: Vec<GlobalScanRoot>,
    pub global_locations: Vec<GlobalScanRoot>,
}

#[derive(Debug, Clone)]
pub struct ScanReport {
    /// The single reference time for facts derived during this scan.
    pub as_of: DateTime<Utc>,
    pub summary: ScanSummary,
    pub entries: Vec<ScanEntry>,
    /// Structured scan coverage facts, intentionally without local error text.
    pub issues: Vec<ScanIssue>,
    pub errors: Vec<ScanError>,
    /// Path-free evidence for soft limits reached during this scan.
    pub budget_exceeded: Vec<ScanBudgetExceeded>,
    /// The actual traversal worker count after clamping and budget serialization.
    pub workers_used: usize,
    /// Requested roots whose traversal reached its natural end. Soft-stop budgets may leave later
    /// roots unattempted; callers must use this field for coverage claims.
    pub completed_roots: Vec<PathBuf>,
}

impl Default for ScanReport {
    fn default() -> Self {
        Self {
            as_of: Utc::now(),
            summary: ScanSummary::default(),
            entries: Vec::new(),
            issues: Vec::new(),
            errors: Vec::new(),
            budget_exceeded: Vec::new(),
            workers_used: 1,
            completed_roots: Vec::new(),
        }
    }
}

impl ScanReport {
    /// Returns whether every requested scope was scanned without an unexpected failure.
    ///
    /// Intentional exclusions, such as configured ignores and filesystem-boundary skips, are
    /// recorded in [`Self::issues`] but do not make the report partial.
    #[must_use]
    pub fn completeness(&self) -> ReportIntegrity {
        ReportIntegrity::from_scan(&self.issues, &self.budget_exceeded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Discovering,
    Scanning,
    Aggregating,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub entries_total: usize,
    pub entries_scanned: usize,
    pub bytes_scanned: u64,
    pub errors: usize,
    pub current_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pub path: Option<PathBuf>,
    pub message: String,
}

pub fn resolve_scan_roots(
    request: &ScanRequest,
    configured_global_kinds: &[GlobalScanKind],
) -> Result<ResolvedScanRoots> {
    resolve_scan_roots_with_env(
        request,
        configured_global_kinds,
        &GlobalScanEnvironment::current(),
    )
}

pub fn resolve_scan_roots_with_env(
    request: &ScanRequest,
    configured_global_kinds: &[GlobalScanKind],
    environment: &GlobalScanEnvironment,
) -> Result<ResolvedScanRoots> {
    let mut roots = request.paths.clone();
    let mut global_roots = Vec::new();
    let mut global_locations = Vec::new();
    if request.include_global {
        let global_kinds = if request.global_kinds.is_empty() {
            configured_global_kinds
        } else {
            &request.global_kinds
        };
        let requested_locations = discover_global_scan_locations(global_kinds, environment);
        global_roots = normalize_global_roots(requested_locations, environment);
        global_locations = discover_global_scan_locations(&GlobalScanKind::ALL, environment)
            .into_iter()
            .filter(|location| {
                global_roots
                    .iter()
                    .any(|root| location.path == root.path || location.path.starts_with(&root.path))
            })
            .collect();
        roots.extend(global_roots.iter().map(|root| root.path.clone()));
    }

    if roots.is_empty() && !request.include_global {
        roots.push(std::env::current_dir()?);
    }

    Ok(ResolvedScanRoots {
        roots: normalize_roots(roots),
        global_roots,
        global_locations,
    })
}

/// Build deterministic, path-local evidence for the global scope covered by a scan.
///
/// Locations that are not contained by a root in the completed scan are intentionally omitted.
#[must_use]
pub fn global_scan_evidence(
    request: &ScanRequest,
    configured_global_kinds: &[GlobalScanKind],
    resolved: &ResolvedScanRoots,
    completed_roots: &[PathBuf],
) -> GlobalScanEvidence {
    if !request.include_global {
        return GlobalScanEvidence::default();
    }

    let mut requested_kinds = if request.global_kinds.is_empty() {
        configured_global_kinds.to_vec()
    } else {
        request.global_kinds.clone()
    };
    requested_kinds.sort();
    requested_kinds.dedup();

    let mut locations = resolved
        .global_locations
        .iter()
        .filter_map(|location| {
            let scan_root = completed_roots.iter().find(|root| {
                location.path == root.as_path() || location.path.starts_with(root.as_path())
            })?;
            Some(GlobalScanLocationEvidence {
                kind: location.kind,
                label: location.label.clone(),
                local_path: location.path.clone(),
                scan_root: scan_root.clone(),
            })
        })
        .collect::<Vec<_>>();
    locations.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.local_path.cmp(&right.local_path))
            .then_with(|| left.scan_root.cmp(&right.scan_root))
    });

    GlobalScanEvidence {
        requested_kinds,
        locations,
    }
}

#[must_use]
pub fn discover_global_scan_roots(
    kinds: &[GlobalScanKind],
    environment: &GlobalScanEnvironment,
) -> Vec<GlobalScanRoot> {
    normalize_global_roots(
        discover_global_scan_locations(kinds, environment),
        environment,
    )
}

/// Discover every existing named global location before parent scan roots are coalesced.
#[must_use]
pub fn discover_global_scan_locations(
    kinds: &[GlobalScanKind],
    environment: &GlobalScanEnvironment,
) -> Vec<GlobalScanRoot> {
    let mut roots = Vec::new();
    if wants(kinds, GlobalScanKind::DeveloperCaches) {
        push_developer_cache_roots(environment, &mut roots);
    }
    if wants(kinds, GlobalScanKind::BrowserCaches) {
        push_browser_cache_roots(environment, &mut roots);
    }
    if wants(kinds, GlobalScanKind::AppCaches) {
        push_app_cache_roots(environment, &mut roots);
    }
    if wants(kinds, GlobalScanKind::TempFiles)
        && let Some(temp) = &environment.temp_dir
    {
        push_global_root(
            &mut roots,
            temp,
            GlobalScanKind::TempFiles,
            "User temporary files",
        );
    }
    if wants(kinds, GlobalScanKind::Logs) {
        push_log_roots(environment, &mut roots);
    }
    if wants(kinds, GlobalScanKind::Downloads) {
        let download_dir = environment.download_dir.clone().or_else(|| {
            environment
                .home_dir
                .as_ref()
                .map(|home| home.join("Downloads"))
        });
        if let Some(download_dir) = download_dir {
            push_global_root(
                &mut roots,
                &download_dir,
                GlobalScanKind::Downloads,
                "Downloads",
            );
        }
    }
    normalize_global_locations(roots, environment)
}

pub fn scan_paths(paths: &[PathBuf], options: &ScanOptions) -> Result<ScanReport> {
    scan_paths_impl(
        paths,
        ScanRootInput::UserProvided,
        options,
        None,
        Instant::now(),
        &mut |_| {},
    )
}

pub fn scan_paths_with_progress(
    paths: &[PathBuf],
    options: &ScanOptions,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_paths_impl(
        paths,
        ScanRootInput::UserProvided,
        options,
        None,
        Instant::now(),
        &mut on_progress,
    )
}

pub fn scan_paths_with_progress_cancellable(
    paths: &[PathBuf],
    options: &ScanOptions,
    cancelled: &AtomicBool,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_paths_impl(
        paths,
        ScanRootInput::UserProvided,
        options,
        Some(cancelled),
        Instant::now(),
        &mut on_progress,
    )
}

/// Scan roots returned by [`resolve_scan_roots`] without canonicalizing or deduplicating them
/// again. An empty resolved scope remains empty instead of falling back to the current directory.
pub fn scan_resolved_paths(
    resolved: &ResolvedScanRoots,
    options: &ScanOptions,
) -> Result<ScanReport> {
    scan_resolved_paths_started_at(resolved, options, Instant::now())
}

/// Scan already-resolved roots while charging elapsed budget time from `started_at`. Callers that
/// resolve or canonicalize roots before entering the filesystem backend can use this entry point
/// so cooperative elapsed limits cover that preparation as well.
pub fn scan_resolved_paths_started_at(
    resolved: &ResolvedScanRoots,
    options: &ScanOptions,
    started_at: Instant,
) -> Result<ScanReport> {
    scan_paths_impl(
        &resolved.roots,
        ScanRootInput::Resolved,
        options,
        None,
        started_at,
        &mut |_| {},
    )
}

pub fn scan_resolved_paths_with_progress(
    resolved: &ResolvedScanRoots,
    options: &ScanOptions,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_paths_impl(
        &resolved.roots,
        ScanRootInput::Resolved,
        options,
        None,
        Instant::now(),
        &mut on_progress,
    )
}

pub fn scan_resolved_paths_with_progress_cancellable(
    resolved: &ResolvedScanRoots,
    options: &ScanOptions,
    cancelled: &AtomicBool,
    on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_resolved_paths_with_progress_cancellable_started_at(
        resolved,
        options,
        cancelled,
        Instant::now(),
        on_progress,
    )
}

/// Cancellable resolved-root scan that includes caller-side root preparation in the elapsed
/// budget. Cancellation and elapsed limits remain cooperative and cannot interrupt one blocked
/// operating-system filesystem call.
pub fn scan_resolved_paths_with_progress_cancellable_started_at(
    resolved: &ResolvedScanRoots,
    options: &ScanOptions,
    cancelled: &AtomicBool,
    started_at: Instant,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_paths_impl(
        &resolved.roots,
        ScanRootInput::Resolved,
        options,
        Some(cancelled),
        started_at,
        &mut on_progress,
    )
}

#[derive(Clone, Copy)]
enum ScanRootInput {
    UserProvided,
    Resolved,
}

fn scan_paths_impl(
    paths: &[PathBuf],
    root_input: ScanRootInput,
    options: &ScanOptions,
    cancelled: Option<&AtomicBool>,
    started_at: Instant,
    on_progress: &mut dyn FnMut(ScanProgress),
) -> Result<ScanReport> {
    let as_of = Utc::now();
    let roots = match root_input {
        ScanRootInput::UserProvided => normalize_roots(if paths.is_empty() {
            vec![std::env::current_dir()?]
        } else {
            paths.to_vec()
        }),
        ScanRootInput::Resolved => paths.to_vec(),
    };
    let ignore = IgnoreMatcher::new(options)?;

    let mut report = ScanReport {
        as_of,
        summary: ScanSummary {
            roots: roots.clone(),
            ..ScanSummary::default()
        },
        entries: Vec::new(),
        issues: Vec::new(),
        errors: Vec::new(),
        budget_exceeded: Vec::new(),
        workers_used: options.effective_workers(),
        completed_roots: Vec::new(),
    };

    let mut progress = ScanProgressTracker::new(on_progress);
    let mut budget = ScanBudgetTracker::new(options.budgets, started_at);
    if options.effective_workers() == 1 {
        let mut hardlinks = HardlinkTracker::default();
        for root in &roots {
            if budget.check_elapsed() {
                break;
            }
            let result = scan_root(
                root,
                options,
                &ignore,
                cancelled,
                &mut SerialScanState {
                    hardlinks: &mut hardlinks,
                    report: &mut report,
                    progress: &mut progress,
                    budget: &mut budget,
                },
            );
            if let Err(err) = result {
                if is_scan_cancelled(&err) {
                    return Err(err);
                }
                return Err(err).with_context(|| format!("failed to scan {}", root.display()));
            }
            if budget.stopped {
                break;
            }
            report.completed_roots.push(root.clone());
        }
        check_cancelled(cancelled)?;
        budget.check_elapsed();
        report
            .entries
            .sort_unstable_by(|left, right| left.path.cmp(&right.path));
        check_cancelled(cancelled)?;
        budget.check_elapsed();
    } else {
        let parallel = scan_roots_parallel(&roots, options, &ignore, cancelled, &mut progress)?;
        let traversal_completed = parallel.traversal_completed;
        report.errors = parallel.errors;
        report.issues = parallel.issues;
        report.entries = parallel.entries;
        progress.bytes_scanned =
            finalize_parallel_entries(&mut report.entries, parallel.hardlinks, cancelled)?;
        budget.error_count = report.errors.len();
        if traversal_completed {
            report.completed_roots = roots.clone();
        }
    }

    sort_report_diagnostics(&mut report);

    (progress.on_progress)(ScanProgress {
        phase: ScanPhase::Aggregating,
        entries_total: progress.entries_scanned,
        entries_scanned: progress.entries_scanned,
        bytes_scanned: progress.bytes_scanned,
        errors: budget.error_count,
        current_path: None,
    });
    // Elapsed time is checked at aggregation boundaries. A limit cannot interrupt an individual
    // blocking filesystem syscall or leave aggregation scratch half-applied.
    budget.check_elapsed();
    aggregate_directory_sizes(&mut report.entries, cancelled)?;
    budget.check_elapsed();
    budget.finish(&mut report);
    report.summary.entries_seen = report.entries.len();
    report.summary.errors = budget.error_count;
    report.summary.total_size_bytes = total_size_for_roots(&report.entries, &report.summary.roots);

    Ok(report)
}

/// Per-retained-entry conservative allocation estimate. This is intentionally not RSS: it covers
/// the persistent `ScanEntry` and encoded path bytes plus O(N) aggregation structures (path-map
/// node allowance, parent index, child count, and ready queue slot). All arithmetic saturates.
fn estimated_entry_memory_bytes(path: &Path) -> u64 {
    const MAP_NODE_ALLOWANCE: usize = 64;
    // Double the retained element costs to conservatively cover Vec spare capacity. The pending
    // hardlink owner also retains a cloned path until final accounting on filesystems that expose
    // identities; charging it for every entry intentionally overestimates mixed trees.
    let fixed = size_of::<ScanEntry>()
        .saturating_mul(2)
        .saturating_add(size_of::<PendingHardlink>())
        .saturating_add(size_of::<Option<usize>>())
        .saturating_add(size_of::<usize>())
        .saturating_add(size_of::<usize>())
        .saturating_add(MAP_NODE_ALLOWANCE);
    u64::try_from(fixed).unwrap_or(u64::MAX).saturating_add(
        u64::try_from(path.as_os_str().as_encoded_bytes().len())
            .unwrap_or(u64::MAX)
            .saturating_mul(2),
    )
}

struct ScanBudgetTracker {
    limits: ScanBudgetLimits,
    started_at: Instant,
    observed_entries: u64,
    estimated_memory_bytes: u64,
    issue_details_observed: u64,
    error_count: usize,
    stopped: bool,
    exceeded: Vec<ScanBudgetExceeded>,
}

impl ScanBudgetTracker {
    fn new(limits: ScanBudgetLimits, started_at: Instant) -> Self {
        Self {
            limits,
            started_at,
            observed_entries: 0,
            estimated_memory_bytes: 0,
            issue_details_observed: 0,
            error_count: 0,
            stopped: false,
            exceeded: Vec::new(),
        }
    }

    fn check_elapsed(&mut self) -> bool {
        if self.limits.elapsed_millis == 0 {
            return self.stopped;
        }
        let observed = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        if observed >= self.limits.elapsed_millis {
            self.upsert_elapsed(observed);
            self.stopped = true;
        }
        self.stopped
    }

    fn retain_entry(&mut self, path: &Path) -> bool {
        self.observed_entries = self.observed_entries.saturating_add(1);
        if self.limits.entries != 0 && self.observed_entries > self.limits.entries {
            self.exceeded.push(ScanBudgetExceeded::EntryCount {
                limit: self.limits.entries,
                observed: self.observed_entries,
            });
            self.stopped = true;
            return false;
        }
        let projected = self
            .estimated_memory_bytes
            .saturating_add(estimated_entry_memory_bytes(path));
        if self.limits.estimated_memory_bytes != 0 && projected > self.limits.estimated_memory_bytes
        {
            self.exceeded.push(ScanBudgetExceeded::EstimatedMemory {
                limit_bytes: self.limits.estimated_memory_bytes,
                observed_bytes: projected,
            });
            self.stopped = true;
            return false;
        }
        self.estimated_memory_bytes = projected;
        true
    }

    fn record_detail(&mut self, issue: &ScanIssue, error: Option<&ScanError>) -> bool {
        self.issue_details_observed = self.issue_details_observed.saturating_add(1);
        if error.is_some() {
            self.error_count = self.error_count.saturating_add(1);
        }
        if self.limits.issue_details != 0 && self.issue_details_observed > self.limits.issue_details
        {
            return false;
        }
        let path_bytes = |path: Option<&Path>| {
            path.map_or(0, |path| {
                u64::try_from(path.as_os_str().as_encoded_bytes().len()).unwrap_or(u64::MAX)
            })
        };
        let mut detail_bytes = u64::try_from(size_of::<ScanIssue>()).unwrap_or(u64::MAX);
        detail_bytes = detail_bytes.saturating_add(path_bytes(issue.path.as_deref()));
        if let Some(error) = error {
            detail_bytes = detail_bytes
                .saturating_add(u64::try_from(size_of::<ScanError>()).unwrap_or(u64::MAX))
                .saturating_add(path_bytes(error.path.as_deref()))
                .saturating_add(u64::try_from(error.message.len()).unwrap_or(u64::MAX));
        }
        let projected = self.estimated_memory_bytes.saturating_add(detail_bytes);
        if self.limits.estimated_memory_bytes != 0 && projected > self.limits.estimated_memory_bytes
        {
            self.exceeded.push(ScanBudgetExceeded::EstimatedMemory {
                limit_bytes: self.limits.estimated_memory_bytes,
                observed_bytes: projected,
            });
            self.stopped = true;
            return false;
        }
        self.estimated_memory_bytes = projected;
        true
    }

    fn upsert_elapsed(&mut self, observed_millis: u64) {
        if let Some(ScanBudgetExceeded::ElapsedTime {
            observed_millis: observed,
            ..
        }) = self
            .exceeded
            .iter_mut()
            .find(|item| matches!(item, ScanBudgetExceeded::ElapsedTime { .. }))
        {
            *observed = (*observed).max(observed_millis);
        } else {
            self.exceeded.push(ScanBudgetExceeded::ElapsedTime {
                limit_millis: self.limits.elapsed_millis,
                observed_millis,
            });
        }
    }

    fn finish(&mut self, report: &mut ScanReport) {
        if self.limits.issue_details != 0 && self.issue_details_observed > self.limits.issue_details
        {
            self.exceeded.push(ScanBudgetExceeded::IssueDetails {
                limit: self.limits.issue_details,
                observed: self.issue_details_observed,
            });
        }
        self.exceeded.sort_unstable_by_key(|item| match item {
            ScanBudgetExceeded::EntryCount { .. } => 0,
            ScanBudgetExceeded::ElapsedTime { .. } => 1,
            ScanBudgetExceeded::EstimatedMemory { .. } => 2,
            ScanBudgetExceeded::IssueDetails { .. } => 3,
        });
        report.budget_exceeded = std::mem::take(&mut self.exceeded);
    }
}

fn record_scan_detail(
    report: &mut ScanReport,
    budget: &mut ScanBudgetTracker,
    issue: ScanIssue,
    error: Option<ScanError>,
) {
    if budget.record_detail(&issue, error.as_ref()) {
        report.issues.push(issue);
        if let Some(error) = error {
            report.errors.push(error);
        }
    }
}

struct PendingHardlink {
    path: PathBuf,
    identity: FileIdentity,
}

#[derive(Default)]
struct ParallelScanBatch {
    entries: Vec<ScanEntry>,
    hardlinks: Vec<PendingHardlink>,
    issues: Vec<ScanIssue>,
    errors: Vec<ScanError>,
    worker_failure_diagnostic: bool,
    traversal_completed: bool,
}

fn push_parallel_worker_failure(batch: &mut ParallelScanBatch, path: Option<PathBuf>) {
    batch.errors.push(ScanError {
        path: path.clone(),
        message: PARALLEL_SCAN_FAILED.to_string(),
    });
    batch.issues.push(ScanIssue {
        code: ScanIssueCode::TraversalError,
        path,
    });
    batch.worker_failure_diagnostic = true;
}

fn ensure_parallel_worker_failure_diagnostic(
    batch: &mut ParallelScanBatch,
    traversal_failed: bool,
    worker_failed: bool,
) {
    if traversal_failed || (worker_failed && !batch.worker_failure_diagnostic) {
        push_parallel_worker_failure(batch, None);
    }
}

struct ParallelScanRoot {
    path: PathBuf,
    device: Option<u64>,
}

struct ParallelScanVisitorBuilder<'a> {
    sender: mpsc::SyncSender<ParallelScanBatch>,
    roots: &'a [ParallelScanRoot],
    ignore: &'a IgnoreMatcher,
    cancelled: Option<&'a AtomicBool>,
    cancellation_observed: &'a AtomicBool,
    stop_requested: &'a AtomicBool,
    worker_failed: &'a AtomicBool,
    stay_on_filesystem: bool,
}

impl<'a> ParallelVisitorBuilder<'a> for ParallelScanVisitorBuilder<'a> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 'a> {
        Box::new(ParallelScanVisitor {
            sender: self.sender.clone(),
            roots: self.roots,
            ignore: self.ignore,
            cancelled: self.cancelled,
            cancellation_observed: self.cancellation_observed,
            stop_requested: self.stop_requested,
            worker_failed: self.worker_failed,
            stay_on_filesystem: self.stay_on_filesystem,
            batch: ParallelScanBatch::default(),
        })
    }
}

struct ParallelScanVisitor<'a> {
    sender: mpsc::SyncSender<ParallelScanBatch>,
    roots: &'a [ParallelScanRoot],
    ignore: &'a IgnoreMatcher,
    cancelled: Option<&'a AtomicBool>,
    cancellation_observed: &'a AtomicBool,
    stop_requested: &'a AtomicBool,
    worker_failed: &'a AtomicBool,
    stay_on_filesystem: bool,
    batch: ParallelScanBatch,
}

impl Drop for ParallelScanVisitor<'_> {
    fn drop(&mut self) {
        let _ = self.flush_batch();
    }
}

impl ParallelVisitor for ParallelScanVisitor<'_> {
    fn visit(
        &mut self,
        entry: std::result::Result<ParallelDirEntry, ParallelWalkError>,
    ) -> WalkState {
        let panic_path = entry.as_ref().ok().map(|entry| entry.path().to_path_buf());
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.visit_inner(entry))) {
            Ok(state) => state,
            Err(_) => {
                self.worker_failed.store(true, Ordering::Relaxed);
                push_parallel_worker_failure(&mut self.batch, panic_path);
                WalkState::Quit
            }
        }
    }
}

impl ParallelScanVisitor<'_> {
    fn visit_inner(
        &mut self,
        entry: std::result::Result<ParallelDirEntry, ParallelWalkError>,
    ) -> WalkState {
        if self.stop_requested.load(Ordering::Relaxed) {
            return WalkState::Quit;
        }
        if self
            .cancelled
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            self.cancellation_observed.store(true, Ordering::Relaxed);
            return WalkState::Quit;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                self.push_walk_error(&error);
                return self.flush_then(WalkState::Continue);
            }
        };
        let path = entry.path().to_path_buf();
        let Some(root) = self.roots.iter().find(|root| path.starts_with(&root.path)) else {
            self.batch.errors.push(ScanError {
                path: Some(path.clone()),
                message: "parallel traversal yielded a path outside the requested roots"
                    .to_string(),
            });
            self.batch.issues.push(ScanIssue {
                code: ScanIssueCode::TraversalError,
                path: Some(path),
            });
            return self.flush_then(WalkState::Continue);
        };
        let is_directory = entry.file_type().is_some_and(|kind| kind.is_dir());

        if self.ignore.matches(&path, &root.path) {
            self.batch.issues.push(ScanIssue {
                code: ScanIssueCode::IgnoredByConfig,
                path: Some(path),
            });
            let next = if is_directory {
                WalkState::Skip
            } else {
                WalkState::Continue
            };
            return self.flush_then(next);
        }

        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                self.batch.errors.push(ScanError {
                    path: Some(path.clone()),
                    message: error.to_string(),
                });
                self.batch.issues.push(ScanIssue {
                    code: ScanIssueCode::MetadataUnavailable,
                    path: Some(path),
                });
                return self.flush_then(WalkState::Continue);
            }
        };

        if is_directory
            && self.stay_on_filesystem
            && is_cross_filesystem(root.device, device_id_for_evidence(&path, &metadata))
        {
            self.batch.issues.push(ScanIssue {
                code: ScanIssueCode::CrossFilesystemSkipped,
                path: Some(path),
            });
            return self.flush_then(WalkState::Skip);
        }

        let kind = kind_of(&metadata);
        let size_bytes = if kind == EntryKind::File {
            metadata.len()
        } else {
            0
        };
        if kind == EntryKind::File
            && let Some(identity) = file_identity(&path, &metadata)
        {
            self.batch.hardlinks.push(PendingHardlink {
                path: path.clone(),
                identity,
            });
        }
        self.batch.entries.push(ScanEntry {
            path,
            kind,
            size_bytes,
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            rule_hits: vec![],
        });
        self.flush_then(WalkState::Continue)
    }

    fn push_walk_error(&mut self, error: &ParallelWalkError) {
        let path = parallel_walk_error_path(error);
        self.batch.errors.push(ScanError {
            path: path.clone(),
            message: parallel_walk_error_message(error),
        });
        self.batch.issues.push(ScanIssue {
            code: ScanIssueCode::TraversalError,
            path,
        });
    }

    fn flush_if_full(&mut self) -> WalkState {
        let records = self.batch.entries.len() + self.batch.issues.len() + self.batch.errors.len();
        if records < PARALLEL_BATCH_SIZE || self.flush_batch() {
            WalkState::Continue
        } else {
            self.worker_failed.store(true, Ordering::Relaxed);
            WalkState::Quit
        }
    }

    fn flush_then(&mut self, next: WalkState) -> WalkState {
        match self.flush_if_full() {
            WalkState::Quit => WalkState::Quit,
            WalkState::Continue | WalkState::Skip => next,
        }
    }

    fn flush_batch(&mut self) -> bool {
        if self.batch.entries.is_empty()
            && self.batch.hardlinks.is_empty()
            && self.batch.issues.is_empty()
            && self.batch.errors.is_empty()
        {
            return true;
        }
        self.sender.send(std::mem::take(&mut self.batch)).is_ok()
    }
}

fn scan_roots_parallel(
    roots: &[PathBuf],
    options: &ScanOptions,
    ignore: &IgnoreMatcher,
    cancelled: Option<&AtomicBool>,
    progress: &mut ScanProgressTracker<'_>,
) -> Result<ParallelScanBatch> {
    let Some((first_root, additional_roots)) = roots.split_first() else {
        check_cancelled(cancelled)?;
        return Ok(ParallelScanBatch::default());
    };

    let root_contexts = roots
        .iter()
        .map(|root| ParallelScanRoot {
            path: root.clone(),
            device: root_device_for_evidence(root, options),
        })
        .collect::<Vec<_>>();
    let mut walker = WalkBuilder::new(first_root);
    for root in additional_roots {
        walker.add(root);
    }
    walker
        .standard_filters(false)
        .follow_links(false)
        .same_file_system(options.stay_on_filesystem)
        .threads(options.effective_workers());

    // The bounded channel caps queued worker output. Traversal runs in a scoped coordinator
    // thread so this thread can continuously drain batches instead of retaining one tree-sized
    // vector per visitor or waiting for all workers before observing cancellation/progress.
    let (sender, receiver) = mpsc::sync_channel(PARALLEL_BATCH_QUEUE_DEPTH);
    let cancellation_observed = AtomicBool::new(false);
    let stop_requested = AtomicBool::new(false);
    let worker_failed = AtomicBool::new(false);
    let mut merged = ParallelScanBatch::default();
    let mut progress_panic = None;
    let traversal_failed = std::thread::scope(|scope| {
        let traversal = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut visitor_builder = ParallelScanVisitorBuilder {
                    sender,
                    roots: &root_contexts,
                    ignore,
                    cancelled,
                    cancellation_observed: &cancellation_observed,
                    stop_requested: &stop_requested,
                    worker_failed: &worker_failed,
                    stay_on_filesystem: options.stay_on_filesystem,
                };
                walker.build_parallel().visit(&mut visitor_builder);
            }))
            .is_err()
        });

        for mut batch in receiver {
            let discovered = batch.entries.len();
            merged.entries.append(&mut batch.entries);
            merged.hardlinks.append(&mut batch.hardlinks);
            merged.issues.append(&mut batch.issues);
            merged.errors.append(&mut batch.errors);
            merged.worker_failure_diagnostic |= batch.worker_failure_diagnostic;
            if discovered > 0
                && progress_panic.is_none()
                && !cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed))
                && let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    progress.record_parallel_batch(discovered, merged.errors.len());
                }))
            {
                progress_panic = Some(payload);
                stop_requested.store(true, Ordering::Relaxed);
            }
        }

        traversal.join().unwrap_or(true)
    });

    if let Some(payload) = progress_panic {
        std::panic::resume_unwind(payload);
    }

    if cancellation_observed.load(Ordering::Relaxed)
        || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        bail!(ScanCancelled);
    }
    let any_worker_failure = worker_failed.load(Ordering::Relaxed);
    ensure_parallel_worker_failure_diagnostic(&mut merged, traversal_failed, any_worker_failure);
    merged.traversal_completed = !traversal_failed && !any_worker_failure;
    Ok(merged)
}

fn finalize_parallel_entries(
    entries: &mut [ScanEntry],
    mut pending_hardlinks: Vec<PendingHardlink>,
    cancelled: Option<&AtomicBool>,
) -> Result<u64> {
    check_cancelled(cancelled)?;
    entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    pending_hardlinks.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    check_cancelled(cancelled)?;

    let mut hardlinks = HardlinkTracker::default();
    let mut pending_hardlinks = pending_hardlinks.into_iter().peekable();
    let mut bytes_scanned = 0u64;
    for entry_index in 0..entries.len() {
        check_cancelled(cancelled)?;
        let identity = pending_hardlinks
            .next_if(|pending| pending.path == entries[entry_index].path)
            .map(|pending| pending.identity);
        let accounting = {
            let entry = &entries[entry_index];
            if entry.kind == EntryKind::File {
                hardlinks.account_identity(identity, entry.size_bytes, &entry.path, entry_index)
            } else {
                HardlinkAccounting::Count(0)
            }
        };
        let size_bytes = match accounting {
            HardlinkAccounting::Count(size_bytes) => {
                bytes_scanned = bytes_scanned.saturating_add(size_bytes);
                size_bytes
            }
            HardlinkAccounting::Duplicate => 0,
            HardlinkAccounting::Reassign {
                previous_entry_index,
                size_bytes,
            } => {
                entries[previous_entry_index].size_bytes = 0;
                size_bytes
            }
        };
        entries[entry_index].size_bytes = size_bytes;
        check_cancelled(cancelled)?;
    }
    debug_assert!(pending_hardlinks.next().is_none());
    Ok(bytes_scanned)
}

fn sort_report_diagnostics(report: &mut ScanReport) {
    report.errors.sort_unstable_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.message.cmp(&right.message))
    });
    report.issues.sort_unstable_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.code.cmp(&right.code))
    });
}

fn parallel_walk_error_path(error: &ParallelWalkError) -> Option<PathBuf> {
    match error {
        ParallelWalkError::Partial(errors) => errors.iter().find_map(parallel_walk_error_path),
        ParallelWalkError::WithLineNumber { err, .. }
        | ParallelWalkError::WithDepth { err, .. } => parallel_walk_error_path(err),
        ParallelWalkError::WithPath { path, .. } => Some(path.clone()),
        ParallelWalkError::Loop { child, .. } => Some(child.clone()),
        ParallelWalkError::Io(_)
        | ParallelWalkError::Glob { .. }
        | ParallelWalkError::UnrecognizedFileType(_)
        | ParallelWalkError::InvalidDefinition => None,
    }
}

fn parallel_walk_error_message(error: &ParallelWalkError) -> String {
    error
        .io_error()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}

struct SerialScanState<'scan, 'progress> {
    hardlinks: &'scan mut HardlinkTracker,
    report: &'scan mut ScanReport,
    progress: &'scan mut ScanProgressTracker<'progress>,
    budget: &'scan mut ScanBudgetTracker,
}

fn scan_root(
    root: &Path,
    options: &ScanOptions,
    ignore: &IgnoreMatcher,
    cancelled: Option<&AtomicBool>,
    state: &mut SerialScanState<'_, '_>,
) -> Result<()> {
    let SerialScanState {
        hardlinks,
        report,
        progress,
        budget,
    } = state;
    // WalkDir owns boundary enforcement on Unix and Windows. An independent device probe retains
    // the structured path evidence for every mount/volume that WalkDir declines to enter.
    let evidence_root_device = root_device_for_evidence(root, options);

    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .same_file_system(options.stay_on_filesystem)
        .into_iter();
    let mut unpruned_ignored_subtree = None::<PathBuf>;
    while let Some(next) = walker.next() {
        check_cancelled(cancelled)?;
        if budget.check_elapsed() {
            break;
        }
        let entry = match next {
            Ok(entry) => entry,
            Err(err) => {
                if unpruned_ignored_subtree
                    .as_ref()
                    .is_some_and(|ignored| err.path().is_some_and(|path| path.starts_with(ignored)))
                {
                    continue;
                }
                let path = err.path().map(Path::to_path_buf);
                let error = ScanError {
                    path: path.clone(),
                    message: err
                        .io_error()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| err.to_string()),
                };
                record_scan_detail(
                    report,
                    budget,
                    ScanIssue {
                        code: ScanIssueCode::TraversalError,
                        path,
                    },
                    Some(error),
                );
                continue;
            }
        };

        let path = entry.path().to_path_buf();
        if unpruned_ignored_subtree
            .as_ref()
            .is_some_and(|ignored| path.starts_with(ignored))
        {
            continue;
        }
        unpruned_ignored_subtree = None;

        let is_directory = entry.file_type().is_dir();
        let boundary_metadata = if is_directory && evidence_root_device.is_some() {
            match entry.path().symlink_metadata() {
                Ok(metadata) => Some(metadata),
                Err(err) => {
                    let error = ScanError {
                        path: Some(path.clone()),
                        message: err.to_string(),
                    };
                    record_scan_detail(
                        report,
                        budget,
                        ScanIssue {
                            code: ScanIssueCode::MetadataUnavailable,
                            path: Some(path),
                        },
                        Some(error),
                    );
                    continue;
                }
            }
        } else {
            None
        };
        if budget.check_elapsed() {
            break;
        }
        let crosses_filesystem = boundary_metadata.as_ref().is_some_and(|metadata| {
            is_cross_filesystem(
                evidence_root_device,
                device_id_for_evidence(entry.path(), metadata),
            )
        });

        if ignore.matches(&path, root) {
            record_scan_detail(
                report,
                budget,
                ScanIssue {
                    code: ScanIssueCode::IgnoredByConfig,
                    path: Some(path.clone()),
                },
                None,
            );
            if is_directory {
                if !options.stay_on_filesystem
                    || (evidence_root_device.is_some() && !crosses_filesystem)
                {
                    walker.skip_current_dir();
                } else {
                    // WalkDir does not expose whether same_file_system pruned this directory.
                    // Filtering descendants is slower when device identity is unavailable, but it
                    // cannot pop and accidentally skip the parent when this is a mount boundary.
                    unpruned_ignored_subtree = Some(path);
                }
            }
            continue;
        }

        let metadata = match boundary_metadata {
            Some(metadata) => metadata,
            None => match entry.path().symlink_metadata() {
                Ok(metadata) => metadata,
                Err(err) => {
                    let error = ScanError {
                        path: Some(path.clone()),
                        message: err.to_string(),
                    };
                    record_scan_detail(
                        report,
                        budget,
                        ScanIssue {
                            code: ScanIssueCode::MetadataUnavailable,
                            path: Some(path),
                        },
                        Some(error),
                    );
                    continue;
                }
            },
        };
        if budget.check_elapsed() {
            break;
        }

        if crosses_filesystem {
            record_scan_detail(
                report,
                budget,
                ScanIssue {
                    code: ScanIssueCode::CrossFilesystemSkipped,
                    path: Some(path),
                },
                None,
            );
            // WalkDir has already declined to push this directory onto its traversal stack.
            continue;
        }

        let kind = kind_of(&metadata);
        if !budget.retain_entry(&path) {
            break;
        }
        let entry_index = report.entries.len();
        let accounting = if kind == EntryKind::File {
            hardlinks.account(&metadata, &path, entry_index)
        } else {
            HardlinkAccounting::Count(0)
        };
        let (size_bytes, progress_bytes) = match accounting {
            HardlinkAccounting::Count(size_bytes) => (size_bytes, size_bytes),
            HardlinkAccounting::Duplicate => (0, 0),
            HardlinkAccounting::Reassign {
                previous_entry_index,
                size_bytes,
            } => {
                report.entries[previous_entry_index].size_bytes = 0;
                // The bytes were already counted when the previous owner was visited.
                (size_bytes, 0)
            }
        };

        progress.record(&path, progress_bytes, budget.error_count);
        check_cancelled(cancelled)?;

        report.entries.push(ScanEntry {
            path,
            kind,
            size_bytes,
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            rule_hits: vec![],
        });
    }

    Ok(())
}

struct ScanProgressTracker<'a> {
    entries_scanned: usize,
    bytes_scanned: u64,
    on_progress: &'a mut dyn FnMut(ScanProgress),
}

impl<'a> ScanProgressTracker<'a> {
    fn new(on_progress: &'a mut dyn FnMut(ScanProgress)) -> Self {
        Self {
            entries_scanned: 0,
            bytes_scanned: 0,
            on_progress,
        }
    }

    fn record(&mut self, path: &Path, size_bytes: u64, errors: usize) {
        self.entries_scanned += 1;
        self.bytes_scanned = self.bytes_scanned.saturating_add(size_bytes);
        if should_emit_progress(self.entries_scanned) {
            (self.on_progress)(ScanProgress {
                phase: ScanPhase::Scanning,
                entries_total: 0,
                entries_scanned: self.entries_scanned,
                bytes_scanned: self.bytes_scanned,
                errors,
                current_path: Some(path.to_path_buf()),
            });
        }
    }

    fn record_parallel_batch(&mut self, entries: usize, errors: usize) {
        self.entries_scanned = self.entries_scanned.saturating_add(entries);
        (self.on_progress)(ScanProgress {
            phase: ScanPhase::Scanning,
            entries_total: 0,
            entries_scanned: self.entries_scanned,
            bytes_scanned: self.bytes_scanned,
            errors,
            // Worker discovery order is deliberately not exposed as a stale "current" path.
            current_path: None,
        });
    }
}

fn root_device_for_evidence(root: &Path, options: &ScanOptions) -> Option<u64> {
    if options.stay_on_filesystem {
        root.symlink_metadata()
            .ok()
            .and_then(|metadata| device_id_for_evidence(root, &metadata))
    } else {
        None
    }
}

fn is_cross_filesystem(root_device: Option<u64>, entry_device: Option<u64>) -> bool {
    matches!((root_device, entry_device), (Some(root), Some(entry)) if root != entry)
}

fn check_cancelled(cancelled: Option<&AtomicBool>) -> Result<()> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        bail!(ScanCancelled);
    }
    Ok(())
}

fn check_cancelled_periodically(cancelled: Option<&AtomicBool>, index: usize) -> Result<()> {
    if index.is_multiple_of(1024) {
        check_cancelled(cancelled)?;
    }
    Ok(())
}

fn should_emit_progress(entries: usize) -> bool {
    entries <= 16 || entries.is_multiple_of(64)
}

fn aggregate_directory_sizes(
    entries: &mut [ScanEntry],
    cancelled: Option<&AtomicBool>,
) -> Result<()> {
    check_cancelled(cancelled)?;
    // Build parent links while paths are borrowed, then release the index before mutating entries.
    // Counting direct children gives us a traversal-order-independent leaf-to-root work queue, so
    // every directory receives each fully aggregated child exactly once without sorting by depth.
    let parent_indices = {
        let mut by_path = HashMap::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            check_cancelled_periodically(cancelled, idx)?;
            by_path.insert(entry.path.as_path(), idx);
        }
        let mut parents = Vec::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            check_cancelled_periodically(cancelled, idx)?;
            parents.push(
                entry
                    .path
                    .parent()
                    .and_then(|parent| by_path.get(parent).copied()),
            );
        }
        parents
    };

    let mut remaining_children = vec![0usize; entries.len()];
    for (idx, parent_idx) in parent_indices.iter().enumerate() {
        check_cancelled_periodically(cancelled, idx)?;
        if let Some(parent_idx) = parent_idx {
            remaining_children[*parent_idx] += 1;
        }
    }
    let mut ready = Vec::with_capacity(entries.len());
    for (idx, children) in remaining_children.iter().enumerate() {
        check_cancelled_periodically(cancelled, idx)?;
        if *children == 0 {
            ready.push(idx);
        }
    }

    let mut processed = 0usize;
    while let Some(idx) = ready.pop() {
        check_cancelled_periodically(cancelled, processed)?;
        processed += 1;
        let Some(parent_idx) = parent_indices[idx] else {
            continue;
        };
        let size = entries[idx].size_bytes;
        entries[parent_idx].size_bytes = entries[parent_idx].size_bytes.saturating_add(size);
        remaining_children[parent_idx] -= 1;
        if remaining_children[parent_idx] == 0 {
            ready.push(parent_idx);
        }
    }
    check_cancelled(cancelled)?;
    debug_assert_eq!(processed, entries.len());
    Ok(())
}

fn total_size_for_roots(entries: &[ScanEntry], roots: &[PathBuf]) -> u64 {
    entries
        .iter()
        .filter(|entry| roots.iter().any(|root| &entry.path == root))
        .map(|entry| entry.size_bytes)
        .fold(0u64, u64::saturating_add)
}

fn kind_of(metadata: &Metadata) -> EntryKind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

struct IgnoreMatcher {
    dirs: Vec<PathBuf>,
    patterns: GlobSet,
}

impl IgnoreMatcher {
    fn new(options: &ScanOptions) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        for pattern in &options.ignore_patterns {
            builder.add(
                Glob::new(pattern)
                    .with_context(|| format!("invalid scan ignore pattern: {pattern}"))?,
            );
        }
        Ok(Self {
            dirs: options
                .ignore_dirs
                .iter()
                .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
                .collect(),
            patterns: builder.build()?,
        })
    }

    fn matches(&self, path: &Path, root: &Path) -> bool {
        if self
            .dirs
            .iter()
            .any(|ignored| path == ignored || path.starts_with(ignored))
        {
            return true;
        }
        self.patterns.is_match(path)
            || path
                .strip_prefix(root)
                .is_ok_and(|relative| self.patterns.is_match(relative))
    }
}

fn normalize_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    for root in &mut roots {
        if let Ok(canonical) = root.canonicalize() {
            *root = canonical;
        }
    }
    roots.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
    let mut normalized = Vec::<PathBuf>::new();
    for root in roots {
        if normalized.iter().any(|parent| root.starts_with(parent)) {
            continue;
        }
        normalized.push(root);
    }
    normalized
}

fn wants(kinds: &[GlobalScanKind], kind: GlobalScanKind) -> bool {
    kinds.contains(&kind)
}

fn push_global_root(
    roots: &mut Vec<GlobalScanRoot>,
    path: &Path,
    kind: GlobalScanKind,
    label: impl Into<String>,
) {
    roots.push(GlobalScanRoot {
        path: path.to_path_buf(),
        kind,
        label: label.into(),
    });
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn push_relative_global_roots(
    roots: &mut Vec<GlobalScanRoot>,
    base: &Path,
    kind: GlobalScanKind,
    targets: &[(&str, &str)],
) {
    for (relative_path, label) in targets {
        push_global_root(roots, &base.join(relative_path), kind, *label);
    }
}

fn push_developer_cache_roots(
    environment: &GlobalScanEnvironment,
    roots: &mut Vec<GlobalScanRoot>,
) {
    if let Some(home) = &environment.home_dir {
        for (path, label) in [
            (home.join(".cargo").join("registry"), "Cargo registry cache"),
            (home.join(".cargo").join("git"), "Cargo Git cache"),
            (home.join(".npm"), "npm cache"),
            (home.join(".cache").join("pnpm"), "pnpm cache"),
            (home.join(".cache").join("yarn"), "Yarn cache"),
            (home.join(".cache").join("pip"), "pip cache"),
            (home.join(".cache").join("uv"), "uv cache"),
            (
                home.join(".local").join("share").join("pnpm").join("store"),
                "pnpm store",
            ),
            (home.join(".gradle").join("caches"), "Gradle cache"),
            (home.join(".m2").join("repository"), "Maven repository"),
            (home.join("go").join("pkg").join("mod"), "Go module cache"),
        ] {
            push_global_root(roots, &path, GlobalScanKind::DeveloperCaches, label);
        }

        #[cfg(target_os = "macos")]
        push_relative_global_roots(
            roots,
            home,
            GlobalScanKind::DeveloperCaches,
            &[
                ("Library/Caches/pip", "pip cache"),
                ("Library/Caches/uv", "uv cache"),
                ("Library/Caches/Yarn", "Yarn cache"),
                ("Library/pnpm/store", "pnpm store"),
                ("Library/Caches/Homebrew", "Homebrew download cache"),
                ("Library/Caches/CocoaPods", "CocoaPods cache"),
                ("Library/Caches/org.swift.swiftpm", "SwiftPM cache"),
                ("Library/Caches/go-build", "Go build cache"),
                ("Library/Caches/deno", "Deno cache"),
                ("Library/Caches/Cypress", "Cypress binary cache"),
                ("Library/Caches/composer", "Composer cache"),
                (".bun/install/cache", "Bun cache"),
                (".pub-cache", "Dart and Flutter pub cache"),
                (".yarn/cache", "Yarn cache"),
                ("Library/Developer/Xcode/DerivedData", "Xcode DerivedData"),
                (
                    "Library/Developer/CoreSimulator/Caches",
                    "CoreSimulator caches",
                ),
                ("Library/Caches/com.apple.dt.Xcode", "Xcode cache"),
                (
                    "Library/Developer/Xcode/iOS DeviceSupport",
                    "Xcode iOS device support",
                ),
                (
                    "Library/Developer/Xcode/watchOS DeviceSupport",
                    "Xcode watchOS device support",
                ),
                (
                    "Library/Developer/Xcode/tvOS DeviceSupport",
                    "Xcode tvOS device support",
                ),
                ("Library/Developer/Xcode/Archives", "Xcode archives"),
                (
                    "Library/Developer/Xcode/UserData/Previews",
                    "Xcode previews",
                ),
                ("Library/Developer/XCTestDevices", "XCTest devices"),
            ],
        );
    }

    if let Some(cache) = &environment.cache_dir {
        for (path, label) in [
            (cache.join("npm"), "npm cache"),
            (cache.join("pnpm"), "pnpm cache"),
            (cache.join("yarn"), "Yarn cache"),
            (cache.join("pip"), "pip cache"),
            (cache.join("uv"), "uv cache"),
        ] {
            push_global_root(roots, &path, GlobalScanKind::DeveloperCaches, label);
        }
    }

    #[cfg(target_os = "windows")]
    if let Some(local) = &environment.data_local_dir {
        for (path, label) in [
            (local.join("npm-cache"), "npm cache"),
            (local.join("Yarn").join("Cache"), "Yarn cache"),
            (local.join("pip").join("Cache"), "pip cache"),
            (local.join("uv").join("cache"), "uv cache"),
        ] {
            push_global_root(roots, &path, GlobalScanKind::DeveloperCaches, label);
        }
    }
}

fn push_browser_cache_roots(environment: &GlobalScanEnvironment, roots: &mut Vec<GlobalScanRoot>) {
    if let Some(home) = &environment.home_dir {
        #[cfg(target_os = "macos")]
        push_relative_global_roots(
            roots,
            home,
            GlobalScanKind::BrowserCaches,
            &[
                ("Library/Caches/Google/Chrome", "Chrome cache"),
                ("Library/Caches/Chromium", "Chromium cache"),
                ("Library/Caches/Microsoft Edge", "Microsoft Edge cache"),
                ("Library/Caches/Firefox", "Firefox cache"),
                ("Library/Caches/BraveSoftware/Brave-Browser", "Brave cache"),
                ("Library/Caches/Arc", "Arc cache"),
                ("Library/Caches/com.apple.Safari", "Safari cache"),
            ],
        );

        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        for (path, label) in [
            (home.join(".cache").join("google-chrome"), "Chrome cache"),
            (home.join(".cache").join("chromium"), "Chromium cache"),
            (
                home.join(".cache").join("microsoft-edge"),
                "Microsoft Edge cache",
            ),
            (
                home.join(".cache").join("mozilla").join("firefox"),
                "Firefox cache",
            ),
        ] {
            push_global_root(roots, &path, GlobalScanKind::BrowserCaches, label);
        }
    }

    #[cfg(target_os = "windows")]
    if let Some(local) = &environment.data_local_dir {
        for (path, label) in [
            (
                local
                    .join("Google")
                    .join("Chrome")
                    .join("User Data")
                    .join("Default")
                    .join("Cache"),
                "Chrome cache",
            ),
            (
                local
                    .join("Microsoft")
                    .join("Edge")
                    .join("User Data")
                    .join("Default")
                    .join("Cache"),
                "Microsoft Edge cache",
            ),
            (
                local.join("Mozilla").join("Firefox").join("Profiles"),
                "Firefox cache",
            ),
        ] {
            push_global_root(roots, &path, GlobalScanKind::BrowserCaches, label);
        }
    }
}

fn push_app_cache_roots(environment: &GlobalScanEnvironment, roots: &mut Vec<GlobalScanRoot>) {
    #[cfg(not(target_os = "windows"))]
    if let Some(cache) = &environment.cache_dir {
        push_global_root(
            roots,
            cache,
            GlobalScanKind::AppCaches,
            "Application caches",
        );
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = &environment.home_dir {
        push_global_root(
            roots,
            &home.join("Library").join("Caches"),
            GlobalScanKind::AppCaches,
            "macOS application caches",
        );

        push_relative_global_roots(
            roots,
            home,
            GlobalScanKind::AppCaches,
            &[
                ("Library/Application Support/Slack/Cache", "Slack cache"),
                (
                    "Library/Application Support/Slack/Code Cache",
                    "Slack code cache",
                ),
                (
                    "Library/Application Support/Slack/GPUCache",
                    "Slack GPU cache",
                ),
                ("Library/Application Support/discord/Cache", "Discord cache"),
                (
                    "Library/Application Support/discord/Code Cache",
                    "Discord code cache",
                ),
                (
                    "Library/Application Support/discord/GPUCache",
                    "Discord GPU cache",
                ),
                ("Library/Application Support/Code/Cache", "VS Code cache"),
                (
                    "Library/Application Support/Code/Code Cache",
                    "VS Code code cache",
                ),
                (
                    "Library/Application Support/Code/GPUCache",
                    "VS Code GPU cache",
                ),
                (
                    "Library/Application Support/Code/CachedData",
                    "VS Code cached data",
                ),
                ("Library/Application Support/Cursor/Cache", "Cursor cache"),
                (
                    "Library/Application Support/Cursor/Code Cache",
                    "Cursor code cache",
                ),
                (
                    "Library/Application Support/Cursor/GPUCache",
                    "Cursor GPU cache",
                ),
                (
                    "Library/Application Support/Cursor/CachedData",
                    "Cursor cached data",
                ),
                ("Library/Application Support/Signal/Cache", "Signal cache"),
                (
                    "Library/Application Support/Signal/Code Cache",
                    "Signal code cache",
                ),
                (
                    "Library/Application Support/Signal/GPUCache",
                    "Signal GPU cache",
                ),
                (
                    "Library/Application Support/obsidian/Cache",
                    "Obsidian cache",
                ),
                (
                    "Library/Application Support/obsidian/Code Cache",
                    "Obsidian code cache",
                ),
                (
                    "Library/Application Support/obsidian/GPUCache",
                    "Obsidian GPU cache",
                ),
                ("Library/Application Support/Notion/Cache", "Notion cache"),
                (
                    "Library/Application Support/Notion/Code Cache",
                    "Notion code cache",
                ),
                (
                    "Library/Application Support/Notion/GPUCache",
                    "Notion GPU cache",
                ),
                (
                    "Library/Application Support/Spotify/PersistentCache",
                    "Spotify persistent cache",
                ),
                (
                    "Library/Containers/com.microsoft.teams2/Data/Library/Caches",
                    "Microsoft Teams cache",
                ),
                (
                    "Library/Application Support/zoom.us/AutoUpdater",
                    "Zoom update installers",
                ),
            ],
        );
    }

    #[cfg(target_os = "windows")]
    push_windows_app_cache_roots(environment, roots);
}

#[cfg(any(target_os = "windows", test))]
fn push_windows_app_cache_roots(
    environment: &GlobalScanEnvironment,
    roots: &mut Vec<GlobalScanRoot>,
) {
    if let Some(local) = &environment.data_local_dir {
        push_relative_global_roots(
            roots,
            local,
            GlobalScanKind::AppCaches,
            &[("D3DSCache", "Windows DirectX compiled shader cache files")],
        );
    }
}

fn push_log_roots(environment: &GlobalScanEnvironment, roots: &mut Vec<GlobalScanRoot>) {
    #[cfg(target_os = "macos")]
    if let Some(home) = &environment.home_dir {
        push_global_root(
            roots,
            &home.join("Library").join("Logs"),
            GlobalScanKind::Logs,
            "macOS user logs",
        );
        push_global_root(
            roots,
            &home.join("Library").join("DiagnosticReports"),
            GlobalScanKind::Logs,
            "Legacy macOS diagnostic reports",
        );
    }

    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    if let Some(home) = &environment.home_dir {
        push_global_root(
            roots,
            &home.join(".local").join("state"),
            GlobalScanKind::Logs,
            "User state and logs",
        );
    }

    #[cfg(not(all(unix, not(target_os = "ios"), not(target_os = "android"))))]
    let _ = (environment, roots);
}

fn normalize_global_roots(
    roots: Vec<GlobalScanRoot>,
    environment: &GlobalScanEnvironment,
) -> Vec<GlobalScanRoot> {
    let roots = normalize_global_locations(roots, environment);
    let mut normalized = Vec::<GlobalScanRoot>::new();
    for root in roots {
        if normalized
            .iter()
            .any(|parent| root.path == parent.path || root.path.starts_with(&parent.path))
        {
            continue;
        }
        normalized.push(root);
    }
    normalized
}

fn normalize_global_locations(
    mut roots: Vec<GlobalScanRoot>,
    environment: &GlobalScanEnvironment,
) -> Vec<GlobalScanRoot> {
    for root in &mut roots {
        if let Ok(canonical) = root.path.canonicalize() {
            root.path = canonical;
        }
    }
    roots.retain(|root| root.path.exists() && allows_global_root(&root.path, environment));
    roots.sort_by(|a, b| {
        a.path
            .components()
            .count()
            .cmp(&b.path.components().count())
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.label.cmp(&b.label))
    });
    roots.dedup_by(|left, right| {
        left.path == right.path && left.kind == right.kind && left.label == right.label
    });
    roots
}

fn allows_global_root(path: &Path, environment: &GlobalScanEnvironment) -> bool {
    !is_root_path(path)
        && environment
            .home_dir
            .as_ref()
            .is_none_or(|home| home != path)
        && environment
            .data_dir
            .as_ref()
            .is_none_or(|data| data != path)
}

fn is_root_path(path: &Path) -> bool {
    path.is_absolute() && path.parent().is_none()
}

#[must_use]
pub fn developer_cache_roots() -> Vec<PathBuf> {
    discover_global_scan_roots(
        &[GlobalScanKind::DeveloperCaches],
        &GlobalScanEnvironment::current(),
    )
    .into_iter()
    .map(|root| root.path)
    .collect()
}

#[derive(Default)]
struct HardlinkTracker {
    owners: HashMap<FileIdentity, HardlinkOwner>,
}

impl HardlinkTracker {
    fn account(
        &mut self,
        metadata: &Metadata,
        path: &Path,
        entry_index: usize,
    ) -> HardlinkAccounting {
        self.account_identity(
            file_identity(path, metadata),
            metadata.len(),
            path,
            entry_index,
        )
    }

    fn account_identity(
        &mut self,
        identity: Option<FileIdentity>,
        size_bytes: u64,
        path: &Path,
        entry_index: usize,
    ) -> HardlinkAccounting {
        let Some(identity) = identity else {
            return HardlinkAccounting::Count(size_bytes);
        };

        match self.owners.entry(identity) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(HardlinkOwner {
                    path: path.to_path_buf(),
                    entry_index,
                    size_bytes,
                });
                HardlinkAccounting::Count(size_bytes)
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let owner = entry.get_mut();
                if path < owner.path.as_path() {
                    let previous_entry_index = owner.entry_index;
                    let size_bytes = owner.size_bytes;
                    owner.path = path.to_path_buf();
                    owner.entry_index = entry_index;
                    HardlinkAccounting::Reassign {
                        previous_entry_index,
                        size_bytes,
                    }
                } else {
                    HardlinkAccounting::Duplicate
                }
            }
        }
    }
}

struct HardlinkOwner {
    path: PathBuf,
    entry_index: usize,
    size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HardlinkAccounting {
    Count(u64),
    Duplicate,
    Reassign {
        previous_entry_index: usize,
        size_bytes: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(_path: &Path, metadata: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    hardlink_identity(metadata.dev(), metadata.ino(), metadata.nlink())
}

#[cfg(windows)]
fn file_identity(path: &Path, _metadata: &Metadata) -> Option<FileIdentity> {
    let information = windows_file_information(path)?;
    hardlink_identity(
        information.volume_serial_number(),
        information.file_index(),
        information.number_of_links(),
    )
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_path: &Path, _metadata: &Metadata) -> Option<FileIdentity> {
    None
}

fn hardlink_identity(device: u64, inode: u64, number_of_links: u64) -> Option<FileIdentity> {
    (number_of_links > 1).then_some(FileIdentity { device, inode })
}

#[cfg(unix)]
fn device_id_for_evidence(_path: &Path, metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(windows)]
fn device_id_for_evidence(path: &Path, _metadata: &Metadata) -> Option<u64> {
    windows_file_information(path).map(|information| information.volume_serial_number())
}

#[cfg(windows)]
fn windows_file_information(path: &Path) -> Option<winapi_util::file::Information> {
    let handle = winapi_util::Handle::from_path_any(path).ok()?;
    // `information` consumes the owned handle. Its returned value contains copied fields, so the
    // underlying OS handle is closed before this helper returns.
    winapi_util::file::information(handle).ok()
}

#[cfg(not(any(unix, windows)))]
fn device_id_for_evidence(_path: &Path, _metadata: &Metadata) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scan_entry(path: PathBuf, kind: EntryKind, size_bytes: u64) -> ScanEntry {
        ScanEntry {
            path,
            kind,
            size_bytes,
            modified_at: None,
            rule_hits: Vec::new(),
        }
    }

    fn assert_reports_equivalent(left: &ScanReport, right: &ScanReport) {
        assert_eq!(left.entries, right.entries);
        assert_eq!(left.summary, right.summary);
        assert_eq!(left.issues, right.issues);
        assert_eq!(left.errors, right.errors);
        assert_eq!(left.completeness(), right.completeness());
    }

    #[test]
    fn scan_worker_count_defaults_to_serial_and_clamps_to_four() {
        assert_eq!(ScanOptions::default().effective_workers(), 1);
        assert_eq!(
            ScanOptions {
                workers: 0,
                ..ScanOptions::default()
            }
            .effective_workers(),
            1
        );
        assert_eq!(
            ScanOptions {
                workers: usize::MAX,
                ..ScanOptions::default()
            }
            .effective_workers(),
            MAX_SCAN_WORKERS
        );
    }

    #[test]
    fn scan_does_not_follow_symlinks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join("cache")).expect("mkdir");
        fs::write(root.join("cache").join("file"), b"1234").expect("write");

        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("cache"), root.join("cache-link")).expect("symlink");

        let report = scan_paths(&[root.to_path_buf()], &ScanOptions::default()).expect("scan");
        let link = report
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("cache-link"));

        #[cfg(unix)]
        assert_eq!(link.map(|entry| entry.kind), Some(EntryKind::Symlink));
        assert!(
            !report
                .entries
                .iter()
                .any(|entry| entry.path.ends_with("cache-link/file"))
        );
    }

    #[test]
    fn directory_sizes_include_nested_descendants() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let nested = target.join("nested");
        fs::create_dir_all(&nested).expect("mkdir");
        fs::write(target.join("direct"), vec![0; 5]).expect("write direct child");
        fs::write(nested.join("artifact"), vec![0; 12]).expect("write nested child");

        let report =
            scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
        let target = report
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("target"))
            .expect("target entry");
        let nested = report
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("target/nested"))
            .expect("nested entry");
        let root = report
            .entries
            .iter()
            .find(|entry| report.summary.roots.contains(&entry.path))
            .expect("root entry");

        assert_eq!(nested.size_bytes, 12);
        assert_eq!(target.size_bytes, 17);
        assert_eq!(root.size_bytes, 17);
        assert_eq!(report.summary.total_size_bytes, 17);
    }

    #[test]
    fn directory_size_aggregation_is_independent_of_entry_and_root_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first_root = temp.path().join("first-root");
        let nested = first_root.join("nested");
        let second_root = temp.path().join("second-root");
        let mut parent_first = vec![
            scan_entry(first_root.clone(), EntryKind::Directory, 0),
            scan_entry(nested.clone(), EntryKind::Directory, 0),
            scan_entry(nested.join("artifact"), EntryKind::File, 5),
            scan_entry(second_root.clone(), EntryKind::Directory, 0),
            scan_entry(second_root.join("download"), EntryKind::File, 7),
        ];
        let mut child_first = parent_first.iter().cloned().rev().collect::<Vec<_>>();

        aggregate_directory_sizes(&mut parent_first, None).expect("aggregate parent-first entries");
        aggregate_directory_sizes(&mut child_first, None).expect("aggregate child-first entries");

        let sorted_sizes = |entries: &[ScanEntry]| {
            let mut sizes = entries
                .iter()
                .map(|entry| (entry.path.clone(), entry.size_bytes))
                .collect::<Vec<_>>();
            sizes.sort_by(|left, right| left.0.cmp(&right.0));
            sizes
        };
        assert_eq!(sorted_sizes(&parent_first), sorted_sizes(&child_first));
        assert_eq!(
            parent_first
                .iter()
                .find(|entry| entry.path == first_root)
                .expect("first root")
                .size_bytes,
            5
        );
        assert_eq!(
            parent_first
                .iter()
                .find(|entry| entry.path == second_root)
                .expect("second root")
                .size_bytes,
            7
        );
    }

    #[test]
    fn root_size_summary_saturates_on_overflow() {
        let first_root = PathBuf::from("/first");
        let second_root = PathBuf::from("/second");
        let entries = vec![
            scan_entry(first_root.clone(), EntryKind::Directory, u64::MAX),
            scan_entry(second_root.clone(), EntryKind::Directory, 1),
        ];

        assert_eq!(
            total_size_for_roots(&entries, &[first_root, second_root]),
            u64::MAX
        );
    }

    #[test]
    fn hardlink_accounting_reassigns_to_the_lexical_owner() {
        let identity = FileIdentity {
            device: 7,
            inode: 42,
        };
        assert_eq!(hardlink_identity(7, 42, 1), None);
        assert_eq!(hardlink_identity(7, 42, 2), Some(identity));
        let mut tracker = HardlinkTracker::default();

        assert_eq!(
            tracker.account_identity(Some(identity), 6, Path::new("z-owner"), 0),
            HardlinkAccounting::Count(6)
        );
        assert_eq!(
            tracker.account_identity(Some(identity), 6, Path::new("a-owner"), 1),
            HardlinkAccounting::Reassign {
                previous_entry_index: 0,
                size_bytes: 6,
            }
        );
        assert_eq!(
            tracker.account_identity(Some(identity), 6, Path::new("m-duplicate"), 2),
            HardlinkAccounting::Duplicate
        );
        assert_eq!(
            tracker.account_identity(None, 9, Path::new("ordinary-file"), 3),
            HardlinkAccounting::Count(9)
        );
    }

    #[test]
    fn multiple_roots_keep_sizes_when_input_order_changes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).expect("first root");
        fs::create_dir(&second).expect("second root");
        fs::write(first.join("artifact"), b"12345").expect("first file");
        fs::write(second.join("artifact"), b"1234567").expect("second file");

        let reversed = scan_paths(&[second.clone(), first.clone()], &ScanOptions::default())
            .expect("reversed root scan");
        let forward = scan_paths(&[first, second], &ScanOptions::default()).expect("forward scan");
        let root_sizes = |report: &ScanReport| {
            report
                .summary
                .roots
                .iter()
                .map(|root| {
                    report
                        .entries
                        .iter()
                        .find(|entry| &entry.path == root)
                        .expect("root entry")
                        .size_bytes
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(root_sizes(&reversed), vec![5, 7]);
        assert_eq!(root_sizes(&reversed), root_sizes(&forward));
        assert_eq!(reversed.summary.total_size_bytes, 12);
        assert_eq!(reversed.summary, forward.summary);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn parallel_scan_matches_serial_for_roots_ignores_hardlinks_and_filesystem_policy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first_root = temp.path().join("first-root");
        let second_root = temp.path().join("second-root");
        let nested = first_root.join("nested");
        let ignored_dir = first_root.join("ignored-dir");
        let pattern_ignored = second_root.join("pattern-ignored");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::create_dir_all(&ignored_dir).expect("ignored directory");
        fs::create_dir_all(&pattern_ignored).expect("pattern ignored directory");
        fs::write(nested.join("artifact"), b"123456").expect("write artifact");
        fs::write(ignored_dir.join("secret"), b"ignored").expect("write ignored file");
        fs::write(pattern_ignored.join("secret"), b"ignored").expect("write pattern ignored file");
        fs::write(second_root.join("visible"), b"visible").expect("write visible file");
        fs::hard_link(nested.join("artifact"), second_root.join("artifact-link"))
            .expect("hard link");
        let paths = vec![second_root, first_root];
        let base = ScanOptions {
            workers: 1,
            stay_on_filesystem: true,
            ignore_dirs: vec![ignored_dir],
            ignore_patterns: vec![
                "**/pattern-ignored".to_string(),
                "**/pattern-ignored/**".to_string(),
            ],
            budgets: ScanBudgetLimits::default(),
        };

        let serial = scan_paths(&paths, &base).expect("serial scan");
        let parallel = scan_paths(
            &paths,
            &ScanOptions {
                workers: MAX_SCAN_WORKERS,
                ..base
            },
        )
        .expect("parallel scan");

        assert_reports_equivalent(&serial, &parallel);
        assert!(serial.issues.iter().any(|issue| {
            issue.code == ScanIssueCode::IgnoredByConfig
                && issue
                    .path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("ignored-dir"))
        }));
        assert!(serial.issues.iter().any(|issue| {
            issue.code == ScanIssueCode::IgnoredByConfig
                && issue
                    .path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("pattern-ignored"))
        }));
    }

    #[test]
    fn parallel_and_serial_reports_match_when_a_root_is_unavailable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let available = temp.path().join("available");
        let unavailable = temp.path().join("unavailable");
        fs::create_dir(&available).expect("available root");
        fs::write(available.join("artifact"), b"1234").expect("write artifact");
        let paths = vec![unavailable, available];

        let serial = scan_paths(&paths, &ScanOptions::default()).expect("serial scan");
        let parallel = scan_paths(
            &paths,
            &ScanOptions {
                workers: 3,
                ..ScanOptions::default()
            },
        )
        .expect("parallel scan");

        assert_reports_equivalent(&serial, &parallel);
        assert_eq!(parallel.completeness(), ReportIntegrity::Partial);
    }

    #[test]
    fn parallel_and_serial_match_ignored_files_across_bounded_batches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ignored_files = PARALLEL_BATCH_SIZE * 3;
        for index in 0..ignored_files {
            fs::write(temp.path().join(format!("artifact-{index:04}.tmp")), b"x")
                .expect("write ignored artifact");
        }
        fs::write(temp.path().join("visible"), b"visible").expect("write visible artifact");
        let paths = [temp.path().to_path_buf()];
        let serial_options = ScanOptions {
            ignore_patterns: vec!["**/*.tmp".to_string()],
            ..ScanOptions::default()
        };

        let serial = scan_paths(&paths, &serial_options).expect("serial scan");
        let parallel = scan_paths(
            &paths,
            &ScanOptions {
                workers: MAX_SCAN_WORKERS,
                ..serial_options
            },
        )
        .expect("parallel scan");

        assert_reports_equivalent(&serial, &parallel);
        assert_eq!(parallel.issues.len(), ignored_files);
        assert!(
            parallel
                .issues
                .iter()
                .all(|issue| issue.code == ScanIssueCode::IgnoredByConfig)
        );
    }

    #[test]
    fn scan_captures_a_single_reference_time_at_start() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("entry"), b"content").expect("write");
        let before = Utc::now();

        let report =
            scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");

        let after = Utc::now();
        assert!(report.as_of >= before);
        assert!(report.as_of <= after);
    }

    #[test]
    fn progress_scans_each_entry_once_and_finishes_with_total() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("cache")).expect("mkdir");
        fs::write(temp.path().join("cache").join("one"), b"1234").expect("write");
        fs::write(temp.path().join("two"), b"12").expect("write");

        let mut progress = Vec::new();
        let report = scan_paths_with_progress(
            &[temp.path().to_path_buf()],
            &ScanOptions::default(),
            |event| progress.push(event),
        )
        .expect("scan");

        let scanned = progress
            .iter()
            .rev()
            .find(|event| event.phase == ScanPhase::Scanning)
            .expect("scan progress");
        let aggregated = progress
            .iter()
            .rev()
            .find(|event| event.phase == ScanPhase::Aggregating)
            .expect("aggregation progress");

        assert_eq!(scanned.entries_total, 0);
        assert_eq!(scanned.entries_scanned, report.entries.len());
        assert_eq!(aggregated.entries_total, report.entries.len());
    }

    #[test]
    fn parallel_progress_is_batched_monotonic_and_hides_worker_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..2048 {
            fs::write(temp.path().join(format!("artifact-{index:04}")), b"x")
                .expect("write artifact");
        }
        let mut events = Vec::new();

        let report = scan_paths_with_progress(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                workers: MAX_SCAN_WORKERS,
                ..ScanOptions::default()
            },
            |event| events.push(event),
        )
        .expect("parallel scan");
        let scanning = events
            .iter()
            .filter(|event| event.phase == ScanPhase::Scanning)
            .collect::<Vec<_>>();

        assert!(scanning.len() > 1);
        assert!(scanning.iter().all(|event| event.current_path.is_none()));
        assert!(
            scanning
                .first()
                .is_some_and(|event| event.entries_scanned <= PARALLEL_BATCH_SIZE)
        );
        assert!(scanning.windows(2).all(|events| {
            events[0].entries_scanned < events[1].entries_scanned
                && events[0].bytes_scanned <= events[1].bytes_scanned
                && events[1].entries_scanned - events[0].entries_scanned <= PARALLEL_BATCH_SIZE
        }));
        assert_eq!(
            scanning.last().map(|event| event.entries_scanned),
            Some(report.entries.len())
        );
        assert_eq!(
            events.last().map(|event| event.phase),
            Some(ScanPhase::Aggregating)
        );
        assert_eq!(
            events.last().map(|event| event.entries_total),
            Some(report.entries.len())
        );
    }

    #[test]
    fn ignore_patterns_skip_matching_subtrees() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".git")).expect("mkdir");
        fs::write(temp.path().join(".git").join("objects"), b"hidden").expect("write");
        fs::write(temp.path().join("visible"), b"visible").expect("write");

        let report = scan_paths(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                ignore_patterns: vec!["**/.git".into(), "**/.git/**".into()],
                ..ScanOptions::default()
            },
        )
        .expect("scan");

        assert!(
            !report
                .entries
                .iter()
                .any(|entry| entry.path.ends_with(".git"))
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.path.ends_with("visible"))
        );
        assert_eq!(report.completeness(), ReportIntegrity::Complete);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ScanIssueCode::IgnoredByConfig
                && issue
                    .path
                    .as_deref()
                    .is_some_and(|path| path.ends_with(".git"))
        }));
    }

    #[test]
    fn unavailable_root_records_a_traversal_issue_and_partial_report() {
        let temp = tempfile::tempdir().expect("tempdir");
        let unavailable = temp.path().join("does-not-exist");
        let available = temp.path().join("available");
        fs::create_dir(&available).expect("available root");
        fs::write(available.join("artifact"), b"1234").expect("available file");

        let report = scan_paths(
            &[unavailable.clone(), available.clone()],
            &ScanOptions::default(),
        )
        .expect("scan report");

        assert_eq!(report.completeness(), ReportIntegrity::Partial);
        assert_eq!(report.errors.len(), 1);
        let available = available.canonicalize().expect("canonical available root");
        assert_eq!(
            report
                .entries
                .iter()
                .find(|entry| entry.path == available)
                .expect("available root entry")
                .size_bytes,
            4
        );
        assert_eq!(report.summary.total_size_bytes, 4);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ScanIssueCode::TraversalError
                && issue.path.as_deref() == Some(unavailable.as_path())
        }));
    }

    #[test]
    fn report_completeness_delegates_to_the_fail_closed_core_policy() {
        assert!(ScanIssueCode::TraversalError.makes_report_partial());
        assert!(ScanIssueCode::MetadataUnavailable.makes_report_partial());
        assert!(ScanIssueCode::PermissionDenied.makes_report_partial());
        assert!(ScanIssueCode::RootUnavailable.makes_report_partial());
        assert!(ScanIssueCode::Unknown.makes_report_partial());
        assert!(!ScanIssueCode::IgnoredByConfig.makes_report_partial());
        assert!(!ScanIssueCode::CrossFilesystemSkipped.makes_report_partial());

        let report = ScanReport {
            issues: vec![ScanIssue {
                code: ScanIssueCode::Unknown,
                path: None,
            }],
            ..ScanReport::default()
        };
        assert_eq!(report.completeness(), ReportIntegrity::Partial);
    }

    #[test]
    fn cancellable_scan_stops_before_completion() {
        use std::sync::atomic::AtomicBool;

        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..128 {
            fs::write(temp.path().join(format!("file-{index}")), b"x").expect("write");
        }
        let cancelled = AtomicBool::new(false);
        let result = scan_paths_with_progress_cancellable(
            &[temp.path().to_path_buf()],
            &ScanOptions::default(),
            &cancelled,
            |event| {
                if event.entries_scanned >= 4 {
                    cancelled.store(true, Ordering::Relaxed);
                }
            },
        );

        let error = result.expect_err("scan should be cancelled");
        assert_eq!(error.to_string(), SCAN_CANCELLED);
        assert!(is_scan_cancelled(&error));
    }

    #[test]
    fn cancellable_scan_stops_when_aggregation_begins() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("cache")).expect("cache directory");
        fs::write(temp.path().join("cache").join("artifact"), b"content").expect("write artifact");
        let cancelled = AtomicBool::new(false);
        let mut saw_aggregation = false;

        let result = scan_paths_with_progress_cancellable(
            &[temp.path().to_path_buf()],
            &ScanOptions::default(),
            &cancelled,
            |event| {
                if event.phase == ScanPhase::Aggregating {
                    saw_aggregation = true;
                    cancelled.store(true, Ordering::Relaxed);
                }
            },
        );

        assert!(saw_aggregation);
        let error = result.expect_err("aggregation should cancel");
        assert_eq!(error.to_string(), SCAN_CANCELLED);
        assert!(is_scan_cancelled(&error));
    }

    #[test]
    fn parallel_wide_directory_cancellation_discards_results_and_stops_progress() {
        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..4096 {
            fs::write(temp.path().join(format!("artifact-{index:04}")), b"x")
                .expect("write artifact");
        }
        let options = ScanOptions {
            workers: MAX_SCAN_WORKERS,
            ..ScanOptions::default()
        };
        let cancelled = AtomicBool::new(true);
        let mut pre_cancel_events = Vec::new();

        let error = scan_paths_with_progress_cancellable(
            &[temp.path().to_path_buf()],
            &options,
            &cancelled,
            |event| pre_cancel_events.push(event),
        )
        .expect_err("pre-cancelled parallel scan");
        assert_eq!(error.to_string(), SCAN_CANCELLED);
        assert!(is_scan_cancelled(&error));
        assert!(pre_cancel_events.is_empty());

        cancelled.store(false, Ordering::Relaxed);
        let mut callback_events = Vec::new();
        let error = scan_paths_with_progress_cancellable(
            &[temp.path().to_path_buf()],
            &options,
            &cancelled,
            |event| {
                callback_events.push(event.clone());
                if event.phase == ScanPhase::Scanning {
                    cancelled.store(true, Ordering::Relaxed);
                }
            },
        )
        .expect_err("callback-cancelled parallel scan");

        assert_eq!(error.to_string(), SCAN_CANCELLED);
        assert!(is_scan_cancelled(&error));
        assert_eq!(callback_events.len(), 1);
        assert_eq!(callback_events[0].phase, ScanPhase::Scanning);
    }

    #[test]
    fn parallel_progress_callback_panic_drains_bounded_batches_before_unwinding() {
        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..4096 {
            fs::write(temp.path().join(format!("artifact-{index:04}")), b"x")
                .expect("write artifact");
        }
        let root = temp.path().to_path_buf();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();

        let scan = std::thread::spawn(move || {
            let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = scan_paths_with_progress(
                    &[root],
                    &ScanOptions {
                        workers: MAX_SCAN_WORKERS,
                        ..ScanOptions::default()
                    },
                    |event| {
                        if event.phase == ScanPhase::Scanning {
                            // Give workers time to fill the bounded queue before unwinding.
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            panic!("parallel progress callback panic");
                        }
                    },
                );
            }));
            done_sender
                .send(panic_result.is_err())
                .expect("test receiver remains available");
        });

        assert!(
            done_receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("parallel scan must not deadlock after a callback panic")
        );
        scan.join().expect("scan thread caught the callback panic");
    }

    #[test]
    fn stay_on_filesystem_preserves_entries_on_a_regular_tree() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("nested")).expect("nested directory");
        fs::write(temp.path().join("nested").join("artifact"), b"1234").expect("write artifact");

        let unrestricted =
            scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
        let restricted = scan_paths(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                stay_on_filesystem: true,
                ..ScanOptions::default()
            },
        )
        .expect("same-filesystem scan");

        assert_eq!(restricted.summary, unrestricted.summary);
        assert_eq!(restricted.entries.len(), unrestricted.entries.len());
        assert!(restricted.issues.is_empty());
    }

    #[test]
    fn stay_on_filesystem_keeps_ignore_pruning_and_following_siblings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ignored = temp.path().join("a-ignored");
        fs::create_dir(&ignored).expect("ignored directory");
        fs::write(ignored.join("secret"), b"secret").expect("write ignored artifact");
        fs::write(temp.path().join("z-visible"), b"visible").expect("write visible artifact");

        let report = scan_paths(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                stay_on_filesystem: true,
                ignore_dirs: vec![ignored.clone()],
                ..ScanOptions::default()
            },
        )
        .expect("same-filesystem scan with ignore");
        let ignored = ignored.canonicalize().expect("canonical ignored path");

        assert!(
            !report
                .entries
                .iter()
                .any(|entry| entry.path.starts_with(&ignored))
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.path.ends_with("z-visible"))
        );
        assert!(report.issues.iter().any(|issue| {
            issue.code == ScanIssueCode::IgnoredByConfig
                && issue.path.as_deref() == Some(ignored.as_path())
        }));
    }

    #[test]
    fn cross_filesystem_evidence_requires_two_known_different_devices() {
        assert!(is_cross_filesystem(Some(1), Some(2)));
        assert!(!is_cross_filesystem(Some(1), Some(1)));
        assert!(!is_cross_filesystem(Some(1), None));
        assert!(!is_cross_filesystem(None, Some(2)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_information_provides_identity_and_volume_without_leaking_handles() {
        let temp = tempfile::tempdir().expect("tempdir");
        let child_dir = temp.path().join("child");
        fs::create_dir(&child_dir).expect("child directory");
        let first = child_dir.join("artifact");
        let second = child_dir.join("artifact-link");
        fs::write(&first, b"123456").expect("write artifact");
        let ordinary_metadata = first.symlink_metadata().expect("ordinary metadata");
        assert_eq!(file_identity(&first, &ordinary_metadata), None);
        fs::hard_link(&first, &second).expect("hard link");

        let first_metadata = first.symlink_metadata().expect("first metadata");
        let second_metadata = second.symlink_metadata().expect("second metadata");
        let first_identity = file_identity(&first, &first_metadata).expect("first identity");
        let second_identity = file_identity(&second, &second_metadata).expect("second identity");
        assert_eq!(first_identity, second_identity);

        let root_metadata = temp.path().symlink_metadata().expect("root metadata");
        let child_metadata = child_dir.symlink_metadata().expect("child metadata");
        let root_volume = device_id_for_evidence(temp.path(), &root_metadata).expect("root volume");
        let child_volume =
            device_id_for_evidence(&child_dir, &child_metadata).expect("child volume");
        assert!(!is_cross_filesystem(Some(root_volume), Some(child_volume)));

        // Removal would fail on Windows if `windows_file_information` retained the link handle.
        fs::remove_file(&second).expect("information handle was closed");
    }

    #[test]
    fn nested_and_duplicate_roots_are_scanned_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let nested = temp.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("file"), b"123").expect("write");

        let report = scan_paths(
            &[
                nested.clone(),
                temp.path().to_path_buf(),
                temp.path().to_path_buf(),
            ],
            &ScanOptions::default(),
        )
        .expect("scan");

        assert_eq!(
            report.summary.roots,
            vec![temp.path().canonicalize().expect("root")]
        );
        assert_eq!(
            report
                .entries
                .iter()
                .filter(|entry| entry.path.ends_with("nested/file"))
                .count(),
            1
        );
    }

    #[test]
    fn resolved_scan_api_trusts_normalized_roots_and_preserves_an_empty_scope() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("artifact"), b"1234").expect("write artifact");
        let trusted_root = temp.path().join(".");
        let resolved = ResolvedScanRoots {
            roots: vec![trusted_root.clone()],
            ..ResolvedScanRoots::default()
        };

        let report = scan_resolved_paths(
            &resolved,
            &ScanOptions {
                workers: 2,
                ..ScanOptions::default()
            },
        )
        .expect("resolved scan");
        assert_eq!(report.summary.roots, vec![trusted_root]);
        assert_eq!(report.summary.total_size_bytes, 4);

        let empty = scan_resolved_paths(
            &ResolvedScanRoots::default(),
            &ScanOptions {
                workers: 2,
                ..ScanOptions::default()
            },
        )
        .expect("empty resolved scan");
        assert!(empty.summary.roots.is_empty());
        assert!(empty.entries.is_empty());
    }

    #[test]
    fn global_scan_roots_use_environment_and_filter_nested_roots() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let cache = home.join(".cache");
        let pnpm = cache.join("pnpm");
        let downloads = home.join("Downloads");
        fs::create_dir_all(&pnpm).expect("pnpm cache");
        fs::create_dir_all(&downloads).expect("downloads");
        let environment = GlobalScanEnvironment {
            home_dir: Some(home.clone()),
            cache_dir: Some(cache.clone()),
            download_dir: Some(downloads.clone()),
            ..GlobalScanEnvironment::default()
        };

        let roots = discover_global_scan_roots(
            &[
                GlobalScanKind::DeveloperCaches,
                GlobalScanKind::AppCaches,
                GlobalScanKind::Downloads,
            ],
            &environment,
        );
        let cache = cache.canonicalize().expect("canonical cache");
        let pnpm = pnpm.canonicalize().expect("canonical pnpm");
        let downloads = downloads.canonicalize().expect("canonical downloads");

        assert!(roots.iter().any(|root| root.path == cache));
        assert!(roots.iter().any(|root| root.path == downloads));
        assert!(!roots.iter().any(|root| root.path == pnpm));
        assert!(!roots.iter().any(|root| root.path == home));
    }

    #[test]
    fn resolved_global_locations_survive_parent_root_deduplication() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let cache = home.join(".cache");
        let pnpm = cache.join("pnpm");
        fs::create_dir_all(&pnpm).expect("pnpm cache");
        let environment = GlobalScanEnvironment {
            home_dir: Some(home),
            cache_dir: Some(cache.clone()),
            ..GlobalScanEnvironment::default()
        };
        let request = ScanRequest::global(vec![
            GlobalScanKind::DeveloperCaches,
            GlobalScanKind::AppCaches,
        ]);

        let resolved =
            resolve_scan_roots_with_env(&request, &[], &environment).expect("resolve global roots");
        let cache = cache.canonicalize().expect("canonical cache");
        let pnpm = pnpm.canonicalize().expect("canonical pnpm");

        assert_eq!(resolved.roots, vec![cache.clone()]);
        assert_eq!(resolved.global_roots.len(), 1);
        assert!(resolved.global_locations.iter().any(|location| {
            location.path == pnpm
                && location.kind == GlobalScanKind::DeveloperCaches
                && location.label == "pnpm cache"
        }));
        assert!(resolved.global_locations.iter().any(|location| {
            location.path == cache && location.kind == GlobalScanKind::AppCaches
        }));

        let mut reversed_request = request;
        reversed_request.global_kinds.reverse();
        let reversed = resolve_scan_roots_with_env(&reversed_request, &[], &environment)
            .expect("resolve reversed global roots");
        assert_eq!(resolved.global_locations, reversed.global_locations);

        let app_only = resolve_scan_roots_with_env(
            &ScanRequest::global(vec![GlobalScanKind::AppCaches]),
            &[],
            &environment,
        )
        .expect("resolve app cache root");
        assert_eq!(app_only.roots, vec![cache]);
        assert!(app_only.global_locations.iter().any(|location| {
            location.path == pnpm && location.kind == GlobalScanKind::DeveloperCaches
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_global_roots_cover_known_user_level_cleanup_locations() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let library_caches = home.join("Library").join("Caches");
        let brave = library_caches.join("BraveSoftware").join("Brave-Browser");
        let slack = home
            .join("Library")
            .join("Application Support")
            .join("Slack")
            .join("Cache");
        let simulator = home
            .join("Library")
            .join("Developer")
            .join("CoreSimulator")
            .join("Caches");
        let diagnostics = home.join("Library").join("DiagnosticReports");
        for path in [&brave, &slack, &simulator, &diagnostics] {
            fs::create_dir_all(path).expect("cleanup root");
        }
        let environment = GlobalScanEnvironment {
            home_dir: Some(home),
            cache_dir: Some(library_caches.clone()),
            ..GlobalScanEnvironment::default()
        };

        let browser_roots =
            discover_global_scan_roots(&[GlobalScanKind::BrowserCaches], &environment);
        let app_roots = discover_global_scan_roots(&[GlobalScanKind::AppCaches], &environment);
        let developer_roots =
            discover_global_scan_roots(&[GlobalScanKind::DeveloperCaches], &environment);
        let log_roots = discover_global_scan_roots(&[GlobalScanKind::Logs], &environment);

        assert!(browser_roots.iter().any(|root| {
            root.path == brave.canonicalize().expect("canonical Brave cache")
                && root.kind == GlobalScanKind::BrowserCaches
        }));
        assert!(app_roots.iter().any(|root| {
            root.path == slack.canonicalize().expect("canonical Slack cache")
                && root.kind == GlobalScanKind::AppCaches
        }));
        assert!(app_roots.iter().any(|root| {
            root.path
                == library_caches
                    .canonicalize()
                    .expect("canonical Library caches")
        }));
        assert!(developer_roots.iter().any(|root| {
            root.path == simulator.canonicalize().expect("canonical simulator cache")
                && root.kind == GlobalScanKind::DeveloperCaches
        }));
        assert!(log_roots.iter().any(|root| {
            root.path == diagnostics.canonicalize().expect("canonical diagnostics")
                && root.kind == GlobalScanKind::Logs
        }));
    }

    #[test]
    fn windows_app_cache_roots_are_narrow_and_user_level() {
        let temp = tempfile::tempdir().expect("tempdir");
        let local = temp.path().join("AppData").join("Local");
        let shader_cache = local.join("D3DSCache");
        let unrelated = local.join("Packages");
        fs::create_dir_all(&shader_cache).expect("shader cache");
        fs::create_dir_all(&unrelated).expect("unrelated local data");
        let environment = GlobalScanEnvironment {
            data_local_dir: Some(local.clone()),
            ..GlobalScanEnvironment::default()
        };

        let mut roots = Vec::new();
        push_windows_app_cache_roots(&environment, &mut roots);
        let roots = normalize_global_roots(roots, &environment);

        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].path,
            shader_cache.canonicalize().expect("canonical shader cache")
        );
        assert_eq!(roots[0].kind, GlobalScanKind::AppCaches);
        assert_ne!(roots[0].path, local);
        assert_ne!(roots[0].path, unrelated);
    }

    #[test]
    fn global_scan_request_does_not_add_current_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let global_temp = temp.path().join("tmp");
        fs::create_dir(&global_temp).expect("tmp dir");
        let environment = GlobalScanEnvironment {
            temp_dir: Some(global_temp.clone()),
            ..GlobalScanEnvironment::default()
        };
        let request = ScanRequest::global(vec![GlobalScanKind::TempFiles]);

        let resolved = resolve_scan_roots_with_env(&request, &GlobalScanKind::ALL, &environment)
            .expect("resolve roots");

        assert_eq!(
            resolved.roots,
            vec![global_temp.canonicalize().expect("canonical tmp")]
        );
    }

    #[test]
    fn global_scan_request_preserves_empty_global_coverage() {
        let request = ScanRequest::global(vec![GlobalScanKind::TempFiles]);

        let resolved = resolve_scan_roots_with_env(
            &request,
            &GlobalScanKind::ALL,
            &GlobalScanEnvironment::default(),
        )
        .expect("empty global coverage");

        assert!(resolved.roots.is_empty());
        assert!(resolved.global_roots.is_empty());
        assert!(resolved.global_locations.is_empty());
    }

    #[test]
    fn global_scan_evidence_is_deterministic_and_uses_completed_roots() {
        let scan_root = PathBuf::from("scan-root");
        let app_cache = scan_root.join("app-cache");
        let logs = scan_root.join("logs");
        let uncovered = PathBuf::from("uncovered");
        let request = ScanRequest::global(Vec::new());
        let configured_kinds = [
            GlobalScanKind::Logs,
            GlobalScanKind::AppCaches,
            GlobalScanKind::Logs,
        ];
        let resolved = ResolvedScanRoots {
            roots: vec![scan_root.clone()],
            global_roots: Vec::new(),
            global_locations: vec![
                GlobalScanRoot {
                    path: logs.clone(),
                    kind: GlobalScanKind::Logs,
                    label: "Logs".to_string(),
                },
                GlobalScanRoot {
                    path: uncovered,
                    kind: GlobalScanKind::BrowserCaches,
                    label: "Uncovered browser cache".to_string(),
                },
                GlobalScanRoot {
                    path: app_cache.clone(),
                    kind: GlobalScanKind::AppCaches,
                    label: "Application cache".to_string(),
                },
            ],
        };

        let evidence = global_scan_evidence(
            &request,
            &configured_kinds,
            &resolved,
            std::slice::from_ref(&scan_root),
        );

        assert_eq!(
            evidence.requested_kinds,
            vec![GlobalScanKind::AppCaches, GlobalScanKind::Logs]
        );
        assert_eq!(
            evidence
                .locations
                .iter()
                .map(|location| (&location.local_path, &location.scan_root))
                .collect::<Vec<_>>(),
            vec![(&app_cache, &scan_root), (&logs, &scan_root)]
        );
    }

    #[test]
    fn explicit_scan_does_not_invent_global_evidence() {
        let request = ScanRequest::paths(vec![PathBuf::from("project")]);
        let resolved = ResolvedScanRoots {
            roots: request.paths.clone(),
            global_roots: Vec::new(),
            global_locations: vec![GlobalScanRoot {
                path: PathBuf::from("project/cache"),
                kind: GlobalScanKind::AppCaches,
                label: "Application cache".to_string(),
            }],
        };

        assert_eq!(
            global_scan_evidence(&request, &GlobalScanKind::ALL, &resolved, &resolved.roots,),
            GlobalScanEvidence::default()
        );
    }

    #[test]
    fn explicit_ignore_directory_skips_the_entire_subtree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ignored = temp.path().join("ignored");
        fs::create_dir(&ignored).expect("ignored dir");
        fs::write(ignored.join("secret"), b"hidden").expect("write");
        fs::write(temp.path().join("visible"), b"visible").expect("write");
        let canonical_ignored = ignored.canonicalize().expect("canonical ignored dir");

        let report = scan_paths(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                ignore_dirs: vec![ignored.clone()],
                ..ScanOptions::default()
            },
        )
        .expect("scan");

        assert!(
            !report
                .entries
                .iter()
                .any(|entry| entry.path.ends_with("ignored"))
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.path.ends_with("visible"))
        );
        assert_eq!(report.completeness(), ReportIntegrity::Complete);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ScanIssueCode::IgnoredByConfig
                && issue.path.as_deref() == Some(canonical_ignored.as_path())
        }));
    }

    #[test]
    fn invalid_ignore_glob_fails_before_scanning() {
        let error = scan_paths(
            &[PathBuf::from(".")],
            &ScanOptions {
                ignore_patterns: vec!["[".to_string()],
                ..ScanOptions::default()
            },
        )
        .expect_err("invalid glob");

        assert!(error.to_string().contains("invalid scan ignore pattern"));
    }

    #[test]
    fn entry_budget_returns_read_only_partial_evidence_and_limits_retention() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("a"), b"a").expect("a");
        fs::write(temp.path().join("b"), b"b").expect("b");
        let report = scan_paths(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                workers: MAX_SCAN_WORKERS,
                budgets: ScanBudgetLimits {
                    entries: 1,
                    ..ScanBudgetLimits::default()
                },
                ..ScanOptions::default()
            },
        )
        .expect("soft-limited scan");

        assert_eq!(report.workers_used, 1);
        assert_eq!(report.entries.len(), 1);
        assert!(report.completed_roots.is_empty());
        assert_eq!(report.completeness(), ReportIntegrity::Partial);
        assert_eq!(
            report.budget_exceeded,
            vec![ScanBudgetExceeded::EntryCount {
                limit: 1,
                observed: 2,
            }]
        );
    }

    #[test]
    fn issue_detail_budget_caps_retention_but_keeps_true_error_count() {
        let temp = tempfile::tempdir().expect("tempdir");
        let roots = (0..3)
            .map(|index| temp.path().join(format!("missing-{index}")))
            .collect::<Vec<_>>();
        let report = scan_paths(
            &roots,
            &ScanOptions {
                budgets: ScanBudgetLimits {
                    issue_details: 1,
                    ..ScanBudgetLimits::default()
                },
                ..ScanOptions::default()
            },
        )
        .expect("detail-limited scan");

        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.summary.errors, 3);
        assert_eq!(report.completed_roots.len(), 3);
        assert_eq!(
            report.budget_exceeded,
            vec![ScanBudgetExceeded::IssueDetails {
                limit: 1,
                observed: 3,
            }]
        );
    }

    #[test]
    fn estimated_memory_budget_stops_before_retaining_an_oversized_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let report = scan_paths(
            &[temp.path().to_path_buf()],
            &ScanOptions {
                budgets: ScanBudgetLimits {
                    estimated_memory_bytes: 1,
                    ..ScanBudgetLimits::default()
                },
                ..ScanOptions::default()
            },
        )
        .expect("memory-limited scan");

        assert!(report.entries.is_empty());
        assert!(matches!(
            report.budget_exceeded.as_slice(),
            [ScanBudgetExceeded::EstimatedMemory {
                limit_bytes: 1,
                observed_bytes,
            }] if *observed_bytes > 1
        ));
    }

    #[test]
    fn diagnostic_memory_budget_keeps_true_error_count_without_retaining_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing-root");

        let report = scan_paths(
            &[missing],
            &ScanOptions {
                budgets: ScanBudgetLimits {
                    estimated_memory_bytes: 1,
                    ..ScanBudgetLimits::default()
                },
                ..ScanOptions::default()
            },
        )
        .expect("diagnostic-memory-limited scan");

        assert!(report.entries.is_empty());
        assert!(report.issues.is_empty());
        assert!(report.errors.is_empty());
        assert_eq!(report.summary.errors, 1);
        assert!(report.completed_roots.is_empty());
        assert!(matches!(
            report.budget_exceeded.as_slice(),
            [ScanBudgetExceeded::EstimatedMemory {
                limit_bytes: 1,
                observed_bytes,
            }] if *observed_bytes > 1
        ));
    }

    #[test]
    fn elapsed_budget_is_typed_and_path_free() {
        let started_at = Instant::now()
            .checked_sub(std::time::Duration::from_millis(2))
            .expect("earlier instant");
        let mut tracker = ScanBudgetTracker::new(
            ScanBudgetLimits {
                elapsed_millis: 1,
                ..ScanBudgetLimits::default()
            },
            started_at,
        );
        assert!(tracker.check_elapsed());
        let mut report = ScanReport::default();
        tracker.finish(&mut report);
        assert!(matches!(
            report.budget_exceeded.as_slice(),
            [ScanBudgetExceeded::ElapsedTime {
                limit_millis: 1,
                observed_millis,
            }] if *observed_millis >= 2
        ));
    }

    #[test]
    fn resolved_scan_elapsed_budget_includes_caller_root_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("entry"), b"data").expect("entry");
        let resolved = ResolvedScanRoots {
            roots: vec![temp.path().to_path_buf()],
            ..ResolvedScanRoots::default()
        };
        let started_at = Instant::now()
            .checked_sub(std::time::Duration::from_millis(2))
            .expect("earlier instant");

        let report = scan_resolved_paths_started_at(
            &resolved,
            &ScanOptions {
                budgets: ScanBudgetLimits {
                    elapsed_millis: 1,
                    ..ScanBudgetLimits::default()
                },
                ..ScanOptions::default()
            },
            started_at,
        )
        .expect("soft-limited resolved scan");

        assert!(report.entries.is_empty());
        assert!(report.completed_roots.is_empty());
        assert!(matches!(
            report.budget_exceeded.as_slice(),
            [ScanBudgetExceeded::ElapsedTime {
                limit_millis: 1,
                observed_millis,
            }] if *observed_millis >= 2
        ));
    }

    #[test]
    fn cancellation_uses_the_typed_error_contract() {
        let cancelled = AtomicBool::new(true);
        let error = scan_paths_with_progress_cancellable(
            &[PathBuf::from(".")],
            &ScanOptions::default(),
            &cancelled,
            |_| {},
        )
        .expect_err("cancelled scan");
        assert_eq!(error.to_string(), SCAN_CANCELLED);
        assert!(is_scan_cancelled(&error));
    }

    #[test]
    fn cancellation_recognition_uses_the_error_chain_not_display_text() {
        let typed = anyhow::Error::new(ScanCancelled).context("outer scan context");
        assert!(is_scan_cancelled(&typed));

        let same_text = anyhow::anyhow!(SCAN_CANCELLED);
        assert!(!is_scan_cancelled(&same_text));
    }

    #[test]
    fn parallel_worker_failure_tracking_does_not_use_diagnostic_text() {
        let mut batch = ParallelScanBatch {
            errors: vec![ScanError {
                path: None,
                message: PARALLEL_SCAN_FAILED.to_string(),
            }],
            ..ParallelScanBatch::default()
        };

        ensure_parallel_worker_failure_diagnostic(&mut batch, false, true);

        assert!(batch.worker_failure_diagnostic);
        assert_eq!(batch.errors.len(), 2);
        assert_eq!(batch.issues.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinked_files_are_counted_once_under_the_lexical_owner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let later_dir = temp.path().join("z-owner");
        let owner_dir = temp.path().join("a-owner");
        fs::create_dir(&later_dir).expect("later directory");
        fs::create_dir(&owner_dir).expect("owner directory");
        let later = later_dir.join("artifact");
        let owner = owner_dir.join("artifact-link");
        fs::write(&later, b"123456").expect("write");
        fs::hard_link(&later, &owner).expect("hard link");

        let mut tracker = HardlinkTracker::default();
        assert_eq!(
            tracker.account(
                &later.symlink_metadata().expect("later metadata"),
                &later,
                0
            ),
            HardlinkAccounting::Count(6)
        );
        assert_eq!(
            tracker.account(
                &owner.symlink_metadata().expect("owner metadata"),
                &owner,
                1
            ),
            HardlinkAccounting::Reassign {
                previous_entry_index: 0,
                size_bytes: 6,
            }
        );

        let report =
            scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
        let file_bytes = report
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .map(|entry| entry.size_bytes)
            .sum::<u64>();
        let directory_bytes = report
            .entries
            .iter()
            .filter(|entry| entry.path.ends_with("a-owner") || entry.path.ends_with("z-owner"))
            .map(|entry| entry.size_bytes)
            .sum::<u64>();
        let owner_bytes = report
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("a-owner/artifact-link"))
            .expect("lexical owner entry")
            .size_bytes;
        let duplicate_bytes = report
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("z-owner/artifact"))
            .expect("duplicate hardlink entry")
            .size_bytes;

        assert_eq!(file_bytes, 6);
        assert_eq!(directory_bytes, 6);
        assert_eq!(owner_bytes, 6);
        assert_eq!(duplicate_bytes, 0);
        assert_eq!(report.summary.total_size_bytes, 6);
    }
}
