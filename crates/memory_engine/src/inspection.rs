use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::archive::{ArchiveEntry, ArchiveStatus, FidelityStatus, MemoryUnit, MemoryUnitStatus};
use crate::storage::StorageReadWarning;
use crate::tasks::{TaskState, TaskType};
use crate::types::{Speaker, TimeRange};

pub const SCOPED_MEMORY_SUMMARY_SCHEMA: &str = "scoped_memory_summary.v1";
pub const SCOPED_ARCHIVE_DETAIL_SCHEMA: &str = "scoped_archive_detail.v1";
pub const SCOPED_MEMORY_UNIT_DETAIL_SCHEMA: &str = "scoped_memory_unit_detail.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedMemorySummary {
    pub schema_version: String,
    pub scope_id: String,
    pub session_ids: Vec<String>,
    pub archives: Vec<ScopedArchiveSummary>,
    pub operations: Vec<ScopedPendingOperation>,
    pub counts: ScopedMemoryCounts,
    pub warnings: Vec<StorageReadWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedMemoryCounts {
    pub sessions: usize,
    pub complete_archives: usize,
    pub active_memory_units: usize,
    pub core_facts: usize,
    pub pending_by_type: BTreeMap<TaskType, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedArchiveSummary {
    pub archive_id: String,
    pub source_session_id: String,
    pub time_range: TimeRange,
    pub status: ArchiveStatus,
    pub memory_units: Vec<ScopedMemoryUnitSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedMemoryUnitSummary {
    pub memory_unit_id: String,
    pub status: MemoryUnitStatus,
    pub fidelity_status: FidelityStatus,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedPendingOperation {
    pub task_id: String,
    pub task_type: TaskType,
    pub state: TaskState,
    pub created_at: String,
    pub updated_at: String,
    pub needs_new_review: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedArchiveDetail {
    pub schema_version: String,
    pub scope_id: String,
    pub archive_id: String,
    /// Absent when ownership is known but the publication cannot be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveEntry>,
    pub sources: Vec<ScopedSourceEvent>,
    pub warnings: Vec<StorageReadWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedMemoryUnitDetail {
    pub schema_version: String,
    pub scope_id: String,
    pub unit: MemoryUnit,
    pub sources: Vec<ScopedSourceEvent>,
    pub warnings: Vec<StorageReadWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedSourceEvent {
    pub event_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_text: Option<String>,
}
