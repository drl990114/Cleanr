use std::{
    collections::{BTreeSet, HashMap, HashSet},
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Confidence, EntryKind, RuleHit, RuleMatchRole, RuleTrust, SafetyPolicy, ScanEntry};

/// The schema identifier for the read-only local analysis contract.
pub const ANALYSIS_REPORT_SCHEMA_VERSION: &str = "cleanr.analysis.v1";

/// The largest supported recommendation age threshold. Zero disables the age gate.
pub const MAX_RECOMMENDATION_AGE_DAYS: u16 = 3650;

/// Whether unexpected scan failures left the report incomplete.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReportIntegrity {
    #[default]
    Complete,
    Partial,
}

impl ReportIntegrity {
    /// Derive the report-wide integrity state from its structured issue ledger.
    #[must_use]
    pub fn from_issues(issues: &[ScanIssue]) -> Self {
        if issues.iter().any(|issue| issue.code.makes_report_partial()) {
            Self::Partial
        } else {
            Self::Complete
        }
    }
}

/// A structured local scan condition. Its `path` is deliberately not a remote DTO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanIssue {
    pub code: ScanIssueCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Stable classifications for scan failures and intentional exclusions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ScanIssueCode {
    PermissionDenied,
    MetadataUnavailable,
    RootUnavailable,
    TraversalError,
    IgnoredByConfig,
    CrossFilesystemSkipped,
    Unknown,
}

impl ScanIssueCode {
    /// Intentional exclusions are auditable but do not make the whole report partial.
    #[must_use]
    pub const fn makes_report_partial(self) -> bool {
        match self {
            Self::IgnoredByConfig | Self::CrossFilesystemSkipped => false,
            Self::PermissionDenied
            | Self::MetadataUnavailable
            | Self::RootUnavailable
            | Self::TraversalError
            | Self::Unknown => true,
        }
    }
}

/// Coverage of one candidate's tree at the scan reference time.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateCoverage {
    #[default]
    Complete,
    Partial,
    Unknown,
}

/// Which observed metadata timestamps contributed the candidate activity value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ActivitySource {
    Own,
    Descendant,
    OwnAndDescendant,
    Unavailable,
}

/// Reliability of a candidate's observed modification timestamp.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityStatus {
    Complete,
    Missing,
    Partial,
    AfterAsOf,
}

/// Modification-time evidence. It is not evidence of file access or real user activity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityEvidence {
    pub latest_observed_modified_at: Option<DateTime<Utc>>,
    pub effective_modified_at: Option<DateTime<Utc>>,
    pub source: ActivitySource,
    pub status: ActivityStatus,
    pub age_days: Option<u32>,
}

/// An opaque identifier scoped to one analysis report. It is never derived from a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CandidateId(String);

impl CandidateId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn new_random() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// An opaque identifier scoped to one analysis report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnalysisId(String);

impl AnalysisId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn new_random() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// A versioned policy for a deterministic recommendation pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecommendationPolicy {
    pub version: String,
    pub preselect_after_days: u16,
    pub strict_report_integrity: bool,
}

impl Default for RecommendationPolicy {
    fn default() -> Self {
        Self {
            version: "v1".to_string(),
            preselect_after_days: 90,
            strict_report_integrity: true,
        }
    }
}

impl RecommendationPolicy {
    /// Create the current policy after validating its bounded age threshold.
    pub fn new(preselect_after_days: u16) -> Result<Self, RecommendationPolicyError> {
        let policy = Self {
            preselect_after_days,
            ..Self::default()
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Reject thresholds that make an age recommendation unintelligible.
    pub fn validate(&self) -> Result<(), RecommendationPolicyError> {
        if self.preselect_after_days > MAX_RECOMMENDATION_AGE_DAYS {
            return Err(RecommendationPolicyError {
                supplied: self.preselect_after_days,
            });
        }
        Ok(())
    }
}

/// An invalid recommendation age threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendationPolicyError {
    supplied: u16,
}

impl fmt::Display for RecommendationPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "recommendation preselect_after_days must be in 0..={MAX_RECOMMENDATION_AGE_DAYS}, got {}",
            self.supplied
        )
    }
}

impl Error for RecommendationPolicyError {}

/// A non-sensitive key identifying a matched rule within a local analysis report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuleKey {
    pub rule_pack_id: String,
    pub rule_id: String,
}

/// The rule facts that informed one candidate's recommendation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleEvidence {
    pub key: RuleKey,
    pub label: String,
    pub category: String,
    pub confidence: Confidence,
    pub default_selected: bool,
    pub trust: RuleTrust,
    pub match_role: RuleMatchRole,
    pub reason: String,
    pub risk_note: String,
}

/// How matching rules were resolved for recommendation purposes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleResolutionState {
    Single,
    Equivalent,
    UnresolvedConflict,
}

impl fmt::Display for RuleResolutionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Single => "single",
            Self::Equivalent => "equivalent",
            Self::UnresolvedConflict => "unresolved-conflict",
        })
    }
}

/// All rule matches, plus an explicit decision about whether they can safely decide selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleResolution {
    pub matched: Vec<RuleEvidence>,
    /// Broad fallback rules retained as evidence but excluded from the effective rule decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed: Vec<RuleKey>,
    pub primary: Option<RuleKey>,
    pub state: RuleResolutionState,
}

/// Relationship to candidates covering the same local filesystem tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OverlapEvidence {
    None,
    Primary { alternatives: Vec<CandidateId> },
    Suppressed { by: CandidateId },
}

/// The recommendation level. It is distinct from a user's later selection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RecommendationState {
    Preselected,
    Available,
    Review,
    Suppressed,
    Excluded,
}

impl fmt::Display for RecommendationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Preselected => "preselected",
            Self::Available => "available",
            Self::Review => "review",
            Self::Suppressed => "suppressed",
            Self::Excluded => "excluded",
        })
    }
}

/// Machine-readable reasons for a recommendation or non-selection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionCode {
    AgeThresholdMet,
    AgeGateDisabled,
    RecentObservedContent,
    RuleNotDefaultSelected,
    ConfidenceBelowHigh,
    UntrustedRule,
    TimestampMissing,
    TimestampAfterAsOf,
    ScanReportPartial,
    CandidateCoveragePartial,
    CandidateCoverageUnknown,
    UnresolvedRuleConflict,
    CoveredDescendantHasBlocker,
    OverlapSuppressed,
    ScanRoot,
    SafetyPolicyExcluded,
}

impl fmt::Display for DecisionCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AgeThresholdMet => "age-threshold-met",
            Self::AgeGateDisabled => "age-gate-disabled",
            Self::RecentObservedContent => "recent-observed-content",
            Self::RuleNotDefaultSelected => "rule-not-default-selected",
            Self::ConfidenceBelowHigh => "confidence-below-high",
            Self::UntrustedRule => "untrusted-rule",
            Self::TimestampMissing => "timestamp-missing",
            Self::TimestampAfterAsOf => "timestamp-after-as-of",
            Self::ScanReportPartial => "scan-report-partial",
            Self::CandidateCoveragePartial => "candidate-coverage-partial",
            Self::CandidateCoverageUnknown => "candidate-coverage-unknown",
            Self::UnresolvedRuleConflict => "unresolved-rule-conflict",
            Self::CoveredDescendantHasBlocker => "covered-descendant-has-blocker",
            Self::OverlapSuppressed => "overlap-suppressed",
            Self::ScanRoot => "scan-root",
            Self::SafetyPolicyExcluded => "safety-policy-excluded",
        })
    }
}

/// A deterministic decision whose codes explain both selection and non-selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecommendationDecision {
    pub state: RecommendationState,
    pub initial_selected: bool,
    pub codes: Vec<DecisionCode>,
}

/// Local-only evidence for a cleanup candidate. `local_path` must never be serialized into a
/// remote prompt or transport object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateEvidence {
    pub id: CandidateId,
    pub local_path: PathBuf,
    pub kind: EntryKind,
    pub size_bytes: u64,
    pub activity: ActivityEvidence,
    pub coverage: CandidateCoverage,
    pub rules: RuleResolution,
    pub overlap: OverlapEvidence,
    pub recommendation: RecommendationDecision,
    pub rollback_method: String,
}

/// Local scan facts attached to an analysis report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanEvidence {
    pub roots: Vec<PathBuf>,
    pub integrity: ReportIntegrity,
    pub issues: Vec<ScanIssue>,
}

/// Immutable analysis facts and their deterministic recommendations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisReport {
    pub schema_version: String,
    pub analysis_id: AnalysisId,
    pub as_of: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub policy: RecommendationPolicy,
    pub scan: ScanEvidence,
    pub candidates: Vec<CandidateEvidence>,
}

/// A user-owned set of candidate identifiers. It intentionally has no recommendation fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserSelection {
    pub candidate_ids: BTreeSet<CandidateId>,
}

impl UserSelection {
    /// Start from only candidates that the deterministic policy preselected.
    #[must_use]
    pub fn from_recommendations(report: &AnalysisReport) -> Self {
        Self {
            candidate_ids: report
                .candidates
                .iter()
                .filter(|candidate| candidate.recommendation.initial_selected)
                .map(|candidate| candidate.id.clone())
                .collect(),
        }
    }

    pub fn select(&mut self, candidate_id: CandidateId) {
        self.candidate_ids.insert(candidate_id);
    }

    pub fn deselect(&mut self, candidate_id: &CandidateId) {
        self.candidate_ids.remove(candidate_id);
    }
}

/// Build an immutable evidence report without mutating the user's later selection.
pub fn build_analysis_report(
    as_of: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    scan_roots: Vec<PathBuf>,
    entries: &[ScanEntry],
    issues: &[ScanIssue],
    policy: RecommendationPolicy,
) -> Result<AnalysisReport, RecommendationPolicyError> {
    build_analysis_report_inner(
        as_of,
        completed_at,
        scan_roots,
        entries,
        issues,
        policy,
        None,
    )
}

/// Build an immutable evidence report with the local cleanup safety policy applied before
/// recommendations are made. Candidates that safety excludes remain visible as evidence but are
/// never eligible for an initial or later plan selection.
pub fn build_analysis_report_with_safety_policy(
    as_of: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    scan_roots: Vec<PathBuf>,
    entries: &[ScanEntry],
    issues: &[ScanIssue],
    policy: RecommendationPolicy,
    safety_policy: &SafetyPolicy,
) -> Result<AnalysisReport, RecommendationPolicyError> {
    build_analysis_report_inner(
        as_of,
        completed_at,
        scan_roots,
        entries,
        issues,
        policy,
        Some(safety_policy),
    )
}

fn build_analysis_report_inner(
    as_of: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    scan_roots: Vec<PathBuf>,
    entries: &[ScanEntry],
    issues: &[ScanIssue],
    policy: RecommendationPolicy,
    safety_policy: Option<&SafetyPolicy>,
) -> Result<AnalysisReport, RecommendationPolicyError> {
    policy.validate()?;
    let integrity = ReportIntegrity::from_issues(issues);
    let recommendation_context = RecommendationContext {
        scan_roots: &scan_roots,
        integrity,
        policy: &policy,
        safety_policy,
    };
    let issue_scopes = IssueScopes::new(issues);
    let activity_by_path = activity_by_path(entries, as_of, &issue_scopes);
    let mut candidates = entries
        .iter()
        .filter(|entry| !entry.rule_hits.is_empty())
        .map(|entry| {
            let coverage = issue_scopes.coverage(&entry.path);
            let rules = resolve_rules(&entry.rule_hits);
            let activity = activity_by_path
                .get(&entry.path)
                .cloned()
                .unwrap_or_else(ActivityFacts::missing)
                .into_evidence(coverage, as_of);
            let recommendation = recommend_candidate(
                &entry.path,
                &rules,
                &activity,
                coverage,
                &recommendation_context,
            );
            CandidateEvidence {
                id: CandidateId::new_random(),
                local_path: entry.path.clone(),
                kind: entry.kind,
                size_bytes: entry.size_bytes,
                activity,
                coverage,
                rules,
                overlap: OverlapEvidence::None,
                recommendation,
                rollback_method: "system-trash+manifest".to_string(),
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.local_path.cmp(&right.local_path));
    resolve_overlaps(&mut candidates);

    Ok(AnalysisReport {
        schema_version: ANALYSIS_REPORT_SCHEMA_VERSION.to_string(),
        analysis_id: AnalysisId::new_random(),
        as_of,
        completed_at,
        policy,
        scan: ScanEvidence {
            roots: scan_roots,
            integrity,
            issues: issues.to_vec(),
        },
        candidates,
    })
}

#[derive(Debug, Clone)]
struct ActivityFacts {
    latest_observed_modified_at: Option<DateTime<Utc>>,
    own_latest: Option<DateTime<Utc>>,
    descendant_latest: Option<DateTime<Utc>>,
    has_missing_timestamp: bool,
    has_after_as_of: bool,
}

impl ActivityFacts {
    fn from_entry(entry: &ScanEntry, as_of: DateTime<Utc>) -> Self {
        let observed = entry.modified_at;
        Self {
            latest_observed_modified_at: observed,
            own_latest: observed,
            descendant_latest: None,
            has_missing_timestamp: observed.is_none(),
            has_after_as_of: observed.is_some_and(|time| time > as_of),
        }
    }

    fn missing() -> Self {
        Self {
            latest_observed_modified_at: None,
            own_latest: None,
            descendant_latest: None,
            has_missing_timestamp: true,
            has_after_as_of: false,
        }
    }

    fn absorb_descendant(&mut self, descendant: &Self) {
        self.descendant_latest = max_datetime(
            self.descendant_latest,
            descendant.latest_observed_modified_at,
        );
        self.latest_observed_modified_at = max_datetime(
            self.latest_observed_modified_at,
            descendant.latest_observed_modified_at,
        );
        self.has_missing_timestamp |= descendant.has_missing_timestamp;
        self.has_after_as_of |= descendant.has_after_as_of;
    }

    fn into_evidence(self, coverage: CandidateCoverage, as_of: DateTime<Utc>) -> ActivityEvidence {
        let source = match (self.own_latest, self.descendant_latest) {
            (None, None) => ActivitySource::Unavailable,
            (Some(_), None) => ActivitySource::Own,
            (None, Some(_)) => ActivitySource::Descendant,
            (Some(own), Some(descendant)) if own == descendant => ActivitySource::OwnAndDescendant,
            (Some(own), Some(descendant)) if own > descendant => ActivitySource::Own,
            (Some(_), Some(_)) => ActivitySource::Descendant,
        };
        let status = if self.has_after_as_of {
            ActivityStatus::AfterAsOf
        } else if coverage != CandidateCoverage::Complete {
            ActivityStatus::Partial
        } else if self.has_missing_timestamp {
            ActivityStatus::Missing
        } else {
            ActivityStatus::Complete
        };
        let effective_modified_at = (status == ActivityStatus::Complete)
            .then_some(self.latest_observed_modified_at)
            .flatten();
        ActivityEvidence {
            latest_observed_modified_at: self.latest_observed_modified_at,
            effective_modified_at,
            source,
            status,
            age_days: effective_modified_at.and_then(|modified_at| {
                let elapsed = as_of.signed_duration_since(modified_at);
                (elapsed >= chrono::Duration::zero())
                    .then(|| elapsed.num_days().clamp(0, i64::from(u32::MAX)) as u32)
            }),
        }
    }
}

fn activity_by_path(
    entries: &[ScanEntry],
    as_of: DateTime<Utc>,
    issue_scopes: &IssueScopes,
) -> HashMap<PathBuf, ActivityFacts> {
    let mut activity = entries
        .iter()
        .map(|entry| (entry.path.clone(), ActivityFacts::from_entry(entry, as_of)))
        .collect::<HashMap<_, _>>();
    let mut paths = activity.keys().cloned().collect::<Vec<_>>();
    paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));

    for path in paths {
        let Some(facts) = activity.get(&path).cloned() else {
            continue;
        };
        let Some(parent) = path.parent() else {
            continue;
        };
        if let Some(parent_facts) = activity.get_mut(parent) {
            parent_facts.absorb_descendant(&facts);
        }
    }

    for (path, facts) in &mut activity {
        if issue_scopes.unknown || issue_scopes.affected_ancestors.contains(path) {
            facts.has_missing_timestamp = true;
        }
    }
    activity
}

#[cfg(test)]
fn candidate_coverage(path: &Path, issues: &[ScanIssue]) -> CandidateCoverage {
    IssueScopes::new(issues).coverage(path)
}

struct IssueScopes {
    unknown: bool,
    affected_ancestors: HashSet<PathBuf>,
}

impl IssueScopes {
    fn new(issues: &[ScanIssue]) -> Self {
        let mut affected_ancestors = HashSet::new();
        for path in issues.iter().filter_map(|issue| issue.path.as_deref()) {
            affected_ancestors.extend(
                path.ancestors()
                    .filter(|ancestor| !ancestor.as_os_str().is_empty())
                    .map(Path::to_path_buf),
            );
        }
        Self {
            unknown: issue_with_unknown_scope(issues),
            affected_ancestors,
        }
    }

    fn coverage(&self, path: &Path) -> CandidateCoverage {
        if self.unknown {
            CandidateCoverage::Unknown
        } else if self.affected_ancestors.contains(path) {
            CandidateCoverage::Partial
        } else {
            CandidateCoverage::Complete
        }
    }
}

fn issue_with_unknown_scope(issues: &[ScanIssue]) -> bool {
    issues.iter().any(|issue| issue.path.is_none())
}

pub(crate) fn resolve_rules(hits: &[RuleHit]) -> RuleResolution {
    let mut matched = hits.iter().map(rule_evidence).collect::<Vec<_>>();
    matched.sort_by(|left, right| left.key.cmp(&right.key));
    let has_trusted_primary = matched.iter().any(|rule| {
        rule.match_role == RuleMatchRole::Primary && rule.trust != RuleTrust::Untrusted
    });
    let shadowed = if has_trusted_primary {
        matched
            .iter()
            .filter(|rule| rule.match_role == RuleMatchRole::Fallback)
            .map(|rule| rule.key.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let effective = matched
        .iter()
        .filter(|rule| !(has_trusted_primary && rule.match_role == RuleMatchRole::Fallback))
        .collect::<Vec<_>>();
    let state = if effective.len() == 1 {
        RuleResolutionState::Single
    } else if !effective.is_empty()
        && effective
            .windows(2)
            .all(|pair| same_safety_semantics(pair[0], pair[1]))
    {
        RuleResolutionState::Equivalent
    } else {
        RuleResolutionState::UnresolvedConflict
    };
    let primary = match state {
        RuleResolutionState::Single | RuleResolutionState::Equivalent => {
            effective.first().map(|rule| rule.key.clone())
        }
        RuleResolutionState::UnresolvedConflict => None,
    };
    RuleResolution {
        matched,
        shadowed,
        primary,
        state,
    }
}

fn rule_evidence(hit: &RuleHit) -> RuleEvidence {
    RuleEvidence {
        key: RuleKey {
            rule_pack_id: hit.rule_pack_id.clone(),
            rule_id: hit.rule_id.clone(),
        },
        label: hit.label.clone(),
        category: hit.category.clone(),
        confidence: hit.confidence,
        default_selected: hit.default_selected,
        trust: hit.trust,
        match_role: hit.match_role,
        reason: hit.reason.clone(),
        risk_note: hit.risk_note.clone(),
    }
}

fn same_safety_semantics(left: &RuleEvidence, right: &RuleEvidence) -> bool {
    left.category == right.category
        && left.confidence == right.confidence
        && left.default_selected == right.default_selected
        && left.trust == right.trust
        && left.match_role == right.match_role
        && left.reason == right.reason
        && left.risk_note == right.risk_note
}

struct RecommendationContext<'a> {
    scan_roots: &'a [PathBuf],
    integrity: ReportIntegrity,
    policy: &'a RecommendationPolicy,
    safety_policy: Option<&'a SafetyPolicy>,
}

fn recommend_candidate(
    path: &Path,
    rules: &RuleResolution,
    activity: &ActivityEvidence,
    coverage: CandidateCoverage,
    context: &RecommendationContext<'_>,
) -> RecommendationDecision {
    let mut codes = BTreeSet::new();
    if context.scan_roots.iter().any(|root| root == path) {
        codes.insert(DecisionCode::ScanRoot);
        return decision(RecommendationState::Excluded, codes);
    }
    if context
        .safety_policy
        .is_some_and(|safety| !safety.allows_candidate(path))
    {
        codes.insert(DecisionCode::SafetyPolicyExcluded);
        return decision(RecommendationState::Excluded, codes);
    }

    if rules.state == RuleResolutionState::UnresolvedConflict {
        codes.insert(DecisionCode::UnresolvedRuleConflict);
    }
    let primary = rules
        .primary
        .as_ref()
        .and_then(|key| rules.matched.iter().find(|rule| &rule.key == key));
    match primary {
        Some(rule) => {
            if !rule.default_selected {
                codes.insert(DecisionCode::RuleNotDefaultSelected);
            }
            if rule.confidence != Confidence::High {
                codes.insert(DecisionCode::ConfidenceBelowHigh);
            }
            if rule.trust == RuleTrust::Untrusted {
                codes.insert(DecisionCode::UntrustedRule);
            }
        }
        None => {
            codes.insert(DecisionCode::UnresolvedRuleConflict);
        }
    }
    match coverage {
        CandidateCoverage::Complete => {}
        CandidateCoverage::Partial => {
            codes.insert(DecisionCode::CandidateCoveragePartial);
        }
        CandidateCoverage::Unknown => {
            codes.insert(DecisionCode::CandidateCoverageUnknown);
        }
    }
    if context.integrity == ReportIntegrity::Partial && context.policy.strict_report_integrity {
        codes.insert(DecisionCode::ScanReportPartial);
    }
    match activity.status {
        ActivityStatus::Complete => {
            if context.policy.preselect_after_days == 0 {
                codes.insert(DecisionCode::AgeGateDisabled);
            } else if activity
                .age_days
                .is_some_and(|age| age >= u32::from(context.policy.preselect_after_days))
            {
                codes.insert(DecisionCode::AgeThresholdMet);
            } else {
                codes.insert(DecisionCode::RecentObservedContent);
            }
        }
        ActivityStatus::Missing => {
            codes.insert(DecisionCode::TimestampMissing);
        }
        ActivityStatus::Partial => {
            codes.insert(DecisionCode::TimestampMissing);
        }
        ActivityStatus::AfterAsOf => {
            codes.insert(DecisionCode::TimestampAfterAsOf);
        }
    }

    let is_high_trusted_default = primary.is_some_and(|rule| {
        rule.default_selected
            && rule.confidence == Confidence::High
            && rule.trust != RuleTrust::Untrusted
    });
    let age_gate_satisfied =
        context.policy.preselect_after_days == 0 || codes.contains(&DecisionCode::AgeThresholdMet);
    let blockers = [
        DecisionCode::UnresolvedRuleConflict,
        DecisionCode::ConfidenceBelowHigh,
        DecisionCode::UntrustedRule,
        DecisionCode::TimestampMissing,
        DecisionCode::TimestampAfterAsOf,
        DecisionCode::ScanReportPartial,
        DecisionCode::CandidateCoveragePartial,
        DecisionCode::CandidateCoverageUnknown,
    ];
    if is_high_trusted_default
        && age_gate_satisfied
        && blockers.iter().all(|blocker| !codes.contains(blocker))
    {
        return decision(RecommendationState::Preselected, codes);
    }
    if blockers.iter().any(|blocker| codes.contains(blocker)) {
        return decision(RecommendationState::Review, codes);
    }
    decision(RecommendationState::Available, codes)
}

fn decision(state: RecommendationState, codes: BTreeSet<DecisionCode>) -> RecommendationDecision {
    RecommendationDecision {
        state,
        initial_selected: state == RecommendationState::Preselected,
        codes: codes.into_iter().collect(),
    }
}

fn resolve_overlaps(candidates: &mut [CandidateEvidence]) {
    // Candidates are path-sorted by the caller. Every connected overlap cluster therefore starts
    // with its shallowest candidate and occupies one contiguous descendant range. Walking those
    // ranges avoids comparing every candidate with every other candidate.
    let mut cluster_start = 0;
    while cluster_start < candidates.len() {
        let mut cluster_end = cluster_start + 1;
        while cluster_end < candidates.len()
            && candidates[cluster_end]
                .local_path
                .starts_with(&candidates[cluster_start].local_path)
            && candidates[cluster_end].local_path != candidates[cluster_start].local_path
        {
            cluster_end += 1;
        }
        let cluster = (cluster_start..cluster_end).collect::<Vec<_>>();
        if cluster.len() == 1 {
            cluster_start = cluster_end;
            continue;
        }
        let primary_index = choose_primary(&cluster, candidates);
        let alternative_ids = cluster
            .iter()
            .copied()
            .filter(|index| *index != primary_index)
            .map(|index| candidates[index].id.clone())
            .collect::<Vec<_>>();
        let descendant_has_blocker = cluster.iter().copied().any(|index| {
            index != primary_index
                && candidates[index]
                    .local_path
                    .starts_with(&candidates[primary_index].local_path)
                && candidates[index].recommendation.state != RecommendationState::Excluded
                && recommendation_has_preselection_blocker(&candidates[index].recommendation)
        });
        candidates[primary_index].overlap = OverlapEvidence::Primary {
            alternatives: alternative_ids,
        };
        if descendant_has_blocker {
            candidates[primary_index]
                .recommendation
                .codes
                .push(DecisionCode::CoveredDescendantHasBlocker);
            candidates[primary_index].recommendation.codes.sort();
            candidates[primary_index].recommendation.codes.dedup();
            candidates[primary_index].recommendation.state = RecommendationState::Review;
            candidates[primary_index].recommendation.initial_selected = false;
        }
        for index in cluster {
            if index == primary_index {
                continue;
            }
            let primary_id = candidates[primary_index].id.clone();
            candidates[index].overlap = OverlapEvidence::Suppressed { by: primary_id };
            if candidates[index].recommendation.state == RecommendationState::Excluded {
                continue;
            }
            candidates[index]
                .recommendation
                .codes
                .push(DecisionCode::OverlapSuppressed);
            candidates[index].recommendation.codes.sort();
            candidates[index].recommendation.codes.dedup();
            candidates[index].recommendation.state = RecommendationState::Suppressed;
            candidates[index].recommendation.initial_selected = false;
        }
        cluster_start = cluster_end;
    }
}

fn choose_primary(indices: &[usize], candidates: &[CandidateEvidence]) -> usize {
    *indices
        .iter()
        .min_by(|left, right| {
            let left_candidate = &candidates[**left];
            let right_candidate = &candidates[**right];
            candidate_priority(left_candidate)
                .cmp(&candidate_priority(right_candidate))
                .then_with(|| {
                    left_candidate
                        .local_path
                        .components()
                        .count()
                        .cmp(&right_candidate.local_path.components().count())
                })
                .then_with(|| left_candidate.local_path.cmp(&right_candidate.local_path))
        })
        .expect("overlap clusters are never empty")
}

fn candidate_priority(candidate: &CandidateEvidence) -> (u8, u8, u8, u8) {
    let excluded = u8::from(candidate.recommendation.state == RecommendationState::Excluded);
    let preselected = u8::from(candidate.recommendation.initial_selected);
    let primary = candidate
        .rules
        .primary
        .as_ref()
        .and_then(|key| candidate.rules.matched.iter().find(|rule| &rule.key == key));
    let trust = primary.map_or(0, |rule| match rule.trust {
        RuleTrust::Untrusted => 0,
        RuleTrust::Trusted => 1,
        RuleTrust::Builtin => 2,
    });
    let confidence = primary.map_or(0, |rule| match rule.confidence {
        Confidence::Low => 0,
        Confidence::Medium => 1,
        Confidence::High => 2,
    });
    (
        excluded,
        u8::MAX - preselected,
        u8::MAX - trust,
        u8::MAX - confidence,
    )
}

fn recommendation_has_preselection_blocker(decision: &RecommendationDecision) -> bool {
    decision.codes.iter().any(|code| {
        matches!(
            code,
            DecisionCode::UnresolvedRuleConflict
                | DecisionCode::ConfidenceBelowHigh
                | DecisionCode::UntrustedRule
                | DecisionCode::TimestampMissing
                | DecisionCode::TimestampAfterAsOf
                | DecisionCode::ScanReportPartial
                | DecisionCode::CandidateCoveragePartial
                | DecisionCode::CandidateCoverageUnknown
        )
    })
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

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn hit(confidence: Confidence, default_selected: bool, trust: RuleTrust) -> RuleHit {
        RuleHit {
            rule_pack_id: "builtin-dev".to_string(),
            rule_id: "cache".to_string(),
            label: "Cache".to_string(),
            category: "developer-cache".to_string(),
            confidence,
            reason: "rebuildable".to_string(),
            risk_note: "rebuild".to_string(),
            default_selected,
            trust,
            match_role: RuleMatchRole::Primary,
        }
    }

    fn entry(path: &str, modified_at: Option<DateTime<Utc>>, rule_hits: Vec<RuleHit>) -> ScanEntry {
        ScanEntry {
            path: PathBuf::from(path),
            kind: EntryKind::Directory,
            size_bytes: 1,
            modified_at,
            rule_hits,
        }
    }

    #[test]
    fn policy_rejects_ages_larger_than_ten_years() {
        assert!(RecommendationPolicy::new(MAX_RECOMMENDATION_AGE_DAYS).is_ok());
        assert!(RecommendationPolicy::new(MAX_RECOMMENDATION_AGE_DAYS + 1).is_err());
    }

    #[test]
    fn report_integrity_ignores_intentional_exclusions() {
        let ignored = ScanIssue {
            code: ScanIssueCode::IgnoredByConfig,
            path: Some(PathBuf::from("/repo/.git")),
        };
        assert_eq!(
            ReportIntegrity::from_issues(&[ignored]),
            ReportIntegrity::Complete
        );
    }

    #[test]
    fn unknown_issue_scope_makes_candidate_coverage_unknown() {
        assert_eq!(
            candidate_coverage(
                Path::new("/repo/cache"),
                &[ScanIssue {
                    code: ScanIssueCode::TraversalError,
                    path: None,
                }]
            ),
            CandidateCoverage::Unknown
        );
    }

    #[test]
    fn incomplete_descendant_blocks_a_parent_recommendation() {
        let as_of = Utc::now();
        let entries = vec![
            entry(
                "/repo/cache",
                Some(as_of - Duration::days(100)),
                vec![hit(Confidence::High, true, RuleTrust::Builtin)],
            ),
            entry(
                "/repo/cache/child",
                Some(as_of - Duration::days(100)),
                vec![hit(Confidence::High, true, RuleTrust::Builtin)],
            ),
        ];
        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &entries,
            &[ScanIssue {
                code: ScanIssueCode::MetadataUnavailable,
                path: Some(PathBuf::from("/repo/cache/child/lost")),
            }],
            RecommendationPolicy::default(),
        )
        .expect("valid policy");
        let parent = report
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache"))
            .expect("parent candidate");
        assert_eq!(parent.recommendation.state, RecommendationState::Review);
        assert!(
            parent
                .recommendation
                .codes
                .contains(&DecisionCode::CandidateCoveragePartial)
        );
    }

    #[test]
    fn age_boundaries_are_calculated_from_one_as_of_instant() {
        let as_of = Utc::now();
        for (days, expected_state, expected_code) in [
            (
                89,
                RecommendationState::Available,
                DecisionCode::RecentObservedContent,
            ),
            (
                90,
                RecommendationState::Preselected,
                DecisionCode::AgeThresholdMet,
            ),
            (
                91,
                RecommendationState::Preselected,
                DecisionCode::AgeThresholdMet,
            ),
        ] {
            let report = build_analysis_report(
                as_of,
                as_of,
                vec![PathBuf::from("/repo")],
                &[entry(
                    "/repo/cache",
                    Some(as_of - Duration::days(days)),
                    vec![hit(Confidence::High, true, RuleTrust::Builtin)],
                )],
                &[],
                RecommendationPolicy::default(),
            )
            .expect("valid policy");
            let candidate = &report.candidates[0];
            assert_eq!(candidate.activity.age_days, Some(days as u32));
            assert_eq!(candidate.recommendation.state, expected_state);
            assert!(candidate.recommendation.codes.contains(&expected_code));
        }
    }

    #[test]
    fn newer_descendant_makes_a_directory_recent() {
        let as_of = Utc::now();
        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &[
                entry(
                    "/repo/cache",
                    Some(as_of - Duration::days(100)),
                    vec![hit(Confidence::High, true, RuleTrust::Builtin)],
                ),
                ScanEntry {
                    path: PathBuf::from("/repo/cache/new-file"),
                    kind: EntryKind::File,
                    size_bytes: 1,
                    modified_at: Some(as_of - Duration::days(1)),
                    rule_hits: vec![],
                },
            ],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid policy");
        let candidate = &report.candidates[0];
        assert_eq!(candidate.activity.source, ActivitySource::Descendant);
        assert_eq!(candidate.activity.age_days, Some(1));
        assert_eq!(
            candidate.recommendation.state,
            RecommendationState::Available
        );
        assert!(
            candidate
                .recommendation
                .codes
                .contains(&DecisionCode::RecentObservedContent)
        );
    }

    #[test]
    fn future_timestamp_and_rule_conflict_require_review() {
        let as_of = Utc::now();
        let mut conflicting_hit = hit(Confidence::Medium, false, RuleTrust::Builtin);
        conflicting_hit.rule_id = "other-cache".to_string();
        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &[entry(
                "/repo/cache",
                Some(as_of + Duration::seconds(1)),
                vec![
                    conflicting_hit,
                    hit(Confidence::High, true, RuleTrust::Builtin),
                ],
            )],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid policy");
        let candidate = &report.candidates[0];
        assert_eq!(
            candidate.rules.state,
            RuleResolutionState::UnresolvedConflict
        );
        assert_eq!(candidate.recommendation.state, RecommendationState::Review);
        assert!(
            candidate
                .recommendation
                .codes
                .contains(&DecisionCode::TimestampAfterAsOf)
        );
        assert!(
            candidate
                .recommendation
                .codes
                .contains(&DecisionCode::UnresolvedRuleConflict)
        );
    }

    #[test]
    fn differing_rule_risk_notes_are_an_unresolved_conflict() {
        let safe = hit(Confidence::High, true, RuleTrust::Builtin);
        let mut inspect = safe.clone();
        inspect.rule_id = "cache-inspect".to_string();
        inspect.risk_note = "inspect before removal".to_string();

        let resolution = resolve_rules(&[safe, inspect]);

        assert_eq!(resolution.state, RuleResolutionState::UnresolvedConflict);
        assert!(resolution.primary.is_none());
    }

    #[test]
    fn trusted_primary_shadows_broad_fallback_but_keeps_its_evidence() {
        let mut primary = hit(Confidence::High, true, RuleTrust::Builtin);
        primary.rule_id = "specific-cache".to_string();
        let mut fallback = hit(Confidence::Medium, false, RuleTrust::Builtin);
        fallback.rule_id = "broad-cache".to_string();
        fallback.match_role = RuleMatchRole::Fallback;

        let resolution = resolve_rules(&[fallback, primary.clone()]);

        assert_eq!(resolution.state, RuleResolutionState::Single);
        assert_eq!(
            resolution.primary,
            Some(RuleKey {
                rule_pack_id: primary.rule_pack_id,
                rule_id: primary.rule_id,
            })
        );
        assert_eq!(resolution.matched.len(), 2);
        assert_eq!(resolution.shadowed.len(), 1);
    }

    #[test]
    fn untrusted_primary_cannot_shadow_a_builtin_fallback() {
        let mut untrusted = hit(Confidence::High, true, RuleTrust::Untrusted);
        untrusted.rule_id = "untrusted-specific".to_string();
        let mut fallback = hit(Confidence::Medium, false, RuleTrust::Builtin);
        fallback.rule_id = "broad-cache".to_string();
        fallback.match_role = RuleMatchRole::Fallback;

        let resolution = resolve_rules(&[fallback, untrusted]);

        assert_eq!(resolution.state, RuleResolutionState::UnresolvedConflict);
        assert!(resolution.primary.is_none());
        assert!(resolution.shadowed.is_empty());
    }

    #[test]
    fn missing_timestamps_require_review_but_zero_disables_only_the_age_gate() {
        let as_of = Utc::now();
        let missing_time_report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &[entry(
                "/repo/cache",
                None,
                vec![hit(Confidence::High, true, RuleTrust::Builtin)],
            )],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid policy");
        assert_eq!(
            missing_time_report.candidates[0].recommendation.state,
            RecommendationState::Review
        );
        assert!(
            missing_time_report.candidates[0]
                .recommendation
                .codes
                .contains(&DecisionCode::TimestampMissing)
        );

        let disabled_age_gate = RecommendationPolicy::new(0).expect("disabled age gate");
        let zero_threshold_report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &[entry(
                "/repo/cache",
                Some(as_of - Duration::days(1)),
                vec![hit(Confidence::High, true, RuleTrust::Builtin)],
            )],
            &[],
            disabled_age_gate,
        )
        .expect("valid policy");
        assert_eq!(
            zero_threshold_report.candidates[0].recommendation.state,
            RecommendationState::Preselected
        );
        assert!(
            zero_threshold_report.candidates[0]
                .recommendation
                .codes
                .contains(&DecisionCode::AgeGateDisabled)
        );
    }

    #[test]
    fn overlap_keeps_the_suppressed_candidate_as_evidence() {
        let as_of = Utc::now();
        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &[
                entry(
                    "/repo/cache",
                    Some(as_of - Duration::days(100)),
                    vec![hit(Confidence::High, true, RuleTrust::Builtin)],
                ),
                entry(
                    "/repo/cache/child",
                    Some(as_of - Duration::days(100)),
                    vec![hit(Confidence::High, true, RuleTrust::Builtin)],
                ),
            ],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid policy");
        assert_eq!(report.candidates.len(), 2);
        let parent = report
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache"))
            .expect("parent candidate");
        let child = report
            .candidates
            .iter()
            .find(|candidate| candidate.local_path == Path::new("/repo/cache/child"))
            .expect("child candidate");
        assert!(matches!(parent.overlap, OverlapEvidence::Primary { .. }));
        assert!(matches!(child.overlap, OverlapEvidence::Suppressed { .. }));
        assert_eq!(child.recommendation.state, RecommendationState::Suppressed);
        assert!(
            child
                .recommendation
                .codes
                .contains(&DecisionCode::OverlapSuppressed)
        );
    }

    #[test]
    fn user_selection_does_not_change_recommendation_facts() {
        let as_of = Utc::now();
        let report = build_analysis_report(
            as_of,
            as_of,
            vec![PathBuf::from("/repo")],
            &[entry(
                "/repo/cache",
                Some(as_of - Duration::days(100)),
                vec![hit(Confidence::High, true, RuleTrust::Builtin)],
            )],
            &[],
            RecommendationPolicy::default(),
        )
        .expect("valid policy");
        let candidate = &report.candidates[0];
        let decision = candidate.recommendation.clone();
        let mut selection = UserSelection::from_recommendations(&report);
        selection.deselect(&candidate.id);

        assert!(selection.candidate_ids.is_empty());
        assert_eq!(report.candidates[0].recommendation, decision);
    }
}
