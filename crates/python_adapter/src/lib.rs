//! Python adapter for Memory Engine.
//!
//! Thin PyO3 wrapper. Accepts JSON strings on the boundary, converts to
//! Rust structs from `memory_engine`, runs the operation, returns JSON.
//!
//! No LLM, no provider, no model selection lives here. The Python caller
//! receives `PendingTask` objects in the returned JSON and is fully
//! responsible for executing them with whatever provider it chooses, then
//! submitting results back through `resume_sleep_compression`,
//! `resume_memory_unit_pass`, or legacy `resume_compact_memory_pass`.

// PyO3 0.22 `#[pymethods]` expansion produces an `Into<PyErr>` step that
// clippy 1.95 flags as `useless_conversion`. Silencing this lint locally
// while the upstream fix lands.
#![allow(clippy::useless_conversion)]

use std::path::PathBuf;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde::de::DeserializeOwned;
use serde::Serialize;

use ::memory_engine::archive::FidelityReview;
use ::memory_engine::core_store::{
    CandidateReviewInput, CoreContextPackage, CoreContextRequest, CoreFactInput, CoreFactPatchInput,
};
use ::memory_engine::event::IngestEvent;
use ::memory_engine::forgetting::ForgetReviewResult;
use ::memory_engine::hypothesis::HypothesisGraphExportRequest;
use ::memory_engine::llm::{LlmResponse, SleepRun, SleepRunOptions};
use ::memory_engine::recall::RecallQuery;
use ::memory_engine::reflection::{
    ReflectionActionsResult, ReflectionAnalyzeResult, ReflectionOpportunity,
};
use ::memory_engine::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use ::memory_engine::vector::{
    DeepRecallQuery, EmbedBatchResult, DEFAULT_ASSOCIATIVE_MEMORY_MIN_SIMILARITY,
};
use ::memory_engine::{
    EngineOptions, FileStorage, MemoryEngine as CoreEngine, MemoryEngineError, WriteGeneration,
};

pyo3::create_exception!(memory_engine, OperationConflictError, PyRuntimeError);
pyo3::create_exception!(
    memory_engine,
    OperationResultUnavailableError,
    PyRuntimeError
);
pyo3::create_exception!(memory_engine, OperationIncompleteError, PyRuntimeError);
pyo3::create_exception!(memory_engine, OperationStaleError, PyRuntimeError);
pyo3::create_exception!(memory_engine, NeedsNewReviewError, PyRuntimeError);

#[pyclass(name = "MemoryEngine")]
pub struct PyMemoryEngine {
    inner: CoreEngine<FileStorage>,
}

#[pymethods]
impl PyMemoryEngine {
    #[new]
    #[pyo3(signature = (
        memory_dir,
        host_id="unknown",
        stage2_candidate_limit=40,
        stage2_vector_weight=0.5,
        associative_memory_min_similarity=None,
        reflection_salience_threshold=2.5,
        reflection_critical_unit_weight=0.95,
        reflection_max_interval_days=7.0,
        reflection_scheduled_min_units=2,
        reflection_max_trigger_units=40,
        reflection_max_related_units=20,
        reflection_min_related_score=2,
        reflection_max_existing_candidates=20,
        fidelity_auto_validate_weight_threshold=0.85
    ))]
    // PyO3 exposes these as optional keyword configuration on one stable
    // constructor; grouping them into a Rust-only options object would make
    // the Python boundary less explicit and break the additive API shape.
    #[allow(clippy::too_many_arguments)]
    fn new(
        memory_dir: &str,
        host_id: &str,
        stage2_candidate_limit: usize,
        stage2_vector_weight: f32,
        associative_memory_min_similarity: Option<f32>,
        reflection_salience_threshold: f64,
        reflection_critical_unit_weight: f64,
        reflection_max_interval_days: f64,
        reflection_scheduled_min_units: usize,
        reflection_max_trigger_units: usize,
        reflection_max_related_units: usize,
        reflection_min_related_score: usize,
        reflection_max_existing_candidates: usize,
        fidelity_auto_validate_weight_threshold: f64,
    ) -> PyResult<Self> {
        if stage2_candidate_limit == 0 {
            return Err(PyValueError::new_err(
                "stage2_candidate_limit must be greater than zero",
            ));
        }
        if !stage2_vector_weight.is_finite() || !(0.0..=1.0).contains(&stage2_vector_weight) {
            return Err(PyValueError::new_err(
                "stage2_vector_weight must be finite and between 0.0 and 1.0",
            ));
        }
        let associative_memory_min_similarity =
            associative_memory_min_similarity.unwrap_or(DEFAULT_ASSOCIATIVE_MEMORY_MIN_SIMILARITY);
        if !(associative_memory_min_similarity.is_finite()
            && 0.0 < associative_memory_min_similarity
            && associative_memory_min_similarity <= 1.0)
        {
            return Err(PyValueError::new_err(
                "associative_memory_min_similarity must be finite, greater than 0.0, and at most 1.0",
            ));
        }
        if !reflection_salience_threshold.is_finite() || reflection_salience_threshold <= 0.0 {
            return Err(PyValueError::new_err(
                "reflection_salience_threshold must be finite and greater than zero",
            ));
        }
        if !fidelity_auto_validate_weight_threshold.is_finite()
            || !(0.0..=1.0).contains(&fidelity_auto_validate_weight_threshold)
        {
            return Err(PyValueError::new_err(
                "fidelity_auto_validate_weight_threshold must be finite and between 0.0 and 1.0",
            ));
        }
        if !reflection_critical_unit_weight.is_finite()
            || !(0.0..=1.0).contains(&reflection_critical_unit_weight)
        {
            return Err(PyValueError::new_err(
                "reflection_critical_unit_weight must be finite and between 0.0 and 1.0",
            ));
        }
        if !reflection_max_interval_days.is_finite() || reflection_max_interval_days <= 0.0 {
            return Err(PyValueError::new_err(
                "reflection_max_interval_days must be finite and greater than zero",
            ));
        }
        if reflection_scheduled_min_units == 0
            || reflection_max_trigger_units == 0
            || reflection_max_related_units == 0
            || reflection_min_related_score == 0
            || reflection_max_existing_candidates == 0
        {
            return Err(PyValueError::new_err(
                "reflection unit limits must be greater than zero",
            ));
        }
        let path = PathBuf::from(memory_dir);
        let storage = FileStorage::with_host_id(&path, host_id);
        storage.ensure_layout().map_err(map_err)?;
        let mut options = EngineOptions::default();
        options.vectors.stage2_candidate_limit = stage2_candidate_limit;
        options.vectors.stage2_vector_weight = stage2_vector_weight;
        options.vectors.contextual_expansion_min_sim = associative_memory_min_similarity;
        options.fidelity.auto_validate_weight_threshold = fidelity_auto_validate_weight_threshold;
        options.reflection.salience_threshold = reflection_salience_threshold;
        options.reflection.critical_unit_weight = reflection_critical_unit_weight;
        options.reflection.max_interval_days = reflection_max_interval_days;
        options.reflection.scheduled_min_units = reflection_scheduled_min_units;
        options.reflection.max_trigger_units = reflection_max_trigger_units;
        options.reflection.max_related_units = reflection_max_related_units;
        options.reflection.min_related_score = reflection_min_related_score;
        options.reflection.max_existing_candidates = reflection_max_existing_candidates;
        Ok(Self {
            inner: CoreEngine::with_options(storage, options),
        })
    }

    fn ingest(&self, py: Python<'_>, event_json: &str) -> PyResult<String> {
        let event: IngestEvent = parse_json(event_json, "event")?;
        let stored = run_without_gil(py, || self.inner.ingest(event))?;
        dump_json(&stored, "stored event")
    }

    fn ingest_idempotent(
        &self,
        py: Python<'_>,
        operation_id: &str,
        event_json: &str,
    ) -> PyResult<String> {
        let event: IngestEvent = parse_json(event_json, "event")?;
        let result = run_without_gil(py, || self.inner.ingest_idempotent(operation_id, event))?;
        dump_json(&result, "idempotent ingest result")
    }

    #[pyo3(signature = (session_id, personality_scope_id=None))]
    fn write_generation(
        &self,
        py: Python<'_>,
        session_id: &str,
        personality_scope_id: Option<&str>,
    ) -> PyResult<String> {
        let generation = run_without_gil(py, || {
            self.inner
                .write_generation(session_id, personality_scope_id)
        })?;
        dump_json(&generation, "write generation")
    }

    fn ingest_idempotent_at_generation(
        &self,
        py: Python<'_>,
        operation_id: &str,
        event_json: &str,
        generation_json: &str,
    ) -> PyResult<String> {
        let event: IngestEvent = parse_json(event_json, "event")?;
        let generation: WriteGeneration = parse_json(generation_json, "write generation")?;
        let result = run_without_gil(py, || {
            self.inner
                .ingest_idempotent_at_generation(operation_id, event, generation)
        })?;
        dump_json(&result, "idempotent ingest result")
    }

    fn sleep(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let result = run_without_gil(py, || self.inner.sleep(session_id))?;
        dump_json(&result, "sleep result")
    }

    fn begin_sleep_run(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let run = run_without_gil(py, || self.inner.begin_sleep_run(session_id))?;
        dump_json(&run, "sleep run")
    }

    fn begin_sleep_run_with_options(
        &self,
        py: Python<'_>,
        session_id: &str,
        options_json: &str,
    ) -> PyResult<String> {
        let options: SleepRunOptions = parse_json(options_json, "sleep run options")?;
        let run = run_without_gil(py, || {
            self.inner.begin_sleep_run_with_options(session_id, options)
        })?;
        dump_json(&run, "sleep run")
    }

    fn next_sleep_batch(&self, py: Python<'_>, run_json: &str) -> PyResult<String> {
        let run: SleepRun = parse_json(run_json, "sleep run")?;
        let step = run_without_gil(py, || self.inner.next_sleep_batch(run))?;
        dump_json(&step, "sleep run step")
    }

    fn submit_sleep_batch(
        &self,
        py: Python<'_>,
        run_json: &str,
        responses_json: &str,
    ) -> PyResult<String> {
        let run: SleepRun = parse_json(run_json, "sleep run")?;
        let responses: Vec<LlmResponse> = parse_json(responses_json, "LLM responses")?;
        let step = run_without_gil(py, || self.inner.submit_sleep_batch(run, responses))?;
        dump_json(&step, "sleep run step")
    }

    fn finish_sleep_run(&self, py: Python<'_>, run_json: &str) -> PyResult<String> {
        let run: SleepRun = parse_json(run_json, "sleep run")?;
        let outcome = run_without_gil(py, || self.inner.finish_sleep_run(run))?;
        dump_json(&outcome, "sleep outcome")
    }

    fn finish_sleep_run_with_available(&self, py: Python<'_>, run_json: &str) -> PyResult<String> {
        let run: SleepRun = parse_json(run_json, "sleep run")?;
        let outcome = run_without_gil(py, || self.inner.finish_sleep_run_with_available(run))?;
        dump_json(&outcome, "sleep outcome")
    }

    fn pending_sleep_runs(&self, py: Python<'_>) -> PyResult<String> {
        let runs = run_without_gil(py, || self.inner.pending_sleep_runs())?;
        dump_json(&runs, "pending sleep runs")
    }

    fn pending_memory_unit_repairs(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let requests = run_without_gil(py, || self.inner.pending_memory_unit_repairs(session_id))?;
        dump_json(&requests, "memory unit repair requests")
    }

    fn submit_memory_unit_repair_response(
        &self,
        py: Python<'_>,
        task_id: &str,
        response_json: &str,
    ) -> PyResult<String> {
        let response: LlmResponse = parse_json(response_json, "memory unit repair response")?;
        let updated = run_without_gil(py, || {
            self.inner
                .submit_memory_unit_repair_response(task_id, response)
        })?;
        dump_json(&updated, "memory unit repair result")
    }

    fn cancel_sleep_run(&self, py: Python<'_>, sleep_task_id: &str) -> PyResult<String> {
        let run = run_without_gil(py, || self.inner.cancel_sleep_run(sleep_task_id))?;
        dump_json(&run, "sleep run")
    }

    fn build_evidence_pack(&self, py: Python<'_>, memory_unit_id: &str) -> PyResult<String> {
        let pack = run_without_gil(py, || self.inner.build_evidence_pack(memory_unit_id))?;
        dump_json(&pack, "evidence pack")
    }

    fn begin_memory_fidelity_pass(&self, py: Python<'_>, memory_unit_id: &str) -> PyResult<String> {
        let start = run_without_gil(py, || self.inner.begin_memory_fidelity_pass(memory_unit_id))?;
        dump_json(&start, "memory fidelity pass")
    }

    fn submit_memory_fidelity_response(
        &self,
        py: Python<'_>,
        task_id: &str,
        response_json: &str,
    ) -> PyResult<String> {
        let response: LlmResponse = parse_json(response_json, "LLM response")?;
        let unit = run_without_gil(py, || {
            self.inner
                .submit_memory_fidelity_response(task_id, response)
        })?;
        dump_json(&unit, "memory unit")
    }

    fn begin_forget_review(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let start = run_without_gil(py, || self.inner.begin_forget_review(session_id))?;
        dump_json(&start, "forget review")
    }

    fn submit_forget_review_response(
        &self,
        py: Python<'_>,
        task_id: &str,
        response_json: &str,
    ) -> PyResult<String> {
        let response: LlmResponse = parse_json(response_json, "LLM response")?;
        let result = run_without_gil(py, || {
            self.inner.submit_forget_review_response(task_id, response)
        })?;
        dump_json(&result, "forget review result")
    }

    fn resume_forget_review(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<String> {
        let result: ForgetReviewResult = parse_json(result_json, "forget review result")?;
        let applied = run_without_gil(py, || self.inner.resume_forget_review(task_id, result))?;
        dump_json(&applied, "forget review result")
    }

    fn list_forgotten_memory_units(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let result = run_without_gil(py, || self.inner.list_forgotten_memory_units(session_id))?;
        dump_json(&result, "forgotten memory units")
    }

    fn remember_back(&self, py: Python<'_>, memory_unit_id: &str) -> PyResult<String> {
        let unit = run_without_gil(py, || self.inner.remember_back(memory_unit_id))?;
        dump_json(&unit, "memory unit")
    }

    #[pyo3(signature = (session_id, core_scope=None))]
    fn begin_reflection_analysis(
        &self,
        py: Python<'_>,
        session_id: &str,
        core_scope: Option<String>,
    ) -> PyResult<String> {
        let start = run_without_gil(py, || {
            self.inner.begin_reflection_analysis(session_id, core_scope)
        })?;
        dump_json(&start, "reflection pass")
    }

    #[pyo3(signature = (session_id, core_scope=None))]
    fn bind_reflection_session(
        &self,
        py: Python<'_>,
        session_id: &str,
        core_scope: Option<String>,
    ) -> PyResult<String> {
        let state = run_without_gil(py, || {
            self.inner.bind_reflection_session(session_id, core_scope)
        })?;
        dump_json(&state, "reflection state")
    }

    #[pyo3(signature = (session_id, core_scope=None, opportunity="after_sleep"))]
    fn reflection_status(
        &self,
        py: Python<'_>,
        session_id: &str,
        core_scope: Option<String>,
        opportunity: &str,
    ) -> PyResult<String> {
        let opportunity = parse_reflection_opportunity(opportunity)?;
        let status = run_without_gil(py, || {
            self.inner
                .reflection_status(session_id, core_scope, opportunity)
        })?;
        dump_json(&status, "reflection trigger status")
    }

    #[pyo3(signature = (session_id, core_scope=None, opportunity="after_sleep"))]
    fn begin_reflection_if_due(
        &self,
        py: Python<'_>,
        session_id: &str,
        core_scope: Option<String>,
        opportunity: &str,
    ) -> PyResult<String> {
        let opportunity = parse_reflection_opportunity(opportunity)?;
        let start = run_without_gil(py, || {
            self.inner
                .begin_reflection_if_due(session_id, core_scope, opportunity)
        })?;
        dump_json(&start, "automatic reflection start")
    }

    fn submit_reflection_response(
        &self,
        py: Python<'_>,
        task_id: &str,
        response_json: &str,
    ) -> PyResult<String> {
        let response: LlmResponse = parse_json(response_json, "LLM response")?;
        let result = run_without_gil(py, || {
            self.inner.submit_reflection_response(task_id, response)
        })?;
        dump_json(&result, "reflection candidates")
    }

    fn resume_reflection_analysis(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<String> {
        let result: ReflectionAnalyzeResult = parse_json(result_json, "reflection result")?;
        let candidates = run_without_gil(py, || {
            self.inner.resume_reflection_analysis(task_id, result)
        })?;
        dump_json(&candidates, "reflection candidates")
    }

    fn resume_reflection_actions(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<String> {
        let result: ReflectionActionsResult = parse_json(result_json, "reflection actions result")?;
        let candidates =
            run_without_gil(py, || self.inner.resume_reflection_actions(task_id, result))?;
        dump_json(&candidates, "reflection action results")
    }

    fn list_candidates(&self, py: Python<'_>) -> PyResult<String> {
        let candidates = run_without_gil(py, || self.inner.list_candidates())?;
        dump_json(&candidates, "candidate beliefs")
    }

    fn migrate_hypothesis_graph(&self, py: Python<'_>) -> PyResult<String> {
        let result = run_without_gil(py, || self.inner.migrate_hypothesis_graph())?;
        dump_json(&result, "hypothesis graph migration result")
    }

    fn export_hypothesis_graph(&self, py: Python<'_>, request_json: &str) -> PyResult<String> {
        let request: HypothesisGraphExportRequest =
            parse_json(request_json, "hypothesis graph export request")?;
        let result = run_without_gil(py, || self.inner.export_hypothesis_graph(request))?;
        dump_json(&result, "hypothesis graph export")
    }

    fn review_candidate(&self, py: Python<'_>, review_json: &str) -> PyResult<String> {
        let input: CandidateReviewInput = parse_json(review_json, "candidate review input")?;
        let result = run_without_gil(py, || self.inner.review_candidate(input))?;
        dump_json(&result, "candidate review result")
    }

    fn recover_pending_memory_operations(&self, py: Python<'_>) -> PyResult<usize> {
        run_without_gil(py, || self.inner.recover_pending_memory_operations())
    }

    fn delete_session(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let report = run_without_gil(py, || self.inner.delete_session(session_id))?;
        dump_json(&report, "session deletion report")
    }

    fn purge_scope(&self, py: Python<'_>, personality_scope_id: &str) -> PyResult<String> {
        let report = run_without_gil(py, || self.inner.purge_scope(personality_scope_id))?;
        dump_json(&report, "scope purge report")
    }

    fn resume_memory_fidelity_pass(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<String> {
        let result: FidelityReview = parse_json(result_json, "fidelity review")?;
        let unit = run_without_gil(py, || {
            self.inner.resume_memory_fidelity_pass(task_id, result)
        })?;
        dump_json(&unit, "memory unit")
    }

    fn read_session(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let session = run_without_gil(py, || self.inner.read_session(session_id))?;
        dump_json(&session, "session")
    }

    fn resume_sleep_compression(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<String> {
        let result: SleepCompressionResult = parse_json(result_json, "sleep compression result")?;
        let updated = run_without_gil(py, || self.inner.resume_sleep_compression(task_id, result))?;
        dump_json(&updated, "archive entry")
    }

    fn resume_compact_memory_pass(
        &self,
        py: Python<'_>,
        task_id: &str,
        compact_memory: &str,
    ) -> PyResult<String> {
        let updated = run_without_gil(py, || {
            self.inner
                .resume_compact_memory_pass(task_id, compact_memory)
        })?;
        dump_json(&updated, "archive entry")
    }

    fn resume_memory_unit_pass(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<String> {
        let result: MemoryUnitPassResult = parse_json(result_json, "memory unit pass result")?;
        let updated = run_without_gil(py, || self.inner.resume_memory_unit_pass(task_id, result))?;
        dump_json(&updated, "archive entry")
    }

    fn recall(&self, py: Python<'_>, query_json: &str) -> PyResult<String> {
        let (query, notes): (RecallQuery, _) = parse_query_json(query_json, "recall query")?;
        let mut result = run_without_gil(py, || self.inner.recall(query))?;
        result.append_notes(notes);
        dump_json(&result, "recall result")
    }

    fn flush_recall_stats(&self, py: Python<'_>) -> PyResult<usize> {
        run_without_gil(py, || self.inner.flush_recall_stats())
    }

    fn core_context_package(&self, py: Python<'_>, request_json: &str) -> PyResult<String> {
        let (request, notes): (CoreContextRequest, _) =
            parse_query_json(request_json, "core context request")?;
        let mut package = run_without_gil(py, || self.inner.core_context_package(request))?;
        package.notes.extend(notes);
        dump_json(&package, "core context package")
    }

    fn render_memory_view(
        &self,
        py: Python<'_>,
        package_json: &str,
        current_user_message: &str,
    ) -> PyResult<String> {
        let package: CoreContextPackage = parse_json(package_json, "core context package")?;
        run_without_gil(py, || {
            Ok(::memory_engine::render_memory_view(
                &package,
                current_user_message,
            ))
        })
    }

    fn render_memory_prompt_plan(
        &self,
        package_json: &str,
        current_user_message: &str,
    ) -> PyResult<String> {
        let package: CoreContextPackage = parse_json(package_json, "core context package")?;
        let plan = ::memory_engine::render_memory_prompt_plan(&package, current_user_message);
        dump_json(&plan, "memory prompt plan")
    }

    fn upsert_core_fact(&self, py: Python<'_>, fact_json: &str) -> PyResult<String> {
        let fact: CoreFactInput = parse_json(fact_json, "core fact input")?;
        let result = run_without_gil(py, || self.inner.upsert_core_fact(fact))?;
        dump_json(&result, "core fact upsert result")
    }

    fn patch_core_fact(&self, py: Python<'_>, patch_json: &str) -> PyResult<String> {
        let patch: CoreFactPatchInput = parse_json(patch_json, "core fact patch input")?;
        let result = run_without_gil(py, || self.inner.patch_core_fact(patch))?;
        dump_json(&result, "core fact patch result")
    }

    fn pending_tasks(&self, py: Python<'_>) -> PyResult<String> {
        let tasks = run_without_gil(py, || self.inner.pending_tasks())?;
        dump_json(&tasks, "pending tasks")
    }

    fn seed_core_from_archives(&self, py: Python<'_>) -> PyResult<String> {
        let summary = run_without_gil(py, || self.inner.seed_core_from_archives())?;
        dump_json(&summary, "core archive seed summary")
    }

    fn vector_state(&self, py: Python<'_>, scope: &str) -> PyResult<String> {
        let state = run_without_gil(py, || self.inner.vector_state(scope))?;
        dump_json(&state, "vector scope state")
    }

    #[pyo3(signature = (scope, enabled, purge=false))]
    fn set_vector_scope(
        &self,
        py: Python<'_>,
        scope: &str,
        enabled: bool,
        purge: bool,
    ) -> PyResult<String> {
        let state = run_without_gil(py, || self.inner.set_vector_scope(scope, enabled, purge))?;
        dump_json(&state, "vector scope state")
    }

    fn rebuild_vectors(&self, py: Python<'_>, scope: &str) -> PyResult<String> {
        let state = run_without_gil(py, || self.inner.rebuild_vectors(scope))?;
        dump_json(&state, "vector scope state")
    }

    fn pending_embedding_backfill(&self, py: Python<'_>, scope: &str) -> PyResult<String> {
        let requests = run_without_gil(py, || self.inner.pending_embedding_backfill(scope))?;
        dump_json(&requests, "embedding requests")
    }

    fn resume_compute_embedding(
        &self,
        py: Python<'_>,
        task_id: &str,
        result_json: &str,
    ) -> PyResult<usize> {
        let result: EmbedBatchResult = parse_json(result_json, "embed batch result")?;
        run_without_gil(py, || self.inner.resume_compute_embedding(task_id, result))
    }

    fn recall_deep(&self, py: Python<'_>, query_json: &str) -> PyResult<String> {
        let query: DeepRecallQuery = parse_json(query_json, "deep recall query")?;
        let result = run_without_gil(py, || self.inner.recall_deep(query))?;
        dump_json(&result, "deep recall result")
    }
}

fn run_without_gil<T, F>(py: Python<'_>, f: F) -> PyResult<T>
where
    T: Send,
    F: FnOnce() -> std::result::Result<T, MemoryEngineError> + Send,
{
    py.allow_threads(f).map_err(map_err)
}

fn parse_json<T: DeserializeOwned>(raw: &str, what: &str) -> PyResult<T> {
    serde_json::from_str(raw)
        .map_err(|err| PyValueError::new_err(format!("Invalid {what} JSON: {err}")))
}

fn parse_query_json<T: DeserializeOwned>(raw: &str, what: &str) -> PyResult<(T, Vec<String>)> {
    ::memory_engine::query_json::from_str(raw)
        .map_err(|err| PyValueError::new_err(format!("Invalid {what} JSON: {err}")))
}

fn dump_json<T: Serialize>(value: &T, what: &str) -> PyResult<String> {
    serde_json::to_string(value)
        .map_err(|err| PyRuntimeError::new_err(format!("Failed to serialize {what}: {err}")))
}

fn parse_reflection_opportunity(raw: &str) -> PyResult<ReflectionOpportunity> {
    match raw.trim() {
        "after_sleep" => Ok(ReflectionOpportunity::AfterSleep),
        "scheduled_maintenance" => Ok(ReflectionOpportunity::ScheduledMaintenance),
        other => Err(PyValueError::new_err(format!(
            "invalid reflection opportunity: {other}"
        ))),
    }
}

fn map_err(err: MemoryEngineError) -> PyErr {
    match err {
        MemoryEngineError::Validation(msg) => PyValueError::new_err(msg),
        MemoryEngineError::IncompatibleSchema { expected, actual } => PyValueError::new_err(
            format!("Incompatible schema: expected {expected}, got {actual}"),
        ),
        error @ MemoryEngineError::OperationConflict => {
            OperationConflictError::new_err(error.to_string())
        }
        error @ MemoryEngineError::OperationResultUnavailable => {
            OperationResultUnavailableError::new_err(error.to_string())
        }
        error @ MemoryEngineError::OperationIncomplete => {
            OperationIncompleteError::new_err(error.to_string())
        }
        error @ MemoryEngineError::OperationStale => {
            OperationStaleError::new_err(error.to_string())
        }
        error @ MemoryEngineError::NeedsNewReview => {
            NeedsNewReviewError::new_err(error.to_string())
        }
        other => PyRuntimeError::new_err(other.to_string()),
    }
}

#[pyfunction]
fn build_info(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let info = ::memory_engine::build_info();
    let fields = PyDict::new_bound(py);
    fields.set_item("version", info.version)?;
    fields.set_item("commit", info.commit)?;
    fields.set_item("dirty", info.dirty)?;
    Ok(fields)
}

#[pymodule(name = "memory_engine")]
fn memory_engine_py(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyMemoryEngine>()?;
    module.add(
        "OperationConflictError",
        py.get_type_bound::<OperationConflictError>(),
    )?;
    module.add(
        "OperationResultUnavailableError",
        py.get_type_bound::<OperationResultUnavailableError>(),
    )?;
    module.add(
        "OperationIncompleteError",
        py.get_type_bound::<OperationIncompleteError>(),
    )?;
    module.add(
        "OperationStaleError",
        py.get_type_bound::<OperationStaleError>(),
    )?;
    module.add(
        "NeedsNewReviewError",
        py.get_type_bound::<NeedsNewReviewError>(),
    )?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_function(wrap_pyfunction!(build_info, module)?)?;
    Ok(())
}
