use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::PlannedAction;

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
    #[serde(default)]
    pub pending: usize,
    #[serde(default)]
    pub not_attempted: usize,
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
    /// Selected for this run, but the executor has not been called.
    NotAttempted,
    /// The operation intent was recorded, but its outcome is not yet durable.
    Pending,
    Restored,
    Failed,
    Skipped,
}
