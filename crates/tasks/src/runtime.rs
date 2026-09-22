use std::{collections::BTreeSet, ffi::OsStr, path::Path};

use anyhow::{Result, bail};
use cleanr_core::{CleanupItem, RuntimeGuardEvidence, RuntimeGuardState, ScanEntry};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// A point-in-time, path-free view of running process names.
///
/// `None` means the operating system could not provide a trustworthy snapshot. Callers treat
/// that state as blocking instead of guessing that an application is idle.
#[derive(Debug, Clone)]
pub(crate) struct ProcessSnapshot {
    names: Option<BTreeSet<String>>,
}

impl ProcessSnapshot {
    pub(crate) fn capture() -> Self {
        if !sysinfo::IS_SUPPORTED_SYSTEM {
            return Self { names: None };
        }

        let mut system = System::new();
        let refreshed = system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
        );
        if refreshed == 0 || system.processes().is_empty() {
            return Self { names: None };
        }

        let mut names = BTreeSet::new();
        for process in system.processes().values() {
            insert_normalized_name(&mut names, process.name());
            if let Some(executable) = process.exe()
                && let Some(file_name) = executable.file_name()
            {
                insert_normalized_name(&mut names, file_name);
            }
        }
        (!names.is_empty())
            .then_some(names)
            .map_or(Self { names: None }, |names| Self { names: Some(names) })
    }

    #[cfg(test)]
    pub(crate) fn from_names(names: &[&str]) -> Self {
        let names = names
            .iter()
            .filter_map(|name| normalize_process_name(name))
            .collect::<BTreeSet<_>>();
        Self { names: Some(names) }
    }

    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        Self { names: None }
    }

    fn guard_state(&self, expected_names: &[String]) -> RuntimeGuardState {
        let Some(running_names) = &self.names else {
            return RuntimeGuardState::Unknown;
        };
        if expected_names
            .iter()
            .filter_map(|name| normalize_process_name(name))
            .any(|name| running_names.contains(&name))
        {
            RuntimeGuardState::Active
        } else {
            RuntimeGuardState::Idle
        }
    }
}

fn insert_normalized_name(names: &mut BTreeSet<String>, name: &OsStr) {
    if let Some(name) = normalize_process_name(&name.to_string_lossy()) {
        names.insert(name);
    }
}

fn normalize_process_name(name: &str) -> Option<String> {
    let normalized = name.trim().to_lowercase();
    let normalized = normalized.strip_suffix(".exe").unwrap_or(&normalized);
    if normalized == "pythonw" {
        return Some("python".into());
    }
    // Python executable names carry version suffixes (including free-threaded builds).
    // Match those without treating arbitrary application prefixes as Python processes.
    if let Some(version) = normalized.strip_prefix("python")
        && !version.is_empty()
        && version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 't' | 'w'))
    {
        return Some("python".into());
    }
    (!normalized.is_empty()).then(|| normalized.to_string())
}

pub(crate) fn resolve_runtime_guards(entries: &mut [ScanEntry], snapshot: &ProcessSnapshot) {
    for entry in entries {
        for guard in entry
            .rule_hits
            .iter_mut()
            .filter_map(|hit| hit.runtime_guard.as_mut())
        {
            guard.state =
                if guard.current_user_only && !cleanr_fs::is_current_user_owned(&entry.path) {
                    RuntimeGuardState::Unknown
                } else {
                    snapshot.guard_state(&guard.process_names)
                };
        }
    }
}

pub(crate) fn capture_and_resolve_runtime_guards(entries: &mut [ScanEntry]) {
    resolve_runtime_guards(entries, &ProcessSnapshot::capture());
}

pub(crate) fn validate_current_runtime_guards(item: &CleanupItem) -> Result<()> {
    let Some(evidence) = &item.evidence else {
        return Ok(());
    };
    if evidence.runtime_guards.is_empty() {
        return Ok(());
    }
    validate_runtime_guards(
        &evidence.runtime_guards,
        &ProcessSnapshot::capture(),
        &item.path,
    )
}

pub(crate) fn validate_plan_current_runtime_guards<'a>(
    items: impl IntoIterator<Item = &'a CleanupItem>,
) -> Result<()> {
    let guarded_items = items
        .into_iter()
        .filter(|item| {
            item.evidence
                .as_ref()
                .is_some_and(|evidence| !evidence.runtime_guards.is_empty())
        })
        .collect::<Vec<_>>();
    if guarded_items.is_empty() {
        return Ok(());
    }

    let snapshot = ProcessSnapshot::capture();
    for item in guarded_items {
        let guards = &item
            .evidence
            .as_ref()
            .expect("filtered evidence")
            .runtime_guards;
        validate_runtime_guards(guards, &snapshot, &item.path)?;
    }
    Ok(())
}

fn validate_runtime_guards(
    guards: &[RuntimeGuardEvidence],
    snapshot: &ProcessSnapshot,
    target: &Path,
) -> Result<()> {
    for guard in guards {
        if guard.current_user_only && !cleanr_fs::is_current_user_owned(target) {
            bail!(
                "refusing to clean {}: current-user ownership could not be verified",
                target.display()
            );
        }
        match snapshot.guard_state(&guard.process_names) {
            RuntimeGuardState::Idle => {}
            RuntimeGuardState::Active => {
                bail!(
                    "refusing to clean {}: an owning application or tool is running",
                    target.display()
                )
            }
            RuntimeGuardState::Unknown => {
                bail!(
                    "refusing to clean {}: owning process state could not be verified",
                    target.display()
                )
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanr_core::{
        Confidence, EntryKind, RuleHit, RuleKey, RuleMatchRole, RuleTrust, RuntimeGuardEvidence,
    };
    use std::path::PathBuf;

    fn guarded_entry() -> ScanEntry {
        ScanEntry {
            path: PathBuf::from("/cache"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "builtin-system".into(),
                rule_id: "browser-cache".into(),
                label: "Browser cache".into(),
                category: "browser-cache".into(),
                confidence: Confidence::High,
                reason: "rebuildable".into(),
                risk_note: "close the application".into(),
                default_selected: true,
                trust: RuleTrust::Builtin,
                match_role: RuleMatchRole::Primary,
                sources: Vec::new(),
                read_only_scope: None,
                runtime_guard: Some(RuntimeGuardEvidence {
                    current_user_only: false,
                    rule: RuleKey {
                        rule_pack_id: "builtin-system".into(),
                        rule_id: "browser-cache".into(),
                    },
                    process_names: vec!["Example Browser".into()],
                    state: RuntimeGuardState::Unknown,
                }),
            }],
        }
    }

    #[test]
    fn cache_expansion_python_versions_and_ownership_fail_closed() {
        for name in [
            "python",
            "python3",
            "Python3.13t.EXE",
            "python3.14",
            "pythonw.exe",
        ] {
            assert_eq!(
                ProcessSnapshot::from_names(&[name]).guard_state(&["python".into()]),
                RuntimeGuardState::Active
            );
        }
        assert_eq!(
            ProcessSnapshot::from_names(&["python-editor"]).guard_state(&["python".into()]),
            RuntimeGuardState::Idle
        );
        let temp = tempfile::tempdir().unwrap();
        let mut entry = guarded_entry();
        entry.path = temp.path().join("missing");
        entry.rule_hits[0]
            .runtime_guard
            .as_mut()
            .unwrap()
            .current_user_only = true;
        resolve_runtime_guards(
            std::slice::from_mut(&mut entry),
            &ProcessSnapshot::from_names(&[]),
        );
        let guard = entry.rule_hits[0].runtime_guard.as_ref().unwrap();
        assert_eq!(guard.state, RuntimeGuardState::Unknown);
        assert!(
            validate_runtime_guards(
                std::slice::from_ref(guard),
                &ProcessSnapshot::from_names(&[]),
                &entry.path
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            std::fs::create_dir(&entry.path).unwrap();
            resolve_runtime_guards(
                std::slice::from_mut(&mut entry),
                &ProcessSnapshot::from_names(&[]),
            );
            assert_eq!(
                entry.rule_hits[0].runtime_guard.as_ref().unwrap().state,
                RuntimeGuardState::Idle
            );
            let link = temp.path().join("link");
            std::os::unix::fs::symlink(&entry.path, &link).unwrap();
            let guard = entry.rule_hits[0].runtime_guard.as_ref().unwrap();
            assert!(
                validate_runtime_guards(
                    std::slice::from_ref(guard),
                    &ProcessSnapshot::from_names(&[]),
                    &link
                )
                .is_err()
            );
        }
    }

    #[test]
    fn process_names_are_case_insensitive_and_ignore_windows_suffix() {
        let snapshot = ProcessSnapshot::from_names(&["Example Browser.EXE"]);
        assert_eq!(
            snapshot.guard_state(&["example browser".into()]),
            RuntimeGuardState::Active
        );
    }

    #[test]
    fn unavailable_snapshots_fail_closed() {
        let mut entries = vec![guarded_entry()];
        resolve_runtime_guards(&mut entries, &ProcessSnapshot::unavailable());
        assert_eq!(
            entries[0].rule_hits[0]
                .runtime_guard
                .as_ref()
                .expect("runtime guard")
                .state,
            RuntimeGuardState::Unknown
        );
    }

    #[test]
    fn entry_guards_are_resolved_from_one_snapshot() {
        let mut entries = vec![guarded_entry()];
        resolve_runtime_guards(
            &mut entries,
            &ProcessSnapshot::from_names(&["example browser"]),
        );
        assert_eq!(
            entries[0].rule_hits[0]
                .runtime_guard
                .as_ref()
                .expect("runtime guard")
                .state,
            RuntimeGuardState::Active
        );
    }
}
