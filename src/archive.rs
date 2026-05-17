use serde::{Deserialize, Serialize};

use crate::types::{Id, Link, Quote, TimeRange, Timestamp, WeightedFact};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveStatus {
    Preliminary,
    Complete,
    Superseded,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub schema_version: String,
    pub archive_id: Id,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub source_session_id: Id,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    pub time_range: TimeRange,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub gist: String,
    pub narrative: String,
    #[serde(default)]
    pub facts: Vec<WeightedFact>,
    #[serde(default)]
    pub quotes: Vec<Quote>,
    pub weight: f64,
    pub freshness: f64,
    pub recall_count: u64,
    #[serde(default)]
    pub last_recalled_at: Option<Timestamp>,
    #[serde(default)]
    pub links: Vec<Link>,
    pub status: ArchiveStatus,
    pub llm_enhanced: bool,
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(default)]
    pub prompt_version: Option<u32>,
    #[serde(default)]
    pub embedding_model_id: Option<String>,
    #[serde(default)]
    pub embedding: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveFilters {
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
}
