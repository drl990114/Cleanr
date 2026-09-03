#![forbid(unsafe_code)]

mod budget;
mod identity;
mod roots;
mod scanner;

pub use roots::{
    GlobalScanEnvironment, GlobalScanRoot, ResolvedScanRoots, developer_cache_roots,
    discover_global_scan_locations, discover_global_scan_roots, global_scan_evidence,
    resolve_scan_roots, resolve_scan_roots_with_env, resolve_scan_roots_with_env_and_locations,
    resolve_scan_roots_with_locations,
};

pub use scanner::{
    MAX_SCAN_WORKERS, NO_GLOBAL_SCAN_ROOTS, SCAN_CANCELLED, ScanCancelled, ScanError, ScanOptions,
    ScanPhase, ScanProgress, ScanReport, is_scan_cancelled, scan_paths, scan_paths_with_progress,
    scan_paths_with_progress_cancellable, scan_resolved_paths, scan_resolved_paths_started_at,
    scan_resolved_paths_with_progress, scan_resolved_paths_with_progress_cancellable,
    scan_resolved_paths_with_progress_cancellable_started_at,
};
