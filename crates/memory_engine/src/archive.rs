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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emotional_markers: Vec<EmotionalMarker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_thread: Vec<TopicThreadItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personal_signals: Vec<PersonalSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relational_tone: Option<RelationalTone>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionalMarker {
    pub target: String,
    pub affect: String,
    pub strength: f64,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicThreadItem {
    pub topic: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtopics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalSignal {
    pub text: String,
    pub category: String,
    pub confidence: f64,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationalTone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warmth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intellectual_engagement: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intimacy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playfulness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
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
