use memory_engine::archive::ArchiveStatus;
use memory_engine::core_store::CoreContextRequest;
use memory_engine::event::IngestEvent;
use memory_engine::llm::{LlmErrorKind, LlmResponse, SleepRunOptions, SleepRunStage, SleepTrack};
use memory_engine::storage::Storage;
use memory_engine::types::{
    CORE_CONTEXT_REQUEST_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION,
};
use memory_engine::{render_memory_prompt_plan, FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn sleep_batch_carries_one_compact_shared_context_for_selected_passes() {
    let root = unique_temp_dir("compact_shared_sleep_context");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        "chat",
        "A concise user-authored memory.",
        json!({
            "text": "A concise user-authored memory.",
            "host_payload": "x".repeat(8_000),
        }),
    );

    let run = engine
        .begin_sleep_run_with_options(
            "chat",
            SleepRunOptions {
                enabled_tracks: vec![SleepTrack::MemoryUnit, SleepTrack::PersonalSignal],
                max_provider_calls: None,
                max_pass_attempts: None,
            },
        )
        .expect("begin selected sleep run");
    let durable_task = storage
        .load_task(&run.sleep_task_id)
        .expect("read durable sleep task");
    let durable_chars = serde_json::to_string(&durable_task.inputs)
        .expect("serialize durable input")
        .chars()
        .count();

    let step = engine.next_sleep_batch(run).expect("read selected batch");
    let batch = step.batch.as_ref().expect("selected extraction batch");
    let shared = batch
        .shared_context
        .as_ref()
        .expect("shared prompt context");
    assert_eq!(shared.event_count, 1);
    assert_eq!(shared.source_event_ids.len(), 1);
    assert!(shared.content.contains("A concise user-authored memory."));
    assert!(!shared.content.contains(&"x".repeat(100)));
    assert!(shared.rendered_chars * 4 < durable_chars);

    let prompt_ids = batch
        .requests
        .iter()
        .map(|request| request.prompt_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        prompt_ids,
        vec!["memory_unit_pass", "sleep_personal_signal_pass"]
    );
    for request in &batch.requests {
        assert_eq!(request.prompt_inputs["source"], "shared_context");
        assert!(request.prompt_inputs.get("events").is_none());
        assert!(request.prompt_inputs.get("sleep_task").is_none());
    }
    assert_eq!(
        step.run.skipped_passes,
        vec![
            "sleep_emotional_pass",
            "sleep_topic_thread_pass",
            "sleep_relational_pass",
            "sleep_consolidator",
        ]
    );

    engine
        .cancel_sleep_run(&step.run.sleep_task_id)
        .expect("cancel selected sleep run");
    fs::remove_dir_all(root).ok();
}

#[test]
fn sleep_call_budget_preserves_completed_work_and_finishes_archive() {
    let root = unique_temp_dir("sleep_provider_budget");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage);
    ingest(
        &engine,
        "chat",
        "The user is building a memory library.",
        json!({"text": "The user is building a memory library."}),
    );

    let run = engine
        .begin_sleep_run_with_options(
            "chat",
            SleepRunOptions {
                max_provider_calls: Some(1),
                ..SleepRunOptions::default()
            },
        )
        .expect("begin budgeted sleep run");
    let first = engine.next_sleep_batch(run).expect("first budgeted batch");
    let batch = first.batch.expect("one-call batch");
    assert_eq!(batch.requests.len(), 1);
    assert_eq!(batch.requests[0].prompt_id, "memory_unit_pass");
    let event_id = batch
        .shared_context
        .as_ref()
        .and_then(|shared| shared.source_event_ids.first())
        .expect("source event id")
        .clone();
    let request_id = batch.requests[0].request_id.clone();
    let archive_id = first.run.archive_id.clone();

    let ready = engine
        .submit_sleep_batch(
            first.run,
            vec![LlmResponse::Ok {
                request_id,
                text: json!({
                    "schema_version": MEMORY_UNITS_RESULT_SCHEMA_VERSION,
                    "archive_id": archive_id,
                    "memory_units": [{
                        "thesis": "The user is building a memory library.",
                        "source_event_ids": [event_id],
                        "weight": 0.75
                    }]
                })
                .to_string(),
            }],
        )
        .expect("submit paid result");
    assert_eq!(ready.run.stage, SleepRunStage::ReadyToFinish);
    assert!(ready.batch.is_none());

    let outcome = engine
        .finish_sleep_run(ready.run)
        .expect("finish budgeted sleep run");
    assert_eq!(outcome.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(outcome.archive_entry.memory_units.len(), 1);
    assert_eq!(outcome.provider_calls_issued, 1);
    assert_eq!(outcome.max_provider_calls, Some(1));
    assert!(outcome.budget_exhausted);
    assert!(outcome.failed_passes.is_empty());
    assert!(outcome
        .skipped_passes
        .contains(&"sleep_consolidator".to_string()));
    assert_eq!(outcome.completion_mode, "budget_exhausted");

    fs::remove_dir_all(root).ok();
}

#[test]
fn sleep_reports_provider_failure_recovered_by_retry() {
    let root = unique_temp_dir("sleep_retry_report");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage);
    ingest(
        &engine,
        "chat",
        "A routine message.",
        json!({"text": "A routine message."}),
    );

    let run = engine
        .begin_sleep_run_with_options(
            "chat",
            SleepRunOptions {
                enabled_tracks: vec![SleepTrack::Emotional],
                max_provider_calls: None,
                max_pass_attempts: Some(2),
            },
        )
        .expect("begin retry-report run");
    let first = engine.next_sleep_batch(run).expect("first emotional batch");
    let request_id = first
        .batch
        .as_ref()
        .expect("first emotional request")
        .requests[0]
        .request_id
        .clone();
    let retry = engine
        .submit_sleep_batch(
            first.run,
            vec![LlmResponse::Err {
                request_id: request_id.clone(),
                kind: LlmErrorKind::ProviderBlocked,
                detail: "provider filter".to_string(),
            }],
        )
        .expect("schedule retry");
    assert!(retry.batch.is_some());
    let ready = engine
        .submit_sleep_batch(
            retry.run,
            vec![LlmResponse::Ok {
                request_id,
                text: json!({"emotional_markers": []}).to_string(),
            }],
        )
        .expect("recover emotional pass");
    let outcome = engine
        .finish_sleep_run(ready.run)
        .expect("finish recovered run");

    assert!(outcome.failed_passes.is_empty());
    assert_eq!(outcome.pass_reports.len(), 1);
    let report = &outcome.pass_reports[0];
    assert_eq!(report.track, SleepTrack::Emotional);
    assert_eq!(report.attempts, 2);
    assert!(report.recovered_after_retry);
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].contains("ProviderBlocked"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn host_can_finalize_early_without_discarding_the_sleep_window() {
    let root = unique_temp_dir("host_sleep_budget_stop");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage);
    ingest(
        &engine,
        "chat",
        "This event must still become archived when the host stops early.",
        json!({"text": "This event must still become archived when the host stops early."}),
    );

    let run = engine.begin_sleep_run("chat").expect("begin sleep run");
    let first = engine.next_sleep_batch(run).expect("first sleep batch");
    assert!(first.batch.is_some());
    let outcome = engine
        .finish_sleep_run_with_available(first.run)
        .expect("finish with available results");
    assert_eq!(outcome.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(outcome.completion_mode, "host_budget_stop");
    assert!(outcome.archive_entry.source_event_ids.len() == 1);
    assert!(outcome
        .skipped_passes
        .contains(&"memory_unit_pass".to_string()));

    fs::remove_dir_all(root).ok();
}

#[test]
fn empty_session_and_structured_prompt_plan_are_host_ready() {
    let root = unique_temp_dir("empty_session_prompt_plan");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage.clone());

    let empty = storage
        .read_session("brand_new_chat")
        .expect("missing session is empty");
    assert_eq!(empty.metadata.event_count, 0);
    assert!(empty.events.is_empty());
    let empty_package = engine
        .core_context_package(context_request("brand_new_chat", "owner"))
        .expect("context for empty session");
    assert!(empty_package.session_recent.is_empty());

    let orphan_dir = root.join("sessions").join("orphan_chat");
    fs::create_dir_all(&orphan_dir).expect("create orphan session fixture");
    fs::write(orphan_dir.join("events.jsonl"), b"{}\n").expect("write orphan event fixture");
    assert!(storage.read_session("orphan_chat").is_err());

    ingest(
        &engine,
        "chat",
        "Earlier question",
        json!({"text": "Earlier question", "telegram_message_id": 41}),
    );
    ingest_event(
        &engine,
        "chat",
        "assistant_message",
        "Earlier answer",
        json!({"text": "Earlier answer", "telegram_message_id": 42}),
    );
    ingest(
        &engine,
        "chat",
        "Current question",
        json!({"text": "Current question", "telegram_message_id": 43}),
    );

    let package = engine
        .core_context_package(context_request("chat", "owner"))
        .expect("context with payload");
    assert_eq!(package.session_recent[0].payload["telegram_message_id"], 41);
    let plan = render_memory_prompt_plan(&package, "Current question");
    assert_eq!(plan.conversation_state, "ongoing");
    assert_eq!(plan.dialogue_events.len(), 2);
    assert_eq!(
        plan.dialogue_events[0].text.as_deref(),
        Some("Earlier question")
    );
    assert_eq!(
        plan.dialogue_events[1].text.as_deref(),
        Some("Earlier answer")
    );
    assert_eq!(plan.dialogue_events[1].payload["telegram_message_id"], 42);
    assert_eq!(plan.current_user_message, "Current question");

    fs::remove_dir_all(root).ok();
}

#[test]
fn structured_prompt_plan_merges_trace_and_recent_without_duplicate_turns() {
    let root = unique_temp_dir("prompt_plan_trace_recent_merge");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage);
    ingest(
        &engine,
        "chat",
        "First question",
        json!({"text": "First question", "ordinal": 1}),
    );
    ingest_event(
        &engine,
        "chat",
        "assistant_message",
        "First answer",
        json!({"text": "First answer", "ordinal": 2}),
    );
    ingest_event_at(
        &engine,
        "chat",
        "user_message",
        "Current question",
        json!({"text": "Current question", "ordinal": 3}),
        "2026-08-23T12:02:00Z",
    );

    let mut request = context_request("chat", "owner");
    request.session_recent_limit = 3;
    request.session_trace_event_limit = 1;
    let package = engine
        .core_context_package(request)
        .expect("context with different recent and trace limits");
    assert_eq!(package.session_recent.len(), 3);
    assert_eq!(package.session_trace.len(), 1);

    let plan = render_memory_prompt_plan(&package, "Current question");
    let dialogue = plan
        .dialogue_events
        .iter()
        .filter_map(|event| event.text.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(dialogue, vec!["First question", "First answer"]);

    fs::remove_dir_all(root).ok();
}

#[test]
fn core_bridge_explains_why_a_signal_was_not_promoted() {
    let root = unique_temp_dir("core_bridge_skip_reason");
    let storage = FileStorage::with_host_id(&root, "production_feedback");
    let engine = MemoryEngine::new(storage);
    ingest(
        &engine,
        "chat",
        "The user may like something.",
        json!({"text": "The user may like something."}),
    );

    let run = engine
        .begin_sleep_run_with_options(
            "chat",
            SleepRunOptions {
                enabled_tracks: vec![SleepTrack::PersonalSignal],
                max_provider_calls: None,
                max_pass_attempts: None,
            },
        )
        .expect("begin personal-signal run");
    let first = engine.next_sleep_batch(run).expect("personal-signal batch");
    let batch = first.batch.expect("personal-signal request");
    let request_id = batch.requests[0].request_id.clone();
    let event_id = batch
        .shared_context
        .as_ref()
        .and_then(|shared| shared.source_event_ids.first())
        .expect("source event id")
        .clone();
    let ready = engine
        .submit_sleep_batch(
            first.run,
            vec![LlmResponse::Ok {
                request_id,
                text: json!({
                    "personal_signals": [{
                        "text": "The user may like something.",
                        "category": "preference",
                        "confidence": 0.60,
                        "source_event_ids": [event_id]
                    }]
                })
                .to_string(),
            }],
        )
        .expect("submit personal signal");
    let outcome = engine
        .finish_sleep_run(ready.run)
        .expect("finish personal-signal run");
    assert_eq!(outcome.core_summary.created, 0);
    assert_eq!(outcome.core_summary.skipped, 1);
    assert_eq!(
        outcome
            .core_summary
            .skip_reasons
            .get("below_confidence_threshold"),
        Some(&1)
    );

    fs::remove_dir_all(root).ok();
}

fn context_request(session_id: &str, scope: &str) -> CoreContextRequest {
    CoreContextRequest {
        schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
        session_id: session_id.to_string(),
        domain_state: json!({}),
        core_scope: Some(scope.to_string()),
        query_text: Some("Current question".to_string()),
        query_embedding: None,
        explain_recall: false,
        recall_limit: 5,
        session_recent_limit: 20,
        session_trace_event_limit: 20,
        include_core: true,
        token_budget: None,
        utc_offset_minutes: 0,
        clock_untrusted: false,
    }
}

fn ingest(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    text: &str,
    payload: serde_json::Value,
) {
    ingest_event(engine, session_id, "user_message", text, payload);
}

fn ingest_event(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    event_type: &str,
    text: &str,
    payload: serde_json::Value,
) {
    ingest_event_at(
        engine,
        session_id,
        event_type,
        text,
        payload,
        "2026-08-23T12:00:00Z",
    );
}

fn ingest_event_at(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    event_type: &str,
    text: &str,
    payload: serde_json::Value,
    timestamp: &str,
) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: event_type.to_string(),
            source: "production_feedback_test".to_string(),
            timestamp: timestamp.to_string(),
            personality_scope_id: Some("owner".to_string()),
            session_id: session_id.to_string(),
            memory_text: Some(text.to_string()),
            payload,
            tags: vec!["production_feedback".to_string()],
            theme: Some("integration".to_string()),
            emotional_tone: None,
            speaker: None,
            quoted_event_ids: None,
            links: Vec::new(),
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest test event");
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
