//! Python adapter for Memory Engine.
//!
//! Thin PyO3 wrapper. Accepts JSON strings on the boundary, converts to
//! Rust structs from `memory_engine`, runs the operation, returns JSON.
//!
//! No LLM, no provider, no model selection lives here. The Python caller
//! receives `PendingTask` objects in the returned JSON and is fully
//! responsible for executing them with whatever provider it chooses, then
//! submitting results back through `resume_sleep_compression` or
//! `resume_compact_memory_pass`.

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
use ::memory_engine::recall::RecallQuery;
use ::memory_engine::sleep::SleepCompressionResult;
use ::memory_engine::storage::Storage;
use ::memory_engine::{EngineOptions, FileStorage, MemoryEngine as CoreEngine, MemoryEngineError};

#[pyclass(name = "MemoryEngine", unsendable)]
pub struct PyMemoryEngine {
    inner: CoreEngine<FileStorage>,
}

#[pymethods]
impl PyMemoryEngine {
    #[new]
    #[pyo3(signature = (memory_dir, host_id="unknown", auto_sleep_after_events=None))]
    fn new(
        memory_dir: &str,
        host_id: &str,
        auto_sleep_after_events: Option<usize>,
    ) -> PyResult<Self> {
        let path = PathBuf::from(memory_dir);
        let storage = FileStorage::with_host_id(&path, host_id);
        storage.ensure_layout().map_err(map_err)?;
        let mut options = EngineOptions::default();
        if let Some(after_events) = auto_sleep_after_events {
            options.auto_sleep.enabled = after_events > 0;
            options.auto_sleep.after_events = after_events;
        }
        Ok(Self {
            inner: CoreEngine::with_options(storage, options),
        })
    }

    fn ingest(&mut self, event_json: &str) -> PyResult<String> {
        let event: IngestEvent = parse_json(event_json, "event")?;
        let stored = self.inner.ingest(event).map_err(map_err)?;
        dump_json(&stored, "stored event")
    }

    fn sleep(&mut self, session_id: &str) -> PyResult<String> {
        let result = self.inner.sleep(session_id).map_err(map_err)?;
        dump_json(&result, "sleep result")
    }

    fn read_session(&self, session_id: &str) -> PyResult<String> {
        let session = self
            .inner
            .storage()
            .read_session(session_id)
            .map_err(map_err)?;
        dump_json(&session, "session")
    }

    fn resume_sleep_compression(&mut self, task_id: &str, result_json: &str) -> PyResult<String> {
        let result: SleepCompressionResult = parse_json(result_json, "sleep compression result")?;
        let updated = self
            .inner
            .resume_sleep_compression(task_id, result)
            .map_err(map_err)?;
        dump_json(&updated, "archive entry")
    }

    fn resume_compact_memory_pass(
        &mut self,
        task_id: &str,
        compact_memory: &str,
    ) -> PyResult<String> {
        let updated = self
            .inner
            .resume_compact_memory_pass(task_id, compact_memory)
            .map_err(map_err)?;
        dump_json(&updated, "archive entry")
    }

    fn recall(&mut self, query_json: &str) -> PyResult<String> {
        let query: RecallQuery = parse_json(query_json, "recall query")?;
        let result = self.inner.recall(query).map_err(map_err)?;
        dump_json(&result, "recall result")
    }

    fn core_context_package(&mut self, request_json: &str) -> PyResult<String> {
        let request: CoreContextRequest = parse_json(request_json, "core context request")?;
        let package = self.inner.core_context_package(request).map_err(map_err)?;
        dump_json(&package, "core context package")
    }

    fn upsert_core_fact(&mut self, fact_json: &str) -> PyResult<String> {
        let fact: CoreFactInput = parse_json(fact_json, "core fact input")?;
        let result = self.inner.upsert_core_fact(fact).map_err(map_err)?;
        dump_json(&result, "core fact upsert result")
    }

    fn patch_core_fact(&mut self, patch_json: &str) -> PyResult<String> {
        let patch: CoreFactPatchInput = parse_json(patch_json, "core fact patch input")?;
        let result = self.inner.patch_core_fact(patch).map_err(map_err)?;
        dump_json(&result, "core fact patch result")
    }

    fn pending_tasks(&self) -> PyResult<String> {
        let tasks = self.inner.pending_tasks().map_err(map_err)?;
        dump_json(&tasks, "pending tasks")
    }
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
