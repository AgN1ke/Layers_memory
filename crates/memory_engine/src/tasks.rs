use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Id, ModelRole, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    SleepCompression,
    CompactMemoryPass,
    ScoreEvent,
    ReflectionAnalyze,
    RecallRerank,
    ComputeEmbedding,
    FactCheck,
    TagProposal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Submitted,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingTask {
    pub schema_version: String,
    pub task_id: Id,
    pub task_type: TaskType,
    pub state: TaskState,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub prompt_id: String,
    pub prompt_version: u32,
    pub role_hint: ModelRole,
    pub expected_output_schema: String,
    #[serde(default)]
    pub inputs: Value,
    #[serde(default)]
    pub attempts: Vec<TaskAttempt>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskAttempt {
    pub attempt_id: Id,
    pub started_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: AttemptStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
