use memory_engine::archive::ArchiveStatus;
use memory_engine::core_store::CoreContextRequest;
use memory_engine::event::IngestEvent;
use memory_engine::llm::{LlmResponse, SleepOutcome};
use memory_engine::types::{
    Speaker, CORE_CONTEXT_REQUEST_SCHEMA_VERSION, EVENT_SCHEMA_VERSION,
    MEMORY_UNITS_RESULT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::{json, Value};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION: &str = "multi_speaker_session";

#[test]
fn multi_speaker_sleep_attributes_self_statements_and_routes_other_claims_to_review() {
    let root = unique_temp_dir("attributed_multi_speaker");
    let engine = MemoryEngine::new(FileStorage::with_host_id(&root, "multi_speaker_test"));

    let alice_self = ingest_speaker_text(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "I am Alice and I live in Kyiv.",
        Some(("user_alice", "Alice")),
    );
    let bob_self = ingest_speaker_text(
        &engine,
        "2026-07-01T10:01:00.000Z",
        "I am Bob and I have a dog named Mars.",
        Some(("user_bob", "Bob")),
    );
    let alice_about_bob = ingest_speaker_text(
        &engine,
        "2026-07-01T10:02:00.000Z",
        "Bob hates his job.",
        Some(("user_alice", "Alice")),
    );
    let alice_with_false_hint = ingest_speaker_text(
        &engine,
        "2026-07-01T10:03:00.000Z",
        "I moved to Odesa.",
        Some(("user_alice", "Alice")),
    );

    let units = vec![
        memory_unit(
            "Alice lives in Kyiv.",
            &alice_self,
            "self_statement",
            "user_alice",
        ),
        memory_unit(
            "Bob has a dog named Mars.",
            &bob_self,
            "self_statement",
            "user_bob",
        ),
        memory_unit(
            "Alice said Bob hates his job.",
            &alice_about_bob,
            "about_other",
            "user_bob",
        ),
        memory_unit(
            "Alice moved to Odesa.",
            &alice_with_false_hint,
            "self_statement",
            "user_bob",
        ),
    ];
    let signals = vec![
        personal_signal(
            "Alice lives in Kyiv.",
            "location",
            &alice_self,
            "self_statement",
            "user_alice",
        ),
        personal_signal(
            "Bob has a dog named Mars.",
            "pet",
            &bob_self,
            "self_statement",
            "user_bob",
        ),
        personal_signal(
            "Bob hates his job.",
            "work",
            &alice_about_bob,
            "about_other",
            "user_bob",
        ),
        personal_signal(
            "Alice moved to Odesa.",
            "location",
            &alice_with_false_hint,
            "self_statement",
            "user_bob",
        ),
    ];

    let outcome = run_sleep(&engine, units, signals);
    assert_eq!(outcome.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(outcome.core_summary.created, 2);
    assert!(outcome.core_summary.skipped >= 2);
    assert_eq!(
        outcome.fidelity_requests.len(),
        2,
        "only verified self-statements belong to the automatic Core path"
    );

    let subjects = outcome
        .archive_entry
        .memory_units
        .iter()
        .map(|unit| unit.subject.as_ref().map(|subject| subject.id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        subjects,
        vec![Some("user_alice"), Some("user_bob"), Some("user_bob"), None]
    );

    let context = engine
        .core_context_package(core_context_request(SESSION, true))
        .expect("core context");
    assert_eq!(context.core_facts.len(), 2);
    assert!(context.core_facts.iter().any(|fact| {
        fact.text == "Alice lives in Kyiv."
            && fact
                .subject
                .as_ref()
                .is_some_and(|subject| subject.id == "user_alice")
    }));
    assert!(context.core_facts.iter().any(|fact| {
        fact.text == "Bob has a dog named Mars."
            && fact
                .subject
                .as_ref()
                .is_some_and(|subject| subject.id == "user_bob")
    }));

    let review_candidates = engine
        .list_candidates()
        .expect("review candidates")
        .into_iter()
        .filter(|candidate| candidate.tags.iter().any(|tag| tag == "attribution_review"))
        .collect::<Vec<_>>();
    assert_eq!(review_candidates.len(), 2);
    assert!(review_candidates.iter().any(|candidate| {
        candidate.text == "Bob hates his job."
            && candidate
                .subject
                .as_ref()
                .is_some_and(|subject| subject.id == "user_bob")
    }));
    assert!(review_candidates
        .iter()
        .any(|candidate| candidate.text == "Alice moved to Odesa." && candidate.subject.is_none()));

    engine
        .set_vector_scope(SESSION, true, false)
        .expect("enable vectors");
    let embedding_requests = engine
        .pending_embedding_backfill(SESSION)
        .expect("embedding backfill");
    let embedded_unit_count = embedding_requests
        .iter()
        .map(|request| {
            request.prompt_inputs["embed_batch"]["items"]
                .as_array()
                .map_or(0, Vec::len)
        })
        .sum::<usize>();
    assert_eq!(
        embedded_unit_count, 3,
        "attributed units are eligible while the unit with a false hint stays out"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn legacy_single_speaker_output_without_attribution_hint_still_reaches_core() {
    let root = unique_temp_dir("legacy_single_speaker");
    let engine = MemoryEngine::new(FileStorage::with_host_id(&root, "multi_speaker_test"));
    let event_id = ingest_speaker_text(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "I bought a motorcycle.",
        Some(("user_alice", "Alice")),
    );

    let outcome = run_sleep(
        &engine,
        vec![json!({
            "thesis": "Alice bought a motorcycle.",
            "source_event_ids": [event_id.clone()],
            "evidence": "Direct statement.",
            "tags": ["vehicle"],
            "weight": 0.5
        })],
        vec![json!({
            "text": "Alice bought a motorcycle.",
            "category": "vehicle",
            "confidence": 0.95,
            "source_event_ids": [event_id],
            "evidence": "Direct statement."
        })],
    );

    assert_eq!(outcome.core_summary.created, 1);
    assert_eq!(
        outcome.archive_entry.personal_signals[0]
            .subject
            .as_ref()
            .map(|subject| subject.id.as_str()),
        Some("user_alice")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn speakerless_single_user_host_keeps_previous_behavior() {
    let root = unique_temp_dir("speakerless_single_user");
    let engine = MemoryEngine::new(FileStorage::with_host_id(&root, "multi_speaker_test"));
    let event_id = ingest_speaker_text(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "I bought a motorcycle.",
        None,
    );

    let outcome = run_sleep(
        &engine,
        vec![json!({
            "thesis": "The user bought a motorcycle.",
            "source_event_ids": [event_id.clone()],
            "evidence": "Direct statement.",
            "tags": ["vehicle"],
            "weight": 0.5
        })],
        vec![json!({
            "text": "The user bought a motorcycle.",
            "category": "vehicle",
            "confidence": 0.95,
            "source_event_ids": [event_id],
            "evidence": "Direct statement."
        })],
    );

    assert_eq!(outcome.core_summary.created, 1);
    assert!(outcome.archive_entry.personal_signals[0].subject.is_none());
    fs::remove_dir_all(root).ok();
}

fn memory_unit(thesis: &str, event_id: &str, subject_hint: &str, speaker_id: &str) -> Value {
    json!({
        "thesis": thesis,
        "subject_hint": subject_hint,
        "speaker_id": speaker_id,
        "source_event_ids": [event_id],
        "evidence": "Direct source event.",
        "tags": ["group_chat"],
        "weight": 0.5
    })
}

fn personal_signal(
    text: &str,
    category: &str,
    event_id: &str,
    subject_hint: &str,
    speaker_id: &str,
) -> Value {
    json!({
        "text": text,
        "category": category,
        "confidence": 0.95,
        "subject_hint": subject_hint,
        "speaker_id": speaker_id,
        "source_event_ids": [event_id],
        "evidence": "Direct source event."
    })
}

fn ingest_speaker_text(
    engine: &MemoryEngine<FileStorage>,
    timestamp: &str,
    text: &str,
    speaker: Option<(&str, &str)>,
) -> String {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test_group_chat".to_string(),
            timestamp: timestamp.to_string(),
            personality_scope_id: None,
            session_id: SESSION.to_string(),
            memory_text: None,
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
        .expect("ingest speaker text")
        .stored_event
        .event_id
}

fn run_sleep(
    engine: &MemoryEngine<FileStorage>,
    memory_units: Vec<Value>,
    personal_signals: Vec<Value>,
) -> SleepOutcome {
    let mut run = engine.begin_sleep_run(SESSION).expect("begin sleep run");
    let step = engine.next_sleep_batch(run).expect("first sleep batch");
    run = step.run;
    let batch = step.batch.expect("extraction batch");
    let archive_id = run.archive_id.clone();

    let responses = batch
        .requests
        .into_iter()
        .map(|request| {
            let text = match request.prompt_id.as_str() {
                "memory_unit_pass" => json!({
                    "schema_version": MEMORY_UNITS_RESULT_SCHEMA_VERSION,
                    "archive_id": archive_id,
                    "memory_units": memory_units
                }),
                "sleep_emotional_pass" => json!({ "emotional_markers": [] }),
                "sleep_topic_thread_pass" => json!({
                    "topic_thread": [{
                        "topic": "group conversation",
                        "summary": "Participants shared personal details.",
                        "source_event_ids": []
                    }]
                }),
                "sleep_personal_signal_pass" => json!({
                    "personal_signals": personal_signals
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
                text: "GIST: Participants shared personal details.\n\nAlice and Bob discussed their lives in a group chat.".to_string(),
            }],
        )
        .expect("submit consolidator");
    engine.finish_sleep_run(step.run).expect("finish sleep run")
}

fn core_context_request(session_id: &str, include_core: bool) -> CoreContextRequest {
    CoreContextRequest {
        schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
        session_id: session_id.to_string(),
        domain_state: json!({}),
        core_scope: Some(session_id.to_string()),
        query_text: None,
        query_embedding: None,
        explain_recall: false,
        recall_limit: 0,
        session_recent_limit: 0,
        session_trace_event_limit: 0,
        include_core,
        token_budget: None,
        utc_offset_minutes: 0,
        clock_untrusted: false,
    }
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
