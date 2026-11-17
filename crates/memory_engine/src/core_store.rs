use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::recall::RecallItem;
use crate::types::{Id, Link, Speaker, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreFactStatus {
    Active,
    Contested,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution_changed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attribution_history: Vec<CoreFactAttributionRevision>,
}

pub const CORE_FACT_ATTRIBUTION_INPUT_SCHEMA: &str = "core_fact_attribution_input.v1";
pub const CORE_FACT_ATTRIBUTION_RESULT_SCHEMA: &str = "core_fact_attribution_result.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreFactAttributionInput {
    pub schema_version: String,
    pub scope_id: String,
    pub core_fact_id: Id,
    pub operation_id: Id,
    pub expected_updated_at: Timestamp,
    pub change: CoreFactSubjectChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CoreFactSubjectChange {
    Assign { subject: Speaker },
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreFactAttributionRevision {
    pub applied_at: Timestamp,
    pub operation_digest: String,
    pub request_digest: String,
    pub assertion_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_subject_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_subject_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreFactAttributionResult {
    pub schema_version: String,
    pub category: String,
    pub fact: CoreFact,
    pub applied_at: Timestamp,
    pub already_applied: bool,
}

pub(crate) fn core_subject_digest(scope: &str, subject_id: &str) -> crate::Result<String> {
    crate::engine::storage_digest(&("core_fact_attribution_subject", scope, subject_id))
}

impl CoreFactAttributionRevision {
    pub(crate) fn refers_to_subject(&self, subject_digest: &str) -> bool {
        self.previous_subject_digest.as_deref() == Some(subject_digest)
            || self.assigned_subject_digest.as_deref() == Some(subject_digest)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
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
pub struct CoreFactPatchInput {
    pub schema_version: String,
    pub core_fact_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<CoreFactStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreFactPatchResult {
    pub schema_version: String,
    pub category: String,
    pub fact: CoreFact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextFact {
    pub category: String,
    pub core_fact_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
    pub text: String,
    #[serde(default = "default_core_context_fact_status")]
    pub status: CoreFactStatus,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

fn default_core_context_fact_status() -> CoreFactStatus {
    CoreFactStatus::Active
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_embedding: Option<CoreContextQueryEmbedding>,
    #[serde(default)]
    pub explain_recall: bool,
    #[serde(default)]
    pub recall_limit: usize,
    #[serde(default)]
    pub session_recent_limit: usize,
    #[serde(default)]
    pub session_trace_event_limit: usize,
    #[serde(default)]
    pub include_core: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<CoreContextTokenBudget>,
    /// Host-local UTC offset in minutes for calendar-day time labels
    /// ("yesterday" is a local-calendar concept while storage is UTC).
    #[serde(default)]
    pub utc_offset_minutes: i32,
    /// Host may mark its clock as untrusted; time labels are then omitted
    /// instead of rendering wrong ones.
    #[serde(default)]
    pub clock_untrusted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextQueryEmbedding {
    pub model_id: String,
    pub query_vec: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextEvent {
    pub event_id: Id,
    pub timestamp: Timestamp,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Budgeted source projections, present only for explicitly referenced quotes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quoted_events: Vec<CoreContextQuote>,
    /// Lossless host-owned event data. Prompt renderers ignore it; structured
    /// chat hosts can recover role/tool metadata without reading Session twice.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreContextQuote {
    pub event_id: Id,
    pub status: CoreContextQuoteStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreContextQuoteStatus {
    Available,
    Unavailable,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// A transient older memory activated by the current conversational focus.
///
/// This is a prompt projection from the rebuildable vector sidecar, not a
/// fourth persisted memory layer and not an item in the Archive recall rank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssociativeMemoryItem {
    pub memory_unit_id: Id,
    pub archive_id: Id,
    pub scope: String,
    pub thesis: String,
    pub created_at: Timestamp,
    pub activation: AssociativeMemoryActivation,
    pub semantic_similarity: f32,
    #[serde(default)]
    pub activation_threshold: f32,
    pub deep_recall_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_recalled: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociativeMemoryActivation {
    CurrentTopicVector,
    CoreAnchor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreContextPackage {
    pub schema_version: String,
    pub created_at: Timestamp,
    #[serde(default)]
    pub utc_offset_minutes: i32,
    #[serde(default)]
    pub clock_untrusted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub core_facts: Vec<CoreContextFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_recent: Vec<CoreContextEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_trace: Vec<CoreContextEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_relevant: Vec<RecallItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub associative_memory: Vec<AssociativeMemoryItem>,
    #[serde(default)]
    pub domain_state: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<CoreContextBudgetReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_debug: Option<crate::recall::RecallDebug>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreContextTokenBudget {
    pub total_tokens: usize,
    pub current_memory_tokens: usize,
    pub compressed_memory_tokens: usize,
    pub core_tokens: usize,
}

impl Default for CoreContextTokenBudget {
    fn default() -> Self {
        Self {
            total_tokens: 11_000,
            current_memory_tokens: 7_000,
            compressed_memory_tokens: 3_000,
            core_tokens: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreContextBudgetReport {
    pub estimator: String,
    pub total_budget_tokens: usize,
    pub current_memory_budget_tokens: usize,
    pub compressed_memory_budget_tokens: usize,
    pub core_budget_tokens: usize,
    pub estimated_total_tokens: usize,
    pub estimated_current_memory_tokens: usize,
    pub estimated_compressed_memory_tokens: usize,
    pub estimated_core_tokens: usize,
    pub estimated_domain_state_tokens: usize,
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub estimated_associative_memory_tokens: usize,
    pub dropped_session_recent: usize,
    pub dropped_session_trace: usize,
    pub dropped_archive_relevant: usize,
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub dropped_associative_memory: usize,
    pub dropped_core_facts: usize,
    pub budget_exceeded: bool,
}

fn usize_is_zero(value: &usize) -> bool {
    *value == 0
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
pub struct CandidateReinforcement {
    pub reinforcement_id: Id,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_session_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_archive_ids: Vec<Id>,
    pub evidence_summary: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateBelief {
    pub schema_version: String,
    pub candidate_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_session_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicted_core_fact_ids: Vec<Id>,
    pub evidence_summary: String,
    pub promotion_checks: PromotionChecks,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_core_fact_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Evidence-bearing updates from later reflection passes. Reinforcement
    /// matures one candidate instead of minting semantic duplicates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reinforcements: Vec<CandidateReinforcement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateReviewInput {
    pub schema_version: String,
    pub candidate_id: Id,
    pub reviewed_by: String,
    pub decision: ReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateReviewResult {
    pub schema_version: String,
    pub candidate: CandidateBelief,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_fact: Option<CoreFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contested_facts: Vec<CoreFact>,
}
