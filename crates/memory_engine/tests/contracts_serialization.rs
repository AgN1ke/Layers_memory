use memory_engine::archive::{
    ArchiveEntry, ArchiveStatus, EmotionalMarker, PersonalSignal, RelationalTone, TopicThreadItem,
};
use memory_engine::event::{IngestEvent, StoredEvent};
use memory_engine::file_storage::FileStorage;
use memory_engine::sleep::SleepCompressionResult;
use memory_engine::storage::Storage;
use memory_engine::tasks::{PendingTask, TaskState, TaskType};
use memory_engine::types::{
    ModelRole, Quote, TimeRange, WeightedFact, ARCHIVE_ENTRY_SCHEMA_VERSION, EVENT_SCHEMA_VERSION,
    PENDING_TASK_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn ingest_event_round_trips_with_contract_names() {
    let raw = json!({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "telegram_user_42",
        "timestamp": "2026-05-17T16:32:11.420Z",
        "session_id": "2026-05-17_005",
        "payload": {
            "text": "Я переїхав у Берлін минулого місяця",
            "chat_id": 42
        },
        "tags": ["personal_fact", "location"],
        "theme": "personal_background",
        "importance_hint": "high"
    });

    let event: IngestEvent = serde_json::from_value(raw).unwrap();
    assert_eq!(event.schema_version, EVENT_SCHEMA_VERSION);
    assert_eq!(event.event_type, "user_message");
    assert_eq!(event.tags, vec!["personal_fact", "location"]);

    let serialized = serde_json::to_value(&event).unwrap();
    assert_eq!(serialized["type"], "user_message");
    assert_eq!(serialized["processing_mode"], "defer_to_sleep");
}

#[test]
fn stored_event_can_be_created_from_ingest_event() {
    let ingest = IngestEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_string(),
        event_type: "user_message".to_string(),
        source: "telegram_user_42".to_string(),
        timestamp: "2026-05-17T16:32:11.420Z".to_string(),
        session_id: "2026-05-17_005".to_string(),
        payload: json!({ "text": "hello" }),
        tags: vec!["test".to_string()],
        theme: None,
        emotional_tone: None,
        links: vec![],
        importance_hint: Default::default(),
        processing_mode: Default::default(),
    };

    let stored = StoredEvent::from_ingest(
        ingest,
        "event_01",
        "2026-05-17T16:32:12.000Z",
        0.5,
        "default weight",
    );

    assert_eq!(stored.event_id, "event_01");
    assert_eq!(stored.initial_weight, 0.5);
}

#[test]
fn archive_entry_serializes_reserved_embedding_fields() {
    let entry = ArchiveEntry {
        schema_version: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
        archive_id: "archive_01".to_string(),
        created_at: "2026-05-17T17:10:00.000Z".to_string(),
        updated_at: "2026-05-17T17:12:00.000Z".to_string(),
        source_session_id: "2026-05-17_005".to_string(),
        source_event_ids: vec!["event_01".to_string()],
        time_range: TimeRange {
            start: "2026-05-17T16:30:00.000Z".to_string(),
            end: "2026-05-17T17:00:00.000Z".to_string(),
        },
        theme: Some("personal_background".to_string()),
        tags: vec!["personal_fact".to_string()],
        gist: "Користувач переїхав у Берлін.".to_string(),
        narrative: "Користувач повідомив важливий особистий факт.".to_string(),
        compact_memory: Some(
            "Переїзд у Берлін — користувач повідомив стабільний особистий контекст.".to_string(),
        ),
        facts: vec![WeightedFact {
            text: "Користувач живе в Берліні.".to_string(),
            confidence: 0.8,
            source_event_ids: vec!["event_01".to_string()],
        }],
        quotes: vec![Quote {
            text: "Я переїхав у Берлін минулого місяця".to_string(),
            source_event_id: Some("event_01".to_string()),
        }],
        weight: 0.82,
        freshness: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        links: vec![],
        emotional_markers: vec![EmotionalMarker {
            target: "cat_named_irzha".to_string(),
            affect: "fondness".to_string(),
            strength: 0.95,
            source_event_ids: vec!["event_01J00000000000000000000001".to_string()],
            quote: Some("У мене є кішечка".to_string()),
            evidence: Some("Користувач тепло представив домашню тварину.".to_string()),
        }],
        topic_thread: vec![TopicThreadItem {
            topic: "personal_pet".to_string(),
            subtopics: vec!["cat_name".to_string()],
            energy: Some("warm".to_string()),
            source_event_ids: vec!["event_01J00000000000000000000001".to_string()],
            summary: Some("Користувач розповів про свою кішку.".to_string()),
        }],
        personal_signals: vec![PersonalSignal {
            text: "Користувач має кішку.".to_string(),
            category: "relationships_with_pets".to_string(),
            confidence: 0.9,
            source_event_ids: vec!["event_01J00000000000000000000001".to_string()],
            evidence: Some("Пряма заява користувача.".to_string()),
        }],
        relational_tone: Some(RelationalTone {
            warmth: Some(0.8),
            intellectual_engagement: None,
            intimacy: Some(0.4),
            trust: None,
            playfulness: Some(0.5),
            tension: None,
            summary: Some("Розмова стала теплішою після особистої згадки.".to_string()),
            source_event_ids: vec!["event_01J00000000000000000000001".to_string()],
        }),
        status: ArchiveStatus::Preliminary,
        llm_enhanced: false,
        prompt_id: None,
        prompt_version: None,
        embedding_model_id: None,
        embedding: None,
    };

    let value = serde_json::to_value(entry).unwrap();
    assert_eq!(value["status"], "preliminary");
    assert!(value["embedding_model_id"].is_null());
    assert_eq!(value["emotional_markers"][0]["target"], "cat_named_irzha");
    assert_eq!(
        value["personal_signals"][0]["category"],
        "relationships_with_pets"
    );
}

#[test]
fn pending_task_uses_model_role_not_provider() {
    let task = PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: "task_01".to_string(),
        task_type: TaskType::SleepCompression,
        state: TaskState::Pending,
        created_at: "2026-05-17T17:05:00.000Z".to_string(),
        updated_at: "2026-05-17T17:05:00.000Z".to_string(),
        prompt_id: "sleep_compression".to_string(),
        prompt_version: 1,
        role_hint: ModelRole::Balanced,
        expected_output_schema: "sleep_compression_result.v1".to_string(),
        inputs: json!({ "session_id": "2026-05-17_005" }),
        attempts: vec![],
        last_error: None,
    };

    let value = serde_json::to_value(task).unwrap();
    assert_eq!(value["task_type"], "sleep_compression");
    assert_eq!(value["role_hint"], "balanced");
    assert!(value.get("provider").is_none());
    assert!(value.get("model").is_none());
}

#[test]
fn sleep_compression_result_validates_basic_shape() {
    let result = SleepCompressionResult {
        schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
        archive_id: "archive_01".to_string(),
        gist: "Короткий зміст.".to_string(),
        narrative: "Людський наратив спогаду.".to_string(),
        compact_memory: Some("Подія → короткий людський висновок.".to_string()),
        facts: vec![],
        quotes: vec![],
        tags: vec!["test".to_string()],
        theme: None,
        weight: 0.7,
        links: vec![],
        emotional_markers: vec![],
        topic_thread: vec![],
        personal_signals: vec![],
        relational_tone: None,
    };

    result.validate_basic().unwrap();
}

#[test]
fn file_storage_appends_and_reads_session_events() {
    let root = unique_temp_dir("file_storage_appends_and_reads_session_events");
    let mut storage = FileStorage::with_host_id(&root, "telegram_bot");

    let ingest = IngestEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_string(),
        event_type: "user_message".to_string(),
        source: "telegram_user_42".to_string(),
        timestamp: "2026-05-17T16:32:11.420Z".to_string(),
        session_id: "2026-05-17_005".to_string(),
        payload: json!({ "text": "hello" }),
        tags: vec!["test".to_string()],
        theme: Some("test_theme".to_string()),
        emotional_tone: None,
        links: vec![],
        importance_hint: Default::default(),
        processing_mode: Default::default(),
    };

    let stored = StoredEvent::from_ingest(
        ingest,
        "event_01",
        "2026-05-17T16:32:12.000Z",
        0.5,
        "default weight",
    );

    storage
        .append_event("2026-05-17_005", &stored)
        .expect("append event");

    let session = storage
        .read_session("2026-05-17_005")
        .expect("read session");

    assert_eq!(session.metadata.event_count, 1);
    assert_eq!(session.metadata.host_id, "telegram_bot");
    assert_eq!(session.metadata.active_theme.as_deref(), Some("test_theme"));
    assert_eq!(session.events.len(), 1);
    assert_eq!(session.events[0].event_id, "event_01");
    assert!(root
        .join("sessions")
        .join("2026-05-17_005")
        .join("session.md")
        .exists());

    fs::remove_dir_all(root).ok();
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
