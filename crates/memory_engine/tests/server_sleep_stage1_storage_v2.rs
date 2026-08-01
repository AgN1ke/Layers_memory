use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::thread;

use memory_engine::archive::{ArchiveEntry, ArchiveStatus};
use memory_engine::event::IngestEvent;
use memory_engine::llm::{SleepRun, SleepRunStage, SleepTrack};
use memory_engine::server_runtime::{
    AccountVaultMembership, AuthenticatedPrincipal, MembershipStatus, PrincipalKind, VaultContext,
    ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION, AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION,
};
use memory_engine::session::{SessionMetadata, SessionStatus};
use memory_engine::storage_v2::{RecordKey, TransactionalStorage};
use memory_engine::tasks::{PendingTask, TaskState, TaskType};
use memory_engine::types::{ImportanceHint, ProcessingMode, EVENT_SCHEMA_VERSION};
use memory_engine::{
    execute_work_unit_with_retry, EngineOptions, ExpectedRevision, InMemoryCommitFault,
    InMemoryTransactionalStorage, MemoryEngine, StorageWorkUnit, TransactionalIngestRequest,
    TransactionalSleepStartRequest, WorkUnitCommit, WorkUnitIntent,
};
use serde_json::json;

const T0: &str = "2026-08-01T12:00:00Z";
const T1: &str = "2026-08-01T12:00:01Z";
const T2: &str = "2026-08-01T12:00:02Z";

#[test]
fn transactional_sleep_start_commits_one_coherent_workflow_and_replays_exactly() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &context,
        "ingest_one",
        "session_owner",
        T0,
        "one",
        false,
    );
    ingest(
        &engine,
        &context,
        "ingest_two",
        "session_owner",
        T1,
        "two",
        true,
    );
    let legacy_shaped_session =
        read_record(&storage, &context, "sessions", "session_owner").payload;
    assert!(legacy_shaped_session.get("active_sleep_task_id").is_none());
    assert!(legacy_shaped_session
        .get("sleep_pending_event_ids")
        .is_none());
    let compatible_session: SessionMetadata =
        serde_json::from_value(legacy_shaped_session).unwrap();
    assert!(compatible_session.active_sleep_task_id.is_none());
    assert!(compatible_session.sleep_pending_event_ids.is_empty());
    let request = sleep_request(&engine, "sleep_start_1", "session_owner");
    assert_eq!(
        request.schema_version,
        "transactional_sleep_start_request.v1"
    );
    assert_eq!(
        request.work_unit.schema_version,
        "storage_work_unit_intent.v1"
    );
    storage
        .inject_commit_fault_once("sleep_start_1", InMemoryCommitFault::AfterPublishBeforeAck)
        .unwrap();

    let first = engine
        .begin_sleep_run_transactional(&context, request.clone())
        .unwrap();
    assert_eq!(first.schema_version, "transactional_sleep_start_result.v1");
    assert_eq!(first.receipt.schema_version, "storage_work_unit_receipt.v1");
    assert_eq!(first.receipt.mutation_count, 6);
    assert_eq!(
        first.receipt.result_reference,
        "operation_results:sleep_start_1"
    );
    assert_eq!(first.receipt.vault_id.as_str(), "vault_alpha");
    assert_eq!(first.receipt.account_id.as_str(), "account_alpha");
    assert_eq!(first.run.stage, SleepRunStage::Extraction);
    assert_eq!(first.run.requests.len(), 5);
    assert_eq!(
        first
            .run
            .requests
            .iter()
            .map(|state| state.request.request_id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        5
    );
    let archive = read_archive(&storage, &context, &first.run.archive_id);
    let sleep_task = read_task(&storage, &context, &first.run.sleep_task_id);
    let memory_task = read_task(
        &storage,
        &context,
        first.run.memory_unit_task_id.as_deref().unwrap(),
    );
    let session = read_session(&storage, &context, "session_owner");
    let immutable_result = read_record(&storage, &context, "operation_results", "sleep_start_1");

    assert_eq!(
        immutable_result.payload_schema_version,
        "sleep_start_operation_result.v1"
    );
    assert_eq!(
        immutable_result.payload["schema_version"],
        "sleep_start_operation_result.v1"
    );
    assert_eq!(
        immutable_result.payload["run"]["archive_id"],
        archive.archive_id
    );
    assert_eq!(archive.status, ArchiveStatus::Preliminary);
    assert_eq!(first.run.archive_id, archive.archive_id);
    assert_eq!(first.run.sleep_task_id, sleep_task.task_id);
    assert_eq!(
        first.run.memory_unit_task_id.as_deref(),
        Some(memory_task.task_id.as_str())
    );
    for state in &first.run.requests {
        let expected_task_id = if state.track == SleepTrack::MemoryUnit {
            &memory_task.task_id
        } else {
            &sleep_task.task_id
        };
        assert_eq!(&state.request.task_id, expected_task_id);
    }
    assert_eq!(sleep_task.task_type, TaskType::SleepCompression);
    assert_eq!(memory_task.task_type, TaskType::MemoryUnitPass);
    assert_eq!(sleep_task.state, TaskState::Pending);
    assert_eq!(memory_task.state, TaskState::Pending);
    assert_eq!(session.event_count, 2);
    assert_eq!(session.status, SessionStatus::SleepPending);
    assert_eq!(
        session.active_sleep_task_id.as_deref(),
        Some(first.run.sleep_task_id.as_str())
    );
    assert_eq!(session.sleep_pending_event_ids, archive.source_event_ids);
    assert!(session.archived_to.is_empty());
    assert!(session.archived_event_ids.is_empty());
    assert_eq!(task_event_ids(&sleep_task), archive.source_event_ids);
    assert_eq!(task_event_ids(&memory_task), archive.source_event_ids);
    assert_eq!(sleep_task.inputs["session_id"], "session_owner");
    assert_eq!(memory_task.inputs["session_id"], "session_owner");
    assert_eq!(sleep_task.inputs["events"], memory_task.inputs["events"]);
    let frozen_event = &sleep_task.inputs["events"][0];
    assert_eq!(frozen_event["schema_version"], "sleep_input_event.v1");
    assert_eq!(frozen_event["source"], "test");
    assert_eq!(frozen_event["received_at"], T1);
    assert_eq!(frozen_event["session_id"], "session_owner");
    assert_eq!(frozen_event["memory_text"], "two");
    assert_eq!(frozen_event["emotional_tone"], "neutral");
    assert_eq!(frozen_event["importance_hint"], "high");
    assert_eq!(frozen_event["processing_mode"], "defer_to_sleep");
    assert_eq!(
        sleep_task.inputs["preliminary_archive_id"],
        archive.archive_id
    );

    ingest(
        &engine,
        &context,
        "ingest_after_start",
        "session_owner",
        T2,
        "later",
        false,
    );
    advance_live_run_for_replay_test(&storage, &context, &first.run);
    let replay = engine
        .begin_sleep_run_transactional(&context, request)
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        read_run(&storage, &context, &first.run.sleep_task_id)
            .completion_mode
            .as_deref(),
        Some("advanced-for-replay-test")
    );
    assert_eq!(
        read_session(&storage, &context, "session_owner").event_count,
        3
    );
    assert_eq!(
        read_record(&storage, &context, "archives", &archive.archive_id).revision,
        1
    );
    assert_eq!(
        read_record(&storage, &context, "tasks", &sleep_task.task_id).revision,
        1
    );
}

#[test]
fn transactional_sleep_start_uses_shared_selection_policy_and_preserves_active_tail() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let mut options = EngineOptions::default();
    options.sleep.active_tail_ratio = 0.30;
    options.sleep.partial_sleep_min_events = 10;
    options.sleep.max_events = 3;
    options.sleep.min_event_weight = 0.55;
    let engine = MemoryEngine::with_options(storage.clone(), options);
    for index in 0..10 {
        ingest(
            &engine,
            &context,
            &format!("ingest_{index}"),
            "session_owner",
            &format!("2026-08-01T12:00:{index:02}Z"),
            &format!("event-{index}"),
            matches!(index, 1 | 3 | 5 | 8 | 9),
        );
    }

    let result = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_selection", "session_owner"),
        )
        .unwrap();
    let archive = read_archive(&storage, &context, &result.run.archive_id);
    let task = read_task(&storage, &context, &result.run.sleep_task_id);
    let selected_text = task.inputs["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["payload"]["text"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(selected_text, vec!["event-1", "event-3", "event-5"]);
    assert_eq!(archive.source_event_ids.len(), 3);
    assert!(!selected_text.contains(&"event-8".to_string()));
    assert!(!selected_text.contains(&"event-9".to_string()));
}

#[test]
fn sleep_selection_and_active_tail_use_receive_order_not_untrusted_client_time() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let mut options = EngineOptions::default();
    options.sleep.active_tail_ratio = 0.20;
    options.sleep.partial_sleep_min_events = 10;
    options.sleep.max_events = 3;
    options.sleep.min_event_weight = 0.55;
    let engine = MemoryEngine::with_options(storage.clone(), options);
    for index in 0..10 {
        let client_timestamp = if index >= 8 {
            format!("2020-01-01T00:00:{index:02}Z")
        } else {
            format!("2026-08-01T12:00:{:02}Z", 9 - index)
        };
        ingest_with_times(
            &engine,
            &context,
            &format!("ingest_receive_order_{index}"),
            "session_owner",
            &client_timestamp,
            &format!("2026-08-01T13:00:{index:02}Z"),
            &format!("event-{index}"),
            index >= 8,
        );
    }

    let result = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_receive_order", "session_owner"),
        )
        .unwrap();
    let task = read_task(&storage, &context, &result.run.sleep_task_id);
    let selected_text = task.inputs["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["payload"]["text"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(selected_text, vec!["event-5", "event-6", "event-7"]);
}

#[test]
fn concurrent_sleep_starts_reserve_one_workflow_per_session() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = Arc::new(MemoryEngine::new(storage.clone()));
    ingest(
        &engine,
        &context,
        "ingest_one",
        "session_owner",
        T0,
        "one",
        false,
    );
    ingest(
        &engine,
        &context,
        "ingest_two",
        "session_owner",
        T1,
        "two",
        false,
    );
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for operation_id in ["sleep_parallel_1", "sleep_parallel_2"] {
        let engine = Arc::clone(&engine);
        let context = context.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let request = sleep_request(&engine, operation_id, "session_owner");
            barrier.wait();
            engine.begin_sleep_run_transactional(&context, request)
        }));
    }
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results.into_iter().find_map(Result::err).unwrap();
    assert!(error.to_string().contains("already has active sleep task"));
    assert_eq!(
        storage.record_count(&context).unwrap(),
        8,
        "session + 2 events + five new workflow records; Session is updated in place"
    );
}

#[test]
fn concurrent_ingest_and_sleep_keep_the_new_event_and_session_revision() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = Arc::new(MemoryEngine::new(storage.clone()));
    ingest(
        &engine,
        &context,
        "ingest_one",
        "session_owner",
        T0,
        "one",
        false,
    );
    let barrier = Arc::new(Barrier::new(3));

    let sleep_engine = Arc::clone(&engine);
    let sleep_context = context.clone();
    let sleep_barrier = Arc::clone(&barrier);
    let sleep = thread::spawn(move || {
        let request = sleep_request(&sleep_engine, "sleep_with_ingest", "session_owner");
        sleep_barrier.wait();
        sleep_engine
            .begin_sleep_run_transactional(&sleep_context, request)
            .unwrap()
    });
    let ingest_engine = Arc::clone(&engine);
    let ingest_context = context.clone();
    let ingest_barrier = Arc::clone(&barrier);
    let ingest_handle = thread::spawn(move || {
        ingest_barrier.wait();
        ingest(
            &ingest_engine,
            &ingest_context,
            "ingest_concurrent",
            "session_owner",
            T1,
            "two",
            false,
        )
    });
    barrier.wait();
    let run = sleep.join().unwrap();
    let concurrent_event_id = ingest_handle.join().unwrap();

    let session_record = read_record(&storage, &context, "sessions", "session_owner");
    let session: SessionMetadata = serde_json::from_value(session_record.payload).unwrap();
    assert_eq!(session.event_count, 2);
    assert_eq!(session_record.revision, 3);
    let archive = read_archive(&storage, &context, &run.run.archive_id);
    assert!(read_record(&storage, &context, "session_events", &concurrent_event_id).revision >= 1);
    assert_eq!(session.sleep_pending_event_ids, archive.source_event_ids);
    assert!(archive.source_event_ids.len() == 1 || archive.source_event_ids.len() == 2);
    if !archive.source_event_ids.contains(&concurrent_event_id) {
        assert!(!session
            .sleep_pending_event_ids
            .contains(&concurrent_event_id));
        assert!(!session.archived_event_ids.contains(&concurrent_event_id));
    }
}

#[test]
fn transactional_sleep_start_rolls_back_at_every_logical_edge() {
    let phases = [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterMutation(3),
        InMemoryCommitFault::AfterMutation(4),
        InMemoryCommitFault::AfterMutation(5),
        InMemoryCommitFault::AfterMutation(6),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let context = context("account_alpha", &format!("vault_fault_{index}"));
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        ingest(
            &engine,
            &context,
            "ingest_one",
            "session_owner",
            T0,
            "one",
            false,
        );
        let operation_id = format!("sleep_fault_{index}");
        let request = sleep_request(&engine, &operation_id, "session_owner");
        let before = read_record(&storage, &context, "sessions", "session_owner");
        storage
            .inject_commit_fault_once(&operation_id, phase)
            .unwrap();

        let error = engine
            .begin_sleep_run_transactional(&context, request.clone())
            .expect_err("pre-publish failure must roll back the whole workflow");
        assert!(error.to_string().contains(&format!("{phase:?}")));
        let after = read_record(&storage, &context, "sessions", "session_owner");
        assert_eq!(after, before);
        assert_eq!(storage.record_count(&context).unwrap(), 2);
        assert!(storage
            .committed_receipt(&context, &operation_id)
            .unwrap()
            .is_none());

        let success = engine
            .begin_sleep_run_transactional(&context, request)
            .unwrap();
        assert_eq!(success.receipt.mutation_count, 6);
        assert_eq!(storage.record_count(&context).unwrap(), 7);
    }
}

#[test]
fn transactional_sleep_start_rejects_frozen_input_tamper_without_writes() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &context,
        "ingest_one",
        "session_owner",
        T0,
        "one",
        false,
    );
    let mut request = sleep_request(&engine, "sleep_tamper", "session_owner");
    request.policy.max_events += 1;

    let error = engine
        .begin_sleep_run_transactional(&context, request)
        .expect_err("changed frozen policy must fail before mutation");
    assert!(error.to_string().contains("request_digest"));
    assert_eq!(storage.record_count(&context).unwrap(), 2);
    assert_eq!(
        read_session(&storage, &context, "session_owner").status,
        SessionStatus::Active
    );
}

#[test]
fn incomplete_archive_coverage_index_fails_closed_before_reservation() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &context,
        "ingest_one",
        "session_owner",
        T0,
        "one",
        false,
    );
    mark_coverage_index_incomplete(&storage, &context, "session_owner");

    let error = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_needs_repair", "session_owner"),
        )
        .expect_err("unknown Complete coverage must not be treated as empty coverage");
    assert!(error.to_string().contains("index repair"));
    let session = read_session(&storage, &context, "session_owner");
    assert_eq!(session.status, SessionStatus::Active);
    assert!(session.active_sleep_task_id.is_none());
    assert!(session.sleep_pending_event_ids.is_empty());
    assert!(storage
        .committed_receipt(&context, "sleep_needs_repair")
        .unwrap()
        .is_none());
}

#[test]
fn rewritten_event_record_fails_closed_before_sleep_start() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let event_id = ingest(
        &engine,
        &context,
        "ingest_one",
        "session_owner",
        T0,
        "one",
        false,
    );
    rewrite_event_at_revision_two(&storage, &context, &event_id);

    let error = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_rewritten_event", "session_owner"),
        )
        .expect_err("append-only source mutation must not enter frozen sleep input");
    assert!(error.to_string().contains("must remain at revision 1"));
    let session = read_session(&storage, &context, "session_owner");
    assert_eq!(session.status, SessionStatus::Active);
    assert!(session.active_sleep_task_id.is_none());
}

#[test]
fn another_sessions_rewritten_event_does_not_block_target_sleep() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &context,
        "ingest_target",
        "session_target",
        T0,
        "target",
        false,
    );
    let foreign_event_id = ingest(
        &engine,
        &context,
        "ingest_foreign",
        "session_foreign",
        T0,
        "foreign",
        false,
    );
    rewrite_event_at_revision_two(&storage, &context, &foreign_event_id);

    let result = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_target_only", "session_target"),
        )
        .expect("typed session query must isolate unrelated corrupt records");

    let archive = read_archive(&storage, &context, &result.run.archive_id);
    assert_eq!(archive.source_session_id, "session_target");
    assert_eq!(archive.source_event_ids.len(), 1);
}

#[test]
fn sleep_operation_id_cannot_be_reused_for_another_session() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &context,
        "ingest_one",
        "session_one",
        T0,
        "one",
        false,
    );
    ingest(
        &engine,
        &context,
        "ingest_two",
        "session_two",
        T0,
        "two",
        false,
    );
    engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_reused", "session_one"),
        )
        .unwrap();

    let error = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_reused", "session_two"),
        )
        .expect_err("operation id must identify exactly one frozen input");
    assert!(
        error.to_string().contains("different input"),
        "unexpected error: {error}"
    );
    assert_eq!(
        read_session(&storage, &context, "session_two").status,
        SessionStatus::Active
    );
}

#[test]
fn transactional_sleep_start_is_vault_isolated() {
    let alpha = context("account_alpha", "vault_alpha");
    let beta = context("account_beta", "vault_beta");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &alpha,
        "ingest_same",
        "session_same",
        T0,
        "alpha",
        false,
    );
    ingest(
        &engine,
        &beta,
        "ingest_same",
        "session_same",
        T0,
        "beta",
        false,
    );

    let alpha_result = engine
        .begin_sleep_run_transactional(&alpha, sleep_request(&engine, "sleep_same", "session_same"))
        .unwrap();
    let beta_result = engine
        .begin_sleep_run_transactional(&beta, sleep_request(&engine, "sleep_same", "session_same"))
        .unwrap();
    assert_ne!(alpha_result.run.archive_id, beta_result.run.archive_id);
    assert_eq!(
        read_archive(&storage, &alpha, &alpha_result.run.archive_id).gist,
        "Попередній спогад: alpha."
    );
    assert_eq!(
        read_archive(&storage, &beta, &beta_result.run.archive_id).gist,
        "Попередній спогад: beta."
    );
}

fn sleep_request<S>(
    engine: &MemoryEngine<S>,
    operation_id: &str,
    session_id: &str,
) -> TransactionalSleepStartRequest {
    engine
        .new_transactional_sleep_start_request(operation_id, T1, T2, session_id)
        .unwrap()
}

fn ingest<S: memory_engine::TransactionalStorage>(
    engine: &MemoryEngine<S>,
    context: &VaultContext,
    operation_id: &str,
    session_id: &str,
    timestamp: &str,
    text: &str,
    high: bool,
) -> String {
    ingest_with_times(
        engine,
        context,
        operation_id,
        session_id,
        timestamp,
        timestamp,
        text,
        high,
    )
}

#[allow(clippy::too_many_arguments)]
fn ingest_with_times<S: memory_engine::TransactionalStorage>(
    engine: &MemoryEngine<S>,
    context: &VaultContext,
    operation_id: &str,
    session_id: &str,
    event_timestamp: &str,
    received_at: &str,
    text: &str,
    high: bool,
) -> String {
    let event = IngestEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_string(),
        event_type: "user_message".to_string(),
        source: "test".to_string(),
        timestamp: event_timestamp.to_string(),
        personality_scope_id: None,
        session_id: session_id.to_string(),
        memory_text: Some(text.to_string()),
        payload: json!({"text": text}),
        tags: vec![],
        theme: None,
        emotional_tone: Some("neutral".to_string()),
        speaker: None,
        links: vec![],
        importance_hint: if high {
            ImportanceHint::High
        } else {
            ImportanceHint::Low
        },
        processing_mode: ProcessingMode::default(),
    };
    let request =
        TransactionalIngestRequest::new(operation_id, "test_host", received_at, received_at, event)
            .unwrap();
    engine
        .ingest_transactional(context, request)
        .unwrap()
        .ingest
        .stored_event
        .event_id
}

fn task_event_ids(task: &PendingTask) -> Vec<String> {
    task.inputs["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["event_id"].as_str().unwrap().to_string())
        .collect()
}

fn read_archive(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    archive_id: &str,
) -> ArchiveEntry {
    serde_json::from_value(read_record(storage, context, "archives", archive_id).payload).unwrap()
}

fn read_task(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    task_id: &str,
) -> PendingTask {
    serde_json::from_value(read_record(storage, context, "tasks", task_id).payload).unwrap()
}

fn read_session(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    session_id: &str,
) -> SessionMetadata {
    serde_json::from_value(read_record(storage, context, "sessions", session_id).payload).unwrap()
}

fn read_record(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    collection: &str,
    record_id: &str,
) -> memory_engine::VaultRecord {
    storage
        .read_record(context, &RecordKey::new(collection, record_id).unwrap())
        .unwrap()
        .unwrap()
}

fn context(account_id: &str, vault_id: &str) -> VaultContext {
    let principal = AuthenticatedPrincipal {
        schema_version: AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION.to_string(),
        account_id: memory_engine::AccountId::parse(account_id).unwrap(),
        authentication_id: format!("auth_{account_id}"),
        kind: PrincipalKind::Guest,
        authenticated_at: T0.to_string(),
    };
    let membership = AccountVaultMembership {
        schema_version: ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION.to_string(),
        membership_id: format!("membership_{account_id}"),
        account_id: principal.account_id.clone(),
        vault_id: memory_engine::VaultId::parse(vault_id).unwrap(),
        status: MembershipStatus::Active,
        created_at: T0.to_string(),
        revision: 1,
    };
    VaultContext::authorize(Some(&principal), Some(&membership)).unwrap()
}

fn read_run(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    task_id: &str,
) -> SleepRun {
    serde_json::from_value(read_record(storage, context, "sleep_runs", task_id).payload).unwrap()
}

fn advance_live_run_for_replay_test(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    initial: &SleepRun,
) {
    let key = RecordKey::new("sleep_runs", initial.sleep_task_id.clone()).unwrap();
    let intent = WorkUnitIntent {
        schema_version: "storage_work_unit_intent.v1".to_string(),
        operation_id: "advance_sleep_run_for_replay_test".to_string(),
        operation_kind: "test_sleep_advance".to_string(),
        request_digest: format!("sha256:{}", "1".repeat(64)),
        started_at: T2.to_string(),
    };
    execute_work_unit_with_retry(storage, context, intent, 1, |work_unit| {
        let record = work_unit.get(&key)?.unwrap();
        let mut run: SleepRun = serde_json::from_value(record.payload).unwrap();
        run.completion_mode = Some("advanced-for-replay-test".to_string());
        work_unit.put(
            key.clone(),
            ExpectedRevision::Exact(record.revision),
            "sleep_run.v1".to_string(),
            serde_json::to_value(&run).unwrap(),
        )?;
        Ok(WorkUnitCommit {
            committed_at: T2.to_string(),
            result_reference: key.reference(),
            result_digest: format!("sha256:{}", "2".repeat(64)),
        })
    })
    .unwrap();
}

fn mark_coverage_index_incomplete(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    session_id: &str,
) {
    let key = RecordKey::new("sessions", session_id).unwrap();
    let intent = WorkUnitIntent {
        schema_version: "storage_work_unit_intent.v1".to_string(),
        operation_id: "mark_coverage_incomplete_for_test".to_string(),
        operation_kind: "test_session_rewrite".to_string(),
        request_digest: format!("sha256:{}", "3".repeat(64)),
        started_at: T1.to_string(),
    };
    execute_work_unit_with_retry(storage, context, intent, 1, |work_unit| {
        let record = work_unit.get(&key)?.unwrap();
        let mut session: SessionMetadata = serde_json::from_value(record.payload).unwrap();
        session.archived_event_index_complete = false;
        work_unit.put(
            key.clone(),
            ExpectedRevision::Exact(record.revision),
            "session.v1".to_string(),
            serde_json::to_value(&session).unwrap(),
        )?;
        Ok(WorkUnitCommit {
            committed_at: T1.to_string(),
            result_reference: key.reference(),
            result_digest: format!("sha256:{}", "4".repeat(64)),
        })
    })
    .unwrap();
}

fn rewrite_event_at_revision_two(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    event_id: &str,
) {
    let key = RecordKey::new("session_events", event_id).unwrap();
    let intent = WorkUnitIntent {
        schema_version: "storage_work_unit_intent.v1".to_string(),
        operation_id: "rewrite_event_for_test".to_string(),
        operation_kind: "test_illegal_event_rewrite".to_string(),
        request_digest: format!("sha256:{}", "5".repeat(64)),
        started_at: T1.to_string(),
    };
    execute_work_unit_with_retry(storage, context, intent, 1, |work_unit| {
        let record = work_unit.get(&key)?.unwrap();
        work_unit.put(
            key.clone(),
            ExpectedRevision::Exact(record.revision),
            record.payload_schema_version,
            record.payload,
        )?;
        Ok(WorkUnitCommit {
            committed_at: T1.to_string(),
            result_reference: key.reference(),
            result_digest: format!("sha256:{}", "6".repeat(64)),
        })
    })
    .unwrap();
    assert_eq!(
        read_record(storage, context, "session_events", event_id).revision,
        2
    );
}
