use std::collections::{BTreeSet, HashMap, HashSet};
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
    render_core_fact_prompt_line, ARCHIVE_MEMORY_PROMPT_LIMIT, OLDER_TRACE_MAX_TEXT_CHARS,
    RECENT_MAX_TEXT_CHARS,
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
use crate::storage::Storage;
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
use crate::{MemoryEngineError, Result};

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
}

#[derive(Debug, Default)]
struct LockRegistry {
    resources: Mutex<HashMap<String, Arc<Mutex<()>>>>,
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
    fn with_resource_lock<T, F>(&self, key: String, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T>,
    {
        let resource = self.locks.resource(&key)?;
        let _guard = lock_resource(&resource, &key)?;
        f()
    }

    pub fn ingest(&self, event: IngestEvent) -> Result<IngestResult> {
        validate_ingest_event(&event)?;
        self.ensure_manifest()?;

        let session_id = event.session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            let (initial_weight, weight_reason) =
                self.options.event_scoring.score_ingest_event(&event);
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
        })
    }

    fn ensure_manifest(&self) -> Result<()> {
        if self.manifest_initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        self.with_resource_lock("manifest".to_string(), || {
            if self.manifest_initialized.load(Ordering::Acquire) {
                return Ok(());
            }
            if !self.storage.manifest_exists()? {
                let now = now_rfc3339()?;
                let manifest = default_manifest(&now);
                self.storage.write_manifest(&manifest)?;
            }
            self.manifest_initialized.store(true, Ordering::Release);
            Ok(())
        })
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

    pub fn upsert_core_fact(&self, input: CoreFactInput) -> Result<CoreFactUpsertResult> {
        validate_core_fact_input(&input)?;
        self.ensure_manifest()?;

        let category_name = normalize_whitespace(&input.category);
        self.with_resource_lock(core_lock_key(&category_name), || {
            self.upsert_core_fact_unlocked(input, category_name)
        })
    }

    fn upsert_core_fact_unlocked(
        &self,
        input: CoreFactInput,
        category_name: String,
    ) -> Result<CoreFactUpsertResult> {
        let now = now_rfc3339()?;
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

    pub fn patch_core_fact(&self, input: CoreFactPatchInput) -> Result<CoreFactPatchResult> {
        validate_core_fact_patch_input(&input)?;
        self.ensure_manifest()?;

        let now = now_rfc3339()?;
        let scope = normalize_optional_string(input.scope.as_deref());
        let patch_text = input.text.as_deref().map(normalize_whitespace);
        let patch_tags = input.tags.map(unique_strings);

        let category_name = self
            .storage
            .read_core_store_categories()?
            .into_iter()
            .find(|category| {
                category
                    .facts
                    .iter()
                    .any(|fact| fact.core_fact_id == input.core_fact_id && fact.scope == scope)
            })
            .map(|category| category.category)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "core fact not found for requested scope: {}",
                    input.core_fact_id
                ))
            })?;

        self.with_resource_lock(core_lock_key(&category_name), || {
            let mut category = self.storage.read_core_store_category(&category_name)?;
            let Some(fact) = category
                .facts
                .iter_mut()
                .find(|fact| fact.core_fact_id == input.core_fact_id && fact.scope == scope)
            else {
                return Err(MemoryEngineError::Validation(format!(
                    "core fact not found for requested scope: {}",
                    input.core_fact_id
                )));
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

            Ok(CoreFactPatchResult {
                schema_version: CORE_FACT_PATCH_RESULT_SCHEMA_VERSION.to_string(),
                category: category_name,
                fact: patched_fact,
            })
        })
    }

    pub fn core_context_package(&self, request: CoreContextRequest) -> Result<CoreContextPackage> {
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
        let query_text_for_core_ranking = query_text.clone();

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
        let core_facts =
            rank_core_facts_for_query(core_facts, query_text_for_core_ranking.as_deref());
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
                if !core_fact_visible_in_context(fact.status) {
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
                    status: fact.status,
                    confidence: fact.confidence,
                    tags: fact.tags,
                });
            }
        }

        facts.sort_by(|left, right| {
            core_context_status_rank(left.status)
                .cmp(&core_context_status_rank(right.status))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| left.category.cmp(&right.category))
                .then_with(|| left.core_fact_id.cmp(&right.core_fact_id))
        });
        Ok(facts)
    }

    pub fn sleep(&self, session_id: &str) -> Result<SleepStage1Result> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
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

            let compactable_events =
                compactable_sleep_events(&unarchived_events, &self.options.sleep);
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
            let memory_unit_task =
                build_memory_unit_task(&session, &selected_events, &archive_entry, &now)?;
            self.storage.save_task(&pending_task)?;
            self.storage.save_task(&memory_unit_task)?;

            Ok(SleepStage1Result {
                archive_entry,
                pending_task,
                memory_unit_task: Some(memory_unit_task),
                compact_memory_task: None,
            })
        })
    }

    pub fn begin_sleep_run(&self, session_id: &str) -> Result<SleepRun> {
        let sleep_result = self.sleep(session_id)?;
        sleep_run_from_stage1(sleep_result)
    }

    pub fn next_sleep_batch(&self, mut run: SleepRun) -> Result<SleepRunStep> {
        validate_sleep_run(&run)?;
        advance_sleep_run_stage(&mut run)?;

        let requests = run
            .requests
            .iter()
            .filter(|state| !state.completed && state_stage(state.track) == run.stage)
            .map(|state| state.request.clone())
            .collect::<Vec<_>>();

        let batch = (!requests.is_empty()).then_some(LlmBatch { requests });
        Ok(SleepRunStep { run, batch })
    }

    pub fn submit_sleep_batch(
        &self,
        mut run: SleepRun,
        responses: Vec<LlmResponse>,
    ) -> Result<SleepRunStep> {
        validate_sleep_run(&run)?;

        for response in responses {
            let request_id = llm_response_request_id(&response).to_string();
            let Some(index) = run
                .requests
                .iter()
                .position(|state| state.request.request_id == request_id)
            else {
                return Err(MemoryEngineError::Validation(format!(
                    "LLM response does not match any request in sleep run: {request_id}"
                )));
            };

            let mut state = run.requests[index].clone();
            if state.completed {
                continue;
            }
            state.attempts += 1;
            handle_sleep_response(&mut run, &mut state, response)?;
            run.requests[index] = state;
        }

        self.next_sleep_batch(run)
    }

    pub fn finish_sleep_run(&self, mut run: SleepRun) -> Result<SleepOutcome> {
        validate_sleep_run(&run)?;
        advance_sleep_run_stage(&mut run)?;
        if run.stage != SleepRunStage::ReadyToFinish {
            return Err(MemoryEngineError::Validation(
                "sleep run is not ready to finish".to_string(),
            ));
        }

        let session_id = run.session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            let mut sleep_result = assemble_sleep_compression_from_tracks(&run)?;
            if let Some(gist) = run
                .consolidator_gist
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sleep_result.gist = gist.to_string();
            }
            if let Some(narrative) = run
                .consolidator_narrative
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sleep_result.narrative = narrative.to_string();
            }
            apply_sleep_run_tags(&mut sleep_result, &run);

            let mut archive_entry =
                self.resume_sleep_compression(&run.sleep_task_id, sleep_result)?;

            if let Some(memory_unit_task_id) = run.memory_unit_task_id.clone() {
                let memory_unit_result = run
                    .memory_unit_result
                    .clone()
                    .map(serde_json::from_value::<MemoryUnitPassResult>)
                    .transpose()?
                    .unwrap_or_else(|| empty_memory_unit_result(&run.archive_id));
                archive_entry =
                    self.resume_memory_unit_pass(&memory_unit_task_id, memory_unit_result)?;
            }

            let core_summary = self.apply_archive_personal_signal_bridge(&archive_entry)?;
            let fidelity_requests = self.auto_route_memory_fidelity_requests(&archive_entry)?;
            run.stage = SleepRunStage::Finished;

            Ok(SleepOutcome {
                archive_entry,
                core_summary,
                fidelity_requests,
                failed_passes: run.failed_passes,
                completion_mode: run
                    .completion_mode
                    .unwrap_or_else(|| "consolidated".to_string()),
            })
        })
    }

    pub fn seed_core_from_archives(&self) -> Result<CoreArchiveSeedSummary> {
        self.ensure_manifest()?;
        let mut summary = CoreArchiveSeedSummary::default();
        let archives = self.storage.read_archive(&ArchiveFilters::default())?;
        for archive in archives
            .into_iter()
            .filter(|archive| archive.status == ArchiveStatus::Complete)
        {
            summary.archives += 1;
            let archive_summary = self
                .with_resource_lock(session_lock_key(&archive.source_session_id), || {
                    self.apply_archive_personal_signal_bridge(&archive)
                })?;
            summary.created += archive_summary.created;
            summary.updated += archive_summary.updated;
            summary.skipped += archive_summary.skipped;
        }
        Ok(summary)
    }

    fn apply_archive_personal_signal_bridge(
        &self,
        archive: &ArchiveEntry,
    ) -> Result<CoreSignalSummary> {
        let mut summary = CoreSignalSummary::default();
        if archive.status != ArchiveStatus::Complete {
            return Ok(summary);
        }

        let session = self.storage.read_session(&archive.source_session_id)?;
        let user_event_ids = session
            .events
            .iter()
            .filter(|event| event.event_type == "user_message")
            .map(|event| event.event_id.clone())
            .collect::<HashSet<_>>();
        let scope = Some(archive.source_session_id.as_str());

        for signal in &archive.personal_signals {
            let text = normalize_whitespace(&signal.text);
            let category = normalize_category_name(&signal.category);
            let has_user_source = !user_event_ids.is_empty()
                && signal
                    .source_event_ids
                    .iter()
                    .any(|event_id| user_event_ids.contains(event_id));

            if text.is_empty()
                || category.is_empty()
                || signal.confidence < 0.85
                || !has_user_source
            {
                summary.skipped += 1;
                continue;
            }

            let result = self.with_resource_lock(core_lock_key(&category), || {
                if self.is_near_duplicate_core_fact(&category, scope, &text)? {
                    return Ok(None);
                }
                self.upsert_core_fact_unlocked(
                    CoreFactInput {
                        schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                        category: category.clone(),
                        scope: scope.map(str::to_string),
                        text,
                        confidence: signal.confidence,
                        tags: vec![
                            "archive_signal".to_string(),
                            "signal_category".to_string(),
                            format!("signal_category:{category}"),
                        ],
                        source_archive_ids: vec![archive.archive_id.clone()],
                        source_candidate_id: None,
                    },
                    category.clone(),
                )
                .map(Some)
            })?;
            let Some(result) = result else {
                summary.skipped += 1;
                continue;
            };
            if result.created {
                summary.created += 1;
            } else {
                summary.updated += 1;
            }
        }

        Ok(summary)
    }

    fn auto_route_memory_fidelity_requests(
        &self,
        archive: &ArchiveEntry,
    ) -> Result<Vec<LlmRequest>> {
        if !self.options.fidelity.auto_validate_after_sleep
            || archive.status != ArchiveStatus::Complete
            || archive.memory_units.is_empty()
        {
            return Ok(Vec::new());
        }

        let core_path_event_ids = self.core_path_signal_event_ids(archive)?;
        let mut requests = Vec::new();
        for unit in &archive.memory_units {
            if !self.should_auto_validate_memory_unit(unit, &core_path_event_ids) {
                continue;
            }
            if self.pending_fidelity_task_exists_unlocked(&unit.memory_unit_id)? {
                continue;
            }
            let start = self.begin_memory_fidelity_pass_unlocked(&unit.memory_unit_id)?;
            requests.push(start.request);
        }
        Ok(requests)
    }

    fn should_auto_validate_memory_unit(
        &self,
        unit: &MemoryUnit,
        core_path_event_ids: &HashSet<String>,
    ) -> bool {
        if unit.status != MemoryUnitStatus::ActiveArchive
            || unit.fidelity_status != FidelityStatus::Unchecked
        {
            return false;
        }

        if unit.weight >= self.options.fidelity.auto_validate_weight_threshold {
            return true;
        }

        if unit
            .source_event_ids
            .iter()
            .any(|id| core_path_event_ids.contains(id))
        {
            return true;
        }

        unit.tags.iter().any(|tag| {
            let tag = normalize_category_name(tag);
            self.options
                .fidelity
                .auto_validate_tags
                .iter()
                .any(|configured| configured == &tag)
        })
    }

    fn core_path_signal_event_ids(&self, archive: &ArchiveEntry) -> Result<HashSet<String>> {
        if archive.personal_signals.is_empty() {
            return Ok(HashSet::new());
        }
        let session = self.storage.read_session(&archive.source_session_id)?;
        let user_event_ids = session
            .events
            .iter()
            .filter(|event| event.event_type == "user_message")
            .map(|event| event.event_id.clone())
            .collect::<HashSet<_>>();
        if user_event_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let mut ids = HashSet::new();
        for signal in &archive.personal_signals {
            let text = normalize_whitespace(&signal.text);
            let category = normalize_category_name(&signal.category);
            let has_user_source = signal
                .source_event_ids
                .iter()
                .any(|event_id| user_event_ids.contains(event_id));
            if text.is_empty()
                || category.is_empty()
                || signal.confidence < 0.85
                || !has_user_source
            {
                continue;
            }
            ids.extend(signal.source_event_ids.iter().cloned());
        }
        Ok(ids)
    }

    fn pending_fidelity_task_exists_unlocked(&self, memory_unit_id: &str) -> Result<bool> {
        let tasks = self.storage.load_tasks()?;
        Ok(tasks.into_iter().any(|task| {
            task.task_type == TaskType::MemoryFidelityPass
                && task
                    .inputs
                    .get("memory_unit_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == memory_unit_id)
        }))
    }

    fn is_near_duplicate_core_fact(
        &self,
        category_name: &str,
        scope: Option<&str>,
        text: &str,
    ) -> Result<bool> {
        let normalized_scope = normalize_optional_string(scope);
        let needle = normalize_match_text(text);
        let needle_tokens = meaningful_tokens(text);
        let category = self.storage.read_core_store_category(category_name)?;

        for fact in category.facts {
            if fact.status != CoreFactStatus::Active || fact.scope != normalized_scope {
                continue;
            }
            if normalize_match_text(&fact.text) == needle {
                return Ok(false);
            }
            if token_overlap_sets(&needle_tokens, &meaningful_tokens(&fact.text)) >= 0.55 {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub fn resume_sleep_compression(
        &self,
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
        &self,
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

    pub fn resume_memory_unit_pass(
        &self,
        task_id: &str,
        result: MemoryUnitPassResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != MEMORY_UNITS_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;
        self.ensure_manifest()?;

        let mut task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::MemoryUnitPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not memory_unit_pass: {task_id}"
            )));
        }

        let archive_id = task
            .inputs
            .get("preliminary_archive_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "memory_unit_pass task has no preliminary_archive_id: {task_id}"
                ))
            })?;
        if archive_id != result.archive_id {
            return Err(MemoryEngineError::Validation(format!(
                "memory_unit_pass archive_id mismatch: task={archive_id} result={}",
                result.archive_id
            )));
        }

        let mut archive_entry = self.storage.read_archive_entry_by_id(archive_id)?;
        let now = now_rfc3339()?;
        let mut units = Vec::new();
        for draft in result.memory_units {
            let thesis = normalize_whitespace(&draft.thesis);
            if thesis.is_empty() {
                continue;
            }
            units.push(MemoryUnit {
                schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
                memory_unit_id: new_id("mu")?,
                archive_id: archive_entry.archive_id.clone(),
                source_session_id: archive_entry.source_session_id.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
                thesis,
                source_event_ids: draft.source_event_ids,
                evidence: normalize_optional_string(draft.evidence.as_deref()),
                tags: draft.tags,
                weight: draft.weight,
                status: MemoryUnitStatus::ActiveArchive,
                fidelity_status: FidelityStatus::Unchecked,
                fidelity_review: None,
                forget_review: None,
            });
        }

        for unit in &units {
            self.storage.write_memory_unit(unit)?;
        }

        archive_entry.updated_at = now.clone();
        archive_entry.memory_units = units;
        archive_entry.compact_memory =
            render_compact_memory_from_units(&archive_entry.memory_units);
        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;

        Ok(archive_entry)
    }

    pub fn build_evidence_pack(&self, memory_unit_id: &str) -> Result<EvidencePack> {
        if memory_unit_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory_unit_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let session_id = unit.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            self.build_evidence_pack_unlocked(memory_unit_id)
        })
    }

    pub fn begin_memory_fidelity_pass(
        &self,
        memory_unit_id: &str,
    ) -> Result<MemoryFidelityPassStart> {
        if memory_unit_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory_unit_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let session_id = unit.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            self.begin_memory_fidelity_pass_unlocked(memory_unit_id)
        })
    }

    fn begin_memory_fidelity_pass_unlocked(
        &self,
        memory_unit_id: &str,
    ) -> Result<MemoryFidelityPassStart> {
        let evidence_pack = self.build_evidence_pack_unlocked(memory_unit_id)?;
        let now = now_rfc3339()?;
        let task = PendingTask {
            schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
            task_id: new_id("task")?,
            task_type: TaskType::MemoryFidelityPass,
            state: TaskState::Pending,
            created_at: now.clone(),
            updated_at: now,
            prompt_id: self.options.fidelity.prompt_id.clone(),
            prompt_version: self.options.fidelity.prompt_version,
            role_hint: ModelRole::Reasoning,
            expected_output_schema: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
            inputs: serde_json::to_value(&evidence_pack)?,
            attempts: Vec::new(),
            last_error: None,
        };
        self.storage.save_task(&task)?;
        let request = llm_request_from_task(
            &task,
            &self.options.fidelity.prompt_id,
            json!({ "evidence_pack": evidence_pack.clone() }),
        )?;
        Ok(MemoryFidelityPassStart {
            evidence_pack,
            pending_task: task,
            request,
        })
    }

    pub fn submit_memory_fidelity_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<MemoryUnit> {
        let request_id = llm_response_request_id(&response).to_string();
        match response {
            LlmResponse::Ok { text, .. } => {
                let result = (|| {
                    let value = parse_json_value_from_llm_text(&text)?;
                    let mut review: FidelityReview = serde_json::from_value(value)?;
                    if review.schema_version.trim().is_empty() {
                        review.schema_version = FIDELITY_REVIEW_SCHEMA_VERSION.to_string();
                    }
                    self.resume_memory_fidelity_pass(task_id, review)
                })();
                if let Err(err) = &result {
                    self.mark_memory_fidelity_task_failed_best_effort(
                        task_id,
                        format!("{request_id} semantic error: {err}"),
                    );
                }
                result
            }
            LlmResponse::Err { kind, detail, .. } => {
                self.mark_memory_fidelity_task_failed_best_effort(
                    task_id,
                    format!("{request_id} {kind:?}: {detail}"),
                );
                Err(MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass failed: {kind:?}: {detail}"
                )))
            }
        }
    }

    pub fn resume_memory_fidelity_pass(
        &self,
        task_id: &str,
        mut review: FidelityReview,
    ) -> Result<MemoryUnit> {
        if review.schema_version != FIDELITY_REVIEW_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                actual: review.schema_version.clone(),
            });
        }
        review.explanation = normalize_whitespace(&review.explanation);
        review.revised_thesis = normalize_optional_string(review.revised_thesis.as_deref());
        review.missing_detail = normalize_optional_string(review.missing_detail.as_deref());
        review.validate_basic()?;
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::MemoryFidelityPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not memory_fidelity_pass: {task_id}"
            )));
        }

        let task_memory_unit_id = task
            .inputs
            .get("memory_unit_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass task has no memory_unit_id: {task_id}"
                ))
            })?;
        if task_memory_unit_id != review.memory_unit_id {
            return Err(MemoryEngineError::Validation(format!(
                "memory_fidelity_pass memory_unit_id mismatch: task={task_memory_unit_id} result={}",
                review.memory_unit_id
            )));
        }

        let session_id = task
            .inputs
            .get("source_session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass task has no source_session_id: {task_id}"
                ))
            })?
            .to_string();

        self.with_resource_lock(session_lock_key(&session_id), || {
            let now = now_rfc3339()?;
            let mut unit = self
                .storage
                .read_memory_unit_by_id(&review.memory_unit_id)?;
            if unit.archive_id != review.archive_id {
                return Err(MemoryEngineError::Validation(format!(
                    "fidelity review archive_id mismatch: unit={} review={}",
                    unit.archive_id, review.archive_id
                )));
            }

            unit.updated_at = now.clone();
            unit.fidelity_status = review.status;
            unit.status = memory_unit_status_after_fidelity(review.status);
            unit.fidelity_review = Some(review.clone());
            self.storage.write_memory_unit(&unit)?;

            let mut archive_entry = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
            archive_entry.updated_at = now.clone();
            for archive_unit in &mut archive_entry.memory_units {
                if archive_unit.memory_unit_id == unit.memory_unit_id {
                    *archive_unit = unit.clone();
                }
            }
            archive_entry.compact_memory =
                render_compact_memory_from_units(&archive_entry.memory_units);
            self.storage
                .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            self.storage.save_task(&task)?;

            Ok(unit)
        })
    }

    fn build_evidence_pack_unlocked(&self, memory_unit_id: &str) -> Result<EvidencePack> {
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let archive = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
        let session = self.storage.read_session(&unit.source_session_id)?;
        let now = now_rfc3339()?;
        let mut pack = EvidencePack::empty_for(
            new_id("evidence_pack")?,
            now,
            unit.memory_unit_id.clone(),
            unit.archive_id.clone(),
            unit.source_session_id.clone(),
            unit.thesis.clone(),
            self.options.fidelity.max_evidence_tokens,
        );
        pack.unit_evidence = unit.evidence.clone();

        let source_ids = if unit.source_event_ids.is_empty() {
            archive.source_event_ids.iter().collect::<HashSet<_>>()
        } else {
            unit.source_event_ids.iter().collect::<HashSet<_>>()
        };
        let source_indices = session
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| source_ids.contains(&event.event_id).then_some(index))
            .collect::<Vec<_>>();

        let mut selected = HashSet::new();
        for index in &source_indices {
            if let Some(event) = session.events.get(*index) {
                let event = evidence_event_from_stored(
                    event,
                    EvidenceEventRole::Source,
                    self.options.fidelity.max_event_text_chars,
                );
                add_evidence_event(&mut pack, event, &mut selected, true);
            }
        }

        let mut neighbor_indices = BTreeSet::new();
        for source_index in &source_indices {
            for offset in 1..=self.options.fidelity.neighbor_events {
                if let Some(left) = source_index.checked_sub(offset) {
                    neighbor_indices.insert(left);
                }
                let right = source_index + offset;
                if right < session.events.len() {
                    neighbor_indices.insert(right);
                }
            }
        }

        for index in neighbor_indices {
            let Some(event) = session.events.get(index) else {
                continue;
            };
            let event = evidence_event_from_stored(
                event,
                EvidenceEventRole::Neighbor,
                self.options.fidelity.max_event_text_chars,
            );
            add_evidence_event(&mut pack, event, &mut selected, false);
        }

        pack.estimated_tokens = estimate_evidence_pack_tokens(&pack);
        Ok(pack)
    }

    fn mark_memory_fidelity_task_failed_best_effort(&self, task_id: &str, detail: String) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::MemoryFidelityPass {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }

    pub fn begin_forget_review(&self, session_id: &str) -> Result<ForgetReviewStart> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "forget review session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
            let now = now_rfc3339()?;
            let inputs = self.build_forget_review_inputs_unlocked(session_id, &now)?;
            let task = PendingTask {
                schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
                task_id: new_id("task")?,
                task_type: TaskType::ForgetReview,
                state: if inputs.candidates.is_empty() {
                    TaskState::Completed
                } else {
                    TaskState::Pending
                },
                created_at: now.clone(),
                updated_at: now,
                prompt_id: self.options.forgetting.prompt_id.clone(),
                prompt_version: self.options.forgetting.prompt_version,
                role_hint: ModelRole::Balanced,
                expected_output_schema: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                inputs: serde_json::to_value(&inputs)?,
                attempts: Vec::new(),
                last_error: None,
            };
            self.storage.save_task(&task)?;
            let request = llm_request_from_task(
                &task,
                &self.options.forgetting.prompt_id,
                json!({ "forget_review": inputs.clone() }),
            )?;
            Ok(ForgetReviewStart {
                source_session_id: session_id.to_string(),
                candidate_count: inputs.candidates.len(),
                candidates: inputs.candidates,
                pending_task: task,
                request,
            })
        })
    }

    pub fn submit_forget_review_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<ForgetReviewApplyResult> {
        let request_id = llm_response_request_id(&response).to_string();
        match response {
            LlmResponse::Ok { text, .. } => {
                let result = (|| {
                    let value = parse_json_value_from_llm_text(&text)?;
                    let mut review: ForgetReviewResult = serde_json::from_value(value)?;
                    review.normalize_schema();
                    self.resume_forget_review(task_id, review)
                })();
                if let Err(err) = &result {
                    self.mark_forget_review_task_failed_best_effort(
                        task_id,
                        format!("{request_id} semantic error: {err}"),
                    );
                }
                result
            }
            LlmResponse::Err { kind, detail, .. } => {
                self.mark_forget_review_task_failed_best_effort(
                    task_id,
                    format!("{request_id} {kind:?}: {detail}"),
                );
                Err(MemoryEngineError::Validation(format!(
                    "forget_review_pass failed: {kind:?}: {detail}"
                )))
            }
        }
    }

    pub fn resume_forget_review(
        &self,
        task_id: &str,
        mut result: ForgetReviewResult,
    ) -> Result<ForgetReviewApplyResult> {
        result.normalize_schema();
        if result.schema_version != FORGET_REVIEW_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        if result.source_session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "forget review result source_session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::ForgetReview {
            return Err(MemoryEngineError::Validation(format!(
                "task is not forget_review: {task_id}"
            )));
        }
        let inputs: ForgetReviewInputs = serde_json::from_value(task.inputs.clone())?;
        if inputs.source_session_id != result.source_session_id {
            return Err(MemoryEngineError::Validation(format!(
                "forget review source_session_id mismatch: task={} result={}",
                inputs.source_session_id, result.source_session_id
            )));
        }

        self.with_resource_lock(session_lock_key(&inputs.source_session_id), || {
            let now = now_rfc3339()?;
            let candidate_ids = inputs
                .candidates
                .iter()
                .map(|candidate| candidate.memory_unit_id.clone())
                .collect::<HashSet<_>>();
            let mut reviewed = 0;
            let mut forgotten = 0;
            let mut kept = 0;
            let mut protected = 0;
            let mut ignored = 0;
            let mut changed_units = Vec::new();

            for recommendation in &mut result.recommendations {
                recommendation.reason = normalize_whitespace(&recommendation.reason);
                if recommendation.reason.is_empty() {
                    recommendation.reason = "No reason provided.".to_string();
                }
                reviewed += 1;
                if !candidate_ids.contains(&recommendation.memory_unit_id) {
                    ignored += 1;
                    continue;
                }

                match self.apply_forget_recommendation_unlocked(task_id, recommendation, &now)? {
                    ForgetApplyAction::Forgotten(unit) => {
                        forgotten += 1;
                        changed_units.push(unit);
                    }
                    ForgetApplyAction::Kept(unit) => {
                        kept += 1;
                        changed_units.push(unit);
                    }
                    ForgetApplyAction::Protected(unit) => {
                        protected += 1;
                        changed_units.push(unit);
                    }
                    ForgetApplyAction::Ignored => {
                        ignored += 1;
                    }
                }
            }

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            self.storage.save_task(&task)?;

            Ok(ForgetReviewApplyResult {
                schema_version: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: inputs.source_session_id,
                reviewed,
                forgotten,
                kept,
                protected,
                ignored,
                changed_units,
            })
        })
    }

    pub fn list_forgotten_memory_units(&self, session_id: &str) -> Result<ForgottenMemoryUnits> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;
        self.with_resource_lock(session_lock_key(session_id), || {
            let mut units = self
                .storage
                .read_archive(&ArchiveFilters::default())?
                .into_iter()
                .filter(|archive| archive.source_session_id == session_id)
                .flat_map(|archive| archive.memory_units.into_iter())
                .filter(|unit| unit.status == MemoryUnitStatus::Forgotten)
                .collect::<Vec<_>>();
            units.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
            });
            Ok(ForgottenMemoryUnits {
                schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
                source_session_id: session_id.to_string(),
                units,
            })
        })
    }

    pub fn remember_back(&self, memory_unit_id: &str) -> Result<MemoryUnit> {
        if memory_unit_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory_unit_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let session_id = unit.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            let now = now_rfc3339()?;
            let mut unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
            if unit.status != MemoryUnitStatus::Forgotten {
                return Err(MemoryEngineError::Validation(format!(
                    "memory unit is not forgotten: {memory_unit_id}"
                )));
            }
            unit.status = MemoryUnitStatus::ActiveArchive;
            unit.updated_at = now.clone();
            unit.tags.retain(|tag| tag != "forgotten");
            merge_unique(&mut unit.tags, &["remembered_back".to_string()]);
            self.storage.write_memory_unit(&unit)?;
            self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, &now)?;
            Ok(unit)
        })
    }

    fn build_forget_review_inputs_unlocked(
        &self,
        session_id: &str,
        reference_at: &str,
    ) -> Result<ForgetReviewInputs> {
        let mut candidates = Vec::new();
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_iter()
            .filter(|archive| {
                archive.source_session_id == session_id && archive.status == ArchiveStatus::Complete
            })
            .collect::<Vec<_>>();

        for archive in archives {
            let age_days = archive_age_days(&archive, reference_at).unwrap_or(0.0);
            if age_days < self.options.forgetting.min_age_days.max(0.0) {
                continue;
            }
            if archive.recall_count > self.options.forgetting.forget_recall_count_max {
                continue;
            }

            for unit in &archive.memory_units {
                if unit.status != MemoryUnitStatus::ActiveArchive {
                    continue;
                }
                if unit.weight >= self.options.forgetting.forget_weight_threshold {
                    continue;
                }
                let protection =
                    self.forget_protection_reasons_unlocked(unit, &archive, reference_at)?;
                if !protection.is_empty() {
                    continue;
                }

                candidates.push(ForgetReviewCandidate {
                    label: format!("m{}", candidates.len() + 1),
                    memory_unit_id: unit.memory_unit_id.clone(),
                    archive_id: archive.archive_id.clone(),
                    age_days,
                    weight: unit.weight,
                    archive_recall_count: archive.recall_count,
                    archive_last_recalled_days: archive.last_recalled_at.as_deref().and_then(
                        |last_recalled_at| timestamp_age_days(last_recalled_at, reference_at),
                    ),
                    fidelity_status: unit.fidelity_status,
                    has_core_link: false,
                    has_emotional: false,
                    thesis: truncate_chars(&unit.thesis, 240),
                });
            }
        }

        candidates.sort_by(|left, right| {
            right
                .age_days
                .total_cmp(&left.age_days)
                .then_with(|| left.weight.total_cmp(&right.weight))
                .then_with(|| left.archive_recall_count.cmp(&right.archive_recall_count))
                .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
        });
        candidates.truncate(self.options.forgetting.max_review_batch);
        for (index, candidate) in candidates.iter_mut().enumerate() {
            candidate.label = format!("m{}", index + 1);
        }

        Ok(ForgetReviewInputs {
            schema_version: FORGET_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            source_session_id: session_id.to_string(),
            created_at: reference_at.to_string(),
            candidates,
        })
    }

    fn apply_forget_recommendation_unlocked(
        &self,
        task_id: &str,
        recommendation: &ForgetRecommendation,
        now: &str,
    ) -> Result<ForgetApplyAction> {
        let mut unit = match self
            .storage
            .read_memory_unit_by_id(&recommendation.memory_unit_id)
        {
            Ok(unit) => unit,
            Err(_) => return Ok(ForgetApplyAction::Ignored),
        };
        if unit.status != MemoryUnitStatus::ActiveArchive {
            return Ok(ForgetApplyAction::Ignored);
        }
        let archive = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
        let protection = self.forget_protection_reasons_unlocked(&unit, &archive, now)?;
        if !protection.is_empty() {
            unit.updated_at = now.to_string();
            unit.forget_review = Some(ForgetReviewRecord {
                reviewed_at: now.to_string(),
                task_id: Some(task_id.to_string()),
                decision: ForgetDecision::Protect,
                reason: format!(
                    "{} Protected by engine gate: {}",
                    recommendation.reason,
                    protection.join(", ")
                ),
            });
            merge_unique(
                &mut unit.tags,
                &[
                    "forget_reviewed".to_string(),
                    "forget_protected".to_string(),
                ],
            );
            self.storage.write_memory_unit(&unit)?;
            self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
            return Ok(ForgetApplyAction::Protected(unit));
        }

        match recommendation.decision {
            ForgetDecision::Forget => {
                unit.status = MemoryUnitStatus::Forgotten;
                unit.updated_at = now.to_string();
                unit.forget_review = Some(ForgetReviewRecord {
                    reviewed_at: now.to_string(),
                    task_id: Some(task_id.to_string()),
                    decision: ForgetDecision::Forget,
                    reason: recommendation.reason.clone(),
                });
                merge_unique(
                    &mut unit.tags,
                    &["forget_reviewed".to_string(), "forgotten".to_string()],
                );
                self.storage.write_memory_unit(&unit)?;
                self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
                Ok(ForgetApplyAction::Forgotten(unit))
            }
            ForgetDecision::Keep => {
                unit.updated_at = now.to_string();
                unit.forget_review = Some(ForgetReviewRecord {
                    reviewed_at: now.to_string(),
                    task_id: Some(task_id.to_string()),
                    decision: ForgetDecision::Keep,
                    reason: recommendation.reason.clone(),
                });
                merge_unique(&mut unit.tags, &["forget_reviewed".to_string()]);
                self.storage.write_memory_unit(&unit)?;
                self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
                Ok(ForgetApplyAction::Kept(unit))
            }
            ForgetDecision::Protect => {
                unit.updated_at = now.to_string();
                unit.forget_review = Some(ForgetReviewRecord {
                    reviewed_at: now.to_string(),
                    task_id: Some(task_id.to_string()),
                    decision: ForgetDecision::Protect,
                    reason: recommendation.reason.clone(),
                });
                merge_unique(
                    &mut unit.tags,
                    &[
                        "forget_reviewed".to_string(),
                        "forget_protected".to_string(),
                    ],
                );
                self.storage.write_memory_unit(&unit)?;
                self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
                Ok(ForgetApplyAction::Protected(unit))
            }
        }
    }

    fn rebuild_archive_units_and_compact_unlocked(
        &self,
        archive_id: &str,
        updated_unit: &MemoryUnit,
        now: &str,
    ) -> Result<()> {
        let mut archive = self.storage.read_archive_entry_by_id(archive_id)?;
        let mut replaced = false;
        for unit in &mut archive.memory_units {
            if unit.memory_unit_id == updated_unit.memory_unit_id {
                *unit = updated_unit.clone();
                replaced = true;
                break;
            }
        }
        if !replaced {
            archive.memory_units.push(updated_unit.clone());
        }
        archive.updated_at = now.to_string();
        archive.compact_memory = render_compact_memory_from_units(&archive.memory_units);
        self.storage
            .update_archive_entry(&archive.archive_id, &archive)
    }

    fn forget_protection_reasons_unlocked(
        &self,
        unit: &MemoryUnit,
        archive: &ArchiveEntry,
        reference_at: &str,
    ) -> Result<Vec<String>> {
        let mut reasons = Vec::new();
        if unit.weight >= self.options.forgetting.protect_weight {
            reasons.push("high_weight".to_string());
        }
        if archive
            .last_recalled_at
            .as_deref()
            .and_then(|last_recalled_at| timestamp_age_days(last_recalled_at, reference_at))
            .is_some_and(|age_days| age_days <= self.options.forgetting.protect_recall_window_days)
        {
            reasons.push("recent_recall".to_string());
        }
        if self.unit_has_core_link_unlocked(unit)? {
            reasons.push("core_link".to_string());
        }
        if self.unit_has_emotional_marker(unit, archive) {
            reasons.push("emotional".to_string());
        }
        Ok(reasons)
    }

    fn unit_has_core_link_unlocked(&self, unit: &MemoryUnit) -> Result<bool> {
        for category in self.storage.read_core_store_categories()? {
            for fact in category.facts {
                if !core_fact_visible_in_context(fact.status) {
                    continue;
                }
                if fact
                    .source_archive_ids
                    .iter()
                    .any(|id| id == &unit.archive_id)
                {
                    return Ok(true);
                }
                if let Some(candidate_id) = fact.source_candidate_id {
                    if self
                        .storage
                        .read_candidate_belief(&candidate_id)
                        .ok()
                        .is_some_and(|candidate| {
                            candidate
                                .source_memory_unit_ids
                                .iter()
                                .any(|id| id == &unit.memory_unit_id)
                        })
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(self
            .storage
            .read_candidate_beliefs()?
            .into_iter()
            .filter(|candidate| candidate.status == CandidateStatus::Promoted)
            .any(|candidate| {
                candidate
                    .source_memory_unit_ids
                    .iter()
                    .any(|id| id == &unit.memory_unit_id)
            }))
    }

    fn unit_has_emotional_marker(&self, unit: &MemoryUnit, archive: &ArchiveEntry) -> bool {
        let source_ids = if unit.source_event_ids.is_empty() {
            archive.source_event_ids.iter().collect::<HashSet<_>>()
        } else {
            unit.source_event_ids.iter().collect::<HashSet<_>>()
        };
        archive.emotional_markers.iter().any(|marker| {
            marker.strength >= self.options.forgetting.protect_emotional_strength
                && (source_ids.is_empty()
                    || marker
                        .source_event_ids
                        .iter()
                        .any(|event_id| source_ids.contains(event_id)))
        })
    }

    fn mark_forget_review_task_failed_best_effort(&self, task_id: &str, detail: String) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::ForgetReview {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }

    pub fn begin_reflection_analysis(
        &self,
        session_id: &str,
        core_scope: Option<String>,
    ) -> Result<ReflectionPassStart> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "reflection session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
            let now = now_rfc3339()?;
            let inputs = self.build_reflection_inputs_unlocked(session_id, core_scope.clone())?;
            let memory_unit_count = inputs
                .get("memory_units")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let core_fact_count = inputs
                .get("core_facts")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let task = PendingTask {
                schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
                task_id: new_id("task")?,
                task_type: TaskType::ReflectionAnalyze,
                state: TaskState::Pending,
                created_at: now.clone(),
                updated_at: now,
                prompt_id: "reflection_analyze".to_string(),
                prompt_version: 1,
                role_hint: ModelRole::Reasoning,
                expected_output_schema: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                inputs: inputs.clone(),
                attempts: Vec::new(),
                last_error: None,
            };
            self.storage.save_task(&task)?;
            let request = llm_request_from_task(
                &task,
                "reflection_analyze",
                json!({ "reflection_task": inputs }),
            )?;
            Ok(ReflectionPassStart {
                pending_task: task,
                request,
                source_session_id: session_id.to_string(),
                core_scope,
                memory_unit_count,
                core_fact_count,
            })
        })
    }

    pub fn submit_reflection_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<ReflectionCandidatesResult> {
        let request_id = llm_response_request_id(&response).to_string();
        match response {
            LlmResponse::Ok { text, .. } => {
                let result = (|| {
                    let value = parse_json_value_from_llm_text(&text)?;
                    let mut reflection: ReflectionAnalyzeResult = serde_json::from_value(value)?;
                    if reflection.schema_version.trim().is_empty() {
                        reflection.schema_version = REFLECTION_RESULT_SCHEMA_VERSION.to_string();
                    }
                    self.resume_reflection_analysis(task_id, reflection)
                })();
                if let Err(err) = &result {
                    self.mark_reflection_task_failed_best_effort(
                        task_id,
                        format!("{request_id} semantic error: {err}"),
                    );
                }
                result
            }
            LlmResponse::Err { kind, detail, .. } => {
                self.mark_reflection_task_failed_best_effort(
                    task_id,
                    format!("{request_id} {kind:?}: {detail}"),
                );
                Err(MemoryEngineError::Validation(format!(
                    "reflection_analyze failed: {kind:?}: {detail}"
                )))
            }
        }
    }

    pub fn resume_reflection_analysis(
        &self,
        task_id: &str,
        mut result: ReflectionAnalyzeResult,
    ) -> Result<ReflectionCandidatesResult> {
        if result.schema_version != REFLECTION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        if result.source_session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "reflection result source_session_id must not be empty".to_string(),
            ));
        }
        result.core_scope = normalize_optional_string(result.core_scope.as_deref());
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::ReflectionAnalyze {
            return Err(MemoryEngineError::Validation(format!(
                "task is not reflection_analyze: {task_id}"
            )));
        }
        let task_session_id = task
            .inputs
            .get("source_session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "reflection_analyze task has no source_session_id: {task_id}"
                ))
            })?;
        if task_session_id != result.source_session_id {
            return Err(MemoryEngineError::Validation(format!(
                "reflection source_session_id mismatch: task={task_session_id} result={}",
                result.source_session_id
            )));
        }
        let task_scope =
            normalize_optional_string(task.inputs.get("core_scope").and_then(Value::as_str));
        if task_scope != result.core_scope {
            result.core_scope = task_scope.clone();
        }

        self.with_resource_lock(session_lock_key(&result.source_session_id), || {
            let now = now_rfc3339()?;
            let known_units = self
                .reflection_memory_units_unlocked(&result.source_session_id)?
                .into_iter()
                .map(|unit| unit.memory_unit_id)
                .collect::<HashSet<_>>();
            let known_core_facts = task
                .inputs
                .get("core_facts")
                .and_then(Value::as_array)
                .map(|facts| {
                    facts
                        .iter()
                        .filter_map(|fact| fact.get("core_fact_id").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            let mut candidates = Vec::new();
            for draft in result.candidates {
                if let Some(candidate) = self.candidate_from_reflection_draft(
                    &result.source_session_id,
                    task_scope.clone(),
                    draft,
                    &known_units,
                    &known_core_facts,
                    &now,
                )? {
                    self.storage.write_candidate_belief(&candidate)?;
                    candidates.push(candidate);
                }
            }

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now.clone();
            task.last_error = None;
            self.storage.save_task(&task)?;

            Ok(ReflectionCandidatesResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: result.source_session_id,
                core_scope: task_scope,
                created_at: now,
                candidates,
            })
        })
    }

    pub fn list_candidates(&self) -> Result<Vec<CandidateBelief>> {
        self.ensure_manifest()?;
        self.storage.read_candidate_beliefs()
    }

    pub fn review_candidate(&self, input: CandidateReviewInput) -> Result<CandidateReviewResult> {
        validate_candidate_review_input(&input)?;
        self.ensure_manifest()?;

        self.with_resource_lock(candidate_lock_key(&input.candidate_id), || {
            let now = now_rfc3339()?;
            let mut candidate = self.storage.read_candidate_belief(&input.candidate_id)?;
            let review = ReviewRecord {
                reviewed_by: normalize_whitespace(&input.reviewed_by),
                reviewed_at: now.clone(),
                decision: input.decision,
                note: normalize_optional_string(input.note.as_deref()),
            };

            let mut promoted_fact = None;
            let mut contested_facts = Vec::new();
            match input.decision {
                ReviewDecision::Approved => {
                    let scope = normalize_optional_string(input.core_scope.as_deref())
                        .or_else(|| candidate.core_scope.clone());
                    let category = normalize_whitespace(&candidate.category);
                    let tags = {
                        let mut tags = candidate.tags.clone();
                        tags.push("reflection_candidate".to_string());
                        tags.push("manual_review".to_string());
                        unique_strings(tags)
                    };
                    let source_archive_ids =
                        unique_strings(candidate.supporting_archive_ids.clone());
                    contested_facts =
                        self.contest_candidate_core_conflicts(&candidate, scope.clone(), &now)?;
                    let upsert = self.with_resource_lock(core_lock_key(&category), || {
                        self.upsert_core_fact_unlocked(
                            CoreFactInput {
                                schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                                category: category.clone(),
                                scope,
                                text: candidate.text.clone(),
                                confidence: candidate.confidence,
                                tags,
                                source_archive_ids,
                                source_candidate_id: Some(candidate.candidate_id.clone()),
                            },
                            category.clone(),
                        )
                    })?;
                    candidate.status = CandidateStatus::Promoted;
                    candidate.promoted_core_fact_id = Some(upsert.fact.core_fact_id.clone());
                    promoted_fact = Some(upsert.fact);
                }
                ReviewDecision::Rejected => {
                    candidate.status = CandidateStatus::Rejected;
                }
                ReviewDecision::NeedsChanges => {
                    candidate.status = CandidateStatus::Draft;
                }
            }

            candidate.updated_at = now;
            candidate.review = Some(review);
            self.storage.write_candidate_belief(&candidate)?;
            Ok(CandidateReviewResult {
                schema_version: CANDIDATE_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                candidate,
                promoted_fact,
                contested_facts,
            })
        })
    }

    fn contest_candidate_core_conflicts(
        &self,
        candidate: &CandidateBelief,
        scope: Option<String>,
        now: &str,
    ) -> Result<Vec<CoreFact>> {
        let target_ids = candidate
            .contradicted_core_fact_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }

        let candidate_id = candidate.candidate_id.clone();
        let category_names = self
            .storage
            .read_core_store_categories()?
            .into_iter()
            .filter(|category| {
                category.facts.iter().any(|fact| {
                    target_ids.contains(&fact.core_fact_id)
                        && fact.status == CoreFactStatus::Active
                        && fact.scope == scope
                })
            })
            .map(|category| category.category)
            .collect::<BTreeSet<_>>();

        let mut contested = Vec::new();
        for category_name in category_names {
            let contested_for_category =
                self.with_resource_lock(core_lock_key(&category_name), || {
                    let mut category = self.storage.read_core_store_category(&category_name)?;
                    let mut changed = false;
                    let mut contested_for_category = Vec::new();
                    for fact in &mut category.facts {
                        if !target_ids.contains(&fact.core_fact_id)
                            || fact.status != CoreFactStatus::Active
                            || fact.scope != scope
                        {
                            continue;
                        }
                        fact.status = CoreFactStatus::Contested;
                        fact.updated_at = now.to_string();
                        merge_unique(
                            &mut fact.tags,
                            &[
                                "contested".to_string(),
                                "contested_by_reflection_candidate".to_string(),
                            ],
                        );
                        push_link_once(
                            &mut fact.links,
                            Link {
                                kind: "contested_by_candidate".to_string(),
                                target: candidate_id.clone(),
                                note: Some(candidate.text.clone()),
                            },
                        );
                        changed = true;
                        contested_for_category.push(fact.clone());
                    }
                    if changed {
                        category.updated_at = now.to_string();
                        self.storage.write_core_store_category(&category)?;
                    }
                    Ok(contested_for_category)
                })?;
            contested.extend(contested_for_category);
        }

        Ok(contested)
    }

    fn build_reflection_inputs_unlocked(
        &self,
        session_id: &str,
        core_scope: Option<String>,
    ) -> Result<Value> {
        let core_scope = normalize_optional_string(core_scope.as_deref());
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .filter(|entry| entry.status == ArchiveStatus::Complete)
            .collect::<Vec<_>>();

        let mut memory_units = Vec::new();
        for unit in self.reflection_memory_units_unlocked(session_id)? {
            memory_units.push(json!({
                "memory_unit_id": unit.memory_unit_id,
                "archive_id": unit.archive_id,
                "thesis": unit.thesis,
                "evidence": unit.evidence,
                "tags": unit.tags,
                "weight": unit.weight,
                "fidelity_status": unit.fidelity_status,
            }));
        }

        let archive_summaries = archives
            .iter()
            .map(|archive| {
                json!({
                    "archive_id": archive.archive_id,
                    "created_at": archive.created_at,
                    "gist": archive.gist,
                    "compact_memory": archive.compact_memory,
                    "tags": archive.tags,
                    "weight": archive.weight,
                    "recall_count": archive.recall_count,
                })
            })
            .collect::<Vec<_>>();

        let core_facts = self
            .storage
            .read_core_store_categories()?
            .into_iter()
            .flat_map(|category| {
                let category_name = category.category.clone();
                let scope_filter = core_scope.clone();
                category.facts.into_iter().filter_map(move |fact| {
                    if !core_fact_visible_in_context(fact.status) {
                        return None;
                    }
                    if scope_filter.is_some() && fact.scope != scope_filter {
                        return None;
                    }
                    Some(json!({
                        "category": category_name,
                        "core_fact_id": fact.core_fact_id,
                        "text": fact.text,
                        "status": fact.status,
                        "confidence": fact.confidence,
                        "tags": fact.tags,
                    }))
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "schema_version": "reflection_task.v1",
            "source_session_id": session_id,
            "core_scope": core_scope,
            "memory_units": memory_units,
            "archives": archive_summaries,
            "core_facts": core_facts,
            "rules": {
                "agents_do_not_write_core": true,
                "first_iteration_requires_manual_review": true,
                "use_only_validated_memory_units": true
            }
        }))
    }

    fn reflection_memory_units_unlocked(&self, session_id: &str) -> Result<Vec<MemoryUnit>> {
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .filter(|entry| entry.status == ArchiveStatus::Complete)
            .collect::<Vec<_>>();
        let mut units = Vec::new();
        for archive in archives {
            for unit in self
                .storage
                .read_memory_units_for_archive(&archive.archive_id)?
            {
                if unit.source_session_id != session_id {
                    continue;
                }
                if unit.status != MemoryUnitStatus::ActiveArchive {
                    continue;
                }
                if unit.fidelity_status != FidelityStatus::Valid {
                    continue;
                }
                units.push(unit);
            }
        }
        units.sort_by(|left, right| {
            right
                .weight
                .total_cmp(&left.weight)
                .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
        });
        Ok(units)
    }

    fn candidate_from_reflection_draft(
        &self,
        source_session_id: &str,
        core_scope: Option<String>,
        draft: ReflectionCandidateDraft,
        known_units: &HashSet<String>,
        known_core_facts: &HashSet<String>,
        now: &str,
    ) -> Result<Option<CandidateBelief>> {
        let text = normalize_whitespace(&draft.text);
        let category = normalize_category_name(&draft.category);
        let evidence_summary = normalize_whitespace(&draft.evidence_summary);
        if text.is_empty() || category.is_empty() || evidence_summary.is_empty() {
            return Ok(None);
        }

        let source_memory_unit_ids = unique_strings(
            draft
                .source_memory_unit_ids
                .into_iter()
                .filter(|unit_id| known_units.contains(unit_id))
                .collect(),
        );
        if source_memory_unit_ids.is_empty() {
            return Ok(None);
        }

        let supporting_archive_ids = unique_strings(draft.supporting_archive_ids);
        let contradicting_archive_ids = unique_strings(draft.contradicting_archive_ids);
        let contradicted_core_fact_ids = unique_strings(
            draft
                .contradicted_core_fact_ids
                .into_iter()
                .filter(|fact_id| known_core_facts.contains(fact_id))
                .collect(),
        );
        let confidence = draft.confidence.clamp(0.0, 1.0);
        Ok(Some(CandidateBelief {
            schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            candidate_id: new_id("candidate")?,
            source_session_id: Some(source_session_id.to_string()),
            core_scope,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            text,
            category,
            status: CandidateStatus::ReadyForReview,
            confidence,
            supporting_archive_ids,
            contradicting_archive_ids: contradicting_archive_ids.clone(),
            contradicted_core_fact_ids: contradicted_core_fact_ids.clone(),
            evidence_summary,
            promotion_checks: PromotionChecks {
                min_sources_met: true,
                weight_threshold_met: confidence >= 0.70,
                no_recent_contradiction: contradicting_archive_ids.is_empty()
                    && contradicted_core_fact_ids.is_empty(),
                manual_review_required: true,
            },
            source_memory_unit_ids,
            tags: unique_strings(draft.tags),
            promoted_core_fact_id: None,
            review: None,
            links: Vec::new(),
        }))
    }

    fn mark_reflection_task_failed_best_effort(&self, task_id: &str, detail: String) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::ReflectionAnalyze {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }

    pub fn recall(&self, query: RecallQuery) -> Result<RecallResult> {
        validate_recall_query(&query)?;
        self.ensure_manifest()?;

        if let Some(session_id) = query.session_id.clone() {
            self.with_resource_lock(session_lock_key(&session_id), || {
                self.recall_unlocked(query)
            })
        } else {
            self.with_resource_lock("archive:all".to_string(), || self.recall_unlocked(query))
        }
    }

    fn recall_unlocked(&self, query: RecallQuery) -> Result<RecallResult> {
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
            .filter_map(|entry| {
                let scored = score_archive_entry(&entry, &query, &created_at, &self.options.recall);
                if query
                    .filters
                    .min_freshness
                    .is_some_and(|min_freshness| scored.effective_freshness < min_freshness)
                {
                    None
                } else {
                    Some((entry, scored))
                }
            })
            .collect::<Vec<_>>();
        let filtered_count = scored_entries.len();

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
                filtered_count,
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
    pub fidelity: FidelityConfig,
    pub forgetting: ForgetConfig,
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

enum ForgetApplyAction {
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

fn validate_candidate_review_input(input: &CandidateReviewInput) -> Result<()> {
    if input.schema_version != CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            actual: input.schema_version.clone(),
        });
    }

    if input.candidate_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "candidate review candidate_id must not be empty".to_string(),
        ));
    }
    if input.reviewed_by.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "candidate review reviewed_by must not be empty".to_string(),
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

    let (core_facts, estimated_core_tokens, dropped_core_facts) = keep_front_within_budget_by(
        core_facts,
        budget.core_tokens,
        estimate_core_fact_prompt_tokens,
    );

    let current_memory_budget = budget
        .current_memory_tokens
        .saturating_sub(estimated_domain_state_tokens);
    let (session_recent, estimated_session_recent_tokens, dropped_session_recent) =
        keep_recent_within_budget_by(
            session_recent,
            current_memory_budget,
            estimate_recent_event_prompt_tokens,
        );
    let remaining_current_budget =
        current_memory_budget.saturating_sub(estimated_session_recent_tokens);
    let (session_trace, estimated_session_trace_tokens, dropped_session_trace) =
        keep_recent_within_budget_by(
            session_trace,
            remaining_current_budget,
            estimate_trace_event_prompt_tokens,
        );

    let original_archive_count = archive_relevant.len();
    let archive_relevant = archive_relevant
        .into_iter()
        .take(ARCHIVE_MEMORY_PROMPT_LIMIT)
        .collect::<Vec<_>>();
    let dropped_by_prompt_archive_limit =
        original_archive_count.saturating_sub(archive_relevant.len());
    let (archive_relevant, estimated_compressed_memory_tokens, dropped_by_compressed_budget) =
        keep_front_within_budget_by(
            archive_relevant,
            budget.compressed_memory_tokens,
            estimate_archive_prompt_tokens,
        );
    let dropped_archive_relevant = dropped_by_prompt_archive_limit + dropped_by_compressed_budget;

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

fn keep_front_within_budget_by<T: Clone>(
    items: Vec<T>,
    budget: usize,
    estimate: impl Fn(&T) -> usize,
) -> (Vec<T>, usize, usize) {
    let original_len = items.len();
    let mut kept = Vec::new();
    let mut used = 0usize;

    for item in items {
        let item_estimate = estimate(&item);
        if used + item_estimate <= budget {
            used += item_estimate;
            kept.push(item);
        }
    }

    let dropped = original_len.saturating_sub(kept.len());
    (kept, used, dropped)
}

fn rank_core_facts_for_query(
    mut facts: Vec<CoreContextFact>,
    query_text: Option<&str>,
) -> Vec<CoreContextFact> {
    let query_tokens = core_query_tokens(query_text.unwrap_or_default());
    if query_tokens.is_empty() {
        return facts;
    }

    facts.sort_by(|left, right| {
        core_fact_query_score(right, &query_tokens)
            .cmp(&core_fact_query_score(left, &query_tokens))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.category.cmp(&right.category))
            .then_with(|| left.core_fact_id.cmp(&right.core_fact_id))
    });
    facts
}

fn estimate_core_fact_prompt_tokens(fact: &CoreContextFact) -> usize {
    render_core_fact_prompt_line(fact)
        .map(|line| estimate_text_tokens(&line))
        .unwrap_or(0)
}

fn estimate_archive_prompt_tokens(archive: &RecallItem) -> usize {
    estimate_text_tokens(&render_archive_memory_prompt_lines(archive).join("\n"))
}

fn estimate_recent_event_prompt_tokens(event: &CoreContextEvent) -> usize {
    render_context_event_prompt_line(event, RECENT_MAX_TEXT_CHARS)
        .map(|line| estimate_text_tokens(&line))
        .unwrap_or(0)
}

fn estimate_trace_event_prompt_tokens(event: &CoreContextEvent) -> usize {
    render_context_event_prompt_line(event, OLDER_TRACE_MAX_TEXT_CHARS)
        .map(|line| estimate_text_tokens(&line))
        .unwrap_or(0)
}

fn core_fact_query_score(fact: &CoreContextFact, query_tokens: &[String]) -> usize {
    let fact_text = normalize_match_text(&fact.text);
    let fact_category = normalize_match_text(&fact.category);
    let fact_tags: Vec<String> = fact
        .tags
        .iter()
        .map(|tag| normalize_match_text(tag))
        .collect();
    let fact_tokens: HashSet<String> = core_query_tokens(&fact.text).into_iter().collect();

    query_tokens
        .iter()
        .map(|token| {
            let mut score = 0usize;
            if fact_tokens.contains(token) {
                score += 12;
            } else if fact_text.contains(token) {
                score += 6;
            }
            if fact_category.contains(token) {
                score += 4;
            }
            if fact_tags.iter().any(|tag| tag.contains(token)) {
                score += 2;
            }
            score
        })
        .sum()
}

fn core_query_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut seen = HashSet::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else {
            push_core_query_token(&mut tokens, &mut seen, &mut current);
        }
    }
    push_core_query_token(&mut tokens, &mut seen, &mut current);
    tokens
}

fn push_core_query_token(
    tokens: &mut Vec<String>,
    seen: &mut HashSet<String>,
    current: &mut String,
) {
    if current.chars().count() >= 3 && seen.insert(current.clone()) {
        tokens.push(current.clone());
    }
    current.clear();
}

fn keep_recent_within_budget_by<T: Clone>(
    items: Vec<T>,
    budget: usize,
    estimate: impl Fn(&T) -> usize,
) -> (Vec<T>, usize, usize) {
    let original_len = items.len();
    let mut kept_reversed = Vec::new();
    let mut used = 0usize;

    for item in items.into_iter().rev() {
        let item_estimate = estimate(&item);
        if used + item_estimate <= budget {
            used += item_estimate;
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
        memory_units: Vec::new(),
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

#[allow(dead_code)]
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

fn build_memory_unit_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::MemoryUnitPass,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: "memory_unit_pass".to_string(),
        prompt_version: 1,
        role_hint: ModelRole::Balanced,
        expected_output_schema: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
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
                "target_style": "atomic_human_memory_units",
                "return_json": true,
                "do_not_invent_facts": true,
                "use_source_event_ids": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

fn render_compact_memory_from_units(units: &[MemoryUnit]) -> Option<String> {
    let lines = units
        .iter()
        .filter(|unit| unit.status == MemoryUnitStatus::ActiveArchive)
        .map(|unit| unit.thesis.trim())
        .filter(|thesis| !thesis.is_empty())
        .collect::<Vec<_>>();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn memory_unit_status_after_fidelity(status: FidelityStatus) -> MemoryUnitStatus {
    match status {
        FidelityStatus::Valid | FidelityStatus::SelfChecked => MemoryUnitStatus::ActiveArchive,
        FidelityStatus::Unsupported | FidelityStatus::Distorted => MemoryUnitStatus::Rejected,
        FidelityStatus::Unchecked
        | FidelityStatus::TooBroad
        | FidelityStatus::MissingKeyDetail
        | FidelityStatus::NeedsRevision => MemoryUnitStatus::NeedsRevision,
    }
}

fn evidence_event_from_stored(
    event: &StoredEvent,
    role: EvidenceEventRole,
    max_text_chars: usize,
) -> EvidenceEvent {
    EvidenceEvent {
        event_id: event.event_id.clone(),
        timestamp: event.timestamp.clone(),
        event_type: event.event_type.clone(),
        source: event.source.clone(),
        role,
        text: truncate_chars(
            &event_text(event).unwrap_or_else(|| event.payload.to_string()),
            max_text_chars,
        ),
        tags: event.tags.clone(),
    }
}

fn add_evidence_event(
    pack: &mut EvidencePack,
    event: EvidenceEvent,
    selected: &mut HashSet<String>,
    force: bool,
) {
    if !selected.insert(event.event_id.clone()) {
        return;
    }
    let mut candidate = pack.clone();
    candidate.events.push(event.clone());
    let candidate_tokens = estimate_evidence_pack_tokens(&candidate);
    if force || candidate_tokens <= pack.max_estimated_tokens {
        pack.events.push(event);
        pack.estimated_tokens = candidate_tokens;
    } else {
        pack.truncated = true;
    }
}

fn estimate_evidence_pack_tokens(pack: &EvidencePack) -> usize {
    let mut chars = pack.target_thesis.chars().count()
        + pack
            .unit_evidence
            .as_deref()
            .map(|value| value.chars().count())
            .unwrap_or(0);
    for event in &pack.events {
        chars += event.event_id.chars().count()
            + event.timestamp.chars().count()
            + event.event_type.chars().count()
            + event.source.chars().count()
            + event.text.chars().count()
            + event
                .tags
                .iter()
                .map(|tag| tag.chars().count())
                .sum::<usize>()
            + 32;
    }
    chars.div_ceil(2)
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
    effective_freshness: f64,
    explanation: String,
}

fn score_archive_entry(
    entry: &ArchiveEntry,
    query: &RecallQuery,
    reference_at: &str,
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

    let freshness_age_days = archive_age_days(entry, reference_at).unwrap_or(0.0);
    let freshness_decay =
        half_life_decay_factor(freshness_age_days, config.freshness_half_life_days);
    let effective_freshness = (entry.freshness.clamp(0.0, 1.0) * freshness_decay).clamp(0.0, 1.0);
    let recall_boost = recall_boost_factor(entry, reference_at, config);

    let score = (entry.weight
        * effective_freshness
        * recall_boost
        * theme_factor
        * tag_factor
        * text_factor)
        .clamp(0.0, 1.0);

    ScoredArchiveEntry {
        score,
        effective_freshness,
        explanation: format!(
            "weight {:.2} * freshness {:.2} * decay {:.2} ({:.1}d) = effective {:.2} * recall {:.2} * theme {:.2} * tags {:.2} * text {:.2}",
            entry.weight,
            entry.freshness,
            freshness_decay,
            freshness_age_days,
            effective_freshness,
            recall_boost,
            theme_factor,
            tag_factor,
            text_factor
        ),
    }
}

fn archive_age_days(entry: &ArchiveEntry, reference_at: &str) -> Option<f64> {
    timestamp_age_days(&entry.time_range.end, reference_at)
        .or_else(|| timestamp_age_days(&entry.updated_at, reference_at))
        .or_else(|| timestamp_age_days(&entry.created_at, reference_at))
}

fn timestamp_age_days(older_timestamp: &str, newer_timestamp: &str) -> Option<f64> {
    let older = parse_rfc3339(older_timestamp)?;
    let newer = parse_rfc3339(newer_timestamp)?;
    if newer <= older {
        return Some(0.0);
    }
    Some((newer - older).whole_seconds() as f64 / 86_400.0)
}

fn parse_rfc3339(timestamp: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(timestamp, &Rfc3339).ok()
}

fn half_life_decay_factor(age_days: f64, half_life_days: f64) -> f64 {
    if !age_days.is_finite() || age_days <= 0.0 {
        return 1.0;
    }
    if !half_life_days.is_finite() || half_life_days <= 0.0 {
        return 1.0;
    }
    0.5_f64.powf(age_days / half_life_days).clamp(0.0, 1.0)
}

fn recall_boost_factor(
    entry: &ArchiveEntry,
    reference_at: &str,
    config: &RecallStage1Config,
) -> f64 {
    let count_boost =
        (entry.recall_count as f64).ln_1p().max(0.0) * config.recall_count_log_bonus.max(0.0);
    let recent_boost = entry
        .last_recalled_at
        .as_deref()
        .and_then(|last_recalled_at| timestamp_age_days(last_recalled_at, reference_at))
        .map(|age_days| {
            config.recent_recall_bonus.max(0.0)
                * half_life_decay_factor(age_days, config.recent_recall_half_life_days)
        })
        .unwrap_or(0.0);
    (1.0 + count_boost + recent_boost).clamp(1.0, config.max_recall_boost_factor.max(1.0))
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
        freshness: scored.effective_freshness,
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
        format!(
            "РџРѕРїРµСЂРµРґРЅС–Р№ СЃРїРѕРіР°Рґ С–Р· {} РїРѕРґС–С—(Р№).",
            events.len()
        )
    } else {
        format!("РџРѕРїРµСЂРµРґРЅС–Р№ СЃРїРѕРіР°Рґ: {}.", texts.join(" / "))
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
            "РџРѕРїРµСЂРµРґРЅС–Р№ Р°СЂС…С–РІРЅРёР№ СЃРїРѕРіР°Рґ С–Р· СЃРµСЃС–С— {session_id}, СЃС‚РІРѕСЂРµРЅРёР№ Р°Р»РіРѕСЂРёС‚РјС–С‡РЅРѕ Р· {} РїРѕРґС–С—(Р№).",
            events.len()
        )
    } else {
        format!(
            "РџРѕРїРµСЂРµРґРЅС–Р№ Р°СЂС…С–РІРЅРёР№ СЃРїРѕРіР°Рґ С–Р· СЃРµСЃС–С— {session_id}. РљР»СЋС‡РѕРІС– РїРѕРґС–С—: {}",
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

fn sleep_run_from_stage1(stage1: SleepStage1Result) -> Result<SleepRun> {
    let mut requests = Vec::new();
    let sleep_task = stage1.pending_task;

    if let Some(memory_unit_task) = stage1.memory_unit_task.clone() {
        requests.push(SleepRequestState {
            track: SleepTrack::MemoryUnit,
            request: llm_request_from_task(
                &memory_unit_task,
                "memory_unit_pass",
                json!({ "sleep_task": memory_unit_task.inputs }),
            )?,
            attempts: 0,
            completed: false,
            last_error: None,
        });
    }

    for (track, prompt_id, fallback_schema) in [
        (
            SleepTrack::Emotional,
            "sleep_emotional_pass",
            "sleep_emotional_pass.v1",
        ),
        (
            SleepTrack::TopicThread,
            "sleep_topic_thread_pass",
            "sleep_topic_thread_pass.v1",
        ),
        (
            SleepTrack::PersonalSignal,
            "sleep_personal_signal_pass",
            "sleep_personal_signal_pass.v1",
        ),
        (
            SleepTrack::Relational,
            "sleep_relational_pass",
            "sleep_relational_pass.v1",
        ),
    ] {
        requests.push(SleepRequestState {
            track,
            request: LlmRequest {
                request_id: new_id("llm_req")?,
                task_id: sleep_task.task_id.clone(),
                role_hint: sleep_task.role_hint,
                prompt_id: prompt_id.to_string(),
                prompt_version: sleep_task.prompt_version,
                prompt_inputs: json!({ "sleep_task": sleep_task.inputs }),
                expected_output_schema: fallback_schema.to_string(),
            },
            attempts: 0,
            completed: false,
            last_error: None,
        });
    }

    Ok(SleepRun {
        schema_version: SLEEP_RUN_SCHEMA_VERSION.to_string(),
        session_id: stage1.archive_entry.source_session_id.clone(),
        archive_id: stage1.archive_entry.archive_id,
        sleep_task_id: sleep_task.task_id,
        memory_unit_task_id: stage1.memory_unit_task.map(|task| task.task_id),
        stage: SleepRunStage::Extraction,
        max_pass_attempts: DEFAULT_SLEEP_PASS_MAX_ATTEMPTS,
        requests,
        failed_passes: Vec::new(),
        memory_unit_result: None,
        emotional_pass: None,
        topic_thread_pass: None,
        personal_signal_pass: None,
        relational_pass: None,
        consolidator_gist: None,
        consolidator_narrative: None,
        completion_mode: None,
    })
}

fn llm_request_from_task(
    task: &PendingTask,
    prompt_id: &str,
    prompt_inputs: Value,
) -> Result<LlmRequest> {
    Ok(LlmRequest {
        request_id: new_id("llm_req")?,
        task_id: task.task_id.clone(),
        role_hint: task.role_hint,
        prompt_id: prompt_id.to_string(),
        prompt_version: task.prompt_version,
        prompt_inputs,
        expected_output_schema: task.expected_output_schema.clone(),
    })
}

fn validate_sleep_run(run: &SleepRun) -> Result<()> {
    if run.schema_version != SLEEP_RUN_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: SLEEP_RUN_SCHEMA_VERSION.to_string(),
            actual: run.schema_version.clone(),
        });
    }
    if run.session_id.trim().is_empty()
        || run.archive_id.trim().is_empty()
        || run.sleep_task_id.trim().is_empty()
    {
        return Err(MemoryEngineError::Validation(
            "sleep run must include session_id, archive_id, and sleep_task_id".to_string(),
        ));
    }
    Ok(())
}

fn advance_sleep_run_stage(run: &mut SleepRun) -> Result<()> {
    match run.stage {
        SleepRunStage::Extraction => {
            if run
                .requests
                .iter()
                .filter(|state| state_stage(state.track) == SleepRunStage::Extraction)
                .all(|state| state.completed)
            {
                run.stage = SleepRunStage::Consolidation;
                ensure_consolidator_request(run)?;
            }
        }
        SleepRunStage::Consolidation => {
            if run
                .requests
                .iter()
                .filter(|state| state_stage(state.track) == SleepRunStage::Consolidation)
                .all(|state| state.completed)
            {
                run.stage = SleepRunStage::ReadyToFinish;
            }
        }
        SleepRunStage::ReadyToFinish | SleepRunStage::Finished => {}
    }
    Ok(())
}

fn ensure_consolidator_request(run: &mut SleepRun) -> Result<()> {
    if run
        .requests
        .iter()
        .any(|state| state.track == SleepTrack::Consolidator)
        || run.completion_mode.as_deref() == Some("fallback_from_tracks")
    {
        return Ok(());
    }

    run.requests.push(SleepRequestState {
        track: SleepTrack::Consolidator,
        request: LlmRequest {
            request_id: new_id("llm_req")?,
            task_id: run.sleep_task_id.clone(),
            role_hint: ModelRole::Balanced,
            prompt_id: "sleep_consolidator".to_string(),
            prompt_version: 1,
            prompt_inputs: json!({
                "sleep_task": sleep_task_input_from_run(run),
                "emotional_pass": run.emotional_pass.clone().unwrap_or_else(|| json!({ "emotional_markers": [] })),
                "topic_thread_pass": run.topic_thread_pass.clone().unwrap_or_else(|| json!({ "topic_thread": [] })),
                "personal_signal_pass": run.personal_signal_pass.clone().unwrap_or_else(|| json!({ "personal_signals": [] })),
                "relational_pass": run.relational_pass.clone().unwrap_or_else(|| json!({ "relational_tone": null })),
            }),
            expected_output_schema: CONSOLIDATOR_TEXT_SCHEMA_VERSION.to_string(),
        },
        attempts: 0,
        completed: false,
        last_error: None,
    });
    Ok(())
}

fn sleep_task_input_from_run(run: &SleepRun) -> Value {
    run.requests
        .iter()
        .find(|state| {
            matches!(
                state.track,
                SleepTrack::Emotional
                    | SleepTrack::TopicThread
                    | SleepTrack::PersonalSignal
                    | SleepTrack::Relational
            )
        })
        .and_then(|state| state.request.prompt_inputs.get("sleep_task").cloned())
        .unwrap_or_else(|| json!({}))
}

fn state_stage(track: SleepTrack) -> SleepRunStage {
    match track {
        SleepTrack::Consolidator => SleepRunStage::Consolidation,
        SleepTrack::MemoryUnit
        | SleepTrack::Emotional
        | SleepTrack::TopicThread
        | SleepTrack::PersonalSignal
        | SleepTrack::Relational => SleepRunStage::Extraction,
    }
}

fn handle_sleep_response(
    run: &mut SleepRun,
    state: &mut SleepRequestState,
    response: LlmResponse,
) -> Result<()> {
    match response {
        LlmResponse::Ok { text, .. } => match parse_sleep_response_text(state.track, &text, run) {
            Ok(value) => {
                assign_sleep_track_result(run, state.track, value);
                state.completed = true;
                state.last_error = None;
            }
            Err(err) => handle_sleep_pass_error(run, state, err.to_string())?,
        },
        LlmResponse::Err { kind, detail, .. } => {
            handle_sleep_pass_error(run, state, format!("{kind:?}: {detail}"))?
        }
    }
    Ok(())
}

fn handle_sleep_pass_error(
    run: &mut SleepRun,
    state: &mut SleepRequestState,
    error: String,
) -> Result<()> {
    state.last_error = Some(error.clone());
    if state.attempts < run.max_pass_attempts {
        state.request.prompt_inputs = add_retry_instruction(&state.request.prompt_inputs, &error);
        return Ok(());
    }

    push_unique(&mut run.failed_passes, state.request.prompt_id.clone());
    let fallback = fallback_value_for_track(state.track, run);
    if state.track != SleepTrack::Consolidator {
        assign_sleep_track_result(run, state.track, fallback);
    }
    state.completed = true;
    Ok(())
}

fn parse_sleep_response_text(track: SleepTrack, text: &str, run: &SleepRun) -> Result<Value> {
    if track == SleepTrack::Consolidator {
        let (gist, narrative) = parse_consolidator_text(text)?;
        return Ok(json!({
            "gist": gist,
            "narrative": narrative,
        }));
    }

    let value = parse_json_value_from_llm_text(text)?;
    match track {
        SleepTrack::MemoryUnit => {
            let mut result: MemoryUnitPassResult = serde_json::from_value(value)?;
            if result.schema_version.trim().is_empty() {
                result.schema_version = MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string();
            }
            if result.archive_id != run.archive_id {
                result.archive_id = run.archive_id.clone();
            }
            result.validate_basic()?;
            Ok(serde_json::to_value(result)?)
        }
        SleepTrack::Consolidator => unreachable!("consolidator text is parsed before JSON tracks"),
        SleepTrack::Emotional
        | SleepTrack::TopicThread
        | SleepTrack::PersonalSignal
        | SleepTrack::Relational => Ok(value),
    }
}

fn parse_json_value_from_llm_text(text: &str) -> Result<Value> {
    let mut candidate = text.trim().to_string();
    if candidate.starts_with("```") {
        candidate = candidate
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim()
            .to_string();
        if candidate.ends_with("```") {
            candidate.truncate(candidate.len().saturating_sub(3));
        }
    }

    match serde_json::from_str::<Value>(candidate.trim()) {
        Ok(value) => Ok(value),
        Err(original_err) => {
            let Some(start) = candidate.find('{') else {
                return Err(original_err.into());
            };
            let Some(end) = candidate.rfind('}') else {
                return Err(original_err.into());
            };
            serde_json::from_str::<Value>(&candidate[start..=end]).map_err(Into::into)
        }
    }
}

fn parse_consolidator_text(text: &str) -> Result<(String, String)> {
    let candidate = strip_markdown_fence(text.trim()).trim();

    if candidate.is_empty() {
        return Err(MemoryEngineError::Validation(
            "consolidator returned empty text".to_string(),
        ));
    }

    if let Some(decoded) = parse_consolidator_json_string(candidate)? {
        return parse_consolidator_text(&decoded);
    }

    if let Some((gist, narrative)) = parse_consolidator_json_object(candidate)? {
        return validate_consolidator_overlay(gist, narrative);
    }

    let mut lines = candidate.lines();
    let first_nonempty = lines
        .by_ref()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .ok_or_else(|| {
            MemoryEngineError::Validation("consolidator returned empty text".to_string())
        })?;

    let gist = strip_consolidator_gist_prefix(first_nonempty).to_string();

    let narrative = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let narrative = if narrative.is_empty() {
        gist.clone()
    } else {
        narrative
    };

    validate_consolidator_overlay(gist, narrative)
}

fn strip_markdown_fence(text: &str) -> &str {
    let Some(stripped) = text.strip_prefix("```") else {
        return text;
    };

    let body = stripped
        .find('\n')
        .map(|index| &stripped[index + 1..])
        .unwrap_or("");
    body.strip_suffix("```").unwrap_or(body).trim()
}

fn parse_consolidator_json_object(candidate: &str) -> Result<Option<(String, String)>> {
    if !candidate.starts_with('{') {
        return Ok(None);
    }

    let value = serde_json::from_str::<Value>(candidate).map_err(|err| {
        MemoryEngineError::Validation(format!(
            "consolidator returned JSON-shaped text that could not be parsed: {err}"
        ))
    })?;

    let gist = value
        .get("gist")
        .and_then(Value::as_str)
        .map(strip_consolidator_gist_prefix)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let narrative = value
        .get("narrative")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    match (gist, narrative) {
        (Some(gist), Some(narrative)) => Ok(Some((gist, narrative))),
        (Some(gist), None) => Ok(Some((gist.clone(), gist))),
        (None, Some(narrative)) => Ok(Some((narrative.clone(), narrative))),
        (None, None) => Err(MemoryEngineError::Validation(
            "consolidator returned JSON without gist or narrative strings".to_string(),
        )),
    }
}

fn parse_consolidator_json_string(candidate: &str) -> Result<Option<String>> {
    if !candidate.starts_with('"') {
        return Ok(None);
    }

    serde_json::from_str::<String>(candidate)
        .map(Some)
        .map_err(|err| {
            MemoryEngineError::Validation(format!(
                "consolidator returned quoted text that could not be decoded: {err}"
            ))
        })
}

fn strip_consolidator_gist_prefix(text: &str) -> &str {
    text.trim()
        .strip_prefix("GIST:")
        .or_else(|| text.trim().strip_prefix("gist:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| text.trim())
}

fn validate_consolidator_overlay(gist: String, narrative: String) -> Result<(String, String)> {
    if !gist_looks_valid(&gist) {
        return Err(MemoryEngineError::Validation(format!(
            "{CONSOLIDATOR_GIST_REJECTED_MARKER}: consolidator gist is not a compact single-line summary"
        )));
    }
    if !narrative_looks_valid(&narrative) {
        return Err(MemoryEngineError::Validation(
            "consolidator_narrative_rejected: consolidator narrative looks like a raw structured blob"
                .to_string(),
        ));
    }
    Ok((gist, narrative))
}

fn gist_looks_valid(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 200 {
        return false;
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }
    if starts_with_structural_wrapper(trimmed) {
        return false;
    }
    serde_json::from_str::<Value>(trimmed).is_err()
}

fn narrative_looks_valid(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if matches!(trimmed.chars().next(), Some('{') | Some('[') | Some('`')) {
        return false;
    }
    if trimmed.starts_with('"') && serde_json::from_str::<String>(trimmed).is_ok() {
        return false;
    }
    true
}

fn starts_with_structural_wrapper(value: &str) -> bool {
    matches!(
        value.chars().next(),
        Some('{') | Some('[') | Some('"') | Some('`')
    )
}

fn assign_sleep_track_result(run: &mut SleepRun, track: SleepTrack, value: Value) {
    match track {
        SleepTrack::MemoryUnit => run.memory_unit_result = Some(value),
        SleepTrack::Emotional => run.emotional_pass = Some(value),
        SleepTrack::TopicThread => run.topic_thread_pass = Some(value),
        SleepTrack::PersonalSignal => run.personal_signal_pass = Some(value),
        SleepTrack::Relational => run.relational_pass = Some(value),
        SleepTrack::Consolidator => {
            run.consolidator_gist = value
                .get("gist")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            run.consolidator_narrative = value
                .get("narrative")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            run.completion_mode = Some("consolidated".to_string());
        }
    }
}

fn fallback_value_for_track(track: SleepTrack, run: &mut SleepRun) -> Value {
    match track {
        SleepTrack::MemoryUnit => serde_json::to_value(empty_memory_unit_result(&run.archive_id))
            .unwrap_or_else(|_| json!({ "memory_units": [] })),
        SleepTrack::Emotional => json!({ "emotional_markers": [] }),
        SleepTrack::TopicThread => json!({ "topic_thread": [] }),
        SleepTrack::PersonalSignal => json!({ "personal_signals": [] }),
        SleepTrack::Relational => json!({ "relational_tone": null }),
        SleepTrack::Consolidator => {
            run.completion_mode = Some("fallback_from_tracks".to_string());
            json!(null)
        }
    }
}

fn add_retry_instruction(prompt_inputs: &Value, error: &str) -> Value {
    let mut value = prompt_inputs.clone();
    if let Value::Object(ref mut object) = value {
        object.insert(
            "retry_instruction".to_string(),
            json!({
                "previous_response_error": error,
                "instruction": "Your previous response was not accepted. Return only the requested valid output schema. No prose, no markdown, no comments."
            }),
        );
    }
    value
}

fn llm_response_request_id(response: &LlmResponse) -> &str {
    match response {
        LlmResponse::Ok { request_id, .. } | LlmResponse::Err { request_id, .. } => request_id,
    }
}

fn empty_memory_unit_result(archive_id: &str) -> MemoryUnitPassResult {
    MemoryUnitPassResult {
        schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
        archive_id: archive_id.to_string(),
        memory_units: Vec::new(),
    }
}

fn assemble_sleep_compression_from_tracks(run: &SleepRun) -> Result<SleepCompressionResult> {
    let emotional_markers = run
        .emotional_pass
        .as_ref()
        .and_then(|value| value.get("emotional_markers"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let topic_thread = run
        .topic_thread_pass
        .as_ref()
        .and_then(|value| value.get("topic_thread"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let personal_signals = run
        .personal_signal_pass
        .as_ref()
        .and_then(|value| value.get("personal_signals"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let relational_tone = run
        .relational_pass
        .as_ref()
        .and_then(|value| value.get("relational_tone"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut result = SleepCompressionResult {
        schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
        archive_id: run.archive_id.clone(),
        gist: "РЎРµСЃС–СЏ Р·Р±РµСЂРµР¶РµРЅР° СЏРє РЅР°Р±С–СЂ РІР°Р¶Р»РёРІРёС… СЃРїРѕРіР°РґС–РІ."
            .to_string(),
        narrative: neutral_narrative_from_tracks(
            &serde_json::from_value::<Vec<crate::archive::EmotionalMarker>>(
                emotional_markers.clone(),
            )?,
            &serde_json::from_value::<Vec<crate::archive::TopicThreadItem>>(topic_thread.clone())?,
            &serde_json::from_value::<Vec<crate::archive::PersonalSignal>>(
                personal_signals.clone(),
            )?,
        ),
        compact_memory: None,
        facts: Vec::new(),
        quotes: Vec::new(),
        tags: vec!["multi_pass_sleep".to_string()],
        theme: None,
        weight: 0.55,
        links: Vec::new(),
        emotional_markers: serde_json::from_value(emotional_markers)?,
        topic_thread: serde_json::from_value(topic_thread)?,
        personal_signals: serde_json::from_value(personal_signals)?,
        relational_tone: serde_json::from_value(relational_tone)?,
    };

    if let Some(signal) = result.personal_signals.first() {
        result.gist = signal.text.clone();
    } else if let Some(topic) = result.topic_thread.first() {
        result.gist = topic.summary.clone().unwrap_or_else(|| topic.topic.clone());
        result.theme = Some(topic.topic.clone());
    } else if let Some(marker) = result.emotional_markers.first() {
        result.gist = format!("{}: {}", marker.target, marker.affect);
    }
    if run.completion_mode.as_deref() == Some("fallback_from_tracks") {
        result.tags.push("consolidator_fallback".to_string());
    }
    result.weight = fallback_archive_weight(&result);
    result.validate_basic()?;
    Ok(result)
}

fn neutral_narrative_from_tracks(
    emotional_markers: &[crate::archive::EmotionalMarker],
    topic_thread: &[crate::archive::TopicThreadItem],
    personal_signals: &[crate::archive::PersonalSignal],
) -> String {
    let mut parts = Vec::new();
    if !personal_signals.is_empty() {
        let signals = personal_signals
            .iter()
            .take(3)
            .map(|signal| signal.text.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!("РћСЃРѕР±РёСЃС‚С– СЃРёРіРЅР°Р»Рё: {signals}."));
    }
    if !emotional_markers.is_empty() {
        let markers = emotional_markers
            .iter()
            .take(3)
            .map(|marker| format!("{} ({})", marker.target, marker.affect))
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!(
            "Р•РјРѕС†С–Р№РЅРѕ РїРѕРјС–С‚РЅС– РјРѕРјРµРЅС‚Рё: {markers}."
        ));
    }
    if !topic_thread.is_empty() {
        let topics = topic_thread
            .iter()
            .take(4)
            .map(|topic| {
                topic
                    .summary
                    .as_deref()
                    .filter(|summary| !summary.trim().is_empty())
                    .unwrap_or(&topic.topic)
            })
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!("РўРµРјРё СЂРѕР·РјРѕРІРё: {topics}."));
    }

    if parts.is_empty() {
        "РЎРµСЃС–СЏ Р±СѓР»Р° СЃС‚РёСЃРЅСѓС‚Р° Сѓ СЃС‚СЂСѓРєС‚СѓСЂРѕРІР°РЅС– С‚СЂРµРєРё, Р°Р»Рµ Р±РµР· РІРёСЂР°Р·РЅРёС… РґРѕРІРіРѕС‚СЂРёРІР°Р»РёС… СЃРёРіРЅР°Р»С–РІ."
            .to_string()
    } else {
        parts.join(" ")
    }
}

fn fallback_archive_weight(result: &SleepCompressionResult) -> f64 {
    let strongest_emotion = result
        .emotional_markers
        .iter()
        .map(|marker| marker.strength)
        .fold(0.0, f64::max);
    let strongest_signal = result
        .personal_signals
        .iter()
        .map(|signal| signal.confidence)
        .fold(0.0, f64::max);
    strongest_emotion.max(strongest_signal).clamp(0.55, 1.0)
}

fn apply_sleep_run_tags(result: &mut SleepCompressionResult, run: &SleepRun) {
    if let Some(mode) = &run.completion_mode {
        result.tags.push(format!("completion_mode:{mode}"));
    }
    if run
        .failed_passes
        .iter()
        .any(|prompt_id| prompt_id == "sleep_consolidator")
        && run.requests.iter().any(|state| {
            state.track == SleepTrack::Consolidator
                && state
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains(CONSOLIDATOR_GIST_REJECTED_MARKER))
        })
    {
        result
            .tags
            .push(CONSOLIDATOR_GIST_REJECTED_MARKER.to_string());
    }
    for prompt_id in &run.failed_passes {
        result.tags.push(format!("pass_failed:{prompt_id}"));
    }
    result.tags = unique_strings(std::mem::take(&mut result.tags));
}

fn push_unique(target: &mut Vec<String>, value: String) {
    if !target.iter().any(|existing| existing == &value) {
        target.push(value);
    }
}

fn normalize_category_name(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_underscore = false;
    for ch in normalize_whitespace(value).to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch);
            previous_underscore = false;
        } else if !previous_underscore {
            normalized.push('_');
            previous_underscore = true;
        }
    }
    normalized.trim_matches('_').chars().take(64).collect()
}

fn meaningful_tokens(text: &str) -> BTreeSet<String> {
    let stop_words = [
        "the",
        "and",
        "this",
        "that",
        "РєРѕСЂРёСЃС‚СѓРІР°С‡",
        "РєРѕСЂРёСЃС‚СѓРІР°С‡Р°",
        "РєРѕСЂРёСЃС‚СѓРІР°С‡Сѓ",
        "РґСѓР¶Рµ",
        "Р»СЋР±РёС‚СЊ",
        "С†С–РєР°РІРёС‚СЊСЃСЏ",
    ];
    let stop_words = stop_words.into_iter().collect::<BTreeSet<_>>();
    tokenize(text)
        .into_iter()
        .filter(|token| !stop_words.contains(token.as_str()))
        .collect()
}

fn token_overlap_sets(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let overlap = left.intersection(right).count();
    overlap as f64 / left.len().min(right.len()) as f64
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

fn push_link_once(target: &mut Vec<Link>, link: Link) {
    if target
        .iter()
        .any(|existing| existing.kind == link.kind && existing.target == link.target)
    {
        return;
    }
    target.push(link);
}

fn core_fact_visible_in_context(status: CoreFactStatus) -> bool {
    matches!(status, CoreFactStatus::Active | CoreFactStatus::Contested)
}

fn core_context_status_rank(status: CoreFactStatus) -> u8 {
    match status {
        CoreFactStatus::Active => 0,
        CoreFactStatus::Contested => 1,
        CoreFactStatus::NeedsReview => 2,
        CoreFactStatus::Contradicted => 3,
        CoreFactStatus::Deprecated => 4,
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

fn session_lock_key(session_id: &str) -> String {
    format!("session:{session_id}")
}

fn core_lock_key(category: &str) -> String {
    format!("core:{category}")
}

fn candidate_lock_key(candidate_id: &str) -> String {
    format!("candidate:{candidate_id}")
}

fn lock_resource<'a>(resource: &'a Arc<Mutex<()>>, key: &str) -> Result<MutexGuard<'a, ()>> {
    resource
        .lock()
        .map_err(|_| MemoryEngineError::Storage(format!("resource lock was poisoned: {key}")))
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

#[cfg(test)]
mod tests {
    use super::parse_consolidator_text;

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
}
