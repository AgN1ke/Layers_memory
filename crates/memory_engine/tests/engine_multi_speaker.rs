use memory_engine::archive::ArchiveStatus;
use memory_engine::event::IngestEvent;
use memory_engine::llm::{LlmResponse, SleepOutcome};
use memory_engine::types::{Speaker, EVENT_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION: &str = "multi_speaker_session";

#[test]
fn bridge_skips_high_confidence_signals_for_multi_speaker_sessions() {
    let root = unique_temp_dir("bridge_skips_multi_speaker");
    let engine = MemoryEngine::new(FileStorage::with_host_id(&root, "multi_speaker_test"));

    ingest_speaker_text(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "У Жеки тепер мотоцикл!",
        Some(("tg_101", "Жека")),
    );
    ingest_speaker_text(
        &engine,
        "2026-07-01T10:01:00.000Z",
        "Та ну, він його ще не забрав із салону.",
        Some(("tg_202", "Антон")),
    );

    let outcome = run_sleep_with_signal(&engine, "Zheka bought a motorcycle.", "vehicle");
    assert_eq!(outcome.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(
        outcome.core_summary.created, 0,
        "signals from a multi-speaker session must not reach Core automatically"
    );
    assert!(outcome.core_summary.skipped >= 1);
    assert!(
        outcome.fidelity_requests.is_empty(),
        "no unit is on the automatic Core path while the bridge is disabled"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn bridge_keeps_working_for_single_identified_speaker() {
    let root = unique_temp_dir("bridge_single_speaker");
    let engine = MemoryEngine::new(FileStorage::with_host_id(&root, "multi_speaker_test"));

    ingest_speaker_text(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "Я купив мотоцикл!",
        Some(("tg_101", "Жека")),
    );
    ingest_speaker_text(
        &engine,
        "2026-07-01T10:01:00.000Z",
        "Заберу його завтра з салону.",
        Some(("tg_101", "Жека")),
    );

    let outcome = run_sleep_with_signal(&engine, "Zheka bought a motorcycle.", "vehicle");
    assert_eq!(
        outcome.core_summary.created, 1,
        "a lone identified speaker keeps the legacy single-user bridge"
    );
    assert_eq!(
        outcome.fidelity_requests.len(),
        1,
        "the Core-path unit is still auto-routed to fidelity validation"
    );

    fs::remove_dir_all(root).ok();
}

fn ingest_speaker_text(
    engine: &MemoryEngine<FileStorage>,
    timestamp: &str,
    text: &str,
    speaker: Option<(&str, &str)>,
) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test_group_chat".to_string(),
            timestamp: timestamp.to_string(),
            session_id: SESSION.to_string(),
            payload: json!({ "text": text }),
            tags: vec!["group_chat".to_string()],
            theme: Some("group_chat".to_string()),
            emotional_tone: None,
            speaker: speaker.map(|(id, name)| Speaker {
                id: id.to_string(),
                name: name.to_string(),
            }),
            links: vec![],
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest speaker text");
}

fn run_sleep_with_signal(
    engine: &MemoryEngine<FileStorage>,
    signal_text: &str,
    category: &str,
) -> SleepOutcome {
    let mut run = engine.begin_sleep_run(SESSION).expect("begin sleep run");
    let step = engine.next_sleep_batch(run).expect("first sleep batch");
    run = step.run;
    let batch = step.batch.expect("extraction batch");
    let event_id = batch.requests[0].prompt_inputs["sleep_task"]["events"][0]["event_id"]
        .as_str()
        .expect("event id")
        .to_string();
    let archive_id = run.archive_id.clone();

    let responses = batch
        .requests
        .into_iter()
        .map(|request| {
            let text = match request.prompt_id.as_str() {
                "memory_unit_pass" => json!({
                    "schema_version": MEMORY_UNITS_RESULT_SCHEMA_VERSION,
                    "archive_id": archive_id,
                    "memory_units": [{
                        "thesis": "Motorcycle -> Zheka bought a motorcycle.",
                        "source_event_ids": [event_id.clone()],
                        "evidence": "The chat discussed Zheka's new motorcycle.",
                        "tags": ["group_chat"],
                        "weight": 0.5
                    }]
                }),
                "sleep_emotional_pass" => json!({ "emotional_markers": [] }),
                "sleep_topic_thread_pass" => json!({
                    "topic_thread": [{
                        "topic": "motorcycles",
                        "summary": "Motorcycle purchase discussion.",
                        "source_event_ids": [event_id.clone()]
                    }]
                }),
                "sleep_personal_signal_pass" => json!({
                    "personal_signals": [{
                        "text": signal_text,
                        "category": category,
                        "confidence": 0.95,
                        "source_event_ids": [event_id.clone()]
                    }]
                }),
                "sleep_relational_pass" => json!({ "relational_tone": null }),
                other => panic!("unexpected sleep request: {other}"),
            };
            LlmResponse::Ok {
                request_id: request.request_id,
                text: text.to_string(),
            }
        })
        .collect::<Vec<_>>();

    let step = engine
        .submit_sleep_batch(run, responses)
        .expect("submit extraction");
    run = step.run;
    let batch = step.batch.expect("consolidator batch");
    let step = engine
        .submit_sleep_batch(
            run,
            vec![LlmResponse::Ok {
                request_id: batch.requests[0].request_id.clone(),
                text: "GIST: The chat discussed Zheka's motorcycle.\n\nTwo friends talked about a new motorcycle in the group chat.".to_string(),
            }],
        )
        .expect("submit consolidator");
    run = step.run;

    engine.finish_sleep_run(run).expect("finish sleep run")
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
