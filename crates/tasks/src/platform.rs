use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::fs;

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
use std::collections::HashSet;

use anyhow::{Context, Result};
use cleanr_core::RollbackReceipt;

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub(super) fn trash_with_receipt(path: &Path) -> Result<RollbackReceipt> {
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
pub(super) fn trash_with_receipt(path: &Path) -> Result<RollbackReceipt> {
    use objc2_foundation::{NSFileManager, NSURL};

    let absolute = absolute_path(path)?;
    let source_url = NSURL::from_file_path(&absolute)
        .with_context(|| format!("failed to create a file URL for {}", absolute.display()))?;
    let mut resulting_url = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&source_url, Some(&mut resulting_url))
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to move {} to the macOS Trash: {error}",
                absolute.display()
            )
        })?;
    let trashed_path = resulting_url
        .context("macOS did not return the trashed item location")?
        .to_file_path()
        .context("macOS returned an invalid trashed item location")?;

    Ok(RollbackReceipt {
        method: "system-trash".to_string(),
        note: "Moved to the macOS Trash with the exact system trash location recorded.".to_string(),
        locator: Some(format!("mac-path:{}", trashed_path.display())),
    })
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    all(unix, not(target_os = "ios"), not(target_os = "android"))
)))]
pub(super) fn trash_with_receipt(path: &Path) -> Result<RollbackReceipt> {
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
pub(super) fn restore_from_system_trash(
    path: &Path,
    receipt: &RollbackReceipt,
    deleted_at: i64,
) -> Result<()> {
    ensure_restore_target_absent(path)?;
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
pub(super) fn restore_from_system_trash(
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
    ensure_restore_target_absent(path)?;
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
    restore_without_replacing(&trashed_path, path).with_context(|| {
        format!(
            "failed to restore {} from {}",
            path.display(),
            trashed_path.display()
        )
    })
}

#[cfg(target_os = "macos")]
fn restore_without_replacing(source: &Path, target: &Path) -> Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE)?;
    Ok(())
}

fn ensure_restore_target_absent(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => anyhow::bail!("restore target already exists: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect restore target {}", path.display())),
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    all(unix, not(target_os = "ios"), not(target_os = "android"))
)))]
pub(super) fn restore_from_system_trash(
    path: &Path,
    _receipt: &RollbackReceipt,
    _deleted_at: i64,
) -> Result<()> {
    ensure_restore_target_absent(path)?;
    anyhow::bail!(
        "programmatic restore is unsupported on this platform for {}",
        path.display()
    )
}

pub(super) fn absolute_path(path: &Path) -> Result<PathBuf> {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn restore_target_check_rejects_dangling_symlinks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        std::os::unix::fs::symlink(temp.path().join("missing"), &target).expect("dangling symlink");
        assert!(ensure_restore_target_absent(&target).is_err());
        assert!(
            target
                .symlink_metadata()
                .expect("symlink preserved")
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restore_rename_never_overwrites_a_target_created_after_the_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        for directory in [false, true] {
            let source = temp.path().join(if directory {
                "source-dir"
            } else {
                "source-file"
            });
            let target = temp.path().join(if directory {
                "target-dir"
            } else {
                "target-file"
            });
            if directory {
                std::fs::create_dir(&source).expect("source directory");
            } else {
                std::fs::write(&source, b"restorable").expect("source file");
            }
            ensure_restore_target_absent(&target).expect("target initially absent");
            // A competing creator wins after the precheck but before the rename.
            if directory {
                std::fs::create_dir(&target).expect("competing empty directory");
            } else {
                std::fs::write(&target, b"new user data").expect("competing file");
            }
            assert!(restore_without_replacing(&source, &target).is_err());
            assert!(source.exists());
            if !directory {
                assert_eq!(
                    std::fs::read(&target).expect("preserved target"),
                    b"new user data"
                );
                assert_eq!(
                    std::fs::read(&source).expect("preserved source"),
                    b"restorable"
                );
            }
        }
    }
}
