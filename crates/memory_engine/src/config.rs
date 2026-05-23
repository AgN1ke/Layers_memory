use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    pub memory_dir: String,
    #[serde(default)]
    pub limits: EngineLimits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineLimits {
    pub default_recall_limit: usize,
    pub max_session_events: usize,
    pub weight_floor_critical: f64,
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self {
            default_recall_limit: 5,
            max_session_events: 1_000,
            weight_floor_critical: 0.95,
        }
    }
}
