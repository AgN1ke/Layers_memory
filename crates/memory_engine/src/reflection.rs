use serde::{Deserialize, Serialize};

use crate::core_store::CandidateBelief;
use crate::tasks::PendingTask;
use crate::types::{Id, Speaker, Timestamp};
use crate::LlmRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionOpportunity {
    AfterSleep,
    ScheduledMaintenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionTriggerReason {
    NoEligibleMemory,
    NotDue,
    AlreadyPending,
    CriticalMemory,
    AccumulatedSalience,
    ScheduledInterval,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionState {
    pub schema_version: String,
    pub reflection_scope: String,
    /// Sessions explicitly observed under this personality/core scope.
    /// The legacy singular field remains the most recent triggering session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_session_ids: Vec<Id>,
    pub source_session_id: Id,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_trigger_reason: Option<ReflectionTriggerReason>,
    #[serde(default)]
    pub last_candidate_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionTriggerStatus {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_session_ids: Vec<Id>,
    pub reflection_scope: String,
    pub opportunity: ReflectionOpportunity,
    pub evaluated_at: Timestamp,
    pub eligible_memory_unit_count: usize,
    pub accumulated_salience: f64,
    pub salience_threshold: f64,
    pub critical_unit_weight: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_eligible_memory_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_task_id: Option<Id>,
    pub due: bool,
    pub reason: ReflectionTriggerReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionPassStart {
    pub pending_task: PendingTask,
    pub request: LlmRequest,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    pub memory_unit_count: usize,
    #[serde(default)]
    pub new_memory_unit_count: usize,
    #[serde(default)]
    pub related_memory_unit_count: usize,
    #[serde(default)]
    pub existing_candidate_count: usize,
    pub core_fact_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomaticReflectionStart {
    pub status: ReflectionTriggerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<ReflectionPassStart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionAnalyzeResult {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    #[serde(default)]
    pub candidates: Vec<ReflectionCandidateDraft>,
}

/// The explicit result contract used by autonomous reflection tasks.
///
/// `reflection_result.v1` remains readable for saved legacy tasks, while new
/// tasks use this action-oriented contract so that an existing conclusion can
/// mature without being duplicated and a Core conflict can remain reviewable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionActionsResult {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ReflectionActionDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_insight: Option<ReflectionNoInsight>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ReflectionActionDraft {
    NewCandidate {
        candidate: ReflectionCandidateDraft,
    },
    ReinforceCandidate {
        candidate_id: Id,
        confidence: f64,
        evidence_summary: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        source_memory_unit_ids: Vec<Id>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        supporting_archive_ids: Vec<Id>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
    },
    ChallengeCoreFact {
        candidate: ReflectionCandidateDraft,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionNoInsightReason {
    InsufficientRepetition,
    AlreadyRepresented,
    TemporaryState,
    ConflictingEvidence,
    NoStablePattern,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionNoInsight {
    pub reason: ReflectionNoInsightReason,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionCandidateDraft {
    pub text: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
    pub confidence: f64,
    pub evidence_summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicted_core_fact_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionCandidatesResult {
    pub schema_version: String,
    pub source_session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    pub created_at: Timestamp,
    #[serde(default)]
    pub candidates: Vec<CandidateBelief>,
    /// Updated existing candidates. These are not new beliefs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reinforced_candidates: Vec<CandidateBelief>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_insight: Option<ReflectionNoInsight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_digest: Option<String>,
}

/// Text-free durable summary saved on the completed PendingTask.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionOutcome {
    pub schema_version: String,
    pub operation_id: Id,
    pub result_digest: String,
    pub completed_at: Timestamp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_candidate_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reinforced_candidate_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_insight_reason: Option<ReflectionNoInsightReason>,
}
