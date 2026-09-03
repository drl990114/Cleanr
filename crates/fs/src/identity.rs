use std::{
    collections::HashMap,
    fs::Metadata,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct HardlinkTracker {
    owners: HashMap<FileIdentity, HardlinkOwner>,
}

impl HardlinkTracker {
    pub(super) fn account(
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

    pub(super) fn account_identity(
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
pub(super) enum HardlinkAccounting {
    Count(u64),
    Duplicate,
    Reassign {
        previous_entry_index: usize,
        size_bytes: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct FileIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
}

#[cfg(unix)]
pub(super) fn file_identity(_path: &Path, metadata: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    hardlink_identity(metadata.dev(), metadata.ino(), metadata.nlink())
}

#[cfg(windows)]
pub(super) fn file_identity(path: &Path, _metadata: &Metadata) -> Option<FileIdentity> {
    let information = windows_file_information(path)?;
    hardlink_identity(
        information.volume_serial_number(),
        information.file_index(),
        information.number_of_links(),
    )
}

#[cfg(not(any(unix, windows)))]
pub(super) fn file_identity(_path: &Path, _metadata: &Metadata) -> Option<FileIdentity> {
    None
}

pub(super) fn hardlink_identity(
    device: u64,
    inode: u64,
    number_of_links: u64,
) -> Option<FileIdentity> {
    (number_of_links > 1).then_some(FileIdentity { device, inode })
}

#[cfg(unix)]
pub(super) fn device_id_for_evidence(_path: &Path, metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(windows)]
pub(super) fn device_id_for_evidence(path: &Path, _metadata: &Metadata) -> Option<u64> {
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
pub(super) fn device_id_for_evidence(_path: &Path, _metadata: &Metadata) -> Option<u64> {
    None
}
