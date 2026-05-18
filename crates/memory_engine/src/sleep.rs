use serde::{Deserialize, Serialize};

use crate::types::{Id, Link, Quote, WeightedFact};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepCompressionResult {
    pub schema_version: String,
    pub archive_id: Id,
    pub gist: String,
    pub narrative: String,
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

        if !(0.0..=1.0).contains(&self.weight) {
            return Err(crate::MemoryEngineError::Validation(
                "sleep compression weight must be between 0.0 and 1.0".to_string(),
            ));
        }

        Ok(())
    }
}
