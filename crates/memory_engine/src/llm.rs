use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::archive::ArchiveEntry;
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
    pub requests: Vec<LlmRequest>,
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
pub struct SleepRun {
    pub schema_version: String,
    pub session_id: Id,
    pub archive_id: Id,
    pub sleep_task_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_unit_task_id: Option<Id>,
    pub stage: SleepRunStage,
    pub max_pass_attempts: u32,
    #[serde(default)]
    pub requests: Vec<SleepRequestState>,
    #[serde(default)]
    pub failed_passes: Vec<String>,
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
    pub consolidated_result: Option<Value>,
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
    #[serde(default)]
    pub failed_passes: Vec<String>,
    pub completion_mode: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSignalSummary {
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreArchiveSeedSummary {
    pub archives: usize,
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
}
