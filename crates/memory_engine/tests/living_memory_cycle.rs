use memory_engine::archive::{ArchiveStatus, FidelityReview, FidelityStatus};
use memory_engine::core_store::{
    CandidateReviewInput, CandidateStatus, CoreContextRequest, CoreFactInput, CoreFactStatus,
    ReviewDecision,
};
use memory_engine::event::IngestEvent;
use memory_engine::llm::{LlmResponse, SleepOutcome};
use memory_engine::recall::{RecallFilters, RecallQuery, RecallSourceLayer};
use memory_engine::storage::Storage;
use memory_engine::types::{
    CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION, CORE_CONTEXT_REQUEST_SCHEMA_VERSION,
    CORE_FACT_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, FIDELITY_REVIEW_SCHEMA_VERSION,
    FORGET_REVIEW_RESULT_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION,
    RECALL_QUERY_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION: &str = "living_cycle_session";

#[test]
fn living_memory_cycle_closes_v02_end_to_end() {
    let root = unique_temp_dir("living_memory_cycle_closes_v02_end_to_end");
    let storage = FileStorage::with_host_id(&root, "cycle_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    ingest_text(
        &engine,
        "2020-01-01T10:00:00.000Z",
        "I love space because it calms me down.",
        &["personal_fact", "interest"],
    );
    let space = run_sleep(
        &engine,
        "The user described space as calming.",
        "The user framed space as a recurring interest that helps them feel calm.",
        vec![UnitSpec {
            thesis: "Space comfort -> the user returns to space because it calms them.",
            weight: 0.92,
            tags: vec!["interest", "emotionally_relevant"],
            evidence: "The user directly said space calms them down.",
        }],
        vec![SignalSpec {
            text: "The user loves space.",
            category: "interest",
            confidence: 0.95,
        }],
        true,
    );
    assert_eq!(space.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(space.core_summary.created, 1);
    assert_eq!(space.fidelity_requests.len(), 1);
    let space_unit_id = space.archive_entry.memory_units[0].memory_unit_id.clone();
    submit_valid_fidelity(
        &engine,
        &space.fidelity_requests[0],
        &space_unit_id,
        &space.archive_entry.archive_id,
        "The source directly supports the space thesis.",
    );

    let context = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: SESSION.to_string(),
            domain_state: json!({ "current_text": "what do you know about my interests?" }),
            core_scope: Some(SESSION.to_string()),
            query_text: Some("space".to_string()),
            recall_limit: 5,
            session_recent_limit: 5,
            session_trace_event_limit: 10,
            include_core: true,
            utc_offset_minutes: 0,
            clock_untrusted: false,
            token_budget: None,
        })
        .expect("context package");
    assert!(context
        .core_facts
        .iter()
        .any(|fact| fact.text == "The user loves space."));
    assert!(context.archive_relevant.iter().any(|item| {
        item.compact_memory
            .as_deref()
            .unwrap_or("")
            .contains("Space comfort")
    }));

    let recall = engine
        .recall(RecallQuery {
            schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            query_id: None,
            created_at: Some("2026-06-01T00:00:00.000Z".to_string()),
            session_id: Some(SESSION.to_string()),
            context: json!({}),
            query_text: Some("space calming interest".to_string()),
            filters: RecallFilters {
                source_layers: vec![RecallSourceLayer::Archive],
                ..RecallFilters::default()
            },
            limit: 5,
            include_core: false,
            explain: true,
        })
        .expect("recall");
    assert!(recall.items.iter().any(|item| {
        item.compact_memory
            .as_deref()
            .unwrap_or("")
            .contains("Space comfort")
    }));

    let reflection = engine
        .begin_reflection_analysis(SESSION, Some(SESSION.to_string()))
        .expect("begin reflection");
    assert!(reflection.memory_unit_count >= 1);
    let reflected = engine
        .submit_reflection_response(
            &reflection.pending_task.task_id,
            LlmResponse::Ok {
                request_id: reflection.request.request_id.clone(),
                text: json!({
                    "schema_version": "reflection_result.v1",
                    "source_session_id": SESSION,
                    "core_scope": SESSION,
                    "candidates": [{
                        "text": "The user uses space as a calming recurring interest.",
                        "category": "interest",
                        "confidence": 0.92,
                        "evidence_summary": "A validated memory unit says the user returns to space because it calms them.",
                        "source_memory_unit_ids": [space_unit_id.clone()],
                        "supporting_archive_ids": [space.archive_entry.archive_id.clone()],
                        "contradicting_archive_ids": [],
                        "tags": ["reflection", "space"]
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit reflection");
    assert_eq!(reflected.candidates.len(), 1);
    assert_eq!(
        reflected.candidates[0].status,
        CandidateStatus::ReadyForReview
    );
    let reflection_review = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: reflected.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: Some("Stable and supported.".to_string()),
            core_scope: Some(SESSION.to_string()),
        })
        .expect("confirm reflection candidate");
    assert_eq!(
        reflection_review.candidate.status,
        CandidateStatus::Promoted
    );
    assert!(reflection_review.promoted_fact.is_some());

    let old_location = engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "location".to_string(),
            scope: Some(SESSION.to_string()),
            text: "The user lives in Berlin.".to_string(),
            confidence: 0.95,
            tags: vec!["manual_seed".to_string()],
            source_archive_ids: Vec::new(),
            source_candidate_id: None,
        })
        .expect("seed old location")
        .fact;

    ingest_text(
        &engine,
        "2020-01-02T10:00:00.000Z",
        "I moved back to Kyiv, so Berlin is outdated now.",
        &["personal_fact", "location"],
    );
    let location = run_sleep(
        &engine,
        "The user updated their location.",
        "The user said they moved back to Kyiv and that Berlin is outdated.",
        vec![UnitSpec {
            thesis: "Location update -> the user moved back to Kyiv and Berlin is outdated.",
            weight: 0.95,
            tags: vec!["location", "core_path"],
            evidence: "The user directly said Berlin is outdated now.",
        }],
        Vec::new(),
        false,
    );
    assert_eq!(location.fidelity_requests.len(), 1);
    let location_unit_id = location.archive_entry.memory_units[0]
        .memory_unit_id
        .clone();
    submit_valid_fidelity(
        &engine,
        &location.fidelity_requests[0],
        &location_unit_id,
        &location.archive_entry.archive_id,
        "The source directly supports the location update.",
    );

    let contradiction = engine
        .begin_reflection_analysis(SESSION, Some(SESSION.to_string()))
        .expect("begin contradiction reflection");
    let contradicted = engine
        .submit_reflection_response(
            &contradiction.pending_task.task_id,
            LlmResponse::Ok {
                request_id: contradiction.request.request_id.clone(),
                text: json!({
                    "schema_version": "reflection_result.v1",
                    "source_session_id": SESSION,
                    "core_scope": SESSION,
                    "candidates": [{
                        "text": "The user lives in Kyiv now.",
                        "category": "location",
                        "confidence": 0.95,
                        "evidence_summary": "A validated memory unit says the user moved back to Kyiv and Berlin is outdated.",
                        "source_memory_unit_ids": [location_unit_id.clone()],
                        "supporting_archive_ids": [location.archive_entry.archive_id.clone()],
                        "contradicting_archive_ids": [location.archive_entry.archive_id.clone()],
                        "contradicted_core_fact_ids": [old_location.core_fact_id.clone()],
                        "tags": ["reflection", "location_update"]
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit contradiction reflection");
    let contradiction_review = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: contradicted.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: Some("The new statement supersedes Berlin.".to_string()),
            core_scope: Some(SESSION.to_string()),
        })
        .expect("confirm contradiction candidate");
    assert_eq!(contradiction_review.contested_facts.len(), 1);
    assert_eq!(
        contradiction_review.contested_facts[0].status,
        CoreFactStatus::Contested
    );
    let location_context = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: SESSION.to_string(),
            domain_state: json!({}),
            core_scope: Some(SESSION.to_string()),
            query_text: Some("where does the user live?".to_string()),
            recall_limit: 0,
            session_recent_limit: 0,
            session_trace_event_limit: 0,
            include_core: true,
            utc_offset_minutes: 0,
            clock_untrusted: false,
            token_budget: None,
        })
        .expect("location context");
    assert!(location_context.core_facts.iter().any(|fact| {
        fact.core_fact_id == old_location.core_fact_id && fact.status == CoreFactStatus::Contested
    }));
    assert!(location_context
        .core_facts
        .iter()
        .any(|fact| fact.text == "The user lives in Kyiv now."
            && fact.status == CoreFactStatus::Active));

    ingest_text(
        &engine,
        "2020-01-03T10:00:00.000Z",
        "For lunch I had soup while testing a small note.",
        &["routine"],
    );
    let routine = run_sleep(
        &engine,
        "A routine lunch note was archived.",
        "The user briefly mentioned soup during a routine exchange.",
        vec![UnitSpec {
            thesis: "Routine lunch -> the user mentioned soup during a minor exchange.",
            weight: 0.2,
            tags: vec!["routine"],
            evidence: "The user mentioned soup as a small lunch note.",
        }],
        Vec::new(),
        false,
    );
    let routine_unit_id = routine.archive_entry.memory_units[0].memory_unit_id.clone();
    assert!(routine
        .archive_entry
        .compact_memory
        .as_deref()
        .unwrap_or("")
        .contains("Routine lunch"));

    let forget = engine
        .begin_forget_review(SESSION)
        .expect("begin forget review");
    assert!(forget
        .candidates
        .iter()
        .any(|candidate| candidate.memory_unit_id == routine_unit_id));
    let forget_result = engine
        .submit_forget_review_response(
            &forget.pending_task.task_id,
            LlmResponse::Ok {
                request_id: forget.request.request_id.clone(),
                text: json!({
                    "schema_version": FORGET_REVIEW_RESULT_SCHEMA_VERSION,
                    "source_session_id": SESSION,
                    "recommendations": [{
                        "memory_unit_id": routine_unit_id.clone(),
                        "decision": "forget",
                        "reason": "Low-signal routine detail with no Core link."
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit forget review");
    assert_eq!(forget_result.forgotten, 1);
    assert_eq!(forget_result.protected, 0);
    let forgotten = engine
        .list_forgotten_memory_units(SESSION)
        .expect("list forgotten");
    assert!(forgotten
        .units
        .iter()
        .any(|unit| unit.memory_unit_id == routine_unit_id));

    let routine_archive = storage_probe
        .read_archive_entry_by_id(&routine.archive_entry.archive_id)
        .expect("read routine archive");
    assert!(!routine_archive
        .compact_memory
        .as_deref()
        .unwrap_or("")
        .contains("Routine lunch"));

    let restored = engine
        .remember_back(&routine_unit_id)
        .expect("remember back");
    assert_eq!(
        restored.status,
        memory_engine::archive::MemoryUnitStatus::ActiveArchive
    );
    let restored_archive = storage_probe
        .read_archive_entry_by_id(&routine.archive_entry.archive_id)
        .expect("read restored archive");
    assert!(restored_archive
        .compact_memory
        .as_deref()
        .unwrap_or("")
        .contains("Routine lunch"));

    fs::remove_dir_all(root).ok();
}

struct UnitSpec<'a> {
    thesis: &'a str,
    weight: f64,
    tags: Vec<&'a str>,
    evidence: &'a str,
}

struct SignalSpec<'a> {
    text: &'a str,
    category: &'a str,
    confidence: f64,
}

fn run_sleep(
    engine: &MemoryEngine<FileStorage>,
    gist: &str,
    narrative: &str,
    units: Vec<UnitSpec<'_>>,
    signals: Vec<SignalSpec<'_>>,
    emotional: bool,
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
                "memory_unit_pass" => {
                    let drafts = units
                        .iter()
                        .map(|unit| {
                            json!({
                                "thesis": unit.thesis,
                                "source_event_ids": [event_id.clone()],
                                "evidence": unit.evidence,
                                "tags": unit.tags,
                                "weight": unit.weight
                            })
                        })
                        .collect::<Vec<_>>();
                    json!({
                        "schema_version": MEMORY_UNITS_RESULT_SCHEMA_VERSION,
                        "archive_id": archive_id,
                        "memory_units": drafts
                    })
                }
                "sleep_emotional_pass" => {
                    if emotional {
                        json!({
                            "emotional_markers": [{
                                "target": "space",
                                "affect": "comfort",
                                "strength": 0.9,
                                "source_event_ids": [event_id.clone()]
                            }]
                        })
                    } else {
                        json!({ "emotional_markers": [] })
                    }
                }
                "sleep_topic_thread_pass" => json!({
                    "topic_thread": [{
                        "topic": "test_thread",
                        "summary": gist,
                        "source_event_ids": [event_id.clone()]
                    }]
                }),
                "sleep_personal_signal_pass" => {
                    let personal_signals = signals
                        .iter()
                        .map(|signal| {
                            json!({
                                "text": signal.text,
                                "category": signal.category,
                                "confidence": signal.confidence,
                                "source_event_ids": [event_id.clone()]
                            })
                        })
                        .collect::<Vec<_>>();
                    json!({ "personal_signals": personal_signals })
                }
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
                text: format!("GIST: {gist}\n\n{narrative}"),
            }],
        )
        .expect("submit consolidator");
    run = step.run;

    engine.finish_sleep_run(run).expect("finish sleep run")
}

fn submit_valid_fidelity(
    engine: &MemoryEngine<FileStorage>,
    request: &memory_engine::LlmRequest,
    unit_id: &str,
    archive_id: &str,
    explanation: &str,
) {
    engine
        .submit_memory_fidelity_response(
            &request.task_id,
            LlmResponse::Ok {
                request_id: request.request_id.clone(),
                text: json!(FidelityReview {
                    schema_version: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                    memory_unit_id: unit_id.to_string(),
                    archive_id: archive_id.to_string(),
                    status: FidelityStatus::Valid,
                    confidence: 0.95,
                    explanation: explanation.to_string(),
                    revised_thesis: None,
                    missing_detail: None,
                })
                .to_string(),
            },
        )
        .expect("submit valid fidelity");
}

fn ingest_text(engine: &MemoryEngine<FileStorage>, timestamp: &str, text: &str, tags: &[&str]) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test_user".to_string(),
            timestamp: timestamp.to_string(),
            session_id: SESSION.to_string(),
            payload: json!({ "text": text }),
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            theme: tags.first().map(|tag| (*tag).to_string()),
            emotional_tone: None,
            speaker: None,
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
