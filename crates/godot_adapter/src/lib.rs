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
    CandidateReviewInput, CoreContextPackage, CoreContextRequest, CoreFactAttributionInput,
    CoreFactInput, CoreFactPatchInput,
};
use memory_engine::event::IngestEvent;
use memory_engine::forgetting::ForgetReviewResult;
use memory_engine::hypothesis::HypothesisGraphExportRequest;
use memory_engine::llm::{LlmResponse, SleepRun, SleepRunOptions};
use memory_engine::recall::RecallQuery;
use memory_engine::reflection::{
    ReflectionActionsResult, ReflectionAnalyzeResult, ReflectionOpportunity,
};
use memory_engine::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::{
    EngineOptions, FileStorage, MemoryEngine as CoreEngine, MemoryEngineError, WriteGeneration,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

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
    fn build_info(&self) -> GString {
        match dump_json(&memory_engine::build_info(), "build info") {
            Ok(json) => gstring(json),
            Err(err) => err_json(err),
        }
    }

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
    fn ingest_idempotent(&self, operation_id: GString, event_json: GString) -> GString {
        self.with_engine(|engine| {
            let event: IngestEvent = parse_json(&event_json.to_string(), "event")?;
            let result = engine
                .ingest_idempotent(&operation_id.to_string(), event)
                .map_err(|err| err.to_string())?;
            dump_json(&result, "idempotent ingest result")
        })
    }

    #[func]
    fn write_generation(&self, session_id: GString, personality_scope_id: GString) -> GString {
        self.with_engine(|engine| {
            let scope = optional_gstring(personality_scope_id);
            let generation = engine
                .write_generation(&session_id.to_string(), scope.as_deref())
                .map_err(|err| err.to_string())?;
            dump_json(&generation, "write generation")
        })
    }

    #[func]
    fn ingest_idempotent_at_generation(
        &self,
        operation_id: GString,
        event_json: GString,
        generation_json: GString,
    ) -> GString {
        self.with_engine(|engine| {
            let event: IngestEvent = parse_json(&event_json.to_string(), "event")?;
            let generation: WriteGeneration =
                parse_json(&generation_json.to_string(), "write generation")?;
            dump_operation_result(
                engine.ingest_idempotent_at_generation(
                    &operation_id.to_string(),
                    event,
                    generation,
                ),
                "idempotent ingest result",
            )
        })
    }

    #[func]
    fn begin_sleep_run(&self, session_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(engine.begin_sleep_run(&session_id.to_string()), "sleep run")
        })
    }

    #[func]
    fn begin_sleep_run_with_options(&self, session_id: GString, options_json: GString) -> GString {
        self.with_engine(|engine| {
            let options: SleepRunOptions =
                parse_json(&options_json.to_string(), "sleep run options")?;
            dump_operation_result(
                engine.begin_sleep_run_with_options(&session_id.to_string(), options),
                "sleep run",
            )
        })
    }

    #[func]
    fn next_sleep_batch(&self, run_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            dump_operation_result(engine.next_sleep_batch(run), "sleep run step")
        })
    }

    #[func]
    fn submit_sleep_batch(&self, run_json: GString, responses_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            let responses: Vec<LlmResponse> =
                parse_json(&responses_json.to_string(), "LLM responses")?;
            dump_operation_result(engine.submit_sleep_batch(run, responses), "sleep run step")
        })
    }

    #[func]
    fn finish_sleep_run(&self, run_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            dump_operation_result(engine.finish_sleep_run(run), "sleep outcome")
        })
    }

    #[func]
    fn finish_sleep_run_with_available(&self, run_json: GString) -> GString {
        self.with_engine(|engine| {
            let run: SleepRun = parse_json(&run_json.to_string(), "sleep run")?;
            dump_operation_result(engine.finish_sleep_run_with_available(run), "sleep outcome")
        })
    }

    #[func]
    fn core_context_package(&self, request_json: GString) -> GString {
        self.with_engine(|engine| {
            let (request, notes): (CoreContextRequest, _) =
                parse_query_json(&request_json.to_string(), "core context request")?;
            let mut package = engine
                .core_context_package(request)
                .map_err(|err| err.to_string())?;
            package.notes.extend(notes);
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
    fn render_memory_prompt_plan(
        &self,
        package_json: GString,
        current_user_message: GString,
    ) -> GString {
        self.with_engine(|engine| {
            let package: CoreContextPackage =
                parse_json(&package_json.to_string(), "core context package")?;
            let _ = engine;
            let plan = memory_engine::render_memory_prompt_plan(
                &package,
                &current_user_message.to_string(),
            );
            dump_json(&plan, "memory prompt plan")
        })
    }

    #[func]
    fn recall(&self, query_json: GString) -> GString {
        self.with_engine(|engine| {
            let (query, notes): (RecallQuery, _) =
                parse_query_json(&query_json.to_string(), "recall query")?;
            let mut result = engine.recall(query).map_err(|err| err.to_string())?;
            result.append_notes(notes);
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
    fn correct_core_fact_attribution(&self, input_json: GString) -> GString {
        self.with_engine(|engine| {
            let input: CoreFactAttributionInput =
                parse_json(&input_json.to_string(), "Core fact attribution input")?;
            dump_operation_result(
                engine.correct_core_fact_attribution(input),
                "Core fact attribution result",
            )
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
            dump_operation_result(
                engine.submit_memory_fidelity_response(&task_id.to_string(), response),
                "memory unit",
            )
        })
    }

    #[func]
    fn resume_memory_fidelity_pass(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: FidelityReview =
                parse_json(&result_json.to_string(), "memory fidelity result")?;
            dump_operation_result(
                engine.resume_memory_fidelity_pass(&task_id.to_string(), result),
                "memory unit",
            )
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
    fn bind_reflection_session(&self, session_id: GString, core_scope: GString) -> GString {
        self.with_engine(|engine| {
            let state = engine
                .bind_reflection_session(&session_id.to_string(), optional_gstring(core_scope))
                .map_err(|err| err.to_string())?;
            dump_json(&state, "reflection state")
        })
    }

    #[func]
    fn reflection_status(
        &self,
        session_id: GString,
        core_scope: GString,
        opportunity: GString,
    ) -> GString {
        self.with_engine(|engine| {
            let opportunity = parse_reflection_opportunity(&opportunity.to_string())?;
            let status = engine
                .reflection_status(
                    &session_id.to_string(),
                    optional_gstring(core_scope),
                    opportunity,
                )
                .map_err(|err| err.to_string())?;
            dump_json(&status, "reflection trigger status")
        })
    }

    #[func]
    fn begin_reflection_if_due(
        &self,
        session_id: GString,
        core_scope: GString,
        opportunity: GString,
    ) -> GString {
        self.with_engine(|engine| {
            let opportunity = parse_reflection_opportunity(&opportunity.to_string())?;
            let start = engine
                .begin_reflection_if_due(
                    &session_id.to_string(),
                    optional_gstring(core_scope),
                    opportunity,
                )
                .map_err(|err| err.to_string())?;
            dump_json(&start, "automatic reflection start")
        })
    }

    #[func]
    fn submit_reflection_response(&self, task_id: GString, response_json: GString) -> GString {
        self.with_engine(|engine| {
            let response: LlmResponse = parse_json(&response_json.to_string(), "LLM response")?;
            dump_operation_result(
                engine.submit_reflection_response(&task_id.to_string(), response),
                "reflection candidates",
            )
        })
    }

    #[func]
    fn resume_reflection_analysis(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: ReflectionAnalyzeResult =
                parse_json(&result_json.to_string(), "reflection result")?;
            dump_operation_result(
                engine.resume_reflection_analysis(&task_id.to_string(), result),
                "reflection candidates",
            )
        })
    }

    #[func]
    fn resume_reflection_actions(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: ReflectionActionsResult =
                parse_json(&result_json.to_string(), "reflection actions result")?;
            dump_operation_result(
                engine.resume_reflection_actions(&task_id.to_string(), result),
                "reflection action results",
            )
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
    fn migrate_hypothesis_graph(&self) -> GString {
        self.with_engine(|engine| {
            let result = engine
                .migrate_hypothesis_graph()
                .map_err(|err| err.to_string())?;
            dump_json(&result, "hypothesis graph migration result")
        })
    }

    #[func]
    fn export_hypothesis_graph(&self, request_json: GString) -> GString {
        self.with_engine(|engine| {
            let request: HypothesisGraphExportRequest =
                parse_json(&request_json.to_string(), "hypothesis graph export request")?;
            let result = engine
                .export_hypothesis_graph(request)
                .map_err(|err| err.to_string())?;
            dump_json(&result, "hypothesis graph export")
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
    fn recover_pending_memory_operations(&self) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.recover_pending_memory_operations(),
                "recovered memory operation count",
            )
        })
    }

    #[func]
    fn prune_completed_memory_operations(&self) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.prune_completed_memory_operations(),
                "memory operation prune report",
            )
        })
    }

    #[func]
    fn delete_session(&self, session_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.delete_session(&session_id.to_string()),
                "session deletion report",
            )
        })
    }

    #[func]
    fn purge_scope(&self, personality_scope_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.purge_scope(&personality_scope_id.to_string()),
                "scope purge report",
            )
        })
    }

    #[func]
    fn inspect_scope(&self, scope_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.inspect_scope(&scope_id.to_string()),
                "scoped memory summary",
            )
        })
    }

    #[func]
    fn inspect_archive(&self, scope_id: GString, archive_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.inspect_archive(&scope_id.to_string(), &archive_id.to_string()),
                "scoped archive detail",
            )
        })
    }

    #[func]
    fn inspect_memory_unit(&self, scope_id: GString, memory_unit_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.inspect_memory_unit(&scope_id.to_string(), &memory_unit_id.to_string()),
                "scoped memory unit detail",
            )
        })
    }

    #[func]
    fn inspect_subject(&self, scope_id: GString, subject_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.inspect_subject(&scope_id.to_string(), &subject_id.to_string()),
                "subject memory view",
            )
        })
    }

    #[func]
    fn erase_subject(
        &self,
        scope_id: GString,
        subject_id: GString,
        operation_id: GString,
    ) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.erase_subject(
                    &scope_id.to_string(),
                    &subject_id.to_string(),
                    &operation_id.to_string(),
                ),
                "subject erasure report",
            )
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
            dump_operation_result(
                engine.submit_forget_review_response(&task_id.to_string(), response),
                "forget review result",
            )
        })
    }

    #[func]
    fn resume_forget_review(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: ForgetReviewResult =
                parse_json(&result_json.to_string(), "forget review result")?;
            dump_operation_result(
                engine.resume_forget_review(&task_id.to_string(), result),
                "forget review result",
            )
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
    fn remember_back_in_scope(&self, scope_id: GString, memory_unit_id: GString) -> GString {
        self.with_engine(|engine| {
            dump_operation_result(
                engine.remember_back_in_scope(&scope_id.to_string(), &memory_unit_id.to_string()),
                "scoped memory unit",
            )
        })
    }

    #[func]
    fn resume_sleep_compression(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: SleepCompressionResult =
                parse_json(&result_json.to_string(), "sleep compression result")?;
            dump_operation_result(
                engine.resume_sleep_compression(&task_id.to_string(), result),
                "archive entry",
            )
        })
    }

    #[func]
    fn resume_memory_unit_pass(&self, task_id: GString, result_json: GString) -> GString {
        self.with_engine(|engine| {
            let result: MemoryUnitPassResult =
                parse_json(&result_json.to_string(), "memory unit pass result")?;
            dump_operation_result(
                engine.resume_memory_unit_pass(&task_id.to_string(), result),
                "archive entry",
            )
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

fn parse_reflection_opportunity(raw: &str) -> Result<ReflectionOpportunity, String> {
    match raw.trim() {
        "after_sleep" => Ok(ReflectionOpportunity::AfterSleep),
        "scheduled_maintenance" => Ok(ReflectionOpportunity::ScheduledMaintenance),
        other => Err(format!("invalid reflection opportunity: {other}")),
    }
}

fn parse_json<T: DeserializeOwned>(raw: &str, label: &str) -> Result<T, String> {
    serde_json::from_value(normalized_json(raw, label)?)
        .map_err(|err| format!("invalid {label} JSON: {err}"))
}

fn parse_query_json<T: DeserializeOwned>(
    raw: &str,
    label: &str,
) -> Result<(T, Vec<String>), String> {
    memory_engine::query_json::from_value(normalized_json(raw, label)?)
        .map_err(|err| format!("invalid {label} JSON: {err}"))
}

fn normalized_json(raw: &str, label: &str) -> Result<Value, String> {
    let mut value: Value =
        serde_json::from_str(raw).map_err(|err| format!("invalid {label} JSON: {err}"))?;
    normalize_godot_numbers(&mut value);
    Ok(value)
}

fn dump_json<T: Serialize>(value: &T, label: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("failed to serialize {label}: {err}"))
}

fn dump_operation_result<T: Serialize>(
    result: memory_engine::Result<T>,
    label: &str,
) -> Result<String, String> {
    let error = match result {
        Ok(value) => return dump_json(&value, label),
        Err(error) => error,
    };
    let code = match error {
        MemoryEngineError::OperationStale => "operation_stale",
        MemoryEngineError::NeedsNewReview => "needs_new_review",
        MemoryEngineError::OperationIncomplete => "operation_incomplete",
        MemoryEngineError::OperationConflict => "operation_conflict",
        MemoryEngineError::OperationResultUnavailable => "operation_result_unavailable",
        _ => return Err(error.to_string()),
    };
    Ok(serde_json::json!({"code": code, "error": error.to_string()}).to_string())
}

fn normalize_godot_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.is_f64() => {
            if let Some(number) = number.as_f64() {
                const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;
                if number.is_finite() && number.fract() == 0.0 && number.abs() <= MAX_EXACT_INTEGER
                {
                    *value = if number >= 0.0 {
                        Value::from(number as u64)
                    } else {
                        Value::from(number as i64)
                    };
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_godot_numbers(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                normalize_godot_numbers(item);
            }
        }
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::parse_json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct GodotNumberPayload {
        count: usize,
        retry_limit: u32,
        weight: f64,
        nested: Vec<usize>,
    }

    #[test]
    fn parse_json_normalizes_godot_integer_floats() {
        let parsed: GodotNumberPayload = parse_json(
            r#"{"count":3000.0,"retry_limit":3.0,"weight":0.95,"nested":[1.0,2.0]}"#,
            "godot number payload",
        )
        .expect("Godot integer-shaped floats should deserialize into integer fields");

        assert_eq!(
            parsed,
            GodotNumberPayload {
                count: 3000,
                retry_limit: 3,
                weight: 0.95,
                nested: vec![1, 2],
            }
        );
    }
}
