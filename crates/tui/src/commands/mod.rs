use std::path::PathBuf;

use anyhow::{Result, bail};
use cleanr_core::{GlobalScanKind, MAX_RECOMMENDATION_AGE_DAYS, RecommendationPolicy, ScanRequest};
use cleanr_i18n::I18n;

#[derive(Debug)]
enum InactiveDaysCommandError {
    Missing,
    InvalidInteger,
    Duplicate,
    OutOfRange,
}

impl std::fmt::Display for InactiveDaysCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(formatter, "--inactive-days requires a value"),
            Self::InvalidInteger => write!(formatter, "--inactive-days must be an integer"),
            Self::Duplicate => write!(formatter, "--inactive-days may be supplied only once"),
            Self::OutOfRange => write!(
                formatter,
                "--inactive-days must be in 0..={MAX_RECOMMENDATION_AGE_DAYS}"
            ),
        }
    }
}

impl std::error::Error for InactiveDaysCommandError {}

pub(crate) fn localized_command_error(error: &anyhow::Error, i18n: &I18n) -> String {
    let Some(error) = error.downcast_ref::<InactiveDaysCommandError>() else {
        return error.to_string();
    };
    let key = match error {
        InactiveDaysCommandError::Missing => "error_inactive_days_missing",
        InactiveDaysCommandError::InvalidInteger => "error_inactive_days_integer",
        InactiveDaysCommandError::Duplicate => "error_inactive_days_duplicate",
        InactiveDaysCommandError::OutOfRange => "error_inactive_days_range",
    };
    i18n.format(key, &[("max", MAX_RECOMMENDATION_AGE_DAYS.to_string())])
}

/// A TUI command action. This is intentionally local UI plumbing, not an AI protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionRequest {
    Scan(ScanRequest),
    Review,
    Plan,
    Clean { intent: CleanupIntent },
    Restore,
    Rules,
    Plugins,
    Languages,
    Tasks,
    Usage(ScanRequest),
    ExportPlan(Option<PathBuf>),
    Help,
    Quit,
}

/// Whether a cleanup command came from an ordinary local request or the local confirmation UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanupIntent {
    UserRequest,
    ExplicitUserConfirmation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandInfo {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) description_key: &'static str,
    pub(crate) requires_scan: bool,
}

pub(crate) fn filtered_palette_commands(
    has_scan_results: bool,
    input: &str,
    i18n: &I18n,
) -> Vec<CommandInfo> {
    // Arguments should not make the command list disappear. Only the command token drives
    // palette filtering; the full line is still parsed when submitted.
    let filter = input
        .strip_prefix('/')
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    command_palette(has_scan_results)
        .into_iter()
        .filter(|command| {
            filter.is_empty()
                || command.name.to_lowercase().contains(&filter)
                || i18n
                    .t(command.description_key)
                    .to_lowercase()
                    .contains(&filter)
        })
        .collect()
}

pub(crate) fn command_name_for_status(action: &ActionRequest) -> &'static str {
    match action {
        ActionRequest::Scan(_) => "/scan",
        ActionRequest::Review => "/review",
        ActionRequest::Plan => "/plan",
        ActionRequest::Clean { .. } => "/clean",
        ActionRequest::Restore => "/restore",
        ActionRequest::Rules => "/rules",
        ActionRequest::Plugins => "/plugins",
        ActionRequest::Languages => "/languages",
        ActionRequest::Tasks => "/tasks",
        ActionRequest::Usage(_) => "/usage",
        ActionRequest::ExportPlan(_) => "/export-plan",
        ActionRequest::Help => "/help",
        ActionRequest::Quit => "/quit",
    }
}

pub(crate) fn palette_command_invocation(name: &str) -> String {
    name.split_whitespace()
        .filter(|part| !part.starts_with('['))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_slash_command(input: &str) -> Result<ActionRequest> {
    let mut parts = split_command(input)?;
    if parts.is_empty() {
        bail!("empty command");
    }
    let command = parts.remove(0);

    match command.as_str() {
        "/scan" => Ok(ActionRequest::Scan(parse_scan_request(parts)?)),
        "/review" => Ok(ActionRequest::Review),
        "/plan" => Ok(ActionRequest::Plan),
        "/clean" => Ok(ActionRequest::Clean {
            intent: if parts.iter().any(|arg| arg == "--confirm") {
                CleanupIntent::ExplicitUserConfirmation
            } else {
                CleanupIntent::UserRequest
            },
        }),
        "/restore" => Ok(ActionRequest::Restore),
        "/rules" => Ok(ActionRequest::Rules),
        "/plugins" => Ok(ActionRequest::Plugins),
        "/languages" | "/lang" => Ok(ActionRequest::Languages),
        "/tasks" => Ok(ActionRequest::Tasks),
        "/usage" | "/stats" => Ok(ActionRequest::Usage(parse_scan_request(parts)?)),
        "/export-plan" => Ok(ActionRequest::ExportPlan(parts.first().map(PathBuf::from))),
        "/help" => Ok(ActionRequest::Help),
        "/quit" | "/q" => Ok(ActionRequest::Quit),
        other => bail!("unknown command: {other}"),
    }
}

fn parse_scan_request(args: Vec<String>) -> Result<ScanRequest> {
    let mut request = ScanRequest::default();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--global" {
            request.include_global = true;
        } else if arg == "--global-kind" {
            index += 1;
            let Some(kind) = args.get(index) else {
                bail!("--global-kind requires a value");
            };
            request.include_global = true;
            request.global_kinds.push(parse_global_kind(kind)?);
        } else if let Some(kind) = arg.strip_prefix("--global-kind=") {
            request.include_global = true;
            request.global_kinds.push(parse_global_kind(kind)?);
        } else if arg == "--inactive-days" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err(InactiveDaysCommandError::Missing.into());
            };
            set_inactive_days(&mut request, value)?;
        } else if let Some(value) = arg.strip_prefix("--inactive-days=") {
            set_inactive_days(&mut request, value)?;
        } else if arg.starts_with("--") {
            bail!("unsupported scan option: {arg}");
        } else {
            request.paths.push(PathBuf::from(arg));
        }
        index += 1;
    }
    request.global_kinds.sort();
    request.global_kinds.dedup();
    Ok(request)
}

fn parse_global_kind(value: &str) -> Result<GlobalScanKind> {
    value.parse().map_err(anyhow::Error::msg)
}

fn set_inactive_days(request: &mut ScanRequest, value: &str) -> Result<()> {
    if request.inactive_days.is_some() {
        return Err(InactiveDaysCommandError::Duplicate.into());
    }
    let days = value
        .parse::<u64>()
        .map_err(|_| InactiveDaysCommandError::InvalidInteger)?;
    if days > u64::from(MAX_RECOMMENDATION_AGE_DAYS) {
        return Err(InactiveDaysCommandError::OutOfRange.into());
    }
    let days = u16::try_from(days).map_err(|_| InactiveDaysCommandError::OutOfRange)?;
    RecommendationPolicy::new(days).map_err(|_| InactiveDaysCommandError::OutOfRange)?;
    request.inactive_days = Some(days);
    Ok(())
}

fn split_command(input: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }

    if escaped {
        current.push('\\');
    }
    if let Some(active) = quote {
        bail!("unterminated {active} quote");
    }
    if !current.is_empty() {
        parts.push(current);
    }
    Ok(parts)
}

fn command_palette(has_scan_results: bool) -> Vec<CommandInfo> {
    vec![
        CommandInfo {
            name: "/scan [path...]",
            description: "Scan current or specified roots",
            description_key: "command_scan",
            requires_scan: false,
        },
        CommandInfo {
            name: "/scan --global",
            description: "Scan all known system cleanup locations",
            description_key: "command_scan_global",
            requires_scan: false,
        },
        CommandInfo {
            name: "/usage [path...]",
            description: "Scan current or specified roots and show usage",
            description_key: "command_usage",
            requires_scan: false,
        },
        CommandInfo {
            name: "/usage --global",
            description: "Scan known system cleanup locations and show usage",
            description_key: "command_usage_global",
            requires_scan: false,
        },
        CommandInfo {
            name: "/review",
            description: "Build and show cleanup candidates",
            description_key: "command_review",
            requires_scan: true,
        },
        CommandInfo {
            name: "/plan",
            description: "Generate a machine-readable cleanup plan",
            description_key: "command_plan",
            requires_scan: true,
        },
        CommandInfo {
            name: "/clean",
            description: "Preview the selected cleanup plan",
            description_key: "command_clean",
            requires_scan: true,
        },
        CommandInfo {
            name: "/clean --confirm",
            description: "Move selected items to the system trash",
            description_key: "command_clean_confirm",
            requires_scan: true,
        },
        CommandInfo {
            name: "/export-plan [path]",
            description: "Write the current JSON plan to disk",
            description_key: "command_export_plan",
            requires_scan: true,
        },
        CommandInfo {
            name: "/restore",
            description: "Restore items from a cleanup run",
            description_key: "command_restore",
            requires_scan: false,
        },
        CommandInfo {
            name: "/rules",
            description: "Show active cleanup rules",
            description_key: "command_rules",
            requires_scan: false,
        },
        CommandInfo {
            name: "/plugins",
            description: "Show loaded declaration-only plugins",
            description_key: "command_plugins",
            requires_scan: false,
        },
        CommandInfo {
            name: "/languages",
            description: "Show loaded language packs",
            description_key: "command_languages",
            requires_scan: false,
        },
        CommandInfo {
            name: "/tasks",
            description: "Show task history",
            description_key: "command_tasks",
            requires_scan: false,
        },
        CommandInfo {
            name: "/help",
            description: "Show command help",
            description_key: "command_help",
            requires_scan: false,
        },
        CommandInfo {
            name: "/quit",
            description: "Quit cleanr",
            description_key: "command_quit",
            requires_scan: false,
        },
    ]
    .into_iter()
    .filter(|command| !command.requires_scan || has_scan_results)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scan_command_with_paths() {
        let action = parse_slash_command("/scan . /tmp").expect("parse");
        assert_eq!(
            action,
            ActionRequest::Scan(ScanRequest::paths(vec![
                PathBuf::from("."),
                PathBuf::from("/tmp"),
            ]))
        );
    }

    #[test]
    fn parses_quoted_slash_command_arguments() {
        let action =
            parse_slash_command(r#"/scan "/tmp/project with spaces" '/tmp/other path' --global"#)
                .expect("parse");
        assert_eq!(
            action,
            ActionRequest::Scan(ScanRequest {
                paths: vec![
                    PathBuf::from("/tmp/project with spaces"),
                    PathBuf::from("/tmp/other path"),
                ],
                include_global: true,
                global_kinds: Vec::new(),
                inactive_days: None,
            })
        );
        assert_eq!(
            parse_slash_command(r#"/export-plan "plans/cleanr plan.json""#).expect("parse"),
            ActionRequest::ExportPlan(Some(PathBuf::from("plans/cleanr plan.json")))
        );
        assert!(parse_slash_command(r#"/scan "unterminated"#).is_err());
    }

    #[test]
    fn parses_global_scan_kinds() {
        assert_eq!(
            parse_slash_command("/scan --global-kind browser-caches --global-kind logs")
                .expect("parse"),
            ActionRequest::Scan(ScanRequest {
                paths: Vec::new(),
                include_global: true,
                global_kinds: vec![GlobalScanKind::BrowserCaches, GlobalScanKind::Logs],
                inactive_days: None,
            })
        );
        assert!(parse_slash_command("/scan --global-kind unknown").is_err());
    }

    #[test]
    fn parses_and_validates_scan_inactivity_override() {
        assert_eq!(
            parse_slash_command("/scan --inactive-days 30 /repo").expect("parse"),
            ActionRequest::Scan(ScanRequest {
                paths: vec![PathBuf::from("/repo")],
                include_global: false,
                global_kinds: Vec::new(),
                inactive_days: Some(30),
            })
        );
        assert_eq!(
            parse_slash_command("/scan --inactive-days=0").expect("parse"),
            ActionRequest::Scan(ScanRequest {
                inactive_days: Some(0),
                ..ScanRequest::default()
            })
        );
        assert!(parse_slash_command("/scan --inactive-days 3651").is_err());
        assert!(parse_slash_command("/scan --inactive-days nope").is_err());
        assert!(parse_slash_command("/scan --inactive-days").is_err());
        assert!(parse_slash_command("/scan --unknown").is_err());
    }

    #[test]
    fn parses_clean_confirm_as_explicit_local_confirmation() {
        assert_eq!(
            parse_slash_command("/clean --confirm").expect("parse"),
            ActionRequest::Clean {
                intent: CleanupIntent::ExplicitUserConfirmation,
            }
        );
    }

    #[test]
    fn palette_hides_scan_dependent_commands_until_results_exist() {
        let before_scan = command_palette(false);
        assert!(before_scan.iter().all(|command| !command.requires_scan));
        assert!(!before_scan.iter().any(|command| command.name == "/review"));

        let after_scan = command_palette(true);
        assert!(after_scan.iter().any(|command| command.name == "/review"));
        assert!(
            after_scan
                .iter()
                .any(|command| command.name == "/clean --confirm")
        );
    }
}
