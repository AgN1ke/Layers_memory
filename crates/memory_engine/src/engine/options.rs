use super::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineOptions {
    pub event_scoring: EventScoringConfig,
    pub sleep: SleepStage1Config,
    pub recall: RecallStage1Config,
    pub context: ContextPackageConfig,
    pub fidelity: FidelityConfig,
    pub forgetting: ForgetConfig,
    pub vectors: VectorConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventScoringConfig {
    pub base_weight: f64,
    pub tag_bonus: f64,
    pub theme_bonus: f64,
    pub link_bonus: f64,
    pub medium_floor: f64,
    pub high_floor: f64,
    pub critical_floor: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SleepStage1Config {
    pub min_event_weight: f64,
    pub max_events: usize,
    pub active_tail_ratio: f64,
    pub partial_sleep_min_events: usize,
    pub prompt_id: String,
    pub prompt_version: u32,
}

impl Default for SleepStage1Config {
    fn default() -> Self {
        Self {
            min_event_weight: 0.55,
            max_events: 80,
            active_tail_ratio: 0.30,
            partial_sleep_min_events: 10,
            prompt_id: "sleep_compression".to_string(),
            prompt_version: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecallStage1Config {
    pub default_limit: usize,
    pub theme_match_factor: f64,
    pub tag_overlap_bonus: f64,
    pub text_match_bonus: f64,
    pub no_text_match_factor: f64,
    pub freshness_half_life_days: f64,
    pub recall_count_log_bonus: f64,
    pub recent_recall_bonus: f64,
    pub recent_recall_half_life_days: f64,
    pub max_recall_boost_factor: f64,
    pub stats_flush_interval: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextPackageConfig {
    pub default_session_recent_limit: usize,
    pub default_session_trace_event_limit: usize,
    /// Legacy seed list kept for compatibility with older host configs.
    /// Core context now reads every category file in Core Store, because
    /// v0.1 uses free normalized categories produced by LLM memory passes.
    pub core_categories: Vec<String>,
    pub token_budget: CoreContextTokenBudget,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FidelityConfig {
    pub neighbor_events: usize,
    pub max_evidence_tokens: usize,
    pub max_event_text_chars: usize,
    pub prompt_id: String,
    pub prompt_version: u32,
    pub auto_validate_after_sleep: bool,
    pub auto_validate_weight_threshold: f64,
    pub auto_validate_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForgetConfig {
    pub min_age_days: f64,
    pub forget_weight_threshold: f64,
    pub forget_recall_count_max: u64,
    pub max_review_batch: usize,
    pub protect_weight: f64,
    pub protect_recall_window_days: f64,
    pub protect_emotional_strength: f64,
    pub prompt_id: String,
    pub prompt_version: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorConfig {
    pub model_id: String,
    pub dim: usize,
    pub embed_batch_size: usize,
    pub deep_recall_default_top_k: usize,
    pub deep_recall_min_sim: f32,
    pub deep_recall_recency_weight: f32,
    pub deep_recall_unit_weight: f32,
}

impl Default for RecallStage1Config {
    fn default() -> Self {
        Self {
            default_limit: 5,
            theme_match_factor: 1.2,
            tag_overlap_bonus: 0.1,
            text_match_bonus: 0.5,
            no_text_match_factor: 0.7,
            freshness_half_life_days: 180.0,
            recall_count_log_bonus: 0.04,
            recent_recall_bonus: 0.10,
            recent_recall_half_life_days: 30.0,
            max_recall_boost_factor: 1.25,
            stats_flush_interval: 100,
        }
    }
}

impl Default for ContextPackageConfig {
    fn default() -> Self {
        Self {
            default_session_recent_limit: 40,
            default_session_trace_event_limit: 120,
            core_categories: vec![
                "profile".to_string(),
                "preferences".to_string(),
                "relationship".to_string(),
            ],
            token_budget: CoreContextTokenBudget::default(),
        }
    }
}

impl Default for FidelityConfig {
    fn default() -> Self {
        Self {
            neighbor_events: 2,
            max_evidence_tokens: 1_500,
            max_event_text_chars: 800,
            prompt_id: "memory_fidelity_pass".to_string(),
            prompt_version: 1,
            auto_validate_after_sleep: true,
            auto_validate_weight_threshold: 0.85,
            auto_validate_tags: [
                "identity",
                "profile",
                "personal",
                "personal_fact",
                "relationship",
                "preference",
                "pet",
                "family",
                "health",
                "location",
                "biography",
                "values",
                "core_candidate",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

impl Default for ForgetConfig {
    fn default() -> Self {
        Self {
            min_age_days: 30.0,
            forget_weight_threshold: 0.4,
            forget_recall_count_max: 1,
            max_review_batch: 40,
            protect_weight: 0.85,
            protect_recall_window_days: 30.0,
            protect_emotional_strength: 0.85,
            prompt_id: "forget_review_pass".to_string(),
            prompt_version: 1,
        }
    }
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
            dim: DEFAULT_VECTOR_DIM,
            embed_batch_size: 64,
            deep_recall_default_top_k: 5,
            deep_recall_min_sim: 0.75,
            deep_recall_recency_weight: 0.10,
            deep_recall_unit_weight: 0.10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestResult {
    pub schema_version: String,
    pub stored_event: StoredEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepStage1Result {
    pub archive_entry: ArchiveEntry,
    pub pending_task: PendingTask,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_unit_task: Option<PendingTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_memory_task: Option<PendingTask>,
}

pub(super) enum ForgetApplyAction {
    Forgotten(MemoryUnit),
    Kept(MemoryUnit),
    Protected(MemoryUnit),
    Ignored,
}

impl Default for EventScoringConfig {
    fn default() -> Self {
        Self {
            base_weight: 0.4,
            tag_bonus: 0.05,
            theme_bonus: 0.1,
            link_bonus: 0.05,
            medium_floor: 0.55,
            high_floor: 0.75,
            critical_floor: 0.95,
        }
    }
}

impl EventScoringConfig {
    pub fn score_ingest_event(&self, event: &IngestEvent) -> (f64, String) {
        let mut weight = self.base_weight;
        let mut reasons = vec![format!("base {:.2}", self.base_weight)];

        if !event.tags.is_empty() {
            let tag_bonus = self.tag_bonus * event.tags.len() as f64;
            weight += tag_bonus;
            reasons.push(format!("{} tag(s) +{tag_bonus:.2}", event.tags.len()));
        }

        if event.theme.is_some() {
            weight += self.theme_bonus;
            reasons.push(format!("theme +{:.2}", self.theme_bonus));
        }

        if !event.links.is_empty() {
            let link_bonus = self.link_bonus * event.links.len() as f64;
            weight += link_bonus;
            reasons.push(format!("{} link(s) +{link_bonus:.2}", event.links.len()));
        }

        let floor = match event.importance_hint {
            ImportanceHint::Low | ImportanceHint::Normal => None,
            ImportanceHint::Medium => Some(("medium importance floor", self.medium_floor)),
            ImportanceHint::High => Some(("high importance floor", self.high_floor)),
            ImportanceHint::Critical => Some(("critical importance floor", self.critical_floor)),
        };

        if let Some((label, floor)) = floor {
            if weight < floor {
                weight = floor;
                reasons.push(format!("{label} {floor:.2}"));
            }
        }

        (weight.clamp(0.0, 1.0), reasons.join("; "))
    }
}
