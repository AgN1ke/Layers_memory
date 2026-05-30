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

use ::memory_engine::core_store::{CoreContextRequest, CoreFactInput, CoreFactPatchInput};
use ::memory_engine::event::IngestEvent;
use ::memory_engine::llm::{LlmResponse, SleepRun};
use ::memory_engine::recall::RecallQuery;
use ::memory_engine::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use ::memory_engine::storage::Storage;
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

    fn core_context_package(&self, py: Python<'_>, request_json: &str) -> PyResult<String> {
        let request: CoreContextRequest = parse_json(request_json, "core context request")?;
        let package = run_without_gil(py, || self.inner.core_context_package(request))?;
        dump_json(&package, "core context package")
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
