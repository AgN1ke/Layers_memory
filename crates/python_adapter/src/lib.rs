//! Python adapter for Memory Engine.
//!
//! Thin PyO3 wrapper. Accepts JSON strings on the boundary, converts to
//! Rust structs from `memory_engine`, runs the operation, returns JSON.
//!
//! No LLM, no provider, no model selection lives here. The Python caller
//! receives `PendingTask` objects in the returned JSON and is fully
//! responsible for executing them with whatever provider it chooses, then
//! submitting the result back through `resume_sleep_compression`.

// PyO3 0.22 `#[pymethods]` expansion produces an `Into<PyErr>` step that
// clippy 1.95 flags as `useless_conversion`. Silencing this lint locally
// while the upstream fix lands.
#![allow(clippy::useless_conversion)]

use std::path::PathBuf;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;

use ::memory_engine::event::IngestEvent;
use ::memory_engine::recall::RecallQuery;
use ::memory_engine::sleep::SleepCompressionResult;
use ::memory_engine::{FileStorage, MemoryEngine as CoreEngine, MemoryEngineError};

#[pyclass(name = "MemoryEngine", unsendable)]
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
            inner: CoreEngine::new(storage),
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

    fn resume_sleep_compression(&mut self, task_id: &str, result_json: &str) -> PyResult<String> {
        let result: SleepCompressionResult = parse_json(result_json, "sleep compression result")?;
        let updated = self
            .inner
            .resume_sleep_compression(task_id, result)
            .map_err(map_err)?;
        dump_json(&updated, "archive entry")
    }

    fn recall(&mut self, query_json: &str) -> PyResult<String> {
        let query: RecallQuery = parse_json(query_json, "recall query")?;
        let result = self.inner.recall(query).map_err(map_err)?;
        dump_json(&result, "recall result")
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
