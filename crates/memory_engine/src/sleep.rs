use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

use crate::archive::{
    EmotionalMarker, PersonalSignal, PersonalSignalSubjectHint, RelationalTone, TopicThreadItem,
};
use crate::event::StoredEvent;
use crate::llm::LlmSharedContext;
use crate::types::{
    Id, ImportanceHint, Link, ProcessingMode, Quote, Speaker, Timestamp, WeightedFact,
    SLEEP_PROMPT_CONTEXT_SCHEMA_VERSION,
};

pub const SLEEP_INPUT_EVENT_SCHEMA_VERSION: &str = "sleep_input_event.v1";
pub const SLEEP_INPUT_REFERENCE_SCHEMA_VERSION: &str = "sleep_input_reference.v1";

/// Renders the provider-facing Sleep source once per run. The durable
/// `SleepInputEvent` remains lossless; this projection contains the fields the
/// reasoning prompts use and places batch constants in one header.
pub fn render_sleep_prompt_context(
    session_id: &str,
    archive_id: &str,
    events: &[SleepInputEvent],
) -> LlmSharedContext {
    let scope = events
        .iter()
        .find_map(|event| event.personality_scope_id.as_deref())
        .unwrap_or(session_id);
    let mut lines = vec![
        SLEEP_PROMPT_CONTEXT_SCHEMA_VERSION.to_string(),
        format!("session={}", compact_prompt_atom(session_id)),
        format!("archive={}", compact_prompt_atom(archive_id)),
        format!("scope={}", compact_prompt_atom(scope)),
        "events:".to_string(),
    ];
    let mut source_text_chars = 0usize;

    for event in events {
        let source_text = event
            .memory_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| compact_payload_text(&event.payload));
        source_text_chars += source_text.chars().count();

        let mut metadata = vec![
            compact_prompt_atom(&event.event_id),
            compact_prompt_atom(&event.timestamp),
            compact_prompt_atom(&event.event_type),
            compact_prompt_atom(&event.source),
        ];
        if let Some(speaker) = &event.speaker {
            metadata.push(format!(
                "speaker={}:{}",
                compact_prompt_atom(&speaker.id),
                compact_prompt_atom(&speaker.name)
            ));
        }
        metadata.push(format!("w={:.2}", event.initial_weight.clamp(0.0, 1.0)));
        if !event.tags.is_empty() {
            metadata.push(format!(
                "tags={}",
                event
                    .tags
                    .iter()
                    .map(|tag| compact_prompt_atom(tag))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if let Some(theme) = event
            .theme
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            metadata.push(format!("theme={}", compact_prompt_atom(theme)));
        }
        if let Some(tone) = event
            .emotional_tone
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            metadata.push(format!("tone={}", compact_prompt_atom(tone)));
        }
        for link in &event.links {
            if matches!(link.kind.as_str(), "reply" | "reply_to") {
                metadata.push(format!("reply_to={}", compact_prompt_atom(&link.target)));
            }
        }

        lines.push(format!(
            "[{}] {}",
            metadata.join("|"),
            compact_prompt_text(&source_text)
        ));
    }

    let content = lines.join("\n");
    let rendered_chars = content.chars().count();
    LlmSharedContext {
        schema_version: SLEEP_PROMPT_CONTEXT_SCHEMA_VERSION.to_string(),
        context_id: format!("sleep:{archive_id}"),
        content,
        source_event_ids: events.iter().map(|event| event.event_id.clone()).collect(),
        event_count: events.len(),
        source_text_chars,
        rendered_chars,
        estimated_tokens: rendered_chars.div_ceil(2),
    }
}

fn compact_prompt_atom(value: &str) -> String {
    value
        .trim()
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(']', "\\]")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn compact_prompt_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_payload_text(payload: &Value) -> String {
    payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(payload).unwrap_or_else(|_| "null".to_string()))
}

/// Durable pointer used by tasks and SleepRun. The private event projection is
/// owned by one immutable workflow-input record instead of being copied into
/// every task and LLM request stored by the server runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SleepInputReference {
    pub schema_version: String,
    pub record_id: Id,
    pub content_digest: String,
}

impl SleepInputReference {
    pub fn validate(&self) -> crate::Result<()> {
        if self.schema_version != SLEEP_INPUT_REFERENCE_SCHEMA_VERSION {
            return Err(crate::MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_INPUT_REFERENCE_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        if self.record_id.trim().is_empty()
            || !self.content_digest.starts_with("sha256:")
            || self.content_digest.len() != "sha256:".len() + 64
        {
            return Err(crate::MemoryEngineError::Validation(
                "sleep input reference requires record_id and sha256 digest".to_string(),
            ));
        }
        Ok(())
    }
}

/// Immutable cognitive projection of one raw event. Sleep tasks use this
/// shared shape instead of maintaining several lossy, hand-written JSON maps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepInputEvent {
    pub schema_version: String,
    pub event_id: Id,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub timestamp: Timestamp,
    pub received_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_sequence: Option<u64>,
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
    pub importance_hint: ImportanceHint,
    pub processing_mode: ProcessingMode,
    pub initial_weight: f64,
    pub weight_reason: String,
}

impl From<&StoredEvent> for SleepInputEvent {
    fn from(event: &StoredEvent) -> Self {
        Self {
            schema_version: SLEEP_INPUT_EVENT_SCHEMA_VERSION.to_string(),
            event_id: event.event_id.clone(),
            event_type: event.event_type.clone(),
            source: event.source.clone(),
            timestamp: event.timestamp.clone(),
            received_at: event.received_at.clone(),
            admission_sequence: event.admission_sequence,
            personality_scope_id: event.personality_scope_id.clone(),
            session_id: event.session_id.clone(),
            memory_text: event.memory_text.clone(),
            payload: event.payload.clone(),
            tags: event.tags.clone(),
            theme: event.theme.clone(),
            emotional_tone: event.emotional_tone.clone(),
            speaker: event.speaker.clone(),
            links: event.links.clone(),
            importance_hint: event.importance_hint,
            processing_mode: event.processing_mode,
            initial_weight: event.initial_weight,
            weight_reason: event.weight_reason.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepCompressionResult {
    pub schema_version: String,
    pub archive_id: Id,
    pub gist: String,
    pub narrative: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_memory: Option<String>,
    #[serde(default)]
    pub facts: Vec<WeightedFact>,
    #[serde(default)]
    pub quotes: Vec<Quote>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub theme: Option<String>,
    pub weight: f64,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub emotional_markers: Vec<EmotionalMarker>,
    #[serde(default)]
    pub topic_thread: Vec<TopicThreadItem>,
    #[serde(default)]
    pub personal_signals: Vec<PersonalSignal>,
    #[serde(default)]
    pub relational_tone: Option<RelationalTone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryUnitPassResult {
    pub schema_version: String,
    pub archive_id: Id,
    #[serde(default)]
    pub memory_units: Vec<MemoryUnitDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryUnitDraft {
    pub thesis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hint: Option<PersonalSignalSubjectHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionEvent {
    pub event_id: Id,
    pub event_type: String,
    pub speaker: Option<Speaker>,
}

impl From<&StoredEvent> for AttributionEvent {
    fn from(event: &StoredEvent) -> Self {
        Self {
            event_id: event.event_id.clone(),
            event_type: event.event_type.clone(),
            speaker: event.speaker.clone(),
        }
    }
}

impl From<&SleepInputEvent> for AttributionEvent {
    fn from(event: &SleepInputEvent) -> Self {
        Self {
            event_id: event.event_id.clone(),
            event_type: event.event_type.clone(),
            speaker: event.speaker.clone(),
        }
    }
}

pub fn attribute_personal_signals(signals: &mut [PersonalSignal], events: &[AttributionEvent]) {
    for signal in signals {
        signal.subject = resolve_attributed_subject(
            &signal.source_event_ids,
            signal.subject_hint,
            signal.speaker_id.as_deref(),
            events,
        );
    }
}

pub fn resolve_attributed_subject(
    source_event_ids: &[Id],
    subject_hint: Option<PersonalSignalSubjectHint>,
    speaker_id_hint: Option<&str>,
    events: &[AttributionEvent],
) -> Option<Speaker> {
    let participants = participant_speakers(events);
    let source_speakers = source_user_speakers(source_event_ids, events);
    let normalized_hint = speaker_id_hint.map(str::trim).filter(|id| !id.is_empty());

    match subject_hint {
        Some(PersonalSignalSubjectHint::SelfStatement) => {
            let source = sole_speaker(&source_speakers)?;
            if normalized_hint.is_some_and(|id| id != source.id) {
                return None;
            }
            Some(source)
        }
        Some(PersonalSignalSubjectHint::AboutOther) => {
            normalized_hint.and_then(|id| participants.get(id).cloned())
        }
        None => {
            // Legacy model output remains promotable only while the session
            // contains at most one identified human speaker.
            (participants.len() <= 1)
                .then(|| sole_speaker(&source_speakers))
                .flatten()
        }
    }
}

pub fn personal_signal_is_auto_core_eligible(
    signal: &PersonalSignal,
    events: &[AttributionEvent],
) -> bool {
    if !personal_signal_has_user_source(signal, events) {
        return false;
    }

    let participants = participant_speakers(events);
    if participants.is_empty() {
        return true;
    }

    match signal.subject_hint {
        Some(PersonalSignalSubjectHint::SelfStatement) => {
            let sources = source_user_speakers(&signal.source_event_ids, events);
            let Some(source) = sole_speaker(&sources) else {
                return false;
            };
            signal
                .subject
                .as_ref()
                .is_some_and(|subject| subject.id == source.id)
                && signal
                    .speaker_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .is_none_or(|id| id == source.id)
        }
        Some(PersonalSignalSubjectHint::AboutOther) => false,
        None => participants.len() <= 1,
    }
}

pub fn personal_signal_has_user_source(
    signal: &PersonalSignal,
    events: &[AttributionEvent],
) -> bool {
    let source_user_ids = events
        .iter()
        .filter(|event| event.event_type == "user_message")
        .map(|event| event.event_id.as_str())
        .collect::<HashSet<_>>();
    signal
        .source_event_ids
        .iter()
        .any(|event_id| source_user_ids.contains(event_id.as_str()))
}

pub fn attribution_session_is_multi_speaker(events: &[AttributionEvent]) -> bool {
    participant_speakers(events).len() >= 2
}

fn participant_speakers(events: &[AttributionEvent]) -> BTreeMap<String, Speaker> {
    events
        .iter()
        .filter(|event| event.event_type == "user_message")
        .filter_map(|event| normalized_speaker(event.speaker.as_ref()))
        .map(|speaker| (speaker.id.clone(), speaker))
        .collect()
}

fn source_user_speakers(
    source_event_ids: &[Id],
    events: &[AttributionEvent],
) -> BTreeMap<String, Speaker> {
    let source_ids = source_event_ids.iter().collect::<HashSet<_>>();
    events
        .iter()
        .filter(|event| event.event_type == "user_message")
        .filter(|event| source_ids.contains(&event.event_id))
        .filter_map(|event| normalized_speaker(event.speaker.as_ref()))
        .map(|speaker| (speaker.id.clone(), speaker))
        .collect()
}

fn sole_speaker(speakers: &BTreeMap<String, Speaker>) -> Option<Speaker> {
    (speakers.len() == 1)
        .then(|| speakers.values().next().cloned())
        .flatten()
}

fn normalized_speaker(speaker: Option<&Speaker>) -> Option<Speaker> {
    speaker.and_then(|speaker| {
        let id = speaker.id.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = speaker
            .name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        (!id.is_empty() && !name.is_empty()).then_some(Speaker { id, name })
    })
}

impl MemoryUnitPassResult {
    pub fn validate_basic(&self) -> crate::Result<()> {
        for unit in &self.memory_units {
            if unit.thesis.trim().is_empty() {
                return Err(crate::MemoryEngineError::Validation(
                    "memory unit thesis must not be empty".to_string(),
                ));
            }

            if !(0.0..=1.0).contains(&unit.weight) {
                return Err(crate::MemoryEngineError::Validation(
                    "memory unit weight must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Proves that model-produced provenance points only at events in the
    /// immutable task input. Empty provenance remains readable for legacy
    /// results, but invented/cross-task IDs fail closed.
    pub fn validate_source_event_ids(&self, allowed_event_ids: &HashSet<Id>) -> crate::Result<()> {
        for unit in &self.memory_units {
            for event_id in &unit.source_event_ids {
                if !allowed_event_ids.contains(event_id) {
                    return Err(crate::MemoryEngineError::Validation(format!(
                        "memory unit cites event outside frozen sleep input: {event_id}"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl SleepCompressionResult {
    pub fn validate_basic(&self) -> crate::Result<()> {
        if self.gist.trim().is_empty() {
            return Err(crate::MemoryEngineError::Validation(
                "sleep compression gist must not be empty".to_string(),
            ));
        }

        if self.narrative.trim().is_empty() {
            return Err(crate::MemoryEngineError::Validation(
                "sleep compression narrative must not be empty".to_string(),
            ));
        }

        if let Some(compact_memory) = &self.compact_memory {
            if compact_memory.trim().is_empty() {
                return Err(crate::MemoryEngineError::Validation(
                    "sleep compression compact_memory must not be empty when provided".to_string(),
                ));
            }
        }

        if !(0.0..=1.0).contains(&self.weight) {
            return Err(crate::MemoryEngineError::Validation(
                "sleep compression weight must be between 0.0 and 1.0".to_string(),
            ));
        }

        for marker in &self.emotional_markers {
            if !(0.0..=1.0).contains(&marker.strength) {
                return Err(crate::MemoryEngineError::Validation(
                    "emotional marker strength must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        for signal in &self.personal_signals {
            if !(0.0..=1.0).contains(&signal.confidence) {
                return Err(crate::MemoryEngineError::Validation(
                    "personal signal confidence must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        if let Some(tone) = &self.relational_tone {
            for value in [
                tone.warmth,
                tone.intellectual_engagement,
                tone.intimacy,
                tone.trust,
                tone.playfulness,
                tone.tension,
            ]
            .into_iter()
            .flatten()
            {
                if !(0.0..=1.0).contains(&value) {
                    return Err(crate::MemoryEngineError::Validation(
                        "relational tone values must be between 0.0 and 1.0".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }
}
