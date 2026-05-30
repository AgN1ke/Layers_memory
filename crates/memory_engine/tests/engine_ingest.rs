use memory_engine::event::IngestEvent;
use memory_engine::types::{
    EVENT_SCHEMA_VERSION, INGEST_RESULT_SCHEMA_VERSION, SESSION_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn engine_ingest_stores_event_and_updates_session_files() {
    let root = unique_temp_dir("engine_ingest_stores_event_and_updates_session_files");
    let storage = FileStorage::with_host_id(&root, "telegram_bot");
    let mut engine = MemoryEngine::new(storage);

    let ingest_result = engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "telegram_user_42".to_string(),
            timestamp: "2026-05-17T16:32:11.420Z".to_string(),
            session_id: "2026-05-17_005".to_string(),
            payload: json!({ "text": "Я переїхав у Берлін" }),
            tags: vec!["personal_fact".to_string(), "location".to_string()],
            theme: Some("personal_background".to_string()),
            emotional_tone: Some("neutral".to_string()),
            links: vec![],
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest event");
    assert_eq!(ingest_result.schema_version, INGEST_RESULT_SCHEMA_VERSION);
    let stored = ingest_result.stored_event;

    assert!(stored.event_id.starts_with("event_"));
    assert_eq!(stored.schema_version, EVENT_SCHEMA_VERSION);
    assert_eq!(stored.session_id, "2026-05-17_005");
    assert!(stored.initial_weight >= 0.75);
    assert!(stored.weight_reason.contains("high importance floor"));

    let session_dir = root.join("sessions").join("2026-05-17_005");
    let session_json = fs::read_to_string(session_dir.join("session.json")).expect("session json");
    let session_value: serde_json::Value =
        serde_json::from_str(&session_json).expect("parse session json");

    assert_eq!(session_value["schema_version"], SESSION_SCHEMA_VERSION);
    assert_eq!(session_value["host_id"], "telegram_bot");
    assert_eq!(session_value["event_count"], 1);

    let events_jsonl = fs::read_to_string(session_dir.join("events.jsonl")).expect("events jsonl");
    assert_eq!(events_jsonl.lines().count(), 1);
    assert!(events_jsonl.contains(&stored.event_id));

    let session_md = fs::read_to_string(session_dir.join("session.md")).expect("session md");
    assert!(session_md.contains("# Сесія 2026-05-17_005"));
    assert!(session_md.contains("Я переїхав у Берлін"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_ingest_rejects_wrong_schema_version() {
    let root = unique_temp_dir("engine_ingest_rejects_wrong_schema_version");
    let storage = FileStorage::with_host_id(&root, "telegram_bot");
    let mut engine = MemoryEngine::new(storage);

    let error = engine
        .ingest(IngestEvent {
            schema_version: "event.v0".to_string(),
            event_type: "user_message".to_string(),
            source: "telegram_user_42".to_string(),
            timestamp: "2026-05-17T16:32:11.420Z".to_string(),
            session_id: "2026-05-17_005".to_string(),
            payload: json!({ "text": "hello" }),
            tags: vec![],
            theme: None,
            emotional_tone: None,
            links: vec![],
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        })
        .expect_err("schema mismatch");

    assert!(error
        .to_string()
        .contains("incompatible schema version: expected event.v1, got event.v0"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_ingest_does_not_trigger_sleep_from_event_count() {
    let root = unique_temp_dir("engine_ingest_does_not_trigger_sleep_from_event_count");
    let storage = FileStorage::with_host_id(&root, "telegram_bot");
    let mut engine = MemoryEngine::new(storage);

    let mut last = None;
    for index in 0..60 {
        last = Some(ingest_numbered_event(&mut engine, index));
    }

    assert_eq!(
        last.expect("last ingest").stored_event.session_id,
        "event_count_session"
    );
    assert!(engine.pending_tasks().expect("pending tasks").is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_ingest_appends_session_markdown_without_rewriting_header() {
    let root = unique_temp_dir("engine_ingest_appends_session_markdown_without_rewriting_header");
    let storage = FileStorage::with_host_id(&root, "telegram_bot");
    let mut engine = MemoryEngine::new(storage);

    for index in 0..3 {
        engine
            .ingest(IngestEvent {
                schema_version: EVENT_SCHEMA_VERSION.to_string(),
                event_type: "user_message".to_string(),
                source: "telegram_user_42".to_string(),
                timestamp: format!("2026-05-17T16:32:1{index}.420Z"),
                session_id: "append_md_session".to_string(),
                payload: json!({ "text": format!("Повідомлення {index}") }),
                tags: vec!["telegram_message".to_string()],
                theme: Some("telegram_conversation".to_string()),
                emotional_tone: None,
                links: vec![],
                importance_hint: Default::default(),
                processing_mode: Default::default(),
            })
            .expect("ingest event");
    }

    let session_md_path = root
        .join("sessions")
        .join("append_md_session")
        .join("session.md");
    let session_md = fs::read_to_string(session_md_path).expect("session md");
    assert_eq!(session_md.matches("# Сесія append_md_session").count(), 1);
    assert_eq!(session_md.matches("- 2026-05-17T16:32:").count(), 3);
    assert!(session_md.contains("Повідомлення 0"));
    assert!(session_md.contains("Повідомлення 2"));

    fs::remove_dir_all(root).ok();
}

fn ingest_numbered_event(
    engine: &mut MemoryEngine<FileStorage>,
    index: usize,
) -> memory_engine::IngestResult {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "telegram_user_42".to_string(),
            timestamp: format!("2026-05-17T16:32:1{index}.420Z"),
            session_id: "event_count_session".to_string(),
            payload: json!({ "text": format!("Подія {index}") }),
            tags: vec!["telegram_message".to_string()],
            theme: Some("telegram_conversation".to_string()),
            emotional_tone: None,
            links: vec![],
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        })
        .expect("ingest numbered event")
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
