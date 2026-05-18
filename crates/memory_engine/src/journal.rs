use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Id, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalOperationType {
    Sleep,
    Migration,
    CorePromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    Started,
    Completed,
    Failed,
    NeedsManualReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPolicy {
    Retry,
    Rollback,
    ManualReview,
    RetryOrManualReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalOperation {
    pub schema_version: String,
    pub op_id: Id,
    pub op_type: JournalOperationType,
    pub state: JournalState,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub intent: Value,
    pub recovery_policy: RecoveryPolicy,
    #[serde(default)]
    pub completed_at: Option<Timestamp>,
    #[serde(default)]
    pub error: Option<String>,
}
