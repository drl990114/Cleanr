#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    fmt,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use cleanr_core::{
    GlobalScanKind, MAX_RECOMMENDATION_AGE_DAYS, ScanBudgetLimits, default_global_scan_kinds,
};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub scan: ScanConfig,
    pub cleanup: CleanupConfig,
    pub recommendations: RecommendationConfig,
    pub plugins: PluginConfig,
    pub i18n: I18nConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ScanConfig {
    pub stay_on_filesystem: bool,
    pub ignore_dirs: Vec<PathBuf>,
    pub ignore_patterns: Vec<String>,
    pub global_kinds: Vec<GlobalScanKind>,
    #[serde(default, skip_serializing_if = "ScanBudgetConfig::is_unlimited")]
    pub budgets: ScanBudgetConfig,
}

/// Optional soft limits for a scan. Zero keeps the historical unlimited behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ScanBudgetConfig {
    pub max_entries: u64,
    pub max_elapsed_seconds: u64,
    pub max_estimated_memory_mib: u64,
    pub max_issue_details: u64,
}

impl ScanBudgetConfig {
    #[must_use]
    pub const fn is_unlimited(&self) -> bool {
        self.max_entries == 0
            && self.max_elapsed_seconds == 0
            && self.max_estimated_memory_mib == 0
            && self.max_issue_details == 0
    }

    /// Normalize user-facing units to the stable base units consumed by scan backends.
    pub fn limits(&self) -> Result<ScanBudgetLimits> {
        const TOML_MAX_INTEGER: u64 = i64::MAX as u64;
        if self.max_entries > TOML_MAX_INTEGER {
            bail!("scan.budgets.max_entries exceeds the largest TOML integer");
        }
        if self.max_issue_details > TOML_MAX_INTEGER {
            bail!("scan.budgets.max_issue_details exceeds the largest TOML integer");
        }
        let elapsed_millis = self
            .max_elapsed_seconds
            .checked_mul(1000)
            .context("scan.budgets.max_elapsed_seconds is too large")?;
        let estimated_memory_bytes = self
            .max_estimated_memory_mib
            .checked_mul(1024 * 1024)
            .context("scan.budgets.max_estimated_memory_mib is too large")?;
        Ok(ScanBudgetLimits {
            entries: self.max_entries,
            elapsed_millis,
            estimated_memory_bytes,
            issue_details: self.max_issue_details,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CleanupConfig {
    pub default_action: CleanupAction,
    pub require_confirm: bool,
    pub enabled_rule_packs: Vec<String>,
}

/// Shared recommendation policy used by the TUI and non-interactive workflows.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RecommendationConfig {
    /// Number of inactive days required for automatic preselection. Zero disables this age gate.
    pub preselect_after_days: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginConfig {
    pub dirs: Vec<PathBuf>,
    pub trusted: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct I18nConfig {
    pub locale: Option<String>,
    pub dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub theme: UiTheme,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAction {
    #[default]
    Trash,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiTheme {
    #[default]
    Auto,
    Dark,
    Light,
}

macro_rules! impl_text_enum {
    ($type:ty, {$($text:literal => $variant:path),+ $(,)?}) => {
        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                let value = match self {
                    $($variant => $text),+
                };
                formatter.write_str(value)
            }
        }

        impl FromStr for $type {
            type Err = anyhow::Error;

            fn from_str(value: &str) -> Result<Self> {
                match value.to_ascii_lowercase().as_str() {
                    $($text => Ok($variant)),+,
                    _ => bail!("unsupported value: {value}"),
                }
            }
        }
    };
}

impl_text_enum!(CleanupAction, {"trash" => CleanupAction::Trash});
impl_text_enum!(UiTheme, {
    "auto" => UiTheme::Auto,
    "dark" => UiTheme::Dark,
    "light" => UiTheme::Light,
});

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: UiTheme::Auto,
        }
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            stay_on_filesystem: false,
            ignore_dirs: Vec::new(),
            ignore_patterns: vec!["**/.git".to_string(), "**/.git/**".to_string()],
            global_kinds: default_global_scan_kinds(),
            budgets: ScanBudgetConfig::default(),
        }
    }
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            default_action: CleanupAction::Trash,
            require_confirm: true,
            enabled_rule_packs: default_enabled_rule_packs(),
        }
    }
}

impl Default for RecommendationConfig {
    fn default() -> Self {
        Self {
            preselect_after_days: 90,
        }
    }
}

impl CleanupConfig {
    #[must_use]
    pub fn effective_enabled_rule_packs(&self) -> Vec<String> {
        let legacy_default = ["builtin-dev", "builtin-general"];
        if self
            .enabled_rule_packs
            .iter()
            .map(String::as_str)
            .eq(legacy_default)
        {
            return default_enabled_rule_packs();
        }
        self.enabled_rule_packs.clone()
    }
}

impl Default for PluginConfig {
    fn default() -> Self {
        let dirs = default_plugin_dir().into_iter().collect();
        Self {
            dirs,
            trusted: Vec::new(),
        }
    }
}

impl Default for I18nConfig {
    fn default() -> Self {
        let dirs = default_language_dir().into_iter().collect();
        Self { locale: None, dirs }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let Some(path) = default_config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from(path)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        atomic_write(path, raw.as_bytes())
            .with_context(|| format!("failed to write config at {}", path.display()))
    }

    #[must_use]
    pub fn default_file_content() -> &'static str {
        concat!(
            "# cleanr configuration\n",
            "\n",
            "[scan]\n",
            "stay_on_filesystem = false\n",
            "ignore_dirs = []\n",
            "ignore_patterns = [\"**/.git\", \"**/.git/**\"]\n",
            "global_kinds = [\"developer-caches\", \"browser-caches\", \"app-caches\", \"temp-files\", \"logs\", \"downloads\"]\n",
            "\n",
            "[cleanup]\n",
            "default_action = \"trash\"\n",
            "require_confirm = true\n",
            "enabled_rule_packs = [\"builtin-dev\", \"builtin-general\", \"builtin-system\"]\n",
            "\n",
            "[recommendations]\n",
            "# Set to 0 to disable the age gate for automatic preselection.\n",
            "preselect_after_days = 90\n",
            "\n",
            "[plugins]\n",
            "# dirs defaults to ~/.config/cleanr/plugins\n",
            "trusted = []\n",
            "\n",
            "[i18n]\n",
            "# locale defaults to LC_ALL, LC_MESSAGES, LANG, then en-US.\n",
            "# locale = \"zh-CN\"\n",
            "# dirs defaults to ~/.config/cleanr/languages\n",
            "\n",
            "[ui]\n",
            "# Theme: \"auto\" detects from terminal background, or \"dark\"/\"light\".\n",
            "theme = \"auto\"\n",
        )
    }

    pub fn validate(&self) -> Result<()> {
        if self.recommendations.preselect_after_days > MAX_RECOMMENDATION_AGE_DAYS {
            bail!(
                "recommendations.preselect_after_days must be in 0..={MAX_RECOMMENDATION_AGE_DAYS}"
            );
        }
        if self
            .plugins
            .trusted
            .iter()
            .any(|plugin| plugin.trim().is_empty())
        {
            bail!("plugins.trusted cannot contain empty plugin IDs");
        }
        if self
            .cleanup
            .enabled_rule_packs
            .iter()
            .any(|pack| pack.trim().is_empty())
        {
            bail!("cleanup.enabled_rule_packs cannot contain empty rule pack IDs");
        }
        reject_duplicates(
            &self.plugins.trusted,
            "plugins.trusted cannot contain duplicate plugin IDs",
        )?;
        reject_duplicates(
            &self.cleanup.enabled_rule_packs,
            "cleanup.enabled_rule_packs cannot contain duplicate rule pack IDs",
        )?;
        reject_duplicates(
            &self.scan.global_kinds,
            "scan.global_kinds cannot contain duplicate global scan kinds",
        )?;
        self.scan.budgets.limits()?;
        Ok(())
    }
}

fn default_enabled_rule_packs() -> Vec<String> {
    ["builtin-dev", "builtin-general", "builtin-system"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn reject_duplicates<T: Ord>(values: &[T], message: &str) -> Result<()> {
    let mut unique = BTreeSet::new();
    if values.iter().any(|value| !unique.insert(value)) {
        bail!("{message}");
    }
    Ok(())
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .with_context(|| format!("failed to create temporary file in {}", directory.display()))?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[must_use]
pub fn config_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(Config)).expect("config schema serializes")
}

#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cleanr").join("config.toml"))
}

#[must_use]
pub fn default_plugin_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cleanr").join("plugins"))
}

#[must_use]
pub fn default_language_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cleanr").join("languages"))
}

#[must_use]
pub fn default_state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".local").join("state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cleanr")
}

#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_config_fields() {
        assert!(
            toml::from_str::<Config>(
                r#"
                [cleanup]
                require_confirm = true
                typo_field = true
                "#
            )
            .is_err()
        );
    }

    #[test]
    fn saves_configuration_atomically_and_reloads_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "incomplete =").expect("seed invalid config");
        let mut config = Config::default();
        config.i18n.locale = Some("zh-CN".to_string());

        config.save_to(&path).expect("save config");

        assert_eq!(
            Config::load_from(&path)
                .expect("reload config")
                .i18n
                .locale
                .as_deref(),
            Some("zh-CN")
        );
        assert_eq!(
            std::fs::read_dir(temp.path())
                .expect("read temp directory")
                .count(),
            1
        );
    }

    #[test]
    fn rejects_duplicate_plugin_ids() {
        let mut config = Config::default();
        config.plugins.trusted = vec!["example".to_string(), "example".to_string()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_a_recommendation_age_above_the_supported_limit() {
        let mut config = Config::default();
        config.recommendations.preselect_after_days = MAX_RECOMMENDATION_AGE_DAYS + 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn allows_zero_to_disable_the_recommendation_age_gate() {
        let mut config = Config::default();
        config.recommendations.preselect_after_days = 0;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn scan_budgets_default_to_unlimited_for_existing_configs() {
        let config: Config = toml::from_str(
            r#"
            [scan]
            stay_on_filesystem = true
            "#,
        )
        .expect("legacy config loads");

        assert_eq!(config.scan.budgets, ScanBudgetConfig::default());
        assert_eq!(
            config.scan.budgets.limits().unwrap(),
            ScanBudgetLimits::default()
        );
        assert!(config.scan.budgets.limits().unwrap().is_unlimited());
    }

    #[test]
    fn unlimited_scan_budgets_are_omitted_when_serializing() {
        let raw = toml::to_string_pretty(&Config::default()).expect("serialize default config");

        assert!(!raw.contains("[scan.budgets]"));
        assert!(!raw.contains("max_entries"));
    }

    #[test]
    fn nonzero_scan_budgets_are_serialized_and_round_trip() {
        let mut config = Config::default();
        config.scan.budgets.max_elapsed_seconds = 180;
        config.scan.budgets.max_issue_details = 1024;

        let raw = toml::to_string_pretty(&config).expect("serialize configured budgets");
        let restored: Config = toml::from_str(&raw).expect("reload configured budgets");

        assert!(raw.contains("[scan.budgets]"));
        assert_eq!(restored.scan.budgets, config.scan.budgets);
    }

    #[test]
    fn scan_budget_memory_unit_overflow_is_rejected() {
        let mut config = Config::default();
        config.scan.budgets.max_estimated_memory_mib = u64::MAX;

        let error = config.validate().expect_err("overflow must be rejected");

        assert!(error.to_string().contains("max_estimated_memory_mib"));
    }

    #[test]
    fn scan_budget_time_unit_overflow_is_rejected() {
        let mut config = Config::default();
        config.scan.budgets.max_elapsed_seconds = u64::MAX;

        let error = config.validate().expect_err("overflow must be rejected");

        assert!(error.to_string().contains("max_elapsed_seconds"));
    }

    #[test]
    fn scan_budget_limits_convert_persistable_boundary_units_without_narrowing_counts() {
        const MIB: u64 = 1024 * 1024;
        let budgets = ScanBudgetConfig {
            max_entries: i64::MAX as u64,
            max_elapsed_seconds: u64::MAX / 1000,
            max_estimated_memory_mib: u64::MAX / MIB,
            max_issue_details: i64::MAX as u64,
        };

        let limits = budgets.limits().expect("boundary limits convert");

        assert_eq!(limits.entries, i64::MAX as u64);
        assert_eq!(limits.elapsed_millis, (u64::MAX / 1000) * 1000);
        assert_eq!(limits.estimated_memory_bytes, (u64::MAX / MIB) * MIB);
        assert_eq!(limits.issue_details, i64::MAX as u64);
    }

    #[test]
    fn scan_budget_counts_above_the_toml_integer_range_are_rejected() {
        let mut config = Config::default();
        config.scan.budgets.max_entries = u64::MAX;
        let error = config
            .validate()
            .expect_err("entry count must be persistable");
        assert!(error.to_string().contains("max_entries"));

        config.scan.budgets.max_entries = 0;
        config.scan.budgets.max_issue_details = u64::MAX;
        let error = config
            .validate()
            .expect_err("issue detail count must be persistable");
        assert!(error.to_string().contains("max_issue_details"));
    }

    #[test]
    fn documented_default_config_matches_runtime_defaults() {
        let documented: Config =
            toml::from_str(Config::default_file_content()).expect("default config parses");

        assert_eq!(documented, Config::default());
    }

    #[test]
    fn config_schema_exposes_scan_budget_defaults() {
        let schema = config_schema();
        let serialized = serde_json::to_string(&schema).expect("serialize schema");

        assert!(serialized.contains("budgets"));
        assert!(serialized.contains("max_estimated_memory_mib"));
        assert!(serialized.contains("max_issue_details"));
    }

    #[test]
    fn text_enums_are_case_insensitive_and_reject_unknown_values() {
        assert_eq!("DARK".parse::<UiTheme>().expect("theme"), UiTheme::Dark);
        assert!("delete".parse::<CleanupAction>().is_err());
        assert!("system".parse::<UiTheme>().is_err());
    }

    #[test]
    fn validation_rejects_empty_values_and_duplicate_rule_packs() {
        let mut config = Config::default();
        config.plugins.trusted = vec!["valid".to_string(), " ".to_string()];
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.cleanup.enabled_rule_packs = vec!["same".to_string(), "same".to_string()];
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.scan.global_kinds = vec![GlobalScanKind::Logs, GlobalScanKind::Logs];
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_configuration_does_not_replace_existing_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "original").expect("seed config");
        let mut config = Config::default();
        config.plugins.trusted = vec![" ".to_string()];

        assert!(config.save_to(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(path).expect("read original"),
            "original"
        );
    }
}
