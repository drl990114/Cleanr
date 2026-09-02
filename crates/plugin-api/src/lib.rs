#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use schemars::{JsonSchema, schema_for};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

pub const PLUGIN_API_VERSION: &str = "1";
pub const PLUGIN_INDEX_SCHEMA_VERSION: u32 = 1;
pub const INSTALLED_PLUGIN_METADATA_FILE: &str = ".cleanr-plugin.json";

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCapability {
    Rules,
    ScanLocations,
    Translations,
    DynamicCandidates,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    Builtin,
    Trusted,
    #[default]
    Untrusted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSource {
    Builtin,
    Bundle(PathBuf),
    LegacyFile(PathBuf),
}

impl PluginSource {
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Builtin => None,
            Self::Bundle(path) | Self::LegacyFile(path) => Some(path),
        }
    }

    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Bundle(_) => "bundle",
            Self::LegacyFile(_) => "legacy-file",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub api_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub cleanr_version: Option<String>,
    pub capabilities: BTreeSet<PluginCapability>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub hooks: PluginHooks,
}

impl PluginManifest {
    pub fn from_toml(raw: &str, host_version: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(raw).context("failed to parse plugin manifest TOML")?;
        manifest.validate(host_version)?;
        Ok(manifest)
    }

    pub fn validate(&self, host_version: &str) -> Result<()> {
        if self.api_version != PLUGIN_API_VERSION {
            bail!(
                "plugin {} requires API version {}; supported version is {}",
                self.id,
                self.api_version,
                PLUGIN_API_VERSION
            );
        }
        validate_plugin_id(&self.id)?;
        if self.name.trim().is_empty() {
            bail!("plugin {} has an empty name", self.id);
        }
        Version::parse(&self.version)
            .with_context(|| format!("plugin {} has an invalid semantic version", self.id))?;
        if self.capabilities.is_empty() {
            bail!("plugin {} declares no capabilities", self.id);
        }
        if self
            .capabilities
            .contains(&PluginCapability::DynamicCandidates)
            && self.hooks.dynamic_candidates.is_empty()
        {
            bail!(
                "plugin {} declares dynamic-candidates but has no dynamic candidate hooks",
                self.id
            );
        }
        if !self
            .capabilities
            .contains(&PluginCapability::DynamicCandidates)
            && !self.hooks.dynamic_candidates.is_empty()
        {
            bail!(
                "plugin {} declares dynamic candidate hooks without dynamic-candidates capability",
                self.id
            );
        }
        self.hooks.validate(&self.id)?;
        if let Some(requirement) = &self.cleanr_version {
            let requirement = VersionReq::parse(requirement).with_context(|| {
                format!(
                    "plugin {} has an invalid cleanr_version requirement",
                    self.id
                )
            })?;
            let host = Version::parse(host_version).context("host cleanr version is invalid")?;
            if !requirement.matches(&host) {
                bail!(
                    "plugin {} requires cleanr {}; current version is {}",
                    self.id,
                    requirement,
                    host
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginHooks {
    pub dynamic_candidates: Vec<PluginHookCommand>,
}

impl PluginHooks {
    fn validate(&self, plugin_id: &str) -> Result<()> {
        for hook in &self.dynamic_candidates {
            hook.validate(plugin_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginHookCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl PluginHookCommand {
    fn validate(&self, plugin_id: &str) -> Result<()> {
        if self.command.trim().is_empty() {
            bail!("plugin {plugin_id} has a hook with an empty command");
        }
        if self
            .timeout_ms
            .is_some_and(|timeout| timeout == 0 || timeout > 30_000)
        {
            bail!("plugin {plugin_id} hook timeout_ms must be between 1 and 30000");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBundle {
    pub manifest: PluginManifest,
    pub root: PathBuf,
    pub source: PluginSource,
    pub trust: TrustLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginIndex {
    pub schema_version: u32,
    pub plugins: Vec<PluginIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginIndexEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub cleanr_version: Option<String>,
    pub capabilities: BTreeSet<PluginCapability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginIndexSource>,
    pub files: Vec<PluginIndexFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginIndexSource {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginIndexFile {
    pub path: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledPlugin {
    pub id: String,
    pub version: String,
    pub index_url: String,
    pub files: Vec<PluginIndexFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
}

impl PluginDiagnostic {
    #[must_use]
    pub fn warning(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            code,
            message: message.into(),
            path,
        }
    }

    #[must_use]
    pub fn error(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            code,
            message: message.into(),
            path,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PluginDiscovery {
    pub bundles: Vec<PluginBundle>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

#[must_use]
pub fn discover_bundles(
    roots: &[PathBuf],
    trusted_plugin_ids: &[String],
    host_version: &str,
) -> PluginDiscovery {
    let trusted = trusted_plugin_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut discovery = PluginDiscovery::default();
    let mut manifest_paths = Vec::new();

    for root in roots {
        if root.join("plugin.toml").is_file() {
            manifest_paths.push(root.join("plugin.toml"));
        }
        let Ok(entries) = sorted_dir_entries(root) else {
            continue;
        };
        manifest_paths.extend(
            entries
                .into_iter()
                .filter(|path| path.is_dir())
                .map(|path| path.join("plugin.toml"))
                .filter(|path| path.is_file()),
        );
    }

    manifest_paths.sort();
    manifest_paths.dedup();
    let mut seen_ids = BTreeSet::new();

    for manifest_path in manifest_paths {
        let result = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))
            .and_then(|raw| PluginManifest::from_toml(&raw, host_version));
        let manifest = match result {
            Ok(manifest) => manifest,
            Err(error) => {
                discovery.diagnostics.push(PluginDiagnostic::error(
                    "plugin-manifest-invalid",
                    error.to_string(),
                    Some(manifest_path),
                ));
                continue;
            }
        };

        if !seen_ids.insert(manifest.id.clone()) {
            discovery.diagnostics.push(PluginDiagnostic::error(
                "plugin-id-duplicate",
                format!("duplicate plugin id {}", manifest.id),
                Some(manifest_path),
            ));
            continue;
        }

        let Some(root) = manifest_path.parent().map(Path::to_path_buf) else {
            continue;
        };
        let trust = if trusted.contains(manifest.id.as_str()) {
            TrustLevel::Trusted
        } else {
            TrustLevel::Untrusted
        };
        discovery.bundles.push(PluginBundle {
            manifest,
            source: PluginSource::Bundle(root.clone()),
            root,
            trust,
        });
    }

    discovery
}

pub fn sorted_dir_entries(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read plugin directory {}", dir.display()))?;
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("failed to enumerate plugin directory {}", dir.display()))?;
    paths.sort();
    Ok(paths)
}

#[must_use]
pub fn plugin_manifest_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(PluginManifest)).expect("plugin manifest schema serializes")
}

#[must_use]
pub fn plugin_index_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(PluginIndex)).expect("plugin index schema serializes")
}

pub fn validate_plugin_id(id: &str) -> Result<()> {
    let mut components = Path::new(id).components();
    let is_single_normal_component =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || !is_single_normal_component
    {
        bail!(
            "plugin id must be a single normal path component containing only ASCII letters, digits, '.', '-' or '_' and be at most 128 characters"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incompatible_plugin_api() {
        let raw = r#"
api_version = "2"
id = "example.rules"
name = "Example"
version = "1.0.0"
capabilities = ["rules"]
"#;
        assert!(PluginManifest::from_toml(raw, "0.1.0").is_err());
    }

    #[test]
    fn discovery_is_sorted_and_rejects_duplicate_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        for name in ["b", "a"] {
            let dir = temp.path().join(name);
            fs::create_dir(&dir).expect("mkdir");
            fs::write(
                dir.join("plugin.toml"),
                r#"
api_version = "1"
id = "example.rules"
name = "Example"
version = "1.0.0"
capabilities = ["rules"]
"#,
            )
            .expect("write manifest");
        }

        let discovery = discover_bundles(&[temp.path().to_path_buf()], &[], "0.1.0");
        assert_eq!(discovery.bundles.len(), 1);
        assert_eq!(discovery.diagnostics.len(), 1);
        assert!(discovery.bundles[0].root.ends_with("a"));
    }

    #[test]
    fn manifest_validation_covers_identity_version_and_capabilities() {
        let valid = PluginManifest {
            api_version: PLUGIN_API_VERSION.to_string(),
            id: "example.rules_1".to_string(),
            name: "Example".to_string(),
            version: "1.2.3".to_string(),
            description: String::new(),
            cleanr_version: Some(">=0.1, <0.2".to_string()),
            capabilities: BTreeSet::from([PluginCapability::Rules]),
            categories: Vec::new(),
            keywords: Vec::new(),
            homepage: None,
            repository: None,
            license: None,
            hooks: PluginHooks::default(),
        };
        valid.validate("0.1.5").expect("valid manifest");

        for invalid_id in ["", ".", "..", "contains space", "slash/name"] {
            let mut manifest = valid.clone();
            manifest.id = invalid_id.to_string();
            assert!(manifest.validate("0.1.5").is_err(), "{invalid_id}");
        }

        let mut manifest = valid.clone();
        manifest.version = "latest".to_string();
        assert!(manifest.validate("0.1.5").is_err());

        let mut manifest = valid.clone();
        manifest.capabilities.clear();
        assert!(manifest.validate("0.1.5").is_err());

        assert!(valid.validate("0.2.0").is_err());
        assert!(valid.validate("not-semver").is_err());
    }

    #[test]
    fn dynamic_candidate_hooks_require_explicit_capability() {
        let raw = r#"
api_version = "1"
id = "example.dynamic"
name = "Dynamic"
version = "1.0.0"
capabilities = ["dynamic-candidates"]

[[hooks.dynamic_candidates]]
command = "cleanr-example"
args = ["scan"]
timeout_ms = 1000
"#;
        PluginManifest::from_toml(raw, "0.1.0").expect("dynamic plugin");

        let missing_capability = raw.replace(
            r#"capabilities = ["dynamic-candidates"]"#,
            r#"capabilities = ["rules"]"#,
        );
        assert!(PluginManifest::from_toml(&missing_capability, "0.1.0").is_err());

        let invalid_timeout = raw.replace("timeout_ms = 1000", "timeout_ms = 30001");
        assert!(PluginManifest::from_toml(&invalid_timeout, "0.1.0").is_err());
    }

    #[test]
    fn discovery_marks_trusted_bundles_and_reports_invalid_manifests() {
        let temp = tempfile::tempdir().expect("tempdir");
        let trusted = temp.path().join("trusted");
        let invalid = temp.path().join("invalid");
        fs::create_dir(&trusted).expect("trusted dir");
        fs::create_dir(&invalid).expect("invalid dir");
        fs::write(
            trusted.join("plugin.toml"),
            r#"
api_version = "1"
id = "trusted.plugin"
name = "Trusted"
version = "1.0.0"
capabilities = ["translations"]
"#,
        )
        .expect("trusted manifest");
        fs::write(invalid.join("plugin.toml"), "not valid toml").expect("invalid manifest");

        let discovery = discover_bundles(
            &[temp.path().to_path_buf()],
            &["trusted.plugin".to_string()],
            "0.1.0",
        );

        assert_eq!(discovery.bundles.len(), 1);
        assert_eq!(discovery.bundles[0].trust, TrustLevel::Trusted);
        assert_eq!(discovery.bundles[0].source.path(), Some(trusted.as_path()));
        assert_eq!(discovery.bundles[0].source.label(), "bundle");
        assert!(discovery.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "plugin-manifest-invalid"
                && diagnostic.severity == DiagnosticSeverity::Error
        }));
    }

    #[test]
    fn root_directory_can_itself_be_a_plugin_bundle() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("plugin.toml"),
            r#"
api_version = "1"
id = "root.plugin"
name = "Root"
version = "1.0.0"
capabilities = ["rules"]
"#,
        )
        .expect("manifest");

        let discovery = discover_bundles(&[temp.path().to_path_buf()], &[], "0.1.0");

        assert_eq!(discovery.bundles.len(), 1);
        assert_eq!(discovery.bundles[0].root, temp.path());
    }
}
