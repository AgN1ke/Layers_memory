use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::archive::ArchiveEntry;
use crate::sleep::SleepInputReference;
use crate::types::{Id, ModelRole};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    pub request_id: Id,
    pub task_id: Id,
    pub role_hint: ModelRole,
    pub prompt_id: String,
    pub prompt_version: u32,
    #[serde(default)]
    pub prompt_inputs: Value,
    pub expected_output_schema: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LlmResponse {
    Ok {
        request_id: Id,
        text: String,
    },
    Err {
        request_id: Id,
        kind: LlmErrorKind,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmErrorKind {
    ProviderBlocked,
    Transport,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmBatch {
    /// Prompt material shared by every request in this batch. Hosts should
    /// place this content before the pass-specific instruction so providers
    /// can reuse an identical prefix across parallel Sleep passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_context: Option<LlmSharedContext>,
    pub requests: Vec<LlmRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmSharedContext {
    pub schema_version: String,
    pub context_id: Id,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_event_ids: Vec<Id>,
    #[serde(default)]
    pub event_count: usize,
    #[serde(default)]
    pub source_text_chars: usize,
    #[serde(default)]
    pub rendered_chars: usize,
    #[serde(default)]
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepRunStage {
    Extraction,
    Consolidation,
    ReadyToFinish,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepTrack {
    MemoryUnit,
    Emotional,
    TopicThread,
    PersonalSignal,
    Relational,
    Consolidator,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepRunOptions {
    /// Explicit pass selection. An omitted field keeps the full historical
    /// pipeline; an explicit empty list requests deterministic finalization.
    #[serde(default = "default_sleep_tracks")]
    pub enabled_tracks: Vec<SleepTrack>,
    /// Maximum provider calls for the complete run, including retries and the
    /// consolidator. Once exhausted, unfinished passes receive deterministic
    /// fallbacks and the run remains finishable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_provider_calls: Option<u32>,
    /// Per-pass retry ceiling. `None` keeps the engine default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pass_attempts: Option<u32>,
}

impl Default for SleepRunOptions {
    fn default() -> Self {
        Self {
            enabled_tracks: default_sleep_tracks(),
            max_provider_calls: None,
            max_pass_attempts: None,
        }
    }
}

pub fn default_sleep_tracks() -> Vec<SleepTrack> {
    vec![
        SleepTrack::MemoryUnit,
        SleepTrack::Emotional,
        SleepTrack::TopicThread,
        SleepTrack::PersonalSignal,
        SleepTrack::Relational,
        SleepTrack::Consolidator,
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepRun {
    pub schema_version: String,
    pub session_id: Id,
    pub archive_id: Id,
    pub sleep_task_id: Id,
    /// Storage v2 owns the frozen event payload in one immutable record.
    /// FileStorage runs keep this absent and retain their legacy inline input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_reference: Option<SleepInputReference>,
    /// One compact, prompt-facing source projection shared by every pass.
    /// The full lossless events remain in the task/workflow input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_prompt_context: Option<LlmSharedContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_unit_task_id: Option<Id>,
    pub stage: SleepRunStage,
    #[serde(default = "default_sleep_tracks")]
    pub enabled_tracks: Vec<SleepTrack>,
    pub max_pass_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_provider_calls: Option<u32>,
    #[serde(default)]
    pub provider_calls_issued: u32,
    #[serde(default)]
    pub budget_exhausted: bool,
    #[serde(default)]
    pub requests: Vec<SleepRequestState>,
    #[serde(default)]
    pub failed_passes: Vec<String>,
    #[serde(default)]
    pub skipped_passes: Vec<String>,
    #[serde(default)]
    pub memory_unit_result: Option<Value>,
    #[serde(default)]
    pub emotional_pass: Option<Value>,
    #[serde(default)]
    pub topic_thread_pass: Option<Value>,
    #[serde(default)]
    pub personal_signal_pass: Option<Value>,
    #[serde(default)]
    pub relational_pass: Option<Value>,
    #[serde(default)]
    pub consolidator_gist: Option<String>,
    #[serde(default)]
    pub consolidator_narrative: Option<String>,
    #[serde(default)]
    pub completion_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepRequestState {
    pub track: SleepTrack,
    pub request: LlmRequest,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    /// Every rejected/provider-failed attempt remains observable even when a
    /// later retry succeeds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempt_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepRunStep {
    pub run: SleepRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<LlmBatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepOutcome {
    pub archive_entry: ArchiveEntry,
    pub core_summary: CoreSignalSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fidelity_requests: Vec<LlmRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding_requests: Vec<LlmRequest>,
    #[serde(default)]
    pub failed_passes: Vec<String>,
    #[serde(default)]
    pub skipped_passes: Vec<String>,
    #[serde(default)]
    pub pass_reports: Vec<SleepPassReport>,
    #[serde(default)]
    pub provider_calls_issued: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_provider_calls: Option<u32>,
    #[serde(default)]
    pub budget_exhausted: bool,
    pub completion_mode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepPassReport {
    pub track: SleepTrack,
    pub prompt_id: String,
    pub attempts: u32,
    pub completed: bool,
    pub recovered_after_retry: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSignalSummary {
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
    #[serde(default)]
    pub review_candidates: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skip_reasons: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreArchiveSeedSummary {
    pub archives: usize,
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
}
