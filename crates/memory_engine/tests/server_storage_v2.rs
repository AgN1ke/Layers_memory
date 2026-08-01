use std::sync::{Arc, Barrier};
use std::thread;

use memory_engine::event::IngestEvent;
use memory_engine::server_runtime::{
    AccountVaultMembership, AuthenticatedPrincipal, MembershipStatus, PrincipalKind, VaultContext,
    ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION, AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION,
};
use memory_engine::storage_v2::{
    BeginWorkUnit, ExpectedRevision, RecordKey, StorageWorkUnit, TransactionalStorage,
    WorkUnitCommit, WorkUnitIntent, WORK_UNIT_INTENT_SCHEMA_VERSION,
};
use memory_engine::types::{EVENT_SCHEMA_VERSION, SESSION_SCHEMA_VERSION};
use memory_engine::{
    InMemoryCommitFault, InMemoryTransactionalStorage, MemoryEngine, TransactionalIngestRequest,
};
use serde_json::json;

const T0: &str = "2026-08-01T12:00:00Z";
const T1: &str = "2026-08-01T12:00:01Z";

#[test]
fn transactional_ingest_rolls_back_at_every_logical_pre_publish_edge() {
    let phases = [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ];

    for (case, phase) in phases.into_iter().enumerate() {
        let context = context("account_alpha", "vault_alpha");
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        let operation_id = format!("ingest_atomic_{case}");
        let request = ingest_request(&operation_id, "session_alpha", "first");
        let session_key = RecordKey::new("sessions", "session_alpha").unwrap();

        storage
            .inject_commit_fault_once(&operation_id, phase)
            .unwrap();
        let error = engine
            .ingest_transactional(&context, request.clone())
            .expect_err("injected commit must fail before publish");
        assert!(error.to_string().contains(&format!("{phase:?}")));
        assert_eq!(storage.record_count(&context).unwrap(), 0);
        assert!(storage
            .read_record(&context, &session_key)
            .unwrap()
            .is_none());
        assert!(storage
            .committed_receipt(&context, &request.work_unit.operation_id)
            .unwrap()
            .is_none());

        let result = engine
            .ingest_transactional(&context, request)
            .expect("same operation can be retried after a pre-publish failure");
        assert_eq!(result.receipt.mutation_count, 2);
        assert_eq!(storage.record_count(&context).unwrap(), 2);
        let session = storage
            .read_record(&context, &session_key)
            .unwrap()
            .expect("session committed with event");
        assert_eq!(session.payload_schema_version, SESSION_SCHEMA_VERSION);
        assert_eq!(session.payload["event_count"], 1);
    }
}

#[test]
fn transactional_ingest_reconciles_receipt_when_commit_ack_is_lost() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let operation_id = "ingest_lost_ack_1";
    let request = ingest_request(operation_id, "session_alpha", "committed once");
    let session_key = RecordKey::new("sessions", "session_alpha").unwrap();

    storage
        .inject_commit_fault_once(operation_id, InMemoryCommitFault::AfterPublishBeforeAck)
        .unwrap();
    let first = engine
        .ingest_transactional(&context, request.clone())
        .expect("work-unit executor must reconcile a lost commit acknowledgement");

    assert_eq!(storage.record_count(&context).unwrap(), 2);
    let committed_receipt = storage
        .committed_receipt(&context, operation_id)
        .unwrap()
        .expect("receipt was published atomically with domain records");
    assert_eq!(first.receipt, committed_receipt);
    let session = storage
        .read_record(&context, &session_key)
        .unwrap()
        .expect("session was committed before acknowledgement was lost");
    assert_eq!(session.payload["event_count"], 1);

    let replay = engine
        .ingest_transactional(&context, request)
        .expect("retry recovers the already committed receipt");
    assert_eq!(replay.receipt, committed_receipt);
    assert_eq!(replay, first);
    assert_eq!(storage.record_count(&context).unwrap(), 2);
    assert_eq!(
        storage
            .read_record(&context, &session_key)
            .unwrap()
            .unwrap()
            .payload["event_count"],
        1
    );
}

#[test]
fn commit_fault_targets_one_operation_and_invalid_edge_is_consumed_safely() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());

    storage
        .inject_commit_fault_once("ingest_target_1", InMemoryCommitFault::BeforeApply)
        .unwrap();
    engine
        .ingest_transactional(
            &context,
            ingest_request("ingest_other_1", "session_alpha", "other operation"),
        )
        .expect("another operation must not consume the targeted fault");
    let target_error = engine
        .ingest_transactional(
            &context,
            ingest_request("ingest_target_1", "session_alpha", "target operation"),
        )
        .expect_err("target operation must consume its fault");
    assert!(target_error.to_string().contains("BeforeApply"));
    assert_eq!(storage.record_count(&context).unwrap(), 2);

    storage
        .inject_commit_fault_once(
            "ingest_invalid_edge_1",
            InMemoryCommitFault::AfterMutation(3),
        )
        .unwrap();
    let request = ingest_request(
        "ingest_invalid_edge_1",
        "session_alpha",
        "invalid fault edge",
    );
    let invalid_error = engine
        .ingest_transactional(&context, request.clone())
        .expect_err("fault beyond two staged mutations must be rejected");
    assert!(invalid_error
        .to_string()
        .contains("exceeds 2 staged mutation(s)"));
    assert_eq!(storage.record_count(&context).unwrap(), 2);

    engine
        .ingest_transactional(&context, request)
        .expect("invalid fault was consumed and cannot poison the retry");
    assert_eq!(storage.record_count(&context).unwrap(), 3);
    let session = storage
        .read_record(
            &context,
            &RecordKey::new("sessions", "session_alpha").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(session.payload["event_count"], 2);
}

#[test]
fn transactional_ingest_replay_returns_one_receipt_and_does_not_count_twice() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let request = ingest_request("ingest_replay_1", "session_alpha", "same payload");

    let first = engine
        .ingest_transactional(&context, request.clone())
        .unwrap();
    let replay = engine.ingest_transactional(&context, request).unwrap();

    assert_eq!(first, replay);
    assert_eq!(storage.record_count(&context).unwrap(), 2);
    let session = storage
        .read_record(
            &context,
            &RecordKey::new("sessions", "session_alpha").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(session.payload["event_count"], 1);
}

#[test]
fn transactional_ingest_rejects_operation_id_reuse_for_different_input() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    engine
        .ingest_transactional(
            &context,
            ingest_request("ingest_reuse_1", "session_alpha", "first"),
        )
        .unwrap();

    let error = engine
        .ingest_transactional(
            &context,
            ingest_request("ingest_reuse_1", "session_alpha", "different"),
        )
        .expect_err("same operation id cannot commit different input");
    assert!(error.to_string().contains("different input"));
    assert_eq!(storage.record_count(&context).unwrap(), 2);
}

#[test]
fn transactional_ingest_rejects_payload_changed_after_digest_was_frozen() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let mut request = ingest_request("ingest_tampered_1", "session_alpha", "original");
    request.event.payload = json!({"text": "tampered"});

    let error = engine
        .ingest_transactional(&context, request)
        .expect_err("frozen digest must cover the complete ingest input");
    assert!(error
        .to_string()
        .contains("request_digest does not match its domain input"));
    assert_eq!(storage.record_count(&context).unwrap(), 0);
}

#[test]
fn transactional_ingest_serializes_versioned_receipt_and_vault_provenance() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage);
    let request = ingest_request("ingest_contract_1", "session_alpha", "contract");
    let request_json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        request_json["schema_version"],
        "transactional_ingest_request.v1"
    );
    assert_eq!(
        request_json["work_unit"]["schema_version"],
        "storage_work_unit_intent.v1"
    );

    let result = engine.ingest_transactional(&context, request).unwrap();
    let result_json = serde_json::to_value(result).unwrap();
    assert_eq!(
        result_json["schema_version"],
        "transactional_ingest_result.v1"
    );
    assert_eq!(
        result_json["receipt"]["schema_version"],
        "storage_work_unit_receipt.v1"
    );
    assert_eq!(result_json["receipt"]["vault_id"], "vault_alpha");
    assert_eq!(result_json["receipt"]["account_id"], "account_alpha");
    assert_eq!(result_json["receipt"]["mutation_count"], 2);
}

#[test]
fn concurrent_session_ingests_preserve_both_events_and_metadata_count() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = Arc::new(MemoryEngine::new(storage.clone()));
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();

    for (operation_id, text) in [
        ("ingest_concurrent_1", "alpha"),
        ("ingest_concurrent_2", "bravo"),
    ] {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        let context = context.clone();
        let request = ingest_request(operation_id, "session_shared", text);
        handles.push(thread::spawn(move || {
            barrier.wait();
            engine.ingest_transactional(&context, request).unwrap()
        }));
    }
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("ingest thread"))
        .collect();

    assert_ne!(
        results[0].ingest.stored_event.event_id,
        results[1].ingest.stored_event.event_id
    );
    assert_eq!(storage.record_count(&context).unwrap(), 3);
    let session = storage
        .read_record(
            &context,
            &RecordKey::new("sessions", "session_shared").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(session.payload["event_count"], 2);
    assert_eq!(session.revision, 2);
}

#[test]
fn optimistic_conflict_discards_every_staged_mutation() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let key_a = RecordKey::new("test", "record_a").unwrap();
    let key_b = RecordKey::new("test", "record_b").unwrap();
    seed_two_records(&storage, &context, &key_a, &key_b);

    let mut stale = started(&storage, &context, intent("stale_update", 'b'));
    let a = stale.get(&key_a).unwrap().unwrap();
    let b = stale.get(&key_b).unwrap().unwrap();
    stale
        .put(
            key_a.clone(),
            ExpectedRevision::Exact(a.revision),
            "test.v1".to_string(),
            json!({"value": "stale-a"}),
        )
        .unwrap();
    stale
        .put(
            key_b.clone(),
            ExpectedRevision::Exact(b.revision),
            "test.v1".to_string(),
            json!({"value": "stale-b"}),
        )
        .unwrap();

    let mut winner = started(&storage, &context, intent("winner_update", 'c'));
    let current_b = winner.get(&key_b).unwrap().unwrap();
    winner
        .put(
            key_b.clone(),
            ExpectedRevision::Exact(current_b.revision),
            "test.v1".to_string(),
            json!({"value": "winner-b"}),
        )
        .unwrap();
    winner.commit(commit("test:record_b", 'c')).unwrap();

    let error = stale
        .commit(commit("test:two_records", 'b'))
        .expect_err("stale transaction must conflict as one unit");
    assert!(matches!(error, memory_engine::StorageV2Error::Conflict(_)));
    let actual_a = storage.read_record(&context, &key_a).unwrap().unwrap();
    let actual_b = storage.read_record(&context, &key_b).unwrap().unwrap();
    assert_eq!(actual_a.payload["value"], "initial-a");
    assert_eq!(actual_a.revision, 1);
    assert_eq!(actual_b.payload["value"], "winner-b");
    assert_eq!(actual_b.revision, 2);
}

#[test]
fn identical_record_keys_are_isolated_by_vault_and_wrong_binding_fails_closed() {
    let alpha = context("account_alpha", "vault_alpha");
    let bravo = context("account_bravo", "vault_bravo");
    let storage = InMemoryTransactionalStorage::new();
    let alpha_engine = MemoryEngine::new(storage.clone());
    let bravo_engine = MemoryEngine::new(storage.clone());

    alpha_engine
        .ingest_transactional(
            &alpha,
            ingest_request("ingest_vault_1", "same_session", "alpha"),
        )
        .unwrap();
    bravo_engine
        .ingest_transactional(
            &bravo,
            ingest_request("ingest_vault_1", "same_session", "bravo"),
        )
        .unwrap();

    let key = RecordKey::new("sessions", "same_session").unwrap();
    assert_eq!(
        storage.read_record(&alpha, &key).unwrap().unwrap().revision,
        1
    );
    assert_eq!(
        storage.read_record(&bravo, &key).unwrap().unwrap().revision,
        1
    );

    let forged = context("account_other", "vault_alpha");
    let error = storage
        .read_record(&forged, &key)
        .expect_err("a bound vault must reject another account membership");
    assert!(error.to_string().contains("different account membership"));
}

fn seed_two_records(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    key_a: &RecordKey,
    key_b: &RecordKey,
) {
    let mut work = started(storage, context, intent("seed_records", 'a'));
    work.put(
        key_a.clone(),
        ExpectedRevision::Absent,
        "test.v1".to_string(),
        json!({"value": "initial-a"}),
    )
    .unwrap();
    work.put(
        key_b.clone(),
        ExpectedRevision::Absent,
        "test.v1".to_string(),
        json!({"value": "initial-b"}),
    )
    .unwrap();
    work.commit(commit("test:seed", 'a')).unwrap();
}

fn started<'a>(
    storage: &'a InMemoryTransactionalStorage,
    context: &VaultContext,
    intent: WorkUnitIntent,
) -> memory_engine::storage_v2::InMemoryWorkUnit<'a> {
    match storage.begin_work_unit(context, intent).unwrap() {
        BeginWorkUnit::Started(work) => work,
        BeginWorkUnit::AlreadyCommitted(_) => panic!("test operation unexpectedly committed"),
    }
}

fn intent(operation_id: &str, digest_char: char) -> WorkUnitIntent {
    WorkUnitIntent {
        schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
        operation_id: operation_id.to_string(),
        operation_kind: "test_mutation".to_string(),
        request_digest: digest(digest_char),
        started_at: T0.to_string(),
    }
}

fn commit(reference: &str, digest_char: char) -> WorkUnitCommit {
    WorkUnitCommit {
        committed_at: T1.to_string(),
        result_reference: reference.to_string(),
        result_digest: digest(digest_char),
    }
}

fn ingest_request(operation_id: &str, session_id: &str, text: &str) -> TransactionalIngestRequest {
    TransactionalIngestRequest::new(
        operation_id,
        "chibigochi_server",
        T0,
        T1,
        IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "game_client".to_string(),
            timestamp: T0.to_string(),
            session_id: session_id.to_string(),
            payload: json!({"text": text}),
            tags: vec!["conversation".to_string()],
            theme: Some("test".to_string()),
            emotional_tone: None,
            speaker: None,
            links: vec![],
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        },
    )
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

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}
