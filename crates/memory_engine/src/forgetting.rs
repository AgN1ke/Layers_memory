use serde::{Deserialize, Serialize};

use crate::archive::{FidelityStatus, ForgetDecision, MemoryUnit, MemoryUnitStatus};
use crate::tasks::PendingTask;
use crate::types::{
    Id, Timestamp, FORGET_REVIEW_INPUT_SCHEMA_VERSION, FORGET_REVIEW_RESULT_SCHEMA_VERSION,
};
use crate::LlmRequest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReviewStart {
    pub pending_task: PendingTask,
    pub request: LlmRequest,
    pub source_session_id: Id,
    pub candidate_count: usize,
    pub candidates: Vec<ForgetReviewCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReviewInputs {
    pub schema_version: String,
    pub source_session_id: Id,
    pub created_at: Timestamp,
    #[serde(default)]
    pub candidates: Vec<ForgetReviewCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReviewCandidate {
    pub label: String,
    pub memory_unit_id: Id,
    pub archive_id: Id,
    pub age_days: f64,
    pub weight: f64,
    pub archive_recall_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_last_recalled_days: Option<f64>,
    pub fidelity_status: FidelityStatus,
    pub has_core_link: bool,
    pub has_emotional: bool,
    pub thesis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReviewResult {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default)]
    pub recommendations: Vec<ForgetRecommendation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetRecommendation {
    pub memory_unit_id: Id,
    pub decision: ForgetDecision,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReviewApplyResult {
    pub schema_version: String,
    pub source_session_id: Id,
    pub reviewed: usize,
    pub forgotten: usize,
    pub kept: usize,
    pub protected: usize,
    pub ignored: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_units: Vec<MemoryUnit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgottenMemoryUnits {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default)]
    pub units: Vec<MemoryUnit>,
}

impl ForgetReviewInputs {
    pub fn empty(source_session_id: Id, created_at: Timestamp) -> Self {
        Self {
            schema_version: FORGET_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            source_session_id,
            created_at,
            candidates: Vec::new(),
        }
    }
}

impl ForgetReviewResult {
    pub fn normalize_schema(&mut self) {
        if self.schema_version.trim().is_empty() {
            self.schema_version = FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string();
        }
    }
}

pub fn is_forgotten(unit: &MemoryUnit) -> bool {
    unit.status == MemoryUnitStatus::Forgotten
}
