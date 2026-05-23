use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::archive::{ArchiveEntry, ArchiveFilters, ArchiveStatus};
use crate::core_store::{
    CoreContextBudgetReport, CoreContextEvent, CoreContextFact, CoreContextPackage,
    CoreContextRequest, CoreContextTokenBudget, CoreFact, CoreFactInput, CoreFactPatchInput,
    CoreFactPatchResult, CoreFactStatus, CoreFactUpsertResult,
};
use crate::event::{IngestEvent, StoredEvent};
use crate::manifest::{FeatureFlags, Manifest, SchemaVersions};
use crate::recall::{
    RecallDebug, RecallFilters, RecallItem, RecallQuery, RecallResult, RecallSourceLayer,
};
use crate::session::SessionRecord;
use crate::sleep::SleepCompressionResult;
use crate::storage::Storage;
use crate::tasks::{PendingTask, TaskState, TaskType};
use crate::types::{
    ImportanceHint, ModelRole, Quote, RecallStage, TimeRange, WeightedFact,
    ARCHIVE_ENTRY_SCHEMA_VERSION, CANDIDATE_BELIEF_SCHEMA_VERSION,
    COMPACT_MEMORY_RESULT_SCHEMA_VERSION, CORE_CONTEXT_PACKAGE_SCHEMA_VERSION,
    CORE_CONTEXT_REQUEST_SCHEMA_VERSION, CORE_FACT_INPUT_SCHEMA_VERSION,
    CORE_FACT_PATCH_INPUT_SCHEMA_VERSION, CORE_FACT_PATCH_RESULT_SCHEMA_VERSION,
    CORE_FACT_SCHEMA_VERSION, CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION, CORE_STORE_SCHEMA_VERSION,
    EVENT_SCHEMA_VERSION, INGEST_RESULT_SCHEMA_VERSION, JOURNAL_OPERATION_SCHEMA_VERSION,
    MANIFEST_SCHEMA_VERSION, PENDING_TASK_SCHEMA_VERSION, RECALL_QUERY_SCHEMA_VERSION,
    RECALL_RESULT_SCHEMA_VERSION, SESSION_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct MemoryEngine<S> {
    storage: S,
    options: EngineOptions,
    manifest_initialized: bool,
}

impl<S> MemoryEngine<S> {
    pub fn new(storage: S) -> Self {
        Self::with_options(storage, EngineOptions::default())
    }

    pub fn with_options(storage: S, options: EngineOptions) -> Self {
        Self {
            storage,
            options,
            manifest_initialized: false,
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

impl<S: Storage> MemoryEngine<S> {
    pub fn ingest(&mut self, event: IngestEvent) -> Result<IngestResult> {
        validate_ingest_event(&event)?;
        self.ensure_manifest()?;

        let (initial_weight, weight_reason) = self.options.event_scoring.score_ingest_event(&event);
        let stored = StoredEvent::from_ingest(
            event,
            new_id("event")?,
            now_rfc3339()?,
            initial_weight,
            weight_reason,
        );

        self.storage.append_event(&stored.session_id, &stored)?;

        Ok(IngestResult {
            schema_version: INGEST_RESULT_SCHEMA_VERSION.to_string(),
            stored_event: stored,
        })
    }

    fn ensure_manifest(&mut self) -> Result<()> {
        if self.manifest_initialized {
            return Ok(());
        }
        if !self.storage.manifest_exists()? {
            let now = now_rfc3339()?;
            let manifest = default_manifest(&now);
            self.storage.write_manifest(&manifest)?;
        }
        self.manifest_initialized = true;
        Ok(())
    }

    pub fn pending_tasks(&self) -> Result<Vec<PendingTask>> {
        Ok(self
            .storage
            .load_tasks()?
            .into_iter()
            .filter(|task| matches!(task.state, TaskState::Pending | TaskState::Submitted))
            .collect())
    }

    fn archived_event_ids_for_session(&self, session_id: &str) -> Result<HashSet<String>> {
        Ok(self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .filter(|entry| entry.status == ArchiveStatus::Complete)
            .flat_map(|entry| entry.source_event_ids)
            .collect())
    }

    pub fn upsert_core_fact(&mut self, input: CoreFactInput) -> Result<CoreFactUpsertResult> {
        validate_core_fact_input(&input)?;
        self.ensure_manifest()?;

        let now = now_rfc3339()?;
        let category_name = normalize_whitespace(&input.category);
        let scope = normalize_optional_string(input.scope.as_deref());
        let fact_text = normalize_whitespace(&input.text);
        let mut category = self.storage.read_core_store_category(&category_name)?;

        if category.schema_version.trim().is_empty() {
            category.schema_version = CORE_STORE_SCHEMA_VERSION.to_string();
        }
        category.category = category_name.clone();
        category.updated_at = now.clone();

        let needle = normalize_match_text(&fact_text);
        let mut created = false;
        let fact = if let Some(existing) = category
            .facts
            .iter_mut()
            .find(|fact| normalize_match_text(&fact.text) == needle && fact.scope == scope)
        {
            existing.scope = scope.clone();
            existing.text = fact_text;
            existing.status = CoreFactStatus::Active;
            existing.confidence = existing.confidence.max(input.confidence).clamp(0.0, 1.0);
            existing.updated_at = now.clone();
            merge_unique(&mut existing.tags, &input.tags);
            merge_unique(&mut existing.source_archive_ids, &input.source_archive_ids);
            if existing.source_candidate_id.is_none() {
                existing.source_candidate_id = input.source_candidate_id.clone();
            }
            existing.clone()
        } else {
            created = true;
            let fact = CoreFact {
                schema_version: CORE_FACT_SCHEMA_VERSION.to_string(),
                core_fact_id: new_id("core_fact")?,
                scope,
                text: fact_text,
                status: CoreFactStatus::Active,
                confidence: input.confidence.clamp(0.0, 1.0),
                created_at: now.clone(),
                updated_at: now,
                source_archive_ids: input.source_archive_ids,
                source_candidate_id: input.source_candidate_id,
                tags: unique_strings(input.tags),
                links: Vec::new(),
                review: None,
            };
            category.facts.push(fact.clone());
            fact
        };

        self.storage.write_core_store_category(&category)?;
        Ok(CoreFactUpsertResult {
            schema_version: CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION.to_string(),
            category: category_name,
            created,
            fact,
        })
    }

    pub fn patch_core_fact(&mut self, input: CoreFactPatchInput) -> Result<CoreFactPatchResult> {
        validate_core_fact_patch_input(&input)?;
        self.ensure_manifest()?;

        let now = now_rfc3339()?;
        let scope = normalize_optional_string(input.scope.as_deref());
        let patch_text = input.text.as_deref().map(normalize_whitespace);
        let patch_tags = input.tags.map(unique_strings);

        for mut category in self.storage.read_core_store_categories()? {
            let Some(fact) = category
                .facts
                .iter_mut()
                .find(|fact| fact.core_fact_id == input.core_fact_id && fact.scope == scope)
            else {
                continue;
            };

            if let Some(text) = patch_text.as_ref() {
                fact.text = text.clone();
            }
            if let Some(status) = input.status {
                fact.status = status;
            }
            if let Some(confidence) = input.confidence {
                fact.confidence = confidence.clamp(0.0, 1.0);
            }
            if let Some(tags) = patch_tags.as_ref() {
                fact.tags = tags.clone();
            }
            fact.updated_at = now.clone();

            let patched_fact = fact.clone();
            category.updated_at = now;
            let category_name = category.category.clone();
            self.storage.write_core_store_category(&category)?;

            return Ok(CoreFactPatchResult {
                schema_version: CORE_FACT_PATCH_RESULT_SCHEMA_VERSION.to_string(),
                category: category_name,
                fact: patched_fact,
            });
        }

        Err(MemoryEngineError::Validation(format!(
            "core fact not found for requested scope: {}",
            input.core_fact_id
        )))
    }

    pub fn core_context_package(
        &mut self,
        request: CoreContextRequest,
    ) -> Result<CoreContextPackage> {
        validate_core_context_request(&request)?;
        self.ensure_manifest()?;

        let created_at = now_rfc3339()?;
        let session = self.storage.read_session(&request.session_id)?;
        let recent_limit = if request.session_recent_limit == 0 {
            self.options.context.default_session_recent_limit
        } else {
            request.session_recent_limit
        };
        let trace_limit = if request.session_trace_event_limit == 0 {
            self.options.context.default_session_trace_event_limit
        } else {
            request.session_trace_event_limit
        };
        let recall_limit = if request.recall_limit == 0 {
            self.options.recall.default_limit
        } else {
            request.recall_limit
        };

        let archived_event_ids =
            self.archived_event_ids_for_session(&session.metadata.session_id)?;
        let session_recent = session_context_events(&session, recent_limit, &archived_event_ids);
        let session_trace = session_context_events(&session, trace_limit, &archived_event_ids);
        let query_text = request
            .query_text
            .clone()
            .or_else(|| {
                request
                    .domain_state
                    .get("recent_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                request
                    .domain_state
                    .get("current_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });

        let archive_relevant = self
            .recall(RecallQuery {
                schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
                query_id: None,
                created_at: Some(created_at.clone()),
                session_id: Some(request.session_id.clone()),
                context: json!({ "recent_text": query_text.clone().unwrap_or_default() }),
                query_text,
                filters: RecallFilters {
                    source_layers: vec![RecallSourceLayer::Archive],
                    ..RecallFilters::default()
                },
                limit: recall_limit,
                include_core: false,
                explain: false,
            })?
            .items;

        let core_facts = if request.include_core {
            self.core_context_facts(request.core_scope.as_deref())?
        } else {
            Vec::new()
        };
        let mut notes = if request.include_core && core_facts.is_empty() {
            vec![
                "core_facts are empty; no stable Core Store facts have been saved yet.".to_string(),
            ]
        } else {
            Vec::new()
        };

        let budget_config = request
            .token_budget
            .unwrap_or(self.options.context.token_budget);
        let budgeted = apply_context_token_budget(
            core_facts,
            session_recent,
            session_trace,
            archive_relevant,
            &request.domain_state,
            budget_config,
        );
        notes.extend(budgeted.notes);

        Ok(CoreContextPackage {
            schema_version: CORE_CONTEXT_PACKAGE_SCHEMA_VERSION.to_string(),
            created_at,
            core_facts: budgeted.core_facts,
            session_recent: budgeted.session_recent,
            session_trace: budgeted.session_trace,
            archive_relevant: budgeted.archive_relevant,
            domain_state: request.domain_state,
            budget: Some(budgeted.report),
            notes,
        })
    }

    fn core_context_facts(&self, scope: Option<&str>) -> Result<Vec<CoreContextFact>> {
        let normalized_scope = normalize_optional_string(scope);
        let mut facts = Vec::new();
        for category in self.storage.read_core_store_categories()? {
            let fact_category = category.category.clone();
            for fact in category.facts {
                if fact.status != CoreFactStatus::Active {
                    continue;
                }
                if fact.scope != normalized_scope {
                    continue;
                }
                facts.push(CoreContextFact {
                    category: fact_category.clone(),
                    core_fact_id: fact.core_fact_id,
                    scope: fact.scope,
                    text: fact.text,
                    confidence: fact.confidence,
                    tags: fact.tags,
                });
            }
        }

        facts.sort_by(|left, right| {
            right
                .confidence
                .total_cmp(&left.confidence)
                .then_with(|| left.category.cmp(&right.category))
                .then_with(|| left.core_fact_id.cmp(&right.core_fact_id))
        });
        Ok(facts)
    }

    pub fn sleep(&mut self, session_id: &str) -> Result<SleepStage1Result> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        let session = self.storage.read_session(session_id)?;
        if session.events.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "session has no events: {session_id}"
            )));
        }

        let archived_event_ids =
            self.archived_event_ids_for_session(&session.metadata.session_id)?;
        let unarchived_events = session
            .events
            .iter()
            .filter(|event| !archived_event_ids.contains(&event.event_id))
            .collect::<Vec<_>>();
        if unarchived_events.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "session has no unarchived events: {session_id}"
            )));
        }

        let compactable_events = compactable_sleep_events(&unarchived_events, &self.options.sleep);
        let selected_events = select_sleep_events(&compactable_events, &self.options.sleep);
        let now = now_rfc3339()?;
        let archive_id = new_id("archive")?;
        let archive_entry =
            build_preliminary_archive(&session, &selected_events, &archive_id, &now);
        self.storage.write_archive_entry(&archive_entry)?;

        let pending_task = build_sleep_compression_task(
            &session,
            &selected_events,
            &archive_entry,
            &self.options.sleep,
            &now,
        )?;
        let compact_memory_task =
            build_compact_memory_task(&session, &selected_events, &archive_entry, &now)?;
        self.storage.save_task(&pending_task)?;
        self.storage.save_task(&compact_memory_task)?;

        Ok(SleepStage1Result {
            archive_entry,
            pending_task,
            compact_memory_task: Some(compact_memory_task),
        })
    }

    pub fn resume_sleep_compression(
        &mut self,
        task_id: &str,
        result: SleepCompressionResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;
        self.ensure_manifest()?;

        let mut task = self.storage.load_task(task_id)?;

        if task.task_type != TaskType::SleepCompression {
            return Err(MemoryEngineError::Validation(format!(
                "task is not sleep_compression: {task_id}"
            )));
        }

        let mut archive_entry = self.storage.read_archive_entry_by_id(&result.archive_id)?;

        let now = now_rfc3339()?;
        archive_entry.updated_at = now.clone();
        archive_entry.theme = result.theme;
        archive_entry.tags = result.tags;
        archive_entry.gist = result.gist;
        archive_entry.narrative = result.narrative;
        if result.compact_memory.is_some() {
            archive_entry.compact_memory = result.compact_memory;
        }
        archive_entry.facts = result.facts;
        archive_entry.quotes = result.quotes;
        archive_entry.weight = result.weight;
        archive_entry.links = result.links;
        archive_entry.emotional_markers = result.emotional_markers;
        archive_entry.topic_thread = result.topic_thread;
        archive_entry.personal_signals = result.personal_signals;
        archive_entry.relational_tone = result.relational_tone;
        archive_entry.status = ArchiveStatus::Complete;
        archive_entry.llm_enhanced = true;
        archive_entry.prompt_id = Some(task.prompt_id.clone());
        archive_entry.prompt_version = Some(task.prompt_version);

        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;

        Ok(archive_entry)
    }

    pub fn resume_compact_memory_pass(
        &mut self,
        task_id: &str,
        compact_memory: &str,
    ) -> Result<ArchiveEntry> {
        let compact_memory = normalize_optional_string(Some(compact_memory)).ok_or_else(|| {
            MemoryEngineError::Validation(
                "compact memory pass result must not be empty".to_string(),
            )
        })?;
        self.ensure_manifest()?;

        let mut task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::CompactMemoryPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not compact_memory_pass: {task_id}"
            )));
        }

        let archive_id = task
            .inputs
            .get("preliminary_archive_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "compact_memory_pass task has no preliminary_archive_id: {task_id}"
                ))
            })?;
        let mut archive_entry = self.storage.read_archive_entry_by_id(archive_id)?;

        let now = now_rfc3339()?;
        archive_entry.updated_at = now.clone();
        archive_entry.compact_memory = Some(compact_memory);
        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;

        Ok(archive_entry)
    }

    pub fn recall(&mut self, query: RecallQuery) -> Result<RecallResult> {
        validate_recall_query(&query)?;
        self.ensure_manifest()?;

        let created_at = query.created_at.clone().map_or_else(now_rfc3339, Ok)?;
        let archive_enabled = query.filters.source_layers.is_empty()
            || query
                .filters
                .source_layers
                .contains(&RecallSourceLayer::Archive);

        let mut archive_entries = if archive_enabled {
            self.storage
                .read_archive(&archive_filters_from_recall(&query.filters))?
                .into_iter()
                .filter(|entry| entry.status == ArchiveStatus::Complete)
                .filter(|entry| {
                    query
                        .session_id
                        .as_ref()
                        .is_none_or(|session_id| &entry.source_session_id == session_id)
                })
                .collect()
        } else {
            Vec::new()
        };

        let candidate_count = archive_entries.len();
        let mut scored_entries = archive_entries
            .drain(..)
            .map(|entry| {
                let scored = score_archive_entry(&entry, &query, &self.options.recall);
                (entry, scored)
            })
            .collect::<Vec<_>>();

        scored_entries.sort_by(|(left_entry, left_score), (right_entry, right_score)| {
            right_score
                .score
                .total_cmp(&left_score.score)
                .then_with(|| right_entry.weight.total_cmp(&left_entry.weight))
                .then_with(|| left_entry.archive_id.cmp(&right_entry.archive_id))
        });

        let limit = if query.limit == 0 {
            self.options.recall.default_limit
        } else {
            query.limit
        };

        let selected_entries = scored_entries.into_iter().take(limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(selected_entries.len());

        for (mut entry, score) in selected_entries {
            entry.recall_count += 1;
            entry.last_recalled_at = Some(created_at.clone());
            self.storage
                .update_archive_entry(&entry.archive_id, &entry)?;
            items.push(recall_item_from_archive(entry, score, query.explain));
        }

        Ok(RecallResult {
            schema_version: RECALL_RESULT_SCHEMA_VERSION.to_string(),
            query_id: query.query_id,
            created_at,
            stage_used: RecallStage::Stage1,
            items,
            debug: query.explain.then_some(RecallDebug {
                candidate_count,
                filtered_count: candidate_count,
            }),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineOptions {
    pub event_scoring: EventScoringConfig,
    pub sleep: SleepStage1Config,
    pub recall: RecallStage1Config,
    pub context: ContextPackageConfig,
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

impl Default for RecallStage1Config {
    fn default() -> Self {
        Self {
            default_limit: 5,
            theme_match_factor: 1.2,
            tag_overlap_bonus: 0.1,
            text_match_bonus: 0.5,
            no_text_match_factor: 0.7,
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
    pub compact_memory_task: Option<PendingTask>,
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

fn validate_ingest_event(event: &IngestEvent) -> Result<()> {
    if event.schema_version != EVENT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: EVENT_SCHEMA_VERSION.to_string(),
            actual: event.schema_version.clone(),
        });
    }

    if event.event_type.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event type must not be empty".to_string(),
        ));
    }

    if event.source.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event source must not be empty".to_string(),
        ));
    }

    if event.timestamp.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event timestamp must not be empty".to_string(),
        ));
    }

    if event.session_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event session_id must not be empty".to_string(),
        ));
    }

    Ok(())
}

fn validate_recall_query(query: &RecallQuery) -> Result<()> {
    if query.schema_version != RECALL_QUERY_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            actual: query.schema_version.clone(),
        });
    }

    Ok(())
}

fn validate_core_context_request(request: &CoreContextRequest) -> Result<()> {
    if request.schema_version != CORE_CONTEXT_REQUEST_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            actual: request.schema_version.clone(),
        });
    }

    if request.session_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core context request session_id must not be empty".to_string(),
        ));
    }

    Ok(())
}

fn validate_core_fact_input(input: &CoreFactInput) -> Result<()> {
    if input.schema_version != CORE_FACT_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            actual: input.schema_version.clone(),
        });
    }

    if input.category.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core fact category must not be empty".to_string(),
        ));
    }

    if input.text.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core fact text must not be empty".to_string(),
        ));
    }

    if !input.confidence.is_finite() {
        return Err(MemoryEngineError::Validation(
            "core fact confidence must be finite".to_string(),
        ));
    }

    Ok(())
}

fn validate_core_fact_patch_input(input: &CoreFactPatchInput) -> Result<()> {
    if input.schema_version != CORE_FACT_PATCH_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_FACT_PATCH_INPUT_SCHEMA_VERSION.to_string(),
            actual: input.schema_version.clone(),
        });
    }

    if input.core_fact_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core fact patch core_fact_id must not be empty".to_string(),
        ));
    }

    if input.text.is_none()
        && input.status.is_none()
        && input.confidence.is_none()
        && input.tags.is_none()
    {
        return Err(MemoryEngineError::Validation(
            "core fact patch must change at least one field".to_string(),
        ));
    }

    if input
        .text
        .as_deref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return Err(MemoryEngineError::Validation(
            "core fact patch text must not be empty".to_string(),
        ));
    }

    if input
        .confidence
        .is_some_and(|confidence| !confidence.is_finite())
    {
        return Err(MemoryEngineError::Validation(
            "core fact patch confidence must be finite".to_string(),
        ));
    }

    Ok(())
}

fn session_context_events(
    session: &SessionRecord,
    limit: usize,
    archived_event_ids: &HashSet<String>,
) -> Vec<CoreContextEvent> {
    if limit == 0 {
        return Vec::new();
    }

    let mut events = session
        .events
        .iter()
        .rev()
        .filter(|event| !archived_event_ids.contains(&event.event_id))
        .take(limit)
        .collect::<Vec<_>>();
    events.reverse();

    events
        .into_iter()
        .map(|event| CoreContextEvent {
            event_id: event.event_id.clone(),
            timestamp: event.timestamp.clone(),
            event_type: event.event_type.clone(),
            source: event.source.clone(),
            text: event_text(event),
            tags: event.tags.clone(),
            theme: event.theme.clone(),
        })
        .collect()
}

struct BudgetedContextPackage {
    core_facts: Vec<CoreContextFact>,
    session_recent: Vec<CoreContextEvent>,
    session_trace: Vec<CoreContextEvent>,
    archive_relevant: Vec<RecallItem>,
    report: CoreContextBudgetReport,
    notes: Vec<String>,
}

fn apply_context_token_budget(
    core_facts: Vec<CoreContextFact>,
    session_recent: Vec<CoreContextEvent>,
    session_trace: Vec<CoreContextEvent>,
    archive_relevant: Vec<RecallItem>,
    domain_state: &Value,
    budget: CoreContextTokenBudget,
) -> BudgetedContextPackage {
    let estimated_domain_state_tokens = estimate_json_tokens(domain_state);

    let (core_facts, estimated_core_tokens, dropped_core_facts) =
        keep_front_within_budget(core_facts, budget.core_tokens);

    let current_memory_budget = budget
        .current_memory_tokens
        .saturating_sub(estimated_domain_state_tokens);
    let (session_recent, estimated_session_recent_tokens, dropped_session_recent) =
        keep_recent_within_budget(session_recent, current_memory_budget);
    let remaining_current_budget =
        current_memory_budget.saturating_sub(estimated_session_recent_tokens);
    let (session_trace, estimated_session_trace_tokens, dropped_session_trace) =
        keep_recent_within_budget(session_trace, remaining_current_budget);

    let (archive_relevant, estimated_compressed_memory_tokens, dropped_archive_relevant) =
        keep_front_within_budget(archive_relevant, budget.compressed_memory_tokens);

    let estimated_current_memory_tokens = estimated_domain_state_tokens
        + estimated_session_recent_tokens
        + estimated_session_trace_tokens;
    let estimated_total_tokens = estimated_current_memory_tokens
        + estimated_compressed_memory_tokens
        + estimated_core_tokens;

    let budget_exceeded = estimated_total_tokens > budget.total_tokens
        || estimated_current_memory_tokens > budget.current_memory_tokens
        || estimated_compressed_memory_tokens > budget.compressed_memory_tokens
        || estimated_core_tokens > budget.core_tokens;

    let mut notes = Vec::new();
    if estimated_domain_state_tokens > budget.current_memory_tokens {
        notes.push(format!(
            "domain_state alone exceeds current-memory budget: estimated {estimated_domain_state_tokens} tokens > budget {}.",
            budget.current_memory_tokens
        ));
    }
    if dropped_session_recent > 0 {
        notes.push(format!(
            "token budget dropped {dropped_session_recent} session_recent event(s); newest events were kept."
        ));
    }
    if dropped_session_trace > 0 {
        notes.push(format!(
            "token budget dropped {dropped_session_trace} session_trace event(s); newest events were kept."
        ));
    }
    if dropped_archive_relevant > 0 {
        notes.push(format!(
            "token budget dropped {dropped_archive_relevant} archive_relevant item(s); highest-ranked recall items were kept."
        ));
    }
    if dropped_core_facts > 0 {
        notes.push(format!(
            "token budget dropped {dropped_core_facts} core fact(s); highest-confidence facts were kept."
        ));
    }
    if budget_exceeded {
        notes.push(
            "token budget is still exceeded after trimming; inspect domain_state/current turn size."
                .to_string(),
        );
    }

    BudgetedContextPackage {
        core_facts,
        session_recent,
        session_trace,
        archive_relevant,
        report: CoreContextBudgetReport {
            estimator: "unicode_chars_div_2_ceil_json_v1".to_string(),
            total_budget_tokens: budget.total_tokens,
            current_memory_budget_tokens: budget.current_memory_tokens,
            compressed_memory_budget_tokens: budget.compressed_memory_tokens,
            core_budget_tokens: budget.core_tokens,
            estimated_total_tokens,
            estimated_current_memory_tokens,
            estimated_compressed_memory_tokens,
            estimated_core_tokens,
            estimated_domain_state_tokens,
            dropped_session_recent,
            dropped_session_trace,
            dropped_archive_relevant,
            dropped_core_facts,
            budget_exceeded,
        },
        notes,
    }
}

fn keep_front_within_budget<T: Clone + Serialize>(
    items: Vec<T>,
    budget: usize,
) -> (Vec<T>, usize, usize) {
    let original_len = items.len();
    let mut kept = Vec::new();
    let mut used = 0usize;

    for item in items {
        let estimate = estimate_json_tokens(&item);
        if used + estimate <= budget {
            used += estimate;
            kept.push(item);
        }
    }

    let dropped = original_len.saturating_sub(kept.len());
    (kept, used, dropped)
}

fn keep_recent_within_budget<T: Clone + Serialize>(
    items: Vec<T>,
    budget: usize,
) -> (Vec<T>, usize, usize) {
    let original_len = items.len();
    let mut kept_reversed = Vec::new();
    let mut used = 0usize;

    for item in items.into_iter().rev() {
        let estimate = estimate_json_tokens(&item);
        if used + estimate <= budget {
            used += estimate;
            kept_reversed.push(item);
        }
    }
    kept_reversed.reverse();

    let dropped = original_len.saturating_sub(kept_reversed.len());
    (kept_reversed, used, dropped)
}

fn estimate_json_tokens<T: Serialize>(value: &T) -> usize {
    serde_json::to_string(value)
        .map(|text| estimate_text_tokens(&text))
        .unwrap_or(0)
}

fn estimate_text_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(2)
    }
}

fn compactable_sleep_events<'a>(
    events: &[&'a StoredEvent],
    config: &SleepStage1Config,
) -> Vec<&'a StoredEvent> {
    let tail_count = active_tail_event_count(events.len(), config);
    let compactable_len = events.len().saturating_sub(tail_count);
    events[..compactable_len].to_vec()
}

fn active_tail_event_count(total_events: usize, config: &SleepStage1Config) -> usize {
    if total_events <= 1 || total_events < config.partial_sleep_min_events {
        return 0;
    }

    if !config.active_tail_ratio.is_finite() || config.active_tail_ratio <= 0.0 {
        return 0;
    }

    let ratio = config.active_tail_ratio.min(0.95);
    let tail_count = ((total_events as f64) * ratio).ceil() as usize;
    tail_count.min(total_events.saturating_sub(1))
}

fn select_sleep_events<'a>(
    events: &[&'a StoredEvent],
    config: &SleepStage1Config,
) -> Vec<&'a StoredEvent> {
    let mut selected = events
        .iter()
        .copied()
        .filter(|event| {
            event.initial_weight >= config.min_event_weight
                || matches!(
                    event.importance_hint,
                    ImportanceHint::High | ImportanceHint::Critical
                )
        })
        .collect::<Vec<_>>();

    if selected.is_empty() {
        selected = events.to_vec();
    }

    selected.sort_by(|left, right| {
        right
            .initial_weight
            .total_cmp(&left.initial_weight)
            .then_with(|| right.timestamp.cmp(&left.timestamp))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    selected.truncate(config.max_events);
    selected.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    selected
}

fn build_preliminary_archive(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_id: &str,
    now: &str,
) -> ArchiveEntry {
    let tags = collect_tags(events);
    let theme = session
        .metadata
        .active_theme
        .clone()
        .or_else(|| events.iter().find_map(|event| event.theme.clone()));
    let quotes = events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| Quote {
                text: truncate_chars(&text, 240),
                source_event_id: Some(event.event_id.clone()),
            })
        })
        .collect::<Vec<_>>();

    ArchiveEntry {
        schema_version: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
        archive_id: archive_id.to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        source_session_id: session.metadata.session_id.clone(),
        source_event_ids: events.iter().map(|event| event.event_id.clone()).collect(),
        time_range: time_range_from_events(events),
        theme,
        tags,
        gist: preliminary_gist(events),
        narrative: preliminary_narrative(&session.metadata.session_id, events),
        compact_memory: None,
        facts: preliminary_facts(events),
        quotes,
        weight: archive_weight(events),
        freshness: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        links: events
            .iter()
            .flat_map(|event| event.links.iter().cloned())
            .collect(),
        emotional_markers: Vec::new(),
        topic_thread: Vec::new(),
        personal_signals: Vec::new(),
        relational_tone: None,
        status: ArchiveStatus::Preliminary,
        llm_enhanced: false,
        prompt_id: None,
        prompt_version: None,
        embedding_model_id: None,
        embedding: None,
    }
}

fn build_sleep_compression_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    config: &SleepStage1Config,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::SleepCompression,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: config.prompt_id.clone(),
        prompt_version: config.prompt_version,
        role_hint: ModelRole::Balanced,
        expected_output_schema: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": &session.metadata.session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "compact_human_readable_memory",
                "preserve_quotes": true,
                "do_not_invent_facts": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

fn build_compact_memory_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::CompactMemoryPass,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: "compact_memory_pass".to_string(),
        prompt_version: 1,
        role_hint: ModelRole::Balanced,
        expected_output_schema: COMPACT_MEMORY_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": &session.metadata.session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "short_human_memory_theses",
                "plain_text_only": true,
                "do_not_invent_facts": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

fn archive_filters_from_recall(filters: &RecallFilters) -> ArchiveFilters {
    ArchiveFilters {
        time_range: filters.time_range.clone(),
        tags: filters.tags.clone(),
        theme: filters.theme.clone(),
        min_weight: filters.min_weight,
        min_freshness: filters.min_freshness,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ScoredArchiveEntry {
    score: f64,
    explanation: String,
}

fn score_archive_entry(
    entry: &ArchiveEntry,
    query: &RecallQuery,
    config: &RecallStage1Config,
) -> ScoredArchiveEntry {
    let theme_factor = if query.filters.theme.is_some() && query.filters.theme == entry.theme {
        config.theme_match_factor
    } else {
        1.0
    };

    let tag_overlap = query
        .filters
        .tags
        .iter()
        .filter(|tag| entry.tags.iter().any(|entry_tag| entry_tag == *tag))
        .count();
    let tag_factor = 1.0 + (tag_overlap as f64 * config.tag_overlap_bonus);

    let query_tokens = query_tokens(query);
    let searchable_tokens = archive_tokens(entry);
    let text_overlap = query_tokens
        .iter()
        .filter(|token| searchable_tokens.contains(*token))
        .count();
    let text_factor = if query_tokens.is_empty() {
        1.0
    } else if text_overlap == 0 {
        config.no_text_match_factor
    } else {
        1.0 + ((text_overlap as f64 / query_tokens.len() as f64) * config.text_match_bonus)
    };

    let score =
        (entry.weight * entry.freshness * theme_factor * tag_factor * text_factor).clamp(0.0, 1.0);

    ScoredArchiveEntry {
        score,
        explanation: format!(
            "weight {:.2} * freshness {:.2} * theme {:.2} * tags {:.2} * text {:.2}",
            entry.weight, entry.freshness, theme_factor, tag_factor, text_factor
        ),
    }
}

fn recall_item_from_archive(
    entry: ArchiveEntry,
    scored: ScoredArchiveEntry,
    explain: bool,
) -> RecallItem {
    let compact_memory = normalize_optional_string(entry.compact_memory.as_deref());
    let prompt_gist = compact_memory.clone().unwrap_or_else(|| entry.gist.clone());
    let narrative = if compact_memory.is_some() {
        None
    } else {
        Some(entry.narrative)
    };
    let facts = if compact_memory.is_some() {
        Vec::new()
    } else {
        entry.facts.into_iter().map(|fact| fact.text).collect()
    };
    let quotes = if compact_memory.is_some() {
        Vec::new()
    } else {
        entry.quotes.into_iter().map(|quote| quote.text).collect()
    };

    RecallItem {
        source_layer: RecallSourceLayer::Archive,
        id: entry.archive_id,
        gist: prompt_gist,
        compact_memory,
        narrative,
        facts,
        quotes,
        source_session_id: Some(entry.source_session_id),
        time_range: Some(entry.time_range),
        tags: entry.tags,
        theme: entry.theme,
        weight: entry.weight,
        freshness: entry.freshness,
        relevance_score: scored.score,
        relevance_explanation: explain.then_some(scored.explanation),
    }
}

fn time_range_from_events(events: &[&StoredEvent]) -> TimeRange {
    let start = events
        .iter()
        .map(|event| event.timestamp.as_str())
        .min()
        .unwrap_or("unknown")
        .to_string();
    let end = events
        .iter()
        .map(|event| event.timestamp.as_str())
        .max()
        .unwrap_or("unknown")
        .to_string();

    TimeRange { start, end }
}

fn collect_tags(events: &[&StoredEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| event.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn preliminary_gist(events: &[&StoredEvent]) -> String {
    let texts = events
        .iter()
        .filter_map(|event| event_text(event))
        .take(3)
        .map(|text| truncate_chars(&text, 120))
        .collect::<Vec<_>>();

    if texts.is_empty() {
        format!("Попередній спогад із {} події(й).", events.len())
    } else {
        format!("Попередній спогад: {}.", texts.join(" / "))
    }
}

fn preliminary_narrative(session_id: &str, events: &[&StoredEvent]) -> String {
    let lines = events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| {
                format!(
                    "{} {}: {}",
                    event.timestamp,
                    event.event_type,
                    truncate_chars(&text, 180)
                )
            })
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        format!(
            "Попередній архівний спогад із сесії {session_id}, створений алгоритмічно з {} події(й).",
            events.len()
        )
    } else {
        format!(
            "Попередній архівний спогад із сесії {session_id}. Ключові події: {}",
            lines.join(" | ")
        )
    }
}

fn preliminary_facts(events: &[&StoredEvent]) -> Vec<WeightedFact> {
    events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| WeightedFact {
                text: truncate_chars(&text, 240),
                confidence: event.initial_weight.clamp(0.0, 1.0),
                source_event_ids: vec![event.event_id.clone()],
            })
        })
        .collect()
}

fn archive_weight(events: &[&StoredEvent]) -> f64 {
    events
        .iter()
        .map(|event| event.initial_weight)
        .fold(0.0, f64::max)
        .clamp(0.0, 1.0)
}

fn event_text(event: &StoredEvent) -> Option<String> {
    event
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn query_tokens(query: &RecallQuery) -> BTreeSet<String> {
    let mut text = String::new();

    if let Some(query_text) = &query.query_text {
        text.push_str(query_text);
        text.push(' ');
    }

    if let Some(recent_text) = query
        .context
        .get("recent_text")
        .and_then(|value| value.as_str())
    {
        text.push_str(recent_text);
    }

    tokenize(&text)
}

fn archive_tokens(entry: &ArchiveEntry) -> BTreeSet<String> {
    let mut text = String::new();
    if let Some(compact_memory) = &entry.compact_memory {
        text.push_str(compact_memory);
        text.push(' ');
    }
    text.push_str(&entry.gist);
    text.push(' ');
    text.push_str(&entry.narrative);
    text.push(' ');

    for fact in &entry.facts {
        text.push_str(&fact.text);
        text.push(' ');
    }

    for quote in &entry.quotes {
        text.push_str(&quote.text);
        text.push(' ');
    }

    for tag in &entry.tags {
        text.push_str(tag);
        text.push(' ');
    }

    if let Some(theme) = &entry.theme {
        text.push_str(theme);
    }

    tokenize(&text)
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .map(|token| token.trim().to_lowercase())
        .filter(|token| token.chars().count() >= 2)
        .collect()
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_match_text(text: &str) -> String {
    normalize_whitespace(text).to_lowercase()
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_whitespace)
        .filter(|value| !value.is_empty())
}

fn unique_strings(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|item| normalize_whitespace(&item))
        .filter(|item| !item.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn merge_unique(target: &mut Vec<String>, source: &[String]) {
    let mut seen = target.iter().cloned().collect::<BTreeSet<_>>();
    for item in source {
        let normalized = normalize_whitespace(item);
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        target.push(normalized);
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn new_id(prefix: &str) -> Result<String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| {
            MemoryEngineError::Storage(format!("system clock before unix epoch: {err}"))
        })?
        .as_nanos();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{prefix}_{nanos}_{counter}"))
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| MemoryEngineError::Storage(format!("failed to format timestamp: {err}")))
}

fn default_manifest(now: &str) -> Manifest {
    Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        storage_id: "default".to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        schema_versions: SchemaVersions {
            event: EVENT_SCHEMA_VERSION.to_string(),
            session: SESSION_SCHEMA_VERSION.to_string(),
            archive_entry: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
            core_store: CORE_STORE_SCHEMA_VERSION.to_string(),
            core_fact: CORE_FACT_SCHEMA_VERSION.to_string(),
            candidate_belief: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            pending_task: PENDING_TASK_SCHEMA_VERSION.to_string(),
            journal_operation: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
        },
        active_embedding_model_id: None,
        last_migration_at: None,
        features: FeatureFlags {
            recall_stage: RecallStage::Stage1,
            embeddings_enabled: false,
            llm_recall_rerank_enabled: false,
            reflection_enabled: false,
        },
    }
}
