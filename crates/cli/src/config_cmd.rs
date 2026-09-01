#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use cleanr_config::{CleanupAction, Config, UiTheme};

fn resolve_config_path(path: Option<PathBuf>) -> Result<PathBuf> {
    path.or_else(cleanr_config::default_config_path)
        .context("platform config directory is unavailable; pass --config")
}

pub fn path(config_path: Option<PathBuf>) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    println!("{}", path.display());
    Ok(())
}

pub fn init(config_path: Option<PathBuf>, force: bool) -> Result<()> {
    let path = resolve_config_path(config_path)?;

    if path.exists() && !force {
        println!("Config already exists at {}", path.display());
        println!("Use --force to overwrite with defaults.");
        return Ok(());
    }

    let config = Config::default();
    config.save_to(&path)?;
    println!("Config written to {}", path.display());
    Ok(())
}

pub fn get(config_path: Option<PathBuf>, key: &str) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    let config = if path.exists() {
        Config::load_from(&path)?
    } else {
        Config::default()
    };

    println!("{}", get_value(&config, key)?);
    Ok(())
}

pub fn set(config_path: Option<PathBuf>, key: &str, value: &str) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    let mut config = if path.exists() {
        Config::load_from(&path)?
    } else {
        Config::default()
    };

    apply_value(&mut config, key, value)?;
    config.save_to(&path)?;
    println!("Set {key} = {value} in {}", path.display());
    Ok(())
}

fn normalize_opt_string(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn get_value(config: &Config, key: &str) -> Result<String> {
    let parts: Vec<&str> = key.split('.').collect();
    match parts.as_slice() {
        ["scan", "stay_on_filesystem"] => Ok(config.scan.stay_on_filesystem.to_string()),
        ["scan", "budgets", "max_entries"] => Ok(config.scan.budgets.max_entries.to_string()),
        ["scan", "budgets", "max_elapsed_seconds"] => {
            Ok(config.scan.budgets.max_elapsed_seconds.to_string())
        }
        ["scan", "budgets", "max_estimated_memory_mib"] => {
            Ok(config.scan.budgets.max_estimated_memory_mib.to_string())
        }
        ["scan", "budgets", "max_issue_details"] => {
            Ok(config.scan.budgets.max_issue_details.to_string())
        }
        ["cleanup", "default_action"] => Ok(config.cleanup.default_action.to_string()),
        ["cleanup", "require_confirm"] => Ok(config.cleanup.require_confirm.to_string()),
        ["recommendations", "preselect_after_days"] => {
            Ok(config.recommendations.preselect_after_days.to_string())
        }
        ["i18n", "locale"] => Ok(config.i18n.locale.clone().unwrap_or_default()),
        ["ui", "theme"] => Ok(config.ui.theme.to_string()),
        _ => bail!("unknown config key: {key}"),
    }
}

fn apply_value(config: &mut Config, key: &str, value: &str) -> Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    match parts.as_slice() {
        ["scan", "stay_on_filesystem"] => {
            config.scan.stay_on_filesystem = parse_bool(value)?;
        }
        ["scan", "budgets", "max_entries"] => {
            config.scan.budgets.max_entries = parse_budget(value, key)?;
        }
        ["scan", "budgets", "max_elapsed_seconds"] => {
            config.scan.budgets.max_elapsed_seconds = parse_budget(value, key)?;
        }
        ["scan", "budgets", "max_estimated_memory_mib"] => {
            config.scan.budgets.max_estimated_memory_mib = parse_budget(value, key)?;
        }
        ["scan", "budgets", "max_issue_details"] => {
            config.scan.budgets.max_issue_details = parse_budget(value, key)?;
        }
        ["cleanup", "default_action"] => {
            config.cleanup.default_action = value.parse::<CleanupAction>()?;
        }
        ["cleanup", "require_confirm"] => {
            config.cleanup.require_confirm = parse_bool(value)?;
        }
        ["recommendations", "preselect_after_days"] => {
            config.recommendations.preselect_after_days = value
                .parse()
                .with_context(|| "recommendations.preselect_after_days must be an integer")?;
        }
        ["i18n", "locale"] => config.i18n.locale = normalize_opt_string(value.to_string()),
        ["ui", "theme"] => config.ui.theme = value.parse::<UiTheme>()?,
        _ => bail!("unknown config key: {key}"),
    }
    Ok(())
}

fn parse_budget(value: &str, key: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("{key} must be a non-negative integer (0 means unlimited)"))
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("expected boolean, got: {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_parser_accepts_cli_friendly_spellings() {
        for value in ["true", "TRUE", "1", "yes", "on"] {
            assert!(parse_bool(value).expect(value));
        }
        for value in ["false", "FALSE", "0", "no", "off"] {
            assert!(!parse_bool(value).expect(value));
        }
        assert!(parse_bool("maybe").is_err());
    }

    #[test]
    fn supported_values_round_trip_through_get_and_apply() {
        let mut config = Config::default();
        for (key, value, expected) in [
            ("scan.stay_on_filesystem", "yes", "true"),
            ("cleanup.default_action", "trash", "trash"),
            ("cleanup.require_confirm", "off", "false"),
            ("recommendations.preselect_after_days", "120", "120"),
            ("i18n.locale", "zh-CN", "zh-CN"),
            ("ui.theme", "dark", "dark"),
            ("scan.budgets.max_entries", "1000", "1000"),
            ("scan.budgets.max_elapsed_seconds", "30", "30"),
            ("scan.budgets.max_estimated_memory_mib", "256", "256"),
            ("scan.budgets.max_issue_details", "100", "100"),
        ] {
            apply_value(&mut config, key, value).expect(key);
            assert_eq!(get_value(&config, key).expect(key), expected);
        }

        assert!(apply_value(&mut config, "missing.key", "value").is_err());
        assert!(get_value(&config, "missing.key").is_err());
        assert!(apply_value(&mut config, "scan.budgets.max_entries", "-1").is_err());
    }

    #[test]
    fn empty_optional_strings_clear_locale_values() {
        let mut config = Config::default();
        config.i18n.locale = Some("zh-CN".to_string());

        let key = "i18n.locale";
        apply_value(&mut config, key, "").expect(key);
        assert_eq!(get_value(&config, key).expect(key), "");
    }

    #[test]
    fn set_persists_configuration() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("nested").join("config.toml");

        set(Some(path.clone()), "ui.theme", "light").expect("set theme");

        let config = Config::load_from(path).expect("load config");
        assert_eq!(config.ui.theme, UiTheme::Light);
    }

    #[test]
    fn invalid_set_does_not_create_or_replace_configuration() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing.toml");
        assert!(set(Some(missing.clone()), "ui.theme", "neon").is_err());
        assert!(!missing.exists());

        assert!(
            set(
                Some(missing.clone()),
                "scan.budgets.max_entries",
                &u64::MAX.to_string(),
            )
            .is_err()
        );
        assert!(!missing.exists());

        let existing = temp.path().join("existing.toml");
        Config::default().save_to(&existing).expect("seed config");
        let before = std::fs::read_to_string(&existing).expect("before");
        assert!(set(Some(existing.clone()), "cleanup.default_action", "delete").is_err());
        assert_eq!(std::fs::read_to_string(&existing).expect("after"), before);

        assert!(
            set(
                Some(existing.clone()),
                "scan.budgets.max_elapsed_seconds",
                &u64::MAX.to_string(),
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(existing).expect("after overflow"),
            before
        );
    }
}
