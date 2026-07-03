use memory_engine::archive::{
    ArchiveEntry, ArchiveStatus, FidelityStatus, MemoryUnit, MemoryUnitStatus,
};
use memory_engine::event::IngestEvent;
use memory_engine::llm::LlmResponse;
use memory_engine::storage::Storage;
use memory_engine::tasks::TaskType;
use memory_engine::types::{
    EVENT_SCHEMA_VERSION, FORGET_REVIEW_RESULT_SCHEMA_VERSION, MEMORY_UNIT_SCHEMA_VERSION,
};
use memory_engine::vector::{
    EmbedBatchResult, EmbedBatchVector, VectorScopeStatus, EMBED_BATCH_RESULT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn disabled_vector_scope_emits_no_embedding_tasks() {
    let root = unique_temp_dir("vectors_disabled");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    create_archive_with_units(&engine, "vector_scope", vec![("User likes quasars.", 0.8)]);

    let state = engine.vector_state("vector_scope").expect("vector state");
    assert_eq!(state.status, VectorScopeStatus::Disabled);
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    assert!(requests.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn vector_backfill_appends_embeddings_and_rejects_model_mismatch() {
    let root = unique_temp_dir("vectors_backfill");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    create_archive_with_units(
        &engine,
        "vector_scope",
        vec![("User keeps returning to astronomy.", 0.8)],
    );
    let state = engine
        .set_vector_scope("vector_scope", true, false)
        .expect("enable vectors");
    assert_eq!(state.status, VectorScopeStatus::Building);

    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].prompt_id, "embed_batch");
    assert_eq!(requests[0].role_hint, memory_engine::types::ModelRole::Fast);

    let task_id = requests[0].task_id.clone();
    let memory_unit_id = requests[0].prompt_inputs["embed_batch"]["items"][0]["memory_unit_id"]
        .as_str()
        .expect("memory unit id")
        .to_string();
    let bad = EmbedBatchResult {
        schema_version: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
        model_id: "wrong-model".to_string(),
        dim: 384,
        results: vec![EmbedBatchVector {
            memory_unit_id: memory_unit_id.clone(),
            vector: unit_vector(384, 0),
        }],
    };
    assert!(engine.resume_compute_embedding(&task_id, bad).is_err());

    let good = EmbedBatchResult {
        schema_version: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
        model_id: "intfloat/multilingual-e5-small".to_string(),
        dim: 384,
        results: vec![EmbedBatchVector {
            memory_unit_id,
            vector: unit_vector(384, 0),
        }],
    };
    let appended = engine
        .resume_compute_embedding(&task_id, good)
        .expect("resume embedding");
    assert_eq!(appended, 1);
    let state = engine.vector_state("vector_scope").expect("vector state");
    assert_eq!(state.status, VectorScopeStatus::Ready);
    assert_eq!(state.rows, 1);

    fs::remove_dir_all(root).ok();
}

#[test]
fn multi_speaker_scope_emits_no_embedding_tasks() {
    let root = unique_temp_dir("vectors_multi_speaker");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    ingest_user_event_with_speaker(&engine, "group_scope", "speaker_a", "A", "I like bikes.");
    ingest_user_event_with_speaker(&engine, "group_scope", "speaker_b", "B", "I like planes.");
    create_archive_with_units(
        &engine,
        "group_scope",
        vec![("A likes bikes; B likes planes.", 0.8)],
    );
    engine
        .set_vector_scope("group_scope", true, false)
        .expect("enable vectors");
    let requests = engine
        .pending_embedding_backfill("group_scope")
        .expect("pending embedding backfill");
    assert!(requests.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn forgotten_unit_is_tombstoned_and_remember_back_reembeds() {
    let root = unique_temp_dir("vectors_forgetting");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    let unit_id = create_archive_with_units(
        &engine,
        "vector_scope",
        vec![("Routine lunch note -> user mentioned soup.", 0.2)],
    )[0]
    .clone();
    engine
        .set_vector_scope("vector_scope", true, false)
        .expect("enable vectors");
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    submit_embedding(&engine, &requests[0], 0);
    assert_eq!(engine.vector_state("vector_scope").expect("state").rows, 1);

    let start = engine
        .begin_forget_review("vector_scope")
        .expect("begin forget review");
    assert_eq!(start.pending_task.task_type, TaskType::ForgetReview);
    let response = LlmResponse::Ok {
        request_id: start.request.request_id,
        text: json!({
            "schema_version": FORGET_REVIEW_RESULT_SCHEMA_VERSION,
            "source_session_id": "vector_scope",
            "recommendations": [{
                "memory_unit_id": unit_id,
                "decision": "forget",
                "reason": "Routine detail."
            }]
        })
        .to_string(),
    };
    let applied = engine
        .submit_forget_review_response(&start.pending_task.task_id, response)
        .expect("forget unit");
    assert_eq!(applied.forgotten, 1);
    let forgotten = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read forgotten unit");
    assert_eq!(forgotten.status, MemoryUnitStatus::Forgotten);

    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("compact after tombstone");
    assert!(requests.is_empty());
    assert_eq!(engine.vector_state("vector_scope").expect("state").rows, 0);

    engine.remember_back(&unit_id).expect("remember back");
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("re-embed remembered unit");
    assert_eq!(requests.len(), 1);
    submit_embedding(&engine, &requests[0], 1);
    assert_eq!(engine.vector_state("vector_scope").expect("state").rows, 1);

    fs::remove_dir_all(root).ok();
}

fn create_archive_with_units(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    units: Vec<(&str, f64)>,
) -> Vec<String> {
    let event = ingest_user_event(engine, session_id, "Hello memory.");
    let now = "2020-01-01T10:00:00Z".to_string();
    let unit_values = units
        .into_iter()
        .enumerate()
        .map(|(index, (thesis, weight))| MemoryUnit {
            schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
            memory_unit_id: format!("mu_{session_id}_{index}"),
            archive_id: format!("archive_{session_id}"),
            source_session_id: session_id.to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
            thesis: thesis.to_string(),
            source_event_ids: vec![event.event_id.clone()],
            evidence: None,
            tags: Vec::new(),
            weight,
            status: MemoryUnitStatus::ActiveArchive,
            fidelity_status: FidelityStatus::Valid,
            fidelity_review: None,
            forget_review: None,
        })
        .collect::<Vec<_>>();
    let archive = ArchiveEntry {
        schema_version: "archive_entry.v1".to_string(),
        archive_id: format!("archive_{session_id}"),
        created_at: now.clone(),
        updated_at: now.clone(),
        source_session_id: session_id.to_string(),
        source_event_ids: vec![event.event_id],
        time_range: memory_engine::types::TimeRange {
            start: now.clone(),
            end: now.clone(),
        },
        theme: None,
        tags: Vec::new(),
        gist: "Test archive.".to_string(),
        narrative: "Test archive.".to_string(),
        compact_memory: Some(
            unit_values
                .iter()
                .map(|unit| unit.thesis.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        memory_units: unit_values.clone(),
        facts: Vec::new(),
        quotes: Vec::new(),
        weight: 0.5,
        freshness: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        links: Vec::new(),
        emotional_markers: Vec::new(),
        topic_thread: Vec::new(),
        personal_signals: Vec::new(),
        relational_tone: None,
        status: ArchiveStatus::Complete,
        llm_enhanced: true,
        prompt_id: None,
        prompt_version: None,
        embedding_model_id: None,
        embedding: None,
    };
    let storage = engine.storage();
    storage
        .write_archive_entry(&archive)
        .expect("write archive");
    for unit in &unit_values {
        storage.write_memory_unit(unit).expect("write unit");
    }
    unit_values
        .iter()
        .map(|unit| unit.memory_unit_id.clone())
        .collect()
}

fn ingest_user_event(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    text: &str,
) -> memory_engine::event::StoredEvent {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test".to_string(),
            timestamp: "2020-01-01T10:00:00Z".to_string(),
            session_id: session_id.to_string(),
            payload: json!({ "text": text }),
            tags: Vec::new(),
            theme: None,
            emotional_tone: None,
            speaker: None,
            links: Vec::new(),
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        })
        .expect("ingest")
        .stored_event
}

fn ingest_user_event_with_speaker(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    speaker_id: &str,
    speaker_name: &str,
    text: &str,
) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test".to_string(),
            timestamp: "2020-01-01T10:00:00Z".to_string(),
            session_id: session_id.to_string(),
            payload: json!({ "text": text }),
            tags: Vec::new(),
            theme: None,
            emotional_tone: None,
            speaker: Some(memory_engine::types::Speaker {
                id: speaker_id.to_string(),
                name: speaker_name.to_string(),
            }),
            links: Vec::new(),
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        })
        .expect("ingest speaker event");
}

fn submit_embedding(
    engine: &MemoryEngine<FileStorage>,
    request: &memory_engine::llm::LlmRequest,
    hot_index: usize,
) {
    let memory_unit_id = request.prompt_inputs["embed_batch"]["items"][0]["memory_unit_id"]
        .as_str()
        .expect("memory unit id")
        .to_string();
    let result = EmbedBatchResult {
        schema_version: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
        model_id: "intfloat/multilingual-e5-small".to_string(),
        dim: 384,
        results: vec![EmbedBatchVector {
            memory_unit_id,
            vector: unit_vector(384, hot_index),
        }],
    };
    engine
        .resume_compute_embedding(&request.task_id, result)
        .expect("submit embedding");
}

fn unit_vector(dim: usize, hot_index: usize) -> Vec<f32> {
    let mut vector = vec![0.0; dim];
    vector[hot_index] = 1.0;
    vector
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("layers_memory_{label}_{nanos}"))
}
