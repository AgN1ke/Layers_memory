use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::event::StoredEvent;
use crate::types::{Id, Timestamp, SESSION_SCHEMA_VERSION};
use crate::{MemoryEngineError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    SleepPending,
    Archived,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub schema_version: String,
    pub session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_scope_id: Option<Id>,
    pub host_id: String,
    pub status: SessionStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<Timestamp>,
    pub event_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_to: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_event_ids: Vec<Id>,
    #[serde(default)]
    pub archived_event_index_complete: bool,
    /// Storage v2 sleep-start claim. It prevents two concurrent runs from
    /// consolidating the same live events. Cleared only by a terminal sleep
    /// transition (finish/cancel), never by a host-side timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_sleep_task_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sleep_pending_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub metadata: SessionMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<StoredEvent>,
}

impl SessionMetadata {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SESSION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SESSION_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        if self.session_id.trim().is_empty() || self.host_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "session identity fields must not be empty".to_string(),
            ));
        }
        if self
            .personality_scope_id
            .as_ref()
            .is_some_and(|scope| scope.trim().is_empty())
        {
            return Err(MemoryEngineError::Validation(
                "session personality_scope_id must not be empty when provided".to_string(),
            ));
        }
        if OffsetDateTime::parse(&self.created_at, &Rfc3339).is_err()
            || OffsetDateTime::parse(&self.updated_at, &Rfc3339).is_err()
            || self
                .closed_at
                .as_ref()
                .is_some_and(|value| OffsetDateTime::parse(value, &Rfc3339).is_err())
        {
            return Err(MemoryEngineError::Validation(
                "session timestamps must be RFC3339".to_string(),
            ));
        }
        let reservation_is_complete =
            self.active_sleep_task_id.is_some() && !self.sleep_pending_event_ids.is_empty();
        let reservation_is_empty =
            self.active_sleep_task_id.is_none() && self.sleep_pending_event_ids.is_empty();
        if !reservation_is_complete && !reservation_is_empty {
            return Err(MemoryEngineError::Validation(
                "session sleep reservation fields must be both present or both absent".to_string(),
            ));
        }
        Ok(())
    }

    pub fn effective_personality_scope_id(&self) -> &str {
        self.personality_scope_id
            .as_deref()
            .unwrap_or(&self.session_id)
    }
}
