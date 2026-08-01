use std::sync::{Arc, Barrier};
use std::thread;

use memory_engine::archive::{ArchiveEntry, ArchiveStatus};
use memory_engine::recall::{RecallFilters, RecallQuery, RecallSourceLayer};
use memory_engine::server_runtime::{
    AccountVaultMembership, AuthenticatedPrincipal, MembershipStatus, PrincipalKind, VaultContext,
    ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION, AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION,
};
use memory_engine::storage_v2::{
    BeginWorkUnit, ExpectedRevision, RecordKey, StorageWorkUnit, TransactionalStorage,
    WorkUnitCommit, WorkUnitIntent, WORK_UNIT_INTENT_SCHEMA_VERSION,
};
use memory_engine::types::{ARCHIVE_ENTRY_SCHEMA_VERSION, RECALL_QUERY_SCHEMA_VERSION};
use memory_engine::{
    InMemoryCommitFault, InMemoryTransactionalStorage, MemoryEngine, TransactionalRecallRequest,
};
use serde_json::json;

const T0: &str = "2026-08-01T12:00:00Z";
const T1: &str = "2026-08-01T12:00:01Z";
const T2: &str = "2026-08-01T12:00:02Z";

#[test]
fn transactional_recall_commits_feedback_and_replays_the_original_result() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    seed_archives(
        &storage,
        &context,
        vec![
            archive("archive_mars", "session_owner", "Mars colonies", 0.8),
            archive("archive_gym", "session_owner", "Gym routine", 0.7),
        ],
    );
    let engine = MemoryEngine::new(storage.clone());
    let request = recall_request("recall_mars_1", Some("Mars"), 1, true);
    storage
        .inject_commit_fault_once("recall_mars_1", InMemoryCommitFault::AfterPublishBeforeAck)
        .unwrap();

    let first = engine
        .recall_transactional(&context, request.clone())
        .unwrap();
    assert_eq!(first.recall.items[0].id, "archive_mars");
    assert_eq!(
        read_archive(&storage, &context, "archive_mars").recall_count,
        1
    );
    assert_eq!(
        read_archive(&storage, &context, "archive_gym").recall_count,
        0
    );

    engine
        .recall_transactional(
            &context,
            recall_request("recall_mars_later", Some("Mars"), 1, false),
        )
        .unwrap();
    let replay = engine.recall_transactional(&context, request).unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        read_archive(&storage, &context, "archive_mars").recall_count,
        2
    );

    let wire = serde_json::to_value(&first).unwrap();
    assert_eq!(wire["schema_version"], "transactional_recall_result.v1");
    assert_eq!(wire["recall"]["schema_version"], "recall_result.v1");
    assert_eq!(wire["receipt"]["vault_id"], "vault_alpha");
    assert_eq!(wire["receipt"]["account_id"], "account_alpha");
}

#[test]
fn concurrent_transactional_recalls_do_not_lose_feedback() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    seed_archives(
        &storage,
        &context,
        vec![archive("archive_mars", "session_owner", "Mars", 0.8)],
    );
    let engine = Arc::new(MemoryEngine::new(storage.clone()));
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for operation_id in ["recall_parallel_1", "recall_parallel_2"] {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        let context = context.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            engine
                .recall_transactional(
                    &context,
                    recall_request(operation_id, Some("Mars"), 1, false),
                )
                .unwrap()
        }));
    }
    barrier.wait();
    for handle in handles {
        assert_eq!(handle.join().unwrap().recall.items[0].id, "archive_mars");
    }
    assert_eq!(
        read_archive(&storage, &context, "archive_mars").recall_count,
        2
    );
}

#[test]
fn transactional_recall_rolls_back_feedback_and_result_together() {
    let phases = [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterMutation(3),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let context = context("account_alpha", &format!("vault_atomic_{index}"));
        let storage = InMemoryTransactionalStorage::new();
        seed_archives(
            &storage,
            &context,
            vec![
                archive("archive_mars", "session_owner", "Mars", 0.8),
                archive("archive_venus", "session_owner", "Venus", 0.7),
            ],
        );
        let engine = MemoryEngine::new(storage.clone());
        let operation_id = format!("recall_atomic_{index}");
        let request = recall_request(&operation_id, None, 2, false);
        storage
            .inject_commit_fault_once(&operation_id, phase)
            .unwrap();

        assert!(engine
            .recall_transactional(&context, request.clone())
            .unwrap_err()
            .to_string()
            .contains(&format!("{phase:?}")));
        assert_eq!(
            read_archive(&storage, &context, "archive_mars").recall_count,
            0
        );
        assert_eq!(
            read_archive(&storage, &context, "archive_venus").recall_count,
            0
        );
        assert!(storage
            .read_record(
                &context,
                &RecordKey::new("operation_results", &operation_id).unwrap(),
            )
            .unwrap()
            .is_none());

        engine.recall_transactional(&context, request).unwrap();
        assert_eq!(
            read_archive(&storage, &context, "archive_mars").recall_count,
            1
        );
        assert_eq!(
            read_archive(&storage, &context, "archive_venus").recall_count,
            1
        );
    }
}

#[test]
fn collection_snapshot_detects_phantom_archive_insert() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    seed_archives(
        &storage,
        &context,
        vec![archive("archive_one", "session_owner", "One", 0.8)],
    );
    let mut reader = match storage
        .begin_work_unit(&context, intent("scan_reader"))
        .unwrap()
    {
        BeginWorkUnit::Started(work_unit) => work_unit,
        BeginWorkUnit::AlreadyCommitted(_) => panic!("new scan operation unexpectedly committed"),
    };
    assert_eq!(reader.scan_collection("archives").unwrap().len(), 1);

    seed_archives_with_operation(
        &storage,
        &context,
        "seed_phantom",
        vec![archive("archive_two", "session_owner", "Two", 0.7)],
    );
    let error = reader
        .commit(WorkUnitCommit {
            committed_at: T1.to_string(),
            result_reference: "scan:reader".to_string(),
            result_digest: digest('b'),
        })
        .expect_err("collection membership changed after snapshot");
    assert!(error.to_string().contains("collection archives changed"));
}

#[test]
fn durable_feedback_is_visible_to_another_engine_and_changes_ranking() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    seed_archives(
        &storage,
        &context,
        vec![
            archive("archive_alpha", "session_owner", "Alpha", 0.50),
            archive("archive_beta", "session_owner", "Beta", 0.49),
        ],
    );
    let first_replica = MemoryEngine::new(storage.clone());
    for index in 0..3 {
        first_replica
            .recall_transactional(
                &context,
                recall_request(&format!("recall_beta_{index}"), Some("Beta"), 1, false),
            )
            .unwrap();
    }

    let second_replica = MemoryEngine::new(storage.clone());
    let result = second_replica
        .recall_transactional(
            &context,
            recall_request("recall_general_after_feedback", None, 2, true),
        )
        .unwrap();
    assert_eq!(result.recall.items[0].id, "archive_beta");
    assert!(
        result.recall.debug.unwrap().candidates[0]
            .components
            .recall_boost
            > 1.0
    );
}

#[test]
fn transactional_recall_rejects_vectors_until_the_vector_storage_slice() {
    let mut query = query(Some("Mars"), 1, false);
    query.query_embedding = Some(memory_engine::recall::RecallQueryEmbedding {
        model_id: "model".to_string(),
        query_vec: vec![1.0, 0.0],
    });
    let error = TransactionalRecallRequest::new("recall_vector_1", T0, T1, query).unwrap_err();
    assert!(error
        .to_string()
        .contains("waits for the Storage v2 vector slice"));
}

#[test]
fn transactional_recall_rejects_query_changed_after_digest() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    seed_archives(
        &storage,
        &context,
        vec![archive("archive_mars", "session_owner", "Mars", 0.8)],
    );
    let engine = MemoryEngine::new(storage.clone());
    let mut request = recall_request("recall_tamper_1", Some("Mars"), 1, false);
    request.query.query_text = Some("Gym".to_string());
    let error = engine
        .recall_transactional(&context, request)
        .expect_err("frozen query digest must reject mutation");
    assert!(error.to_string().contains("request_digest"));
    assert_eq!(
        read_archive(&storage, &context, "archive_mars").recall_count,
        0
    );
}

#[test]
fn transactional_recall_isolates_identical_archive_ids_between_vaults() {
    let alpha = context("account_alpha", "vault_alpha");
    let beta = context("account_beta", "vault_beta");
    let storage = InMemoryTransactionalStorage::new();
    seed_archives(
        &storage,
        &alpha,
        vec![archive("archive_shared", "session_owner", "Alpha", 0.8)],
    );
    seed_archives(
        &storage,
        &beta,
        vec![archive("archive_shared", "session_owner", "Beta", 0.8)],
    );
    let engine = MemoryEngine::new(storage.clone());
    let alpha_result = engine
        .recall_transactional(&alpha, recall_request("recall_alpha_vault", None, 1, false))
        .unwrap();
    assert_eq!(alpha_result.recall.items[0].gist, "Alpha");
    assert_eq!(
        read_archive(&storage, &alpha, "archive_shared").recall_count,
        1
    );
    assert_eq!(
        read_archive(&storage, &beta, "archive_shared").recall_count,
        0
    );
}

fn recall_request(
    operation_id: &str,
    text: Option<&str>,
    limit: usize,
    explain: bool,
) -> TransactionalRecallRequest {
    TransactionalRecallRequest::new(operation_id, T0, T2, query(text, limit, explain)).unwrap()
}

fn query(text: Option<&str>, limit: usize, explain: bool) -> RecallQuery {
    RecallQuery {
        schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
        query_id: None,
        created_at: None,
        session_id: Some("session_owner".to_string()),
        context: json!({"recent_text": text.unwrap_or_default()}),
        query_text: text.map(str::to_string),
        query_embedding: None,
        filters: RecallFilters {
            source_layers: vec![RecallSourceLayer::Archive],
            ..RecallFilters::default()
        },
        limit,
        include_core: false,
        explain,
    }
}

fn seed_archives(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    archives: Vec<ArchiveEntry>,
) {
    seed_archives_with_operation(storage, context, "seed_archives", archives)
}

fn seed_archives_with_operation(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    operation_id: &str,
    archives: Vec<ArchiveEntry>,
) {
    let mut work_unit = match storage
        .begin_work_unit(context, intent(operation_id))
        .unwrap()
    {
        BeginWorkUnit::Started(work_unit) => work_unit,
        BeginWorkUnit::AlreadyCommitted(_) => panic!("seed operation unexpectedly replayed"),
    };
    for archive in archives {
        work_unit
            .put(
                RecordKey::new("archives", archive.archive_id.clone()).unwrap(),
                ExpectedRevision::Absent,
                ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                serde_json::to_value(archive).unwrap(),
            )
            .unwrap();
    }
    work_unit
        .commit(WorkUnitCommit {
            committed_at: T0.to_string(),
            result_reference: format!("seed:{operation_id}"),
            result_digest: digest('a'),
        })
        .unwrap();
}

fn read_archive(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    archive_id: &str,
) -> ArchiveEntry {
    let record = storage
        .read_record(context, &RecordKey::new("archives", archive_id).unwrap())
        .unwrap()
        .unwrap();
    serde_json::from_value(record.payload).unwrap()
}

fn archive(id: &str, session_id: &str, gist: &str, weight: f64) -> ArchiveEntry {
    ArchiveEntry {
        schema_version: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
        archive_id: id.to_string(),
        created_at: T0.to_string(),
        updated_at: T0.to_string(),
        source_session_id: session_id.to_string(),
        source_event_ids: vec![],
        time_range: memory_engine::types::TimeRange {
            start: T0.to_string(),
            end: T0.to_string(),
        },
        theme: None,
        tags: vec![],
        gist: gist.to_string(),
        narrative: gist.to_string(),
        compact_memory: Some(gist.to_string()),
        memory_units: vec![],
        facts: vec![],
        quotes: vec![],
        weight,
        freshness: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        links: vec![],
        emotional_markers: vec![],
        topic_thread: vec![],
        personal_signals: vec![],
        relational_tone: None,
        status: ArchiveStatus::Complete,
        llm_enhanced: false,
        prompt_id: None,
        prompt_version: None,
        embedding_model_id: None,
        embedding: None,
    }
}

fn intent(operation_id: &str) -> WorkUnitIntent {
    WorkUnitIntent {
        schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
        operation_id: operation_id.to_string(),
        operation_kind: "test_seed".to_string(),
        request_digest: digest('f'),
        started_at: T0.to_string(),
    }
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
