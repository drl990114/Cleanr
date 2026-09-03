use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cleanr_core::{
    CLEANUP_PLAN_SCHEMA_VERSION, CleanupAuthorizationSource, CleanupItem, CleanupItemFingerprint,
    CleanupPlan, EXECUTION_SCHEMA_VERSION, EntryKind, ExecutionAuthorization, ExecutionItem,
    ExecutionManifest, ExecutionStatus, ExecutionSummary, PlannedAction, RollbackReceipt,
};
use uuid::Uuid;

use crate::{ManifestRepository, platform::absolute_path, platform::trash_with_receipt};

pub trait CleanupExecutor {
    fn trash(&self, path: &Path) -> Result<RollbackReceipt>;
}

#[derive(Debug)]
pub(crate) struct CleanupAuthorization {
    source: CleanupAuthorizationSource,
}

impl CleanupAuthorization {
    #[must_use]
    pub(crate) fn explicit_user_confirmation() -> Self {
        Self {
            source: CleanupAuthorizationSource::LocalUserConfirmation,
        }
    }

    #[must_use]
    pub(crate) fn explicit_user_delegation() -> Self {
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

pub(crate) fn execute_cleanup_plan(
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

/// Execute a plan immediately after the local TUI has collected an explicit confirmation.
///
/// Agent-delegated execution must use [`crate::execute_delegated_cleanup`], which additionally
/// binds the authorization to the reviewed plan digest and re-scans before reaching this executor.
pub fn execute_locally_confirmed_plan(
    plan: &CleanupPlan,
    state_dir: impl AsRef<Path>,
) -> Result<ExecutionManifest> {
    execute_locally_confirmed_plan_with_executor(plan, &TrashExecutor, state_dir)
}

#[doc(hidden)]
pub fn execute_locally_confirmed_plan_with_executor(
    plan: &CleanupPlan,
    executor: &impl CleanupExecutor,
    state_dir: impl AsRef<Path>,
) -> Result<ExecutionManifest> {
    let authorization = CleanupAuthorization::explicit_user_confirmation();
    execute_cleanup_plan(plan, executor, state_dir, Some(&authorization))
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
            .map(DateTime::<Utc>::from)
            .is_some_and(|actual| actual != expected)
    {
        anyhow::bail!(
            "cleanup target was modified after the scan: {}",
            absolute.display()
        );
    }
    Ok(())
}

pub(crate) fn validate_recoverable_plan(plan: &CleanupPlan) -> Result<()> {
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
