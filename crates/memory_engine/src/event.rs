use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub session_id: Id,
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
        if self.timestamp.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "event timestamp must not be empty".to_string(),
            ));
        }
        if self.session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "event session_id must not be empty".to_string(),
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
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub timestamp: Timestamp,
    pub session_id: Id,
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
            event_type: event.event_type,
            source: event.source,
            timestamp: event.timestamp,
            session_id: event.session_id,
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
}
