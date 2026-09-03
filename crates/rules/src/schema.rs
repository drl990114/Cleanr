use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use anyhow::{Context, Result, bail};
use cleanr_core::{
    Confidence, EntryKind, RuleMatchRole, RulePlatform, RuleSource, ScanLocationMode,
    ScanLocationPack,
};
use schemars::{JsonSchema, schema_for};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::matcher::{compile_path_glob, project_artifact_components, validate_child_name_glob};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RulePack {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub categories: Vec<String>,
    #[serde(default)]
    pub sources: Vec<RuleSource>,
    pub rules: Vec<RuleDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleDefinition {
    pub id: String,
    pub label: String,
    pub category: String,
    #[serde(rename = "match")]
    pub matcher: RuleMatcher,
    pub confidence: Confidence,
    pub default_selected: bool,
    #[serde(default)]
    pub match_role: RuleMatchRole,
    pub action: RuleAction,
    pub reason: String,
    pub risk_note: String,
    #[serde(default)]
    pub platforms: Vec<RulePlatform>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_guard: Option<RuntimeGuardDefinition>,
}

/// A fail-closed runtime condition for data owned by a running application or tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeGuardDefinition {
    pub process_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleMatcher {
    pub kind: Option<EntryKind>,
    pub dir_name: Option<String>,
    pub path_glob: Option<String>,
    pub file_name: Option<String>,
    pub extension: Option<String>,
    pub project: Option<ProjectMatcher>,
    pub max_age_days: Option<i64>,
    pub min_size: Option<u64>,
    #[serde(default)]
    pub cache_tagged: bool,
}

/// Match generated directories relative to a project root identified by direct children.
///
/// Name fields use glob syntax and are evaluated against the entries captured by the same scan.
/// Each non-empty positive group uses any-of semantics; excluded groups must have no matches.
/// Exclusions only veto children present in that snapshot and are not a standalone safety boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectMatcher {
    pub marker_globs: Vec<String>,
    pub root_dir_globs: Vec<String>,
    pub excluded_marker_globs: Vec<String>,
    pub excluded_root_dir_globs: Vec<String>,
    pub artifact_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleAction {
    Trash,
}

impl RulePack {
    pub fn from_toml(raw: &str) -> Result<Self> {
        let pack: Self = toml::from_str(raw).context("failed to parse rule pack TOML")?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("rule pack id cannot be empty");
        }
        if self.version.trim().is_empty() {
            bail!("rule pack {} has an empty version", self.id);
        }
        Version::parse(&self.version)
            .with_context(|| format!("rule pack {} has an invalid semantic version", self.id))?;
        if self.rules.is_empty() {
            bail!("rule pack {} contains no rules", self.id);
        }
        let mut rule_ids = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        for source in &self.sources {
            if source.id.trim().is_empty()
                || source.repository.trim().is_empty()
                || source.revision.trim().is_empty()
                || source.license.trim().is_empty()
            {
                bail!("rule pack {} has incomplete source provenance", self.id);
            }
            if !source_ids.insert(source.id.as_str()) {
                bail!(
                    "rule pack {} contains duplicate source id {}",
                    self.id,
                    source.id
                );
            }
        }
        for rule in &self.rules {
            if rule.id.trim().is_empty() {
                bail!("rule pack {} contains a rule with an empty id", self.id);
            }
            if rule.default_selected && rule.confidence != Confidence::High {
                bail!(
                    "rule {}:{} cannot be default_selected unless confidence is high",
                    self.id,
                    rule.id
                );
            }
            if rule.match_role == RuleMatchRole::Fallback && rule.default_selected {
                bail!(
                    "rule {}:{} cannot be both fallback and default_selected",
                    self.id,
                    rule.id
                );
            }
            if !rule_ids.insert(rule.id.as_str()) {
                bail!(
                    "rule pack {} contains duplicate rule id {}",
                    self.id,
                    rule.id
                );
            }
            if rule
                .platforms
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != rule.platforms.len()
            {
                bail!("rule {}:{} contains duplicate platforms", self.id, rule.id);
            }
            let mut referenced_sources = BTreeSet::new();
            for source_ref in &rule.source_refs {
                if !referenced_sources.insert(source_ref.as_str()) {
                    bail!(
                        "rule {}:{} contains duplicate source reference {}",
                        self.id,
                        rule.id,
                        source_ref
                    );
                }
                if !source_ids.contains(source_ref.as_str()) {
                    bail!(
                        "rule {}:{} references unknown source {}",
                        self.id,
                        rule.id,
                        source_ref
                    );
                }
            }
            if !self
                .categories
                .iter()
                .any(|category| category == &rule.category)
            {
                bail!(
                    "rule {}:{} uses undeclared category {}",
                    self.id,
                    rule.id,
                    rule.category
                );
            }
            if rule.matcher.max_age_days.is_some_and(|days| days < 0) {
                bail!("rule {}:{} has a negative max_age_days", self.id, rule.id);
            }
            if let Some(guard) = &rule.runtime_guard {
                guard.validate(&self.id, &rule.id)?;
            }
            let has_matcher = rule.matcher.dir_name.is_some()
                || rule.matcher.kind.is_some()
                || rule.matcher.path_glob.is_some()
                || rule.matcher.file_name.is_some()
                || rule.matcher.extension.is_some()
                || rule.matcher.project.is_some()
                || rule.matcher.max_age_days.is_some()
                || rule.matcher.min_size.is_some()
                || rule.matcher.cache_tagged;
            if !has_matcher {
                bail!("rule {}:{} has no matcher", self.id, rule.id);
            }
            if let Some(path_glob) = &rule.matcher.path_glob {
                compile_path_glob(path_glob).with_context(|| {
                    format!("rule {}:{} has an invalid path_glob", self.id, rule.id)
                })?;
            }
            if let Some(project) = &rule.matcher.project {
                if rule.matcher.kind != Some(EntryKind::Directory) {
                    bail!(
                        "rule {}:{} project matcher requires kind = directory",
                        self.id,
                        rule.id
                    );
                }
                if rule.matcher.dir_name.is_some()
                    || rule.matcher.path_glob.is_some()
                    || rule.matcher.file_name.is_some()
                    || rule.matcher.extension.is_some()
                {
                    bail!(
                        "rule {}:{} project matcher cannot be combined with another path matcher",
                        self.id,
                        rule.id
                    );
                }
                project.validate(&self.id, &rule.id)?;
            }
            if rule.matcher.cache_tagged {
                if rule.matcher.kind != Some(EntryKind::Directory) {
                    bail!(
                        "rule {}:{} cache_tagged matcher requires kind = directory",
                        self.id,
                        rule.id
                    );
                }
                if rule.matcher.dir_name.is_some()
                    || rule.matcher.path_glob.is_some()
                    || rule.matcher.file_name.is_some()
                    || rule.matcher.extension.is_some()
                    || rule.matcher.project.is_some()
                {
                    bail!(
                        "rule {}:{} cache_tagged matcher cannot be combined with another path matcher",
                        self.id,
                        rule.id
                    );
                }
            }
        }
        Ok(())
    }
}

impl RuntimeGuardDefinition {
    fn validate(&self, pack_id: &str, rule_id: &str) -> Result<()> {
        if self.process_names.is_empty() {
            bail!("rule {pack_id}:{rule_id} runtime guard has no process_names");
        }
        if self.process_names.len() > 16 {
            bail!("rule {pack_id}:{rule_id} runtime guard has too many process_names");
        }
        let mut normalized = BTreeSet::new();
        for name in &self.process_names {
            let trimmed = name.trim();
            if trimmed.is_empty()
                || trimmed.len() > 128
                || trimmed
                    .chars()
                    .any(|character| character.is_control() || matches!(character, '/' | '\\'))
                || trimmed
                    .bytes()
                    .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
            {
                bail!("rule {pack_id}:{rule_id} has an invalid runtime process name");
            }
            if !normalized.insert(trimmed.to_lowercase()) {
                bail!("rule {pack_id}:{rule_id} has duplicate runtime process names");
            }
        }
        Ok(())
    }
}

impl ProjectMatcher {
    fn validate(&self, pack_id: &str, rule_id: &str) -> Result<()> {
        if self.marker_globs.is_empty() {
            bail!("rule {pack_id}:{rule_id} project matcher has no marker_globs");
        }
        if self.artifact_paths.is_empty() {
            bail!("rule {pack_id}:{rule_id} project matcher has no artifact_paths");
        }
        for (field, patterns) in [
            ("marker_globs", &self.marker_globs),
            ("root_dir_globs", &self.root_dir_globs),
            ("excluded_marker_globs", &self.excluded_marker_globs),
            ("excluded_root_dir_globs", &self.excluded_root_dir_globs),
        ] {
            for pattern in patterns {
                validate_child_name_glob(pattern).with_context(|| {
                    format!("rule {pack_id}:{rule_id} has an invalid project {field} pattern")
                })?;
            }
        }
        for artifact_path in &self.artifact_paths {
            project_artifact_components(artifact_path).with_context(|| {
                format!(
                    "rule {pack_id}:{rule_id} has an invalid project artifact path {artifact_path:?}"
                )
            })?;
        }
        Ok(())
    }
}

pub fn scan_location_pack_from_toml(raw: &str) -> Result<ScanLocationPack> {
    let pack: ScanLocationPack =
        toml::from_str(raw).context("failed to parse scan location pack TOML")?;
    validate_scan_location_pack(&pack)?;
    Ok(pack)
}

#[must_use]
pub fn scan_location_pack_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(ScanLocationPack)).expect("scan location schema")
}

fn validate_scan_location_pack(pack: &ScanLocationPack) -> Result<()> {
    if pack.id.trim().is_empty() {
        bail!("scan location pack id cannot be empty");
    }
    Version::parse(&pack.version)
        .with_context(|| format!("scan location pack {} has an invalid version", pack.id))?;
    if pack.locations.is_empty() {
        bail!("scan location pack {} contains no locations", pack.id);
    }
    let mut ids = BTreeSet::new();
    for location in &pack.locations {
        if location.id.trim().is_empty() || location.label.trim().is_empty() {
            bail!("scan location pack {} has an incomplete location", pack.id);
        }
        if !ids.insert(location.id.as_str()) {
            bail!(
                "scan location pack {} contains duplicate location id {}",
                pack.id,
                location.id
            );
        }
        if location.platforms.is_empty() {
            bail!("scan location {} declares no platforms", location.id);
        }
        if location
            .platforms
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != location.platforms.len()
        {
            bail!("scan location {} contains duplicate platforms", location.id);
        }
        let relative = Path::new(&location.relative_path);
        if location.mode == ScanLocationMode::Scan
            && location.relative_path.trim().is_empty()
            && location.expansion.is_none()
        {
            bail!(
                "scan location {} requires a non-empty relative_path unless it uses a bounded expansion",
                location.id
            );
        }
        if location.relative_path.contains(['\\', ':']) {
            bail!(
                "scan location {} relative_path must use portable forward-slash components",
                location.id
            );
        }
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!(
                "scan location {} relative_path must contain only normal relative components",
                location.id
            );
        }
        if location
            .relative_path
            .bytes()
            .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']' | b'{' | b'}'))
        {
            bail!(
                "scan location {} relative_path cannot contain globs",
                location.id
            );
        }
        if let Some(expansion) = &location.expansion {
            if location.mode != ScanLocationMode::Scan {
                bail!(
                    "scan location {} cannot expand an os-managed location",
                    location.id
                );
            }
            if expansion.child_globs.is_empty() || expansion.suffixes.is_empty() {
                bail!(
                    "scan location {} expansion requires child_globs and suffixes",
                    location.id
                );
            }
            if !(1..=256).contains(&expansion.max_matches) {
                bail!(
                    "scan location {} expansion max_matches must be in 1..=256",
                    location.id
                );
            }
            for child_glob in &expansion.child_globs {
                validate_child_name_glob(child_glob).with_context(|| {
                    format!(
                        "scan location {} has an invalid direct-child glob",
                        location.id
                    )
                })?;
            }
            for suffix in &expansion.suffixes {
                validate_portable_relative_path(suffix, &location.id, "expansion suffix")?;
                if Path::new(suffix).components().count() > 4 {
                    bail!(
                        "scan location {} expansion suffix exceeds four components",
                        location.id
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_portable_relative_path(value: &str, location_id: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() || value.contains(['\\', ':']) {
        bail!("scan location {location_id} {field} must be a non-empty portable relative path");
    }
    let relative = Path::new(value);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || value
            .bytes()
            .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']' | b'{' | b'}'))
    {
        bail!("scan location {location_id} {field} must contain only fixed relative components");
    }
    Ok(())
}

#[must_use]
pub fn rule_pack_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(RulePack)).expect("rule pack schema serializes")
}
