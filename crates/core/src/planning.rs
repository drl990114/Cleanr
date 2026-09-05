use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};

use crate::{
    ANALYSIS_REPORT_SCHEMA_VERSION, AnalysisReport, CLEANUP_PLAN_SCHEMA_VERSION, CandidateEvidence,
    CleanupItem, CleanupItemEvidence, CleanupItemFingerprint, CleanupPlan, CleanupPlanBuildError,
    CleanupPlanSourceScan, Confidence, EntryKind, PlanSafety, PlanSummary, PlannedAction,
    RecommendationState, RuleHit, RuleTrust, RulesetVersion, SafetyPolicy, ScanEntry,
    UserSelection,
    evidence::resolve_rules,
    safety::{normalize_path, normalize_protected_paths},
};

/// Legacy entry-only builder for callers that already proved their scan is complete.
///
/// This API cannot observe a filesystem report's budget ledger. Never pass entries from a report
/// whose `budget_exceeded` collection is non-empty. Product workflows should build analysis with
/// `AnalysisScanContext` and then call [`build_cleanup_plan_from_analysis`] so the read-only budget
/// boundary is retained in the plan provenance.
#[must_use]
#[allow(deprecated)]
#[deprecated(
    since = "0.13.0",
    note = "use build_analysis_report_with_scan_context followed by build_cleanup_plan_from_analysis so scan integrity and provenance cannot be dropped"
)]
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

/// Legacy policy-aware entry-only builder with the same complete-scan precondition as
/// [`build_cleanup_plan`]. Budget-limited reports must use the analysis-backed builder instead.
#[must_use]
#[deprecated(
    since = "0.13.0",
    note = "use build_analysis_report_with_scan_context followed by build_cleanup_plan_from_analysis so scan integrity and provenance cannot be dropped"
)]
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
            let normalized_path = normalize_path(&entry.path);
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
                    .then(|| tree_fingerprints.get(entry.path.as_path()).cloned())
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
        None,
    )
}

/// Build a cleanup plan from a single immutable analysis report and its user-owned selection.
///
/// This is the product-facing plan builder: recommendation and overlap decisions come only from
/// `analysis`, while `selection` records the user's later choices. The local safety policy is
/// applied again defensively even though safety-aware analysis already records excluded paths.
pub fn build_cleanup_plan_from_analysis(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    entries: &[ScanEntry],
    analysis: &AnalysisReport,
    selection: &UserSelection,
    policy: &SafetyPolicy,
) -> Result<CleanupPlan, CleanupPlanBuildError> {
    crate::control::uninterrupted(build_cleanup_plan_from_analysis_cancellable(
        scan_roots,
        ruleset_versions,
        entries,
        analysis,
        selection,
        policy,
        &|| false,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn build_cleanup_plan_from_analysis_cancellable(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    entries: &[ScanEntry],
    analysis: &AnalysisReport,
    selection: &UserSelection,
    policy: &SafetyPolicy,
    cancelled: &dyn Fn() -> bool,
) -> Result<CleanupPlan, crate::WorkError<CleanupPlanBuildError>> {
    use crate::control::{WorkError, check_work};
    check_work(cancelled)?;
    if analysis.schema_version != ANALYSIS_REPORT_SCHEMA_VERSION {
        return Err(WorkError::Failed(
            CleanupPlanBuildError::UnsupportedAnalysisSchema {
                found: analysis.schema_version.clone(),
            },
        ));
    }
    if !analysis.scan.budget_exceeded.is_empty() {
        return Err(WorkError::Failed(CleanupPlanBuildError::ScanBudgetExceeded));
    }
    let mut selected_candidates = analysis
        .candidates
        .iter()
        .filter(|candidate| {
            selection.candidate_ids.contains(&candidate.id)
                && !matches!(
                    candidate.recommendation.state,
                    RecommendationState::Suppressed | RecommendationState::Excluded
                )
        })
        .collect::<Vec<_>>();
    selected_candidates.sort_by(|left, right| {
        left.local_path
            .components()
            .count()
            .cmp(&right.local_path.components().count())
            .then_with(|| left.local_path.cmp(&right.local_path))
    });
    let mut selected_by_path = HashMap::<&Path, &CandidateEvidence>::new();
    check_work(cancelled)?;
    for (index, candidate) in selected_candidates.into_iter().enumerate() {
        if index % 256 == 0 {
            check_work(cancelled)?;
        }
        if let Some(ancestor) = candidate
            .local_path
            .ancestors()
            .find_map(|path| selected_by_path.get(path).copied())
        {
            return Err(WorkError::Failed(
                CleanupPlanBuildError::OverlappingSelection {
                    left: ancestor.local_path.clone(),
                    right: candidate.local_path.clone(),
                },
            ));
        }
        selected_by_path.insert(candidate.local_path.as_path(), candidate);
    }
    let normalized_scan_roots = normalize_protected_paths(scan_roots.clone());
    check_work(cancelled)?;
    let tree_fingerprints = tree_fingerprints(entries);
    check_work(cancelled)?;
    let entries_by_path = entries
        .iter()
        .map(|entry| (entry.path.as_path(), entry))
        .collect::<HashMap<_, _>>();
    let items = analysis
        .candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            if index % 256 == 0 && cancelled() {
                return Some(Err(WorkError::Cancelled));
            }
            if matches!(
                candidate.recommendation.state,
                RecommendationState::Suppressed | RecommendationState::Excluded
            ) {
                return None;
            }
            let selected = selection.candidate_ids.contains(&candidate.id);
            // The normal cleanup flow contains only candidates whose complete activity evidence
            // satisfies the configured inactivity threshold. An exact, explicit user selection
            // remains an escape hatch for a recent or timestamp-less review candidate.
            if !selected
                && analysis.policy.filters_candidate_projection_by_inactivity()
                && !analysis
                    .policy
                    .activity_meets_inactivity_threshold(&candidate.activity)
            {
                return None;
            }
            let normalized_path = normalize_path(&candidate.local_path);
            let is_scan_root = normalized_scan_roots
                .iter()
                .any(|root| root == &normalized_path);
            if (is_scan_root && !candidate.known_global_location)
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
            Some(Ok(CleanupItem {
                path: candidate.local_path.clone(),
                kind: candidate.kind,
                size_bytes: candidate.size_bytes,
                modified_at,
                tree_fingerprint: (candidate.kind == EntryKind::Directory)
                    .then(|| {
                        tree_fingerprints
                            .get(candidate.local_path.as_path())
                            .cloned()
                    })
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
                    known_global_location: candidate.known_global_location,
                    runtime_guards: candidate.runtime_guards.clone(),
                }),
                selected,
                planned_action: PlannedAction::Trash,
                rollback_method: candidate.rollback_method.clone(),
            }))
        })
        .collect::<Result<Vec<_>, WorkError<CleanupPlanBuildError>>>()?;

    let source_scan = CleanupPlanSourceScan {
        analysis_id: analysis.analysis_id.clone(),
        integrity: analysis.scan.integrity,
        budget_exceeded: analysis.scan.budget_exceeded.clone(),
        recommendation_policy: Some(analysis.policy.clone()),
        scope: None,
    };
    check_work(cancelled)?;
    let plan = finish_cleanup_plan(
        scan_roots,
        ruleset_versions,
        remove_overlapping_items(items),
        policy,
        Some(source_scan),
    );
    check_work(cancelled)?;
    Ok(plan)
}

fn finish_cleanup_plan(
    scan_roots: Vec<PathBuf>,
    ruleset_versions: Vec<RulesetVersion>,
    items: Vec<CleanupItem>,
    policy: &SafetyPolicy,
    source_scan: Option<CleanupPlanSourceScan>,
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
        source_scan,
        summary,
        items,
        safety: PlanSafety {
            requires_confirmation: policy.requires_confirmation(),
            protected_paths: policy.protected_paths().to_vec(),
            protected_subtrees: policy.protected_subtrees().to_vec(),
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

/// Merge a set of unique paths from leaves towards their immediate parents without cloning paths
/// or sorting by depth. Callers must use an order-independent merge operation.
pub(crate) fn merge_path_forest(
    paths: &[&Path],
    can_merge_into: impl Fn(usize) -> bool,
    mut merge: impl FnMut(usize, usize),
) {
    let parent_indices = {
        let by_path = paths
            .iter()
            .enumerate()
            .map(|(idx, path)| (*path, idx))
            .collect::<HashMap<_, _>>();
        paths
            .iter()
            .map(|path| {
                path.parent()
                    .and_then(|parent| by_path.get(parent).copied())
                    .filter(|parent_idx| can_merge_into(*parent_idx))
            })
            .collect::<Vec<_>>()
    };

    let mut remaining_children = vec![0usize; paths.len()];
    for parent_idx in parent_indices.iter().flatten() {
        remaining_children[*parent_idx] = remaining_children[*parent_idx].saturating_add(1);
    }
    let mut ready = remaining_children
        .iter()
        .enumerate()
        .filter_map(|(idx, children)| (*children == 0).then_some(idx))
        .collect::<Vec<_>>();

    let mut processed = 0usize;
    while let Some(child_idx) = ready.pop() {
        processed += 1;
        let Some(parent_idx) = parent_indices[child_idx] else {
            continue;
        };
        merge(parent_idx, child_idx);
        remaining_children[parent_idx] -= 1;
        if remaining_children[parent_idx] == 0 {
            ready.push(parent_idx);
        }
    }
    debug_assert_eq!(processed, paths.len());
}

#[derive(Clone)]
pub(crate) struct TreeFingerprintFacts {
    pub(crate) descendants: usize,
    pub(crate) latest_modified_at: Option<DateTime<Utc>>,
}

impl TreeFingerprintFacts {
    fn absorb_leaf(&mut self, modified_at: Option<DateTime<Utc>>) {
        self.descendants = self.descendants.saturating_add(1);
        self.latest_modified_at = max_datetime(self.latest_modified_at, modified_at);
    }

    pub(crate) fn absorb_descendant(&mut self, descendant: &Self) {
        self.descendants = self
            .descendants
            .saturating_add(1)
            .saturating_add(descendant.descendants);
        self.latest_modified_at =
            max_datetime(self.latest_modified_at, descendant.latest_modified_at);
    }
}

pub(crate) fn tree_fingerprints(entries: &[ScanEntry]) -> HashMap<&Path, CleanupItemFingerprint> {
    // Only directories need parent links. Indexing every file made a shallow, file-heavy tree pay
    // a large hash-table cost even though each file can be folded directly into its parent.
    let directories = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .collect::<Vec<_>>();
    let paths = directories
        .iter()
        .map(|entry| entry.path.as_path())
        .collect::<Vec<_>>();
    let directory_by_path = paths
        .iter()
        .enumerate()
        .map(|(idx, path)| (*path, idx))
        .collect::<HashMap<_, _>>();
    let mut facts = directories
        .iter()
        .map(|entry| TreeFingerprintFacts {
            descendants: 0,
            latest_modified_at: entry.modified_at,
        })
        .collect::<Vec<_>>();

    for entry in entries
        .iter()
        .filter(|entry| entry.kind != EntryKind::Directory)
    {
        let Some(parent_idx) = entry
            .path
            .parent()
            .and_then(|parent| directory_by_path.get(parent).copied())
        else {
            continue;
        };
        facts[parent_idx].absorb_leaf(entry.modified_at);
    }

    merge_path_forest(
        &paths,
        |_| true,
        |parent_idx, child_idx| {
            let child = facts[child_idx].clone();
            facts[parent_idx].absorb_descendant(&child);
        },
    );

    directories
        .iter()
        .zip(facts)
        .map(|(entry, facts)| {
            (
                entry.path.as_path(),
                CleanupItemFingerprint {
                    descendants: facts.descendants,
                    total_size_bytes: entry.size_bytes,
                    latest_modified_at: facts.latest_modified_at,
                },
            )
        })
        .collect()
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
    let resolution = resolve_rules(&entry.rule_hits);
    let primary = resolution.primary?;
    entry
        .rule_hits
        .iter()
        .find(|hit| hit.rule_pack_id == primary.rule_pack_id && hit.rule_id == primary.rule_id)
}
