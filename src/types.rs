use serde::{Deserialize, Serialize};

pub type Id = String;
pub type Timestamp = String;

pub const EVENT_SCHEMA_VERSION: &str = "event.v1";
pub const SESSION_SCHEMA_VERSION: &str = "session.v1";
pub const ARCHIVE_ENTRY_SCHEMA_VERSION: &str = "archive_entry.v1";
pub const CORE_STORE_SCHEMA_VERSION: &str = "core_store.v1";
pub const CORE_FACT_SCHEMA_VERSION: &str = "core_fact.v1";
pub const CANDIDATE_BELIEF_SCHEMA_VERSION: &str = "candidate_belief.v1";
pub const RECALL_QUERY_SCHEMA_VERSION: &str = "recall_query.v1";
pub const RECALL_RESULT_SCHEMA_VERSION: &str = "recall_result.v1";
pub const PENDING_TASK_SCHEMA_VERSION: &str = "pending_task.v1";
pub const SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION: &str = "sleep_compression_result.v1";
pub const MANIFEST_SCHEMA_VERSION: &str = "manifest.v1";
pub const JOURNAL_OPERATION_SCHEMA_VERSION: &str = "journal_operation.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub kind: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Timestamp,
    pub end: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportanceHint {
    Low,
    Normal,
    Medium,
    High,
    Critical,
}

impl Default for ImportanceHint {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingMode {
    Immediate,
    DeferToSleep,
}

impl Default for ProcessingMode {
    fn default() -> Self {
        Self::DeferToSleep
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Reasoning,
    Balanced,
    Fast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallStage {
    Stage1,
    Stage2Embeddings,
    Stage3Llm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightedFact {
    pub text: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_event_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<Id>,
}
