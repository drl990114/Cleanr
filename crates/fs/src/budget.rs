use std::{mem::size_of, path::Path, time::Instant};

use cleanr_core::{ScanBudgetExceeded, ScanBudgetLimits, ScanEntry, ScanIssue};

use crate::{ScanError, ScanReport, scanner::PendingHardlink};

/// Per-retained-entry conservative allocation estimate. This is intentionally not RSS: it covers
/// the persistent `ScanEntry` and encoded path bytes plus O(N) aggregation structures (path-map
/// node allowance, parent index, child count, and ready queue slot). All arithmetic saturates.
fn estimated_entry_memory_bytes(path: &Path) -> u64 {
    const MAP_NODE_ALLOWANCE: usize = 64;
    // Double the retained element costs to conservatively cover Vec spare capacity. The pending
    // hardlink owner also retains a cloned path until final accounting on filesystems that expose
    // identities; charging it for every entry intentionally overestimates mixed trees.
    let fixed = size_of::<ScanEntry>()
        .saturating_mul(2)
        .saturating_add(size_of::<PendingHardlink>())
        .saturating_add(size_of::<Option<usize>>())
        .saturating_add(size_of::<usize>())
        .saturating_add(size_of::<usize>())
        .saturating_add(MAP_NODE_ALLOWANCE);
    u64::try_from(fixed).unwrap_or(u64::MAX).saturating_add(
        u64::try_from(path.as_os_str().as_encoded_bytes().len())
            .unwrap_or(u64::MAX)
            .saturating_mul(2),
    )
}

pub(super) struct ScanBudgetTracker {
    pub(super) limits: ScanBudgetLimits,
    pub(super) started_at: Instant,
    pub(super) observed_entries: u64,
    pub(super) estimated_memory_bytes: u64,
    pub(super) issue_details_observed: u64,
    pub(super) error_count: usize,
    pub(super) stopped: bool,
    pub(super) exceeded: Vec<ScanBudgetExceeded>,
}

impl ScanBudgetTracker {
    pub(super) fn new(limits: ScanBudgetLimits, started_at: Instant) -> Self {
        Self {
            limits,
            started_at,
            observed_entries: 0,
            estimated_memory_bytes: 0,
            issue_details_observed: 0,
            error_count: 0,
            stopped: false,
            exceeded: Vec::new(),
        }
    }

    pub(super) fn check_elapsed(&mut self) -> bool {
        if self.limits.elapsed_millis == 0 {
            return self.stopped;
        }
        let observed = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        if observed >= self.limits.elapsed_millis {
            self.upsert_elapsed(observed);
            self.stopped = true;
        }
        self.stopped
    }

    pub(super) fn retain_entry(&mut self, path: &Path) -> bool {
        self.observed_entries = self.observed_entries.saturating_add(1);
        if self.limits.entries != 0 && self.observed_entries > self.limits.entries {
            self.exceeded.push(ScanBudgetExceeded::EntryCount {
                limit: self.limits.entries,
                observed: self.observed_entries,
            });
            self.stopped = true;
            return false;
        }
        let projected = self
            .estimated_memory_bytes
            .saturating_add(estimated_entry_memory_bytes(path));
        if self.limits.estimated_memory_bytes != 0 && projected > self.limits.estimated_memory_bytes
        {
            self.exceeded.push(ScanBudgetExceeded::EstimatedMemory {
                limit_bytes: self.limits.estimated_memory_bytes,
                observed_bytes: projected,
            });
            self.stopped = true;
            return false;
        }
        self.estimated_memory_bytes = projected;
        true
    }

    pub(super) fn record_detail(&mut self, issue: &ScanIssue, error: Option<&ScanError>) -> bool {
        self.issue_details_observed = self.issue_details_observed.saturating_add(1);
        if error.is_some() {
            self.error_count = self.error_count.saturating_add(1);
        }
        if self.limits.issue_details != 0 && self.issue_details_observed > self.limits.issue_details
        {
            return false;
        }
        let path_bytes = |path: Option<&Path>| {
            path.map_or(0, |path| {
                u64::try_from(path.as_os_str().as_encoded_bytes().len()).unwrap_or(u64::MAX)
            })
        };
        let mut detail_bytes = u64::try_from(size_of::<ScanIssue>()).unwrap_or(u64::MAX);
        detail_bytes = detail_bytes.saturating_add(path_bytes(issue.path.as_deref()));
        if let Some(error) = error {
            detail_bytes = detail_bytes
                .saturating_add(u64::try_from(size_of::<ScanError>()).unwrap_or(u64::MAX))
                .saturating_add(path_bytes(error.path.as_deref()))
                .saturating_add(u64::try_from(error.message.len()).unwrap_or(u64::MAX));
        }
        let projected = self.estimated_memory_bytes.saturating_add(detail_bytes);
        if self.limits.estimated_memory_bytes != 0 && projected > self.limits.estimated_memory_bytes
        {
            self.exceeded.push(ScanBudgetExceeded::EstimatedMemory {
                limit_bytes: self.limits.estimated_memory_bytes,
                observed_bytes: projected,
            });
            self.stopped = true;
            return false;
        }
        self.estimated_memory_bytes = projected;
        true
    }

    fn upsert_elapsed(&mut self, observed_millis: u64) {
        if let Some(ScanBudgetExceeded::ElapsedTime {
            observed_millis: observed,
            ..
        }) = self
            .exceeded
            .iter_mut()
            .find(|item| matches!(item, ScanBudgetExceeded::ElapsedTime { .. }))
        {
            *observed = (*observed).max(observed_millis);
        } else {
            self.exceeded.push(ScanBudgetExceeded::ElapsedTime {
                limit_millis: self.limits.elapsed_millis,
                observed_millis,
            });
        }
    }

    pub(super) fn finish(&mut self, report: &mut ScanReport) {
        if self.limits.issue_details != 0 && self.issue_details_observed > self.limits.issue_details
        {
            self.exceeded.push(ScanBudgetExceeded::IssueDetails {
                limit: self.limits.issue_details,
                observed: self.issue_details_observed,
            });
        }
        self.exceeded.sort_unstable_by_key(|item| match item {
            ScanBudgetExceeded::EntryCount { .. } => 0,
            ScanBudgetExceeded::ElapsedTime { .. } => 1,
            ScanBudgetExceeded::EstimatedMemory { .. } => 2,
            ScanBudgetExceeded::IssueDetails { .. } => 3,
        });
        report.budget_exceeded = std::mem::take(&mut self.exceeded);
    }
}
