use serde::{Deserialize, Serialize};

use crate::types::{Id, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryErasureKind {
    DeleteSession,
    PurgeScope,
}

/// Local admission stamp. Capture it before queuing work and reuse it on retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteGeneration {
    pub schema_version: String,
    pub scope_key: String,
    pub session_key: String,
    pub scope_generation: u64,
    pub session_generation: u64,
}

pub(crate) const WRITE_GENERATION_SCHEMA: &str = "write_generation.v1";

pub(crate) fn session_generation_id(session_id: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    return std::borrow::Cow::Owned(session_id.to_lowercase());
    #[cfg(not(windows))]
    std::borrow::Cow::Borrowed(session_id)
}

pub(crate) fn erasure_key(kind: MemoryErasureKind, target: &str) -> String {
    use sha2::{Digest, Sha256};

    let (prefix, target) = match kind {
        MemoryErasureKind::DeleteSession => ("session", session_generation_id(target)),
        MemoryErasureKind::PurgeScope => ("scope", target.into()),
    };
    format!("{prefix}_{:x}", Sha256::digest(target.as_bytes()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryErasureCounts {
    pub sessions: usize,
    pub active_events: usize,
    pub archived_events: usize,
    pub archives: usize,
    pub memory_units: usize,
    pub core_facts: usize,
    pub candidates: usize,
    pub hypothesis_nodes: usize,
    pub hypothesis_evidence_links: usize,
    pub hypothesis_relations: usize,
    pub tasks: usize,
    pub sleep_runs: usize,
    pub reflection_states: usize,
    pub vector_scopes: usize,
    pub journal_operations: usize,
}

impl MemoryErasureCounts {
    pub fn is_empty(&self) -> bool {
        self.sessions == 0
            && self.active_events == 0
            && self.archived_events == 0
            && self.archives == 0
            && self.memory_units == 0
            && self.core_facts == 0
            && self.candidates == 0
            && self.hypothesis_nodes == 0
            && self.hypothesis_evidence_links == 0
            && self.hypothesis_relations == 0
            && self.tasks == 0
            && self.sleep_runs == 0
            && self.reflection_states == 0
            && self.vector_scopes == 0
            && self.journal_operations == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryErasureReport {
    pub schema_version: String,
    pub operation: MemoryErasureKind,
    pub target_id: String,
    pub completed_at: Timestamp,
    pub deleted: MemoryErasureCounts,
    /// Session deletion removes conversation records while preserving the
    /// compressed memories already promoted into the personality scope.
    pub archived_memory_preserved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureCoreFactRef {
    pub category: String,
    pub core_fact_id: Id,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryErasurePlan {
    pub schema_version: String,
    pub operation: MemoryErasureKind,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub core_facts: Vec<ErasureCoreFactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypothesis_node_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypothesis_evidence_link_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypothesis_relation_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sleep_task_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reflection_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub journal_operation_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryErasureOperationIntent {
    pub schema_version: String,
    pub plan: MemoryErasurePlan,
    pub planned_counts: MemoryErasureCounts,
    pub archived_memory_preserved: bool,
}
