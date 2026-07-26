#![forbid(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    fs::Metadata,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use cleanr_core::{
    EntryKind, GlobalScanKind, ReportIntegrity, ScanEntry, ScanIssue, ScanIssueCode, ScanRequest,
    ScanSummary,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

pub const SCAN_CANCELLED: &str = "scan cancelled";
pub const NO_GLOBAL_SCAN_ROOTS: &str = "no system cleanup locations were found";

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub stay_on_filesystem: bool,
    pub ignore_dirs: Vec<PathBuf>,
    pub ignore_patterns: Vec<String>,
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
}

impl Default for ScanReport {
    fn default() -> Self {
        Self {
            as_of: Utc::now(),
            summary: ScanSummary::default(),
            entries: Vec::new(),
            issues: Vec::new(),
            errors: Vec::new(),
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
        ReportIntegrity::from_issues(&self.issues)
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
    if request.include_global {
        let global_kinds = if request.global_kinds.is_empty() {
            configured_global_kinds
        } else {
            &request.global_kinds
        };
        global_roots = discover_global_scan_roots(global_kinds, environment);
        roots.extend(global_roots.iter().map(|root| root.path.clone()));
    }

    if roots.is_empty() {
        if request.include_global {
            bail!(NO_GLOBAL_SCAN_ROOTS);
        }
        roots.push(std::env::current_dir()?);
    }

    Ok(ResolvedScanRoots {
        roots: normalize_roots(roots),
        global_roots,
    })
}

#[must_use]
pub fn discover_global_scan_roots(
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
    normalize_global_roots(roots, environment)
}

pub fn scan_paths(paths: &[PathBuf], options: &ScanOptions) -> Result<ScanReport> {
    scan_paths_impl(paths, options, None, &mut |_| {})
}

pub fn scan_paths_with_progress(
    paths: &[PathBuf],
    options: &ScanOptions,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_paths_impl(paths, options, None, &mut on_progress)
}

pub fn scan_paths_with_progress_cancellable(
    paths: &[PathBuf],
    options: &ScanOptions,
    cancelled: &AtomicBool,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanReport> {
    scan_paths_impl(paths, options, Some(cancelled), &mut on_progress)
}

fn scan_paths_impl(
    paths: &[PathBuf],
    options: &ScanOptions,
    cancelled: Option<&AtomicBool>,
    on_progress: &mut dyn FnMut(ScanProgress),
) -> Result<ScanReport> {
    let as_of = Utc::now();
    let roots = normalize_roots(if paths.is_empty() {
        vec![std::env::current_dir()?]
    } else {
        paths.to_vec()
    });
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
    };

    let mut hardlinks = HardlinkTracker::default();
    let mut progress = ScanProgressTracker::new(on_progress);
    for root in &roots {
        let result = scan_root(
            root,
            options,
            &ignore,
            cancelled,
            &mut hardlinks,
            &mut report,
            &mut progress,
        );
        if let Err(err) = result {
            if err.to_string() == SCAN_CANCELLED {
                return Err(err);
            }
            return Err(err).with_context(|| format!("failed to scan {}", root.display()));
        }
    }

    (progress.on_progress)(ScanProgress {
        phase: ScanPhase::Aggregating,
        entries_total: progress.entries_scanned,
        entries_scanned: progress.entries_scanned,
        bytes_scanned: progress.bytes_scanned,
        errors: report.errors.len(),
        current_path: None,
    });
    aggregate_directory_sizes(&mut report.entries);
    report.summary.entries_seen = report.entries.len();
    report.summary.errors = report.errors.len();
    report.summary.total_size_bytes = report
        .entries
        .iter()
        .filter(|entry| report.summary.roots.iter().any(|root| &entry.path == root))
        .map(|entry| entry.size_bytes)
        .sum();

    Ok(report)
}

fn scan_root(
    root: &Path,
    options: &ScanOptions,
    ignore: &IgnoreMatcher,
    cancelled: Option<&AtomicBool>,
    hardlinks: &mut HardlinkTracker,
    report: &mut ScanReport,
    progress: &mut ScanProgressTracker<'_>,
) -> Result<()> {
    let root_device = root_device(root, options);

    let mut walker = WalkDir::new(root).follow_links(false).into_iter();
    while let Some(next) = walker.next() {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            bail!(SCAN_CANCELLED);
        }
        let entry = match next {
            Ok(entry) => entry,
            Err(err) => {
                let path = err.path().map(Path::to_path_buf);
                report.errors.push(ScanError {
                    path: path.clone(),
                    message: err.to_string(),
                });
                report.issues.push(ScanIssue {
                    code: ScanIssueCode::TraversalError,
                    path,
                });
                continue;
            }
        };

        let path = entry.path().to_path_buf();
        let is_directory = entry.file_type().is_dir();
        if ignore.matches(&path, root) {
            report.issues.push(ScanIssue {
                code: ScanIssueCode::IgnoredByConfig,
                path: Some(path),
            });
            if is_directory {
                walker.skip_current_dir();
            }
            continue;
        }

        let metadata = match entry.path().symlink_metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                report.errors.push(ScanError {
                    path: Some(path.clone()),
                    message: err.to_string(),
                });
                report.issues.push(ScanIssue {
                    code: ScanIssueCode::MetadataUnavailable,
                    path: Some(path),
                });
                continue;
            }
        };

        if is_directory
            && let Some(root_device) = root_device
            && device_id(&metadata).is_some_and(|device| device != root_device)
        {
            report.issues.push(ScanIssue {
                code: ScanIssueCode::CrossFilesystemSkipped,
                path: Some(path),
            });
            walker.skip_current_dir();
            continue;
        }

        let kind = kind_of(&metadata);
        let size_bytes = if kind == EntryKind::File && hardlinks.insert(&metadata) {
            metadata.len()
        } else {
            0
        };

        progress.record(&path, size_bytes, report.errors.len());

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
}

fn root_device(root: &Path, options: &ScanOptions) -> Option<u64> {
    if options.stay_on_filesystem {
        root.symlink_metadata()
            .ok()
            .and_then(|metadata| device_id(&metadata))
    } else {
        None
    }
}

fn should_emit_progress(entries: usize) -> bool {
    entries <= 16 || entries.is_multiple_of(64)
}

fn aggregate_directory_sizes(entries: &mut [ScanEntry]) {
    let by_path = entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| (entry.path.clone(), idx))
        .collect::<HashMap<_, _>>();

    let mut indices = (0..entries.len()).collect::<Vec<_>>();
    indices.sort_by_key(|idx| std::cmp::Reverse(entries[*idx].path.components().count()));

    for idx in indices {
        let size = entries[idx].size_bytes;
        let Some(parent) = entries[idx].path.parent() else {
            continue;
        };
        if let Some(parent_idx) = by_path.get(parent) {
            entries[*parent_idx].size_bytes = entries[*parent_idx].size_bytes.saturating_add(size);
        }
    }
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
    });

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
    seen: HashSet<FileIdentity>,
}

impl HardlinkTracker {
    fn insert(&mut self, metadata: &Metadata) -> bool {
        let Some(identity) = file_identity(metadata) else {
            return true;
        };
        self.seen.insert(identity)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    (metadata.nlink() > 1).then_some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(unix)]
fn device_id(metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(not(unix))]
fn device_id(_metadata: &Metadata) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
    fn directory_sizes_include_children() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("mkdir");
        fs::write(temp.path().join("target").join("artifact"), vec![0; 12]).expect("write");

        let report =
            scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
        let target = report
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("target"))
            .expect("target entry");

        assert_eq!(target.size_bytes, 12);
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

        let report = scan_paths(std::slice::from_ref(&unavailable), &ScanOptions::default())
            .expect("scan report");

        assert_eq!(report.completeness(), ReportIntegrity::Partial);
        assert_eq!(report.errors.len(), 1);
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

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(SCAN_CANCELLED));
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
    fn global_scan_request_reports_missing_global_roots() {
        let request = ScanRequest::global(vec![GlobalScanKind::TempFiles]);

        let error = resolve_scan_roots_with_env(
            &request,
            &GlobalScanKind::ALL,
            &GlobalScanEnvironment::default(),
        )
        .expect_err("missing roots");

        assert!(error.to_string().contains(NO_GLOBAL_SCAN_ROOTS));
    }

    #[test]
    fn explicit_ignore_directory_skips_the_entire_subtree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ignored = temp.path().join("ignored");
        fs::create_dir(&ignored).expect("ignored dir");
        fs::write(ignored.join("secret"), b"hidden").expect("write");
        fs::write(temp.path().join("visible"), b"visible").expect("write");

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
                && issue.path.as_deref() == Some(ignored.as_path())
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

    #[cfg(unix)]
    #[test]
    fn hardlinked_files_are_counted_only_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::write(&first, b"123456").expect("write");
        fs::hard_link(&first, &second).expect("hard link");

        let report =
            scan_paths(&[temp.path().to_path_buf()], &ScanOptions::default()).expect("scan");
        let file_bytes = report
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .map(|entry| entry.size_bytes)
            .sum::<u64>();

        assert_eq!(file_bytes, 6);
        assert_eq!(report.summary.total_size_bytes, 6);
    }
}
