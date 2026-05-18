use memory_engine::archive::ArchiveStatus;
use memory_engine::core_store::{CoreContextRequest, CoreFactInput};
use memory_engine::event::IngestEvent;
use memory_engine::recall::{RecallFilters, RecallQuery};
use memory_engine::sleep::SleepCompressionResult;
use memory_engine::types::{
    CORE_CONTEXT_REQUEST_SCHEMA_VERSION, CORE_FACT_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION,
    RECALL_QUERY_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
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
fn engine_recall_returns_complete_entry_after_resume_sleep_compression() {
    let root =
        unique_temp_dir("engine_recall_returns_complete_entry_after_resume_sleep_compression");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Я живу в Берліні.",
        vec!["personal_fact", "location"],
    );
    let sleep_result = engine.sleep("live_session").expect("sleep stage1");

    let preliminary_recall = engine
        .recall(make_recall_query(
            "Берлін",
            "2026-05-17T16:30:00.000Z",
            "recall_preliminary",
        ))
        .expect("preliminary recall");

    assert_eq!(preliminary_recall.items.len(), 1);
    let preliminary_item = &preliminary_recall.items[0];
    assert!(preliminary_item.gist.starts_with("Попередній спогад"));
    assert!(preliminary_item
        .narrative
        .as_deref()
        .unwrap_or("")
        .contains("Попередній архівний спогад"));

    let llm_gist = "Користувач живе в Берліні, Німеччина.";
    let llm_narrative =
        "Користувач прямо повідомив, що проживає в Берліні; це стабільний особистий факт.";

    engine
        .resume_sleep_compression(
            &sleep_result.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                gist: llm_gist.to_string(),
                narrative: llm_narrative.to_string(),
                facts: vec![],
                quotes: vec![],
                tags: vec!["personal_fact".to_string(), "location".to_string()],
                theme: Some("personal_background".to_string()),
                weight: 0.95,
                links: vec![],
            },
        )
        .expect("resume sleep compression");

    let complete_recall = engine
        .recall(make_recall_query(
            "Берлін",
            "2026-05-17T17:30:00.000Z",
            "recall_complete",
        ))
        .expect("complete recall");

    assert_eq!(complete_recall.items.len(), 1);
    let complete_item = &complete_recall.items[0];
    assert_eq!(complete_item.gist, llm_gist);
    assert_eq!(complete_item.narrative.as_deref(), Some(llm_narrative));
    assert!(complete_item.weight >= 0.9);

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_recall_with_zero_limit_uses_engine_default() {
    let root = unique_temp_dir("engine_recall_with_zero_limit_uses_engine_default");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    for index in 0..7 {
        ingest_text(
            &mut engine,
            &format!("2026-05-17T16:0{index}:00.000Z"),
            &format!("Факт номер {index} про Берлін."),
            vec!["personal_fact", "location"],
        );
        engine.sleep("live_session").expect("sleep stage1");
    }

    let result = engine
        .recall(make_recall_query(
            "Берлін",
            "2026-05-17T18:00:00.000Z",
            "recall_zero",
        ))
        .expect("recall zero limit");

    assert_eq!(
        result.items.len(),
        5,
        "limit==0 must fall back to engine default of 5"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_core_context_package_combines_session_and_archive_context() {
    let root = unique_temp_dir("engine_core_context_package_combines_session_and_archive_context");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Ми говорили про МіГ-15.",
        vec!["aircraft"],
    );
    let sleep_result = engine.sleep("live_session").expect("sleep stage1");
    engine
        .resume_sleep_compression(
            &sleep_result.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                gist: "Розмова про МіГ-15.".to_string(),
                narrative: "Користувач питав про радянський винищувач МіГ-15.".to_string(),
                facts: vec![],
                quotes: vec![],
                tags: vec!["aircraft".to_string()],
                theme: Some("aviation".to_string()),
                weight: 0.9,
                links: vec![],
            },
        )
        .expect("resume sleep compression");

    ingest_text(
        &mut engine,
        "2026-05-17T16:05:00.000Z",
        "А тепер говоримо про риболовлю.",
        vec!["fishing"],
    );

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "А про літаки?" }),
            core_scope: None,
            query_text: Some("літаки МіГ-15".to_string()),
            recall_limit: 5,
            session_recent_limit: 2,
            session_trace_event_limit: 10,
            include_core: false,
        })
        .expect("core context package");

    assert_eq!(package.session_recent.len(), 2);
    assert!(package.session_trace.iter().any(|event| event
        .text
        .as_deref()
        .unwrap_or("")
        .contains("МіГ-15")));
    assert_eq!(package.archive_relevant.len(), 1);
    assert_eq!(package.archive_relevant[0].gist, "Розмова про МіГ-15.");

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_upsert_core_fact_adds_stable_fact_to_context_package() {
    let root = unique_temp_dir("engine_upsert_core_fact_adds_stable_fact_to_context_package");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Мене звати Микита.",
        vec!["personal_fact"],
    );

    let result = engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "profile".to_string(),
            scope: Some("telegram_chat_a".to_string()),
            text: "Користувача звати Микита.".to_string(),
            confidence: 0.95,
            tags: vec!["telegram".to_string(), "name".to_string()],
            source_archive_ids: vec![],
            source_candidate_id: None,
        })
        .expect("upsert core fact");

    assert!(result.created);
    assert_eq!(result.category, "profile");
    assert!(result.fact.core_fact_id.starts_with("core_fact_"));

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "Як мене звати?" }),
            core_scope: Some("telegram_chat_a".to_string()),
            query_text: Some("ім'я користувача".to_string()),
            recall_limit: 5,
            session_recent_limit: 2,
            session_trace_event_limit: 10,
            include_core: true,
        })
        .expect("core context package");

    assert!(package
        .core_facts
        .iter()
        .any(|fact| fact.text == "Користувача звати Микита."));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_core_context_package_filters_core_facts_by_scope() {
    let root = unique_temp_dir("engine_core_context_package_filters_core_facts_by_scope");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Початок scoped core test.",
        vec!["test"],
    );

    engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "profile".to_string(),
            scope: Some("telegram_1".to_string()),
            text: "Користувача звати Микита.".to_string(),
            confidence: 0.95,
            tags: vec!["name".to_string()],
            source_archive_ids: vec![],
            source_candidate_id: None,
        })
        .expect("upsert first scoped fact");

    engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "profile".to_string(),
            scope: Some("telegram_2".to_string()),
            text: "Користувача звати Аліса.".to_string(),
            confidence: 0.95,
            tags: vec!["name".to_string()],
            source_archive_ids: vec![],
            source_candidate_id: None,
        })
        .expect("upsert second scoped fact");

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "Як мене звати?" }),
            core_scope: Some("telegram_2".to_string()),
            query_text: Some("ім'я користувача".to_string()),
            recall_limit: 5,
            session_recent_limit: 2,
            session_trace_event_limit: 10,
            include_core: true,
        })
        .expect("core context package");

    assert_eq!(package.core_facts.len(), 1);
    assert_eq!(package.core_facts[0].text, "Користувача звати Аліса.");
    assert_eq!(package.core_facts[0].scope.as_deref(), Some("telegram_2"));

    fs::remove_dir_all(root).ok();
}

fn make_recall_query(query_text: &str, created_at: &str, query_id: &str) -> RecallQuery {
    RecallQuery {
        schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
        query_id: Some(query_id.to_string()),
        created_at: Some(created_at.to_string()),
        session_id: Some("live_session".to_string()),
        context: json!({ "recent_text": query_text }),
        query_text: Some(query_text.to_string()),
        filters: RecallFilters::default(),
        limit: 0,
        include_core: false,
        explain: true,
    }
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
