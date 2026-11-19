use serde::{Deserialize, Serialize};

use crate::types::{Id, Timestamp};

pub const SUBJECT_MEMORY_VIEW_SCHEMA: &str = "subject_memory_view.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectMemoryView {
    pub schema_version: String,
    pub scope_id: Id,
    pub subject_id: Id,
    pub items: Vec<SubjectMemoryItem>,
    /// The scope contains material whose attribution or provenance is incomplete.
    pub incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectMemoryItem {
    pub kind: SubjectMemoryKind,
    pub id: Id,
    pub timestamp: Timestamp,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectMemoryKind {
    Event,
    MemoryUnit,
    CoreFact,
    Candidate,
    Hypothesis,
}
