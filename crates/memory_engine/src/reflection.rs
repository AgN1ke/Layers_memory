use serde::{Deserialize, Serialize};

use crate::core_store::CandidateBelief;
use crate::tasks::PendingTask;
use crate::types::{Id, Timestamp};
use crate::LlmRequest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionPassStart {
    pub pending_task: PendingTask,
    pub request: LlmRequest,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    pub memory_unit_count: usize,
    pub core_fact_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionAnalyzeResult {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    #[serde(default)]
    pub candidates: Vec<ReflectionCandidateDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionCandidateDraft {
    pub text: String,
    pub category: String,
    pub confidence: f64,
    pub evidence_summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicted_core_fact_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionCandidatesResult {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    pub created_at: Timestamp,
    #[serde(default)]
    pub candidates: Vec<CandidateBelief>,
}
