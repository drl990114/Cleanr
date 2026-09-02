#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use cleanr_config::Config;
use cleanr_core::{
    Confidence, EntryKind, RuleHit, RuleMatchRole, RulePlatform, RuleSource, RuleTrust,
    RulesetVersion, ScanEntry, ScanLocationDefinition, ScanLocationMode, ScanLocationPack,
};
use cleanr_plugin_api::{
    PluginCapability, PluginDiagnostic, PluginDiscovery, PluginManifest, PluginSource, TrustLevel,
    discover_bundles, sorted_dir_entries,
};
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use schemars::{JsonSchema, schema_for};
use semver::Version;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone)]
pub struct LoadedRulePack {
    pub definition: RulePack,
    pub source: PluginSource,
    pub trust: TrustLevel,
    pub plugin_id: Option<String>,
    compiled_rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
struct CompiledRule {
    path_glob: Option<GlobMatcher>,
    project: Option<CompiledProjectMatcher>,
}

#[derive(Debug, Clone)]
struct CompiledProjectMatcher {
    marker_globs: Vec<GlobMatcher>,
    root_dir_globs: Vec<GlobMatcher>,
    excluded_marker_globs: Vec<GlobMatcher>,
    excluded_root_dir_globs: Vec<GlobMatcher>,
    artifact_paths: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct ScanContext {
    children_by_dir: BTreeMap<PathBuf, DirectoryChildren>,
}

#[derive(Debug, Clone, Default)]
struct DirectoryChildren {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct RuleRegistry {
    packs: Vec<LoadedRulePack>,
    scan_locations: Vec<ScanLocationDefinition>,
    diagnostics: Vec<PluginDiagnostic>,
    dir_name_index: BTreeMap<String, Vec<(usize, usize)>>,
    file_name_index: BTreeMap<String, Vec<(usize, usize)>>,
    extension_index: BTreeMap<String, Vec<(usize, usize)>>,
    generic_rules: Vec<(usize, usize)>,
    project_marker_filter: GlobSet,
    project_root_dir_filter: GlobSet,
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
            let has_matcher = rule.matcher.dir_name.is_some()
                || rule.matcher.kind.is_some()
                || rule.matcher.path_glob.is_some()
                || rule.matcher.file_name.is_some()
                || rule.matcher.extension.is_some()
                || rule.matcher.project.is_some()
                || rule.matcher.max_age_days.is_some()
                || rule.matcher.min_size.is_some();
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
        if location.mode == ScanLocationMode::Scan && location.relative_path.trim().is_empty() {
            bail!(
                "scan location {} requires a non-empty relative_path",
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
    }
    Ok(())
}

impl CompiledProjectMatcher {
    fn compile(project: &ProjectMatcher) -> Result<Self> {
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
    fn from_entries(
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

impl RuleRegistry {
    pub fn builtin() -> Result<Self> {
        let mut registry = Self::empty();
        registry.add_builtin_plugin(
            BUILTIN_DEV_MANIFEST,
            &[BUILTIN_DEV_RULES],
            &[BUILTIN_DEV_LOCATIONS],
        )?;
        registry.add_builtin_plugin(BUILTIN_GENERAL_MANIFEST, &[BUILTIN_GENERAL_RULES], &[])?;
        registry.add_builtin_plugin(
            BUILTIN_SYSTEM_MANIFEST,
            &[BUILTIN_SYSTEM_RULES],
            &[BUILTIN_SYSTEM_LOCATIONS],
        )?;
        Ok(registry)
    }

    pub fn load(config: &Config) -> Result<Self> {
        let discovery = discover_bundles(
            &config.plugins.dirs,
            &config.plugins.trusted,
            env!("CARGO_PKG_VERSION"),
        );
        Self::load_with_discovery(config, &discovery)
    }

    pub fn load_with_discovery(config: &Config, discovery: &PluginDiscovery) -> Result<Self> {
        let mut registry = Self::builtin()?;
        registry
            .diagnostics
            .extend(discovery.diagnostics.iter().cloned());

        for bundle in &discovery.bundles {
            if bundle
                .manifest
                .capabilities
                .contains(&PluginCapability::DynamicCandidates)
            {
                registry.diagnostics.push(PluginDiagnostic::warning(
                    "dynamic-candidates-runtime-disabled",
                    format!(
                        "plugin {} declares dynamic-candidates, but hook execution is not enabled in this release",
                        bundle.manifest.id
                    ),
                    Some(bundle.root.clone()),
                ));
            }
            if !bundle
                .manifest
                .capabilities
                .contains(&PluginCapability::Rules)
            {
                // A bundle may contribute only trusted scan locations.
            } else {
                let rules_dir = bundle.root.join("rules");
                let paths = match sorted_dir_entries(&rules_dir) {
                    Ok(paths) => paths,
                    Err(error) => {
                        registry.diagnostics.push(PluginDiagnostic::warning(
                            "plugin-rules-directory-missing",
                            error.to_string(),
                            Some(rules_dir.clone()),
                        ));
                        Vec::new()
                    }
                };
                let paths = paths
                    .into_iter()
                    .filter(|path| is_toml_file(path))
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    registry.diagnostics.push(PluginDiagnostic::warning(
                        "plugin-rules-empty",
                        format!(
                            "plugin {} declares rules but contains no rule packs",
                            bundle.manifest.id
                        ),
                        Some(rules_dir),
                    ));
                }
                for path in paths {
                    registry.load_user_pack(
                        &path,
                        bundle.trust,
                        Some(bundle.manifest.id.clone()),
                        PluginSource::Bundle(bundle.root.clone()),
                    );
                }
            }

            if bundle
                .manifest
                .capabilities
                .contains(&PluginCapability::ScanLocations)
            {
                registry.load_scan_location_directory(bundle);
            }
        }

        for dir in &config.plugins.dirs {
            registry.load_legacy_dir(dir, &config.plugins.trusted);
        }

        let loaded_ids = registry
            .packs
            .iter()
            .map(|pack| pack.definition.id.clone())
            .collect::<BTreeSet<_>>();
        let enabled_rule_packs = config.cleanup.effective_enabled_rule_packs();
        for enabled in &enabled_rule_packs {
            if !loaded_ids.contains(enabled) {
                registry.diagnostics.push(PluginDiagnostic::warning(
                    "rule-pack-not-found",
                    format!("enabled rule pack {enabled} was not found"),
                    None,
                ));
            }
        }
        registry.packs.retain(|pack| {
            enabled_rule_packs
                .iter()
                .any(|enabled| enabled == &pack.definition.id)
        });
        registry.rebuild_indexes()?;
        Ok(registry)
    }

    pub fn load_dir(&mut self, dir: impl AsRef<Path>) -> Result<()> {
        self.load_legacy_dir(dir.as_ref(), &[]);
        Ok(())
    }

    #[must_use]
    pub fn packs(&self) -> &[LoadedRulePack] {
        &self.packs
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn scan_locations(&self) -> &[ScanLocationDefinition] {
        &self.scan_locations
    }

    #[must_use]
    pub fn versions(&self) -> Vec<RulesetVersion> {
        self.packs
            .iter()
            .map(|pack| RulesetVersion {
                id: pack.definition.id.clone(),
                version: pack.definition.version.clone(),
                sources: pack.definition.sources.clone(),
            })
            .collect()
    }

    pub fn annotate_entries(&self, entries: &mut [ScanEntry]) {
        self.annotate_entries_at(entries, Utc::now());
    }

    /// Annotate a scan with one fixed reference time for all age-based rules.
    pub fn annotate_entries_at(&self, entries: &mut [ScanEntry], as_of: DateTime<Utc>) {
        let project_roots = self.project_roots(entries);
        let context = ScanContext::from_entries(
            entries,
            &project_roots,
            &self.project_marker_filter,
            &self.project_root_dir_filter,
        );
        for entry in entries {
            entry.rule_hits = self.hits_for_at_with_context(
                entry,
                as_of,
                Some(&context),
                RulePlatform::current(),
            );
        }
    }

    /// Match rules that depend only on one entry.
    ///
    /// Use [`Self::annotate_entries`] for project-aware rules because their marker evidence comes
    /// from the complete scan snapshot.
    #[must_use]
    pub fn hits_for(&self, entry: &ScanEntry) -> Vec<RuleHit> {
        self.hits_for_at(entry, Utc::now())
    }

    /// Match entry-local rules using a caller-provided reference time.
    #[must_use]
    pub fn hits_for_at(&self, entry: &ScanEntry, as_of: DateTime<Utc>) -> Vec<RuleHit> {
        self.hits_for_at_with_context(entry, as_of, None, RulePlatform::current())
    }

    #[cfg(test)]
    fn hits_for_at_on_platform(
        &self,
        entry: &ScanEntry,
        as_of: DateTime<Utc>,
        platform: RulePlatform,
    ) -> Vec<RuleHit> {
        self.hits_for_at_with_context(entry, as_of, None, Some(platform))
    }

    fn hits_for_at_with_context(
        &self,
        entry: &ScanEntry,
        as_of: DateTime<Utc>,
        context: Option<&ScanContext>,
        platform: Option<RulePlatform>,
    ) -> Vec<RuleHit> {
        let mut candidates = Vec::with_capacity(self.generic_rules.len() + 4);
        candidates.extend(self.generic_rules.iter().copied());
        let file_name = entry.path.file_name().map(|name| name.to_string_lossy());
        if let Some(name) = file_name.as_deref() {
            if entry.kind == EntryKind::Directory {
                candidates.extend(self.dir_name_index.get(name).into_iter().flatten().copied());
            } else {
                candidates.extend(
                    self.file_name_index
                        .get(name)
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        if let Some(extension) = entry.path.extension().and_then(|value| value.to_str()) {
            if let Some(indexed) = self.extension_index.get(extension) {
                candidates.extend(indexed.iter().copied());
            } else {
                let extension = extension.to_ascii_lowercase();
                candidates.extend(
                    self.extension_index
                        .get(&extension)
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        if candidates.len() > 1 {
            candidates.sort_unstable();
            candidates.dedup();
        }
        let path_for_glob = candidates
            .iter()
            .any(|(pack_index, rule_index)| {
                self.packs
                    .get(*pack_index)
                    .and_then(|pack| pack.compiled_rules.get(*rule_index))
                    .is_some_and(|compiled| compiled.path_glob.is_some())
            })
            .then_some(entry.path.as_path());

        candidates
            .into_iter()
            .filter_map(|(pack_index, rule_index)| {
                let pack = self.packs.get(pack_index)?;
                let rule = pack.definition.rules.get(rule_index)?;
                let compiled = pack.compiled_rules.get(rule_index)?;
                if !rule.platforms.is_empty()
                    && !platform.is_some_and(|platform| rule.platforms.contains(&platform))
                {
                    return None;
                }
                matches_rule(
                    entry,
                    rule,
                    compiled,
                    file_name.as_deref(),
                    path_for_glob,
                    context,
                    as_of,
                )
                .then(|| RuleHit {
                    rule_pack_id: pack.definition.id.clone(),
                    rule_id: rule.id.clone(),
                    label: rule.label.clone(),
                    category: rule.category.clone(),
                    confidence: rule.confidence,
                    reason: rule.reason.clone(),
                    risk_note: rule.risk_note.clone(),
                    default_selected: rule.default_selected,
                    match_role: rule.match_role,
                    sources: rule
                        .source_refs
                        .iter()
                        .filter_map(|source_ref| {
                            pack.definition
                                .sources
                                .iter()
                                .find(|source| source.id == *source_ref)
                                .cloned()
                        })
                        .collect(),
                    trust: match pack.trust {
                        TrustLevel::Builtin => RuleTrust::Builtin,
                        TrustLevel::Trusted => RuleTrust::Trusted,
                        TrustLevel::Untrusted => RuleTrust::Untrusted,
                    },
                })
            })
            .collect()
    }

    fn project_roots(&self, entries: &[ScanEntry]) -> HashSet<PathBuf> {
        let mut roots = HashSet::new();
        for entry in entries {
            if entry.kind != EntryKind::Directory {
                continue;
            }
            let Some(name) = entry.path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            for (pack_index, rule_index) in
                self.dir_name_index.get(name).into_iter().flatten().copied()
            {
                let Some(project) = self
                    .packs
                    .get(pack_index)
                    .and_then(|pack| pack.compiled_rules.get(rule_index))
                    .and_then(|compiled| compiled.project.as_ref())
                else {
                    continue;
                };
                for artifact_path in &project.artifact_paths {
                    if let Some(root) = project_root(&entry.path, artifact_path) {
                        roots.insert(root.to_path_buf());
                    }
                }
            }
        }
        roots
    }

    fn empty() -> Self {
        Self {
            packs: Vec::new(),
            scan_locations: Vec::new(),
            diagnostics: Vec::new(),
            dir_name_index: BTreeMap::new(),
            file_name_index: BTreeMap::new(),
            extension_index: BTreeMap::new(),
            generic_rules: Vec::new(),
            project_marker_filter: GlobSet::empty(),
            project_root_dir_filter: GlobSet::empty(),
        }
    }

    fn add_pack(
        &mut self,
        pack: RulePack,
        source: PluginSource,
        trust: TrustLevel,
        plugin_id: Option<String>,
    ) -> Result<()> {
        if self
            .packs
            .iter()
            .any(|loaded| loaded.definition.id == pack.id)
        {
            bail!("duplicate rule pack id {}", pack.id);
        }
        let compiled_rules = pack
            .rules
            .iter()
            .map(|rule| -> Result<CompiledRule> {
                let path_glob = rule
                    .matcher
                    .path_glob
                    .as_deref()
                    .map(compile_path_glob)
                    .transpose()?
                    .map(|glob| glob.compile_matcher());
                let project = rule
                    .matcher
                    .project
                    .as_ref()
                    .map(CompiledProjectMatcher::compile)
                    .transpose()?;
                Ok(CompiledRule { path_glob, project })
            })
            .collect::<Result<Vec<_>>>()?;
        if trust == TrustLevel::Untrusted && pack.rules.iter().any(|rule| rule.default_selected) {
            self.diagnostics.push(PluginDiagnostic::warning(
                "untrusted-default-selection-disabled",
                format!(
                    "rule pack {} requested default selection, but it is not trusted",
                    pack.id
                ),
                source.path().map(Path::to_path_buf),
            ));
        }
        self.packs.push(LoadedRulePack {
            definition: pack,
            source,
            trust,
            plugin_id,
            compiled_rules,
        });
        if let Err(error) = self.rebuild_indexes() {
            self.packs.pop();
            self.rebuild_indexes()
                .context("failed to restore rule indexes after rejecting a rule pack")?;
            return Err(error);
        }
        Ok(())
    }

    fn add_builtin_plugin(
        &mut self,
        manifest_raw: &str,
        rules: &[&str],
        location_packs: &[&str],
    ) -> Result<()> {
        let manifest = PluginManifest::from_toml(manifest_raw, env!("CARGO_PKG_VERSION"))?;
        if !manifest.capabilities.contains(&PluginCapability::Rules) {
            bail!("built-in plugin {} does not provide rules", manifest.id);
        }
        for raw in rules {
            self.add_pack(
                RulePack::from_toml(raw)?,
                PluginSource::Builtin,
                TrustLevel::Builtin,
                Some(manifest.id.clone()),
            )?;
        }
        if !location_packs.is_empty()
            && !manifest
                .capabilities
                .contains(&PluginCapability::ScanLocations)
        {
            bail!(
                "built-in plugin {} has scan locations without the capability",
                manifest.id
            );
        }
        for raw in location_packs {
            for location in scan_location_pack_from_toml(raw)?.locations {
                if self
                    .scan_locations
                    .iter()
                    .any(|existing| existing.id == location.id)
                {
                    bail!("duplicate built-in scan location id {}", location.id);
                }
                self.scan_locations.push(location);
            }
        }
        self.scan_locations
            .sort_by(|left, right| left.id.cmp(&right.id));
        Ok(())
    }

    fn load_scan_location_directory(&mut self, bundle: &cleanr_plugin_api::PluginBundle) {
        let directory = bundle.root.join("locations");
        if bundle.trust == TrustLevel::Untrusted {
            self.diagnostics.push(PluginDiagnostic::warning(
                "untrusted-scan-locations-disabled",
                format!(
                    "plugin {} declares scan locations, but it is not trusted",
                    bundle.manifest.id
                ),
                Some(directory),
            ));
            return;
        }
        let paths = match sorted_dir_entries(&directory) {
            Ok(paths) => paths,
            Err(error) => {
                self.diagnostics.push(PluginDiagnostic::warning(
                    "plugin-scan-locations-directory-missing",
                    error.to_string(),
                    Some(directory),
                ));
                return;
            }
        };
        let mut loaded_any = false;
        for path in paths.into_iter().filter(|path| is_toml_file(path)) {
            loaded_any = true;
            let result = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))
                .and_then(|raw| scan_location_pack_from_toml(&raw));
            match result {
                Ok(pack) => {
                    for location in pack.locations {
                        if self
                            .scan_locations
                            .iter()
                            .any(|existing| existing.id == location.id)
                        {
                            self.diagnostics.push(PluginDiagnostic::warning(
                                "duplicate-scan-location-id",
                                format!("duplicate scan location id {}", location.id),
                                Some(path.clone()),
                            ));
                        } else {
                            self.scan_locations.push(location);
                        }
                    }
                }
                Err(error) => self.diagnostics.push(PluginDiagnostic::warning(
                    "invalid-scan-location-pack",
                    error.to_string(),
                    Some(path),
                )),
            }
        }
        if !loaded_any {
            self.diagnostics.push(PluginDiagnostic::warning(
                "plugin-scan-locations-empty",
                format!(
                    "plugin {} declares scan locations but contains no location packs",
                    bundle.manifest.id
                ),
                Some(directory),
            ));
        }
        self.scan_locations
            .sort_by(|left, right| left.id.cmp(&right.id));
    }

    fn load_user_pack(
        &mut self,
        path: &Path,
        trust: TrustLevel,
        plugin_id: Option<String>,
        source: PluginSource,
    ) {
        let result = fs::read_to_string(path)
            .with_context(|| format!("failed to read rule plugin {}", path.display()))
            .and_then(|raw| RulePack::from_toml(&raw))
            .and_then(|pack| self.add_pack(pack, source, trust, plugin_id));
        if let Err(error) = result {
            self.diagnostics.push(PluginDiagnostic::error(
                "rule-pack-invalid",
                error.to_string(),
                Some(path.to_path_buf()),
            ));
        }
    }

    fn load_legacy_dir(&mut self, dir: &Path, trusted_ids: &[String]) {
        let paths = match sorted_dir_entries(dir) {
            Ok(paths) => paths,
            Err(_) => return,
        };
        for path in paths.into_iter().filter(|path| is_toml_file(path)) {
            if path.file_name().and_then(|name| name.to_str()) == Some("plugin.toml") {
                continue;
            }
            let raw = match fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(error) => {
                    self.diagnostics.push(PluginDiagnostic::error(
                        "rule-pack-read-failed",
                        error.to_string(),
                        Some(path),
                    ));
                    continue;
                }
            };
            let pack = match RulePack::from_toml(&raw) {
                Ok(pack) => pack,
                Err(error) => {
                    self.diagnostics.push(PluginDiagnostic::error(
                        "rule-pack-invalid",
                        error.to_string(),
                        Some(path),
                    ));
                    continue;
                }
            };
            let trust = if trusted_ids.iter().any(|trusted| trusted == &pack.id) {
                TrustLevel::Trusted
            } else {
                TrustLevel::Untrusted
            };
            if let Err(error) =
                self.add_pack(pack, PluginSource::LegacyFile(path.clone()), trust, None)
            {
                self.diagnostics.push(PluginDiagnostic::error(
                    "rule-pack-invalid",
                    error.to_string(),
                    Some(path),
                ));
            }
        }
    }

    fn rebuild_indexes(&mut self) -> Result<()> {
        self.dir_name_index.clear();
        self.file_name_index.clear();
        self.extension_index.clear();
        self.generic_rules.clear();
        let mut project_marker_filter = GlobSetBuilder::new();
        let mut project_root_dir_filter = GlobSetBuilder::new();
        for (pack_index, pack) in self.packs.iter().enumerate() {
            for (rule_index, rule) in pack.definition.rules.iter().enumerate() {
                let key = (pack_index, rule_index);
                if let Some(name) = &rule.matcher.dir_name {
                    self.dir_name_index
                        .entry(name.clone())
                        .or_default()
                        .push(key);
                } else if let Some(project) = &rule.matcher.project {
                    for pattern in project
                        .marker_globs
                        .iter()
                        .chain(&project.excluded_marker_globs)
                    {
                        project_marker_filter.add(Glob::new(pattern).with_context(|| {
                            format!("failed to compile project marker glob {pattern:?}")
                        })?);
                    }
                    for pattern in project
                        .root_dir_globs
                        .iter()
                        .chain(&project.excluded_root_dir_globs)
                    {
                        project_root_dir_filter.add(Glob::new(pattern).with_context(|| {
                            format!("failed to compile project root-directory glob {pattern:?}")
                        })?);
                    }
                    for artifact_path in &project.artifact_paths {
                        if let Some(name) = artifact_path.rsplit('/').next() {
                            self.dir_name_index
                                .entry(name.to_string())
                                .or_default()
                                .push(key);
                        }
                    }
                } else if let Some(name) = &rule.matcher.file_name {
                    self.file_name_index
                        .entry(name.clone())
                        .or_default()
                        .push(key);
                } else if let Some(extension) = &rule.matcher.extension {
                    self.extension_index
                        .entry(extension.to_ascii_lowercase())
                        .or_default()
                        .push(key);
                } else {
                    self.generic_rules.push(key);
                }
            }
        }
        self.project_marker_filter = project_marker_filter
            .build()
            .context("failed to build project marker glob index")?;
        self.project_root_dir_filter = project_root_dir_filter
            .build()
            .context("failed to build project root-directory glob index")?;
        Ok(())
    }
}

fn compile_path_glob(pattern: &str) -> Result<Glob> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .context("invalid segment-aware path glob")
}

fn matches_rule(
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

fn validate_child_name_glob(pattern: &str) -> Result<()> {
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

fn project_artifact_components(path: &str) -> Result<Vec<String>> {
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

fn project_root<'a>(path: &'a Path, artifact_path: &[String]) -> Option<&'a Path> {
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

fn is_toml_file(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|extension| extension.to_str()) == Some("toml")
}

#[must_use]
pub fn rule_pack_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(RulePack)).expect("rule pack schema serializes")
}

const BUILTIN_DEV_MANIFEST: &str = include_str!("../builtin-plugins/builtin-dev/plugin.toml");
const BUILTIN_DEV_RULES: &str = include_str!("../builtin-plugins/builtin-dev/rules/dev.toml");
const BUILTIN_DEV_LOCATIONS: &str =
    include_str!("../builtin-plugins/builtin-dev/locations/global.toml");
const BUILTIN_GENERAL_MANIFEST: &str =
    include_str!("../builtin-plugins/builtin-general/plugin.toml");
const BUILTIN_GENERAL_RULES: &str =
    include_str!("../builtin-plugins/builtin-general/rules/general.toml");
const BUILTIN_SYSTEM_MANIFEST: &str = include_str!("../builtin-plugins/builtin-system/plugin.toml");
const BUILTIN_SYSTEM_RULES: &str =
    include_str!("../builtin-plugins/builtin-system/rules/system.toml");
const BUILTIN_SYSTEM_LOCATIONS: &str =
    include_str!("../builtin-plugins/builtin-system/locations/global.toml");

#[cfg(test)]
mod tests {
    use super::*;
    use cleanr_core::{
        RecommendationPolicy, RuleResolutionState, RuleTrust, ScanEntry, build_analysis_report,
        build_cleanup_plan,
    };
    use std::path::PathBuf;

    #[test]
    fn path_globs_treat_single_star_as_one_path_segment() {
        let direct_child = compile_path_glob("**/Library/Caches/*")
            .expect("valid path glob")
            .compile_matcher();
        assert!(direct_child.is_match("/Users/me/Library/Caches/Yarn"));
        assert!(!direct_child.is_match("/Users/me/Library/Caches/App/node_modules"));

        let recursive = compile_path_glob("**/Library/Caches/**")
            .expect("valid recursive path glob")
            .compile_matcher();
        assert!(recursive.is_match("/Users/me/Library/Caches/App/node_modules"));
    }

    #[test]
    fn builtin_rules_match_developer_caches() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let entry = ScanEntry {
            path: PathBuf::from("/repo/node_modules"),
            kind: EntryKind::Directory,
            size_bytes: 2 * 1024 * 1024,
            modified_at: None,
            rule_hits: vec![],
        };

        let hits = registry.hits_for(&entry);
        assert_eq!(hits[0].rule_id, "node-modules");
        assert!(hits[0].default_selected);
        assert_eq!(hits[0].confidence, Confidence::High);
    }

    #[test]
    fn specific_builtin_cache_rule_shadows_application_cache_fallback() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let as_of = Utc::now();
        let mut entry = ScanEntry {
            path: PathBuf::from("/Users/me/Library/Caches/Yarn"),
            kind: EntryKind::Directory,
            size_bytes: 20 * 1024 * 1024,
            modified_at: Some(as_of - Duration::days(100)),
            rule_hits: vec![],
        };
        entry.rule_hits = registry.hits_for_at(&entry, as_of);

        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/Users/me")],
            &[entry],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid analysis");
        let resolution = &report.candidates[0].rules;

        assert_eq!(resolution.state, RuleResolutionState::Single);
        assert_eq!(resolution.matched.len(), 2);
        assert_eq!(resolution.shadowed.len(), 1);
        assert_eq!(
            resolution
                .primary
                .as_ref()
                .expect("specific primary rule")
                .rule_id,
            "macos-tool-download-cache"
        );
    }

    #[test]
    fn duplicate_general_and_system_rules_have_equivalent_safety_semantics() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let as_of = Utc::now();
        let mut entry = ScanEntry {
            path: PathBuf::from("/Users/me/Downloads/archive.zip"),
            kind: EntryKind::File,
            size_bytes: 200 * 1024 * 1024,
            modified_at: Some(as_of - Duration::days(100)),
            rule_hits: vec![],
        };
        entry.rule_hits = registry.hits_for_at(&entry, as_of);

        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/Users/me")],
            &[entry],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid analysis");

        assert_eq!(
            report.candidates[0].rules.state,
            RuleResolutionState::Equivalent
        );
        assert!(report.candidates[0].rules.shadowed.is_empty());
    }

    #[test]
    fn builtin_dev_covers_project_artifacts_across_supported_stacks() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let cases = [
            ("/cargo/Cargo.toml", "/cargo/target", "rust-target"),
            ("/node/package.json", "/node/node_modules", "node-modules"),
            (
                "/react-native/package.json",
                "/react-native/android/build",
                "react-native-android-build-cache",
            ),
            (
                "/unity/Assembly-CSharp.csproj",
                "/unity/Library",
                "unity-generated-cache",
            ),
            (
                "/stack/stack.yaml",
                "/stack/.stack-work",
                "haskell-stack-work",
            ),
            (
                "/cabal/cabal.project",
                "/cabal/dist-newstyle",
                "haskell-cabal-dist",
            ),
            ("/sbt/build.sbt", "/sbt/project/target", "sbt-target"),
            ("/maven/pom.xml", "/maven/target", "maven-target"),
            (
                "/gradle/build.gradle.kts",
                "/gradle/build",
                "gradle-project-artifacts",
            ),
            (
                "/cmake/CMakeLists.txt",
                "/cmake/cmake-build-debug",
                "cmake-build-output",
            ),
            (
                "/unreal/game.uproject",
                "/unreal/DerivedDataCache",
                "unreal-generated-cache",
            ),
            (
                "/jupyter/notebook.ipynb",
                "/jupyter/.ipynb_checkpoints",
                "jupyter-checkpoints",
            ),
            ("/python/app.py", "/python/.nox", "python-nox-environments"),
            ("/pixi/pixi.toml", "/pixi/.pixi", "pixi-environment"),
            (
                "/composer/composer.json",
                "/composer/vendor",
                "composer-vendor",
            ),
            (
                "/flutter/pubspec.yaml",
                "/flutter/.dart_tool",
                "dart-tooling-cache",
            ),
            (
                "/elixir/mix.exs",
                "/elixir/.lexical",
                "elixir-language-server-cache",
            ),
            ("/swift/Package.swift", "/swift/.build", "swift-build-cache"),
            ("/zig/build.zig", "/zig/.zig-cache", "zig-cache"),
            (
                "/godot/project.godot",
                "/godot/.godot",
                "godot-import-cache",
            ),
            (
                "/dotnet/app.csproj",
                "/dotnet/obj",
                "dotnet-intermediate-output",
            ),
            ("/turbo/turbo.json", "/turbo/.turbo", "turborepo-cache"),
            (
                "/terraform/.terraform.lock.hcl",
                "/terraform/.terraform",
                "terraform-working-data",
            ),
            (
                "/cocoapods/Podfile",
                "/cocoapods/Pods",
                "cocoapods-dependencies",
            ),
        ];
        let mut entries = cases
            .iter()
            .flat_map(|(marker, artifact, _)| {
                [
                    test_entry(marker, EntryKind::File),
                    test_entry(artifact, EntryKind::Directory),
                ]
            })
            .collect::<Vec<_>>();
        entries.push(test_entry("/react-native/android", EntryKind::Directory));

        registry.annotate_entries(&mut entries);

        for (_, artifact, expected_rule) in cases {
            let entry = entries
                .iter()
                .find(|entry| entry.path.as_path() == Path::new(artifact))
                .expect("artifact entry");
            assert!(
                entry
                    .rule_hits
                    .iter()
                    .any(|hit| hit.rule_id == expected_rule),
                "{artifact} should match {expected_rule}"
            );
        }
    }

    #[test]
    fn builtin_dev_keeps_sensitive_or_expensive_artifacts_review_only() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let cases = [
            (
                "/unreal/game.uproject",
                "/unreal/Saved",
                "unreal-saved-data",
                Confidence::Low,
            ),
            (
                "/jupyter/notebook.ipynb",
                "/jupyter/.ipynb_checkpoints",
                "jupyter-checkpoints",
                Confidence::Low,
            ),
            (
                "/terraform/.terraform.lock.hcl",
                "/terraform/.terraform",
                "terraform-working-data",
                Confidence::Low,
            ),
            (
                "/composer/composer.json",
                "/composer/vendor",
                "composer-vendor",
                Confidence::Medium,
            ),
        ];
        let mut entries = cases
            .iter()
            .flat_map(|(marker, artifact, _, _)| {
                [
                    test_entry(marker, EntryKind::File),
                    test_entry(artifact, EntryKind::Directory),
                ]
            })
            .collect::<Vec<_>>();
        entries.push(test_entry("/python/app.py", EntryKind::File));
        entries.push(test_entry("/python/.venv", EntryKind::Directory));

        registry.annotate_entries(&mut entries);

        for (_, artifact, expected_rule, expected_confidence) in cases {
            let hit = entries
                .iter()
                .find(|entry| entry.path.as_path() == Path::new(artifact))
                .and_then(|entry| {
                    entry
                        .rule_hits
                        .iter()
                        .find(|hit| hit.rule_id == expected_rule)
                })
                .expect("review-only rule hit");
            assert_eq!(hit.confidence, expected_confidence);
            assert!(!hit.default_selected);
        }
        assert!(
            entries
                .iter()
                .find(|entry| entry.path.as_path() == Path::new("/python/.venv"))
                .expect("venv entry")
                .rule_hits
                .is_empty()
        );
    }

    #[test]
    fn builtin_dev_covers_macos_global_caches_with_conservative_defaults() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let cases = [
            (
                "/Users/me/Library/Caches/Homebrew",
                "macos-package-manager-cache",
                Confidence::High,
                true,
            ),
            (
                "/Users/me/Library/Developer/CoreSimulator/Caches",
                "xcode-coresimulator-cache",
                Confidence::High,
                true,
            ),
            (
                "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/18.5",
                "xcode-device-support",
                Confidence::Medium,
                false,
            ),
            (
                "/Users/me/Library/Developer/Xcode/Archives/2026-07-25/App.xcarchive",
                "xcode-archives",
                Confidence::Low,
                false,
            ),
        ];

        for (path, expected_rule, confidence, default_selected) in cases {
            let hit = registry
                .hits_for(&test_entry(path, EntryKind::Directory))
                .into_iter()
                .find(|hit| hit.rule_id == expected_rule)
                .expect("macOS developer cache rule");
            assert_eq!(hit.confidence, confidence, "{path}");
            assert_eq!(hit.default_selected, default_selected, "{path}");
        }
    }

    #[test]
    fn project_markers_disambiguate_target_directories() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let mut entries = vec![
            test_entry("/cargo/Cargo.toml", EntryKind::File),
            test_entry("/cargo/target", EntryKind::Directory),
            test_entry("/maven/pom.xml", EntryKind::File),
            test_entry("/maven/target", EntryKind::Directory),
            test_entry("/sbt/build.sbt", EntryKind::File),
            test_entry("/sbt/target", EntryKind::Directory),
            test_entry("/unrelated/target", EntryKind::Directory),
        ];

        registry.annotate_entries(&mut entries);

        for (path, expected_rule) in [
            ("/cargo/target", "rust-target"),
            ("/maven/target", "maven-target"),
            ("/sbt/target", "sbt-target"),
        ] {
            let hits = &entries
                .iter()
                .find(|entry| entry.path.as_path() == Path::new(path))
                .expect("target entry")
                .rule_hits;
            assert_eq!(hits.len(), 1, "{path} should have one unambiguous hit");
            assert_eq!(hits[0].rule_id, expected_rule);
        }
        assert!(entries[6].rule_hits.is_empty());
    }

    #[test]
    fn nested_project_rules_keep_equivalent_safety_semantics() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let mut entries = vec![
            test_entry("/react-native/package.json", EntryKind::File),
            test_entry("/react-native/android", EntryKind::Directory),
            test_entry("/react-native/ios", EntryKind::Directory),
            test_entry("/react-native/android/build.gradle", EntryKind::File),
            test_entry("/react-native/android/build", EntryKind::Directory),
            test_entry("/react-native/ios/Podfile", EntryKind::File),
            test_entry("/react-native/ios/Pods", EntryKind::Directory),
        ];

        registry.annotate_entries(&mut entries);

        for index in [4, 6] {
            let hits = &entries[index].rule_hits;
            assert_eq!(hits.len(), 2);
            assert!(hits.windows(2).all(|pair| {
                pair[0].category == pair[1].category
                    && pair[0].confidence == pair[1].confidence
                    && pair[0].default_selected == pair[1].default_selected
                    && pair[0].trust == pair[1].trust
                    && pair[0].reason == pair[1].reason
                    && pair[0].risk_note == pair[1].risk_note
            }));
        }
    }

    #[test]
    fn builtin_system_rules_match_with_safe_defaults() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let browser_cache = ScanEntry {
            path: PathBuf::from("/Users/me/Library/Caches/Google/Chrome/Default/Cache"),
            kind: EntryKind::Directory,
            size_bytes: 2 * 1024 * 1024,
            modified_at: None,
            rule_hits: vec![],
        };
        let download = ScanEntry {
            path: PathBuf::from("/Users/me/Downloads/installer.dmg"),
            kind: EntryKind::File,
            size_bytes: 200 * 1024 * 1024,
            modified_at: None,
            rule_hits: vec![],
        };
        let temporary = ScanEntry {
            path: PathBuf::from("/tmp/export.tmp"),
            kind: EntryKind::File,
            size_bytes: 20 * 1024 * 1024,
            modified_at: None,
            rule_hits: vec![],
        };

        let browser_hit = registry
            .hits_for(&browser_cache)
            .into_iter()
            .find(|hit| hit.rule_id == "chrome-cache-directory")
            .expect("browser hit");
        assert_eq!(browser_hit.confidence, Confidence::High);
        assert!(browser_hit.default_selected);

        let download_hit = registry
            .hits_for(&download)
            .into_iter()
            .find(|hit| hit.rule_id == "large-download-file")
            .expect("download hit");
        assert_eq!(download_hit.confidence, Confidence::Low);
        assert!(!download_hit.default_selected);

        let temporary_hit = registry
            .hits_for(&temporary)
            .into_iter()
            .find(|hit| hit.rule_id == "large-temporary-file")
            .expect("temporary hit");
        assert_eq!(temporary_hit.confidence, Confidence::Medium);
        assert!(!temporary_hit.default_selected);
    }

    #[test]
    fn builtin_system_covers_macos_routine_cleanup_without_preselecting_user_data() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let safe_cases = [
            (
                "/Users/me/Library/Caches/com.apple.QuickLook.thumbnailcache",
                "macos-quicklook-thumbnail-cache",
            ),
            (
                "/Users/me/Library/Application Support/Slack/Cache",
                "macos-electron-app-cache",
            ),
            (
                "/Users/me/Library/Containers/com.microsoft.teams2/Data/Library/Caches",
                "macos-teams-cache",
            ),
        ];
        for (path, expected_rule) in safe_cases {
            let hit = registry
                .hits_for(&test_entry(path, EntryKind::Directory))
                .into_iter()
                .find(|hit| hit.rule_id == expected_rule)
                .expect("safe macOS cleanup rule");
            assert_eq!(hit.confidence, Confidence::High, "{path}");
            assert!(hit.default_selected, "{path}");
        }

        let mut spotify = test_entry(
            "/Users/me/Library/Application Support/Spotify/PersistentCache",
            EntryKind::Directory,
        );
        spotify.size_bytes = 20 * 1024 * 1024;
        let spotify_hit = registry
            .hits_for(&spotify)
            .into_iter()
            .find(|hit| hit.rule_id == "macos-spotify-persistent-cache")
            .expect("Spotify review rule");
        assert_eq!(spotify_hit.confidence, Confidence::Medium);
        assert!(!spotify_hit.default_selected);

        for (path, expected_rule) in [
            (
                "/Users/me/Library/Logs/DiagnosticReports/App-2026.ips",
                "macos-diagnostic-reports",
            ),
            (
                "/Users/me/Downloads/old-installer.dmg",
                "downloaded-macos-disk-image",
            ),
        ] {
            let hit = registry
                .hits_for(&test_entry(path, EntryKind::File))
                .into_iter()
                .find(|hit| hit.rule_id == expected_rule)
                .expect("review-only macOS cleanup rule");
            assert_eq!(hit.confidence, Confidence::Low, "{path}");
            assert!(!hit.default_selected, "{path}");
        }
    }

    #[test]
    fn builtin_system_covers_only_narrow_stale_windows_system_files() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let as_of = Utc::now();
        let mut temporary = test_entry(
            "/Users/me/AppData/Local/Temp/cleanr-old.tmp",
            EntryKind::File,
        );
        temporary.size_bytes = 20 * 1024 * 1024;
        temporary.modified_at = Some(as_of - Duration::days(31));
        temporary.rule_hits =
            registry.hits_for_at_on_platform(&temporary, as_of, RulePlatform::Windows);

        let temporary_hit = temporary
            .rule_hits
            .iter()
            .find(|hit| hit.rule_id == "windows-stale-user-temporary-file")
            .expect("stale Windows user temp file");
        assert_eq!(temporary_hit.confidence, Confidence::High);
        assert!(temporary_hit.default_selected);

        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/Users/me/AppData/Local/Temp")],
            std::slice::from_ref(&temporary),
            &[],
            RecommendationPolicy::new(30).expect("valid policy"),
        )
        .expect("valid analysis");
        let resolution = &report.candidates[0].rules;
        assert_eq!(resolution.state, RuleResolutionState::Single);
        assert_eq!(
            resolution
                .primary
                .as_ref()
                .expect("specific Windows temp rule")
                .rule_id,
            "windows-stale-user-temporary-file"
        );
        assert_eq!(resolution.shadowed.len(), 2);

        let mut shader = test_entry(
            "/Users/me/AppData/Local/D3DSCache/8f4d.cache",
            EntryKind::File,
        );
        shader.modified_at = Some(as_of - Duration::days(31));
        let shader_hit = registry
            .hits_for_at_on_platform(&shader, as_of, RulePlatform::Windows)
            .into_iter()
            .find(|hit| hit.rule_id == "windows-stale-directx-shader-cache-file")
            .expect("stale DirectX shader cache file");
        assert_eq!(shader_hit.confidence, Confidence::High);
        assert!(shader_hit.default_selected);

        let mut fresh_temporary = test_entry(
            "/Users/me/AppData/Local/Temp/cleanr-fresh.dat",
            EntryKind::File,
        );
        fresh_temporary.modified_at = Some(as_of - Duration::days(29));
        assert!(
            registry
                .hits_for_at_on_platform(&fresh_temporary, as_of, RulePlatform::Windows)
                .into_iter()
                .all(|hit| hit.rule_id != "windows-stale-user-temporary-file")
        );

        let mut shader_directory =
            test_entry("/Users/me/AppData/Local/D3DSCache", EntryKind::Directory);
        shader_directory.modified_at = Some(as_of - Duration::days(31));
        assert!(
            registry
                .hits_for_at(&shader_directory, as_of)
                .into_iter()
                .all(|hit| hit.rule_id != "windows-stale-directx-shader-cache-file")
        );

        for excluded in [
            "/Users/me/AppData/Local/Microsoft/Windows/Explorer/thumbcache_256.db",
            "/Windows/SoftwareDistribution/Download/update.cab",
            "/Windows/Prefetch/APP.EXE-12345678.pf",
        ] {
            assert!(
                registry
                    .hits_for_at(&test_entry(excluded, EntryKind::File), as_of)
                    .is_empty(),
                "{excluded} must not be a Windows cleanup candidate"
            );
        }

        let crash_dump = registry
            .hits_for_at_on_platform(
                &test_entry(
                    "/Users/me/AppData/Local/CrashDumps/app.dmp",
                    EntryKind::File,
                ),
                as_of,
                RulePlatform::Windows,
            )
            .into_iter()
            .find(|hit| hit.rule_id == "windows-user-crash-dump")
            .expect("user crash dump review rule");
        assert_eq!(crash_dump.confidence, Confidence::Low);
        assert!(!crash_dump.default_selected);
    }

    #[test]
    fn matcher_kind_restricts_rule_matches() {
        let raw = r#"
        id = "kind-test"
        name = "Kind Test"
        version = "1.0.0"
        description = "Kind matcher"
        categories = ["cache"]

        [[rules]]
        id = "directory-cache"
        label = "Directory Cache"
        category = "cache"
        match = { kind = "directory", path_glob = "**/Cache" }
        confidence = "medium"
        default_selected = false
        action = "trash"
        reason = "cache"
        risk_note = "review"
        "#;
        let mut registry = RuleRegistry::empty();
        registry
            .add_pack(
                RulePack::from_toml(raw).expect("rule pack"),
                PluginSource::Builtin,
                TrustLevel::Builtin,
                None,
            )
            .expect("add pack");

        assert!(
            registry
                .hits_for(&ScanEntry {
                    path: PathBuf::from("/repo/Cache"),
                    kind: EntryKind::Directory,
                    size_bytes: 1,
                    modified_at: None,
                    rule_hits: vec![],
                })
                .iter()
                .any(|hit| hit.rule_id == "directory-cache")
        );
        assert!(
            registry
                .hits_for(&ScanEntry {
                    path: PathBuf::from("/repo/Cache"),
                    kind: EntryKind::File,
                    size_bytes: 1,
                    modified_at: None,
                    rule_hits: vec![],
                })
                .is_empty()
        );
    }

    #[test]
    fn project_matcher_uses_scan_snapshot_and_exact_artifact_paths() {
        let raw = r#"
        id = "project-test"
        name = "Project Test"
        version = "1.0.0"
        description = "Project matcher"
        categories = ["build-cache"]

        [[rules]]
        id = "gradle-build"
        label = "Gradle build"
        category = "build-cache"
        match = { kind = "directory", project = { marker_globs = ["build.gradle", "build.gradle.kts"], artifact_paths = ["build", "nested/output"] } }
        confidence = "high"
        default_selected = true
        action = "trash"
        reason = "generated"
        risk_note = "rebuild"
        "#;
        let mut registry = RuleRegistry::empty();
        registry
            .add_pack(
                RulePack::from_toml(raw).expect("rule pack"),
                PluginSource::Builtin,
                TrustLevel::Builtin,
                None,
            )
            .expect("add pack");
        let artifact = ScanEntry {
            path: PathBuf::from("/repo/build"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: None,
            rule_hits: vec![],
        };

        assert!(registry.hits_for(&artifact).is_empty());

        let mut entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/build.gradle.kts"),
                kind: EntryKind::File,
                size_bytes: 1,
                modified_at: None,
                rule_hits: vec![],
            },
            artifact,
            ScanEntry {
                path: PathBuf::from("/repo/nested/output"),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: None,
                rule_hits: vec![],
            },
            ScanEntry {
                path: PathBuf::from("/unrelated/build"),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: None,
                rule_hits: vec![],
            },
            ScanEntry {
                path: PathBuf::from("/repo/other/output"),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: None,
                rule_hits: vec![],
            },
        ];

        registry.annotate_entries(&mut entries);

        assert_eq!(entries[1].rule_hits[0].rule_id, "gradle-build");
        assert_eq!(entries[2].rule_hits[0].rule_id, "gradle-build");
        assert!(entries[3].rule_hits.is_empty());
        assert!(entries[4].rule_hits.is_empty());
    }

    #[test]
    fn project_matcher_supports_required_and_excluded_root_children() {
        let raw = r#"
        id = "project-conditions"
        name = "Project Conditions"
        version = "1.0.0"
        description = "Project matcher conditions"
        categories = ["build-cache"]

        [[rules]]
        id = "mobile-build"
        label = "Mobile build"
        category = "build-cache"
        match = { kind = "directory", project = { marker_globs = ["package.json"], root_dir_globs = ["ios", "android"], excluded_marker_globs = ["blocked.json"], excluded_root_dir_globs = ["vendor-project"], artifact_paths = ["android/build"] } }
        confidence = "high"
        default_selected = true
        action = "trash"
        reason = "generated"
        risk_note = "rebuild"
        "#;
        let mut registry = RuleRegistry::empty();
        registry
            .add_pack(
                RulePack::from_toml(raw).expect("rule pack"),
                PluginSource::Builtin,
                TrustLevel::Builtin,
                None,
            )
            .expect("add pack");
        let scan = |extra: Vec<ScanEntry>| {
            let mut entries = vec![
                test_entry("/repo/package.json", EntryKind::File),
                test_entry("/repo/android", EntryKind::Directory),
                test_entry("/repo/android/build", EntryKind::Directory),
            ];
            entries.extend(extra);
            registry.annotate_entries(&mut entries);
            entries[2].rule_hits.clone()
        };

        assert_eq!(scan(vec![])[0].rule_id, "mobile-build");
        assert!(scan(vec![test_entry("/repo/blocked.json", EntryKind::File)]).is_empty());
        assert!(
            scan(vec![test_entry(
                "/repo/vendor-project",
                EntryKind::Directory
            )])
            .is_empty()
        );

        let mut missing_required_dir = vec![
            test_entry("/repo/package.json", EntryKind::File),
            test_entry("/repo/android/build", EntryKind::Directory),
        ];
        registry.annotate_entries(&mut missing_required_dir);
        assert!(missing_required_dir[1].rule_hits.is_empty());
    }

    #[test]
    fn project_context_does_not_index_markers_without_artifact_candidates() {
        let registry = RuleRegistry::builtin().expect("builtin rules load");
        let entries = (0..128)
            .map(|index| test_entry(&format!("/repo/package-{index}/module.py"), EntryKind::File))
            .collect::<Vec<_>>();

        let roots = registry.project_roots(&entries);
        let context = ScanContext::from_entries(
            &entries,
            &roots,
            &registry.project_marker_filter,
            &registry.project_root_dir_filter,
        );

        assert!(roots.is_empty());
        assert!(context.children_by_dir.is_empty());
    }

    #[test]
    fn project_matcher_rejects_unsafe_or_ambiguous_declarations() {
        let rule_pack = |matcher: &str| {
            format!(
                r#"
                id = "invalid-project"
                name = "Invalid Project"
                version = "1.0.0"
                description = "Invalid project matcher"
                categories = ["build-cache"]

                [[rules]]
                id = "invalid"
                label = "Invalid"
                category = "build-cache"
                match = {matcher}
                confidence = "medium"
                default_selected = false
                action = "trash"
                reason = "generated"
                risk_note = "review"
                "#
            )
        };

        assert!(
            RulePack::from_toml(&rule_pack(
                r#"{ project = { marker_globs = ["Cargo.toml"], artifact_paths = ["target"] } }"#
            ))
            .is_err()
        );
        assert!(
            RulePack::from_toml(&rule_pack(
                r#"{ kind = "directory", project = { marker_globs = ["nested/Cargo.toml"], artifact_paths = ["target"] } }"#
            ))
            .is_err()
        );
        assert!(
            RulePack::from_toml(&rule_pack(
                r#"{ kind = "directory", project = { marker_globs = ["Cargo.toml"], artifact_paths = ["../target"] } }"#
            ))
            .is_err()
        );
        assert!(
            RulePack::from_toml(&rule_pack(
                r#"{ kind = "directory", dir_name = "target", project = { marker_globs = ["Cargo.toml"], artifact_paths = ["target"] } }"#
            ))
            .is_err()
        );
    }

    #[test]
    fn age_matchers_use_one_caller_provided_reference_time() {
        let raw = r#"
        id = "age-test"
        name = "Age Test"
        version = "1.0.0"
        description = "Age matcher"
        categories = ["cache"]

        [[rules]]
        id = "old-cache"
        label = "Old Cache"
        category = "cache"
        match = { dir_name = "cache", max_age_days = 90 }
        confidence = "high"
        default_selected = true
        action = "trash"
        reason = "old cache"
        risk_note = "rebuild"
        "#;
        let mut registry = RuleRegistry::empty();
        registry
            .add_pack(
                RulePack::from_toml(raw).expect("rule pack"),
                PluginSource::Builtin,
                TrustLevel::Builtin,
                None,
            )
            .expect("add pack");

        let as_of = Utc::now();
        let entry = ScanEntry {
            path: PathBuf::from("/repo/cache"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: Some(as_of - Duration::days(90)),
            rule_hits: vec![],
        };

        assert_eq!(registry.hits_for_at(&entry, as_of).len(), 1);
        assert!(
            registry
                .hits_for_at(&entry, as_of - Duration::days(1))
                .is_empty()
        );
        assert_eq!(
            registry
                .hits_for_at(&entry, as_of + Duration::days(1))
                .len(),
            1
        );
    }

    #[test]
    fn legacy_default_rule_pack_list_includes_builtin_system() {
        let mut config = Config::default();
        config.cleanup.enabled_rule_packs =
            vec!["builtin-dev".to_string(), "builtin-general".to_string()];

        let registry = RuleRegistry::load(&config).expect("load registry");

        assert!(
            registry
                .packs()
                .iter()
                .any(|pack| pack.definition.id == "builtin-system")
        );
    }

    #[test]
    fn plugin_rejects_default_selected_non_high_confidence() {
        let raw = r#"
        id = "bad"
        name = "Bad"
        version = "0.1.0"
        description = "Bad"
        categories = ["x"]

        [[rules]]
        id = "bad-rule"
        label = "Bad"
        category = "x"
        match = { dir_name = "x" }
        confidence = "low"
        default_selected = true
        action = "trash"
        reason = "x"
        risk_note = "x"
        "#;

        assert!(RulePack::from_toml(raw).is_err());
    }

    #[test]
    fn plugin_rejects_default_selected_fallback_rule() {
        let raw = r#"
        id = "bad"
        name = "Bad"
        version = "0.1.0"
        description = "Bad"
        categories = ["x"]

        [[rules]]
        id = "bad-rule"
        label = "Bad"
        category = "x"
        match = { dir_name = "x" }
        confidence = "high"
        default_selected = true
        match_role = "fallback"
        action = "trash"
        reason = "x"
        risk_note = "x"
        "#;

        assert!(RulePack::from_toml(raw).is_err());
    }

    #[test]
    fn duplicate_rule_packs_keep_the_first_sorted_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("b.toml"), test_rule_pack("duplicate", "B")).expect("write b");
        fs::write(temp.path().join("a.toml"), test_rule_pack("duplicate", "A")).expect("write a");
        let mut config = Config::default();
        config.plugins.dirs = vec![temp.path().to_path_buf()];
        config.cleanup.enabled_rule_packs = vec!["duplicate".to_string()];

        let registry = RuleRegistry::load(&config).expect("load registry");

        assert_eq!(registry.packs().len(), 1);
        assert_eq!(registry.packs()[0].definition.name, "A");
        assert!(
            registry
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "rule-pack-invalid")
        );
    }

    #[test]
    fn untrusted_rules_never_preselect_cleanup_items() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("custom.toml"),
            test_rule_pack("custom", "Custom"),
        )
        .expect("write custom rule");
        let mut config = Config::default();
        config.plugins.dirs = vec![temp.path().to_path_buf()];
        config.cleanup.enabled_rule_packs = vec!["custom".to_string()];
        let registry = RuleRegistry::load(&config).expect("load registry");
        let mut entry = ScanEntry {
            path: PathBuf::from("/repo/target"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: None,
            rule_hits: vec![],
        };
        entry.rule_hits = registry.hits_for(&entry);

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], registry.versions(), &[entry]);

        assert_eq!(plan.summary.candidate_count, 1);
        assert_eq!(plan.summary.selected_count, 0);
        assert_eq!(
            registry.hits_for(&ScanEntry {
                path: PathBuf::from("/repo/target"),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: None,
                rule_hits: vec![],
            })[0]
                .trust,
            RuleTrust::Untrusted
        );
    }

    #[test]
    fn dynamic_candidate_hooks_are_reported_as_runtime_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("plugin.toml"),
            r#"
api_version = "1"
id = "dynamic.example"
name = "Dynamic Example"
version = "1.0.0"
capabilities = ["dynamic-candidates"]

[[hooks.dynamic_candidates]]
command = "cleanr-dynamic-example"
"#,
        )
        .expect("write manifest");
        let mut config = Config::default();
        config.plugins.dirs = vec![temp.path().to_path_buf()];

        let registry = RuleRegistry::load(&config).expect("load registry");

        assert!(registry.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == "dynamic-candidates-runtime-disabled"
                && diagnostic.message.contains("dynamic.example")
        }));
    }

    #[test]
    fn platform_scoped_rules_retain_pinned_source_evidence() {
        let registry = RuleRegistry::builtin().expect("builtin registry");
        let entry = test_entry(
            "/Users/test/Library/Caches/com.apple.QuickLook.thumbnailcache",
            EntryKind::Directory,
        );
        let as_of = Utc::now();

        let macos = registry.hits_for_at_on_platform(&entry, as_of, RulePlatform::Macos);
        let windows = registry.hits_for_at_on_platform(&entry, as_of, RulePlatform::Windows);

        let quicklook = macos
            .iter()
            .find(|hit| hit.rule_id == "macos-quicklook-thumbnail-cache")
            .expect("macOS rule");
        assert_eq!(
            quicklook
                .sources
                .iter()
                .map(|source| source.id.as_str())
                .collect::<Vec<_>>(),
            vec!["dusty", "puremac"]
        );
        assert!(
            windows
                .iter()
                .all(|hit| hit.rule_id != "macos-quicklook-thumbnail-cache")
        );
        let system_version = registry
            .versions()
            .into_iter()
            .find(|version| version.id == "builtin-system")
            .expect("system ruleset provenance");
        assert!(
            system_version
                .sources
                .iter()
                .any(|source| source.id == "winapp2")
        );
    }

    #[test]
    fn scan_location_packs_reject_escaping_or_globbed_paths() {
        for relative_path in ["../secrets", "cache/*", "C:/Temp", r"C:\\Temp", ""] {
            let raw = format!(
                r#"
id = "unsafe"
version = "1.0.0"

[[locations]]
id = "unsafe"
label = "Unsafe"
kind = "app-caches"
platforms = ["linux"]
base = "home"
relative_path = "{relative_path}"
"#
            );
            assert!(scan_location_pack_from_toml(&raw).is_err());
        }
    }

    #[test]
    fn only_trusted_plugins_can_expand_global_scan_locations() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("locations")).expect("locations");
        fs::write(
            temp.path().join("plugin.toml"),
            r#"
api_version = "1"
id = "example.locations"
name = "Example locations"
version = "1.0.0"
capabilities = ["scan-locations"]
"#,
        )
        .expect("manifest");
        fs::write(
            temp.path().join("locations/global.toml"),
            r#"
id = "example-locations"
version = "1.0.0"

[[locations]]
id = "example-cache"
label = "Example cache"
kind = "app-caches"
platforms = ["macos", "windows", "linux"]
base = "cache"
relative_path = "example"
"#,
        )
        .expect("locations");

        let mut config = Config::default();
        config.plugins.dirs = vec![temp.path().to_path_buf()];
        let untrusted = RuleRegistry::load(&config).expect("untrusted registry");
        assert!(
            untrusted
                .scan_locations()
                .iter()
                .all(|item| item.id != "example-cache")
        );
        assert!(
            untrusted
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code == "untrusted-scan-locations-disabled" })
        );

        config.plugins.trusted = vec!["example.locations".to_string()];
        let trusted = RuleRegistry::load(&config).expect("trusted registry");
        assert!(
            trusted
                .scan_locations()
                .iter()
                .any(|item| item.id == "example-cache")
        );
    }

    fn test_entry(path: &str, kind: EntryKind) -> ScanEntry {
        ScanEntry {
            path: PathBuf::from(path),
            kind,
            size_bytes: 2 * 1024 * 1024,
            modified_at: None,
            rule_hits: vec![],
        }
    }

    fn test_rule_pack(id: &str, name: &str) -> String {
        format!(
            r#"
id = "{id}"
name = "{name}"
version = "1.0.0"
description = "Test"
categories = ["build-cache"]

[[rules]]
id = "target"
label = "Target"
category = "build-cache"
match = {{ dir_name = "target" }}
confidence = "high"
default_selected = true
action = "trash"
reason = "generated"
risk_note = "rebuild"
"#
        )
    }
}
