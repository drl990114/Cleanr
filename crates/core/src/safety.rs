use std::path::{Path, PathBuf};

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
