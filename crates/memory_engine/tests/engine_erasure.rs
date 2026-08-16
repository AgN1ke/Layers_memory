use memory_engine::archive::ArchiveStatus;
use memory_engine::core_store::{
    CandidateBelief, CandidateStatus, CoreContextRequest, PromotionChecks,
};
use memory_engine::erasure::{
    ErasureCoreFactRef, MemoryErasureCounts, MemoryErasureKind, MemoryErasureOperationIntent,
    MemoryErasurePlan,
};
use memory_engine::event::IngestEvent;
use memory_engine::journal::{
    JournalOperation, JournalOperationType, JournalState, RecoveryPolicy,
};
use memory_engine::llm::{LlmResponse, SleepOutcome};
use memory_engine::recall::{RecallFilters, RecallQuery};
use memory_engine::storage::Storage;
use memory_engine::types::{
    CANDIDATE_BELIEF_SCHEMA_VERSION, CORE_CONTEXT_REQUEST_SCHEMA_VERSION, EVENT_SCHEMA_VERSION,
    JOURNAL_OPERATION_SCHEMA_VERSION, MEMORY_ERASURE_OPERATION_SCHEMA_VERSION,
    MEMORY_ERASURE_PLAN_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION,
    RECALL_QUERY_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn delete_session_removes_conversation_records_but_preserves_scope_memory() {
    let root = unique_temp_dir("delete_session");
    let storage = FileStorage::with_host_id(&root, "erasure_test");
    let engine = MemoryEngine::new(storage.clone());
    let seed = seed_scope_memory(&engine, "chat_old", "owner_scope", "Alice lives in Kyiv.");
    ingest_text(
        &engine,
        "chat_current",
        "owner_scope",
        "A new conversation starts here.",
    );

    let report = engine.delete_session("chat_old").expect("delete session");
    assert_eq!(report.operation, MemoryErasureKind::DeleteSession);
    assert!(report.archived_memory_preserved);
    assert_eq!(report.deleted.sessions, 1);
    assert!(report.deleted.archived_events >= 1);
    assert!(report.deleted.tasks >= 1);
    assert!(storage.read_session("chat_old").is_err());
    assert!(storage.read_session("chat_current").is_ok());
    assert!(storage.read_archive_entry_by_id(&seed.archive_id).is_ok());
    assert!(storage.read_memory_unit_by_id(&seed.memory_unit_id).is_ok());
    assert!(storage
        .read_core_store_categories()
        .expect("core categories")
        .items
        .into_iter()
        .flat_map(|category| category.facts)
        .any(|fact| fact.scope.as_deref() == Some("owner_scope")
            && fact.text == "Alice lives in Kyiv."));
    assert!(storage
        .load_all_tasks()
        .expect("all tasks")
        .items
        .iter()
        .all(|task| !serde_json::to_string(task)
            .expect("serialize task")
            .contains("chat_old")));

    let repeated = engine
        .delete_session("chat_old")
        .expect("repeat delete session");
    assert!(repeated.deleted.is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn purge_scope_removes_all_owned_data_and_preserves_neighbor_scope() {
    let root = unique_temp_dir("purge_scope");
    let storage = FileStorage::with_host_id(&root, "erasure_test");
    let engine = MemoryEngine::new(storage.clone());
    let target = seed_scope_memory(
        &engine,
        "target_chat",
        "target_scope",
        "Alice lives in Kyiv.",
    );
    let neighbor = seed_scope_memory(
        &engine,
        "neighbor_chat",
        "neighbor_scope",
        "Bob lives in Lviv.",
    );
    seed_candidate(&storage, "target_scope", "target_chat", &target);
    engine
        .bind_reflection_session("target_chat", Some("target_scope".to_string()))
        .expect("bind reflection state");
    engine
        .set_vector_scope("target_scope", true, false)
        .expect("enable target vectors");
    assert!(!engine
        .pending_embedding_backfill("target_scope")
        .expect("target backfill")
        .is_empty());

    let report = engine.purge_scope("target_scope").expect("purge scope");
    assert_eq!(report.operation, MemoryErasureKind::PurgeScope);
    assert!(!report.archived_memory_preserved);
    assert_eq!(report.deleted.sessions, 1);
    assert_eq!(report.deleted.archives, 1);
    assert_eq!(report.deleted.memory_units, 1);
    assert_eq!(report.deleted.core_facts, 1);
    assert_eq!(report.deleted.candidates, 1);
    assert!(report.deleted.hypothesis_nodes >= 1);
    assert_eq!(report.deleted.vector_scopes, 1);

    assert!(storage.read_session("target_chat").is_err());
    assert!(storage.read_session("neighbor_chat").is_ok());
    assert!(storage
        .read_archive_entry_by_id(&target.archive_id)
        .is_err());
    assert!(storage
        .read_archive_entry_by_id(&neighbor.archive_id)
        .is_ok());
    assert!(storage
        .read_memory_unit_by_id(&target.memory_unit_id)
        .is_err());
    assert!(storage
        .read_memory_unit_by_id(&neighbor.memory_unit_id)
        .is_ok());
    assert!(storage
        .read_core_store_categories()
        .expect("core after purge")
        .items
        .into_iter()
        .flat_map(|category| category.facts)
        .all(|fact| fact.scope.as_deref() != Some("target_scope")));
    assert!(engine
        .list_candidates()
        .expect("candidates after purge")
        .iter()
        .all(|candidate| candidate.core_scope.as_deref() != Some("target_scope")));
    assert!(storage
        .read_hypothesis_nodes()
        .expect("hypotheses after purge")
        .items
        .iter()
        .all(|node| node.core_scope.as_deref() != Some("target_scope")));
    assert!(storage
        .read_vector_index("target_scope")
        .expect("target vectors after purge")
        .is_none());

    let empty_target = engine
        .core_context_package(core_context_request("neighbor_chat", "target_scope"))
        .expect("empty target context");
    assert!(empty_target.core_facts.is_empty());
    let target_recall = engine
        .recall(RecallQuery {
            schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            query_id: None,
            created_at: Some("2026-08-16T12:00:00Z".to_string()),
            personality_scope_id: Some("target_scope".to_string()),
            session_id: None,
            context: json!({}),
            query_text: Some("Where does Alice live?".to_string()),
            query_embedding: None,
            filters: RecallFilters::default(),
            limit: 10,
            include_core: false,
            explain: false,
        })
        .expect("target recall after purge");
    assert!(target_recall.items.is_empty());
    let neighbor_context = engine
        .core_context_package(core_context_request("neighbor_chat", "neighbor_scope"))
        .expect("neighbor context");
    assert!(neighbor_context
        .core_facts
        .iter()
        .any(|fact| fact.text == "Bob lives in Lviv."));

    let repeated = engine
        .purge_scope("target_scope")
        .expect("repeat purge scope");
    assert!(repeated.deleted.is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn recovery_replays_a_partially_applied_scope_purge() {
    let root = unique_temp_dir("purge_recovery");
    let storage = FileStorage::with_host_id(&root, "erasure_test");
    let engine = MemoryEngine::new(storage.clone());
    let seed = seed_scope_memory(
        &engine,
        "recover_chat",
        "recover_scope",
        "Alice lives in Kyiv.",
    );
    let core_fact = storage
        .read_core_store_categories()
        .expect("core before recovery")
        .items
        .into_iter()
        .flat_map(|category| {
            let name = category.category;
            category
                .facts
                .into_iter()
                .filter(|fact| fact.scope.as_deref() == Some("recover_scope"))
                .map(move |fact| ErasureCoreFactRef {
                    category: name.clone(),
                    core_fact_id: fact.core_fact_id,
                })
        })
        .next()
        .expect("recover Core fact");
    let sleep_task_ids = storage
        .load_sleep_runs()
        .expect("sleep runs")
        .items
        .into_iter()
        .filter(|run| run.session_id == "recover_chat")
        .map(|run| run.sleep_task_id)
        .collect::<Vec<_>>();
    let task_ids = storage
        .load_all_tasks()
        .expect("tasks")
        .items
        .into_iter()
        .filter(|task| {
            serde_json::to_string(task)
                .expect("serialize task")
                .contains("recover_chat")
                || sleep_task_ids.contains(&task.task_id)
        })
        .map(|task| task.task_id)
        .collect::<Vec<_>>();
    let plan = MemoryErasurePlan {
        schema_version: MEMORY_ERASURE_PLAN_SCHEMA_VERSION.to_string(),
        operation: MemoryErasureKind::PurgeScope,
        target_id: "recover_scope".to_string(),
        session_ids: vec!["recover_chat".to_string()],
        archive_ids: vec![seed.archive_id.clone()],
        memory_unit_ids: vec![seed.memory_unit_id.clone()],
        core_facts: vec![core_fact],
        candidate_ids: Vec::new(),
        hypothesis_node_ids: Vec::new(),
        hypothesis_evidence_link_ids: Vec::new(),
        hypothesis_relation_ids: Vec::new(),
        task_ids,
        sleep_task_ids,
        reflection_scopes: Vec::new(),
        vector_scopes: Vec::new(),
        journal_operation_ids: Vec::new(),
    };
    let counts = MemoryErasureCounts {
        sessions: 1,
        archived_events: 1,
        archives: 1,
        memory_units: 1,
        core_facts: 1,
        tasks: plan.task_ids.len(),
        sleep_runs: plan.sleep_task_ids.len(),
        ..MemoryErasureCounts::default()
    };
    let operation = JournalOperation {
        schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
        op_id: "memory_erasure_recovery_fixture".to_string(),
        op_type: JournalOperationType::PurgeScope,
        state: JournalState::Started,
        created_at: "2026-08-16T10:00:00Z".to_string(),
        updated_at: "2026-08-16T10:00:00Z".to_string(),
        target_files: vec!["recover_scope".to_string()],
        intent: serde_json::to_value(MemoryErasureOperationIntent {
            schema_version: MEMORY_ERASURE_OPERATION_SCHEMA_VERSION.to_string(),
            plan: plan.clone(),
            planned_counts: counts,
            archived_memory_preserved: false,
        })
        .expect("serialize erasure intent"),
        recovery_policy: RecoveryPolicy::Retry,
        completed_at: None,
        error: None,
    };
    storage
        .begin_journaled_operation(&operation)
        .expect("seed erasure journal");

    let partial_plan = MemoryErasurePlan {
        session_ids: plan.session_ids.clone(),
        archive_ids: Vec::new(),
        memory_unit_ids: Vec::new(),
        core_facts: Vec::new(),
        task_ids: Vec::new(),
        sleep_task_ids: Vec::new(),
        ..plan.clone()
    };
    storage
        .apply_memory_erasure_plan(&partial_plan)
        .expect("apply partial purge before simulated crash");
    assert!(storage.read_session("recover_chat").is_err());
    assert!(storage.read_archive_entry_by_id(&seed.archive_id).is_ok());

    let restarted = MemoryEngine::new(storage.clone());
    let core_path = root
        .join("core")
        .join("store")
        .join(format!("{}.json", plan.core_facts[0].category));
    let valid_core = fs::read(&core_path).expect("read valid Core category");
    fs::write(&core_path, b"{broken json").expect("corrupt Core category after planning");
    assert!(restarted.recover_pending_memory_operations().is_err());
    assert!(storage
        .load_journaled_operation("memory_erasure_recovery_fixture")
        .expect("journal after failed recovery")
        .is_some());
    fs::write(&core_path, valid_core).expect("restore Core category for retry");
    assert_eq!(
        restarted
            .recover_pending_memory_operations()
            .expect("recover pending purge"),
        1
    );
    assert!(storage.read_archive_entry_by_id(&seed.archive_id).is_err());
    assert!(storage
        .read_memory_unit_by_id(&seed.memory_unit_id)
        .is_err());
    assert!(storage
        .read_core_store_categories()
        .expect("core after recovery")
        .items
        .into_iter()
        .flat_map(|category| category.facts)
        .all(|fact| fact.scope.as_deref() != Some("recover_scope")));
    assert!(storage
        .load_journaled_operation("memory_erasure_recovery_fixture")
        .expect("journal after recovery")
        .is_none());
    fs::remove_dir_all(root).ok();
}

struct SeededMemory {
    archive_id: String,
    memory_unit_id: String,
}

fn seed_scope_memory(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    scope: &str,
    fact: &str,
) -> SeededMemory {
    let event_id = ingest_text(engine, session_id, scope, fact);
    let outcome = run_sleep(engine, session_id, &event_id, fact);
    assert_eq!(outcome.archive_entry.status, ArchiveStatus::Complete);
    assert_eq!(outcome.core_summary.created, 1);
    SeededMemory {
        archive_id: outcome.archive_entry.archive_id,
        memory_unit_id: outcome.archive_entry.memory_units[0].memory_unit_id.clone(),
    }
}

fn ingest_text(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    scope: &str,
    text: &str,
) -> String {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "erasure_test".to_string(),
            timestamp: "2026-08-16T09:00:00Z".to_string(),
            personality_scope_id: Some(scope.to_string()),
            session_id: session_id.to_string(),
            memory_text: None,
            payload: json!({ "text": text }),
            tags: vec!["personal".to_string()],
            theme: Some("profile".to_string()),
            emotional_tone: None,
            speaker: None,
            links: Vec::new(),
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest seed event")
        .stored_event
        .event_id
}

fn run_sleep(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    event_id: &str,
    fact: &str,
) -> SleepOutcome {
    let mut run = engine.begin_sleep_run(session_id).expect("begin sleep");
    let step = engine.next_sleep_batch(run).expect("next extraction");
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
                    "memory_units": [{
                        "thesis": fact,
                        "source_event_ids": [event_id],
                        "evidence": "Direct user statement.",
                        "tags": ["stable_fact"],
                        "weight": 0.5
                    }]
                }),
                "sleep_emotional_pass" => json!({ "emotional_markers": [] }),
                "sleep_topic_thread_pass" => json!({
                    "topic_thread": [{
                        "topic": "profile",
                        "summary": fact,
                        "source_event_ids": [event_id]
                    }]
                }),
                "sleep_personal_signal_pass" => json!({
                    "personal_signals": [{
                        "text": fact,
                        "category": "profile",
                        "confidence": 0.95,
                        "source_event_ids": [event_id],
                        "evidence": "Direct user statement."
                    }]
                }),
                "sleep_relational_pass" => json!({ "relational_tone": null }),
                other => panic!("unexpected sleep request: {other}"),
            };
            LlmResponse::Ok {
                request_id: request.request_id,
                text: text.to_string(),
            }
        })
        .collect();
    let step = engine
        .submit_sleep_batch(run, responses)
        .expect("submit extraction");
    let batch = step.batch.expect("consolidator batch");
    let step = engine
        .submit_sleep_batch(
            step.run,
            vec![LlmResponse::Ok {
                request_id: batch.requests[0].request_id.clone(),
                text: format!("GIST: {fact}\n\nThe user shared a stable personal fact."),
            }],
        )
        .expect("submit consolidator");
    engine.finish_sleep_run(step.run).expect("finish sleep")
}

fn seed_candidate(storage: &FileStorage, scope: &str, session_id: &str, memory: &SeededMemory) {
    storage
        .write_candidate_belief(&CandidateBelief {
            schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            candidate_id: "candidate_target_scope".to_string(),
            source_session_id: Some(session_id.to_string()),
            source_session_ids: vec![session_id.to_string()],
            core_scope: Some(scope.to_string()),
            subject: None,
            created_at: "2026-08-16T09:30:00Z".to_string(),
            updated_at: "2026-08-16T09:30:00Z".to_string(),
            text: "Alice values stable memory.".to_string(),
            category: "preference".to_string(),
            status: CandidateStatus::ReadyForReview,
            confidence: 0.8,
            supporting_archive_ids: vec![memory.archive_id.clone()],
            contradicting_archive_ids: Vec::new(),
            contradicted_core_fact_ids: Vec::new(),
            evidence_summary: "A stable statement.".to_string(),
            promotion_checks: PromotionChecks {
                min_sources_met: true,
                weight_threshold_met: true,
                no_recent_contradiction: true,
                manual_review_required: true,
            },
            source_memory_unit_ids: vec![memory.memory_unit_id.clone()],
            tags: vec!["reflection".to_string()],
            promoted_core_fact_id: None,
            review: None,
            links: Vec::new(),
            reinforcements: Vec::new(),
        })
        .expect("seed candidate");
}

fn core_context_request(session_id: &str, scope: &str) -> CoreContextRequest {
    CoreContextRequest {
        schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
        session_id: session_id.to_string(),
        domain_state: json!({}),
        core_scope: Some(scope.to_string()),
        query_text: Some("Where does the person live?".to_string()),
        query_embedding: None,
        explain_recall: false,
        recall_limit: 10,
        session_recent_limit: 0,
        session_trace_event_limit: 0,
        include_core: true,
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
