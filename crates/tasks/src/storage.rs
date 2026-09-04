use std::{
    cmp::Reverse,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use cleanr_core::{CleanupPlan, ExecutionManifest, RESTORE_SCHEMA_VERSION, RestoreManifest};
use serde::de::DeserializeOwned;

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

    /// Hold across history lookup, filesystem mutation, and the final journal write.
    /// The OS releases the lock when this handle closes, including after a crash.
    pub(crate) fn lock_operations(&self) -> Result<File> {
        if self.state_dir.as_os_str().is_empty() {
            anyhow::bail!("cleanup and restore require a non-empty state directory");
        }
        fs::create_dir_all(&self.state_dir)
            .with_context(|| format!("failed to create {}", self.state_dir.display()))?;
        let path = self.state_dir.join("operation.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open operation lock {}", path.display()))?;
        lock.try_lock().with_context(|| {
            format!(
                "another cleanup or restore may be running; could not lock {}",
                path.display()
            )
        })?;
        Ok(lock)
    }

    pub fn write_execution(&self, manifest: &ExecutionManifest) -> Result<PathBuf> {
        let path = self.runs_dir().join(format!("{}.json", manifest.run_id));
        atomic_write_json(&path, manifest)?;
        Ok(path)
    }

    pub fn list_executions(&self) -> Result<Vec<ExecutionManifest>> {
        let mut manifests = list_json_manifests::<ExecutionManifest>(&self.runs_dir())?;
        manifests.sort_by_key(|manifest| Reverse(manifest.created_at));
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
        for manifest in &manifests {
            if manifest.schema_version != "cleanr.restore.v1"
                && manifest.schema_version != RESTORE_SCHEMA_VERSION
            {
                anyhow::bail!(
                    "unsupported restore manifest schema: {}",
                    manifest.schema_version
                );
            }
        }
        manifests.sort_by_key(|manifest| Reverse(manifest.created_at));
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
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to read manifest history {}", directory.display())
            });
        }
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

pub(super) fn atomic_write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
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
