use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::recall::RecallItem;
use crate::types::{Id, Link, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreFactStatus {
    Active,
    Deprecated,
    Contradicted,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    Rejected,
    NeedsChanges,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub reviewed_by: String,
    pub reviewed_at: Timestamp,
    pub decision: ReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreFact {
    pub schema_version: String,
    pub core_fact_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub text: String,
    pub status: CoreFactStatus,
    pub confidence: f64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreStoreCategory {
    pub schema_version: String,
    pub category: String,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<CoreFact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreFactInput {
    pub schema_version: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub text: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreFactUpsertResult {
    pub schema_version: String,
    pub category: String,
    pub created: bool,
    pub fact: CoreFact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextFact {
    pub category: String,
    pub core_fact_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub text: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextRequest {
    pub schema_version: String,
    pub session_id: Id,
    #[serde(default)]
    pub domain_state: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_text: Option<String>,
    #[serde(default)]
    pub recall_limit: usize,
    #[serde(default)]
    pub session_recent_limit: usize,
    #[serde(default)]
    pub session_trace_event_limit: usize,
    #[serde(default)]
    pub include_core: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextEvent {
    pub event_id: Id,
    pub timestamp: Timestamp,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextPackage {
    pub schema_version: String,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub core_facts: Vec<CoreContextFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_recent: Vec<CoreContextEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_trace: Vec<CoreContextEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_relevant: Vec<RecallItem>,
    #[serde(default)]
    pub domain_state: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionChecks {
    pub min_sources_met: bool,
    pub weight_threshold_met: bool,
    pub no_recent_contradiction: bool,
    pub manual_review_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Draft,
    ReadyForReview,
    Approved,
    Rejected,
    Promoted,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateBelief {
    pub schema_version: String,
    pub candidate_id: Id,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub text: String,
    pub category: String,
    pub status: CandidateStatus,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicting_archive_ids: Vec<Id>,
    pub evidence_summary: String,
    pub promotion_checks: PromotionChecks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}
