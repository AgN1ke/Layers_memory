use memory_engine::archive::{ArchiveStatus, FidelityReview, FidelityStatus, RelationalTone};
use memory_engine::core_store::{CandidateReviewInput, CandidateStatus, ReviewDecision};
use memory_engine::event::IngestEvent;
use memory_engine::llm::LlmResponse;
use memory_engine::sleep::{MemoryUnitDraft, MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::storage::Storage;
use memory_engine::types::{
    CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, FIDELITY_REVIEW_SCHEMA_VERSION,
    MEMORY_UNITS_RESULT_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn reflection_creates_reviewable_candidate_and_confirm_promotes_to_core() {
    let root = unique_temp_dir("reflection_candidate_confirm_promotes");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    ingest_text(
        &engine,
        "2026-05-31T20:00:00.000Z",
        "I keep returning to space because it calms me down.",
    );
    let sleep = engine.sleep("live_session").expect("sleep stage1");
    let unit_archive = engine
        .resume_memory_unit_pass(
            &sleep
                .memory_unit_task
                .as_ref()
                .expect("memory unit task")
                .task_id,
            MemoryUnitPassResult {
                schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                memory_units: vec![MemoryUnitDraft {
                    thesis: "Space interest -> the user returns to space because it calms them."
                        .to_string(),
                    source_event_ids: sleep.archive_entry.source_event_ids.clone(),
                    evidence: Some("The user directly said space calms them down.".to_string()),
                    tags: vec!["interest".to_string(), "values".to_string()],
                    weight: 0.9,
                }],
            },
        )
        .expect("resume memory unit pass");
    let unit_id = unit_archive.memory_units[0].memory_unit_id.clone();

    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                gist: "The user said space calms them.".to_string(),
                narrative: "The user framed space as a recurring topic that helps them feel calm."
                    .to_string(),
                compact_memory: None,
                facts: Vec::new(),
                quotes: Vec::new(),
                tags: vec!["space".to_string(), "personal_pattern".to_string()],
                theme: Some("space_interest".to_string()),
                weight: 0.9,
                links: Vec::new(),
                emotional_markers: Vec::new(),
                topic_thread: Vec::new(),
                personal_signals: Vec::new(),
                relational_tone: Some(RelationalTone {
                    warmth: None,
                    intellectual_engagement: Some(0.7),
                    intimacy: None,
                    trust: None,
                    playfulness: None,
                    tension: None,
                    summary: Some("The exchange was reflective.".to_string()),
                    source_event_ids: sleep.archive_entry.source_event_ids.clone(),
                }),
            },
        )
        .expect("resume sleep compression");

    let fidelity = engine
        .begin_memory_fidelity_pass(&unit_id)
        .expect("begin fidelity");
    engine
        .resume_memory_fidelity_pass(
            &fidelity.pending_task.task_id,
            FidelityReview {
                schema_version: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                memory_unit_id: unit_id.clone(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                status: FidelityStatus::Valid,
                confidence: 0.95,
                explanation: "The thesis is directly supported by the source event.".to_string(),
                revised_thesis: None,
                missing_detail: None,
            },
        )
        .expect("mark unit valid");

    let start = engine
        .begin_reflection_analysis("live_session", Some("live_session".to_string()))
        .expect("begin reflection");
    assert_eq!(start.memory_unit_count, 1);
    assert_eq!(
        start.request.role_hint,
        memory_engine::types::ModelRole::Reasoning
    );

    let candidates = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id.clone(),
                text: json!({
                    "schema_version": "reflection_result.v1",
                    "source_session_id": "live_session",
                    "core_scope": "live_session",
                    "candidates": [{
                        "text": "The user uses space as a calming recurring interest.",
                        "category": "interest",
                        "confidence": 0.9,
                        "evidence_summary": "A validated memory unit says the user returns to space because it calms them.",
                        "source_memory_unit_ids": [unit_id.clone()],
                        "supporting_archive_ids": [sleep.archive_entry.archive_id.clone()],
                        "contradicting_archive_ids": [],
                        "tags": ["reflection", "space"]
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit reflection");
    assert_eq!(candidates.candidates.len(), 1);
    assert_eq!(
        candidates.candidates[0].status,
        CandidateStatus::ReadyForReview
    );

    let review = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: candidates.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: Some("Accurate and stable enough.".to_string()),
            core_scope: Some("live_session".to_string()),
        })
        .expect("confirm candidate");
    assert_eq!(review.candidate.status, CandidateStatus::Promoted);
    assert!(review.promoted_fact.is_some());

    let core = storage_probe
        .read_core_store_category("interest")
        .expect("read core category");
    assert_eq!(core.facts.len(), 1);
    assert_eq!(
        core.facts[0].source_candidate_id.as_deref(),
        Some(candidates.candidates[0].candidate_id.as_str())
    );
    assert_eq!(core.facts[0].scope.as_deref(), Some("live_session"));
    assert_eq!(
        core.facts[0].status,
        memory_engine::core_store::CoreFactStatus::Active
    );

    let archive = storage_probe
        .read_archive_entry_by_id(&sleep.archive_entry.archive_id)
        .expect("read archive");
    assert_eq!(archive.status, ArchiveStatus::Complete);

    fs::remove_dir_all(root).ok();
}

fn ingest_text(engine: &MemoryEngine<FileStorage>, timestamp: &str, text: &str) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test_user".to_string(),
            timestamp: timestamp.to_string(),
            session_id: "live_session".to_string(),
            payload: json!({ "text": text }),
            tags: vec!["reflection_test".to_string()],
            theme: Some("space".to_string()),
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
