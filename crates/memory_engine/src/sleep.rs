use serde::{Deserialize, Serialize};

use crate::archive::{EmotionalMarker, PersonalSignal, RelationalTone, TopicThreadItem};
use crate::types::{Id, Link, Quote, WeightedFact};

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
