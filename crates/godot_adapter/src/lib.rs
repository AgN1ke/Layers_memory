//! Godot GDExtension adapter for Memory Engine.
//!
//! This crate is intentionally thin. It mirrors the JSON-string boundary of
//! the Python adapter: Godot passes JSON strings in, the Rust core owns memory
//! policy, and the adapter returns JSON strings back to Godot.

use std::path::PathBuf;

use godot::classes::RefCounted;
use godot::prelude::*;
use memory_engine::archive::FidelityReview;
use memory_engine::core_store::{
    CandidateReviewInput, CoreContextPackage, CoreContextRequest, CoreFactInput, CoreFactPatchInput,
};
use memory_engine::event::IngestEvent;
use memory_engine::forgetting::ForgetReviewResult;
use memory_engine::llm::{LlmResponse, SleepRun};
use memory_engine::recall::RecallQuery;
use memory_engine::reflection::ReflectionAnalyzeResult;
use memory_engine::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::{EngineOptions, FileStorage, MemoryEngine as CoreEngine};
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct MemoryEngineGodot {
    inner: Option<CoreEngine<FileStorage>>,
    base: Base<RefCounted>,
}

#[godot_api]
impl IRefCounted for MemoryEngineGodot {
    fn init(base: Base<RefCounted>) -> Self {
        Self { inner: None, base }
    }
}

#[godot_api]
impl MemoryEngineGodot {
    #[func]
    fn open(&mut self, memory_dir: GString, host_id: GString) -> GString {
        let path = PathBuf::from(memory_dir.to_string());
        let storage = FileStorage::with_host_id(&path, host_id.to_string());
        match storage.ensure_layout() {
            Ok(()) => {
                self.inner = Some(CoreEngine::with_options(storage, EngineOptions::default()));
                ok_json("opened")
            }
            Err(err) => err_json(err.to_string()),
        }
    }

    #[func]
    fn ingest(&self, event_json: GString) -> GString {
        self.with_engine(|engine| {
            let event: IngestEvent = parse_json(&event_json.to_string(), "event")?;
            let stored = engine.ingest(event).map_err(|err| err.to_string())?;
            dump_json(&stored, "stored event")
        })
    }

    #[func]
    fn begin_sleep_run(&self, session_id: GString) -> GString {
        self.with_engine(|engine| {
            let run = engine
                .begin_sleep_run(&session_id.to_string())
                .map_err(|err| err.to_string())?;
            dump_json(&run, "sleep run")
        })
    }

    #[func]
    fn next_sleep_batch(&self, run_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            let step = engine
                .next_sleep_batch(run)
                .map_err(|err| err.to_string())?;
            dump_json(&step, "sleep run step")
        })
    }

    #[func]
    fn submit_sleep_batch(&self, run_json: GString, responses_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            let responses: Vec<LlmResponse> =
                parse_json(&responses_json.to_string(), "LLM responses")?;
            let step = engine
                .submit_sleep_batch(run, responses)
                .map_err(|err| err.to_string())?;
            dump_json(&step, "sleep run step")
        })
    }

    #[func]
    fn finish_sleep_run(&self, run_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            let outcome = engine
                .finish_sleep_run(run)
                .map_err(|err| err.to_string())?;
            dump_json(&outcome, "sleep outcome")
        })
    }

    #[func]
    fn core_context_package(&self, request_json: GString) -> GString {
        self.with_engine(|engine| {
            let request: CoreContextRequest =
                parse_json(&request_json.to_string(), "core context request")?;
            let package = engine
                .core_context_package(request)
                .map_err(|err| err.to_string())?;
            dump_json(&package, "core context package")
        })
    }

    #[func]
    fn render_memory_view(&self, package_json: GString, current_user_message: GString) -> GString {
        self.with_engine(|engine| {
            let package: CoreContextPackage =
                parse_json(&package_json.to_string(), "core context package")?;
            let _ = engine;
            Ok(memory_engine::render_memory_view(
                &package,
                &current_user_message.to_string(),
            ))
        })
    }

    #[func]
    fn recall(&self, query_json: GString) -> GString {
        self.with_engine(|engine| {
            let query: RecallQuery = parse_json(&query_json.to_string(), "recall query")?;
            let result = engine.recall(query).map_err(|err| err.to_string())?;
            dump_json(&result, "recall result")
        })
    }

    #[func]
    fn upsert_core_fact(&self, fact_json: GString) -> GString {
        self.with_engine(|engine| {
            let fact: CoreFactInput = parse_json(&fact_json.to_string(), "core fact input")?;
            let stored = engine
                .upsert_core_fact(fact)
                .map_err(|err| err.to_string())?;
            dump_json(&stored, "core fact")
        })
    }

    #[func]
    fn patch_core_fact(&self, patch_json: GString) -> GString {
        self.with_engine(|engine| {
            let patch: CoreFactPatchInput = parse_json(&patch_json.to_string(), "core fact patch")?;
            let stored = engine
                .patch_core_fact(patch)
                .map_err(|err| err.to_string())?;
            dump_json(&stored, "core fact")
        })
    }

    #[func]
    fn begin_memory_fidelity_pass(&self, memory_unit_id: GString) -> GString {
        self.with_engine(|engine| {
            let start = engine
                .begin_memory_fidelity_pass(&memory_unit_id.to_string())
                .map_err(|err| err.to_string())?;
            dump_json(&start, "memory fidelity pass")
        })
    }

    #[func]
    fn submit_memory_fidelity_response(&self, task_id: GString, response_json: GString) -> GString {
        self.with_engine(|engine| {
            let response: LlmResponse = parse_json(&response_json.to_string(), "LLM response")?;
            let unit = engine
                .submit_memory_fidelity_response(&task_id.to_string(), response)
                .map_err(|err| err.to_string())?;
            dump_json(&unit, "memory unit")
        })
    }

    #[func]
    fn resume_memory_fidelity_pass(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: FidelityReview =
                parse_json(&result_json.to_string(), "memory fidelity result")?;
            let unit = engine
                .resume_memory_fidelity_pass(&task_id.to_string(), result)
                .map_err(|err| err.to_string())?;
            dump_json(&unit, "memory unit")
        })
    }

    #[func]
    fn begin_reflection_analysis(&self, session_id: GString, core_scope: GString) -> GString {
        self.with_engine(|engine| {
            let scope = optional_gstring(core_scope);
            let start = engine
                .begin_reflection_analysis(&session_id.to_string(), scope)
                .map_err(|err| err.to_string())?;
            dump_json(&start, "reflection pass")
        })
    }

    #[func]
    fn submit_reflection_response(&self, task_id: GString, response_json: GString) -> GString {
        self.with_engine(|engine| {
            let response: LlmResponse = parse_json(&response_json.to_string(), "LLM response")?;
            let candidates = engine
                .submit_reflection_response(&task_id.to_string(), response)
                .map_err(|err| err.to_string())?;
            dump_json(&candidates, "reflection candidates")
        })
    }

    #[func]
    fn resume_reflection_analysis(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: ReflectionAnalyzeResult =
                parse_json(&result_json.to_string(), "reflection result")?;
            let candidates = engine
                .resume_reflection_analysis(&task_id.to_string(), result)
                .map_err(|err| err.to_string())?;
            dump_json(&candidates, "reflection candidates")
        })
    }

    #[func]
    fn list_candidates(&self) -> GString {
        self.with_engine(|engine| {
            let candidates = engine.list_candidates().map_err(|err| err.to_string())?;
            dump_json(&candidates, "candidate beliefs")
        })
    }

    #[func]
    fn review_candidate(&self, review_json: GString) -> GString {
        self.with_engine(|engine| {
            let input: CandidateReviewInput =
                parse_json(&review_json.to_string(), "candidate review input")?;
            let result = engine
                .review_candidate(input)
                .map_err(|err| err.to_string())?;
            dump_json(&result, "candidate review result")
        })
    }

    #[func]
    fn begin_forget_review(&self, session_id: GString) -> GString {
        self.with_engine(|engine| {
            let start = engine
                .begin_forget_review(&session_id.to_string())
                .map_err(|err| err.to_string())?;
            dump_json(&start, "forget review")
        })
    }

    #[func]
    fn submit_forget_review_response(&self, task_id: GString, response_json: GString) -> GString {
        self.with_engine(|engine| {
            let response: LlmResponse = parse_json(&response_json.to_string(), "LLM response")?;
            let result = engine
                .submit_forget_review_response(&task_id.to_string(), response)
                .map_err(|err| err.to_string())?;
            dump_json(&result, "forget review result")
        })
    }

    #[func]
    fn resume_forget_review(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: ForgetReviewResult =
                parse_json(&result_json.to_string(), "forget review result")?;
            let applied = engine
                .resume_forget_review(&task_id.to_string(), result)
                .map_err(|err| err.to_string())?;
            dump_json(&applied, "forget review result")
        })
    }

    #[func]
    fn list_forgotten_memory_units(&self, session_id: GString) -> GString {
        self.with_engine(|engine| {
            let result = engine
                .list_forgotten_memory_units(&session_id.to_string())
                .map_err(|err| err.to_string())?;
            dump_json(&result, "forgotten memory units")
        })
    }

    #[func]
    fn remember_back(&self, memory_unit_id: GString) -> GString {
        self.with_engine(|engine| {
            let unit = engine
                .remember_back(&memory_unit_id.to_string())
                .map_err(|err| err.to_string())?;
            dump_json(&unit, "memory unit")
        })
    }

    #[func]
    fn resume_sleep_compression(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: SleepCompressionResult =
                parse_json(&result_json.to_string(), "sleep compression result")?;
            let archive = engine
                .resume_sleep_compression(&task_id.to_string(), result)
                .map_err(|err| err.to_string())?;
            dump_json(&archive, "archive entry")
        })
    }

    #[func]
    fn resume_memory_unit_pass(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: MemoryUnitPassResult =
                parse_json(&result_json.to_string(), "memory unit pass result")?;
            let archive = engine
                .resume_memory_unit_pass(&task_id.to_string(), result)
                .map_err(|err| err.to_string())?;
            dump_json(&archive, "archive entry")
        })
    }

    #[func]
    fn flush_recall_stats(&self) -> GString {
        self.with_engine(|engine| {
            let count = engine.flush_recall_stats().map_err(|err| err.to_string())?;
            dump_json(&count, "recall flush count")
        })
    }

    fn with_engine<F>(&self, f: F) -> GString
    where
        F: FnOnce(&CoreEngine<FileStorage>) -> Result<String, String>,
    {
        match self.inner.as_ref() {
            Some(engine) => match f(engine) {
                Ok(raw) => gstring(raw),
                Err(err) => err_json(err),
            },
            None => err_json("MemoryEngineGodot is not opened"),
        }
    }
}

fn optional_gstring(value: GString) -> Option<String> {
    let value = value.to_string();
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn parse_json<T: DeserializeOwned>(raw: &str, label: &str) -> Result<T, String> {
    serde_json::from_str(raw).map_err(|err| format!("invalid {label} JSON: {err}"))
}

fn dump_json<T: Serialize>(value: &T, label: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("failed to serialize {label}: {err}"))
}

fn ok_json(status: &str) -> GString {
    gstring(serde_json::json!({ "status": status }).to_string())
}

fn err_json(err: impl Into<String>) -> GString {
    gstring(serde_json::json!({ "error": err.into() }).to_string())
}

fn gstring(value: impl AsRef<str>) -> GString {
    value.as_ref().into()
}

struct MemoryEngineGodotExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MemoryEngineGodotExtension {}
