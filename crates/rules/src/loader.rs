use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result, bail};
use cleanr_config::Config;
use cleanr_plugin_api::{
    PluginCapability, PluginDiagnostic, PluginDiscovery, PluginManifest, PluginSource, TrustLevel,
    discover_bundles, sorted_dir_entries,
};

use crate::{
    registry::RuleRegistry,
    schema::{RulePack, scan_location_pack_from_toml},
};

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
}

fn is_toml_file(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|extension| extension.to_str()) == Some("toml")
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
