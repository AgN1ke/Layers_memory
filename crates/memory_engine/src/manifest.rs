use serde::{Deserialize, Serialize};

use crate::types::{RecallStage, Timestamp};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: String,
    pub engine_version: String,
    pub storage_id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub schema_versions: SchemaVersions,
    #[serde(default)]
    pub active_embedding_model_id: Option<String>,
    #[serde(default)]
    pub last_migration_at: Option<Timestamp>,
    pub features: FeatureFlags,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaVersions {
    pub event: String,
    pub session: String,
    pub archive_entry: String,
    pub core_store: String,
    pub core_fact: String,
    pub candidate_belief: String,
    pub pending_task: String,
    pub journal_operation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureFlags {
    pub recall_stage: RecallStage,
    pub embeddings_enabled: bool,
    pub llm_recall_rerank_enabled: bool,
    pub reflection_enabled: bool,
}
