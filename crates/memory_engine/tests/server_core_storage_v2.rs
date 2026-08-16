use std::sync::{Arc, Barrier};
use std::thread;

use memory_engine::core_store::{
    CoreFactInput, CoreFactPatchInput, CoreFactStatus, CoreStoreCategory,
};
use memory_engine::server_runtime::{
    AccountVaultMembership, AuthenticatedPrincipal, MembershipStatus, PrincipalKind, VaultContext,
    ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION, AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION,
};
use memory_engine::storage_v2::{RecordKey, TransactionalStorage};
use memory_engine::types::{CORE_FACT_INPUT_SCHEMA_VERSION, CORE_FACT_PATCH_INPUT_SCHEMA_VERSION};
use memory_engine::{
    InMemoryCommitFault, InMemoryTransactionalStorage, MemoryEngine, TransactionalCorePatchRequest,
    TransactionalCoreUpsertRequest,
};
use sha2::{Digest, Sha256};

const T0: &str = "2026-08-01T12:00:00Z";
const T1: &str = "2026-08-01T12:00:01Z";
const T2: &str = "2026-08-01T12:00:02Z";

#[test]
fn concurrent_core_upserts_preserve_both_facts_and_advance_one_category_revision() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = Arc::new(MemoryEngine::new(storage.clone()));
    let barrier = Arc::new(Barrier::new(3));

    let mut handles = Vec::new();
    for (operation_id, text) in [
        ("core_concurrent_mars", "Interested in Mars colonies"),
        ("core_concurrent_music", "Enjoys chamber music"),
    ] {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        let context = context.clone();
        handles.push(thread::spawn(move || {
            let request = upsert_request(operation_id, text, None, 0.7, vec![], vec![]);
            barrier.wait();
            engine
                .upsert_core_fact_transactional(&context, request)
                .unwrap()
        }));
    }
    barrier.wait();
    for handle in handles {
        assert!(handle.join().unwrap().upsert.created);
    }

    let record = read_category_record(&storage, &context, "interests");
    let category: CoreStoreCategory = serde_json::from_value(record.payload).unwrap();
    assert_eq!(record.revision, 2);
    assert_eq!(category.facts.len(), 2);
    assert!(category
        .facts
        .iter()
        .any(|fact| fact.text == "Interested in Mars colonies"));
    assert!(category
        .facts
        .iter()
        .any(|fact| fact.text == "Enjoys chamber music"));
}

#[test]
fn core_upsert_replay_returns_the_original_durable_result() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let request = upsert_request(
        "core_replay_1",
        "Interested in Mars colonies",
        None,
        0.7,
        vec!["space"],
        vec!["archive_one"],
    );

    let first = engine
        .upsert_core_fact_transactional(&context, request.clone())
        .unwrap();
    let wire = serde_json::to_value(&first).unwrap();
    assert_eq!(
        wire["schema_version"],
        "transactional_core_upsert_result.v1"
    );
    assert_eq!(
        wire["receipt"]["schema_version"],
        "storage_work_unit_receipt.v1"
    );
    assert_eq!(wire["receipt"]["vault_id"], "vault_alpha");
    assert_eq!(wire["receipt"]["account_id"], "account_alpha");
    engine
        .upsert_core_fact_transactional(
            &context,
            upsert_request(
                "core_reinforce_after_replay",
                "Interested in Mars colonies",
                None,
                0.9,
                vec!["colonization"],
                vec!["archive_two"],
            ),
        )
        .unwrap();
    let replay = engine
        .upsert_core_fact_transactional(&context, request)
        .unwrap();

    assert_eq!(replay, first);
    assert!(replay.upsert.created);
    assert_eq!(replay.upsert.fact.confidence, 0.7);
    let current = read_category(&storage, &context, "interests");
    assert_eq!(current.facts.len(), 1);
    assert_eq!(current.facts[0].confidence, 0.9);
    assert_eq!(current.facts[0].source_archive_ids.len(), 2);
}

#[test]
fn core_upsert_rejects_digest_tampering_before_writing() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let mut request = upsert_request("core_tamper_1", "Original fact", None, 0.7, vec![], vec![]);
    request.input.text = "Changed after digest".to_string();

    let error = engine
        .upsert_core_fact_transactional(&context, request)
        .expect_err("frozen digest must reject changed domain input");
    assert!(error.to_string().contains("request_digest"));
    assert_eq!(storage.record_count(&context).unwrap(), 0);
}

#[test]
fn core_reinforcement_merges_same_scope_but_preserves_scope_isolation() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());

    let first = engine
        .upsert_core_fact_transactional(
            &context,
            upsert_request(
                "core_scope_owner_1",
                "Interested in Mars colonies",
                Some("owner"),
                0.6,
                vec!["space"],
                vec!["archive_one"],
            ),
        )
        .unwrap();
    let reinforced = engine
        .upsert_core_fact_transactional(
            &context,
            upsert_request(
                "core_scope_owner_2",
                "  Interested   in Mars colonies ",
                Some("owner"),
                0.8,
                vec!["colonization"],
                vec!["archive_two"],
            ),
        )
        .unwrap();
    let other_scope = engine
        .upsert_core_fact_transactional(
            &context,
            upsert_request(
                "core_scope_guest_1",
                "Interested in Mars colonies",
                Some("guest"),
                0.5,
                vec![],
                vec![],
            ),
        )
        .unwrap();

    assert!(first.upsert.created);
    assert!(!reinforced.upsert.created);
    assert_eq!(
        first.upsert.fact.core_fact_id,
        reinforced.upsert.fact.core_fact_id
    );
    assert_ne!(
        first.upsert.fact.core_fact_id,
        other_scope.upsert.fact.core_fact_id
    );
    let category = read_category(&storage, &context, "interests");
    assert_eq!(category.facts.len(), 2);
    let owner = category
        .facts
        .iter()
        .find(|fact| fact.scope.as_deref() == Some("owner"))
        .unwrap();
    assert_eq!(owner.confidence, 0.8);
    assert_eq!(owner.tags, vec!["space", "colonization"]);
    assert_eq!(owner.source_archive_ids.len(), 2);
}

#[test]
fn core_patch_is_a_scoped_point_mutation_and_replays_exactly() {
    let context = context("account_alpha", "vault_alpha");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let created = engine
        .upsert_core_fact_transactional(
            &context,
            upsert_request(
                "core_patch_seed",
                "Interested in Mars colonies",
                Some("owner"),
                0.7,
                vec!["space"],
                vec![],
            ),
        )
        .unwrap();
    let request = patch_request(
        "core_patch_1",
        "interests",
        &created.upsert.fact.core_fact_id,
        Some("owner"),
    );

    let first = engine
        .patch_core_fact_transactional(&context, request.clone())
        .unwrap();
    let replay = engine
        .patch_core_fact_transactional(&context, request)
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.patch.fact.status, CoreFactStatus::Contested);
    assert_eq!(first.patch.fact.confidence, 0.4);

    let wrong_scope = patch_request(
        "core_patch_wrong_scope",
        "interests",
        &created.upsert.fact.core_fact_id,
        Some("guest"),
    );
    assert!(engine
        .patch_core_fact_transactional(&context, wrong_scope)
        .unwrap_err()
        .to_string()
        .contains("requested scope"));
    let wrong_category = patch_request(
        "core_patch_wrong_category",
        "identity",
        &created.upsert.fact.core_fact_id,
        Some("owner"),
    );
    assert!(engine
        .patch_core_fact_transactional(&context, wrong_category)
        .unwrap_err()
        .to_string()
        .contains("category not found"));
    let current = read_category(&storage, &context, "interests");
    assert_eq!(current.facts.len(), 1);
    assert_eq!(current.facts[0].status, CoreFactStatus::Contested);
}

#[test]
fn core_upsert_rolls_back_at_each_pre_publish_edge_and_recovers_lost_ack() {
    let phases = [
        InMemoryCommitFault::BeforeApply,
        InMemoryCommitFault::AfterMutation(1),
        InMemoryCommitFault::AfterMutation(2),
        InMemoryCommitFault::AfterReceiptBeforePublish,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let context = context("account_alpha", "vault_alpha");
        let storage = InMemoryTransactionalStorage::new();
        let engine = MemoryEngine::new(storage.clone());
        let operation_id = format!("core_fault_{index}");
        let request = upsert_request(&operation_id, "Atomic fact", None, 0.7, vec![], vec![]);
        storage
            .inject_commit_fault_once(&operation_id, phase)
            .unwrap();
        assert!(engine
            .upsert_core_fact_transactional(&context, request.clone())
            .unwrap_err()
            .to_string()
            .contains(&format!("{phase:?}")));
        assert_eq!(storage.record_count(&context).unwrap(), 0);
        engine
            .upsert_core_fact_transactional(&context, request)
            .unwrap();
        assert_eq!(storage.record_count(&context).unwrap(), 2);
    }

    let context = context("account_beta", "vault_beta");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());
    let operation_id = "core_lost_ack";
    let request = upsert_request(operation_id, "Committed fact", None, 0.7, vec![], vec![]);
    storage
        .inject_commit_fault_once(operation_id, InMemoryCommitFault::AfterPublishBeforeAck)
        .unwrap();
    let first = engine
        .upsert_core_fact_transactional(&context, request.clone())
        .unwrap();
    let replay = engine
        .upsert_core_fact_transactional(&context, request)
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(storage.record_count(&context).unwrap(), 2);
}

#[test]
fn identical_core_categories_are_isolated_between_vaults() {
    let alpha = context("account_alpha", "vault_alpha");
    let beta = context("account_beta", "vault_beta");
    let storage = InMemoryTransactionalStorage::new();
    let engine = MemoryEngine::new(storage.clone());

    engine
        .upsert_core_fact_transactional(
            &alpha,
            upsert_request("core_alpha", "Alpha fact", None, 0.7, vec![], vec![]),
        )
        .unwrap();
    engine
        .upsert_core_fact_transactional(
            &beta,
            upsert_request("core_beta", "Beta fact", None, 0.7, vec![], vec![]),
        )
        .unwrap();

    assert_eq!(
        read_category(&storage, &alpha, "interests").facts[0].text,
        "Alpha fact"
    );
    assert_eq!(
        read_category(&storage, &beta, "interests").facts[0].text,
        "Beta fact"
    );
}

fn upsert_request(
    operation_id: &str,
    text: &str,
    scope: Option<&str>,
    confidence: f64,
    tags: Vec<&str>,
    archives: Vec<&str>,
) -> TransactionalCoreUpsertRequest {
    TransactionalCoreUpsertRequest::new(
        operation_id,
        T0,
        T1,
        CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "interests".to_string(),
            scope: scope.map(str::to_string),
            subject: None,
            text: text.to_string(),
            confidence,
            tags: tags.into_iter().map(str::to_string).collect(),
            source_archive_ids: archives.into_iter().map(str::to_string).collect(),
            source_candidate_id: None,
        },
    )
    .unwrap()
}

fn patch_request(
    operation_id: &str,
    category: &str,
    fact_id: &str,
    scope: Option<&str>,
) -> TransactionalCorePatchRequest {
    TransactionalCorePatchRequest::new(
        operation_id,
        T1,
        category,
        T2,
        CoreFactPatchInput {
            schema_version: CORE_FACT_PATCH_INPUT_SCHEMA_VERSION.to_string(),
            core_fact_id: fact_id.to_string(),
            scope: scope.map(str::to_string),
            text: None,
            status: Some(CoreFactStatus::Contested),
            confidence: Some(0.4),
            tags: Some(vec!["reviewed".to_string()]),
        },
    )
    .unwrap()
}

fn read_category(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    category: &str,
) -> CoreStoreCategory {
    serde_json::from_value(read_category_record(storage, context, category).payload).unwrap()
}

fn read_category_record(
    storage: &InMemoryTransactionalStorage,
    context: &VaultContext,
    category: &str,
) -> memory_engine::VaultRecord {
    let key = RecordKey::new(
        "core_categories",
        format!("category_{:x}", Sha256::digest(category.as_bytes())),
    )
    .unwrap();
    storage.read_record(context, &key).unwrap().unwrap()
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
