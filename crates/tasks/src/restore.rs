use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::Result;
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

#[must_use]
pub fn restored_run_ids(manifests: &[RestoreManifest]) -> HashSet<&str> {
    manifests
        .iter()
        .filter(|manifest| manifest.summary.failed == 0 && manifest.summary.succeeded > 0)
        .map(|manifest| manifest.source_run_id.as_str())
        .collect()
}
