use std::collections::{BTreeSet, HashMap, HashSet};
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::archive::{
    ArchiveEntry, ArchiveFilters, ArchiveStatus, FidelityReview, FidelityStatus, ForgetDecision,
    ForgetReviewRecord, MemoryUnit, MemoryUnitStatus,
};
use crate::core_store::{
    CandidateBelief, CandidateReviewInput, CandidateReviewResult, CandidateStatus,
    CoreContextBudgetReport, CoreContextEvent, CoreContextFact, CoreContextPackage,
    CoreContextRequest, CoreContextTokenBudget, CoreFact, CoreFactInput, CoreFactPatchInput,
    CoreFactPatchResult, CoreFactStatus, CoreFactUpsertResult, PromotionChecks, ReviewDecision,
    ReviewRecord,
};
use crate::event::{IngestEvent, StoredEvent};
use crate::fidelity::{EvidenceEvent, EvidenceEventRole, EvidencePack, MemoryFidelityPassStart};
use crate::forgetting::{
    ForgetRecommendation, ForgetReviewApplyResult, ForgetReviewCandidate, ForgetReviewInputs,
    ForgetReviewResult, ForgetReviewStart, ForgottenMemoryUnits,
};
use crate::llm::{
    CoreArchiveSeedSummary, CoreSignalSummary, LlmBatch, LlmRequest, LlmResponse, SleepOutcome,
    SleepRequestState, SleepRun, SleepRunStage, SleepRunStep, SleepTrack,
};
use crate::manifest::{FeatureFlags, Manifest, SchemaVersions};
use crate::prompt_view::{
    render_archive_memory_prompt_lines, render_context_event_prompt_line,
    render_core_fact_prompt_line, TimeLabelContext, ARCHIVE_MEMORY_PROMPT_LIMIT,
    OLDER_TRACE_MAX_TEXT_CHARS, RECENT_MAX_TEXT_CHARS,
};
use crate::recall::{
    RecallDebug, RecallFilters, RecallItem, RecallQuery, RecallResult, RecallSourceLayer,
};
use crate::reflection::{
    ReflectionAnalyzeResult, ReflectionCandidateDraft, ReflectionCandidatesResult,
    ReflectionPassStart,
};
use crate::session::SessionRecord;
use crate::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use crate::storage::{Storage, StorageReadWarning};
use crate::tasks::{PendingTask, TaskState, TaskType};
use crate::types::{
    ImportanceHint, Link, ModelRole, Quote, RecallStage, TimeRange, WeightedFact,
    ARCHIVE_ENTRY_SCHEMA_VERSION, CANDIDATE_BELIEF_SCHEMA_VERSION,
    CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION, CANDIDATE_REVIEW_RESULT_SCHEMA_VERSION,
    COMPACT_MEMORY_RESULT_SCHEMA_VERSION, CONSOLIDATOR_TEXT_SCHEMA_VERSION,
    CORE_CONTEXT_PACKAGE_SCHEMA_VERSION, CORE_CONTEXT_REQUEST_SCHEMA_VERSION,
    CORE_FACT_INPUT_SCHEMA_VERSION, CORE_FACT_PATCH_INPUT_SCHEMA_VERSION,
    CORE_FACT_PATCH_RESULT_SCHEMA_VERSION, CORE_FACT_SCHEMA_VERSION,
    CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION, CORE_STORE_SCHEMA_VERSION, EVENT_SCHEMA_VERSION,
    FIDELITY_REVIEW_SCHEMA_VERSION, FORGET_REVIEW_INPUT_SCHEMA_VERSION,
    FORGET_REVIEW_RESULT_SCHEMA_VERSION, INGEST_RESULT_SCHEMA_VERSION,
    JOURNAL_OPERATION_SCHEMA_VERSION, MANIFEST_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION,
    MEMORY_UNIT_SCHEMA_VERSION, PENDING_TASK_SCHEMA_VERSION, RECALL_QUERY_SCHEMA_VERSION,
    RECALL_RESULT_SCHEMA_VERSION, REFLECTION_RESULT_SCHEMA_VERSION, SESSION_SCHEMA_VERSION,
    SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use crate::vector::{
    default_vector_manifest, memory_unit_is_vector_eligible, normalize_vector, thesis_hash,
    DeepRecallHit, DeepRecallQuery, DeepRecallResult, EmbedBatchInputs, EmbedBatchItem,
    EmbedBatchResult, VectorAppendRecord, VectorIndexData, VectorRow, VectorScopeState,
    VectorScopeStatus, VectorTombstone, DEEP_RECALL_RESULT_SCHEMA_VERSION, DEFAULT_VECTOR_DIM,
    DEFAULT_VECTOR_MODEL_ID, EMBED_BATCH_RESULT_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

mod context_budget;
mod core_context;
mod fidelity_flow;
mod forgetting_flow;
mod options;
mod recall_api;
mod recall_stage1;
mod reflection_flow;
mod session_ops;
mod sleep_driver;
mod sleep_flow;
mod validation;
mod vector_flow;

use context_budget::*;
use options::ForgetApplyAction;
pub use options::{
    ContextPackageConfig, EngineOptions, EventScoringConfig, FidelityConfig, ForgetConfig,
    IngestResult, RecallStage1Config, SleepStage1Config, SleepStage1Result, VectorConfig,
};
use recall_api::*;
use recall_stage1::*;
use sleep_driver::*;
use sleep_flow::session_is_multi_speaker;
use validation::*;
const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
const SLEEP_RUN_SCHEMA_VERSION: &str = "sleep_run.v1";
const DEFAULT_SLEEP_PASS_MAX_ATTEMPTS: u32 = 3;
const CONSOLIDATOR_GIST_REJECTED_MARKER: &str = "consolidator_gist_rejected";

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct MemoryEngine<S> {
    storage: S,
    options: EngineOptions,
    manifest_initialized: AtomicBool,
    locks: LockRegistry,
    recall_stats: Mutex<HashMap<String, RecallStatDelta>>,
    recall_stats_flush_lock: Mutex<()>,
    recall_calls_since_flush: AtomicU64,
}

#[derive(Debug, Default)]
struct LockRegistry {
    resources: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

#[derive(Debug, Clone, Default)]
struct RecallStatDelta {
    added_count: u64,
    last_recalled_at: Option<String>,
}

impl LockRegistry {
    fn resource(&self, key: &str) -> Result<Arc<Mutex<()>>> {
        let mut resources = self.resources.lock().map_err(|_| {
            MemoryEngineError::Storage("lock registry mutex was poisoned".to_string())
        })?;
        Ok(resources
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
}

impl<S> MemoryEngine<S> {
    pub fn new(storage: S) -> Self {
        Self::with_options(storage, EngineOptions::default())
    }

    pub fn with_options(storage: S, options: EngineOptions) -> Self {
        Self {
            storage,
            options,
            manifest_initialized: AtomicBool::new(false),
            locks: LockRegistry::default(),
            recall_stats: Mutex::new(HashMap::new()),
            recall_stats_flush_lock: Mutex::new(()),
            recall_calls_since_flush: AtomicU64::new(0),
        }
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    pub fn into_storage(self) -> S {
        self.storage
    }
}

impl<S: Storage> MemoryEngine<S> {}

#[cfg(test)]
mod tests {
    use super::{
        meaningful_tokens, neutral_narrative_from_tracks, parse_consolidator_text,
        preliminary_gist, preliminary_narrative,
    };

    #[test]
    fn parse_consolidator_text_accepts_plain_gist_and_narrative() {
        let (gist, narrative) = parse_consolidator_text(
            "GIST: User corrected the assistant about quasars.\n\nThe user stayed engaged and pushed back on an astronomy explanation.",
        )
        .expect("plain consolidator text should parse");

        assert_eq!(gist, "User corrected the assistant about quasars.");
        assert_eq!(
            narrative,
            "The user stayed engaged and pushed back on an astronomy explanation."
        );
    }

    #[test]
    fn parse_consolidator_text_unwraps_json_shaped_response() {
        let (gist, narrative) = parse_consolidator_text(
            r#"{
  "gist": "GIST: User shared stable personal context.",
  "narrative": "The user said they live in Kyiv and were born in 1989."
}"#,
        )
        .expect("json-shaped consolidator text should be unwrapped");

        assert_eq!(gist, "User shared stable personal context.");
        assert_eq!(
            narrative,
            "The user said they live in Kyiv and were born in 1989."
        );
    }

    #[test]
    fn parse_consolidator_text_unwraps_json_string_response() {
        let (gist, narrative) = parse_consolidator_text(
            r#""GIST: User challenged an astronomy explanation.\n\nThe user kept testing the assistant's claim about quasars and galactic dust.""#,
        )
        .expect("quoted consolidator text should be decoded and parsed");

        assert_eq!(gist, "User challenged an astronomy explanation.");
        assert_eq!(
            narrative,
            "The user kept testing the assistant's claim about quasars and galactic dust."
        );
    }

    #[test]
    fn parse_consolidator_text_rejects_structural_gist() {
        let error = parse_consolidator_text(
            "GIST: {\"gist\":\"still structured\"}\n\nThe narrative itself is readable.",
        )
        .expect_err("structured gist should be rejected");

        assert!(error.to_string().contains("consolidator_gist_rejected"));
    }

    #[test]
    fn ukrainian_stop_words_filter_common_user_tokens() {
        let tokens = meaningful_tokens("Користувач дуже любить космос і цікавиться Європою");

        assert!(!tokens.contains("користувач"));
        assert!(!tokens.contains("дуже"));
        assert!(!tokens.contains("любить"));
        assert!(!tokens.contains("цікавиться"));
        assert!(tokens.contains("космос"));
        assert!(tokens.contains("європою"));
    }

    #[test]
    fn fallback_ukrainian_literals_are_valid_text() {
        let gist = preliminary_gist(&[]);
        let narrative = preliminary_narrative("session_a", &[]);
        let neutral = neutral_narrative_from_tracks(&[], &[], &[]);

        assert_eq!(gist, "Попередній спогад із 0 події(й).");
        assert!(narrative.contains("Попередній архівний спогад"));
        assert!(neutral.contains("Сесія була стиснута"));
        assert!(!gist.contains('Р'));
        assert!(!narrative.contains('Р'));
        assert!(!neutral.contains('Р'));
    }
}
