use serde::{Deserialize, Serialize};

use crate::types::{Id, Link, Quote, Speaker, TimeRange, Timestamp, WeightedFact};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveStatus {
    Preliminary,
    Complete,
    Superseded,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryUnitStatus {
    ActiveArchive,
    CoreCandidate,
    Forgotten,
    Rejected,
    NeedsRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgetDecision {
    Forget,
    Keep,
    Protect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityStatus {
    Unchecked,
    SelfChecked,
    Valid,
    TooBroad,
    Unsupported,
    Distorted,
    MissingKeyDetail,
    NeedsRevision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub schema_version: String,
    pub archive_id: Id,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_scope_id: Option<Id>,
    pub source_session_id: Id,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    pub time_range: TimeRange,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub gist: String,
    pub narrative: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_memory: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_units: Vec<MemoryUnit>,
    #[serde(default)]
    pub facts: Vec<WeightedFact>,
    #[serde(default)]
    pub quotes: Vec<Quote>,
    pub weight: f64,
    pub freshness: f64,
    pub recall_count: u64,
    #[serde(default)]
    pub last_recalled_at: Option<Timestamp>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emotional_markers: Vec<EmotionalMarker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_thread: Vec<TopicThreadItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personal_signals: Vec<PersonalSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relational_tone: Option<RelationalTone>,
    pub status: ArchiveStatus,
    pub llm_enhanced: bool,
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(default)]
    pub prompt_version: Option<u32>,
    #[serde(default)]
    pub embedding_model_id: Option<String>,
    #[serde(default)]
    pub embedding: Option<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryUnit {
    pub schema_version: String,
    pub memory_unit_id: Id,
    pub archive_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_scope_id: Option<Id>,
    pub source_session_id: Id,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub thesis: String,
    /// Canonical person this unit describes when the engine can prove
    /// attribution from the cited source events. `None` preserves legacy and
    /// non-personal memory semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub weight: f64,
    pub status: MemoryUnitStatus,
    pub fidelity_status: FidelityStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fidelity_review: Option<FidelityReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forget_review: Option<ForgetReviewRecord>,
    #[serde(default)]
    pub reflection_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reflected_at: Option<Timestamp>,
    /// Idempotency marker for the latest reflection checkpoint operation.
    /// Additive in memory_unit.v1 so older records remain readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reflection_operation_id: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalSignalSubjectHint {
    SelfStatement,
    AboutOther,
}

impl ArchiveEntry {
    pub fn effective_personality_scope_id(&self) -> &str {
        self.personality_scope_id
            .as_deref()
            .unwrap_or(&self.source_session_id)
    }
}

impl MemoryUnit {
    pub fn effective_personality_scope_id(&self) -> &str {
        self.personality_scope_id
            .as_deref()
            .unwrap_or(&self.source_session_id)
    }
}

/// One policy for every prompt-facing Archive path. Tombstoned, rejected or
/// revision-pending units remain auditable but cannot leak through their
/// parent Archive's cached gist/compact text.
pub fn memory_unit_is_prompt_eligible(unit: &MemoryUnit) -> bool {
    matches!(
        unit.status,
        MemoryUnitStatus::ActiveArchive | MemoryUnitStatus::CoreCandidate
    ) && !matches!(
        unit.fidelity_status,
        FidelityStatus::Unsupported | FidelityStatus::Distorted | FidelityStatus::NeedsRevision
    )
}

pub fn archive_has_prompt_eligible_memory(entry: &ArchiveEntry) -> bool {
    entry.memory_units.is_empty()
        || entry
            .memory_units
            .iter()
            .any(memory_unit_is_prompt_eligible)
}

pub(crate) fn render_compact_memory_from_units(units: &[MemoryUnit]) -> Option<String> {
    let lines = units
        .iter()
        .filter(|unit| {
            unit.status == MemoryUnitStatus::ActiveArchive && memory_unit_is_prompt_eligible(unit)
        })
        .map(|unit| unit.thesis.trim())
        .filter(|thesis| !thesis.is_empty())
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionalMarker {
    pub target: String,
    pub affect: String,
    pub strength: f64,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicThreadItem {
    pub topic: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtopics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalSignal {
    pub text: String,
    pub category: String,
    pub confidence: f64,
    /// Canonical subject derived by the engine from the cited user events.
    /// Model output is never trusted to populate this field directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Speaker>,
    /// Semantic intent proposed by the model. The engine verifies it against
    /// source-event authorship before automatic Core promotion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hint: Option<PersonalSignalSubjectHint>,
    /// Stable speaker id proposed by the model. A mismatch with the sole
    /// source speaker fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationalTone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warmth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intellectual_engagement: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intimacy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playfulness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FidelityReview {
    pub schema_version: String,
    pub memory_unit_id: Id,
    pub archive_id: Id,
    pub status: FidelityStatus,
    pub confidence: f64,
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_thesis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgetReviewRecord {
    pub reviewed_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    pub decision: ForgetDecision,
    pub reason: String,
}

impl FidelityReview {
    pub fn validate_basic(&self) -> crate::Result<()> {
        if self.memory_unit_id.trim().is_empty() {
            return Err(crate::MemoryEngineError::Validation(
                "fidelity review memory_unit_id must not be empty".to_string(),
            ));
        }
        if self.archive_id.trim().is_empty() {
            return Err(crate::MemoryEngineError::Validation(
                "fidelity review archive_id must not be empty".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(crate::MemoryEngineError::Validation(
                "fidelity review confidence must be between 0.0 and 1.0".to_string(),
            ));
        }
        if self.explanation.trim().is_empty() {
            return Err(crate::MemoryEngineError::Validation(
                "fidelity review explanation must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_freshness: Option<f64>,
}
