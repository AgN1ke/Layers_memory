use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::archive::{ArchiveEntry, MemoryUnit};
use memory_engine::core_store::CandidateBelief;
use memory_engine::event::{IngestEvent, StoredEvent};
use memory_engine::storage::Storage;
use memory_engine::vector::{
    DeepRecallQuery, EmbedBatchInputs, EmbedBatchResult, EmbedBatchVector, VectorScopeStatus,
    EMBED_BATCH_RESULT_SCHEMA_VERSION,
};
use memory_engine::{EngineOptions, FileStorage, MemoryEngine, MemoryEngineError};
use serde_json::{json, Value};

const SCOPE: &str = "group_scope";
const SESSION: &str = "group_session";
const OTHER_SCOPE: &str = "foreign_scope";
const OTHER_SESSION: &str = "foreign_session";
const FIRST: &str = "first_alex";
const SECOND: &str = "second_alex";
const THIRD: &str = "casey";
const TARGET_TEXT: &str = "My private codeword is marigold.";
const SECOND_TEXT: &str = "I collect copper buttons.";
const THIRD_TEXT: &str = "My project will start tomorrow.";
const FOREIGN_TEXT: &str = "I keep a watercolor journal.";
const VECTOR_MODEL: &str = "subject_test_model";
const TARGET_CANDIDATE: &str = "target_candidate";
const SIBLING_CANDIDATE: &str = "sibling_candidate";

#[test]
fn subject_erasure_preserves_lookalikes_and_typed_reply_author_after_restart() {
    let fixture = Fixture::new("typed_quotes");
    let engine = fixture.open();
    let generation = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let target_input = event_input(SCOPE, SESSION, FIRST, TARGET_TEXT, Some(vec![]), None);
    let target = engine
        .ingest_idempotent_at_generation(
            "target_delivery",
            target_input.clone(),
            generation.clone(),
        )
        .unwrap()
        .event;
    let lookalike = engine
        .ingest(event_input(
            SCOPE,
            SESSION,
            SECOND,
            SECOND_TEXT,
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    let reply = engine
        .ingest(event_input(
            SCOPE,
            SESSION,
            THIRD,
            THIRD_TEXT,
            Some(vec![&target.event_id]),
            Some(&target.event_id),
        ))
        .unwrap()
        .stored_event;
    let foreign = engine
        .ingest(event_input(
            OTHER_SCOPE,
            OTHER_SESSION,
            FIRST,
            FOREIGN_TEXT,
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    assert!(matches!(
        engine.ingest(event_input(
            SCOPE,
            SESSION,
            THIRD,
            "Cross-scope quotation",
            Some(vec![&foreign.event_id]),
            None,
        )),
        Err(MemoryEngineError::Validation(_))
    ));
    let episode = publish_archive(
        &engine,
        SESSION,
        vec![
            unit(
                "Alex uses the private codeword marigold.",
                FIRST,
                &[&target.event_id],
            ),
            unit(
                "Alex collects copper buttons.",
                SECOND,
                &[&lookalike.event_id],
            ),
            unit(
                "Alex collects copper buttons and knows the codeword marigold.",
                SECOND,
                &[&lookalike.event_id, &target.event_id],
            ),
        ],
        &format!("{TARGET_TEXT} {SECOND_TEXT} {THIRD_TEXT}"),
    );
    let erased_unit = episode
        .memory_units
        .iter()
        .find(|unit| {
            unit.subject
                .as_ref()
                .is_some_and(|subject| subject.id == FIRST)
        })
        .unwrap();
    let surviving_unit = episode
        .memory_units
        .iter()
        .find(|unit| unit.thesis == "Alex collects copper buttons.")
        .unwrap();
    add_candidate(&engine, TARGET_CANDIDATE, erased_unit);
    add_candidate(&engine, SIBLING_CANDIDATE, surviving_unit);
    let links = engine
        .storage()
        .read_hypothesis_evidence_links()
        .unwrap()
        .items;
    assert_eq!(links.len(), 2);
    assert!(links
        .iter()
        .any(|link| link.hypothesis_id == TARGET_CANDIDATE
            && link.memory_unit_id == erased_unit.memory_unit_id));
    assert!(links
        .iter()
        .any(|link| link.hypothesis_id == SIBLING_CANDIDATE
            && link.memory_unit_id == surviving_unit.memory_unit_id));

    engine.set_vector_scope(SCOPE, true, false).unwrap();
    let requests = engine.pending_embedding_backfill(SCOPE).unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let batch: EmbedBatchInputs =
            serde_json::from_value(request.prompt_inputs["embed_batch"].clone()).unwrap();
        let vector = if batch.items[0].memory_unit_id == surviving_unit.memory_unit_id {
            vec![0.0, 1.0]
        } else {
            vec![1.0, 0.0]
        };
        assert_eq!(
            engine
                .resume_compute_embedding(&request.task_id, embedding_result(batch, vector))
                .unwrap(),
            1
        );
    }
    assert!(deep_recall_text(&engine, vec![1.0, 0.0]).contains("marigold"));
    // An outdated on-disk hash schedules a genuine Pending backfill for the
    // target while its earlier row still exists to be physically cleaned.
    let mut index = engine.storage().read_vector_index(SCOPE).unwrap().unwrap();
    index
        .rows
        .iter_mut()
        .find(|row| row.memory_unit_id == erased_unit.memory_unit_id)
        .unwrap()
        .thesis_hash = memory_engine::vector::thesis_hash("A superseded vector.");
    engine.storage().write_vector_index(SCOPE, &index).unwrap();
    let mut pending = engine.pending_embedding_backfill(SCOPE).unwrap();
    assert_eq!(pending.len(), 1);
    let request = pending.remove(0);
    let batch: EmbedBatchInputs =
        serde_json::from_value(request.prompt_inputs["embed_batch"].clone()).unwrap();
    assert_eq!(batch.items[0].memory_unit_id, erased_unit.memory_unit_id);
    let embedding_task = request.task_id;
    let delayed_embedding = embedding_result(batch, vec![1.0, 0.0]);
    assert_eq!(
        engine.storage().load_task(&embedding_task).unwrap().state,
        memory_engine::tasks::TaskState::Pending
    );
    add_core_fact(&engine, SCOPE, FIRST, "Alex uses the codeword marigold.");
    add_core_fact(&engine, SCOPE, SECOND, "Alex collects copper buttons.");
    add_core_fact(
        &engine,
        OTHER_SCOPE,
        FIRST,
        "Alex keeps a watercolor journal.",
    );
    let fidelity = engine
        .begin_memory_fidelity_pass(&erased_unit.memory_unit_id)
        .unwrap();

    let late = engine
        .ingest(event_input(
            SCOPE,
            SESSION,
            FIRST,
            "The codeword marigold must stay private.",
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    let sleep = engine.begin_sleep_run(SESSION).unwrap();
    assert!(sleep
        .shared_prompt_context
        .as_ref()
        .unwrap()
        .source_event_ids
        .contains(&late.event_id));

    let first_before = subject_view(&engine, SCOPE, FIRST);
    assert_eq!(first_before["incomplete"], true);
    assert!(view_has(&first_before, "event", TARGET_TEXT));
    assert!(view_has(&first_before, "memory_unit", "marigold"));
    assert!(view_has(&first_before, "core_fact", "marigold"));
    assert!(view_has(&first_before, "hypothesis", "marigold"));
    let text = first_before.to_string();
    assert!(!text.contains("copper buttons"));
    assert!(!text.contains(THIRD_TEXT));
    assert!(!text.contains(FOREIGN_TEXT));
    assert!(subject_view(&engine, SCOPE, "Alex")["items"]
        .as_array()
        .unwrap()
        .is_empty());

    let report = engine
        .erase_subject(SCOPE, FIRST, "erase_first_alex")
        .unwrap();
    let report = serde_json::to_value(report).unwrap();
    assert_eq!(report["deleted_events"], 2);
    assert_eq!(report["detached_quotes"], 1);
    assert_eq!(report["deleted_core_facts"], 1);
    assert_eq!(report["deleted_candidates"], 1);
    assert_eq!(report["deleted_hypotheses"], 1);
    drop(engine);

    let engine = fixture.open();
    let remaining = session_events(&engine, SESSION);
    assert!(remaining
        .iter()
        .all(|event| event.event_id != target.event_id));
    assert!(remaining
        .iter()
        .all(|event| event.event_id != late.event_id));
    let surviving_lookalike = remaining
        .iter()
        .find(|event| event.event_id == lookalike.event_id)
        .unwrap();
    assert_eq!(
        surviving_lookalike.memory_text.as_deref(),
        Some(SECOND_TEXT)
    );
    let surviving_reply = remaining
        .iter()
        .find(|event| event.event_id == reply.event_id)
        .unwrap();
    assert_eq!(surviving_reply.memory_text.as_deref(), Some(THIRD_TEXT));
    assert_eq!(surviving_reply.payload["text"], THIRD_TEXT);
    assert_eq!(surviving_reply.quoted_event_ids, Some(Vec::new()));
    assert!(surviving_reply
        .links
        .iter()
        .all(|link| link.target != target.event_id));
    assert_eq!(session_events(&engine, OTHER_SESSION), [foreign]);

    let first_after = subject_view(&engine, SCOPE, FIRST);
    assert!(first_after["items"].as_array().unwrap().is_empty());
    let lookalike_after = subject_view(&engine, SCOPE, SECOND);
    assert!(view_has(&lookalike_after, "event", SECOND_TEXT));
    assert!(view_has(&lookalike_after, "memory_unit", "copper buttons"));
    assert!(view_has(&lookalike_after, "core_fact", "copper buttons"));
    assert!(view_has(&lookalike_after, "hypothesis", "copper buttons"));
    assert!(!lookalike_after.to_string().contains("marigold"));
    let third_after = subject_view(&engine, SCOPE, THIRD);
    assert!(view_has(&third_after, "event", THIRD_TEXT));
    let foreign_after = subject_view(&engine, OTHER_SCOPE, FIRST);
    assert!(view_has(&foreign_after, "event", FOREIGN_TEXT));
    assert!(view_has(&foreign_after, "core_fact", "watercolor journal"));
    assert!(!scope_context(&engine, SCOPE, SESSION).contains("marigold"));
    let vector_index = engine.storage().read_vector_index(SCOPE).unwrap().unwrap();
    assert_eq!(vector_index.manifest.rows, 1);
    assert_eq!(vector_index.rows[0].row, 0);
    assert_eq!(
        vector_index.rows[0].memory_unit_id,
        surviving_unit.memory_unit_id
    );
    assert_eq!(vector_index.vectors, [vec![0.0, 1.0]]);
    assert!(matches!(
        engine.resume_compute_embedding(&embedding_task, delayed_embedding),
        Err(MemoryEngineError::OperationStale)
    ));
    // Re-evaluate readiness through the public maintenance API after the
    // selected scope lost its only pending work. No new embedding is needed.
    assert!(engine.pending_embedding_backfill(SCOPE).unwrap().is_empty());
    assert_eq!(
        engine.vector_state(SCOPE).unwrap().status,
        VectorScopeStatus::Ready
    );
    assert!(!deep_recall_text(&engine, vec![1.0, 0.0]).contains("marigold"));
    assert!(deep_recall_text(&engine, vec![0.0, 1.0]).contains("copper buttons"));

    assert!(engine
        .storage()
        .read_candidate_belief_optional(TARGET_CANDIDATE)
        .unwrap()
        .is_none());
    assert!(engine
        .storage()
        .read_hypothesis_node(TARGET_CANDIDATE)
        .is_err());
    assert!(engine
        .storage()
        .read_candidate_belief_optional(SIBLING_CANDIDATE)
        .unwrap()
        .is_some());
    let nodes = engine.storage().read_hypothesis_nodes().unwrap().items;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].hypothesis_id, SIBLING_CANDIDATE);
    let links = engine
        .storage()
        .read_hypothesis_evidence_links()
        .unwrap()
        .items;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].hypothesis_id, SIBLING_CANDIDATE);
    assert_eq!(links[0].memory_unit_id, surviving_unit.memory_unit_id);

    let original_delivery =
        engine.ingest_idempotent_at_generation("target_delivery", target_input, generation);
    assert!(matches!(
        original_delivery,
        Err(MemoryEngineError::OperationStale)
    ));
    assert!(matches!(
        engine.next_sleep_batch(sleep),
        Err(MemoryEngineError::OperationStale)
    ));
    let delayed_verdict = engine.resume_memory_fidelity_pass(
        &fidelity.pending_task.task_id,
        serde_json::from_value(json!({
            "schema_version": "fidelity_review.v1",
            "memory_unit_id": erased_unit.memory_unit_id,
            "archive_id": episode.archive_id,
            "status": "valid",
            "confidence": 1.0,
            "explanation": "The frozen evidence used to support this memory."
        }))
        .unwrap(),
    );
    assert!(matches!(
        delayed_verdict,
        Err(MemoryEngineError::OperationStale)
    ));

    let latest_text = "I have started a new chapter.";
    let fresh_generation = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    engine
        .ingest_idempotent_at_generation(
            "fresh_delivery",
            event_input(SCOPE, SESSION, FIRST, latest_text, Some(vec![]), None),
            fresh_generation,
        )
        .unwrap();
    let replay = engine
        .erase_subject(SCOPE, FIRST, "erase_first_alex")
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), report);
    assert!(matches!(
        engine.erase_subject(SCOPE, SECOND, "erase_first_alex"),
        Err(MemoryEngineError::OperationConflict)
    ));
    let current = subject_view(&engine, SCOPE, FIRST);
    assert!(view_has(&current, "event", latest_text));
    assert!(!current.to_string().contains("marigold"));
}

#[test]
fn legacy_reply_with_opaque_copy_reports_incomplete_and_leaves_no_recall_text() {
    let fixture = Fixture::new("legacy_quote");
    let engine = fixture.open();
    let target = engine
        .ingest(event_input(
            SCOPE,
            SESSION,
            FIRST,
            TARGET_TEXT,
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    let mut legacy = event_input(
        SCOPE,
        SESSION,
        THIRD,
        &format!("My comments on the quote: {TARGET_TEXT}"),
        None,
        Some(&target.event_id),
    );
    legacy.payload["copied_quote"] = json!(TARGET_TEXT);
    let legacy = engine.ingest(legacy).unwrap().stored_event;
    let independent = engine
        .ingest(event_input(
            SCOPE,
            SESSION,
            THIRD,
            THIRD_TEXT,
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    let before = subject_view(&engine, SCOPE, FIRST);
    assert_eq!(before["incomplete"], true);
    assert!(view_has(&before, "event", TARGET_TEXT));

    let report = engine
        .erase_subject(SCOPE, FIRST, "erase_legacy_quote")
        .unwrap();
    let report = serde_json::to_value(report).unwrap();
    assert_eq!(report["incomplete"], true);
    assert_eq!(report["deleted_events"], 1);
    assert_eq!(report["suppressed_events"], 1);
    drop(engine);

    let engine = fixture.open();
    let remaining = session_events(&engine, SESSION);
    let legacy_row = remaining
        .iter()
        .find(|event| event.event_id == legacy.event_id)
        .unwrap();
    assert_eq!(legacy_row.speaker.as_ref().unwrap().id, THIRD);
    assert!(legacy_row.memory_text.is_none());
    assert!(!legacy_row.payload.to_string().contains(TARGET_TEXT));
    assert!(legacy_row
        .links
        .iter()
        .all(|link| link.target != target.event_id));
    assert!(remaining.contains(&independent));
    assert!(!scope_context(&engine, SCOPE, SESSION).contains("marigold"));
    let author = subject_view(&engine, SCOPE, THIRD);
    assert!(view_has(&author, "event", THIRD_TEXT));
    assert!(!author.to_string().contains("marigold"));
}

#[test]
fn erasing_a_former_scopes_archive_preserves_a_reused_session() {
    let fixture = Fixture::new("reused_session");
    let engine = fixture.open();
    let original = engine
        .ingest(event_input(
            SCOPE,
            SESSION,
            FIRST,
            TARGET_TEXT,
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    let archive = publish_archive(
        &engine,
        SESSION,
        vec![unit(TARGET_TEXT, FIRST, &[&original.event_id])],
        TARGET_TEXT,
    );
    assert!(
        engine
            .delete_session(SESSION)
            .unwrap()
            .archived_memory_preserved
    );
    let replacement = engine
        .ingest(event_input(
            OTHER_SCOPE,
            SESSION,
            FIRST,
            FOREIGN_TEXT,
            Some(vec![]),
            None,
        ))
        .unwrap()
        .stored_event;
    drop(engine);

    let engine = fixture.open();
    let report = engine
        .erase_subject(SCOPE, FIRST, "erase_prior_scope")
        .unwrap();
    assert_eq!(report.deleted_events, 0);
    assert_eq!(report.deleted_memory_units, 1);
    let prior = engine.inspect_archive(SCOPE, &archive.archive_id).unwrap();
    assert!(!serde_json::to_string(&prior).unwrap().contains(TARGET_TEXT));
    let current = engine.read_session(SESSION).unwrap();
    assert_eq!(
        current.metadata.effective_personality_scope_id(),
        OTHER_SCOPE
    );
    assert_eq!(current.metadata.event_count, 1);
    assert_eq!(current.events, [replacement]);
    assert!(view_has(
        &subject_view(&engine, OTHER_SCOPE, FIRST),
        "event",
        FOREIGN_TEXT
    ));
}

fn event_input(
    scope: &str,
    session: &str,
    speaker_id: &str,
    text: &str,
    quoted_event_ids: Option<Vec<&str>>,
    reply_to: Option<&str>,
) -> IngestEvent {
    let links = reply_to
        .map(|target| vec![json!({"kind": "reply_to", "target": target})])
        .unwrap_or_default();
    serde_json::from_value(json!({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "group_subject_test",
        "timestamp": "2026-05-01T12:00:00Z",
        "personality_scope_id": scope,
        "session_id": session,
        "memory_text": text,
        "payload": {"text": text},
        "speaker": {"id": speaker_id, "name": if speaker_id == THIRD { "Casey" } else { "Alex" }},
        "links": links,
        "quoted_event_ids": quoted_event_ids
    }))
    .unwrap()
}

fn publish_archive(
    engine: &MemoryEngine<FileStorage>,
    session: &str,
    units: Vec<Value>,
    text: &str,
) -> ArchiveEntry {
    let sleep = engine.sleep(session).unwrap();
    let archive_id = &sleep.archive_entry.archive_id;
    engine
        .resume_memory_unit_pass(
            &sleep.memory_unit_task.unwrap().task_id,
            serde_json::from_value(json!({
                "schema_version": "memory_units_result.v1",
                "archive_id": archive_id,
                "memory_units": units
            }))
            .unwrap(),
        )
        .unwrap();
    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            serde_json::from_value(json!({
                "schema_version": "sleep_compression_result.v1",
                "archive_id": archive_id,
                "gist": text,
                "narrative": text,
                "weight": 0.5
            }))
            .unwrap(),
        )
        .unwrap()
}

fn unit(thesis: &str, speaker: &str, event_ids: &[&str]) -> Value {
    json!({
        "thesis": thesis,
        "subject_hint": "self_statement",
        "speaker_id": speaker,
        "source_event_ids": event_ids,
        "weight": 0.5
    })
}

fn add_core_fact(engine: &MemoryEngine<FileStorage>, scope: &str, subject: &str, text: &str) {
    engine
        .upsert_core_fact(
            serde_json::from_value(json!({
                "schema_version": "core_fact_input.v1",
                "category": "personal_notes",
                "scope": scope,
                "subject": {"id": subject, "name": "Alex"},
                "text": text,
                "confidence": 1.0
            }))
            .unwrap(),
        )
        .unwrap();
}

fn add_candidate(engine: &MemoryEngine<FileStorage>, id: &str, source: &MemoryUnit) {
    let candidate: CandidateBelief = serde_json::from_value(json!({
        "schema_version": "candidate_belief.v1",
        "candidate_id": id,
        "source_session_id": source.source_session_id,
        "source_session_ids": [source.source_session_id],
        "core_scope": source.personality_scope_id,
        "subject": source.subject,
        "created_at": source.created_at,
        "updated_at": source.updated_at,
        "text": source.thesis,
        "category": "personal_notes",
        "status": "ready_for_review",
        "confidence": 0.95,
        "evidence_summary": source.thesis,
        "source_memory_unit_ids": [source.memory_unit_id],
        "promotion_checks": {
            "min_sources_met": true,
            "weight_threshold_met": true,
            "no_recent_contradiction": true,
            "manual_review_required": true
        }
    }))
    .unwrap();
    engine.storage().write_candidate_belief(&candidate).unwrap();
}

fn deep_recall_text(engine: &MemoryEngine<FileStorage>, query_vec: Vec<f32>) -> String {
    let recall = engine
        .recall_deep(DeepRecallQuery {
            scope: SCOPE.into(),
            query_vec,
            model_id: VECTOR_MODEL.into(),
            top_k: 5,
            min_sim: 0.5,
            now: None,
        })
        .unwrap();
    recall
        .hits
        .into_iter()
        .map(|hit| hit.thesis)
        .collect::<Vec<_>>()
        .join("\n")
}

fn embedding_result(batch: EmbedBatchInputs, vector: Vec<f32>) -> EmbedBatchResult {
    assert_eq!(batch.items.len(), 1);
    EmbedBatchResult {
        schema_version: EMBED_BATCH_RESULT_SCHEMA_VERSION.into(),
        model_id: batch.model_id,
        dim: batch.dim,
        results: vec![EmbedBatchVector {
            memory_unit_id: batch.items[0].memory_unit_id.clone(),
            vector,
        }],
    }
}

fn subject_view(engine: &MemoryEngine<FileStorage>, scope: &str, subject: &str) -> Value {
    serde_json::to_value(engine.inspect_subject(scope, subject).unwrap()).unwrap()
}

fn view_has(view: &Value, kind: &str, text: &str) -> bool {
    view["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["kind"] == kind && item["text"].as_str().is_some_and(|s| s.contains(text)))
}

fn session_events(engine: &MemoryEngine<FileStorage>, session: &str) -> Vec<StoredEvent> {
    let mut events = engine
        .storage()
        .read_session_archived_events(session)
        .unwrap();
    events.extend(engine.read_session(session).unwrap().events);
    events
}

fn scope_context(engine: &MemoryEngine<FileStorage>, scope: &str, session: &str) -> String {
    serde_json::to_string(
        &engine
            .core_context_package(
                serde_json::from_value(json!({
                    "schema_version": "core_context_request.v1",
                    "session_id": session,
                    "core_scope": scope,
                    "query_text": "What did these people discuss?",
                    "include_core": true
                }))
                .unwrap(),
            )
            .unwrap(),
    )
    .unwrap()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new(case: &str) -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let root = std::env::temp_dir().join(format!(
            "memory_engine_subject_{case}_{}_{}",
            std::process::id(),
            stamp.as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        let mut options = EngineOptions::default();
        options.vectors.model_id = VECTOR_MODEL.into();
        options.vectors.dim = 2;
        options.vectors.embed_batch_size = 1;
        MemoryEngine::with_options(FileStorage::with_host_id(&self.0, "subject_test"), options)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
