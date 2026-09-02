#![forbid(unsafe_code)]

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cleanr_core::{
    CLEANUP_PLAN_SCHEMA_VERSION, CleanupAuthorizationSource, CleanupItem, CleanupItemFingerprint,
    CleanupPlan, EXECUTION_SCHEMA_VERSION, EntryKind, ExecutionAuthorization, ExecutionItem,
    ExecutionManifest, ExecutionStatus, ExecutionSummary, PlannedAction, RESTORE_SCHEMA_VERSION,
    RestoreItem, RestoreManifest, RestoreStatus, RestoreSummary, RollbackReceipt,
};
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub trait CleanupExecutor {
    fn trash(&self, path: &Path) -> Result<RollbackReceipt>;
}

#[derive(Debug)]
pub struct CleanupAuthorization {
    source: CleanupAuthorizationSource,
}

impl CleanupAuthorization {
    #[must_use]
    pub fn explicit_user_confirmation() -> Self {
        Self {
            source: CleanupAuthorizationSource::LocalUserConfirmation,
        }
    }

    #[must_use]
    pub fn explicit_user_delegation() -> Self {
        Self {
            source: CleanupAuthorizationSource::ExplicitUserDelegation,
        }
    }
}

#[derive(Debug, Default)]
pub struct TrashExecutor;

impl CleanupExecutor for TrashExecutor {
    fn trash(&self, path: &Path) -> Result<RollbackReceipt> {
        trash_with_receipt(path)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FakeTrashExecutor {
    trashed: Arc<Mutex<Vec<PathBuf>>>,
}

impl FakeTrashExecutor {
    #[must_use]
    pub fn trashed_paths(&self) -> Vec<PathBuf> {
        self.trashed.lock().expect("fake trash mutex").clone()
    }
}

impl CleanupExecutor for FakeTrashExecutor {
    fn trash(&self, path: &Path) -> Result<RollbackReceipt> {
        self.trashed
            .lock()
            .expect("fake trash mutex")
            .push(path.to_path_buf());
        Ok(RollbackReceipt {
            method: "fake-trash".to_string(),
            note: "Test-only fake trash receipt.".to_string(),
            locator: Some(format!("fake:{}", path.display())),
        })
    }
}

pub trait RestoreExecutor {
    fn restore(&self, path: &Path, receipt: &RollbackReceipt, deleted_at: i64) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct SystemRestoreExecutor;

impl RestoreExecutor for SystemRestoreExecutor {
    fn restore(&self, path: &Path, receipt: &RollbackReceipt, deleted_at: i64) -> Result<()> {
        restore_from_system_trash(path, receipt, deleted_at)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FakeRestoreExecutor {
    restored: Arc<Mutex<Vec<PathBuf>>>,
}

impl FakeRestoreExecutor {
    #[must_use]
    pub fn restored_paths(&self) -> Vec<PathBuf> {
        self.restored.lock().expect("fake restore mutex").clone()
    }
}

impl RestoreExecutor for FakeRestoreExecutor {
    fn restore(&self, path: &Path, _receipt: &RollbackReceipt, _deleted_at: i64) -> Result<()> {
        self.restored
            .lock()
            .expect("fake restore mutex")
            .push(path.to_path_buf());
        Ok(())
    }
}

pub fn execute_cleanup_plan(
    plan: &CleanupPlan,
    executor: &impl CleanupExecutor,
    state_dir: impl AsRef<Path>,
    authorization: Option<&CleanupAuthorization>,
) -> Result<ExecutionManifest> {
    let authorization =
        authorization.context("cleanup requires explicit local user authorization")?;
    validate_recoverable_plan(plan)?;
    let repository = ManifestRepository::new(state_dir);
    let selected_items = plan
        .items
        .iter()
        .filter(|item| item.selected)
        .collect::<Vec<_>>();
    let items = selected_items
        .iter()
        .map(|item| ExecutionItem {
            path: item.path.clone(),
            planned_action: item.planned_action,
            status: ExecutionStatus::Pending,
            rule_id: item.rule_id.clone(),
            rollback_receipt: None,
            error: None,
        })
        .collect::<Vec<_>>();

    let mut manifest = ExecutionManifest {
        schema_version: EXECUTION_SCHEMA_VERSION.to_string(),
        run_id: Uuid::new_v4().to_string(),
        created_at: Utc::now(),
        plan_schema_version: plan.schema_version.clone(),
        authorization: Some(ExecutionAuthorization {
            source: authorization.source,
        }),
        summary: execution_summary(&items),
        items,
    };

    repository.write_execution(&manifest)?;
    for (index, item) in selected_items.iter().enumerate() {
        let result = validate_cleanup_target(item, plan).and_then(|()| executor.trash(&item.path));
        manifest.items[index] = match result {
            Ok(receipt) => ExecutionItem {
                path: item.path.clone(),
                planned_action: item.planned_action,
                status: ExecutionStatus::Trashed,
                rule_id: item.rule_id.clone(),
                rollback_receipt: Some(receipt),
                error: None,
            },
            Err(err) => ExecutionItem {
                path: item.path.clone(),
                planned_action: item.planned_action,
                status: ExecutionStatus::Failed,
                rule_id: item.rule_id.clone(),
                rollback_receipt: None,
                error: Some(err.to_string()),
            },
        };
        manifest.summary = execution_summary(&manifest.items);
        repository.write_execution(&manifest)?;
    }
    Ok(manifest)
}

fn execution_summary(items: &[ExecutionItem]) -> ExecutionSummary {
    ExecutionSummary {
        attempted: items
            .iter()
            .filter(|item| item.status != ExecutionStatus::Pending)
            .count(),
        succeeded: items
            .iter()
            .filter(|item| item.status == ExecutionStatus::Trashed)
            .count(),
        failed: items
            .iter()
            .filter(|item| item.status == ExecutionStatus::Failed)
            .count(),
    }
}

pub fn write_execution_manifest(
    manifest: &ExecutionManifest,
    state_dir: impl AsRef<Path>,
) -> Result<PathBuf> {
    ManifestRepository::new(state_dir).write_execution(manifest)
}

pub fn write_cleanup_plan(plan: &CleanupPlan, path: impl AsRef<Path>) -> Result<()> {
    atomic_write_json(path.as_ref(), plan)
}

pub fn list_execution_manifests(state_dir: impl AsRef<Path>) -> Result<Vec<ExecutionManifest>> {
    ManifestRepository::new(state_dir).list_executions()
}

pub fn restore_execution_manifest(
    manifest: &ExecutionManifest,
    executor: &impl RestoreExecutor,
    state_dir: impl AsRef<Path>,
) -> Result<RestoreManifest> {
    let repository = ManifestRepository::new(state_dir);
    let deleted_at = manifest.created_at.timestamp();
    let mut items = Vec::with_capacity(manifest.items.len());
    let already_restored = repository
        .list_restores()?
        .into_iter()
        .filter(|restore| restore.source_run_id == manifest.run_id)
        .flat_map(|restore| restore.items)
        .filter(|item| item.status == RestoreStatus::Restored)
        .map(|item| item.path)
        .collect::<HashSet<_>>();

    for item in &manifest.items {
        if already_restored.contains(&item.path) {
            items.push(RestoreItem {
                path: item.path.clone(),
                status: RestoreStatus::Skipped,
                error: Some("item was restored by an earlier restore run".to_string()),
            });
            continue;
        }
        if item.status != ExecutionStatus::Trashed {
            items.push(RestoreItem {
                path: item.path.clone(),
                status: RestoreStatus::Skipped,
                error: Some("cleanup item was not successfully moved to trash".to_string()),
            });
            continue;
        }

        let result = item.rollback_receipt.as_ref().map_or_else(
            || anyhow::bail!("cleanup manifest does not contain a rollback receipt"),
            |receipt| executor.restore(&item.path, receipt, deleted_at),
        );
        match result {
            Ok(()) => items.push(RestoreItem {
                path: item.path.clone(),
                status: RestoreStatus::Restored,
                error: None,
            }),
            Err(err) => items.push(RestoreItem {
                path: item.path.clone(),
                status: RestoreStatus::Failed,
                error: Some(err.to_string()),
            }),
        }
    }

    let summary = RestoreSummary {
        attempted: items
            .iter()
            .filter(|item| item.status != RestoreStatus::Skipped)
            .count(),
        succeeded: items
            .iter()
            .filter(|item| item.status == RestoreStatus::Restored)
            .count(),
        failed: items
            .iter()
            .filter(|item| item.status == RestoreStatus::Failed)
            .count(),
    };
    let restore = RestoreManifest {
        schema_version: RESTORE_SCHEMA_VERSION.to_string(),
        restore_id: Uuid::new_v4().to_string(),
        source_run_id: manifest.run_id.clone(),
        created_at: Utc::now(),
        summary,
        items,
    };
    repository.write_restore(&restore)?;
    Ok(restore)
}

pub fn write_restore_manifest(
    manifest: &RestoreManifest,
    state_dir: impl AsRef<Path>,
) -> Result<PathBuf> {
    ManifestRepository::new(state_dir).write_restore(manifest)
}

pub fn list_restore_manifests(state_dir: impl AsRef<Path>) -> Result<Vec<RestoreManifest>> {
    ManifestRepository::new(state_dir).list_restores()
}

#[derive(Debug, Clone)]
pub struct ManifestRepository {
    state_dir: PathBuf,
}

impl ManifestRepository {
    #[must_use]
    pub fn new(state_dir: impl AsRef<Path>) -> Self {
        Self {
            state_dir: state_dir.as_ref().to_path_buf(),
        }
    }

    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn write_execution(&self, manifest: &ExecutionManifest) -> Result<PathBuf> {
        let path = self.runs_dir().join(format!("{}.json", manifest.run_id));
        atomic_write_json(&path, manifest)?;
        Ok(path)
    }

    pub fn list_executions(&self) -> Result<Vec<ExecutionManifest>> {
        let mut manifests = list_json_manifests::<ExecutionManifest>(&self.runs_dir())?;
        manifests.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(manifests)
    }

    pub fn find_execution(&self, run_id: &str) -> Result<Option<ExecutionManifest>> {
        Ok(self
            .list_executions()?
            .into_iter()
            .find(|manifest| manifest.run_id == run_id))
    }

    pub fn write_restore(&self, manifest: &RestoreManifest) -> Result<PathBuf> {
        let path = self
            .restores_dir()
            .join(format!("{}.json", manifest.restore_id));
        atomic_write_json(&path, manifest)?;
        Ok(path)
    }

    pub fn list_restores(&self) -> Result<Vec<RestoreManifest>> {
        let mut manifests = list_json_manifests::<RestoreManifest>(&self.restores_dir())?;
        manifests.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(manifests)
    }

    pub fn history(&self) -> Result<(Vec<ExecutionManifest>, Vec<RestoreManifest>)> {
        Ok((self.list_executions()?, self.list_restores()?))
    }

    fn runs_dir(&self) -> PathBuf {
        self.state_dir.join("runs")
    }

    fn restores_dir(&self) -> PathBuf {
        self.state_dir.join("restores")
    }
}

fn list_json_manifests<T>(directory: &Path) -> Result<Vec<T>>
where
    T: DeserializeOwned,
{
    let paths = json_manifest_paths(directory)?;
    paths
        .iter()
        .map(|path| read_json_manifest(path))
        .collect::<Result<Vec<_>>>()
}

fn json_manifest_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_json_manifest<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

#[must_use]
pub fn restored_run_ids(manifests: &[RestoreManifest]) -> HashSet<&str> {
    manifests
        .iter()
        .filter(|manifest| manifest.summary.failed == 0 && manifest.summary.succeeded > 0)
        .map(|manifest| manifest.source_run_id.as_str())
        .collect()
}

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn trash_with_receipt(path: &Path) -> Result<RollbackReceipt> {
    let absolute = absolute_path(path)?;
    let before = trash::os_limited::list()
        .unwrap_or_default()
        .into_iter()
        .map(|item| encode_os_string(&item.id))
        .collect::<HashSet<_>>();

    trash::delete(&absolute)
        .with_context(|| format!("failed to move {} to trash", absolute.display()))?;

    let locator = trash::os_limited::list().ok().and_then(|items| {
        items
            .into_iter()
            .filter(|item| item.original_path() == absolute)
            .filter(|item| !before.contains(&encode_os_string(&item.id)))
            .max_by_key(|item| item.time_deleted)
            .map(|item| format!("trash-id:{}", encode_os_string(&item.id)))
    });
    Ok(RollbackReceipt {
        method: "system-trash".to_string(),
        note: if locator.is_some() {
            "Moved to the operating system trash with a restorable item locator.".to_string()
        } else {
            "Moved to the operating system trash; restore will match the original path and deletion time."
                .to_string()
        },
        locator,
    })
}

#[cfg(target_os = "macos")]
fn trash_with_receipt(path: &Path) -> Result<RollbackReceipt> {
    use std::process::Command;

    let absolute = absolute_path(path)?;
    let script = "on run argv\n\
        tell application \"Finder\"\n\
        set trashedItem to delete POSIX file (item 1 of argv)\n\
        set trashedAlias to trashedItem as alias\n\
        return POSIX path of trashedAlias\n\
        end tell\n\
        end run";
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .arg(&absolute)
        .output()
        .context("failed to start Finder trash operation")?;
    if !output.status.success() {
        anyhow::bail!(
            "failed to move {} to trash: {}",
            absolute.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let trashed_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trashed_path.is_empty() {
        anyhow::bail!("Finder did not return the trashed item location");
    }
    Ok(RollbackReceipt {
        method: "system-trash".to_string(),
        note: "Moved to the macOS Trash with the exact Finder trash location recorded.".to_string(),
        locator: Some(format!("mac-path:{trashed_path}")),
    })
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    all(unix, not(target_os = "ios"), not(target_os = "android"))
)))]
fn trash_with_receipt(path: &Path) -> Result<RollbackReceipt> {
    trash::delete(path).with_context(|| format!("failed to move {} to trash", path.display()))?;
    Ok(RollbackReceipt {
        method: "system-trash".to_string(),
        note: "Moved to the operating system trash.".to_string(),
        locator: None,
    })
}

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn restore_from_system_trash(
    path: &Path,
    receipt: &RollbackReceipt,
    deleted_at: i64,
) -> Result<()> {
    let expected_locator = receipt
        .locator
        .as_deref()
        .and_then(|locator| locator.strip_prefix("trash-id:"));
    let mut matching = trash::os_limited::list()
        .context("failed to list the operating system trash")?
        .into_iter()
        .filter(|item| {
            expected_locator.map_or_else(
                || item.original_path() == path,
                |locator| encode_os_string(&item.id) == locator,
            )
        })
        .collect::<Vec<_>>();

    if matching.is_empty() {
        anyhow::bail!("the item is no longer present in the operating system trash");
    }
    matching.sort_by_key(|item| item.time_deleted.abs_diff(deleted_at));
    let item = matching.remove(0);
    trash::os_limited::restore_all([item])
        .with_context(|| format!("failed to restore {}", path.display()))
}

#[cfg(target_os = "macos")]
fn restore_from_system_trash(
    path: &Path,
    receipt: &RollbackReceipt,
    _deleted_at: i64,
) -> Result<()> {
    let trashed_path = receipt
        .locator
        .as_deref()
        .and_then(|locator| locator.strip_prefix("mac-path:"))
        .map(PathBuf::from)
        .context("cleanup manifest does not contain a macOS trash locator")?;
    if path.try_exists()? {
        anyhow::bail!("restore target already exists: {}", path.display());
    }
    if !trashed_path.try_exists()? {
        anyhow::bail!(
            "the item is no longer present in the macOS Trash: {}",
            trashed_path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to recreate {}", parent.display()))?;
    }
    fs::rename(&trashed_path, path).with_context(|| {
        format!(
            "failed to restore {} from {}",
            path.display(),
            trashed_path.display()
        )
    })
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    all(unix, not(target_os = "ios"), not(target_os = "android"))
)))]
fn restore_from_system_trash(
    path: &Path,
    _receipt: &RollbackReceipt,
    _deleted_at: i64,
) -> Result<()> {
    anyhow::bail!(
        "programmatic restore is unsupported on this platform for {}",
        path.display()
    )
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute
        .parent()
        .context("cleanup target has no parent directory")?
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", absolute.display()))?;
    let name = absolute
        .file_name()
        .context("cleanup target has no file name")?;
    Ok(parent.join(name))
}

#[derive(Default)]
struct HardlinkTracker {
    seen: HashSet<FileIdentity>,
}

impl HardlinkTracker {
    fn insert(&mut self, metadata: &fs::Metadata) -> bool {
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
fn file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    (metadata.nlink() > 1).then_some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> Option<FileIdentity> {
    None
}

fn validate_cleanup_target(item: &CleanupItem, plan: &CleanupPlan) -> Result<()> {
    if item.kind == EntryKind::Symlink {
        anyhow::bail!("refusing to clean a symbolic link: {}", item.path.display());
    }

    let absolute = absolute_path(&item.path)?;
    if absolute.parent().is_none() {
        anyhow::bail!(
            "refusing to clean a filesystem root: {}",
            absolute.display()
        );
    }

    let within_scan_root = plan.scan_roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        absolute != root && absolute.starts_with(root)
    });
    if !within_scan_root {
        anyhow::bail!(
            "cleanup target is outside the scanned roots: {}",
            absolute.display()
        );
    }

    if plan.safety.protected_paths.iter().any(|protected| {
        protected
            .canonicalize()
            .unwrap_or_else(|_| protected.clone())
            .starts_with(&absolute)
    }) {
        anyhow::bail!(
            "cleanup target contains a protected path: {}",
            absolute.display()
        );
    }
    if plan.safety.protected_subtrees.iter().any(|protected| {
        let protected = protected
            .canonicalize()
            .unwrap_or_else(|_| protected.clone());
        protected.starts_with(&absolute) || absolute.starts_with(protected)
    }) {
        anyhow::bail!(
            "cleanup target overlaps a protected subtree: {}",
            absolute.display()
        );
    }

    let metadata = absolute.symlink_metadata().with_context(|| {
        format!(
            "cleanup target changed or disappeared: {}",
            absolute.display()
        )
    })?;
    let actual_kind = if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    if actual_kind != item.kind {
        anyhow::bail!(
            "cleanup target type changed since the scan: {}",
            absolute.display()
        );
    }
    if item.kind == EntryKind::File && metadata.len() != item.size_bytes {
        anyhow::bail!(
            "cleanup target size changed since the scan: {}",
            absolute.display()
        );
    }
    if item.kind == EntryKind::Directory
        && let Some(expected) = &item.tree_fingerprint
    {
        let actual = directory_fingerprint(&absolute)?;
        if !directory_fingerprint_matches(expected, &actual) {
            anyhow::bail!(
                "cleanup target contents changed since the scan: {}",
                absolute.display()
            );
        }
    }
    if let Some(expected) = item.modified_at
        && metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<Utc>::from)
            .is_some_and(|actual| actual != expected)
    {
        anyhow::bail!(
            "cleanup target was modified after the scan: {}",
            absolute.display()
        );
    }
    Ok(())
}

fn validate_recoverable_plan(plan: &CleanupPlan) -> Result<()> {
    if plan.schema_version != CLEANUP_PLAN_SCHEMA_VERSION {
        anyhow::bail!("unsupported cleanup plan schema: {}", plan.schema_version);
    }
    if plan
        .source_scan
        .as_ref()
        .is_some_and(|source| !source.budget_exceeded.is_empty())
    {
        anyhow::bail!(
            "cleanup plan came from a scan that exceeded a budget and is read-only; it cannot be executed"
        );
    }
    if plan.safety.default_action != PlannedAction::Trash
        || plan.safety.rollback_method != "system-trash+manifest"
    {
        anyhow::bail!("cleanup plan is not recoverable through system trash and a manifest");
    }
    if plan.items.iter().filter(|item| item.selected).any(|item| {
        item.planned_action != PlannedAction::Trash
            || item.rollback_method != "system-trash+manifest"
    }) {
        anyhow::bail!("selected cleanup items must use system trash and a manifest");
    }
    let selected_count = plan.items.iter().filter(|item| item.selected).count();
    let selected_size_bytes = plan
        .items
        .iter()
        .filter(|item| item.selected)
        .map(|item| item.size_bytes)
        .sum::<u64>();
    if plan.summary.selected_count != selected_count
        || plan.summary.selected_size_bytes != selected_size_bytes
        || plan.summary.candidate_count != plan.items.len()
    {
        anyhow::bail!("cleanup plan summary does not match its items");
    }
    Ok(())
}

fn directory_fingerprint_matches(
    expected: &CleanupItemFingerprint,
    actual: &CleanupItemFingerprint,
) -> bool {
    expected.descendants == actual.descendants
        && expected.total_size_bytes == actual.total_size_bytes
        && expected
            .latest_modified_at
            .is_none_or(|expected| Some(expected) == actual.latest_modified_at)
}

fn directory_fingerprint(path: &Path) -> Result<CleanupItemFingerprint> {
    let root_metadata = path
        .symlink_metadata()
        .with_context(|| format!("cleanup target changed or disappeared: {}", path.display()))?;
    let mut fingerprint = CleanupItemFingerprint {
        descendants: 0,
        total_size_bytes: 0,
        latest_modified_at: root_metadata.modified().ok().map(DateTime::<Utc>::from),
    };
    let mut hardlinks = HardlinkTracker::default();
    let mut stack = vec![path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory {}", dir.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
            let child = entry.path();
            let metadata = child.symlink_metadata().with_context(|| {
                format!(
                    "cleanup target contents changed or disappeared: {}",
                    child.display()
                )
            })?;
            fingerprint.descendants += 1;
            fingerprint.latest_modified_at = max_datetime(
                fingerprint.latest_modified_at,
                metadata.modified().ok().map(DateTime::<Utc>::from),
            );

            if metadata.file_type().is_dir() {
                stack.push(child);
            } else if metadata.file_type().is_file() && hardlinks.insert(&metadata) {
                fingerprint.total_size_bytes =
                    fingerprint.total_size_bytes.saturating_add(metadata.len());
            }
        }
    }

    Ok(fingerprint)
}

fn max_datetime(
    left: Option<DateTime<Utc>>,
    right: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn atomic_write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let raw = serde_json::to_vec_pretty(value)?;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .with_context(|| format!("failed to create temporary file in {}", directory.display()))?;
    temporary.write_all(&raw)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn encode_os_string(value: &std::ffi::OsStr) -> String {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        return value
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .map(|byte| format!("{byte:02x}"))
            .collect();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanr_core::{
        AnalysisScanContext, CleanupPlanSourceScan, Confidence, EntryKind, RecommendationPolicy,
        RecommendationState, ReportIntegrity, RuleTrust, SafetyPolicy, ScanBudgetExceeded,
        ScanIssue, ScanIssueCode, UserSelection, build_analysis_report_with_scan_context,
        build_cleanup_plan, build_cleanup_plan_from_analysis, build_cleanup_plan_with_policy,
    };
    use cleanr_core::{RuleHit, ScanEntry};

    fn cleanup_entry(path: PathBuf, kind: EntryKind, size_bytes: u64) -> ScanEntry {
        ScanEntry {
            path,
            kind,
            size_bytes,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "generated".into(),
                label: "Generated".into(),
                category: "build-cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: RuleTrust::Builtin,
                match_role: cleanr_core::RuleMatchRole::Primary,
                sources: Vec::new(),
            }],
        }
    }

    fn restorable_manifest(run_id: &str, path: PathBuf) -> ExecutionManifest {
        ExecutionManifest {
            schema_version: EXECUTION_SCHEMA_VERSION.to_string(),
            run_id: run_id.to_string(),
            created_at: Utc::now(),
            plan_schema_version: "plan".to_string(),
            authorization: None,
            summary: ExecutionSummary {
                attempted: 1,
                succeeded: 1,
                failed: 0,
            },
            items: vec![ExecutionItem {
                path,
                planned_action: PlannedAction::Trash,
                status: ExecutionStatus::Trashed,
                rule_id: "test".to_string(),
                rollback_receipt: Some(RollbackReceipt {
                    method: "fake-trash".to_string(),
                    note: "test".to_string(),
                    locator: Some("fake".to_string()),
                }),
                error: None,
            }],
        }
    }

    #[test]
    fn fake_clean_writes_execution_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("create target");
        let entry = ScanEntry {
            path: temp.path().join("target"),
            kind: EntryKind::Directory,
            size_bytes: 0,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "rust-target".into(),
                label: "Rust target".into(),
                category: "build-cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: cleanr_core::RuleTrust::Builtin,
                match_role: cleanr_core::RuleMatchRole::Primary,
                sources: Vec::new(),
            }],
        };
        let plan = build_cleanup_plan(vec![temp.path().to_path_buf()], vec![], &[entry]);
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_delegation();
        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.succeeded, 1);
        assert_eq!(manifest.items[0].planned_action, PlannedAction::Trash);
        assert_eq!(
            manifest.authorization,
            Some(ExecutionAuthorization {
                source: CleanupAuthorizationSource::ExplicitUserDelegation,
            })
        );
        assert_eq!(fake.trashed_paths().len(), 1);
        assert_eq!(
            list_execution_manifests(temp.path()).expect("list").len(),
            1
        );
    }

    #[test]
    fn manifest_repository_round_trips_history_and_finds_runs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repository = ManifestRepository::new(temp.path());
        let mut older = restorable_manifest("run-old", temp.path().join("old"));
        older.created_at = Utc::now() - chrono::Duration::seconds(60);
        let newer = restorable_manifest("run-new", temp.path().join("new"));
        repository
            .write_execution(&older)
            .expect("write older execution");
        repository
            .write_execution(&newer)
            .expect("write newer execution");
        let restore = RestoreManifest {
            schema_version: RESTORE_SCHEMA_VERSION.to_string(),
            restore_id: "restore-1".to_string(),
            source_run_id: "run-old".to_string(),
            created_at: Utc::now(),
            summary: RestoreSummary {
                attempted: 0,
                succeeded: 0,
                failed: 0,
            },
            items: Vec::new(),
        };
        repository.write_restore(&restore).expect("write restore");

        let (executions, restores) = repository.history().expect("history");

        assert_eq!(executions.len(), 2);
        assert_eq!(executions[0].run_id, "run-new");
        assert_eq!(executions[1].run_id, "run-old");
        assert_eq!(restores.len(), 1);
        assert_eq!(restores[0].restore_id, "restore-1");
        assert_eq!(
            repository
                .find_execution("run-old")
                .expect("find")
                .expect("run")
                .run_id,
            "run-old"
        );
        assert!(
            repository
                .find_execution("missing")
                .expect("find")
                .is_none()
        );
    }

    #[test]
    fn execution_manifest_is_journaled_before_each_cleanup_item() {
        struct JournalInspectingExecutor {
            state_dir: PathBuf,
            calls: Mutex<usize>,
        }

        impl CleanupExecutor for JournalInspectingExecutor {
            fn trash(&self, path: &Path) -> Result<RollbackReceipt> {
                let mut calls = self.calls.lock().expect("calls mutex");
                *calls += 1;
                let manifests = list_execution_manifests(&self.state_dir).expect("journal");
                assert_eq!(manifests.len(), 1);
                let manifest = &manifests[0];
                match *calls {
                    1 => {
                        assert_eq!(manifest.summary.attempted, 0);
                        assert!(
                            manifest
                                .items
                                .iter()
                                .all(|item| item.status == ExecutionStatus::Pending)
                        );
                    }
                    2 => {
                        assert_eq!(manifest.summary.attempted, 1);
                        assert_eq!(manifest.summary.succeeded, 1);
                        assert_eq!(manifest.items[0].status, ExecutionStatus::Trashed);
                        assert_eq!(manifest.items[1].status, ExecutionStatus::Pending);
                    }
                    _ => unreachable!("test only creates two cleanup items"),
                }

                Ok(RollbackReceipt {
                    method: "fake-trash".to_string(),
                    note: "journal test".to_string(),
                    locator: Some(format!("fake:{}", path.display())),
                })
            }
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first.bin");
        let second = temp.path().join("second.bin");
        fs::write(&first, b"one").expect("first");
        fs::write(&second, b"two").expect("second");
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[
                cleanup_entry(first, EntryKind::File, 3),
                cleanup_entry(second, EntryKind::File, 3),
            ],
        );
        let executor = JournalInspectingExecutor {
            state_dir: temp.path().to_path_buf(),
            calls: Mutex::new(0),
        };
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest = execute_cleanup_plan(&plan, &executor, temp.path(), Some(&authorization))
            .expect("execute");

        assert_eq!(manifest.summary.attempted, 2);
        assert_eq!(manifest.summary.succeeded, 2);
        assert!(
            manifest
                .items
                .iter()
                .all(|item| item.status == ExecutionStatus::Trashed)
        );
    }

    #[test]
    fn cleanup_requires_user_authorization_without_a_confirmation_dialog() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("create target");
        let entry = ScanEntry {
            path: temp.path().join("target"),
            kind: EntryKind::Directory,
            size_bytes: 0,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-dev".into(),
                rule_id: "rust-target".into(),
                label: "Rust target".into(),
                category: "build-cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "rebuild".into(),
                default_selected: true,
                trust: cleanr_core::RuleTrust::Builtin,
                match_role: cleanr_core::RuleMatchRole::Primary,
                sources: Vec::new(),
            }],
        };
        let policy = cleanr_core::SafetyPolicy::new(vec![], false);
        let plan = cleanr_core::build_cleanup_plan_with_policy(
            vec![temp.path().to_path_buf()],
            vec![],
            &[entry],
            &policy,
        );
        let fake = FakeTrashExecutor::default();

        let error = execute_cleanup_plan(&plan, &fake, temp.path(), None)
            .expect_err("cleanup without local authorization must be denied");
        assert!(error.to_string().contains("user authorization"));
        assert!(fake.trashed_paths().is_empty());
    }

    #[test]
    fn cleanup_rejects_budget_exhausted_plan_before_manifest_or_trash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("create target");
        let entries = vec![cleanup_entry(target, EntryKind::Directory, 0)];
        let policy = SafetyPolicy::new(vec![], false);
        let budgets = [ScanBudgetExceeded::EntryCount {
            limit: 1,
            observed: 2,
        }];
        let analysis = build_analysis_report_with_scan_context(
            Utc::now(),
            Utc::now(),
            vec![temp.path().to_path_buf()],
            &entries,
            &[],
            RecommendationPolicy::default(),
            AnalysisScanContext {
                budget_exceeded: &budgets,
                safety_policy: Some(&policy),
            },
        )
        .expect("analysis");
        let mut plan = build_cleanup_plan_with_policy(
            vec![temp.path().to_path_buf()],
            vec![],
            &entries,
            &policy,
        );
        plan.source_scan = Some(CleanupPlanSourceScan {
            analysis_id: analysis.analysis_id,
            integrity: analysis.scan.integrity,
            budget_exceeded: analysis.scan.budget_exceeded,
        });
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let error = execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization))
            .expect_err("budget-exhausted plan must be denied");

        assert!(error.to_string().contains("read-only"));
        assert!(fake.trashed_paths().is_empty());
        assert!(
            list_execution_manifests(temp.path())
                .expect("list manifests")
                .is_empty()
        );
    }

    #[test]
    fn cleanup_allows_explicit_selection_from_an_ordinary_partial_scan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("create target");
        let entries = vec![cleanup_entry(target.clone(), EntryKind::Directory, 0)];
        let policy = SafetyPolicy::new(vec![], false);
        let analysis = build_analysis_report_with_scan_context(
            Utc::now(),
            Utc::now(),
            vec![temp.path().to_path_buf()],
            &entries,
            &[ScanIssue {
                code: ScanIssueCode::MetadataUnavailable,
                path: Some(target.join("unreadable")),
            }],
            RecommendationPolicy::default(),
            AnalysisScanContext {
                budget_exceeded: &[],
                safety_policy: Some(&policy),
            },
        )
        .expect("ordinary partial analysis");
        assert_eq!(analysis.scan.integrity, ReportIntegrity::Partial);
        assert!(analysis.scan.budget_exceeded.is_empty());
        assert_eq!(
            analysis.candidates[0].recommendation.state,
            RecommendationState::Review
        );
        let mut selection = UserSelection::default();
        selection.select(analysis.candidates[0].id.clone());
        let plan = build_cleanup_plan_from_analysis(
            vec![temp.path().to_path_buf()],
            vec![],
            &entries,
            &analysis,
            &selection,
            &policy,
        )
        .expect("ordinary partial analysis may be explicitly selected");
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest = execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization))
            .expect("ordinary partial plan remains executable after explicit selection");

        assert_eq!(manifest.summary.succeeded, 1);
        assert_eq!(fake.trashed_paths(), vec![target]);
    }

    #[test]
    fn fake_restore_writes_restore_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("target");
        let manifest = restorable_manifest("run-1", source.clone());
        let fake = FakeRestoreExecutor::default();
        let restored = restore_execution_manifest(&manifest, &fake, temp.path()).expect("restore");

        assert_eq!(restored.summary.succeeded, 1);
        assert_eq!(fake.restored_paths(), vec![source]);
        assert_eq!(list_restore_manifests(temp.path()).expect("list").len(), 1);
    }

    #[test]
    fn changed_file_is_recorded_as_failure_without_calling_executor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("artifact");
        fs::write(&target, b"old").expect("seed file");
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[cleanup_entry(target.clone(), EntryKind::File, 3)],
        );
        fs::write(&target, b"changed").expect("change file");
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.failed, 1);
        assert!(
            manifest.items[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("size changed"))
        );
        assert!(fake.trashed_paths().is_empty());
    }

    #[test]
    fn changed_directory_contents_are_rejected_before_trash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let child = target.join("artifact");
        fs::create_dir(&target).expect("target");
        fs::write(&child, b"old").expect("seed child");
        let target_metadata = target.symlink_metadata().expect("target metadata");
        let child_metadata = child.symlink_metadata().expect("child metadata");
        let mut target_entry = cleanup_entry(target.clone(), EntryKind::Directory, 3);
        target_entry.modified_at = target_metadata.modified().ok().map(DateTime::<Utc>::from);
        let child_entry = ScanEntry {
            path: child,
            kind: EntryKind::File,
            size_bytes: 3,
            modified_at: child_metadata.modified().ok().map(DateTime::<Utc>::from),
            rule_hits: Vec::new(),
        };
        let plan = build_cleanup_plan(
            vec![temp.path().to_path_buf()],
            vec![],
            &[target_entry, child_entry],
        );
        fs::write(target.join("new-artifact"), b"new").expect("new child");
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.failed, 1);
        assert!(
            manifest.items[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("contents changed"))
        );
        assert!(fake.trashed_paths().is_empty());
    }

    #[test]
    fn protected_target_is_revalidated_at_execution_time() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("target");
        let mut plan = build_cleanup_plan_with_policy(
            vec![temp.path().to_path_buf()],
            vec![],
            &[cleanup_entry(target.clone(), EntryKind::Directory, 0)],
            &SafetyPolicy::new(vec![], true),
        );
        plan.safety.protected_subtrees.push(target);
        let fake = FakeTrashExecutor::default();
        let authorization = CleanupAuthorization::explicit_user_confirmation();

        let manifest =
            execute_cleanup_plan(&plan, &fake, temp.path(), Some(&authorization)).expect("execute");

        assert_eq!(manifest.summary.failed, 1);
        assert!(
            manifest.items[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("protected subtree"))
        );
    }

    #[test]
    fn repeated_restore_skips_items_already_restored() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("target");
        let manifest = restorable_manifest("run-repeat", source.clone());
        let fake = FakeRestoreExecutor::default();

        let first =
            restore_execution_manifest(&manifest, &fake, temp.path()).expect("first restore");
        let second =
            restore_execution_manifest(&manifest, &fake, temp.path()).expect("second restore");

        assert_eq!(first.summary.succeeded, 1);
        assert_eq!(second.summary.attempted, 0);
        assert_eq!(second.items[0].status, RestoreStatus::Skipped);
        assert_eq!(fake.restored_paths(), vec![source]);
    }

    #[test]
    fn restore_reports_missing_receipts_and_skips_failed_cleanup_items() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut manifest = restorable_manifest("run-invalid", temp.path().join("missing-receipt"));
        manifest.items[0].rollback_receipt = None;
        manifest.items.push(ExecutionItem {
            path: temp.path().join("never-trashed"),
            planned_action: PlannedAction::Trash,
            status: ExecutionStatus::Failed,
            rule_id: "test".to_string(),
            rollback_receipt: None,
            error: Some("cleanup failed".to_string()),
        });
        let fake = FakeRestoreExecutor::default();

        let restore =
            restore_execution_manifest(&manifest, &fake, temp.path()).expect("restore manifest");

        assert_eq!(restore.summary.attempted, 1);
        assert_eq!(restore.summary.failed, 1);
        assert_eq!(restore.items[0].status, RestoreStatus::Failed);
        assert_eq!(restore.items[1].status, RestoreStatus::Skipped);
        assert!(fake.restored_paths().is_empty());
    }

    #[test]
    fn restored_run_ids_require_at_least_one_success_and_no_failures() {
        let restore = |run_id: &str, succeeded: usize, failed: usize| RestoreManifest {
            schema_version: RESTORE_SCHEMA_VERSION.to_string(),
            restore_id: format!("restore-{run_id}"),
            source_run_id: run_id.to_string(),
            created_at: Utc::now(),
            summary: RestoreSummary {
                attempted: succeeded + failed,
                succeeded,
                failed,
            },
            items: vec![],
        };
        let manifests = vec![
            restore("complete", 1, 0),
            restore("partial", 1, 1),
            restore("empty", 0, 0),
        ];

        assert_eq!(restored_run_ids(&manifests), HashSet::from(["complete"]));
    }
}
