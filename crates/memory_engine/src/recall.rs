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
    pub session_id: Option<Id>,
    #[serde(default)]
    pub context: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_text: Option<String>,
    pub filters: RecallFilters,
    pub limit: usize,
    pub include_core: bool,
    pub explain: bool,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallItem {
    pub source_layer: RecallSourceLayer,
    pub id: Id,
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
    pub candidate_count: usize,
    pub filtered_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}
