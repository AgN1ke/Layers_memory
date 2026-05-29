use memory_engine::archive::{
    ArchiveStatus, EmotionalMarker, PersonalSignal, RelationalTone, TopicThreadItem,
};
use memory_engine::core_store::{
    CoreContextRequest, CoreContextTokenBudget, CoreFactInput, CoreFactPatchInput, CoreFactStatus,
};
use memory_engine::event::IngestEvent;
use memory_engine::llm::{LlmResponse, SleepRunStage};
use memory_engine::recall::{RecallFilters, RecallQuery};
use memory_engine::sleep::{MemoryUnitDraft, MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::storage::Storage;
use memory_engine::tasks::TaskType;
use memory_engine::types::{
    CORE_CONTEXT_REQUEST_SCHEMA_VERSION, CORE_FACT_INPUT_SCHEMA_VERSION,
    CORE_FACT_PATCH_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION,
    RECALL_QUERY_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use memory_engine::{EngineOptions, FileStorage, MemoryEngine, SleepStage1Result};
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
    let memory_unit_task = sleep_result
        .memory_unit_task
        .as_ref()
        .expect("memory unit task");
    assert_eq!(memory_unit_task.task_type, TaskType::MemoryUnitPass);
    assert_eq!(memory_unit_task.prompt_id, "memory_unit_pass");
    assert_eq!(
        memory_unit_task.expected_output_schema,
        MEMORY_UNITS_RESULT_SCHEMA_VERSION
    );

    let tasks = engine.pending_tasks().expect("pending tasks");
    assert_eq!(tasks.len(), 2);
    assert!(tasks
        .iter()
        .any(|task| task.task_id == sleep_result.pending_task.task_id));
    assert!(tasks
        .iter()
        .any(|task| task.task_id == memory_unit_task.task_id));

    assert!(root
        .join("tasks")
        .join(format!("{}.json", sleep_result.pending_task.task_id))
        .exists());
    assert!(root
        .join("tasks")
        .join(format!("{}.json", memory_unit_task.task_id))
        .exists());

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_context_keeps_preliminary_sleep_events_active() {
    let root = unique_temp_dir("engine_context_keeps_preliminary_sleep_events_active");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Ми говорили про кішечку Іржу.",
        vec!["personal_story"],
    );
    engine.sleep("live_session").expect("sleep stage1");

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "Про що ми говорили?" }),
            core_scope: None,
            query_text: Some("Іржа".to_string()),
            recall_limit: 5,
            session_recent_limit: 5,
            session_trace_event_limit: 5,
            include_core: false,
            token_budget: None,
        })
        .expect("core context package");

    assert!(package.archive_relevant.is_empty());
    assert!(package.session_trace.iter().any(|event| event
        .text
        .as_deref()
        .unwrap_or("")
        .contains("Іржу")));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_sleep_uses_unarchived_events_only() {
    let root = unique_temp_dir("engine_sleep_uses_unarchived_events_only");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Перша тема вже була стиснута.",
        vec!["first_topic"],
    );
    let first_sleep = engine.sleep("live_session").expect("first sleep");
    resume_test_sleep(
        &mut engine,
        &first_sleep,
        "Перша тема вже була стиснута.",
        "Користувач говорив про першу тему.",
    );

    ingest_text(
        &mut engine,
        "2026-05-17T16:05:00.000Z",
        "Друга тема має потрапити в наступний сон.",
        vec!["second_topic"],
    );
    let second_sleep = engine.sleep("live_session").expect("second sleep");

    assert!(first_sleep
        .archive_entry
        .source_event_ids
        .iter()
        .all(|event_id| !second_sleep
            .archive_entry
            .source_event_ids
            .contains(event_id)));
    assert!(second_sleep.archive_entry.gist.contains("Друга тема"));
    assert!(!second_sleep.archive_entry.gist.contains("Перша тема"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_sleep_preserves_configured_active_tail() {
    let root = unique_temp_dir("engine_sleep_preserves_configured_active_tail");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut options = EngineOptions::default();
    options.sleep.partial_sleep_min_events = 4;
    options.sleep.active_tail_ratio = 0.25;
    options.sleep.max_events = 20;
    let mut engine = MemoryEngine::with_options(storage, options);

    for index in 0..8 {
        ingest_text(
            &mut engine,
            &format!("2026-05-17T16:{index:02}:00.000Z"),
            &format!("Подія {index}"),
            vec!["rolling_sleep"],
        );
    }

    let sleep_result = engine.sleep("live_session").expect("sleep stage1");
    let task_events = sleep_result.pending_task.inputs["events"]
        .as_array()
        .expect("sleep task events");
    let selected_texts = task_events
        .iter()
        .filter_map(|event| event["payload"]["text"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(selected_texts.len(), 6);
    assert!(selected_texts.iter().any(|text| text.contains("Подія 0")));
    assert!(selected_texts.iter().any(|text| text.contains("Подія 5")));
    assert!(!selected_texts.iter().any(|text| text.contains("Подія 6")));
    assert!(!selected_texts.iter().any(|text| text.contains("Подія 7")));
    resume_test_sleep(
        &mut engine,
        &sleep_result,
        "Стиснуто старші події rolling sleep.",
        "Старша частина сесії була перенесена в архів, активний tail лишився в сесії.",
    );

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "Що зараз активне?" }),
            core_scope: None,
            query_text: Some("Подія".to_string()),
            recall_limit: 5,
            session_recent_limit: 10,
            session_trace_event_limit: 10,
            include_core: false,
            token_budget: None,
        })
        .expect("core context package");

    let active_texts = package
        .session_trace
        .iter()
        .filter_map(|event| event.text.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(active_texts, vec!["Подія 6", "Подія 7"]);

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_sleep_keeps_late_events_when_weights_tie() {
    let root = unique_temp_dir("engine_sleep_keeps_late_events_when_weights_tie");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut options = EngineOptions::default();
    options.sleep.max_events = 4;
    let mut engine = MemoryEngine::with_options(storage, options);

    for index in 0..7 {
        ingest_text(
            &mut engine,
            &format!("2026-05-17T16:0{index}:00.000Z"),
            &format!("Робоча тема номер {index}."),
            vec!["work_topic"],
        );
    }
    ingest_text(
        &mut engine,
        "2026-05-17T16:07:00.000Z",
        "Пізня особиста історія з явним поясненням, чому вона важлива.",
        vec!["personal_story"],
    );

    let sleep_result = engine.sleep("live_session").expect("sleep stage1");
    let task_events = sleep_result.pending_task.inputs["events"]
        .as_array()
        .expect("sleep task events");
    let selected_texts = task_events
        .iter()
        .filter_map(|event| event["payload"]["text"].as_str())
        .collect::<Vec<_>>();

    assert!(selected_texts
        .iter()
        .any(|text| text.contains("Пізня особиста історія")));
    assert!(!selected_texts
        .iter()
        .any(|text| text.contains("Робоча тема номер 0")));

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
    let sleep_result = engine.sleep("live_session").expect("sleep stage1");
    resume_test_sleep(
        &mut engine,
        &sleep_result,
        "Користувач живе в Берліні.",
        "Користувач прямо повідомив, що живе в Берліні і часто питає про місцевий контекст.",
    );

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

    assert!(
        preliminary_recall.items.is_empty(),
        "preliminary archives must not appear in recall before sleep compression is resumed"
    );

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
                compact_memory: None,
                facts: vec![],
                quotes: vec![],
                tags: vec!["personal_fact".to_string(), "location".to_string()],
                theme: Some("personal_background".to_string()),
                weight: 0.95,
                links: vec![],
                emotional_markers: vec![],
                topic_thread: vec![],
                personal_signals: vec![],
                relational_tone: None,
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
        let sleep_result = engine.sleep("live_session").expect("sleep stage1");
        resume_test_sleep(
            &mut engine,
            &sleep_result,
            &format!("Факт номер {index} про Берлін."),
            &format!("Користувач згадав факт номер {index} про Берлін."),
        );
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
fn engine_recall_with_session_id_does_not_leak_other_sessions() {
    let root = unique_temp_dir("engine_recall_with_session_id_does_not_leak_other_sessions");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text_in_session(
        &mut engine,
        "session_a",
        "2026-05-17T16:00:00.000Z",
        "Користувача у цій сесії звати Микита, і це не має протікати в іншу сесію.",
        vec!["name"],
    );
    let sleep_result = engine.sleep("session_a").expect("sleep session_a");
    resume_test_sleep(
        &mut engine,
        &sleep_result,
        "Користувача у session_a звати Микита.",
        "Стабільний спогад із першої сесії про ім'я Микита.",
    );

    ingest_text_in_session(
        &mut engine,
        "session_b",
        "2026-05-17T16:05:00.000Z",
        "Нова сесія почалась без власних архівних спогадів.",
        vec!["fresh_session"],
    );

    let scoped = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "session_b".to_string(),
            domain_state: json!({ "current_text": "Як мене звати?" }),
            core_scope: None,
            query_text: Some("Микита ім'я користувача".to_string()),
            recall_limit: 5,
            session_recent_limit: 5,
            session_trace_event_limit: 5,
            include_core: false,
            token_budget: None,
        })
        .expect("core context package");

    assert!(scoped.archive_relevant.is_empty());

    let global = engine
        .recall(RecallQuery {
            schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            query_id: None,
            created_at: Some("2026-05-17T16:10:00.000Z".to_string()),
            session_id: None,
            context: json!({ "recent_text": "Микита" }),
            query_text: Some("Микита ім'я користувача".to_string()),
            filters: RecallFilters::default(),
            limit: 5,
            include_core: false,
            explain: false,
        })
        .expect("global recall");

    assert_eq!(global.items.len(), 1);
    assert_eq!(
        global.items[0].source_session_id.as_deref(),
        Some("session_a")
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
                compact_memory: None,
                facts: vec![],
                quotes: vec![],
                tags: vec!["aircraft".to_string()],
                theme: Some("aviation".to_string()),
                weight: 0.9,
                links: vec![],
                emotional_markers: vec![],
                topic_thread: vec![],
                personal_signals: vec![],
                relational_tone: None,
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
            token_budget: None,
        })
        .expect("core context package");

    assert_eq!(package.session_recent.len(), 1);
    assert!(package.session_recent[0]
        .text
        .as_deref()
        .unwrap_or("")
        .contains("риболовлю"));
    assert!(!package.session_trace.iter().any(|event| event
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
            category: "name".to_string(),
            scope: Some("telegram_chat_a".to_string()),
            text: "Користувача звати Микита.".to_string(),
            confidence: 0.95,
            tags: vec!["telegram".to_string(), "name".to_string()],
            source_archive_ids: vec![],
            source_candidate_id: None,
        })
        .expect("upsert core fact");

    assert!(result.created);
    assert_eq!(result.category, "name");
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
            token_budget: None,
        })
        .expect("core context package");

    assert!(package
        .core_facts
        .iter()
        .any(|fact| fact.category == "name"));
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
            token_budget: None,
        })
        .expect("core context package");

    assert_eq!(package.core_facts.len(), 1);
    assert_eq!(package.core_facts[0].text, "Користувача звати Аліса.");
    assert_eq!(package.core_facts[0].scope.as_deref(), Some("telegram_2"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_core_context_package_enforces_token_budget_by_layer() {
    let root = unique_temp_dir("engine_core_context_package_enforces_token_budget_by_layer");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Користувач тепло розповів про кішечку Іржу.",
        vec!["personal_pet"],
    );
    let sleep_result = engine.sleep("live_session").expect("sleep stage1");
    engine
        .resume_sleep_compression(
            &sleep_result.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                gist: "Користувач тепло згадав кішечку Іржу.".to_string(),
                narrative: "Емоційний центр спогаду — тепла особиста згадка про кішечку Іржу."
                    .to_string(),
                compact_memory: Some(
                    "Розмова про кішечку Іржу — теплий особистий спогад, важливий для користувача."
                        .to_string(),
                ),
                facts: vec![],
                quotes: vec![],
                tags: vec!["personal_pet".to_string(), "emotional_memory".to_string()],
                theme: Some("personal_pet".to_string()),
                weight: 0.95,
                links: vec![],
                emotional_markers: vec![EmotionalMarker {
                    target: "cat_named_irzha".to_string(),
                    affect: "warmth".to_string(),
                    strength: 0.95,
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                    quote: None,
                    evidence: Some("Тепла особиста згадка.".to_string()),
                }],
                topic_thread: vec![],
                personal_signals: vec![PersonalSignal {
                    text: "Користувач має кішечку на ім'я Іржа.".to_string(),
                    category: "relationships_with_pets".to_string(),
                    confidence: 0.95,
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                    evidence: Some("Пряма заява користувача.".to_string()),
                }],
                relational_tone: None,
            },
        )
        .expect("resume sleep compression");

    for index in 0..12 {
        ingest_text(
            &mut engine,
            &format!("2026-05-17T16:{:02}:00.000Z", index + 1),
            &format!(
                "Активна свіжа тема {index}: користувач обговорює Європу Юпітера, океан під льодом, приливний розігрів і можливість життя."
            ),
            vec!["space_topic"],
        );
    }

    for index in 0..4 {
        engine
            .upsert_core_fact(CoreFactInput {
                schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                category: "profile".to_string(),
                scope: Some("telegram_1".to_string()),
                text: format!(
                    "Стабільний профільний факт {index}: користувач тестує довготривалу памʼять і уважно перевіряє, чи не губляться сенси."
                ),
                confidence: 0.95 - (index as f64 * 0.05),
                tags: vec!["profile".to_string()],
                source_archive_ids: vec![],
                source_candidate_id: None,
            })
            .expect("upsert core fact");
    }

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "Що ти памʼятаєш про Іржу і Європу?" }),
            core_scope: Some("telegram_1".to_string()),
            query_text: Some("Іржа Європа Юпітера".to_string()),
            recall_limit: 5,
            session_recent_limit: 12,
            session_trace_event_limit: 12,
            include_core: true,
            token_budget: Some(CoreContextTokenBudget {
                total_tokens: 1_600,
                current_memory_tokens: 700,
                compressed_memory_tokens: 500,
                core_tokens: 250,
            }),
        })
        .expect("core context package");

    let report = package.budget.as_ref().expect("budget report");
    assert!(!report.budget_exceeded);
    assert!(report.estimated_total_tokens <= report.total_budget_tokens);
    assert!(report.estimated_current_memory_tokens <= report.current_memory_budget_tokens);
    assert!(report.estimated_compressed_memory_tokens <= report.compressed_memory_budget_tokens);
    assert!(report.estimated_core_tokens <= report.core_budget_tokens);
    assert!(report.dropped_session_recent > 0 || report.dropped_session_trace > 0);
    assert!(report.dropped_core_facts > 0);
    assert!(!package.archive_relevant.is_empty());
    assert!(package.archive_relevant[0].gist.contains("Іржу"));
    assert_eq!(
        package.archive_relevant[0].compact_memory.as_deref(),
        Some("Розмова про кішечку Іржу — теплий особистий спогад, важливий для користувача.")
    );
    assert!(package.archive_relevant[0].narrative.is_none());
    assert!(package.archive_relevant[0].facts.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_patch_core_fact_updates_text_and_deprecates_fact() {
    let root = unique_temp_dir("engine_patch_core_fact_updates_text_and_deprecates_fact");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Початок core patch test.",
        vec!["test"],
    );

    let upsert = engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "pet".to_string(),
            scope: Some("telegram_1".to_string()),
            text: "Користувача звати Микита.".to_string(),
            confidence: 0.95,
            tags: vec!["name".to_string()],
            source_archive_ids: vec![],
            source_candidate_id: None,
        })
        .expect("upsert core fact");

    let updated = engine
        .patch_core_fact(CoreFactPatchInput {
            schema_version: CORE_FACT_PATCH_INPUT_SCHEMA_VERSION.to_string(),
            core_fact_id: upsert.fact.core_fact_id.clone(),
            scope: Some("telegram_1".to_string()),
            text: Some("Користувача звати Микита Загамула.".to_string()),
            status: Some(CoreFactStatus::Active),
            confidence: None,
            tags: None,
        })
        .expect("patch core fact text");

    assert_eq!(updated.fact.text, "Користувача звати Микита Загамула.");
    assert_eq!(updated.category, "pet");
    assert_eq!(updated.fact.status, CoreFactStatus::Active);

    let deprecated = engine
        .patch_core_fact(CoreFactPatchInput {
            schema_version: CORE_FACT_PATCH_INPUT_SCHEMA_VERSION.to_string(),
            core_fact_id: upsert.fact.core_fact_id,
            scope: Some("telegram_1".to_string()),
            text: None,
            status: Some(CoreFactStatus::Deprecated),
            confidence: None,
            tags: None,
        })
        .expect("deprecate core fact");

    assert_eq!(deprecated.fact.status, CoreFactStatus::Deprecated);

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({ "current_text": "Як мене звати?" }),
            core_scope: Some("telegram_1".to_string()),
            query_text: Some("ім'я користувача".to_string()),
            recall_limit: 5,
            session_recent_limit: 2,
            session_trace_event_limit: 10,
            include_core: true,
            token_budget: None,
        })
        .expect("core context package");

    assert!(package.core_facts.is_empty());

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

fn resume_test_sleep(
    engine: &mut MemoryEngine<FileStorage>,
    sleep_result: &SleepStage1Result,
    gist: &str,
    narrative: &str,
) {
    engine
        .resume_sleep_compression(
            &sleep_result.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                gist: gist.to_string(),
                narrative: narrative.to_string(),
                compact_memory: None,
                facts: vec![],
                quotes: vec![],
                tags: vec![],
                theme: Some("test_memory".to_string()),
                weight: 0.9,
                links: vec![],
                emotional_markers: vec![],
                topic_thread: vec![],
                personal_signals: vec![],
                relational_tone: None,
            },
        )
        .expect("resume test sleep compression");
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
                compact_memory: None,
                facts: vec![],
                quotes: vec![],
                tags: vec!["personal_fact".to_string(), "location".to_string()],
                theme: Some("personal_background".to_string()),
                weight: 0.9,
                links: vec![],
                emotional_markers: vec![],
                topic_thread: vec![],
                personal_signals: vec![],
                relational_tone: None,
            },
        )
        .expect("resume sleep compression");

    assert_eq!(updated.status, ArchiveStatus::Complete);
    assert!(updated.llm_enhanced);
    assert_eq!(updated.prompt_id.as_deref(), Some("sleep_compression"));
    let unit_updated = engine
        .resume_memory_unit_pass(
            &sleep_result
                .memory_unit_task
                .as_ref()
                .expect("memory unit task")
                .task_id,
            MemoryUnitPassResult {
                schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                memory_units: vec![MemoryUnitDraft {
                    thesis: "Берлін -> користувач повідомив стабільний особистий контекст."
                        .to_string(),
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                    evidence: Some("Користувач прямо сказав, що живе в Берліні.".to_string()),
                    tags: vec!["location".to_string()],
                    weight: 0.9,
                }],
            },
        )
        .expect("resume memory unit pass");
    assert_eq!(
        unit_updated.compact_memory.as_deref(),
        Some("Берлін -> користувач повідомив стабільний особистий контекст.")
    );
    assert_eq!(unit_updated.memory_units.len(), 1);
    assert!(engine.pending_tasks().expect("pending tasks").is_empty());

    let sleep_task_id = &sleep_result.pending_task.task_id;
    let memory_unit_task_id = &sleep_result
        .memory_unit_task
        .as_ref()
        .expect("memory unit task")
        .task_id;
    assert!(!root
        .join("tasks")
        .join(format!("{sleep_task_id}.json"))
        .exists());
    assert!(!root
        .join("tasks")
        .join(format!("{memory_unit_task_id}.json"))
        .exists());
    assert!(root
        .join("tasks")
        .join("completed")
        .join(format!("{sleep_task_id}.json"))
        .exists());
    assert!(root
        .join("tasks")
        .join("completed")
        .join(format!("{memory_unit_task_id}.json"))
        .exists());

    let storage_view = FileStorage::with_host_id(&root, "terminal");
    assert_eq!(
        storage_view
            .load_task(sleep_task_id)
            .expect("load completed sleep task")
            .state,
        memory_engine::tasks::TaskState::Completed
    );
    assert_eq!(
        storage_view
            .load_task(memory_unit_task_id)
            .expect("load completed memory unit task")
            .state,
        memory_engine::tasks::TaskState::Completed
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_sleep_run_driver_finishes_archive_and_seeds_core() {
    let root = unique_temp_dir("engine_sleep_run_driver_finishes_archive_and_seeds_core");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "Мене звати Микита і я дуже люблю космос.",
        vec!["personal_fact", "interest"],
    );

    let mut run = engine
        .begin_sleep_run("live_session")
        .expect("begin sleep run");
    let step = engine.next_sleep_batch(run).expect("first batch");
    run = step.run;
    let batch = step.batch.expect("extraction batch");
    assert_eq!(batch.requests.len(), 5);

    let event_id = batch.requests[0].prompt_inputs["sleep_task"]["events"][0]["event_id"]
        .as_str()
        .expect("event id")
        .to_string();
    let mut responses = Vec::new();
    for request in batch.requests {
        let text = match request.prompt_id.as_str() {
            "memory_unit_pass" => json!({
                "schema_version": MEMORY_UNITS_RESULT_SCHEMA_VERSION,
                "archive_id": run.archive_id,
                "memory_units": [{
                    "thesis": "Користувач прямо назвався Микитою і проявив любов до космосу.",
                    "source_event_ids": [event_id.clone()],
                    "weight": 0.95
                }]
            }),
            "sleep_emotional_pass" => json!({
                "emotional_markers": [{
                    "target": "космос",
                    "affect": "love",
                    "strength": 0.9,
                    "source_event_ids": [event_id.clone()]
                }]
            }),
            "sleep_topic_thread_pass" => json!({
                "topic_thread": [{
                    "topic": "space_interest",
                    "summary": "Користувач сказав, що дуже любить космос.",
                    "source_event_ids": [event_id.clone()]
                }]
            }),
            "sleep_personal_signal_pass" => json!({
                "personal_signals": [{
                    "text": "Користувач любить космос.",
                    "category": "interest",
                    "confidence": 0.95,
                    "source_event_ids": [event_id.clone()]
                }]
            }),
            "sleep_relational_pass" => json!({
                "relational_tone": {
                    "warmth": 0.6,
                    "intellectual_engagement": 0.8,
                    "source_event_ids": [event_id.clone()]
                }
            }),
            other => panic!("unexpected request: {other}"),
        };
        responses.push(LlmResponse::Ok {
            request_id: request.request_id,
            text: text.to_string(),
        });
    }

    let step = engine
        .submit_sleep_batch(run, responses)
        .expect("submit extraction");
    run = step.run;
    let batch = step.batch.expect("consolidator batch");
    assert_eq!(batch.requests.len(), 1);
    assert_eq!(batch.requests[0].prompt_id, "sleep_consolidator");

    let consolidated = json!({
        "schema_version": SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
        "archive_id": run.archive_id,
        "gist": "Користувач сказав, що любить космос.",
        "narrative": "Користувач прямо назвався Микитою і тепло описав любов до космосу.",
        "compact_memory": "Космос -> користувач любить цю тему.",
        "facts": [],
        "quotes": [],
        "tags": ["space_interest"],
        "theme": "space_interest",
        "weight": 0.95,
        "links": [],
        "emotional_markers": [{
            "target": "космос",
            "affect": "love",
            "strength": 0.9,
            "source_event_ids": [event_id.clone()]
        }],
        "topic_thread": [],
        "personal_signals": [{
            "text": "Користувач любить космос.",
            "category": "interest",
            "confidence": 0.95,
            "source_event_ids": [event_id]
        }],
        "relational_tone": null
    });
    let step = engine
        .submit_sleep_batch(
            run,
            vec![LlmResponse::Ok {
                request_id: batch.requests[0].request_id.clone(),
                text: format!("```json\n{consolidated}\n```"),
            }],
        )
        .expect("submit consolidator");
    run = step.run;
    assert_eq!(run.stage, SleepRunStage::ReadyToFinish);

    let outcome = engine.finish_sleep_run(run).expect("finish sleep run");
    assert_eq!(outcome.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(outcome.completion_mode, "consolidated");
    assert_eq!(outcome.core_summary.created, 1);
    assert!(engine.pending_tasks().expect("pending tasks").is_empty());

    let package = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({}),
            core_scope: Some("live_session".to_string()),
            query_text: Some("космос".to_string()),
            recall_limit: 5,
            session_recent_limit: 5,
            session_trace_event_limit: 10,
            include_core: true,
            token_budget: None,
        })
        .expect("context package");
    assert!(package
        .core_facts
        .iter()
        .any(|fact| fact.text == "Користувач любить космос."));

    fs::remove_dir_all(root).ok();
}

#[test]
fn engine_resume_sleep_compression_persists_multi_track_memory() {
    let root = unique_temp_dir("engine_resume_sleep_compression_persists_multi_track_memory");
    let storage = FileStorage::with_host_id(&root, "terminal");
    let mut engine = MemoryEngine::new(storage);

    ingest_text(
        &mut engine,
        "2026-05-17T16:00:00.000Z",
        "У мене є кішечка Іржа, вона мені дуже дорога.",
        vec!["personal_story"],
    );
    let sleep_result = engine.sleep("live_session").expect("sleep stage1");

    let updated = engine
        .resume_sleep_compression(
            &sleep_result.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep_result.archive_entry.archive_id.clone(),
                gist: "Користувач тепло розповів про кішечку Іржу.".to_string(),
                narrative: "Користувач поділився особистим теплим фактом: у нього є кішечка Іржа, яка для нього важлива.".to_string(),
                compact_memory: None,
                facts: vec![],
                quotes: vec![],
                tags: vec!["personal_pet".to_string(), "emotional_memory".to_string()],
                theme: Some("personal_pet".to_string()),
                weight: 0.95,
                links: vec![],
                emotional_markers: vec![EmotionalMarker {
                    target: "cat_named_irzha".to_string(),
                    affect: "fondness".to_string(),
                    strength: 0.95,
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                    quote: Some("У мене є кішечка Іржа".to_string()),
                    evidence: Some("Користувач назвав кішку дорогою для себе.".to_string()),
                }],
                topic_thread: vec![TopicThreadItem {
                    topic: "personal_pet".to_string(),
                    subtopics: vec!["cat_named_irzha".to_string()],
                    energy: Some("warm".to_string()),
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                    summary: Some("Користувач розповів про кішечку.".to_string()),
                }],
                personal_signals: vec![PersonalSignal {
                    text: "Користувач має кішечку на ім'я Іржа.".to_string(),
                    category: "relationships_with_pets".to_string(),
                    confidence: 0.95,
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                    evidence: Some("Пряма заява користувача.".to_string()),
                }],
                relational_tone: Some(RelationalTone {
                    warmth: Some(0.8),
                    intellectual_engagement: None,
                    intimacy: Some(0.5),
                    trust: None,
                    playfulness: None,
                    tension: None,
                    summary: Some("Користувач поділився особистим теплим фактом.".to_string()),
                    source_event_ids: sleep_result.archive_entry.source_event_ids.clone(),
                }),
            },
        )
        .expect("resume sleep compression");

    assert_eq!(updated.emotional_markers.len(), 1);
    assert_eq!(updated.emotional_markers[0].target, "cat_named_irzha");
    assert_eq!(updated.personal_signals.len(), 1);
    assert_eq!(
        updated.personal_signals[0].category,
        "relationships_with_pets"
    );
    assert_eq!(
        updated
            .relational_tone
            .as_ref()
            .and_then(|tone| tone.warmth),
        Some(0.8)
    );

    fs::remove_dir_all(root).ok();
}

fn ingest_text(
    engine: &mut MemoryEngine<FileStorage>,
    timestamp: &str,
    text: &str,
    tags: Vec<&str>,
) {
    ingest_text_in_session(engine, "live_session", timestamp, text, tags);
}

fn ingest_text_in_session(
    engine: &mut MemoryEngine<FileStorage>,
    session_id: &str,
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
            session_id: session_id.to_string(),
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
