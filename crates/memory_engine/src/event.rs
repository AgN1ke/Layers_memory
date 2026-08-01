use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::types::{
    Id, ImportanceHint, Link, ProcessingMode, Speaker, Timestamp, EVENT_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestEvent {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub timestamp: Timestamp,
    /// Stable memory/personality boundary. `session_id` identifies one live
    /// interaction; this scope may span many such interactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_scope_id: Option<Id>,
    pub session_id: Id,
    /// Host-owned text that the memory system may reason over. `payload`
    /// remains the lossless domain event and may contain non-text data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_text: Option<String>,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_tone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default)]
    pub importance_hint: ImportanceHint,
    #[serde(default)]
    pub processing_mode: ProcessingMode,
}

impl IngestEvent {
    /// Validates the domain event independently of a physical storage backend.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EVENT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: EVENT_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        if self.event_type.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "event type must not be empty".to_string(),
            ));
        }
        if self.source.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "event source must not be empty".to_string(),
            ));
        }
        if OffsetDateTime::parse(&self.timestamp, &Rfc3339).is_err() {
            return Err(MemoryEngineError::Validation(
                "event timestamp must be RFC3339".to_string(),
            ));
        }
        if self.session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "event session_id must not be empty".to_string(),
            ));
        }
        if self
            .personality_scope_id
            .as_ref()
            .is_some_and(|scope| scope.trim().is_empty())
        {
            return Err(MemoryEngineError::Validation(
                "event personality_scope_id must not be empty when provided".to_string(),
            ));
        }
        if self
            .memory_text
            .as_ref()
            .is_some_and(|memory_text| memory_text.trim().is_empty())
        {
            return Err(MemoryEngineError::Validation(
                "event memory_text must not be empty when provided".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub schema_version: String,
    pub event_id: Id,
    pub received_at: Timestamp,
    /// Server-assigned order inside one Session. FileStorage/legacy events may
    /// omit it and retain append/receive-order compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_sequence: Option<u64>,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub timestamp: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_scope_id: Option<Id>,
    pub session_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_text: Option<String>,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_tone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default)]
    pub importance_hint: ImportanceHint,
    #[serde(default)]
    pub processing_mode: ProcessingMode,
    pub initial_weight: f64,
    pub weight_reason: String,
}

impl StoredEvent {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EVENT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: EVENT_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        if self.event_id.trim().is_empty()
            || self.event_type.trim().is_empty()
            || self.source.trim().is_empty()
            || self.session_id.trim().is_empty()
        {
            return Err(MemoryEngineError::Validation(
                "stored event identity fields must not be empty".to_string(),
            ));
        }
        if self
            .personality_scope_id
            .as_ref()
            .is_some_and(|scope| scope.trim().is_empty())
        {
            return Err(MemoryEngineError::Validation(
                "stored event personality_scope_id must not be empty when provided".to_string(),
            ));
        }
        if OffsetDateTime::parse(&self.timestamp, &Rfc3339).is_err()
            || OffsetDateTime::parse(&self.received_at, &Rfc3339).is_err()
        {
            return Err(MemoryEngineError::Validation(
                "stored event timestamps must be RFC3339".to_string(),
            ));
        }
        if self
            .memory_text
            .as_ref()
            .is_some_and(|memory_text| memory_text.trim().is_empty())
        {
            return Err(MemoryEngineError::Validation(
                "stored event memory_text must not be empty when provided".to_string(),
            ));
        }
        if self.admission_sequence == Some(0) {
            return Err(MemoryEngineError::Validation(
                "stored event admission_sequence must be at least 1".to_string(),
            ));
        }
        Ok(())
    }

    pub fn from_ingest(
        event: IngestEvent,
        event_id: impl Into<Id>,
        received_at: impl Into<Timestamp>,
        initial_weight: f64,
        weight_reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: event.schema_version,
            event_id: event_id.into(),
            received_at: received_at.into(),
            admission_sequence: None,
            event_type: event.event_type,
            source: event.source,
            timestamp: event.timestamp,
            personality_scope_id: event.personality_scope_id,
            session_id: event.session_id,
            memory_text: event.memory_text,
            payload: event.payload,
            tags: event.tags,
            theme: event.theme,
            emotional_tone: event.emotional_tone,
            speaker: event.speaker,
            links: event.links,
            importance_hint: event.importance_hint,
            processing_mode: event.processing_mode,
            initial_weight,
            weight_reason: weight_reason.into(),
        }
    }

    pub fn effective_personality_scope_id(&self) -> &str {
        self.personality_scope_id
            .as_deref()
            .unwrap_or(&self.session_id)
    }
}
