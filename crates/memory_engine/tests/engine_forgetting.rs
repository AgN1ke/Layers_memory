use memory_engine::archive::{ArchiveStatus, ForgetDecision, MemoryUnitStatus};
use memory_engine::core_store::CoreFactInput;
use memory_engine::event::IngestEvent;
use memory_engine::forgetting::{ForgetRecommendation, ForgetReviewResult};
use memory_engine::llm::LlmResponse;
use memory_engine::sleep::{MemoryUnitDraft, MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::storage::Storage;
use memory_engine::tasks::{TaskState, TaskType};
use memory_engine::types::{
    CORE_FACT_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, FORGET_REVIEW_RESULT_SCHEMA_VERSION,
    MEMORY_UNITS_RESULT_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn forget_review_forgets_low_signal_unit_and_rebuilds_compact_memory() {
    let root = unique_temp_dir("forget_review_forgets_low_signal");
    let storage = FileStorage::with_host_id(&root, "forget_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    let fixture = create_archive_with_units(
        &engine,
        "2020-01-01T10:00:00.000Z",
        vec![
            ("Routine lunch note -> user mentioned soup.", 0.2),
            ("Minor weather note -> user said it rained.", 0.2),
        ],
    );
    let unit_id = fixture.unit_ids[0].clone();

    let start = engine
        .begin_forget_review("live_session")
        .expect("begin forget review");
    assert_eq!(start.pending_task.task_type, TaskType::ForgetReview);
    assert_eq!(start.candidate_count, 2);

    let result = engine
        .submit_forget_review_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id.clone(),
                text: json!({
                    "schema_version": FORGET_REVIEW_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "recommendations": [{
                        "memory_unit_id": unit_id,
                        "decision": "forget",
                        "reason": "Routine detail with little future value."
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit forget review");
    assert_eq!(result.forgotten, 1);
    assert_eq!(result.protected, 0);

    let forgotten = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read forgotten unit");
    assert_eq!(forgotten.status, MemoryUnitStatus::Forgotten);
    let archive = storage_probe
        .read_archive_entry_by_id(&fixture.archive_id)
        .expect("read archive");
    assert_eq!(archive.status, ArchiveStatus::Complete);
    assert!(!archive
        .compact_memory
        .as_deref()
        .unwrap_or("")
        .contains("Routine lunch note"));
    assert!(archive
        .compact_memory
        .as_deref()
        .unwrap_or("")
        .contains("Minor weather note"));

    let restored = engine.remember_back(&unit_id).expect("remember back");
    assert_eq!(restored.status, MemoryUnitStatus::ActiveArchive);
    let archive = storage_probe
        .read_archive_entry_by_id(&fixture.archive_id)
        .expect("read restored archive");
    assert!(archive
        .compact_memory
        .as_deref()
        .unwrap_or("")
        .contains("Routine lunch note"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn forget_review_rechecks_core_link_protection_on_submit() {
    let root = unique_temp_dir("forget_review_rechecks_core_link");
    let storage = FileStorage::with_host_id(&root, "forget_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    let fixture = create_archive_with_units(
        &engine,
        "2020-01-01T10:00:00.000Z",
        vec![("Candidate routine note -> could be forgotten.", 0.2)],
    );
    let unit_id = fixture.unit_ids[0].clone();

    let start = engine
        .begin_forget_review("live_session")
        .expect("begin forget review");
    assert_eq!(start.candidate_count, 1);

    engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "profile".to_string(),
            scope: Some("live_session".to_string()),
            text: "Core-linked fact from the archive.".to_string(),
            confidence: 0.95,
            tags: vec!["test".to_string()],
            source_archive_ids: vec![fixture.archive_id.clone()],
            source_candidate_id: None,
        })
        .expect("seed core link after begin");

    let result = engine
        .submit_forget_review_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id.clone(),
                text: json!({
                    "schema_version": FORGET_REVIEW_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "recommendations": [{
                        "memory_unit_id": unit_id,
                        "decision": "forget",
                        "reason": "The model wants to forget this."
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit forget review");
    assert_eq!(result.forgotten, 0);
    assert_eq!(result.protected, 1);

    let unit = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read protected unit");
    assert_eq!(unit.status, MemoryUnitStatus::ActiveArchive);
    assert!(unit.tags.iter().any(|tag| tag == "forget_protected"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn forget_review_bad_json_marks_task_failed() {
    let root = unique_temp_dir("forget_review_bad_json");
    let storage = FileStorage::with_host_id(&root, "forget_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    create_archive_with_units(
        &engine,
        "2020-01-01T10:00:00.000Z",
        vec![("Routine note -> candidate.", 0.2)],
    );
    let start = engine
        .begin_forget_review("live_session")
        .expect("begin forget review");
    engine
        .submit_forget_review_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id.clone(),
                text: "{not valid json".to_string(),
            },
        )
        .expect_err("bad json should fail");
    let task = storage_probe
        .load_task(&start.pending_task.task_id)
        .expect("load task");
    assert_eq!(task.state, TaskState::Failed);

    fs::remove_dir_all(root).ok();
}

#[test]
fn forget_review_ignores_unknown_recommendations() {
    let root = unique_temp_dir("forget_review_ignores_unknown");
    let storage = FileStorage::with_host_id(&root, "forget_test");
    let engine = MemoryEngine::new(storage);

    create_archive_with_units(
        &engine,
        "2020-01-01T10:00:00.000Z",
        vec![("Routine note -> candidate.", 0.2)],
    );
    let start = engine
        .begin_forget_review("live_session")
        .expect("begin forget review");
    let result = engine
        .resume_forget_review(
            &start.pending_task.task_id,
            ForgetReviewResult {
                schema_version: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "live_session".to_string(),
                recommendations: vec![ForgetRecommendation {
                    memory_unit_id: "not_in_candidate_set".to_string(),
                    decision: ForgetDecision::Forget,
                    reason: "Unknown unit.".to_string(),
                }],
            },
        )
        .expect("resume forget review");
    assert_eq!(result.ignored, 1);
    assert_eq!(result.forgotten, 0);

    fs::remove_dir_all(root).ok();
}

#[test]
fn forget_review_with_no_candidates_does_not_leave_pending_task() {
    let root = unique_temp_dir("forget_review_no_candidates");
    let storage = FileStorage::with_host_id(&root, "forget_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    let start = engine
        .begin_forget_review("live_session")
        .expect("begin empty forget review");
    assert_eq!(start.candidate_count, 0);
    let task = storage_probe
        .load_task(&start.pending_task.task_id)
        .expect("load completed empty task");
    assert_eq!(task.state, TaskState::Completed);
    assert!(engine.pending_tasks().expect("pending tasks").is_empty());

    fs::remove_dir_all(root).ok();
}

struct ArchiveFixture {
    archive_id: String,
    unit_ids: Vec<String>,
}

fn create_archive_with_units(
    engine: &MemoryEngine<FileStorage>,
    timestamp: &str,
    units: Vec<(&str, f64)>,
) -> ArchiveFixture {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "terminal_user".to_string(),
            timestamp: timestamp.to_string(),
            session_id: "live_session".to_string(),
            payload: json!({ "text": "A routine conversation detail." }),
            tags: vec!["routine".to_string()],
            theme: Some("routine".to_string()),
            emotional_tone: None,
            links: vec![],
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest");
    let sleep = engine.sleep("live_session").expect("sleep");
    let drafts = units
        .into_iter()
        .map(|(thesis, weight)| MemoryUnitDraft {
            thesis: thesis.to_string(),
            source_event_ids: sleep.archive_entry.source_event_ids.clone(),
            evidence: Some("Routine source event.".to_string()),
            tags: vec!["routine".to_string()],
            weight,
        })
        .collect::<Vec<_>>();
    let archive = engine
        .resume_memory_unit_pass(
            &sleep
                .memory_unit_task
                .as_ref()
                .expect("memory unit task")
                .task_id,
            MemoryUnitPassResult {
                schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                memory_units: drafts,
            },
        )
        .expect("resume units");
    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                gist: "Routine conversation details.".to_string(),
                narrative: "The user had a routine exchange without lasting importance."
                    .to_string(),
                compact_memory: None,
                facts: Vec::new(),
                quotes: Vec::new(),
                tags: vec!["routine".to_string()],
                theme: Some("routine".to_string()),
                weight: 0.2,
                links: Vec::new(),
                emotional_markers: Vec::new(),
                topic_thread: Vec::new(),
                personal_signals: Vec::new(),
                relational_tone: None,
            },
        )
        .expect("resume sleep");

    ArchiveFixture {
        archive_id: archive.archive_id,
        unit_ids: archive
            .memory_units
            .into_iter()
            .map(|unit| unit.memory_unit_id)
            .collect(),
    }
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
