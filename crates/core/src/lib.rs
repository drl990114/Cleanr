#![forbid(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod evidence;

pub use evidence::{
    ANALYSIS_REPORT_SCHEMA_VERSION, ActivityEvidence, ActivitySource, ActivityStatus, AnalysisId,
    AnalysisReport, CandidateCoverage, CandidateEvidence, CandidateId, DecisionCode,
    MAX_RECOMMENDATION_AGE_DAYS, OverlapEvidence, RecommendationDecision, RecommendationPolicy,
    RecommendationPolicyError, RecommendationState, ReportIntegrity, RuleEvidence, RuleKey,
    RuleResolution, RuleResolutionState, ScanEvidence, ScanIssue, ScanIssueCode, UserSelection,
    build_analysis_report, build_analysis_report_with_safety_policy,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(default)]
pub struct ScanRequest {
    pub paths: Vec<PathBuf>,
    pub include_global: bool,
    pub global_kinds: Vec<GlobalScanKind>,
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
    pub summary: PlanSummary,
    pub items: Vec<CleanupItem>,
    pub safety: PlanSafety,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RulesetVersion {
    pub id: String,
    pub version: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupItemFingerprint {
    pub descendants: usize,
    pub total_size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifest {
    pub schema_version: String,
    pub run_id: String,
    pub created_at: DateTime<Utc>,
    pub plan_schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<ExecutionAuthorization>,
    pub summary: ExecutionSummary,
    pub items: Vec<ExecutionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAuthorization {
    pub source: CleanupAuthorizationSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAuthorizationSource {
    LocalUserConfirmation,
    ExplicitUserDelegation,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ExecutionSummary {
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionItem {
    pub path: PathBuf,
    pub planned_action: PlannedAction,
    pub status: ExecutionStatus,
    pub rule_id: String,
    pub rollback_receipt: Option<RollbackReceipt>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionStatus {
    Pending,
    Trashed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollbackReceipt {
    pub method: String,
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreManifest {
    pub schema_version: String,
    pub restore_id: String,
    pub source_run_id: String,
    pub created_at: DateTime<Utc>,
    pub summary: RestoreSummary,
    pub items: Vec<RestoreItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RestoreSummary {
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreItem {
    pub path: PathBuf,
    pub status: RestoreStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreStatus {
    Restored,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafetyPolicy {
    protected_paths: Vec<PathBuf>,
    protected_subtrees: Vec<PathBuf>,
    requires_confirmation: bool,
}

impl SafetyPolicy {
    #[must_use]
    pub fn new(protected_paths: Vec<PathBuf>, requires_confirmation: bool) -> Self {
        Self {
            protected_paths: normalize_protected_paths(protected_paths),
            protected_subtrees: Vec::new(),
            requires_confirmation,
        }
    }

    #[must_use]
    pub fn with_protected_subtrees(mut self, protected_subtrees: Vec<PathBuf>) -> Self {
        self.protected_subtrees = normalize_protected_paths(protected_subtrees);
        self
    }

    #[must_use]
    pub fn protected_paths(&self) -> &[PathBuf] {
        &self.protected_paths
    }

    #[must_use]
    pub fn protected_subtrees(&self) -> &[PathBuf] {
        &self.protected_subtrees
    }

    #[must_use]
    pub fn allows_candidate(&self, path: &std::path::Path) -> bool {
        let normalized_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        !is_filesystem_root(&normalized_path)
            && !self
                .protected_paths
                .iter()
                .any(|protected| protected.starts_with(&normalized_path))
            && !self.protected_subtrees.iter().any(|protected| {
                protected.starts_with(&normalized_path) || normalized_path.starts_with(protected)
            })
    }
}

#[must_use]
pub fn build_cleanup_plan(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    entries: &[ScanEntry],
) -> CleanupPlan {
    build_cleanup_plan_with_policy(
        scan_roots,
        ruleset_versions,
        entries,
        &SafetyPolicy::new(Vec::new(), true),
    )
}

#[must_use]
pub fn build_cleanup_plan_with_policy(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    entries: &[ScanEntry],
    policy: &SafetyPolicy,
) -> CleanupPlan {
    let normalized_scan_roots = normalize_protected_paths(scan_roots.clone());
    let tree_fingerprints = tree_fingerprints(entries);
    let items = entries
        .iter()
        .filter_map(|entry| {
            let hit = best_hit(entry)?;
            let normalized_path = entry
                .path
                .canonicalize()
                .unwrap_or_else(|_| entry.path.clone());
            if normalized_scan_roots
                .iter()
                .any(|root| root == &normalized_path)
                || !policy.allows_candidate(&entry.path)
            {
                return None;
            }
            let selected = hit.default_selected
                && hit.confidence == Confidence::High
                && hit.trust != RuleTrust::Untrusted;
            Some(CleanupItem {
                path: entry.path.clone(),
                kind: entry.kind,
                size_bytes: entry.size_bytes,
                modified_at: entry.modified_at,
                tree_fingerprint: (entry.kind == EntryKind::Directory)
                    .then(|| tree_fingerprints.get(&entry.path).cloned())
                    .flatten(),
                rule_id: format!("{}:{}", hit.rule_pack_id, hit.rule_id),
                category: hit.category.clone(),
                confidence: hit.confidence,
                reason: hit.reason.clone(),
                risk_note: hit.risk_note.clone(),
                evidence: None,
                selected,
                planned_action: PlannedAction::Trash,
                rollback_method: "system-trash+manifest".to_string(),
            })
        })
        .collect::<Vec<_>>();

    finish_cleanup_plan(
        scan_roots,
        ruleset_versions,
        remove_overlapping_items(items),
        policy,
    )
}

/// Build a cleanup plan from a single immutable analysis report and its user-owned selection.
///
/// This is the product-facing plan builder: recommendation and overlap decisions come only from
/// `analysis`, while `selection` records the user's later choices. The local safety policy is
/// applied again defensively even though safety-aware analysis already records excluded paths.
#[must_use]
pub fn build_cleanup_plan_from_analysis(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    entries: &[ScanEntry],
    analysis: &AnalysisReport,
    selection: &UserSelection,
    policy: &SafetyPolicy,
) -> CleanupPlan {
    let normalized_scan_roots = normalize_protected_paths(scan_roots.clone());
    let tree_fingerprints = tree_fingerprints(entries);
    let entries_by_path = entries
        .iter()
        .map(|entry| (entry.path.as_path(), entry))
        .collect::<HashMap<_, _>>();
    let items = analysis
        .candidates
        .iter()
        .filter_map(|candidate| {
            if matches!(
                candidate.recommendation.state,
                RecommendationState::Suppressed | RecommendationState::Excluded
            ) {
                return None;
            }
            let normalized_path = candidate
                .local_path
                .canonicalize()
                .unwrap_or_else(|_| candidate.local_path.clone());
            if normalized_scan_roots
                .iter()
                .any(|root| root == &normalized_path)
                || !policy.allows_candidate(&candidate.local_path)
            {
                return None;
            }
            let rule = candidate
                .rules
                .primary
                .as_ref()
                .and_then(|key| candidate.rules.matched.iter().find(|rule| &rule.key == key))
                // Unresolved conflicts remain visible for an explicitly confirmed user choice;
                // use their stable first rule only for plan display fields, never selection.
                .or_else(|| candidate.rules.matched.first())?;
            let modified_at = entries_by_path
                .get(candidate.local_path.as_path())
                .and_then(|entry| entry.modified_at);
            Some(CleanupItem {
                path: candidate.local_path.clone(),
                kind: candidate.kind,
                size_bytes: candidate.size_bytes,
                modified_at,
                tree_fingerprint: (candidate.kind == EntryKind::Directory)
                    .then(|| tree_fingerprints.get(&candidate.local_path).cloned())
                    .flatten(),
                rule_id: format!("{}:{}", rule.key.rule_pack_id, rule.key.rule_id),
                category: rule.category.clone(),
                confidence: rule.confidence,
                reason: rule.reason.clone(),
                risk_note: rule.risk_note.clone(),
                evidence: Some(CleanupItemEvidence {
                    recommendation_state: candidate.recommendation.state,
                    decision_codes: candidate.recommendation.codes.clone(),
                    rule_resolution_state: candidate.rules.state,
                    matched_rules: candidate.rules.matched.clone(),
                    shadowed_rules: candidate.rules.shadowed.clone(),
                }),
                selected: selection.candidate_ids.contains(&candidate.id),
                planned_action: PlannedAction::Trash,
                rollback_method: candidate.rollback_method.clone(),
            })
        })
        .collect::<Vec<_>>();

    finish_cleanup_plan(scan_roots, ruleset_versions, items, policy)
}

fn finish_cleanup_plan(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    items: Vec<CleanupItem>,
    policy: &SafetyPolicy,
) -> CleanupPlan {
    let mut items = items;
    items.sort_by(|a, b| {
        b.selected
            .cmp(&a.selected)
            .then_with(|| b.confidence.cmp(&a.confidence))
            .then_with(|| b.size_bytes.cmp(&a.size_bytes))
            .then_with(|| a.path.cmp(&b.path))
    });

    let summary = PlanSummary {
        candidate_count: items.len(),
        selected_count: items.iter().filter(|item| item.selected).count(),
        selected_size_bytes: items
            .iter()
            .filter(|item| item.selected)
            .map(|item| item.size_bytes)
            .sum(),
        total_candidate_size_bytes: items.iter().map(|item| item.size_bytes).sum(),
    };

    CleanupPlan {
        schema_version: CLEANUP_PLAN_SCHEMA_VERSION.to_string(),
        created_at: Utc::now(),
        scan_roots,
        ruleset_versions,
        summary,
        items,
        safety: PlanSafety {
            requires_confirmation: policy.requires_confirmation,
            protected_paths: policy.protected_paths.clone(),
            protected_subtrees: policy.protected_subtrees.clone(),
            ..PlanSafety::default()
        },
    }
}

fn remove_overlapping_items(mut items: Vec<CleanupItem>) -> Vec<CleanupItem> {
    items.sort_by(|a, b| {
        b.selected
            .cmp(&a.selected)
            .then_with(|| b.confidence.cmp(&a.confidence))
            .then_with(|| b.size_bytes.cmp(&a.size_bytes))
            .then_with(|| {
                a.path
                    .components()
                    .count()
                    .cmp(&b.path.components().count())
            })
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut non_overlapping: Vec<CleanupItem> = Vec::with_capacity(items.len());
    let mut kept_paths = HashSet::with_capacity(items.len());
    let mut kept_ancestor_paths = HashSet::with_capacity(items.len());
    for item in items {
        if overlaps_kept_path(&item.path, &kept_paths, &kept_ancestor_paths) {
            continue;
        }
        remember_kept_path(&item.path, &mut kept_paths, &mut kept_ancestor_paths);
        non_overlapping.push(item);
    }
    non_overlapping
}

fn overlaps_kept_path(
    path: &Path,
    kept_paths: &HashSet<PathBuf>,
    kept_ancestor_paths: &HashSet<PathBuf>,
) -> bool {
    path.ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .any(|ancestor| kept_paths.contains(ancestor))
        || kept_ancestor_paths.contains(path)
}

fn remember_kept_path(
    path: &Path,
    kept_paths: &mut HashSet<PathBuf>,
    kept_ancestor_paths: &mut HashSet<PathBuf>,
) {
    kept_paths.insert(path.to_path_buf());
    kept_ancestor_paths.extend(
        path.ancestors()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
            .map(Path::to_path_buf),
    );
}

fn tree_fingerprints(entries: &[ScanEntry]) -> HashMap<PathBuf, CleanupItemFingerprint> {
    let mut fingerprints = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .map(|entry| {
            (
                entry.path.clone(),
                CleanupItemFingerprint {
                    descendants: 0,
                    total_size_bytes: entry.size_bytes,
                    latest_modified_at: entry.modified_at,
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let mut indices = (0..entries.len()).collect::<Vec<_>>();
    indices.sort_by_key(|idx| std::cmp::Reverse(entries[*idx].path.components().count()));

    for idx in indices {
        let entry = &entries[idx];
        let Some(parent) = entry.path.parent() else {
            continue;
        };
        let child_fingerprint = (entry.kind == EntryKind::Directory)
            .then(|| fingerprints.get(&entry.path).cloned())
            .flatten();
        let descendants = 1 + child_fingerprint
            .as_ref()
            .map_or(0, |fingerprint| fingerprint.descendants);
        let latest_modified_at = child_fingerprint
            .and_then(|fingerprint| fingerprint.latest_modified_at)
            .or(entry.modified_at);
        if let Some(parent_fingerprint) = fingerprints.get_mut(parent) {
            parent_fingerprint.descendants += descendants;
            parent_fingerprint.latest_modified_at =
                max_datetime(parent_fingerprint.latest_modified_at, latest_modified_at);
        }
    }

    fingerprints
}

fn max_datetime(
    left: Option<DateTime<Utc>>,
    right: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn best_hit(entry: &ScanEntry) -> Option<&RuleHit> {
    let resolution = evidence::resolve_rules(&entry.rule_hits);
    let primary = resolution.primary?;
    entry
        .rule_hits
        .iter()
        .find(|hit| hit.rule_pack_id == primary.rule_pack_id && hit.rule_id == primary.rule_id)
}

fn normalize_protected_paths(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    for path in &mut paths {
        if let Ok(canonical) = path.canonicalize() {
            *path = canonical;
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn is_filesystem_root(path: &std::path::Path) -> bool {
    path.is_absolute() && path.parent().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_selects_only_high_confidence_defaults() {
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("node_modules"),
                kind: EntryKind::Directory,
                size_bytes: 10,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "builtin-dev".into(),
                    rule_id: "node-modules".into(),
                    label: "Node modules".into(),
                    category: "developer-cache".into(),
                    confidence: Confidence::High,
                    reason: "reinstallable dependency cache".into(),
                    risk_note: "can be restored by package manager".into(),
                    default_selected: true,
                    trust: RuleTrust::Builtin,
                    match_role: RuleMatchRole::Primary,
                }],
            },
            ScanEntry {
                path: PathBuf::from("Downloads/big.zip"),
                kind: EntryKind::File,
                size_bytes: 20,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "builtin-general".into(),
                    rule_id: "large-download".into(),
                    label: "Large download".into(),
                    category: "downloads".into(),
                    confidence: Confidence::Low,
                    reason: "large download".into(),
                    risk_note: "user data review required".into(),
                    default_selected: false,
                    trust: RuleTrust::Builtin,
                    match_role: RuleMatchRole::Primary,
                }],
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from(".")], vec![], &entries);
        assert_eq!(plan.summary.candidate_count, 2);
        assert_eq!(plan.summary.selected_count, 1);
        assert!(plan.items.iter().any(|item| item.selected));
    }

    #[test]
    fn analysis_plan_uses_the_same_ninety_day_preselection_boundary() {
        let as_of = Utc::now();
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "rebuildable".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
        };
        let entries = [89_i64, 90, 91, 1]
            .into_iter()
            .map(|days| ScanEntry {
                path: PathBuf::from(format!("/repo/cache-{days}")),
                kind: EntryKind::Directory,
                size_bytes: 1,
                modified_at: Some(as_of - chrono::Duration::days(days)),
                rule_hits: vec![hit.clone()],
            })
            .collect::<Vec<_>>();
        let safety = SafetyPolicy::default();
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            RecommendationPolicy::default(),
            &safety,
        )
        .expect("default policy is valid");
        let selection = UserSelection::from_recommendations(&analysis);
        let plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &selection,
            &safety,
        );

        let is_selected = |path: &str| {
            plan.items
                .iter()
                .find(|item| item.path == Path::new(path))
                .expect("candidate appears in plan")
                .selected
        };
        assert!(!is_selected("/repo/cache-89"));
        assert!(is_selected("/repo/cache-90"));
        assert!(is_selected("/repo/cache-91"));
        assert!(!is_selected("/repo/cache-1"));
        let evidence = plan.items[0]
            .evidence
            .as_ref()
            .expect("analysis plan keeps recommendation evidence");
        assert_eq!(evidence.matched_rules.len(), 1);
        assert_eq!(evidence.rule_resolution_state, RuleResolutionState::Single);
        assert!(!evidence.decision_codes.is_empty());
    }

    #[test]
    fn analysis_plan_does_not_let_a_safety_excluded_parent_hide_a_safe_child() {
        let as_of = Utc::now();
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "rebuildable".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
        };
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 10,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit.clone()],
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/child"),
                kind: EntryKind::Directory,
                size_bytes: 5,
                modified_at: Some(as_of - chrono::Duration::days(100)),
                rule_hits: vec![hit],
            },
        ];
        let safety = SafetyPolicy::new(vec![PathBuf::from("/repo/cache/protected")], true);
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[],
            RecommendationPolicy::default(),
            &safety,
        )
        .expect("default policy is valid");
        let parent = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache"))
            .expect("parent candidate");
        let child = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache/child"))
            .expect("child candidate");
        assert_eq!(parent.recommendation.state, RecommendationState::Excluded);
        assert_ne!(child.recommendation.state, RecommendationState::Suppressed);

        let selection = UserSelection::from_recommendations(&analysis);
        let plan = build_cleanup_plan_from_analysis(
            vec![PathBuf::from("/repo")],
            vec![],
            &entries,
            &analysis,
            &selection,
            &safety,
        );
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].path, PathBuf::from("/repo/cache/child"));
        assert!(plan.items[0].selected);
    }

    #[test]
    fn plan_rule_ties_use_a_stable_rule_key_not_input_order() {
        let hit = |rule_id: &str| RuleHit {
            rule_pack_id: "builtin-dev".to_string(),
            rule_id: rule_id.to_string(),
            label: rule_id.to_string(),
            category: "developer-cache".to_string(),
            confidence: Confidence::High,
            reason: "equivalent generated cache".to_string(),
            risk_note: "rebuild".to_string(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
        };
        let make_entry = |rule_hits| ScanEntry {
            path: PathBuf::from("/repo/cache"),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at: None,
            rule_hits,
        };

        let forward = build_cleanup_plan(
            vec![PathBuf::from("/repo")],
            vec![],
            &[make_entry(vec![hit("z-rule"), hit("a-rule")])],
        );
        let reverse = build_cleanup_plan(
            vec![PathBuf::from("/repo")],
            vec![],
            &[make_entry(vec![hit("a-rule"), hit("z-rule")])],
        );

        assert_eq!(forward.items[0].rule_id, "builtin-dev:a-rule");
        assert_eq!(forward.items[0].rule_id, reverse.items[0].rule_id);
    }

    #[test]
    fn plan_serializes_as_json_manifest() {
        let plan = build_cleanup_plan(vec![PathBuf::from(".")], vec![], &[]);
        let json = serde_json::to_string(&plan).expect("plan serializes");
        assert!(json.contains(CLEANUP_PLAN_SCHEMA_VERSION));
        assert!(json.contains("system-trash+manifest"));
    }

    #[test]
    fn plan_removes_overlapping_parent_and_child_candidates() {
        let hit = |rule_id: &str| RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: rule_id.into(),
            label: rule_id.into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "generated".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
        };
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/node_modules"),
                kind: EntryKind::Directory,
                size_bytes: 100,
                modified_at: None,
                rule_hits: vec![hit("node-modules")],
            },
            ScanEntry {
                path: PathBuf::from("/repo/node_modules/pkg/node_modules"),
                kind: EntryKind::Directory,
                size_bytes: 40,
                modified_at: None,
                rule_hits: vec![hit("node-modules")],
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &entries);
        assert_eq!(plan.summary.candidate_count, 1);
        assert_eq!(plan.summary.selected_size_bytes, 100);
        assert_eq!(plan.items[0].path, PathBuf::from("/repo/node_modules"));
    }

    #[test]
    fn directory_candidates_include_tree_fingerprints() {
        let hit = RuleHit {
            rule_pack_id: "builtin-dev".into(),
            rule_id: "cache".into(),
            label: "Cache".into(),
            category: "developer-cache".into(),
            confidence: Confidence::High,
            reason: "generated".into(),
            risk_note: "rebuild".into(),
            default_selected: true,
            trust: RuleTrust::Builtin,
            match_role: RuleMatchRole::Primary,
        };
        let modified_at = Utc::now();
        let newer_modified_at = modified_at + chrono::Duration::seconds(1);
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 7,
                modified_at: Some(modified_at),
                rule_hits: vec![hit],
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/file"),
                kind: EntryKind::File,
                size_bytes: 3,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/nested"),
                kind: EntryKind::Directory,
                size_bytes: 4,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/nested/file"),
                kind: EntryKind::File,
                size_bytes: 4,
                modified_at: Some(newer_modified_at),
                rule_hits: Vec::new(),
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &entries);

        assert_eq!(
            plan.items[0].tree_fingerprint,
            Some(CleanupItemFingerprint {
                descendants: 3,
                total_size_bytes: 7,
                latest_modified_at: Some(newer_modified_at),
            })
        );
    }

    #[test]
    fn selected_child_wins_over_unselected_parent() {
        let entries = vec![
            ScanEntry {
                path: PathBuf::from("/repo/cache"),
                kind: EntryKind::Directory,
                size_bytes: 100,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "custom".into(),
                    rule_id: "review-parent".into(),
                    label: "review".into(),
                    category: "cache".into(),
                    confidence: Confidence::Medium,
                    reason: "review".into(),
                    risk_note: "review".into(),
                    default_selected: false,
                    trust: RuleTrust::Trusted,
                    match_role: RuleMatchRole::Primary,
                }],
            },
            ScanEntry {
                path: PathBuf::from("/repo/cache/generated"),
                kind: EntryKind::Directory,
                size_bytes: 40,
                modified_at: None,
                rule_hits: vec![RuleHit {
                    rule_pack_id: "custom".into(),
                    rule_id: "safe-child".into(),
                    label: "safe".into(),
                    category: "cache".into(),
                    confidence: Confidence::High,
                    reason: "generated".into(),
                    risk_note: "rebuild".into(),
                    default_selected: true,
                    trust: RuleTrust::Trusted,
                    match_role: RuleMatchRole::Primary,
                }],
            },
        ];

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &entries);
        assert_eq!(plan.summary.candidate_count, 1);
        assert_eq!(plan.items[0].path, PathBuf::from("/repo/cache/generated"));
        assert!(plan.items[0].selected);
    }

    #[test]
    fn protected_subtrees_reject_their_children_and_parents() {
        let policy = SafetyPolicy::new(vec![], true)
            .with_protected_subtrees(vec![PathBuf::from("/repo/.cleanr")]);

        assert!(!policy.allows_candidate(std::path::Path::new("/repo")));
        assert!(!policy.allows_candidate(std::path::Path::new("/repo/.cleanr/history")));
        assert!(policy.allows_candidate(std::path::Path::new("/repo/target")));
    }

    #[test]
    fn scan_root_itself_is_never_a_cleanup_candidate() {
        let entry = ScanEntry {
            path: PathBuf::from("/repo"),
            kind: EntryKind::Directory,
            size_bytes: 100,
            modified_at: None,
            rule_hits: vec![RuleHit {
                rule_pack_id: "trusted".into(),
                rule_id: "broad".into(),
                label: "Broad".into(),
                category: "cache".into(),
                confidence: Confidence::High,
                reason: "generated".into(),
                risk_note: "dangerous".into(),
                default_selected: true,
                trust: RuleTrust::Trusted,
                match_role: RuleMatchRole::Primary,
            }],
        };

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &[entry]);

        assert!(plan.items.is_empty());
    }

    #[test]
    fn legacy_plan_omits_unresolved_rule_conflicts() {
        let hit = |rule_id: &str,
                   trust: RuleTrust,
                   confidence: Confidence,
                   default_selected: bool| RuleHit {
            rule_pack_id: "pack".into(),
            rule_id: rule_id.into(),
            label: rule_id.into(),
            category: "cache".into(),
            confidence,
            reason: rule_id.into(),
            risk_note: "review".into(),
            default_selected,
            trust,
            match_role: RuleMatchRole::Primary,
        };
        let entry = ScanEntry {
            path: PathBuf::from("/repo/cache"),
            kind: EntryKind::Directory,
            size_bytes: 42,
            modified_at: None,
            rule_hits: vec![
                hit("untrusted", RuleTrust::Untrusted, Confidence::High, true),
                hit("trusted", RuleTrust::Trusted, Confidence::Medium, false),
                hit("builtin", RuleTrust::Builtin, Confidence::Low, false),
            ],
        };

        let plan = build_cleanup_plan(vec![PathBuf::from("/repo")], vec![], &[entry]);

        assert!(plan.items.is_empty());
    }

    #[test]
    fn protected_paths_are_normalized_sorted_and_deduplicated() {
        let temp = tempfile::tempdir().expect("tempdir");
        let protected = temp.path().join("protected");
        std::fs::create_dir(&protected).expect("protected dir");
        let policy = SafetyPolicy::new(
            vec![
                protected.clone(),
                temp.path().join(".").join("protected"),
                temp.path().join("another"),
            ],
            false,
        );

        assert_eq!(policy.protected_paths().len(), 2);
        assert!(!policy.allows_candidate(&protected));
        assert!(!policy.allows_candidate(temp.path()));
        assert!(!policy.requires_confirmation);
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_root_is_never_allowed() {
        assert!(!SafetyPolicy::default().allows_candidate(std::path::Path::new("/")));
    }
}
