use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Id, RecallStage, TimeRange, Timestamp};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallQuery {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_scope_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Id>,
    #[serde(default)]
    pub context: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_embedding: Option<RecallQueryEmbedding>,
    pub filters: RecallFilters,
    pub limit: usize,
    pub include_core: bool,
    pub explain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallQueryEmbedding {
    pub model_id: String,
    pub query_vec: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecallFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_freshness: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_layers: Vec<RecallSourceLayer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallSourceLayer {
    Session,
    Archive,
    Core,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallResult {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_id: Option<Id>,
    pub created_at: Timestamp,
    pub stage_used: RecallStage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<RecallItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<RecallDebug>,
}

impl RecallResult {
    /// Keep the optional debug view aligned with the response diagnostics.
    pub fn append_notes(&mut self, notes: Vec<String>) {
        if let Some(debug) = &mut self.debug {
            debug.notes.extend(notes.iter().cloned());
        }
        self.notes.extend(notes);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallItem {
    pub source_layer: RecallSourceLayer,
    pub id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_memory_unit_id: Option<Id>,
    pub gist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub weight: f64,
    pub freshness: f64,
    pub relevance_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relevance_explanation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallDebug {
    #[serde(default = "default_recall_debug_stage")]
    pub stage_used: RecallStage,
    pub candidate_count: usize,
    pub filtered_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<RecallCandidateDebug>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_stage: Option<RecallVectorStageDebug>,
}

fn default_recall_debug_stage() -> RecallStage {
    RecallStage::Stage1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallCandidateDebug {
    pub archive_id: Id,
    pub stage1_rank: usize,
    pub final_rank: usize,
    pub selected: bool,
    pub stage1_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_memory_unit_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_similarity: Option<f64>,
    pub final_score: f64,
    pub components: RecallScoreComponents,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallScoreComponents {
    pub weight: f64,
    pub stored_freshness: f64,
    pub freshness_decay: f64,
    pub age_days: f64,
    pub effective_freshness: f64,
    pub recall_boost: f64,
    pub theme_factor: f64,
    pub tag_factor: f64,
    pub text_factor: f64,
    pub query_token_count: usize,
    pub text_overlap_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallVectorStageDebug {
    pub status: RecallVectorStageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub candidate_limit: usize,
    pub candidate_count: usize,
    pub matched_archive_count: usize,
    pub vector_weight: f64,
    pub stage1_weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallVectorStageStatus {
    NotRequested,
    Disabled,
    Building,
    Corrupt,
    Applied,
}
