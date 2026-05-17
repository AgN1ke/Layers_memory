use memory_engine::archive::ArchiveStatus;
use memory_engine::event::IngestEvent;
use memory_engine::recall::{RecallFilters, RecallQuery};
use memory_engine::sleep::SleepCompressionResult;
use memory_engine::types::{
    EVENT_SCHEMA_VERSION, RECALL_QUERY_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn engine_sleep_creates_preliminary_archive_and_pending_task() {
    let root = unique_temp_dir("engine_sleep_creates_preliminary_archive_and_pending_task");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Я живу в Берліні і хочу, щоб ти це памʼятав.",
        vec!["personal_fact", "location"],
    );
    ingest_text(
        &mut engine,
        "2026-05-17T16:01:00.000Z",
        "Мені подобається працювати зранку.",
        vec!["preference"],
    );

    let sleep_result = engine.sleep("live_session").expect("sleep stage1");

    assert!(sleep_result
        .archive_entry
        .archive_id
        .starts_with("archive_"));
    assert_eq!(
        sleep_result.archive_entry.status,
        ArchiveStatus::Preliminary
    );
    assert_eq!(sleep_result.archive_entry.source_session_id, "live_session");
    assert_eq!(sleep_result.archive_entry.source_event_ids.len(), 2);
    assert!(sleep_result.archive_entry.gist.contains("Берліні"));
    assert_eq!(sleep_result.pending_task.prompt_id, "sleep_compression");
    assert_eq!(
        sleep_result.pending_task.expected_output_schema,
        SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION
    );

    let tasks = engine.pending_tasks().expect("pending tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id, sleep_result.pending_task.task_id);

    assert!(root
        .join("tasks")
        .join(format!("{}.json", sleep_result.pending_task.task_id))
        .exists());

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_recall_finds_archived_memory_by_text() {
    let root = unique_temp_dir("engine_recall_finds_archived_memory_by_text");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Я живу в Берліні і часто питаю про місцевий контекст.",
        vec!["personal_fact", "location"],
    );
    engine.sleep("live_session").expect("sleep stage1");

    let result = engine
        .recall(RecallQuery {
            schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            query_id: Some("recall_test".to_string()),
            created_at: Some("2026-05-17T17:00:00.000Z".to_string()),
            session_id: Some("live_session".to_string()),
            context: json!({ "recent_text": "Що ти памʼятаєш про моє місто?" }),
            query_text: Some("Берлін місто".to_string()),
            filters: RecallFilters::default(),
            limit: 3,
            include_core: false,
            explain: true,
        })
        .expect("recall");

    assert_eq!(result.items.len(), 1);
    assert!(result.items[0].gist.contains("Берліні"));
    assert!(result.items[0].relevance_score > 0.0);
    assert!(result.items[0].relevance_explanation.is_some());

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_resume_sleep_compression_updates_archive_and_completes_task() {
    let root =
        unique_temp_dir("engine_resume_sleep_compression_updates_archive_and_completes_task");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Я живу в Берліні.",
        vec!["personal_fact", "location"],
    );
    let sleep_result = engine.sleep("live_session").expect("sleep stage1");

    let updated = engine
        .resume_sleep_compression(
            &sleep_result.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                gist: "Користувач живе в Берліні.".to_string(),
                narrative: "Користувач прямо повідомив, що живе в Берліні.".to_string(),
                facts: vec![],
                quotes: vec![],
                tags: vec!["personal_fact".to_string(), "location".to_string()],
                theme: Some("personal_background".to_string()),
                weight: 0.9,
                links: vec![],
            },
        )
        .expect("resume sleep compression");

    assert_eq!(updated.status, ArchiveStatus::Complete);
    assert!(updated.llm_enhanced);
    assert_eq!(updated.prompt_id.as_deref(), Some("sleep_compression"));
    assert!(engine.pending_tasks().expect("pending tasks").is_empty());

    fs::remove_dir_all(root).ok();
}

fn ingest_text(
    engine: &mut MemoryEngine<FileStorage>,
    timestamp: &str,
    text: &str,
    tags: Vec<&str>,
) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "terminal_user".to_string(),
            timestamp: timestamp.to_string(),
            session_id: "live_session".to_string(),
            payload: json!({ "text": text }),
            tags: tags.into_iter().map(str::to_string).collect(),
            theme: Some("personal_background".to_string()),
            emotional_tone: None,
            links: vec![],
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest text");
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
