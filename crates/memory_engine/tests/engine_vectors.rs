use memory_engine::archive::{
    ArchiveEntry, ArchiveStatus, FidelityStatus, MemoryUnit, MemoryUnitStatus,
};
use memory_engine::core_store::{CoreContextQueryEmbedding, CoreContextRequest};
use memory_engine::event::IngestEvent;
use memory_engine::llm::LlmResponse;
use memory_engine::storage::Storage;
use memory_engine::tasks::TaskType;
use memory_engine::types::{
    CORE_CONTEXT_REQUEST_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, FORGET_REVIEW_RESULT_SCHEMA_VERSION,
    MEMORY_UNIT_SCHEMA_VERSION,
};
use memory_engine::vector::{
    DeepRecallQuery, EmbedBatchResult, EmbedBatchVector, VectorScopeStatus, DEFAULT_VECTOR_DIM,
    DEFAULT_VECTOR_MODEL_ID, EMBED_BATCH_RESULT_SCHEMA_VERSION,
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
        dim: DEFAULT_VECTOR_DIM,
        results: vec![EmbedBatchVector {
            memory_unit_id: memory_unit_id.clone(),
            vector: unit_vector(DEFAULT_VECTOR_DIM, 0),
        }],
    };
    assert!(engine.resume_compute_embedding(&task_id, bad).is_err());

    let good = EmbedBatchResult {
        schema_version: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
        model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
        dim: DEFAULT_VECTOR_DIM,
        results: vec![EmbedBatchVector {
            memory_unit_id,
            vector: unit_vector(DEFAULT_VECTOR_DIM, 0),
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

#[test]
fn deep_recall_returns_disabled_without_catalog() {
    let root = unique_temp_dir("vectors_deep_disabled");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    let result = engine
        .recall_deep(deep_query(
            "vector_scope",
            unit_vector(DEFAULT_VECTOR_DIM, 0),
            5,
            0.75,
        ))
        .expect("deep recall");

    assert!(!result.found);
    assert_eq!(result.reason.as_deref(), Some("disabled"));
    assert!(result.hits.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn deep_recall_finds_hits_limits_results_and_records_recall_stats() {
    let root = unique_temp_dir("vectors_deep_hits");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    let unit_ids = create_archive_with_units(
        &engine,
        "vector_scope",
        vec![
            ("User loves astronomy.", 0.8),
            ("User keeps a cat named Irzha.", 0.9),
        ],
    );
    engine
        .set_vector_scope("vector_scope", true, false)
        .expect("enable vectors");
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    submit_embeddings(&engine, &requests[0], vec![0, 1]);

    let result = engine
        .recall_deep(deep_query(
            "vector_scope",
            unit_vector(DEFAULT_VECTOR_DIM, 1),
            1,
            0.75,
        ))
        .expect("deep recall");

    assert!(result.found);
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].memory_unit_id, unit_ids[1]);
    assert!(result.hits[0].sim > 0.99);

    assert_eq!(engine.flush_recall_stats().expect("flush stats"), 1);
    let archive = storage_probe
        .read_archive_entry_by_id("archive_vector_scope")
        .expect("read archive");
    assert_eq!(archive.recall_count, 1);
    assert_eq!(
        archive.last_recalled_at.as_deref(),
        Some("2020-07-01T00:00:00Z")
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn deep_recall_enforces_scope_isolation_threshold_and_model_dim() {
    let root = unique_temp_dir("vectors_deep_scope");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    create_archive_with_units(&engine, "scope_a", vec![("User likes astronomy.", 0.8)]);
    create_archive_with_units(
        &engine,
        "scope_b",
        vec![("User keeps a cat named Irzha.", 0.8)],
    );
    for (scope, hot_index) in [("scope_a", 0usize), ("scope_b", 1usize)] {
        engine
            .set_vector_scope(scope, true, false)
            .expect("enable vectors");
        let requests = engine
            .pending_embedding_backfill(scope)
            .expect("pending embedding backfill");
        submit_embedding(&engine, &requests[0], hot_index);
    }

    let result = engine
        .recall_deep(deep_query(
            "scope_a",
            unit_vector(DEFAULT_VECTOR_DIM, 1),
            5,
            0.9,
        ))
        .expect("deep recall");
    assert!(!result.found);
    assert_eq!(result.reason.as_deref(), Some("below_threshold"));

    let wrong_model = DeepRecallQuery {
        model_id: "wrong-model".to_string(),
        ..deep_query("scope_a", unit_vector(DEFAULT_VECTOR_DIM, 0), 5, 0.75)
    };
    assert!(engine.recall_deep(wrong_model).is_err());

    let wrong_dim = DeepRecallQuery {
        query_vec: unit_vector(3, 0),
        ..deep_query("scope_a", unit_vector(DEFAULT_VECTOR_DIM, 0), 5, 0.75)
    };
    assert!(engine.recall_deep(wrong_dim).is_err());

    fs::remove_dir_all(root).ok();
}

#[test]
fn deep_recall_score_uses_recency_weight_and_top_k() {
    let root = unique_temp_dir("vectors_deep_ranking");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    let unit_ids = create_archive_with_units_at(
        &engine,
        "vector_scope",
        vec![
            (
                "Old but semantically matching note.",
                0.5,
                "2019-01-01T00:00:00Z",
            ),
            (
                "Fresh semantically matching note.",
                0.5,
                "2020-06-30T00:00:00Z",
            ),
            ("Weakly related note.", 0.5, "2020-06-30T00:00:00Z"),
        ],
    );
    engine
        .set_vector_scope("vector_scope", true, false)
        .expect("enable vectors");
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    submit_embeddings(&engine, &requests[0], vec![0, 0, 1]);

    let result = engine
        .recall_deep(deep_query(
            "vector_scope",
            unit_vector(DEFAULT_VECTOR_DIM, 0),
            2,
            0.75,
        ))
        .expect("deep recall");

    assert!(result.found);
    assert_eq!(result.hits.len(), 2);
    assert_eq!(result.hits[0].memory_unit_id, unit_ids[1]);
    assert_eq!(result.hits[1].memory_unit_id, unit_ids[0]);

    fs::remove_dir_all(root).ok();
}

#[test]
fn core_context_package_expands_with_vector_detail_when_query_embedding_is_present() {
    let root = unique_temp_dir("vectors_context_expansion");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    create_archive_with_units_named(
        &engine,
        "vector_scope",
        "current_topic",
        "The current visible topic is garden planning.",
        vec![("The current visible topic is garden planning.", 0.7)],
    );
    create_archive_with_units_named(
        &engine,
        "vector_scope",
        "irzha_detail",
        "Irzha detail.",
        vec![(
            "Irzha is a tortoiseshell cat with black fur and rusty orange patches.",
            0.9,
        )],
    );
    engine
        .set_vector_scope("vector_scope", true, false)
        .expect("enable vectors");
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    submit_embeddings(&engine, &requests[0], vec![0, 1]);

    let without_embedding = engine
        .core_context_package(CoreContextRequest {
            recall_limit: 1,
            query_text: Some("garden planning".to_string()),
            ..context_request("vector_scope", None)
        })
        .expect("context without embedding");
    assert_eq!(without_embedding.archive_relevant.len(), 1);
    assert!(!without_embedding.archive_relevant[0]
        .tags
        .iter()
        .any(|tag| tag == "contextual_expansion"));

    let with_embedding = engine
        .core_context_package(CoreContextRequest {
            recall_limit: 1,
            query_text: Some("garden planning".to_string()),
            query_embedding: Some(CoreContextQueryEmbedding {
                model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
                query_vec: unit_vector(DEFAULT_VECTOR_DIM, 1),
            }),
            ..context_request("vector_scope", None)
        })
        .expect("context with embedding");

    assert_eq!(with_embedding.archive_relevant.len(), 2);
    assert_eq!(
        with_embedding.archive_relevant[0].compact_memory.as_deref(),
        Some("Irzha is a tortoiseshell cat with black fur and rusty orange patches.")
    );
    assert!(with_embedding.archive_relevant[0]
        .tags
        .iter()
        .any(|tag| tag == "contextual_expansion"));
    assert!(with_embedding
        .notes
        .iter()
        .any(|note| note.contains("contextual memory expansion added 1")));

    fs::remove_dir_all(root).ok();
}

#[test]
fn core_context_package_skips_contextual_detail_already_visible_in_long_memory() {
    let root = unique_temp_dir("vectors_context_expansion_dedup");
    let storage = FileStorage::with_host_id(&root, "vector_test");
    let engine = MemoryEngine::new(storage);

    create_archive_with_units(
        &engine,
        "vector_scope",
        vec![(
            "Irzha is a tortoiseshell cat with rusty orange patches.",
            0.9,
        )],
    );
    engine
        .set_vector_scope("vector_scope", true, false)
        .expect("enable vectors");
    let requests = engine
        .pending_embedding_backfill("vector_scope")
        .expect("pending embedding backfill");
    submit_embedding(&engine, &requests[0], 1);

    let package = engine
        .core_context_package(CoreContextRequest {
            recall_limit: 1,
            query_text: Some("Irzha tortoiseshell rusty orange patches".to_string()),
            query_embedding: Some(CoreContextQueryEmbedding {
                model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
                query_vec: unit_vector(DEFAULT_VECTOR_DIM, 1),
            }),
            ..context_request("vector_scope", None)
        })
        .expect("context with visible long memory");

    assert_eq!(package.archive_relevant.len(), 1);
    assert!(!package.archive_relevant[0]
        .tags
        .iter()
        .any(|tag| tag == "contextual_expansion"));
    assert!(package
        .notes
        .iter()
        .any(|note| note.contains("contextual memory expansion found only already-visible")));

    fs::remove_dir_all(root).ok();
}

fn create_archive_with_units(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    units: Vec<(&str, f64)>,
) -> Vec<String> {
    create_archive_with_units_at(
        engine,
        session_id,
        units
            .into_iter()
            .map(|(thesis, weight)| (thesis, weight, "2020-01-01T10:00:00Z"))
            .collect(),
    )
}

fn create_archive_with_units_at(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    units: Vec<(&str, f64, &str)>,
) -> Vec<String> {
    create_archive_with_units_named_at(engine, session_id, session_id, "Test archive.", units)
}

fn create_archive_with_units_named(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    archive_suffix: &str,
    gist: &str,
    units: Vec<(&str, f64)>,
) -> Vec<String> {
    create_archive_with_units_named_at(
        engine,
        session_id,
        archive_suffix,
        gist,
        units
            .into_iter()
            .map(|(thesis, weight)| (thesis, weight, "2020-01-01T10:00:00Z"))
            .collect(),
    )
}

fn create_archive_with_units_named_at(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    archive_suffix: &str,
    gist: &str,
    units: Vec<(&str, f64, &str)>,
) -> Vec<String> {
    let event = ingest_user_event(engine, session_id, "Hello memory.");
    let archive_time = units
        .first()
        .map(|(_, _, created_at)| (*created_at).to_string())
        .unwrap_or_else(|| "2020-01-01T10:00:00Z".to_string());
    let archive_id = format!("archive_{archive_suffix}");
    let unit_values = units
        .into_iter()
        .enumerate()
        .map(|(index, (thesis, weight, created_at))| MemoryUnit {
            schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
            memory_unit_id: format!("mu_{archive_suffix}_{index}"),
            archive_id: archive_id.clone(),
            source_session_id: session_id.to_string(),
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
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
        archive_id,
        created_at: archive_time.clone(),
        updated_at: archive_time.clone(),
        source_session_id: session_id.to_string(),
        source_event_ids: vec![event.event_id],
        time_range: memory_engine::types::TimeRange {
            start: archive_time.clone(),
            end: archive_time.clone(),
        },
        theme: None,
        tags: Vec::new(),
        gist: gist.to_string(),
        narrative: gist.to_string(),
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
    submit_embeddings(engine, request, vec![hot_index]);
}

fn submit_embeddings(
    engine: &MemoryEngine<FileStorage>,
    request: &memory_engine::llm::LlmRequest,
    hot_indices: Vec<usize>,
) {
    let items = request.prompt_inputs["embed_batch"]["items"]
        .as_array()
        .expect("embed items");
    assert_eq!(items.len(), hot_indices.len());
    let results = items
        .iter()
        .zip(hot_indices)
        .map(|(item, hot_index)| EmbedBatchVector {
            memory_unit_id: item["memory_unit_id"]
                .as_str()
                .expect("memory unit id")
                .to_string(),
            vector: unit_vector(DEFAULT_VECTOR_DIM, hot_index),
        })
        .collect();
    let result = EmbedBatchResult {
        schema_version: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
        model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
        dim: DEFAULT_VECTOR_DIM,
        results,
    };
    engine
        .resume_compute_embedding(&request.task_id, result)
        .expect("submit embedding");
}

fn deep_query(scope: &str, query_vec: Vec<f32>, top_k: usize, min_sim: f32) -> DeepRecallQuery {
    DeepRecallQuery {
        scope: scope.to_string(),
        query_vec,
        model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
        top_k,
        min_sim,
        now: Some("2020-07-01T00:00:00Z".to_string()),
    }
}

fn context_request(scope: &str, query_vec: Option<Vec<f32>>) -> CoreContextRequest {
    CoreContextRequest {
        schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
        session_id: scope.to_string(),
        domain_state: json!({ "current_text": "Tell me about the current topic." }),
        core_scope: Some(scope.to_string()),
        query_text: Some("Tell me about the current topic.".to_string()),
        query_embedding: query_vec.map(|query_vec| CoreContextQueryEmbedding {
            model_id: DEFAULT_VECTOR_MODEL_ID.to_string(),
            query_vec,
        }),
        recall_limit: 0,
        session_recent_limit: 0,
        session_trace_event_limit: 0,
        include_core: false,
        token_budget: None,
        utc_offset_minutes: 0,
        clock_untrusted: false,
    }
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
