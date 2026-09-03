use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use cleanr_core::{EntryKind, ScanEntry};
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet};

use crate::schema::{ProjectMatcher, RuleDefinition};

#[derive(Debug, Clone)]
pub(super) struct CompiledRule {
    pub(super) path_glob: Option<GlobMatcher>,
    pub(super) project: Option<CompiledProjectMatcher>,
}

#[derive(Debug, Clone)]
pub(super) struct CompiledProjectMatcher {
    marker_globs: Vec<GlobMatcher>,
    root_dir_globs: Vec<GlobMatcher>,
    excluded_marker_globs: Vec<GlobMatcher>,
    excluded_root_dir_globs: Vec<GlobMatcher>,
    pub(super) artifact_paths: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ScanContext {
    pub(super) children_by_dir: BTreeMap<PathBuf, DirectoryChildren>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DirectoryChildren {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

impl CompiledProjectMatcher {
    pub(super) fn compile(project: &ProjectMatcher) -> Result<Self> {
        Ok(Self {
            marker_globs: compile_name_globs(&project.marker_globs)?,
            root_dir_globs: compile_name_globs(&project.root_dir_globs)?,
            excluded_marker_globs: compile_name_globs(&project.excluded_marker_globs)?,
            excluded_root_dir_globs: compile_name_globs(&project.excluded_root_dir_globs)?,
            artifact_paths: project
                .artifact_paths
                .iter()
                .map(|path| project_artifact_components(path))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    fn matches(&self, entry: &ScanEntry, context: &ScanContext) -> bool {
        self.artifact_paths.iter().any(|artifact_path| {
            let Some(root) = project_root(&entry.path, artifact_path) else {
                return false;
            };
            let Some(children) = context.children_by_dir.get(root) else {
                return false;
            };
            matches_required_group(&self.marker_globs, &children.files)
                && matches_required_group(&self.root_dir_globs, &children.directories)
                && !matches_any(&self.excluded_marker_globs, &children.files)
                && !matches_any(&self.excluded_root_dir_globs, &children.directories)
        })
    }
}

impl ScanContext {
    pub(super) fn from_entries(
        entries: &[ScanEntry],
        project_roots: &HashSet<PathBuf>,
        project_marker_filter: &GlobSet,
        project_root_dir_filter: &GlobSet,
    ) -> Self {
        let mut context = Self::default();
        for entry in entries {
            let Some(parent) = entry.path.parent() else {
                continue;
            };
            if !project_roots.contains(parent) {
                continue;
            }
            let Some(name) = entry.path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let relevant = match entry.kind {
                EntryKind::File => project_marker_filter.is_match(name),
                EntryKind::Directory => project_root_dir_filter.is_match(name),
                EntryKind::Symlink | EntryKind::Other => false,
            };
            if !relevant {
                continue;
            }
            let children = context
                .children_by_dir
                .entry(parent.to_path_buf())
                .or_default();
            match entry.kind {
                EntryKind::File => {
                    children.files.insert(name.to_string());
                }
                EntryKind::Directory => {
                    children.directories.insert(name.to_string());
                }
                EntryKind::Symlink | EntryKind::Other => {}
            }
        }
        context
    }
}

pub(super) fn compile_path_glob(pattern: &str) -> Result<Glob> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .context("invalid segment-aware path glob")
}

pub(super) fn matches_rule(
    entry: &ScanEntry,
    rule: &RuleDefinition,
    compiled: &CompiledRule,
    file_name: Option<&str>,
    path_for_glob: Option<&Path>,
    context: Option<&ScanContext>,
    as_of: DateTime<Utc>,
) -> bool {
    let matcher = &rule.matcher;
    if let Some(kind) = matcher.kind
        && entry.kind != kind
    {
        return false;
    }
    if let Some(dir_name) = &matcher.dir_name
        && (entry.kind != EntryKind::Directory || file_name != Some(dir_name))
    {
        return false;
    }
    if let Some(expected_file_name) = &matcher.file_name
        && (entry.kind == EntryKind::Directory || file_name != Some(expected_file_name.as_str()))
    {
        return false;
    }
    if let Some(extension) = &matcher.extension
        && entry
            .path
            .extension()
            .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case(extension))
            != Some(true)
    {
        return false;
    }
    if let Some(min_size) = matcher.min_size
        && entry.size_bytes < min_size
    {
        return false;
    }
    if let Some(max_age_days) = matcher.max_age_days {
        let Some(modified_at) = entry.modified_at else {
            return false;
        };
        if modified_at > as_of - Duration::days(max_age_days) {
            return false;
        }
    }
    if let Some(matcher) = &compiled.path_glob {
        let Some(path) = path_for_glob else {
            return false;
        };
        if !matcher.is_match(path) {
            return false;
        }
    }
    if let Some(project) = &compiled.project {
        let Some(context) = context else {
            return false;
        };
        if !project.matches(entry, context) {
            return false;
        }
    }
    true
}

pub(super) fn validate_child_name_glob(pattern: &str) -> Result<()> {
    if pattern.trim().is_empty() {
        bail!("name glob cannot be empty");
    }
    if pattern.contains('/') || pattern.contains('\\') {
        bail!("name glob must match one direct child name");
    }
    Glob::new(pattern).context("invalid glob syntax")?;
    Ok(())
}

fn compile_name_globs(patterns: &[String]) -> Result<Vec<GlobMatcher>> {
    patterns
        .iter()
        .map(|pattern| {
            Glob::new(pattern)
                .map(|glob| glob.compile_matcher())
                .context("invalid project child-name glob")
        })
        .collect()
}

pub(super) fn project_artifact_components(path: &str) -> Result<Vec<String>> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        bail!("artifact path must be a non-empty relative path using '/' separators");
    }
    let components = path.split('/').map(str::to_string).collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| component.is_empty() || matches!(component.as_str(), "." | ".."))
    {
        bail!("artifact path contains an unsupported component");
    }
    Ok(components)
}

pub(super) fn project_root<'a>(path: &'a Path, artifact_path: &[String]) -> Option<&'a Path> {
    let mut current = path;
    for expected in artifact_path.iter().rev() {
        if current.file_name().and_then(|name| name.to_str()) != Some(expected.as_str()) {
            return None;
        }
        current = current.parent()?;
    }
    Some(current)
}

fn matches_required_group(matchers: &[GlobMatcher], names: &BTreeSet<String>) -> bool {
    matchers.is_empty() || matches_any(matchers, names)
}

fn matches_any(matchers: &[GlobMatcher], names: &BTreeSet<String>) -> bool {
    matchers
        .iter()
        .any(|matcher| names.iter().any(|name| matcher.is_match(name)))
}
