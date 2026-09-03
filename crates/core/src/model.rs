use std::{error::Error, fmt, path::PathBuf, str::FromStr};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::evidence::{
    AnalysisId, DecisionCode, RecommendationPolicy, RecommendationState, ReportIntegrity,
    RuleEvidence, RuleKey, RuleResolutionState, ScanBudgetExceeded,
};

pub const CLEANUP_PLAN_SCHEMA_VERSION: &str = "cleanr.cleanup-plan.v1";
pub const EXECUTION_SCHEMA_VERSION: &str = "cleanr.execution.v1";
pub const RESTORE_SCHEMA_VERSION: &str = "cleanr.restore.v1";

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "kebab-case")]
pub enum GlobalScanKind {
    DeveloperCaches,
    BrowserCaches,
    AppCaches,
    TempFiles,
    Logs,
    Downloads,
}

impl GlobalScanKind {
    pub const ALL: [Self; 6] = [
        Self::DeveloperCaches,
        Self::BrowserCaches,
        Self::AppCaches,
        Self::TempFiles,
        Self::Logs,
        Self::Downloads,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeveloperCaches => "developer-caches",
            Self::BrowserCaches => "browser-caches",
            Self::AppCaches => "app-caches",
            Self::TempFiles => "temp-files",
            Self::Logs => "logs",
            Self::Downloads => "downloads",
        }
    }
}

impl fmt::Display for GlobalScanKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GlobalScanKind {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "developer-caches" => Ok(Self::DeveloperCaches),
            "browser-caches" => Ok(Self::BrowserCaches),
            "app-caches" => Ok(Self::AppCaches),
            "temp-files" => Ok(Self::TempFiles),
            "logs" => Ok(Self::Logs),
            "downloads" => Ok(Self::Downloads),
            _ => Err(format!("unsupported global scan kind: {value}")),
        }
    }
}

#[must_use]
pub fn default_global_scan_kinds() -> Vec<GlobalScanKind> {
    GlobalScanKind::ALL.to_vec()
}

/// A constrained platform directory used as the anchor for plugin-provided scan locations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScanLocationBase {
    Home,
    Cache,
    DataLocal,
    Data,
    Temp,
    Downloads,
}

/// Whether a known location may be traversed or is only reported as operating-system managed.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScanLocationMode {
    #[default]
    Scan,
    OsManaged,
}

/// A bounded way to resolve cache leaves below direct children of a known location.
///
/// The anchor remains a fixed relative path. Globs may match one child name only, and every
/// suffix is another fixed relative path below that child.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScanLocationExpansion {
    pub child_globs: Vec<String>,
    pub suffixes: Vec<String>,
    #[serde(default = "default_scan_location_max_matches")]
    pub max_matches: u16,
}

const fn default_scan_location_max_matches() -> u16 {
    64
}

/// Declarative, path-relative global coverage contributed by a built-in or trusted plugin.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScanLocationDefinition {
    pub id: String,
    pub label: String,
    pub kind: GlobalScanKind,
    pub platforms: Vec<RulePlatform>,
    pub base: ScanLocationBase,
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub mode: ScanLocationMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<ScanLocationExpansion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScanLocationPack {
    pub id: String,
    pub version: String,
    pub locations: Vec<ScanLocationDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(default)]
pub struct ScanRequest {
    pub paths: Vec<PathBuf>,
    pub include_global: bool,
    pub global_kinds: Vec<GlobalScanKind>,
    /// One-scan override for the shared inactivity threshold used by recommendations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactive_days: Option<u16>,
}

impl ScanRequest {
    #[must_use]
    pub fn paths(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn global(global_kinds: Vec<GlobalScanKind>) -> Self {
        Self {
            include_global: true,
            global_kinds,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlannedAction {
    Trash,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum RuleTrust {
    #[default]
    Untrusted,
    Trusted,
    Builtin,
}

/// Operating systems a declarative cleanup rule may target.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "kebab-case")]
pub enum RulePlatform {
    Macos,
    Windows,
    Linux,
}

impl RulePlatform {
    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// How a Cleanr rule used an upstream open-source project during review.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSourceRelation {
    Adapted,
    AuditedAgainst,
    IndependentlyVerified,
}

/// Non-sensitive, revision-pinned provenance for a cleanup rule.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleSource {
    pub id: String,
    pub repository: String,
    pub revision: String,
    pub license: String,
    pub relation: RuleSourceRelation,
}

/// Whether a matching rule is authoritative or only a broad fallback when no trusted primary
/// rule matches the same candidate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleMatchRole {
    #[default]
    Primary,
    Fallback,
}

impl fmt::Display for RuleMatchRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Primary => "primary",
            Self::Fallback => "fallback",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleHit {
    pub rule_pack_id: String,
    pub rule_id: String,
    pub label: String,
    pub category: String,
    pub confidence: Confidence,
    pub reason: String,
    pub risk_note: String,
    pub default_selected: bool,
    #[serde(default)]
    pub trust: RuleTrust,
    #[serde(default)]
    pub match_role: RuleMatchRole,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<RuleSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_guard: Option<crate::evidence::RuntimeGuardEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size_bytes: u64,
    pub modified_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub rule_hits: Vec<RuleHit>,
}

impl ScanEntry {
    #[must_use]
    pub fn file_name(&self) -> Option<String> {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub roots: Vec<PathBuf>,
    pub entries_seen: usize,
    pub errors: usize,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupPlan {
    pub schema_version: String,
    pub created_at: DateTime<Utc>,
    pub scan_roots: Vec<PathBuf>,
    pub ruleset_versions: Vec<RulesetVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_scan: Option<CleanupPlanSourceScan>,
    pub summary: PlanSummary,
    pub items: Vec<CleanupItem>,
    pub safety: PlanSafety,
}

/// Read-only scan provenance retained by plans built from an analysis report.
///
/// Legacy entry-based plan builders leave this absent. Execution rejects any plan whose source
/// analysis exhausted a scan budget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupPlanSourceScan {
    pub analysis_id: AnalysisId,
    pub integrity: ReportIntegrity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub budget_exceeded: Vec<ScanBudgetExceeded>,
    /// Exact recommendation policy used to derive the reviewed candidate set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommendation_policy: Option<RecommendationPolicy>,
    /// Semantic scan scope needed to reproduce global-category suppression during re-scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<CleanupPlanScanScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CleanupPlanScanScope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explicit_roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_kinds: Vec<GlobalScanKind>,
}

impl CleanupPlanScanScope {
    #[must_use]
    pub fn new(mut explicit_roots: Vec<PathBuf>, mut global_kinds: Vec<GlobalScanKind>) -> Self {
        explicit_roots.sort();
        explicit_roots.dedup();
        global_kinds.sort();
        global_kinds.dedup();
        Self {
            explicit_roots,
            global_kinds,
        }
    }

    #[must_use]
    pub fn to_scan_request(&self) -> ScanRequest {
        ScanRequest {
            paths: self.explicit_roots.clone(),
            include_global: !self.global_kinds.is_empty(),
            global_kinds: self.global_kinds.clone(),
            inactive_days: None,
        }
    }
}

/// Why an analysis report cannot be promoted into an executable cleanup plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupPlanBuildError {
    UnsupportedAnalysisSchema { found: String },
    ScanBudgetExceeded,
    OverlappingSelection { left: PathBuf, right: PathBuf },
}

impl fmt::Display for CleanupPlanBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedAnalysisSchema { found } => write!(
                formatter,
                "unsupported analysis report schema for cleanup planning: {found}"
            ),
            Self::ScanBudgetExceeded => formatter.write_str(
                "scan budget was exceeded; analysis is read-only and cannot produce a cleanup plan",
            ),
            Self::OverlappingSelection { left, right } => write!(
                formatter,
                "selected cleanup candidates overlap: {} and {}",
                left.display(),
                right.display()
            ),
        }
    }
}

impl Error for CleanupPlanBuildError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RulesetVersion {
    pub id: String,
    pub version: String,
    /// Immutable upstream inputs that materially shaped this ruleset version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<RuleSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PlanSummary {
    pub candidate_count: usize,
    pub selected_count: usize,
    pub selected_size_bytes: u64,
    pub total_candidate_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanSafety {
    pub default_action: PlannedAction,
    pub requires_confirmation: bool,
    pub rollback_method: String,
    #[serde(default)]
    pub protected_paths: Vec<PathBuf>,
    #[serde(default)]
    pub protected_subtrees: Vec<PathBuf>,
}

impl Default for PlanSafety {
    fn default() -> Self {
        Self {
            default_action: PlannedAction::Trash,
            requires_confirmation: true,
            rollback_method: "system-trash+manifest".to_string(),
            protected_paths: Vec::new(),
            protected_subtrees: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupItem {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_fingerprint: Option<CleanupItemFingerprint>,
    pub rule_id: String,
    pub category: String,
    pub confidence: Confidence,
    pub reason: String,
    pub risk_note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<CleanupItemEvidence>,
    pub selected: bool,
    pub planned_action: PlannedAction,
    pub rollback_method: String,
}

/// Recommendation and rule evidence retained in a cleanup plan for human review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupItemEvidence {
    pub recommendation_state: RecommendationState,
    pub decision_codes: Vec<DecisionCode>,
    pub rule_resolution_state: RuleResolutionState,
    pub matched_rules: Vec<RuleEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed_rules: Vec<RuleKey>,
    /// This item is itself a bounded, named global location rather than an arbitrary scan root.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub known_global_location: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_guards: Vec<crate::evidence::RuntimeGuardEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupItemFingerprint {
    pub descendants: usize,
    pub total_size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_modified_at: Option<DateTime<Utc>>,
}
