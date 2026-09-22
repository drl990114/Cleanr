use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{ReadOnlyScope, ScanEntry};

/// Shared by analysis and both plan builders. A trash rule can never shadow a retention rule.
#[derive(Default)]
pub(crate) struct RuleProtectionIndex {
    retained_ancestors: HashSet<PathBuf>,
    retained_subtrees: HashSet<PathBuf>,
}

impl RuleProtectionIndex {
    pub(crate) fn from_entries(entries: &[ScanEntry]) -> Self {
        let mut index = Self::default();
        for entry in entries {
            for scope in entry.rule_hits.iter().filter_map(|hit| hit.read_only_scope) {
                index.insert(&entry.path, scope);
            }
        }
        index
    }

    pub(crate) fn insert(&mut self, path: &Path, scope: ReadOnlyScope) {
        let path = normalize_path(path);
        self.retained_ancestors
            .extend(path.ancestors().map(Path::to_path_buf));
        if scope == ReadOnlyScope::Subtree {
            self.retained_subtrees.insert(path.to_path_buf());
        }
    }

    pub(crate) fn excludes(&self, path: &Path) -> bool {
        if self.retained_ancestors.is_empty() {
            return false;
        }
        self.excludes_normalized(path) || self.excludes_normalized(&normalize_path(path))
    }

    fn excludes_normalized(&self, path: &Path) -> bool {
        self.retained_ancestors.contains(path)
            || path
                .ancestors()
                .any(|ancestor| self.retained_subtrees.contains(ancestor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_keeps_ancestors_and_subtrees_but_allows_container_leaves() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let version = root.join("IDE2026");
        let history = version.join("LocalHistory");
        let index = version.join("index");
        std::fs::create_dir_all(&history).unwrap();
        std::fs::create_dir(&index).unwrap();
        let mut protection = RuleProtectionIndex::default();
        protection.insert(&version, ReadOnlyScope::Entry);
        assert!(protection.excludes(&version));
        assert!(protection.excludes(&root));
        assert!(!protection.excludes(&index));
        protection.insert(&history, ReadOnlyScope::Subtree);
        assert!(protection.excludes(&history.join("node_modules")));
        assert!(protection.excludes(&index.join("../LocalHistory")));
        assert!(!protection.excludes(&index));
        #[cfg(unix)]
        {
            let alias = root.join("alias");
            std::os::unix::fs::symlink(&version, &alias).unwrap();
            assert!(protection.excludes(&alias));
            assert!(protection.excludes(&alias.join("LocalHistory")));
            assert!(!protection.excludes(&alias.join("index")));
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafetyPolicy {
    protected_paths: Vec<PathBuf>,
    protected_subtrees: Vec<PathBuf>,
    requires_confirmation: bool,
}

impl SafetyPolicy {
    #[must_use]
    pub fn new(protected_paths: Vec<PathBuf>, requires_confirmation: bool) -> Self {
        Self {
            protected_paths: normalize_protected_paths(protected_paths),
            protected_subtrees: Vec::new(),
            requires_confirmation,
        }
    }

    #[must_use]
    pub fn with_protected_subtrees(mut self, protected_subtrees: Vec<PathBuf>) -> Self {
        self.protected_subtrees = normalize_protected_paths(protected_subtrees);
        self
    }

    #[must_use]
    pub fn protected_paths(&self) -> &[PathBuf] {
        &self.protected_paths
    }

    #[must_use]
    pub fn protected_subtrees(&self) -> &[PathBuf] {
        &self.protected_subtrees
    }

    #[must_use]
    pub(crate) fn requires_confirmation(&self) -> bool {
        self.requires_confirmation
    }

    #[must_use]
    pub fn allows_candidate(&self, path: &Path) -> bool {
        let normalized_path = normalize_path(path);
        !is_filesystem_root(&normalized_path)
            && !self
                .protected_paths
                .iter()
                .any(|protected| protected.starts_with(&normalized_path))
            && !self.protected_subtrees.iter().any(|protected| {
                protected.starts_with(&normalized_path) || normalized_path.starts_with(protected)
            })
    }
}

pub(crate) fn normalize_protected_paths(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    for path in &mut paths {
        *path = normalize_path(path);
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    absolute.canonicalize().unwrap_or(absolute)
}

fn is_filesystem_root(path: &std::path::Path) -> bool {
    path.is_absolute() && path.parent().is_none()
}
