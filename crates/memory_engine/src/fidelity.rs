use serde::{Deserialize, Serialize};

use crate::tasks::PendingTask;
use crate::types::{Id, Timestamp, EVIDENCE_PACK_SCHEMA_VERSION};
use crate::LlmRequest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidencePack {
    pub schema_version: String,
    pub evidence_pack_id: Id,
    pub created_at: Timestamp,
    pub memory_unit_id: Id,
    pub archive_id: Id,
    pub source_session_id: Id,
    pub target_thesis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_evidence: Option<String>,
    #[serde(default)]
    pub events: Vec<EvidenceEvent>,
    pub max_estimated_tokens: usize,
    pub estimated_tokens: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEvent {
    pub event_id: Id,
    pub timestamp: Timestamp,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub role: EvidenceEventRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceEventRole {
    Source,
    Neighbor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryFidelityPassStart {
    pub evidence_pack: EvidencePack,
    pub pending_task: PendingTask,
    pub request: LlmRequest,
}

impl EvidencePack {
    pub fn empty_for(
        evidence_pack_id: Id,
        created_at: Timestamp,
        memory_unit_id: Id,
        archive_id: Id,
        source_session_id: Id,
        target_thesis: String,
        max_estimated_tokens: usize,
    ) -> Self {
        Self {
            schema_version: EVIDENCE_PACK_SCHEMA_VERSION.to_string(),
            evidence_pack_id,
            created_at,
            memory_unit_id,
            archive_id,
            source_session_id,
            target_thesis,
            unit_evidence: None,
            events: Vec::new(),
            max_estimated_tokens,
            estimated_tokens: 0,
            truncated: false,
        }
    }
}
