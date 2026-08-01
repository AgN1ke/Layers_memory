use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::event::IngestEvent;
use memory_engine::server_runtime::{
    plan_verified_identity_link, AccountId, AccountLinkDecision, AccountVaultMembership,
    AuthenticatedPrincipal, ClientRequestId, ExternalIdentityProvider, LocalVaultHandle,
    MembershipStatus, PrincipalKind, TurnAdmissionKind, TurnOperationStatus, TurnRequest,
    TurnTerminalResult, VaultContext, VaultId, VaultTurnLedger,
    ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION, AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION,
    TURN_REQUEST_SCHEMA_VERSION,
};
use memory_engine::storage::Storage;
use memory_engine::types::EVENT_SCHEMA_VERSION;
use memory_engine::MemoryEngine;
use serde_json::json;

const T0: &str = "2026-08-01T12:00:00Z";
const T1: &str = "2026-08-01T12:00:01Z";
const T2: &str = "2026-08-01T12:00:02Z";
const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn server_vault_context_fails_closed_without_matching_identity() {
    let principal = principal("account_alpha", PrincipalKind::Guest);
    let active_membership = membership("account_alpha", "vault_alpha", MembershipStatus::Active);

    assert!(VaultContext::authorize(None, Some(&active_membership)).is_err());
    assert!(VaultContext::authorize(Some(&principal), None).is_err());

    let forged = membership("account_bravo", "vault_bravo", MembershipStatus::Active);
    assert!(VaultContext::authorize(Some(&principal), Some(&forged)).is_err());

    let suspended = membership("account_alpha", "vault_alpha", MembershipStatus::Suspended);
    assert!(VaultContext::authorize(Some(&principal), Some(&suspended)).is_err());

    let context = VaultContext::authorize(Some(&principal), Some(&active_membership))
        .expect("matching active membership should authorize");
    assert_eq!(context.vault_id.as_str(), "vault_alpha");
    assert_eq!(context.account_id.as_str(), "account_alpha");
}

#[test]
fn verified_identity_conflict_requires_manual_review_without_merge() {
    let context = context("account_guest", "vault_guest");
    let other = AccountId::parse("account_play").expect("valid account id");
    let decision =
        plan_verified_identity_link(&context, ExternalIdentityProvider::PlayGames, Some(&other))
            .expect("verified link should produce a decision");

    assert_eq!(
        decision,
        AccountLinkDecision::ManualReviewRequired {
            current_account_id: AccountId::parse("account_guest").unwrap(),
            existing_account_id: other,
            provider: ExternalIdentityProvider::PlayGames,
        }
    );
}

#[test]
fn duplicate_request_reuses_one_operation_and_conflicting_replay_fails() {
    let context = context("account_alpha", "vault_alpha");
    let mut ledger = VaultTurnLedger::new(&context).unwrap();
    let request = turn_request("request_0001", "session_alpha", DIGEST_A, None);

    let first = ledger.begin_turn(&context, request.clone()).unwrap();
    let duplicate = ledger.begin_turn(&context, request).unwrap();
    assert_eq!(first.kind, TurnAdmissionKind::AcceptedNew);
    assert_eq!(duplicate.kind, TurnAdmissionKind::Existing);
    assert_eq!(first.operation.membership_revision, 1);
    assert_eq!(
        first.operation.operation_id,
        duplicate.operation.operation_id
    );
    assert_eq!(ledger.operation_count(), 1);

    let conflicting = turn_request("request_0001", "session_alpha", DIGEST_B, None);
    assert!(ledger.begin_turn(&context, conflicting).is_err());
    assert_eq!(ledger.operation_count(), 1);
}

#[test]
fn same_session_results_cannot_commit_out_of_order() {
    let context = context("account_alpha", "vault_alpha");
    let mut ledger = VaultTurnLedger::new(&context).unwrap();
    let first_id = ClientRequestId::parse("request_0001").unwrap();
    let second_id = ClientRequestId::parse("request_0002").unwrap();
    ledger
        .begin_turn(
            &context,
            turn_request("request_0001", "session_alpha", DIGEST_A, None),
        )
        .unwrap();
    ledger
        .begin_turn(
            &context,
            turn_request("request_0002", "session_alpha", DIGEST_B, None),
        )
        .unwrap();

    ledger
        .submit_terminal_result(
            &context,
            &second_id,
            TurnTerminalResult::succeeded("response:2", DIGEST_B),
            T1,
        )
        .unwrap();
    assert!(ledger.commit_turn(&context, &second_id, T2).is_err());

    ledger
        .submit_terminal_result(
            &context,
            &first_id,
            TurnTerminalResult::succeeded("response:1", DIGEST_A),
            T1,
        )
        .unwrap();
    let first = ledger.commit_turn(&context, &first_id, T2).unwrap();
    let second = ledger.commit_turn(&context, &second_id, T2).unwrap();
    assert_eq!(first.turn_sequence, 1);
    assert_eq!(second.turn_sequence, 2);
    assert_eq!(first.status, TurnOperationStatus::Committed);
    assert_eq!(second.status, TurnOperationStatus::Committed);
}

#[test]
fn cross_vault_context_cannot_operate_on_another_ledger() {
    let alpha = context("account_alpha", "vault_alpha");
    let bravo = context("account_bravo", "vault_bravo");
    let mut ledger = VaultTurnLedger::new(&alpha).unwrap();
    let request = turn_request("request_0001", "session_alpha", DIGEST_A, None);

    assert!(ledger.begin_turn(&bravo, request).is_err());
    assert_eq!(ledger.operation_count(), 0);
}

#[test]
fn local_vault_handle_survives_restart_and_never_uses_shared_fallback() {
    let root = unique_temp_dir("server_runtime_s1");
    let alpha = context("account_alpha", "vault_alpha");
    let bravo = context("account_bravo", "vault_bravo");
    let alpha_id = ClientRequestId::parse("request_0001").unwrap();
    let alpha_operation_id;

    {
        let handle = LocalVaultHandle::open(&root, alpha.clone(), "s1_test").unwrap();
        let admission = handle
            .begin_turn(turn_request(
                "request_0001",
                "session_alpha",
                DIGEST_A,
                Some(1),
            ))
            .unwrap();
        assert_eq!(admission.operation.turn_sequence, 1);
        alpha_operation_id = admission.operation.operation_id;
        handle
            .submit_terminal_result(
                &alpha_id,
                TurnTerminalResult::succeeded("response:1", DIGEST_A),
                T1,
            )
            .unwrap();
        handle.commit_turn(&alpha_id, T2).unwrap();
        assert!(
            handle
                .memory_storage()
                .root()
                .ends_with("vault_alpha\\memory")
                || handle
                    .memory_storage()
                    .root()
                    .ends_with("vault_alpha/memory")
        );
    }

    let restarted = LocalVaultHandle::open(&root, alpha, "s1_test").unwrap();
    let duplicate = restarted
        .begin_turn(turn_request(
            "request_0001",
            "session_alpha",
            DIGEST_A,
            Some(1),
        ))
        .unwrap();
    assert_eq!(duplicate.kind, TurnAdmissionKind::Existing);
    assert_eq!(duplicate.operation.status, TurnOperationStatus::Committed);
    assert_eq!(restarted.load_turn_ledger().unwrap().operation_count(), 1);

    let other = LocalVaultHandle::open(&root, bravo, "s1_test").unwrap();
    let other_admission = other
        .begin_turn(turn_request(
            "request_0001",
            "session_alpha",
            DIGEST_A,
            Some(1),
        ))
        .unwrap();
    assert_ne!(restarted.vault_root(), other.vault_root());
    assert_ne!(alpha_operation_id, other_admission.operation.operation_id);
    assert_eq!(other.load_turn_ledger().unwrap().operation_count(), 1);
    assert!(!root.join("vaults").join("local_user").exists());

    let alpha_engine = MemoryEngine::new(restarted.memory_storage().clone());
    let bravo_engine = MemoryEngine::new(other.memory_storage().clone());
    alpha_engine.ingest(event("alpha text")).unwrap();
    bravo_engine.ingest(event("bravo text")).unwrap();
    let alpha_session = alpha_engine
        .storage()
        .read_session("session_shared")
        .unwrap();
    let bravo_session = bravo_engine
        .storage()
        .read_session("session_shared")
        .unwrap();
    assert_eq!(alpha_session.events[0].payload["text"], "alpha text");
    assert_eq!(bravo_session.events[0].payload["text"], "bravo text");

    fs::remove_dir_all(root).ok();
}

#[test]
fn first_admission_enforces_optional_expected_sequence() {
    let context = context("account_alpha", "vault_alpha");
    let mut ledger = VaultTurnLedger::new(&context).unwrap();
    let wrong = turn_request("request_0001", "session_alpha", DIGEST_A, Some(2));
    assert!(ledger.begin_turn(&context, wrong).is_err());
    assert_eq!(ledger.operation_count(), 0);

    let first = turn_request("request_0001", "session_alpha", DIGEST_A, Some(1));
    assert_eq!(
        ledger
            .begin_turn(&context, first)
            .unwrap()
            .operation
            .turn_sequence,
        1
    );
}

fn principal(account_id: &str, kind: PrincipalKind) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        schema_version: AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION.to_string(),
        account_id: AccountId::parse(account_id).unwrap(),
        authentication_id: format!("auth_{account_id}"),
        kind,
        authenticated_at: T0.to_string(),
    }
}

fn membership(
    account_id: &str,
    vault_id: &str,
    status: MembershipStatus,
) -> AccountVaultMembership {
    AccountVaultMembership {
        schema_version: ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION.to_string(),
        membership_id: format!("membership_{account_id}"),
        account_id: AccountId::parse(account_id).unwrap(),
        vault_id: VaultId::parse(vault_id).unwrap(),
        status,
        created_at: T0.to_string(),
        revision: 1,
    }
}

fn context(account_id: &str, vault_id: &str) -> VaultContext {
    let principal = principal(account_id, PrincipalKind::Guest);
    let membership = membership(account_id, vault_id, MembershipStatus::Active);
    VaultContext::authorize(Some(&principal), Some(&membership)).unwrap()
}

fn turn_request(
    request_id: &str,
    session_id: &str,
    digest: &str,
    expected_turn_sequence: Option<u64>,
) -> TurnRequest {
    TurnRequest {
        schema_version: TURN_REQUEST_SCHEMA_VERSION.to_string(),
        client_request_id: ClientRequestId::parse(request_id).unwrap(),
        session_id: session_id.to_string(),
        request_digest: digest.to_string(),
        expected_turn_sequence,
        submitted_at: T0.to_string(),
    }
}

fn event(text: &str) -> IngestEvent {
    IngestEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_string(),
        event_type: "user_message".to_string(),
        source: "server_runtime_s1".to_string(),
        timestamp: T0.to_string(),
        session_id: "session_shared".to_string(),
        payload: json!({"text": text}),
        tags: Vec::new(),
        theme: None,
        emotional_tone: None,
        speaker: None,
        links: Vec::new(),
        importance_hint: Default::default(),
        processing_mode: Default::default(),
    }
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
