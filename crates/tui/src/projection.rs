use cleanr_core::{
    CleanupItem, CleanupPlan, RuleHit, RuleMatchRole, RuleResolutionState, RuleTrust, ScanEntry,
};
use cleanr_i18n::I18n;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const BUILTIN_CATEGORIES: [&str; 9] = [
    "developer-cache",
    "build-cache",
    "package-cache",
    "browser-cache",
    "application-cache",
    "temporary-files",
    "logs",
    "diagnostics",
    "downloads",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CategoryKey {
    Named(String),
    Multiple,
    Unknown,
}

impl CategoryKey {
    pub(crate) fn label(&self, i18n: &I18n, short: bool) -> String {
        let name = match self {
            Self::Named(name) if BUILTIN_CATEGORIES.contains(&name.as_str()) => {
                name.replace('-', "_")
            }
            Self::Named(name) => return name.clone(),
            Self::Multiple => "multiple".to_string(),
            Self::Unknown => "unknown".to_string(),
        };
        let suffix = if short { "_short" } else { "" };
        i18n.t(&format!("category_{name}{suffix}"))
    }

    pub(crate) fn order(&self) -> usize {
        match self {
            Self::Named(name) => BUILTIN_CATEGORIES
                .iter()
                .position(|builtin| *builtin == name)
                .unwrap_or(BUILTIN_CATEGORIES.len()),
            Self::Multiple => BUILTIN_CATEGORIES.len() + 1,
            Self::Unknown => BUILTIN_CATEGORIES.len() + 2,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CandidateCategory {
    pub(crate) key: CategoryKey,
    pub(crate) categories: Vec<String>,
    pub(crate) conflict: bool,
    pub(crate) tentative: bool,
}

impl CandidateCategory {
    fn new(categories: impl Iterator<Item = String>, conflict: bool, tentative: bool) -> Self {
        let categories = categories
            .filter(|category| !category.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let key = match categories.as_slice() {
            [] => CategoryKey::Unknown,
            [category] => CategoryKey::Named(category.clone()),
            _ => CategoryKey::Multiple,
        };
        Self {
            key,
            categories,
            conflict,
            tentative,
        }
    }

    pub(crate) fn from_plan_item(item: &CleanupItem) -> Self {
        if let Some(evidence) = &item.evidence {
            return Self::new(
                evidence
                    .matched_rules
                    .iter()
                    .filter(|rule| !evidence.shadowed_rules.contains(&rule.key))
                    .map(|rule| rule.category.clone()),
                evidence.rule_resolution_state == RuleResolutionState::UnresolvedConflict,
                false,
            );
        }
        Self::new(std::iter::once(item.category.clone()), false, false)
    }

    /// A failed or budget-limited scan can have rule hits without a usable plan. This display
    /// projection mirrors only rule shadowing and never grants cleanup or selection authority.
    pub(crate) fn from_rule_hits(hits: &[RuleHit]) -> Self {
        let has_trusted_primary = hits.iter().any(|hit| {
            hit.match_role == RuleMatchRole::Primary && hit.trust != RuleTrust::Untrusted
        });
        let effective = hits
            .iter()
            .filter(|hit| !(has_trusted_primary && hit.match_role == RuleMatchRole::Fallback))
            .collect::<Vec<_>>();
        let mut category = Self::new(
            effective.iter().map(|hit| hit.category.clone()),
            false,
            true,
        );
        // Only plan evidence authoritatively resolves same-category safety conflicts. Raw rows
        // can identify differing categories without duplicating the domain's risk semantics.
        category.conflict = category.key == CategoryKey::Multiple;
        category
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScanCandidateRow {
    pub(crate) source_index: usize,
    pub(crate) category: CandidateCategory,
    pub(crate) path: PathBuf,
    pub(crate) size_bytes: u64,
    pub(crate) search_text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CategorySummary {
    pub(crate) key: CategoryKey,
    pub(crate) count: usize,
    pub(crate) size_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum ScanSort {
    #[default]
    Plan,
    Size,
    Path,
}

impl ScanSort {
    pub(crate) const ALL: [Self; 3] = [Self::Plan, Self::Size, Self::Path];
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Plan => "scan_sort_plan",
            Self::Size => "scan_sort_size",
            Self::Path => "scan_sort_path",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanQuery {
    pub(crate) category: Option<CategoryKey>,
    pub(crate) text: String,
    pub(crate) only_selected: bool,
    pub(crate) sort: ScanSort,
}

#[derive(Debug, Default)]
pub(crate) struct ScanIndex {
    pub(crate) rows: Vec<ScanCandidateRow>,
    pub(crate) groups: Vec<CategorySummary>,
    pub(crate) total_size_bytes: u64,
    pub(crate) plan_order: Arc<Vec<usize>>,
    size_order: Arc<Vec<usize>>,
    path_order: Arc<Vec<usize>>,
}

pub(crate) fn prepare_scan_index(plan: Option<&CleanupPlan>, entries: &[ScanEntry]) -> ScanIndex {
    let row = |source_index, path: &PathBuf, size_bytes, category| ScanCandidateRow {
        source_index,
        category,
        path: path.clone(),
        size_bytes,
        search_text: path.to_string_lossy().replace('\\', "/").to_lowercase(),
    };
    let rows: Vec<_> = if let Some(plan) = plan {
        plan.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                row(
                    index,
                    &item.path,
                    item.size_bytes,
                    CandidateCategory::from_plan_item(item),
                )
            })
            .collect()
    } else {
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.rule_hits.is_empty())
            .map(|(index, entry)| {
                row(
                    index,
                    &entry.path,
                    entry.size_bytes,
                    CandidateCategory::from_rule_hits(&entry.rule_hits),
                )
            })
            .collect()
    };
    let mut categories: BTreeMap<CategoryKey, (usize, u64)> = BTreeMap::new();
    let mut total_size_bytes = 0u64;
    for row in &rows {
        let group = categories.entry(row.category.key.clone()).or_default();
        group.0 += 1;
        group.1 = group.1.saturating_add(row.size_bytes);
        total_size_bytes = total_size_bytes.saturating_add(row.size_bytes);
    }
    let mut groups: Vec<_> = categories
        .into_iter()
        .map(|(key, (count, size_bytes))| CategorySummary {
            key,
            count,
            size_bytes,
        })
        .collect();
    groups.sort_by(|left, right| {
        left.key
            .order()
            .cmp(&right.key.order())
            .then(left.key.cmp(&right.key))
    });
    let order: Vec<_> = (0..rows.len()).collect();
    let mut size_order = order.clone();
    size_order.sort_by(|a, b| {
        rows[*b]
            .size_bytes
            .cmp(&rows[*a].size_bytes)
            .then(rows[*a].path.cmp(&rows[*b].path))
    });
    let mut path_order = order.clone();
    path_order.sort_by(|a, b| rows[*a].path.cmp(&rows[*b].path));
    ScanIndex {
        rows,
        groups,
        total_size_bytes,
        plan_order: Arc::new(order),
        size_order: Arc::new(size_order),
        path_order: Arc::new(path_order),
    }
}

/// Display projection only: the cleanup plan remains the sole source of eligible candidates.
pub(crate) fn project_scan(
    index: &ScanIndex,
    query: &ScanQuery,
    selected: &[bool],
    cancelled: &AtomicBool,
) -> Option<Arc<Vec<usize>>> {
    let order = match query.sort {
        ScanSort::Plan => &index.plan_order,
        ScanSort::Size => &index.size_order,
        ScanSort::Path => &index.path_order,
    };
    if query.category.is_none() && query.text.is_empty() && !query.only_selected {
        return Some(Arc::clone(order));
    }
    let mut visible = Vec::new();
    for (position, row_index) in order.iter().copied().enumerate() {
        if position % 256 == 0 && cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let row = &index.rows[row_index];
        if query
            .category
            .as_ref()
            .is_none_or(|key| *key == row.category.key)
            && row.search_text.contains(&query.text)
            && (!query.only_selected || selected.get(row.source_index).copied().unwrap_or(false))
        {
            visible.push(row_index);
        }
    }
    Some(Arc::new(visible))
}
