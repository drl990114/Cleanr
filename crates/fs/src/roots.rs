use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::Result;
use cleanr_core::{
    GlobalManagedLocationEvidence, GlobalScanEvidence, GlobalScanKind, GlobalScanLocationEvidence,
    RulePlatform, ScanIssue, ScanIssueCode, ScanLocationBase, ScanLocationDefinition,
    ScanLocationMode, ScanRequest,
};
use globset::{Glob, GlobSetBuilder};

use crate::scanner::normalize_roots;

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
    pub os_managed: Vec<GlobalManagedLocationEvidence>,
    /// Fail-closed discovery evidence produced before recursive traversal begins.
    pub issues: Vec<ScanIssue>,
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

pub fn resolve_scan_roots_with_locations(
    request: &ScanRequest,
    configured_global_kinds: &[GlobalScanKind],
    locations: &[ScanLocationDefinition],
) -> Result<ResolvedScanRoots> {
    resolve_scan_roots_with_env_and_locations(
        request,
        configured_global_kinds,
        &GlobalScanEnvironment::current(),
        locations,
        RulePlatform::current(),
    )
}

pub fn resolve_scan_roots_with_env(
    request: &ScanRequest,
    configured_global_kinds: &[GlobalScanKind],
    environment: &GlobalScanEnvironment,
) -> Result<ResolvedScanRoots> {
    resolve_scan_roots_with_env_and_locations(
        request,
        configured_global_kinds,
        environment,
        &[],
        RulePlatform::current(),
    )
}

pub fn resolve_scan_roots_with_env_and_locations(
    request: &ScanRequest,
    configured_global_kinds: &[GlobalScanKind],
    environment: &GlobalScanEnvironment,
    locations: &[ScanLocationDefinition],
    platform: Option<RulePlatform>,
) -> Result<ResolvedScanRoots> {
    let mut roots = request.paths.clone();
    let mut global_roots = Vec::new();
    let mut global_locations = Vec::new();
    let mut os_managed = Vec::new();
    let mut issues = Vec::new();
    if request.include_global {
        let global_kinds = if request.global_kinds.is_empty() {
            configured_global_kinds
        } else {
            &request.global_kinds
        };
        let requested_locations = discover_global_scan_locations_with_definitions(
            global_kinds,
            environment,
            locations,
            platform,
            &mut issues,
        )?;
        global_roots = normalize_global_roots(requested_locations.clone(), environment);

        // Reuse the exact locations that produced the traversal roots. Discover only the
        // unrequested kinds needed for nested-category evidence, so a changing profile directory
        // cannot make the scan roots and their recorded provenance disagree.
        let unrequested_kinds = GlobalScanKind::ALL
            .into_iter()
            .filter(|kind| !global_kinds.contains(kind))
            .collect::<Vec<_>>();
        let mut all_locations = requested_locations;
        all_locations.extend(discover_global_scan_locations_with_definitions(
            &unrequested_kinds,
            environment,
            locations,
            platform,
            &mut Vec::new(),
        )?);
        global_locations = normalize_global_locations(all_locations, environment)
            .into_iter()
            .filter(|location| {
                global_roots
                    .iter()
                    .any(|root| location.path == root.path || location.path.starts_with(&root.path))
            })
            .collect();
        roots.extend(global_roots.iter().map(|root| root.path.clone()));
        os_managed = locations
            .iter()
            .filter(|definition| {
                definition.mode == ScanLocationMode::OsManaged
                    && wants(global_kinds, definition.kind)
                    && platform.is_some_and(|platform| definition.platforms.contains(&platform))
            })
            .map(|definition| GlobalManagedLocationEvidence {
                id: definition.id.clone(),
                kind: definition.kind,
                label: definition.label.clone(),
            })
            .collect();
        os_managed.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    if roots.is_empty() && !request.include_global {
        roots.push(std::env::current_dir()?);
    }

    Ok(ResolvedScanRoots {
        roots: normalize_roots(roots),
        global_roots,
        global_locations,
        os_managed,
        issues,
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
        os_managed: resolved.os_managed.clone(),
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
    discover_global_scan_locations_with_definitions(
        kinds,
        environment,
        &[],
        RulePlatform::current(),
        &mut Vec::new(),
    )
    .expect("fixed built-in global locations do not require fallible expansion")
}

fn discover_global_scan_locations_with_definitions(
    kinds: &[GlobalScanKind],
    environment: &GlobalScanEnvironment,
    definitions: &[ScanLocationDefinition],
    platform: Option<RulePlatform>,
    issues: &mut Vec<ScanIssue>,
) -> Result<Vec<GlobalScanRoot>> {
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
    for definition in definitions {
        if definition.mode != ScanLocationMode::Scan
            || !wants(kinds, definition.kind)
            || !platform.is_some_and(|platform| definition.platforms.contains(&platform))
        {
            continue;
        }
        let base = match definition.base {
            ScanLocationBase::Home => environment.home_dir.as_ref(),
            ScanLocationBase::Cache => environment.cache_dir.as_ref(),
            ScanLocationBase::DataLocal => environment.data_local_dir.as_ref(),
            ScanLocationBase::Data => environment.data_dir.as_ref(),
            ScanLocationBase::Temp => environment.temp_dir.as_ref(),
            ScanLocationBase::Downloads => environment.download_dir.as_ref(),
        };
        if let Some(base) = base {
            push_definition_locations(&mut roots, issues, base, definition)?;
        }
    }
    Ok(normalize_global_locations(roots, environment))
}

fn push_definition_locations(
    roots: &mut Vec<GlobalScanRoot>,
    issues: &mut Vec<ScanIssue>,
    base: &Path,
    definition: &ScanLocationDefinition,
) -> Result<()> {
    let path = if definition.relative_path.is_empty() {
        base.to_path_buf()
    } else {
        base.join(&definition.relative_path)
    };
    if !path.exists() {
        return Ok(());
    }
    let Some(anchor) = contained_directory(base, &path, issues) else {
        return Ok(());
    };
    let Some(expansion) = &definition.expansion else {
        push_global_root(roots, &anchor, definition.kind, definition.label.clone());
        return Ok(());
    };

    let mut builder = GlobSetBuilder::new();
    for pattern in &expansion.child_globs {
        builder.add(Glob::new(pattern)?);
    }
    let child_matcher = builder.build()?;
    let entries = match fs::read_dir(&anchor) {
        Ok(entries) => entries,
        Err(error) => {
            issues.push(ScanIssue {
                code: issue_code_for_io(&error),
                path: Some(anchor),
            });
            return Ok(());
        }
    };
    let mut leaves = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(ScanIssue {
                    code: issue_code_for_io(&error),
                    path: Some(anchor.clone()),
                });
                continue;
            }
        };
        if !child_matcher.is_match(entry.file_name()) {
            continue;
        }
        let child = entry.path();
        let Ok(file_type) = entry.file_type() else {
            issues.push(ScanIssue {
                code: ScanIssueCode::MetadataUnavailable,
                path: Some(child),
            });
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Some(child) = contained_directory(&anchor, &child, issues) else {
            continue;
        };
        for suffix in &expansion.suffixes {
            let leaf = child.join(suffix);
            if !leaf.exists() {
                continue;
            }
            let Some(leaf) = contained_directory(&child, &leaf, issues) else {
                continue;
            };
            leaves.push(leaf);
        }
    }
    leaves.sort();
    leaves.dedup();
    if leaves.len() > usize::from(expansion.max_matches) {
        leaves.truncate(usize::from(expansion.max_matches));
        issues.push(ScanIssue {
            code: ScanIssueCode::Unknown,
            path: Some(anchor),
        });
    }
    for leaf in leaves {
        push_global_root(roots, &leaf, definition.kind, definition.label.clone());
    }
    Ok(())
}

fn contained_directory(base: &Path, path: &Path, issues: &mut Vec<ScanIssue>) -> Option<PathBuf> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            issues.push(ScanIssue {
                code: issue_code_for_io(&error),
                path: Some(path.to_path_buf()),
            });
            return None;
        }
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let base = match base.canonicalize() {
        Ok(base) => base,
        Err(error) => {
            issues.push(ScanIssue {
                code: issue_code_for_io(&error),
                path: Some(base.to_path_buf()),
            });
            return None;
        }
    };
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            issues.push(ScanIssue {
                code: issue_code_for_io(&error),
                path: Some(path.to_path_buf()),
            });
            return None;
        }
    };
    if path == base || path.starts_with(&base) {
        Some(path)
    } else {
        issues.push(ScanIssue {
            code: ScanIssueCode::TraversalError,
            path: Some(path),
        });
        None
    }
}

fn issue_code_for_io(error: &io::Error) -> ScanIssueCode {
    if error.kind() == io::ErrorKind::PermissionDenied {
        ScanIssueCode::PermissionDenied
    } else {
        ScanIssueCode::TraversalError
    }
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
    #[cfg(all(unix, not(target_os = "ios"), not(target_os = "android")))]
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
pub(super) fn push_windows_app_cache_roots(
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

pub(super) fn normalize_global_roots(
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
