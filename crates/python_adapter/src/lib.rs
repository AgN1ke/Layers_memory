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
use serde::de::DeserializeOwned;
use serde::Serialize;

use ::memory_engine::archive::FidelityReview;
use ::memory_engine::core_store::{
    CandidateReviewInput, CoreContextPackage, CoreContextRequest, CoreFactInput, CoreFactPatchInput,
};
use ::memory_engine::event::IngestEvent;
use ::memory_engine::forgetting::ForgetReviewResult;
use ::memory_engine::llm::{LlmResponse, SleepRun};
use ::memory_engine::recall::RecallQuery;
use ::memory_engine::reflection::ReflectionAnalyzeResult;
use ::memory_engine::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use ::memory_engine::storage::Storage;
use ::memory_engine::vector::{DeepRecallQuery, EmbedBatchResult};
use ::memory_engine::{EngineOptions, FileStorage, MemoryEngine as CoreEngine, MemoryEngineError};

#[pyclass(name = "MemoryEngine")]
pub struct PyMemoryEngine {
    inner: CoreEngine<FileStorage>,
}

#[pymethods]
impl PyMemoryEngine {
    #[new]
    #[pyo3(signature = (memory_dir, host_id="unknown"))]
    fn new(memory_dir: &str, host_id: &str) -> PyResult<Self> {
        let path = PathBuf::from(memory_dir);
        let storage = FileStorage::with_host_id(&path, host_id);
        storage.ensure_layout().map_err(map_err)?;
        Ok(Self {
            inner: CoreEngine::with_options(storage, EngineOptions::default()),
        })
    }

    fn ingest(&self, py: Python<'_>, event_json: &str) -> PyResult<String> {
        let event: IngestEvent = parse_json(event_json, "event")?;
        let stored = run_without_gil(py, || self.inner.ingest(event))?;
        dump_json(&stored, "stored event")
    }

    fn sleep(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let result = run_without_gil(py, || self.inner.sleep(session_id))?;
        dump_json(&result, "sleep result")
    }

    fn begin_sleep_run(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let run = run_without_gil(py, || self.inner.begin_sleep_run(session_id))?;
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

    fn pending_sleep_runs(&self, py: Python<'_>) -> PyResult<String> {
        let runs = run_without_gil(py, || self.inner.pending_sleep_runs())?;
        dump_json(&runs, "pending sleep runs")
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

    fn list_candidates(&self, py: Python<'_>) -> PyResult<String> {
        let candidates = run_without_gil(py, || self.inner.list_candidates())?;
        dump_json(&candidates, "candidate beliefs")
    }

    fn review_candidate(&self, py: Python<'_>, review_json: &str) -> PyResult<String> {
        let input: CandidateReviewInput = parse_json(review_json, "candidate review input")?;
        let result = run_without_gil(py, || self.inner.review_candidate(input))?;
        dump_json(&result, "candidate review result")
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
        let session = run_without_gil(py, || self.inner.storage().read_session(session_id))?;
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
        let query: RecallQuery = parse_json(query_json, "recall query")?;
        let result = run_without_gil(py, || self.inner.recall(query))?;
        dump_json(&result, "recall result")
    }

    fn flush_recall_stats(&self, py: Python<'_>) -> PyResult<usize> {
        run_without_gil(py, || self.inner.flush_recall_stats())
    }

    fn core_context_package(&self, py: Python<'_>, request_json: &str) -> PyResult<String> {
        let request: CoreContextRequest = parse_json(request_json, "core context request")?;
        let package = run_without_gil(py, || self.inner.core_context_package(request))?;
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

fn dump_json<T: Serialize>(value: &T, what: &str) -> PyResult<String> {
    serde_json::to_string(value)
        .map_err(|err| PyRuntimeError::new_err(format!("Failed to serialize {what}: {err}")))
}

fn map_err(err: MemoryEngineError) -> PyErr {
    match err {
        MemoryEngineError::Validation(msg) => PyValueError::new_err(msg),
        MemoryEngineError::IncompatibleSchema { expected, actual } => PyValueError::new_err(
            format!("Incompatible schema: expected {expected}, got {actual}"),
        ),
        other => PyRuntimeError::new_err(other.to_string()),
    }
}

#[pymodule(name = "memory_engine")]
fn memory_engine_py(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyMemoryEngine>()?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
