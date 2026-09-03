#![forbid(unsafe_code)]

mod config_cmd;
mod language_cmd;
mod plugin_cmd;
mod update;
mod workflow;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use cleanr_core::{GlobalScanKind, RecommendationPolicy, ScanRequest};

#[derive(Debug, Parser)]
#[command(
    name = "cleanr",
    version,
    subcommand_precedence_over_arg = true,
    about = "TUI-first, safe local disk cleanup"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Scan roots to use inside the TUI. Defaults to the current directory.
    #[arg(value_parser)]
    paths: Vec<PathBuf>,

    /// Read configuration from this TOML file instead of the default location.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Override the observed modification-age threshold without changing configuration.
    #[arg(long, global = true, value_name = "DAYS")]
    inactive_days: Option<u16>,

    /// Reserved for debug logging output.
    #[arg(long, env = "CLEANR_LOG_FILE")]
    log_file: Option<PathBuf>,

    /// Skip the automatic startup update check.
    #[arg(long, global = true, env = "CLEANR_NO_UPDATE_CHECK")]
    no_update_check: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scan paths once and print a summary or JSON report.
    Scan {
        /// Scan roots. Defaults to the current directory.
        #[arg(value_parser)]
        paths: Vec<PathBuf>,

        /// Include known system cleanup locations.
        #[arg(long)]
        global: bool,

        /// Include one global system cleanup category. May be repeated.
        #[arg(long = "global-kind")]
        global_kinds: Vec<GlobalScanKind>,

        /// Print a JSON scan report.
        #[arg(long)]
        json: bool,
    },

    /// Analyze paths and print a read-only JSON evidence report for local tools and agents.
    Analyze {
        /// Scan roots. Defaults to the current directory.
        #[arg(value_parser)]
        paths: Vec<PathBuf>,

        /// Include known system cleanup locations.
        #[arg(long)]
        global: bool,

        /// Include one global system cleanup category. May be repeated.
        #[arg(long = "global-kind")]
        global_kinds: Vec<GlobalScanKind>,
    },

    /// Build a cleanup plan without executing it.
    Plan {
        /// Scan roots. Defaults to the current directory.
        #[arg(value_parser)]
        paths: Vec<PathBuf>,

        /// Include known system cleanup locations.
        #[arg(long)]
        global: bool,

        /// Include one global system cleanup category. May be repeated.
        #[arg(long = "global-kind")]
        global_kinds: Vec<GlobalScanKind>,

        /// Select one exact discovered candidate path. May be repeated.
        #[arg(long = "select", value_name = "PATH")]
        select_paths: Vec<PathBuf>,

        /// Deselect one exact discovered candidate path. May be repeated.
        #[arg(long = "deselect", value_name = "PATH")]
        deselect_paths: Vec<PathBuf>,

        /// Write the JSON plan to this path instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Show what would be cleaned without moving anything to trash.
    DryRun {
        /// Scan roots. Defaults to the current directory.
        #[arg(value_parser)]
        paths: Vec<PathBuf>,

        /// Include known system cleanup locations.
        #[arg(long)]
        global: bool,

        /// Include one global system cleanup category. May be repeated.
        #[arg(long = "global-kind")]
        global_kinds: Vec<GlobalScanKind>,

        /// Select one exact discovered candidate path. May be repeated.
        #[arg(long = "select", value_name = "PATH")]
        select_paths: Vec<PathBuf>,

        /// Deselect one exact discovered candidate path. May be repeated.
        #[arg(long = "deselect", value_name = "PATH")]
        deselect_paths: Vec<PathBuf>,

        /// Print the JSON cleanup plan.
        #[arg(long)]
        json: bool,

        /// Write the JSON plan to this path.
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Execute an unchanged cleanup plan through the system trash.
    Clean {
        /// Local cleanup plan previously written by `cleanr plan --output`.
        #[arg(long)]
        plan: PathBuf,

        /// SHA-256 printed when the reviewed plan was written.
        #[arg(long)]
        plan_sha256: String,

        /// Attest that the current user explicitly authorized this exact cleanup plan.
        #[arg(long)]
        authorized_by_user: bool,
    },

    /// List or restore cleanup manifests without opening the TUI.
    Restore {
        #[command(subcommand)]
        action: RestoreAction,
    },

    /// Initialize cleanr config and language selection.
    Init {
        /// Locale to select, for example en-US or zh-CN.
        #[arg(long)]
        locale: String,

        /// Download the language file from GitHub instead of seeding the built-in file.
        #[arg(long)]
        from_github: bool,

        /// GitHub repository in owner/name form.
        #[arg(long, default_value = "drl990114/cleanr")]
        language_repo: String,

        /// GitHub ref used when downloading from GitHub.
        #[arg(long, default_value = "main")]
        language_ref: String,

        /// Expected SHA-256 of the downloaded language file.
        #[arg(long, requires = "from_github")]
        language_sha256: Option<String>,

        /// Override the language output directory.
        #[arg(long)]
        language_dir: Option<PathBuf>,
    },

    /// Manage cleanr configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Create, install, update, publish, or inspect Cleanr plugins.
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
}

#[derive(Debug, Subcommand)]
enum RestoreAction {
    /// List cleanup runs.
    List,

    /// Restore one cleanup run. Requires --confirm.
    Run {
        /// Execution manifest run ID.
        run_id: String,

        /// Confirm that files should be moved back from the system trash.
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Print the configuration file path.
    Path,

    /// Write a default configuration file.
    Init {
        /// Overwrite an existing configuration file.
        #[arg(long)]
        force: bool,
    },

    /// Read a configuration value by dotted key.
    Get { key: String },

    /// Set a configuration value by dotted key.
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
enum PluginAction {
    /// Scaffold a local declarative plugin.
    Init {
        /// Directory to create or update.
        path: PathBuf,

        /// Stable plugin manifest ID.
        #[arg(long)]
        id: String,

        /// Human-readable plugin name.
        #[arg(long)]
        name: String,

        /// Overwrite template files if they already exist.
        #[arg(long)]
        force: bool,
    },

    /// Validate plugin bundles or individual declaration files.
    Validate {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },

    /// Print a JSON schema: manifest, rules, language, or config.
    Schema { kind: String },

    /// Generate or check a static plugin index.
    Index {
        /// Directory containing plugin bundles.
        #[arg(long, default_value = "plugins")]
        plugin_dir: PathBuf,

        /// Output path. Defaults to <plugin-dir>/index.json.
        #[arg(long)]
        output: Option<PathBuf>,

        /// HTTP base URL that will serve <plugin-dir> contents.
        #[arg(
            long,
            default_value = "https://raw.githubusercontent.com/drl990114/cleanr/main/plugins"
        )]
        base_url: String,

        /// Validate the existing output instead of writing it.
        #[arg(long)]
        check: bool,
    },

    /// Install a plugin from an index.
    Install {
        /// Plugin manifest ID to install.
        id: String,

        /// Plugin index URL. Defaults to the GitHub raw URL for --github-repo and --github-ref.
        #[arg(long)]
        index_url: Option<String>,

        /// GitHub repository in owner/name form.
        #[arg(long, default_value = "drl990114/cleanr")]
        github_repo: String,

        /// GitHub ref used when reading plugins/index.json.
        #[arg(long, default_value = "main")]
        github_ref: String,

        /// Override the plugin output directory.
        #[arg(long)]
        plugin_dir: Option<PathBuf>,

        /// Replace an installed plugin with the same ID.
        #[arg(long)]
        force: bool,

        /// Trust the installed plugin so high-confidence rules may preselect cleanup items.
        #[arg(long)]
        trust: bool,

        /// Install files without adding rule packs to cleanup.enabled_rule_packs.
        #[arg(long)]
        no_enable: bool,
    },

    /// Update installed plugins from their recorded index.
    Update {
        /// Optional plugin ID. Updates all installed plugins when omitted.
        id: Option<String>,

        /// Reinstall even when the index version is not newer.
        #[arg(long)]
        force: bool,
    },

    /// Remove an installed plugin.
    Remove { id: String },

    /// Add a local plugin bundle to the configured plugin paths.
    Link {
        path: PathBuf,

        /// Trust the linked plugin so high-confidence rules may preselect cleanup items.
        #[arg(long)]
        trust: bool,

        /// Link without adding rule packs to cleanup.enabled_rule_packs.
        #[arg(long)]
        no_enable: bool,
    },

    /// Remove a linked plugin bundle from the configured plugin paths.
    Unlink { id: String },

    /// List local installed and linked plugins.
    List,

    /// Search plugins in an index.
    Search {
        query: Option<String>,

        #[arg(long)]
        index_url: Option<String>,

        #[arg(long, default_value = "drl990114/cleanr")]
        github_repo: String,

        #[arg(long, default_value = "main")]
        github_ref: String,
    },

    /// Show local or index metadata for one plugin.
    Info {
        id: String,

        #[arg(long)]
        index_url: Option<String>,

        #[arg(long, default_value = "drl990114/cleanr")]
        github_repo: String,

        #[arg(long, default_value = "main")]
        github_ref: String,

        /// Only inspect locally discovered plugins.
        #[arg(long)]
        local: bool,
    },

    /// Mark a plugin as trusted for high-confidence default selection.
    Trust { id: String },

    /// Remove plugin trust.
    Untrust { id: String },

    /// Inspect plugin discovery, manifests, and diagnostics.
    Doctor,
}

fn main() -> Result<()> {
    let args = Args::parse();
    validate_inactive_days_scope(&args)?;

    if let Some(command) = args.command {
        return match command {
            Command::Scan {
                paths,
                global,
                global_kinds,
                json,
            } => workflow::scan(workflow::ScanCommand {
                config_path: args.config,
                request: scan_request(paths, global, global_kinds, args.inactive_days),
                json,
            }),
            Command::Analyze {
                paths,
                global,
                global_kinds,
            } => workflow::analyze(workflow::AnalyzeCommand {
                config_path: args.config,
                request: scan_request(paths, global, global_kinds, args.inactive_days),
            }),
            Command::Plan {
                paths,
                global,
                global_kinds,
                select_paths,
                deselect_paths,
                output,
            } => workflow::plan(workflow::PlanCommand {
                config_path: args.config,
                request: scan_request(paths, global, global_kinds, args.inactive_days),
                selection: workflow::SelectionOverrides {
                    select_paths,
                    deselect_paths,
                },
                output,
            }),
            Command::DryRun {
                paths,
                global,
                global_kinds,
                select_paths,
                deselect_paths,
                json,
                output,
            } => workflow::dry_run(workflow::DryRunCommand {
                config_path: args.config,
                request: scan_request(paths, global, global_kinds, args.inactive_days),
                selection: workflow::SelectionOverrides {
                    select_paths,
                    deselect_paths,
                },
                json,
                output,
            }),
            Command::Clean {
                plan,
                plan_sha256,
                authorized_by_user,
            } => workflow::clean(workflow::CleanCommand {
                config_path: args.config,
                plan_path: plan,
                plan_sha256,
                authorized_by_user,
            }),
            Command::Restore { action } => match action {
                RestoreAction::List => workflow::restore_list(),
                RestoreAction::Run { run_id, confirm } => workflow::restore_run(&run_id, confirm),
            },
            Command::Init {
                locale,
                from_github,
                language_repo,
                language_ref,
                language_sha256,
                language_dir,
            } => init(
                locale,
                from_github,
                language_repo,
                language_ref,
                language_sha256,
                language_dir,
                args.config,
            ),
            Command::Config { action } => match action {
                ConfigAction::Path => config_cmd::path(args.config),
                ConfigAction::Init { force } => config_cmd::init(args.config, force),
                ConfigAction::Get { key } => config_cmd::get(args.config, &key),
                ConfigAction::Set { key, value } => config_cmd::set(args.config, &key, &value),
            },
            Command::Plugin { action } => match action {
                PluginAction::Init {
                    path,
                    id,
                    name,
                    force,
                } => plugin_cmd::init(plugin_cmd::InitOptions {
                    path,
                    id,
                    name,
                    force,
                }),
                PluginAction::Validate { paths } => plugin_cmd::validate(&paths),
                PluginAction::Schema { kind } => plugin_cmd::print_schema(&kind),
                PluginAction::Index {
                    plugin_dir,
                    output,
                    base_url,
                    check,
                } => plugin_cmd::generate_index(plugin_cmd::IndexOptions {
                    plugin_dir,
                    output,
                    base_url,
                    check,
                }),
                PluginAction::Install {
                    id,
                    index_url,
                    github_repo,
                    github_ref,
                    plugin_dir,
                    force,
                    trust,
                    no_enable,
                } => plugin_cmd::install(plugin_cmd::InstallOptions {
                    id,
                    index_url,
                    github_repo,
                    github_ref,
                    plugin_dir,
                    config_path: args.config,
                    trust,
                    enable: !no_enable,
                    force,
                }),
                PluginAction::Update { id, force } => {
                    plugin_cmd::update(plugin_cmd::UpdateOptions {
                        id,
                        config_path: args.config,
                        force,
                    })
                }
                PluginAction::Remove { id } => plugin_cmd::remove(plugin_cmd::RemoveOptions {
                    id,
                    config_path: args.config,
                }),
                PluginAction::Link {
                    path,
                    trust,
                    no_enable,
                } => plugin_cmd::link(plugin_cmd::LinkOptions {
                    path,
                    config_path: args.config,
                    trust,
                    enable: !no_enable,
                }),
                PluginAction::Unlink { id } => plugin_cmd::unlink(plugin_cmd::UnlinkOptions {
                    id,
                    config_path: args.config,
                }),
                PluginAction::List => plugin_cmd::list(args.config),
                PluginAction::Search {
                    query,
                    index_url,
                    github_repo,
                    github_ref,
                } => plugin_cmd::search(plugin_cmd::SearchOptions {
                    query,
                    index_url,
                    github_repo,
                    github_ref,
                }),
                PluginAction::Info {
                    id,
                    index_url,
                    github_repo,
                    github_ref,
                    local,
                } => plugin_cmd::info(plugin_cmd::InfoOptions {
                    id,
                    index_url,
                    github_repo,
                    github_ref,
                    config_path: args.config,
                    local,
                }),
                PluginAction::Trust { id } => plugin_cmd::trust(plugin_cmd::TrustOptions {
                    id,
                    config_path: args.config,
                }),
                PluginAction::Untrust { id } => plugin_cmd::untrust(plugin_cmd::TrustOptions {
                    id,
                    config_path: args.config,
                }),
                PluginAction::Doctor => plugin_cmd::doctor(args.config),
            },
        };
    }

    let config_path = args.config.clone();
    let config = match config_path.as_ref() {
        Some(path) => cleanr_config::Config::load_from(path)?,
        None => cleanr_config::Config::load()?,
    };
    if let Some(inactive_days) = args.inactive_days {
        RecommendationPolicy::new(inactive_days)?;
    }

    let roots = if args.paths.is_empty() {
        vec![std::env::current_dir()?]
    } else {
        args.paths
    };

    let update_available = if args.no_update_check {
        None
    } else {
        update::check_for_update(env!("CARGO_PKG_VERSION"))
    }
    .map(|update| cleanr_tui::UpdateNotice {
        version: update.version,
        release_url: update.release_url,
    });

    let _ = args.log_file;
    cleanr_tui::run_with_config_path_and_inactivity_override(
        cleanr_tui::TuiOptions {
            roots,
            config,
            update_available,
        },
        config_path,
        args.inactive_days,
    )
}

fn validate_inactive_days_scope(args: &Args) -> Result<()> {
    if args.inactive_days.is_some()
        && !matches!(
            args.command.as_ref(),
            None | Some(
                Command::Scan { .. }
                    | Command::Analyze { .. }
                    | Command::Plan { .. }
                    | Command::DryRun { .. }
            )
        )
    {
        bail!(
            "--inactive-days is supported only for TUI startup, scan, analyze, plan, and dry-run"
        );
    }
    Ok(())
}

fn scan_request(
    paths: Vec<PathBuf>,
    include_global: bool,
    global_kinds: Vec<GlobalScanKind>,
    inactive_days: Option<u16>,
) -> ScanRequest {
    ScanRequest {
        paths,
        include_global: include_global || !global_kinds.is_empty(),
        global_kinds,
        inactive_days,
    }
}

fn init(
    locale: String,
    from_github: bool,
    language_repo: String,
    language_ref: String,
    language_sha256: Option<String>,
    language_dir: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let config_path = config_path
        .or_else(cleanr_config::default_config_path)
        .context("platform config directory is unavailable; pass --config")?;
    let language_dir = language_dir
        .or_else(cleanr_config::default_language_dir)
        .context("platform language directory is unavailable; pass --language-dir")?;

    let mut config = if config_path.exists() {
        cleanr_config::Config::load_from(&config_path)?
    } else {
        cleanr_config::Config::default()
    };

    let language_file = if from_github {
        language_cmd::install_github_language(
            &locale,
            &language_repo,
            &language_ref,
            &language_dir,
            language_sha256.as_deref(),
        )?
    } else {
        cleanr_i18n::seed_builtin_language(&locale, &language_dir)?
    };

    config.i18n.locale = Some(locale);
    if !config.i18n.dirs.iter().any(|dir| dir == &language_dir) {
        config.i18n.dirs.push(language_dir.clone());
    }
    config.save_to(&config_path)?;

    println!("Config written to {}", config_path.display());
    println!("Language file installed at {}", language_file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_global_config_after_nested_subcommand() {
        let args = Args::try_parse_from([
            "cleanr",
            "config",
            "set",
            "ui.theme",
            "dark",
            "--config",
            "/tmp/cleanr.toml",
        ])
        .expect("parse");

        assert_eq!(args.config, Some(PathBuf::from("/tmp/cleanr.toml")));
        assert!(matches!(
            args.command,
            Some(Command::Config {
                action: ConfigAction::Set { key, value }
            }) if key == "ui.theme" && value == "dark"
        ));
    }

    #[test]
    fn plugin_validate_requires_at_least_one_path() {
        assert!(Args::try_parse_from(["cleanr", "plugin", "validate"]).is_err());
        assert!(Args::try_parse_from(["cleanr", "plugin", "validate", "plugin.toml"]).is_ok());
    }

    #[test]
    fn parses_non_interactive_workflow_commands() {
        let scan = Args::try_parse_from([
            "cleanr",
            "--config",
            "/tmp/cleanr.toml",
            "scan",
            "--json",
            "--inactive-days",
            "30",
            "--global",
            "--global-kind",
            "browser-caches",
            "/repo/with spaces",
        ])
        .expect("parse scan");
        validate_inactive_days_scope(&scan).expect("scan supports inactivity override");
        assert_eq!(scan.config, Some(PathBuf::from("/tmp/cleanr.toml")));
        assert_eq!(scan.inactive_days, Some(30));
        assert!(matches!(
            scan.command,
            Some(Command::Scan { paths, global: true, global_kinds, json: true })
                if paths == vec![PathBuf::from("/repo/with spaces")]
                    && global_kinds == vec![GlobalScanKind::BrowserCaches]
        ));

        let analyze = Args::try_parse_from([
            "cleanr",
            "analyze",
            "--inactive-days=120",
            "--global-kind",
            "developer-caches",
            "/repo",
        ])
        .expect("parse analyze");
        validate_inactive_days_scope(&analyze).expect("analyze supports inactivity override");
        assert_eq!(analyze.inactive_days, Some(120));
        assert!(matches!(
            analyze.command,
            Some(Command::Analyze {
                paths,
                global: false,
                global_kinds,
            }) if paths == vec![PathBuf::from("/repo")]
                && global_kinds == vec![GlobalScanKind::DeveloperCaches]
        ));
        assert!(
            Args::try_parse_from(["cleanr", "analyze", "--preselect-after-days", "120",]).is_err()
        );

        let plan = Args::try_parse_from(["cleanr", "plan", "--inactive-days", "3650", "/repo"])
            .expect("parse plan");
        validate_inactive_days_scope(&plan).expect("plan supports inactivity override");
        assert_eq!(plan.inactive_days, Some(3650));
        assert!(matches!(
            plan.command,
            Some(Command::Plan { paths, .. }) if paths == vec![PathBuf::from("/repo")]
        ));

        let dry_run = Args::try_parse_from([
            "cleanr",
            "dry-run",
            "--inactive-days",
            "0",
            "--output",
            "/tmp/plan.json",
            "--select",
            "/repo/cache",
            "--deselect",
            "/repo/keep",
            "/repo",
        ])
        .expect("parse dry-run");
        validate_inactive_days_scope(&dry_run).expect("dry-run supports inactivity override");
        assert_eq!(dry_run.inactive_days, Some(0));
        assert!(matches!(
            dry_run.command,
            Some(Command::DryRun {
                paths,
                global: false,
                global_kinds,
                select_paths,
                deselect_paths,
                json: false,
                output: Some(path),
            })
                if paths == vec![PathBuf::from("/repo")]
                    && global_kinds.is_empty()
                    && select_paths == vec![PathBuf::from("/repo/cache")]
                    && deselect_paths == vec![PathBuf::from("/repo/keep")]
                    && path.as_path() == std::path::Path::new("/tmp/plan.json")
        ));

        let restore = Args::try_parse_from(["cleanr", "restore", "run", "run-1", "--confirm"])
            .expect("parse restore");
        assert!(matches!(
            restore.command,
            Some(Command::Restore {
                action: RestoreAction::Run { run_id, confirm: true }
            }) if run_id == "run-1"
        ));

        let clean = Args::try_parse_from([
            "cleanr",
            "clean",
            "--plan",
            "/tmp/cleanr-plan.json",
            "--plan-sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--authorized-by-user",
        ])
        .expect("parse clean");
        assert!(matches!(
            clean.command,
            Some(Command::Clean {
                plan,
                plan_sha256,
                authorized_by_user: true,
            }) if plan == std::path::Path::new("/tmp/cleanr-plan.json")
                && plan_sha256 == "a".repeat(64)
        ));
    }

    #[test]
    fn restore_run_requires_confirm_before_lookup() {
        let error = workflow::restore_run("missing-run", false)
            .expect_err("restore must require explicit confirmation");

        assert!(error.to_string().contains("--confirm"));
    }

    #[test]
    fn clean_requires_authorization_before_plan_lookup() {
        let error = workflow::clean(workflow::CleanCommand {
            config_path: None,
            plan_path: PathBuf::from("/missing/cleanr-plan.json"),
            plan_sha256: "a".repeat(64),
            authorized_by_user: false,
        })
        .expect_err("clean must require explicit user authorization");

        assert!(error.to_string().contains("--authorized-by-user"));
    }

    #[test]
    fn language_hash_requires_github_install_mode() {
        assert!(
            Args::try_parse_from([
                "cleanr",
                "init",
                "--locale",
                "zh-CN",
                "--language-sha256",
                "abc",
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "cleanr",
                "init",
                "--locale",
                "zh-CN",
                "--from-github",
                "--language-sha256",
                "abc",
            ])
            .is_ok()
        );
    }

    #[test]
    fn paths_are_preserved_when_launching_the_tui_mode() {
        let args = Args::try_parse_from([
            "cleanr",
            "--no-update-check",
            "--inactive-days",
            "45",
            "/repo/one",
            "/repo/two",
        ])
        .expect("parse");

        assert!(args.command.is_none());
        assert!(args.no_update_check);
        assert_eq!(args.inactive_days, Some(45));
        validate_inactive_days_scope(&args).expect("TUI startup supports inactivity override");
        assert_eq!(
            args.paths,
            vec![PathBuf::from("/repo/one"), PathBuf::from("/repo/two")]
        );
    }

    #[test]
    fn scan_request_preserves_the_one_scan_inactivity_override() {
        let request = scan_request(vec![PathBuf::from("/repo")], false, Vec::new(), Some(30));

        assert_eq!(request.inactive_days, Some(30));
    }

    #[test]
    fn inactivity_override_is_rejected_for_unrelated_commands() {
        let unsupported = vec![
            vec!["cleanr", "config", "path", "--inactive-days", "30"],
            vec!["cleanr", "restore", "list", "--inactive-days", "30"],
            vec!["cleanr", "plugin", "doctor", "--inactive-days", "30"],
            vec![
                "cleanr",
                "clean",
                "--plan",
                "plan.json",
                "--plan-sha256",
                "abc",
                "--inactive-days",
                "30",
            ],
        ];

        for argv in unsupported {
            let args = Args::try_parse_from(argv).expect("global option parses before scope check");
            let error = validate_inactive_days_scope(&args)
                .expect_err("unrelated command must reject inactivity override");
            assert!(error.to_string().contains("supported only"));
        }
    }
}
