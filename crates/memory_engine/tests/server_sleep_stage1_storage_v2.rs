use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::thread;

use memory_engine::archive::{ArchiveEntry, ArchiveStatus};
use memory_engine::event::IngestEvent;
use memory_engine::llm::{LlmResponse, SleepRun, SleepRunStage, SleepTrack};
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
    TransactionalSleepCancelRequest, TransactionalSleepFinalizeRequest,
    TransactionalSleepStartRequest, TransactionalSleepSubmitRequest, WorkUnitCommit,
    WorkUnitIntent, MAX_SLEEP_SUBMIT_RESPONSE_BYTES, MAX_SLEEP_WORKFLOW_INPUT_BYTES,
};
use serde_json::json;

const T0: &str = "2026-08-01T12:00:00Z";
const T1: &str = "2026-08-01T12:00:01Z";
const T2: &str = "2026-08-01T12:00:02Z";
const T3: &str = "2026-08-01T12:00:03Z";
const T4: &str = "2026-08-01T12:00:04Z";
const T5: &str = "2026-08-01T12:00:05Z";

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
    assert_eq!(first.receipt.mutation_count, 7);
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
    let immutable_input = read_record(&storage, &context, "sleep_inputs", &first.run.sleep_task_id);

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
    assert!(!serde_json::to_string(&immutable_result.payload)
        .unwrap()
        .contains("two"));
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
    assert_eq!(
        immutable_input.payload_schema_version,
        "sleep_workflow_input.v1"
    );
    assert_eq!(immutable_input.revision, 1);
    assert_eq!(immutable_input.payload["session_id"], "session_owner");
    assert_eq!(immutable_input.payload["archive_id"], archive.archive_id);
    assert!(sleep_task.inputs.get("events").is_none());
    assert!(memory_task.inputs.get("events").is_none());
    assert_eq!(
        sleep_task.inputs["sleep_input_ref"],
        memory_task.inputs["sleep_input_ref"]
    );
    assert_eq!(
        first.run.input_reference.as_ref().unwrap().record_id,
        first.run.sleep_task_id
    );
    let first_batch = first.batch.as_ref().expect("hydrated extraction batch");
    assert_eq!(first_batch.requests.len(), 5);
    let frozen_event = &first_batch.requests[0].prompt_inputs["sleep_task"]["events"][0];
    assert_eq!(frozen_event["schema_version"], "sleep_input_event.v1");
    assert_eq!(frozen_event["source"], "test");
    assert_eq!(frozen_event["received_at"], T1);
    assert_eq!(frozen_event["session_id"], "session_owner");
    assert_eq!(frozen_event["memory_text"], "two");
    assert_eq!(frozen_event["emotional_tone"], "neutral");
    assert_eq!(frozen_event["importance_hint"], "high");
    assert_eq!(frozen_event["processing_mode"], "defer_to_sleep");
    assert_eq!(
        first_batch.requests[0].prompt_inputs["sleep_task"]["preliminary_archive_id"],
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
    let selected_text = result.batch.unwrap().requests[0].prompt_inputs["sleep_task"]["events"]
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
    let selected_text = result.batch.unwrap().requests[0].prompt_inputs["sleep_task"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["payload"]["text"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(selected_text, vec!["event-5", "event-6", "event-7"]);
}

#[test]
fn sleep_uses_server_admission_sequence_when_receive_times_are_equal() {
    let context = context("account_alpha", "vault_sequence_order");
    let storage = InMemoryTransactionalStorage::new();
    let mut options = EngineOptions::default();
    options.sleep.max_events = 1;
    options.sleep.active_tail_ratio = 0.0;
    let engine = MemoryEngine::with_options(storage.clone(), options);
    let first = ingest_with_times(
        &engine,
        &context,
        "ingest_sequence_first",
        "session_owner",
        "2030-01-01T00:00:00Z",
        T0,
        "first admitted",
        false,
    );
    let second = ingest_with_times(
        &engine,
        &context,
        "ingest_sequence_second",
        "session_owner",
        "2020-01-01T00:00:00Z",
        T0,
        "second admitted",
        false,
    );
    assert_eq!(
        read_record(&storage, &context, "session_events", &first).payload["admission_sequence"],
        1
    );
    assert_eq!(
        read_record(&storage, &context, "session_events", &second).payload["admission_sequence"],
        2
    );
    let result = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_sequence_order", "session_owner"),
        )
        .unwrap();
    assert_eq!(
        read_archive(&storage, &context, &result.run.archive_id).source_event_ids,
        vec![second],
        "equal receive times must follow server admission, not client time or hash ID"
    );
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
        9,
        "session + 2 events + six new workflow records; Session is updated in place"
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
        InMemoryCommitFault::AfterMutation(7),
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
        assert_eq!(success.receipt.mutation_count, 7);
        assert_eq!(storage.record_count(&context).unwrap(), 8);
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

#[test]
fn transactional_sleep_lifecycle_finishes_once_and_publishes_canonical_memory() {
    let context = context("account_alpha", "vault_lifecycle");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let event_id = ingest(
        &engine,
        &context,
        "ingest_lifecycle",
        "session_owner",
        T0,
        "I love exploring Mars.",
        true,
    );
    let start = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_lifecycle", "session_owner"),
        )
        .unwrap();
    let input = read_record(&storage, &context, "sleep_inputs", &start.run.sleep_task_id);
    assert_eq!(input.revision, 1);
    assert_eq!(input.payload["events"][0]["event_id"], event_id);
    for task_id in [
        start.run.sleep_task_id.as_str(),
        start.run.memory_unit_task_id.as_deref().unwrap(),
    ] {
        let task = read_task(&storage, &context, task_id);
        assert!(task.inputs.get("events").is_none());
        assert_eq!(
            task.inputs["sleep_input_ref"]["record_id"],
            start.run.sleep_task_id
        );
    }

    let extraction = start.batch.clone().expect("extraction batch");
    let submit_extract = TransactionalSleepSubmitRequest::new(
        "sleep_submit_extract",
        T2,
        T2,
        &start.run,
        extraction_responses(&start.run, extraction, &event_id),
    )
    .unwrap();
    assert_eq!(
        submit_extract.schema_version,
        "transactional_sleep_submit_request.v1"
    );
    storage
        .inject_commit_fault_once(
            "sleep_submit_extract",
            InMemoryCommitFault::AfterPublishBeforeAck,
        )
        .unwrap();
    let extracted = engine
        .submit_sleep_batch_transactional(&context, submit_extract.clone())
        .unwrap();
    assert_eq!(
        extracted.schema_version,
        "transactional_sleep_submit_result.v1"
    );
    assert_eq!(extracted.run.stage, SleepRunStage::Consolidation);
    assert_eq!(extracted.batch.as_ref().unwrap().requests.len(), 1);
    let replay = engine
        .submit_sleep_batch_transactional(&context, submit_extract)
        .unwrap();
    assert_eq!(replay, extracted);

    let consolidator = extracted.batch.clone().unwrap().requests.remove(0);
    let submit_consolidator = TransactionalSleepSubmitRequest::new(
        "sleep_submit_consolidator",
        T3,
        T3,
        &extracted.run,
        vec![LlmResponse::Ok {
            request_id: consolidator.request_id,
            text: "GIST: The user loves exploring Mars.\n\nThe user expressed a durable interest in exploring Mars."
                .to_string(),
        }],
    )
    .unwrap();
    let ready = engine
        .submit_sleep_batch_transactional(&context, submit_consolidator)
        .unwrap();
    assert_eq!(ready.run.stage, SleepRunStage::ReadyToFinish);
    assert!(ready.batch.is_none());

    let finalize_request =
        TransactionalSleepFinalizeRequest::new("sleep_finalize", T4, T4, &ready.run).unwrap();
    storage
        .inject_commit_fault_once("sleep_finalize", InMemoryCommitFault::AfterPublishBeforeAck)
        .unwrap();
    let finished = engine
        .finalize_sleep_run_transactional(&context, finalize_request.clone())
        .unwrap();
    assert_eq!(
        finished.schema_version,
        "transactional_sleep_finalize_result.v1"
    );
    assert_eq!(
        finished.post_finalize.schema_version,
        "transactional_sleep_post_finalize_state.v1"
    );
    assert_eq!(finished.run.stage, SleepRunStage::Finished);
    assert_eq!(
        finished.outcome.archive_entry.status,
        ArchiveStatus::Complete
    );
    assert_eq!(finished.outcome.archive_entry.memory_units.len(), 1);
    assert!(finished.post_finalize.core_bridge_pending);
    assert!(finished.post_finalize.fidelity_routing_pending);
    assert!(finished.post_finalize.embedding_backfill_pending);
    let unit = &finished.outcome.archive_entry.memory_units[0];
    assert_eq!(unit.thesis, "The user loves exploring Mars.");
    assert_eq!(unit.source_event_ids, vec![event_id.clone()]);
    assert_eq!(
        read_record(&storage, &context, "memory_units", &unit.memory_unit_id).payload["thesis"],
        unit.thesis
    );
    let session = read_session(&storage, &context, "session_owner");
    assert_eq!(session.status, SessionStatus::Active);
    assert!(session.active_sleep_task_id.is_none());
    assert!(session.sleep_pending_event_ids.is_empty());
    assert_eq!(session.archived_event_ids, vec![event_id]);
    assert_eq!(session.archived_to, vec![finished.run.archive_id.clone()]);
    assert_eq!(
        read_task(&storage, &context, &finished.run.sleep_task_id).state,
        TaskState::Completed
    );
    assert_eq!(
        read_task(
            &storage,
            &context,
            finished.run.memory_unit_task_id.as_deref().unwrap()
        )
        .state,
        TaskState::Completed
    );
    assert_eq!(
        read_record(
            &storage,
            &context,
            "sleep_inputs",
            &finished.run.sleep_task_id
        )
        .revision,
        1,
        "workflow input remains immutable through finalize"
    );
    let replay = engine
        .finalize_sleep_run_transactional(&context, finalize_request)
        .unwrap();
    assert_eq!(replay, finished);
}

#[test]
fn transactional_sleep_submit_rejects_stale_run_and_foreign_response_without_writes() {
    let context = context("account_alpha", "vault_submit_guard");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let event_id = ingest(
        &engine,
        &context,
        "ingest_submit_guard",
        "session_owner",
        T0,
        "Mars matters.",
        false,
    );
    let start = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_submit_guard", "session_owner"),
        )
        .unwrap();
    let before_run = read_record(&storage, &context, "sleep_runs", &start.run.sleep_task_id);
    let foreign = TransactionalSleepSubmitRequest::new(
        "submit_foreign",
        T2,
        T2,
        &start.run,
        vec![LlmResponse::Ok {
            request_id: "llm_req_foreign".to_string(),
            text: "{}".to_string(),
        }],
    )
    .unwrap();
    let error = engine
        .submit_sleep_batch_transactional(&context, foreign)
        .expect_err("foreign provider response must fail closed");
    assert!(error.to_string().contains("does not match"));
    assert_eq!(
        read_record(&storage, &context, "sleep_runs", &start.run.sleep_task_id),
        before_run
    );

    let batch = start.batch.clone().unwrap();
    let request = TransactionalSleepSubmitRequest::new(
        "submit_once",
        T2,
        T2,
        &start.run,
        extraction_responses(&start.run, batch, &event_id),
    )
    .unwrap();
    let advanced = engine
        .submit_sleep_batch_transactional(&context, request)
        .unwrap();
    let stale = TransactionalSleepSubmitRequest::new(
        "submit_stale",
        T3,
        T3,
        &start.run,
        vec![LlmResponse::Ok {
            request_id: start.run.requests[0].request.request_id.clone(),
            text: "{}".to_string(),
        }],
    )
    .unwrap();
    let error = engine
        .submit_sleep_batch_transactional(&context, stale)
        .expect_err("stale worker must not overwrite advanced run");
    assert!(error.to_string().contains("changed after its digest"));
    assert_eq!(
        read_run(&storage, &context, &start.run.sleep_task_id),
        advanced.run
    );
}

#[test]
fn transactional_sleep_cancel_releases_events_without_claiming_archive_coverage() {
    let context = context("account_alpha", "vault_cancel");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let event_id = ingest(
        &engine,
        &context,
        "ingest_cancel",
        "session_owner",
        T0,
        "Keep this event available.",
        false,
    );
    let start = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_cancel_target", "session_owner"),
        )
        .unwrap();
    let request = TransactionalSleepCancelRequest::new(
        "sleep_cancel",
        T2,
        T2,
        &start.run,
        "worker abandoned provider run",
    )
    .unwrap();
    storage
        .inject_commit_fault_once("sleep_cancel", InMemoryCommitFault::AfterPublishBeforeAck)
        .unwrap();
    let cancelled = engine
        .cancel_sleep_run_transactional(&context, request.clone())
        .unwrap();
    assert_eq!(
        cancelled.schema_version,
        "transactional_sleep_cancel_result.v1"
    );
    assert_eq!(cancelled.run.stage, SleepRunStage::Finished);
    assert_eq!(cancelled.run.completion_mode.as_deref(), Some("cancelled"));
    let session = read_session(&storage, &context, "session_owner");
    assert_eq!(session.status, SessionStatus::Active);
    assert!(session.active_sleep_task_id.is_none());
    assert!(session.sleep_pending_event_ids.is_empty());
    assert!(session.archived_event_ids.is_empty());
    assert!(session.archived_to.is_empty());
    assert_eq!(
        read_archive(&storage, &context, &start.run.archive_id).status,
        ArchiveStatus::Superseded
    );
    assert_eq!(
        read_task(&storage, &context, &start.run.sleep_task_id).state,
        TaskState::Cancelled
    );
    assert_eq!(
        engine
            .cancel_sleep_run_transactional(&context, request)
            .unwrap(),
        cancelled
    );

    let restarted = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_after_cancel", "session_owner"),
        )
        .unwrap();
    assert_eq!(
        read_archive(&storage, &context, &restarted.run.archive_id).source_event_ids,
        vec![event_id],
        "cancelled Sleep must not hide its source event"
    );
}

#[test]
fn transactional_sleep_finalize_rolls_back_every_record_on_failure() {
    for (index, phase) in [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterMutation(3),
        InMemoryCommitFault::AfterMutation(4),
        InMemoryCommitFault::AfterMutation(5),
        InMemoryCommitFault::AfterMutation(6),
        InMemoryCommitFault::AfterMutation(7),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ]
    .into_iter()
    .enumerate()
    {
        let context = context("account_alpha", &format!("vault_finalize_fault_{index}"));
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        let ready = ready_transactional_run(&engine, &storage, &context, &format!("f{index}"));
        let request = TransactionalSleepFinalizeRequest::new(
            format!("finalize_fault_{index}"),
            T4,
            T4,
            &ready,
        )
        .unwrap();
        let operation_id = request.work_unit.operation_id.clone();
        let before_session = read_record(&storage, &context, "sessions", "session_owner");
        let before_archive = read_record(&storage, &context, "archives", &ready.archive_id);
        let before_run = read_record(&storage, &context, "sleep_runs", &ready.sleep_task_id);
        storage
            .inject_commit_fault_once(&operation_id, phase)
            .unwrap();
        let error = engine
            .finalize_sleep_run_transactional(&context, request.clone())
            .expect_err("pre-publish finalize fault must roll back every record");
        assert!(error.to_string().contains(&format!("{phase:?}")));
        assert_eq!(
            read_record(&storage, &context, "sessions", "session_owner"),
            before_session
        );
        assert_eq!(
            read_record(&storage, &context, "archives", &ready.archive_id),
            before_archive
        );
        assert_eq!(
            read_record(&storage, &context, "sleep_runs", &ready.sleep_task_id),
            before_run
        );
        assert!(storage
            .committed_receipt(&context, &operation_id)
            .unwrap()
            .is_none());
        let success = engine
            .finalize_sleep_run_transactional(&context, request)
            .unwrap();
        assert_eq!(success.receipt.mutation_count, 7);
    }
}

#[test]
fn transactional_sleep_finalize_rejects_invented_memory_provenance_atomically() {
    let context = context("account_alpha", "vault_bad_provenance");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let ready =
        ready_transactional_run_with_event_id(&engine, &context, "bad_provenance", "mu_missing");
    let request =
        TransactionalSleepFinalizeRequest::new("finalize_bad_provenance", T4, T4, &ready).unwrap();
    let before = read_record(&storage, &context, "sessions", "session_owner");
    let error = engine
        .finalize_sleep_run_transactional(&context, request)
        .expect_err("invented source event must fail before publish");
    assert!(error.to_string().contains("outside frozen sleep input"));
    assert_eq!(
        read_record(&storage, &context, "sessions", "session_owner"),
        before
    );
    assert_eq!(
        read_archive(&storage, &context, &ready.archive_id).status,
        ArchiveStatus::Preliminary
    );
}

#[test]
fn transactional_sleep_enforces_serialized_input_and_response_limits() {
    let response_context = context("account_alpha", "vault_response_limit");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    ingest(
        &engine,
        &response_context,
        "ingest_response_limit",
        "session_owner",
        T0,
        "Mars matters.",
        false,
    );
    let start = engine
        .begin_sleep_run_transactional(
            &response_context,
            sleep_request(&engine, "sleep_response_limit", "session_owner"),
        )
        .unwrap();
    let error = TransactionalSleepSubmitRequest::new(
        "submit_too_large",
        T2,
        T2,
        &start.run,
        vec![LlmResponse::Ok {
            request_id: start.run.requests[0].request.request_id.clone(),
            text: "x".repeat(MAX_SLEEP_SUBMIT_RESPONSE_BYTES + 1),
        }],
    )
    .expect_err("oversized provider result must fail before a work unit starts");
    assert!(error.to_string().contains("serialized bytes"));
    assert_eq!(
        read_run(&storage, &response_context, &start.run.sleep_task_id),
        start.run
    );

    let large_context = context("account_alpha", "vault_input_limit");
    let large_storage = InMemoryTransactionalStorage::new();
    let large_engine = MemoryEngine::new(large_storage.clone());
    ingest(
        &large_engine,
        &large_context,
        "ingest_input_limit",
        "session_owner",
        T0,
        &"x".repeat(MAX_SLEEP_WORKFLOW_INPUT_BYTES),
        false,
    );
    let error = large_engine
        .begin_sleep_run_transactional(
            &large_context,
            sleep_request(&large_engine, "sleep_input_limit", "session_owner"),
        )
        .expect_err("oversized frozen input must not reserve the Session");
    assert!(error.to_string().contains("serialized bytes"));
    let session = read_session(&large_storage, &large_context, "session_owner");
    assert_eq!(session.status, SessionStatus::Active);
    assert!(session.active_sleep_task_id.is_none());
}

#[test]
fn transactional_sleep_submit_rolls_back_all_logical_edges() {
    for (index, phase) in [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterMutation(3),
        InMemoryCommitFault::AfterMutation(4),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ]
    .into_iter()
    .enumerate()
    {
        let context = context("account_alpha", &format!("vault_submit_fault_{index}"));
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        let event_id = ingest(
            &engine,
            &context,
            &format!("ingest_submit_fault_{index}"),
            "session_owner",
            T0,
            "Mars matters.",
            false,
        );
        let start = engine
            .begin_sleep_run_transactional(
                &context,
                sleep_request(
                    &engine,
                    &format!("sleep_submit_fault_{index}"),
                    "session_owner",
                ),
            )
            .unwrap();
        let request = TransactionalSleepSubmitRequest::new(
            format!("submit_fault_{index}"),
            T2,
            T2,
            &start.run,
            extraction_responses(&start.run, start.batch.clone().unwrap(), &event_id),
        )
        .unwrap();
        let before_run = read_record(&storage, &context, "sleep_runs", &start.run.sleep_task_id);
        let before_sleep_task = read_record(&storage, &context, "tasks", &start.run.sleep_task_id);
        let before_memory_task = read_record(
            &storage,
            &context,
            "tasks",
            start.run.memory_unit_task_id.as_deref().unwrap(),
        );
        storage
            .inject_commit_fault_once(&request.work_unit.operation_id, phase)
            .unwrap();
        let error = engine
            .submit_sleep_batch_transactional(&context, request.clone())
            .expect_err("submit fault must roll back run and both tasks");
        assert!(error.to_string().contains(&format!("{phase:?}")));
        assert_eq!(
            read_record(&storage, &context, "sleep_runs", &start.run.sleep_task_id),
            before_run
        );
        assert_eq!(
            read_record(&storage, &context, "tasks", &start.run.sleep_task_id),
            before_sleep_task
        );
        assert_eq!(
            read_record(
                &storage,
                &context,
                "tasks",
                start.run.memory_unit_task_id.as_deref().unwrap()
            ),
            before_memory_task
        );
        let success = engine
            .submit_sleep_batch_transactional(&context, request)
            .unwrap();
        assert_eq!(success.receipt.mutation_count, 4);
    }
}

#[test]
fn transactional_sleep_cancel_rolls_back_all_logical_edges() {
    for (index, phase) in [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterMutation(3),
        InMemoryCommitFault::AfterMutation(4),
        InMemoryCommitFault::AfterMutation(5),
        InMemoryCommitFault::AfterMutation(6),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ]
    .into_iter()
    .enumerate()
    {
        let context = context("account_alpha", &format!("vault_cancel_fault_{index}"));
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        ingest(
            &engine,
            &context,
            &format!("ingest_cancel_fault_{index}"),
            "session_owner",
            T0,
            "Mars matters.",
            false,
        );
        let start = engine
            .begin_sleep_run_transactional(
                &context,
                sleep_request(
                    &engine,
                    &format!("sleep_cancel_fault_{index}"),
                    "session_owner",
                ),
            )
            .unwrap();
        let request = TransactionalSleepCancelRequest::new(
            format!("cancel_fault_{index}"),
            T2,
            T2,
            &start.run,
            "cancel fault proof",
        )
        .unwrap();
        let before_session = read_record(&storage, &context, "sessions", "session_owner");
        let before_archive = read_record(&storage, &context, "archives", &start.run.archive_id);
        let before_run = read_record(&storage, &context, "sleep_runs", &start.run.sleep_task_id);
        storage
            .inject_commit_fault_once(&request.work_unit.operation_id, phase)
            .unwrap();
        let error = engine
            .cancel_sleep_run_transactional(&context, request.clone())
            .expect_err("cancel fault must roll back every workflow record");
        assert!(error.to_string().contains(&format!("{phase:?}")));
        assert_eq!(
            read_record(&storage, &context, "sessions", "session_owner"),
            before_session
        );
        assert_eq!(
            read_record(&storage, &context, "archives", &start.run.archive_id),
            before_archive
        );
        assert_eq!(
            read_record(&storage, &context, "sleep_runs", &start.run.sleep_task_id),
            before_run
        );
        let success = engine
            .cancel_sleep_run_transactional(&context, request)
            .unwrap();
        assert_eq!(success.receipt.mutation_count, 6);
    }
}

#[test]
fn concurrent_transactional_sleep_submits_advance_one_frozen_run_once() {
    let context = context("account_alpha", "vault_submit_race");
    let storage = InMemoryTransactionalStorage::new();
    let engine = Arc::new(MemoryEngine::new(storage.clone()));
    let event_id = ingest(
        &engine,
        &context,
        "ingest_submit_race",
        "session_owner",
        T0,
        "Mars matters.",
        false,
    );
    let start = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_submit_race", "session_owner"),
        )
        .unwrap();
    let responses = extraction_responses(&start.run, start.batch.clone().unwrap(), &event_id);
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for operation_id in ["submit_race_a", "submit_race_b"] {
        let engine = Arc::clone(&engine);
        let context = context.clone();
        let barrier = Arc::clone(&barrier);
        let run = start.run.clone();
        let responses = responses.clone();
        handles.push(thread::spawn(move || {
            let request =
                TransactionalSleepSubmitRequest::new(operation_id, T2, T2, &run, responses)
                    .unwrap();
            barrier.wait();
            engine.submit_sleep_batch_transactional(&context, request)
        }));
    }
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results.into_iter().find_map(Result::err).unwrap();
    assert!(error.to_string().contains("changed after its digest"));
    assert_eq!(
        read_run(&storage, &context, &start.run.sleep_task_id).stage,
        SleepRunStage::Consolidation
    );
}

#[test]
fn transactional_sleep_finalize_preserves_turn_admitted_after_workflow_start() {
    let context = context("account_alpha", "vault_late_turn");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let first_event_id = ingest(
        &engine,
        &context,
        "ingest_before_sleep",
        "session_owner",
        T0,
        "First turn.",
        false,
    );
    let ready =
        ready_transactional_run_after_ingest(&engine, &context, "late_turn", &first_event_id);
    let late_event_id = ingest(
        &engine,
        &context,
        "ingest_during_sleep",
        "session_owner",
        T3,
        "Late live turn.",
        false,
    );
    let finished = engine
        .finalize_sleep_run_transactional(
            &context,
            TransactionalSleepFinalizeRequest::new("finalize_with_late_turn", T4, T4, &ready)
                .unwrap(),
        )
        .unwrap();
    let session = read_session(&storage, &context, "session_owner");
    assert_eq!(session.event_count, 2);
    assert_eq!(session.archived_event_ids, vec![first_event_id]);
    assert!(!session.archived_event_ids.contains(&late_event_id));
    assert_eq!(finished.outcome.archive_entry.source_event_ids.len(), 1);

    let next = engine
        .begin_sleep_run_transactional(
            &context,
            engine
                .new_transactional_sleep_start_request(
                    "sleep_late_turn_only",
                    T5,
                    T5,
                    "session_owner",
                )
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        read_archive(&storage, &context, &next.run.archive_id).source_event_ids,
        vec![late_event_id],
        "finalize must leave later live turns available for the next Sleep"
    );
}

#[test]
fn rewritten_sleep_input_is_rejected_by_recovery_and_submit() {
    let context = context("account_alpha", "vault_input_rewrite");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let event_id = ingest(
        &engine,
        &context,
        "ingest_input_rewrite",
        "session_owner",
        T0,
        "Mars matters.",
        false,
    );
    let start = engine
        .begin_sleep_run_transactional(
            &context,
            sleep_request(&engine, "sleep_input_rewrite", "session_owner"),
        )
        .unwrap();
    rewrite_sleep_input_at_revision_two(&storage, &context, &start.run.sleep_task_id);

    let error = engine
        .resume_sleep_run_transactional(&context, &start.run.sleep_task_id)
        .expect_err("recovery must reject a rewritten immutable input");
    assert!(error.to_string().contains("must remain at revision 1"));
    let request = TransactionalSleepSubmitRequest::new(
        "submit_rewritten_input",
        T2,
        T2,
        &start.run,
        extraction_responses(&start.run, start.batch.unwrap(), &event_id),
    )
    .unwrap();
    let error = engine
        .submit_sleep_batch_transactional(&context, request)
        .expect_err("submit must reject a rewritten immutable input");
    assert!(error.to_string().contains("must remain at revision 1"));
    assert_eq!(
        read_run(&storage, &context, &start.run.sleep_task_id),
        start.run
    );
}

#[test]
fn rewritten_preliminary_archive_or_task_contract_is_rejected() {
    for target in ["archive", "task"] {
        let context = context("account_alpha", &format!("vault_rewrite_{target}"));
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        let event_id = ingest(
            &engine,
            &context,
            &format!("ingest_rewrite_{target}"),
            "session_owner",
            T0,
            "Mars matters.",
            false,
        );
        let start = engine
            .begin_sleep_run_transactional(
                &context,
                sleep_request(&engine, &format!("sleep_rewrite_{target}"), "session_owner"),
            )
            .unwrap();
        if target == "archive" {
            rewrite_record_payload(
                &storage,
                &context,
                "archives",
                &start.run.archive_id,
                "gist",
                json!("tampered gist"),
                &format!("rewrite_archive_{target}"),
            );
        } else {
            rewrite_record_payload(
                &storage,
                &context,
                "tasks",
                &start.run.sleep_task_id,
                "prompt_id",
                json!("tampered_prompt"),
                &format!("rewrite_task_{target}"),
            );
        }
        let request = TransactionalSleepSubmitRequest::new(
            format!("submit_rewrite_{target}"),
            T2,
            T2,
            &start.run,
            extraction_responses(&start.run, start.batch.unwrap(), &event_id),
        )
        .unwrap();
        let error = engine
            .submit_sleep_batch_transactional(&context, request)
            .expect_err("rewritten workflow dependency must fail closed");
        if target == "archive" {
            assert!(error.to_string().contains("Archive changed"));
        } else {
            assert!(error.to_string().contains("task contract changed"));
        }
        assert_eq!(
            read_run(&storage, &context, &start.run.sleep_task_id),
            start.run
        );
    }
}

fn extraction_responses(
    run: &SleepRun,
    batch: memory_engine::LlmBatch,
    cited_event_id: &str,
) -> Vec<LlmResponse> {
    batch
        .requests
        .into_iter()
        .map(|request| {
            let text = match request.prompt_id.as_str() {
                "memory_unit_pass" => json!({
                    "schema_version": "memory_units_result.v1",
                    "archive_id": run.archive_id,
                    "memory_units": [{
                        "thesis": "The user loves exploring Mars.",
                        "source_event_ids": [cited_event_id],
                        "tags": ["space", "mars"],
                        "weight": 0.9
                    }]
                }),
                "sleep_emotional_pass" => json!({"emotional_markers": []}),
                "sleep_topic_thread_pass" => json!({
                    "topic_thread": [{
                        "topic": "Mars exploration",
                        "summary": "The user discussed exploring Mars.",
                        "source_event_ids": [cited_event_id]
                    }]
                }),
                "sleep_personal_signal_pass" => json!({"personal_signals": []}),
                "sleep_relational_pass" => json!({"relational_tone": null}),
                other => panic!("unexpected extraction prompt: {other}"),
            };
            LlmResponse::Ok {
                request_id: request.request_id,
                text: text.to_string(),
            }
        })
        .collect()
}

fn ready_transactional_run(
    engine: &MemoryEngine<InMemoryTransactionalStorage>,
    _storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    suffix: &str,
) -> SleepRun {
    let event_id = ingest(
        engine,
        context,
        &format!("ingest_{suffix}"),
        "session_owner",
        T0,
        "I love exploring Mars.",
        true,
    );
    ready_transactional_run_after_ingest(engine, context, suffix, &event_id)
}

fn ready_transactional_run_with_event_id(
    engine: &MemoryEngine<InMemoryTransactionalStorage>,
    context: &VaultContext,
    suffix: &str,
    cited_event_id: &str,
) -> SleepRun {
    let _actual_event_id = ingest(
        engine,
        context,
        &format!("ingest_{suffix}"),
        "session_owner",
        T0,
        "I love exploring Mars.",
        true,
    );
    ready_transactional_run_after_ingest(engine, context, suffix, cited_event_id)
}

fn ready_transactional_run_after_ingest(
    engine: &MemoryEngine<InMemoryTransactionalStorage>,
    context: &VaultContext,
    suffix: &str,
    cited_event_id: &str,
) -> SleepRun {
    let start = engine
        .begin_sleep_run_transactional(
            context,
            sleep_request(engine, &format!("sleep_{suffix}"), "session_owner"),
        )
        .unwrap();
    let extraction = TransactionalSleepSubmitRequest::new(
        format!("submit_extract_{suffix}"),
        T2,
        T2,
        &start.run,
        extraction_responses(&start.run, start.batch.unwrap(), cited_event_id),
    )
    .unwrap();
    let extracted = engine
        .submit_sleep_batch_transactional(context, extraction)
        .unwrap();
    let consolidator = extracted.batch.clone().unwrap().requests.remove(0);
    let consolidation = TransactionalSleepSubmitRequest::new(
        format!("submit_consolidator_{suffix}"),
        T3,
        T3,
        &extracted.run,
        vec![LlmResponse::Ok {
            request_id: consolidator.request_id,
            text: "GIST: The user loves exploring Mars.\n\nThe user expressed a durable interest in exploring Mars."
                .to_string(),
        }],
    )
    .unwrap();
    engine
        .submit_sleep_batch_transactional(context, consolidation)
        .unwrap()
        .run
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

fn rewrite_sleep_input_at_revision_two(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    sleep_task_id: &str,
) {
    let key = RecordKey::new("sleep_inputs", sleep_task_id).unwrap();
    let intent = WorkUnitIntent {
        schema_version: "storage_work_unit_intent.v1".to_string(),
        operation_id: "rewrite_sleep_input_for_test".to_string(),
        operation_kind: "test_illegal_sleep_input_rewrite".to_string(),
        request_digest: format!("sha256:{}", "7".repeat(64)),
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
            result_digest: format!("sha256:{}", "8".repeat(64)),
        })
    })
    .unwrap();
}

fn rewrite_record_payload(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    collection: &str,
    record_id: &str,
    field: &str,
    value: serde_json::Value,
    operation_id: &str,
) {
    let key = RecordKey::new(collection, record_id).unwrap();
    let intent = WorkUnitIntent {
        schema_version: "storage_work_unit_intent.v1".to_string(),
        operation_id: operation_id.to_string(),
        operation_kind: "test_illegal_workflow_rewrite".to_string(),
        request_digest: format!("sha256:{}", "9".repeat(64)),
        started_at: T1.to_string(),
    };
    execute_work_unit_with_retry(storage, context, intent, 1, |work_unit| {
        let record = work_unit.get(&key)?.unwrap();
        let mut payload = record.payload;
        payload
            .as_object_mut()
            .unwrap()
            .insert(field.to_string(), value.clone());
        work_unit.put(
            key.clone(),
            ExpectedRevision::Exact(record.revision),
            record.payload_schema_version,
            payload,
        )?;
        Ok(WorkUnitCommit {
            committed_at: T1.to_string(),
            result_reference: key.reference(),
            result_digest: format!("sha256:{}", "a".repeat(64)),
        })
    })
    .unwrap();
}
