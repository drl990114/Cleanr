use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::Utc;
use cleanr_core::{
    ExecutionManifest, ExecutionStatus, RESTORE_SCHEMA_VERSION, RestoreItem, RestoreManifest,
    RestoreStatus, RestoreSummary, RollbackReceipt,
};
use uuid::Uuid;

use crate::{ManifestRepository, platform::restore_from_system_trash};

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

pub fn restore_execution_manifest(
    manifest: &ExecutionManifest,
    executor: &impl RestoreExecutor,
    state_dir: impl AsRef<Path>,
) -> Result<RestoreManifest> {
    let repository = ManifestRepository::new(state_dir);
    let _operation_lock = repository.lock_operations()?;
    let deleted_at = manifest.created_at.timestamp();
    let history = repository
        .list_restores()?
        .into_iter()
        .filter(|restore| restore.source_run_id == manifest.run_id)
        .flat_map(|restore| restore.items)
        .collect::<Vec<_>>();
    let already_restored = history
        .iter()
        .filter(|item| item.status == RestoreStatus::Restored)
        .map(|item| item.path.clone())
        .collect::<HashSet<_>>();
    let unresolved = history
        .iter()
        .filter(|item| item.status == RestoreStatus::Pending)
        .map(|item| item.path.clone())
        .collect::<HashSet<_>>();
    let items = manifest
        .items
        .iter()
        .map(|item| {
            let (status, error) = if already_restored.contains(&item.path) {
                (RestoreStatus::Skipped, Some("item was restored by an earlier restore run"))
            } else if unresolved.contains(&item.path) {
                (RestoreStatus::Failed, Some("an earlier restore started but its outcome was not recorded; inspect the original path, system trash, and restore history before manual recovery; automatic retry is blocked"))
            } else if item.status == ExecutionStatus::Pending {
                (RestoreStatus::Failed, Some("cleanup outcome was not recorded; the item may already be in system trash; inspect the original path, system trash, and cleanup manifest before manual recovery"))
            } else if item.status != ExecutionStatus::Trashed {
                (RestoreStatus::Skipped, Some("cleanup item was not successfully moved to trash"))
            } else if item.rollback_receipt.is_none() {
                (RestoreStatus::Failed, Some("cleanup manifest does not contain a rollback receipt"))
            } else {
                (RestoreStatus::NotAttempted, None)
            };
            RestoreItem {
                path: item.path.clone(),
                status,
                error: error.map(str::to_string),
            }
        })
        .collect::<Vec<_>>();
    let mut restore = RestoreManifest {
        schema_version: RESTORE_SCHEMA_VERSION.to_string(),
        restore_id: Uuid::new_v4().to_string(),
        source_run_id: manifest.run_id.clone(),
        created_at: Utc::now(),
        summary: restore_summary(&items),
        items,
    };
    repository.write_restore(&restore)?;

    for (index, item) in manifest.items.iter().enumerate() {
        if restore.items[index].status != RestoreStatus::NotAttempted {
            continue;
        }
        restore.items[index].status = RestoreStatus::Pending;
        restore.items[index].error = Some(
            "restore started but its outcome has not been recorded; inspect the original path and system trash before retrying".to_string(),
        );
        restore.summary = restore_summary(&restore.items);
        repository.write_restore(&restore).with_context(|| {
            format!(
                "restore {} stopped before attempting {}; no further items were attempted",
                restore.restore_id,
                item.path.display()
            )
        })?;
        let result = item.rollback_receipt.as_ref().map_or_else(
            || anyhow::bail!("cleanup manifest does not contain a rollback receipt"),
            |receipt| executor.restore(&item.path, receipt, deleted_at),
        );
        restore.items[index] = match result {
            Ok(()) => RestoreItem {
                path: item.path.clone(),
                status: RestoreStatus::Restored,
                error: None,
            },
            Err(err) => RestoreItem {
                path: item.path.clone(),
                status: RestoreStatus::Failed,
                error: Some(err.to_string()),
            },
        };
        restore.summary = restore_summary(&restore.items);
        repository.write_restore(&restore).with_context(|| {
            format!(
                "restore {} could not record the outcome for {} (executor reported {:?}); no further items were attempted; inspect the original path and system trash before manual recovery",
                restore.restore_id, item.path.display(), restore.items[index].status,
            )
        })?;
    }
    Ok(restore)
}

fn restore_summary(items: &[RestoreItem]) -> RestoreSummary {
    RestoreSummary {
        attempted: items
            .iter()
            .filter(|item| {
                matches!(
                    item.status,
                    RestoreStatus::Pending | RestoreStatus::Restored | RestoreStatus::Failed
                )
            })
            .count(),
        succeeded: items
            .iter()
            .filter(|item| item.status == RestoreStatus::Restored)
            .count(),
        failed: items
            .iter()
            .filter(|item| item.status == RestoreStatus::Failed)
            .count(),
        pending: items
            .iter()
            .filter(|item| item.status == RestoreStatus::Pending)
            .count(),
        not_attempted: items
            .iter()
            .filter(|item| item.status == RestoreStatus::NotAttempted)
            .count(),
    }
}

#[must_use]
pub fn restored_run_ids(manifests: &[RestoreManifest]) -> HashSet<&str> {
    manifests
        .iter()
        .filter(|manifest| {
            manifest.summary.failed == 0
                && manifest.summary.succeeded > 0
                && manifest.items.iter().all(|item| {
                    !matches!(
                        item.status,
                        RestoreStatus::Pending | RestoreStatus::NotAttempted
                    )
                })
        })
        .map(|manifest| manifest.source_run_id.as_str())
        .collect()
}
