//! Server-runtime identity and ordered-turn contracts.
//!
//! This module is the S1 reference boundary. It deliberately does not provide
//! network authentication or a production multi-process storage backend. A
//! trusted server authenticates a principal, derives a [`VaultContext`] from
//! server-owned membership, and then opens a vault-bound handle. PostgreSQL
//! transactions and the public game API belong to later server phases.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{FileStorage, MemoryEngineError, Result};

pub const ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION: &str = "account_vault_membership.v1";
pub const AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION: &str = "authenticated_principal.v1";
pub const VAULT_CONTEXT_SCHEMA_VERSION: &str = "vault_context.v1";
pub const TURN_REQUEST_SCHEMA_VERSION: &str = "turn_request.v1";
pub const TURN_OPERATION_SCHEMA_VERSION: &str = "turn_operation.v1";
pub const VAULT_TURN_LEDGER_SCHEMA_VERSION: &str = "vault_turn_ledger.v1";

static TURN_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

macro_rules! opaque_id {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate_opaque_id($label, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(D::Error::custom)
            }
        }
    };
}

opaque_id!(AccountId, "account_id");
opaque_id!(VaultId, "vault_id");
opaque_id!(ClientRequestId, "client_request_id");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Guest,
    PlayGames,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIdentityProvider {
    Guest,
    PlayGames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipStatus {
    Active,
    Suspended,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedPrincipal {
    pub schema_version: String,
    pub account_id: AccountId,
    pub authentication_id: String,
    pub kind: PrincipalKind,
    pub authenticated_at: String,
}

impl AuthenticatedPrincipal {
    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "authenticated principal",
            &self.schema_version,
            AUTHENTICATED_PRINCIPAL_SCHEMA_VERSION,
        )?;
        validate_storage_component("authentication_id", &self.authentication_id)?;
        validate_timestamp("authenticated_at", &self.authenticated_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountVaultMembership {
    pub schema_version: String,
    pub membership_id: String,
    pub account_id: AccountId,
    pub vault_id: VaultId,
    pub status: MembershipStatus,
    pub created_at: String,
    pub revision: u64,
}

impl AccountVaultMembership {
    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "account-vault membership",
            &self.schema_version,
            ACCOUNT_VAULT_MEMBERSHIP_SCHEMA_VERSION,
        )?;
        validate_storage_component("membership_id", &self.membership_id)?;
        validate_timestamp("membership created_at", &self.created_at)?;
        if self.revision == 0 {
            return Err(MemoryEngineError::Validation(
                "membership revision must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

/// A request-scoped internal authorization capability. Public clients must
/// never submit or persist this structure as a reusable security token; the
/// trusted server derives it from current authentication and membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultContext {
    pub schema_version: String,
    pub account_id: AccountId,
    pub vault_id: VaultId,
    pub membership_id: String,
    pub membership_revision: u64,
    pub authentication_id: String,
    pub principal_kind: PrincipalKind,
    pub authorized_at: String,
}

impl VaultContext {
    pub fn authorize(
        principal: Option<&AuthenticatedPrincipal>,
        membership: Option<&AccountVaultMembership>,
    ) -> Result<Self> {
        let principal = principal.ok_or_else(|| {
            MemoryEngineError::Validation(
                "server mode requires an authenticated principal".to_string(),
            )
        })?;
        let membership = membership.ok_or_else(|| {
            MemoryEngineError::Validation(
                "server mode requires an explicit account-to-vault membership".to_string(),
            )
        })?;
        principal.validate()?;
        membership.validate()?;
        if membership.status != MembershipStatus::Active {
            return Err(MemoryEngineError::Validation(format!(
                "vault membership is not active: {:?}",
                membership.status
            )));
        }
        if principal.account_id != membership.account_id {
            return Err(MemoryEngineError::Validation(
                "authenticated account does not own the requested vault membership".to_string(),
            ));
        }

        Ok(Self {
            schema_version: VAULT_CONTEXT_SCHEMA_VERSION.to_string(),
            account_id: principal.account_id.clone(),
            vault_id: membership.vault_id.clone(),
            membership_id: membership.membership_id.clone(),
            membership_revision: membership.revision,
            authentication_id: principal.authentication_id.clone(),
            principal_kind: principal.kind,
            authorized_at: principal.authenticated_at.clone(),
        })
    }

    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "vault context",
            &self.schema_version,
            VAULT_CONTEXT_SCHEMA_VERSION,
        )?;
        validate_storage_component("membership_id", &self.membership_id)?;
        validate_storage_component("authentication_id", &self.authentication_id)?;
        validate_timestamp("authorized_at", &self.authorized_at)?;
        if self.membership_revision == 0 {
            return Err(MemoryEngineError::Validation(
                "vault context membership_revision must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum AccountLinkDecision {
    AttachToCurrentAccount {
        account_id: AccountId,
        provider: ExternalIdentityProvider,
    },
    AlreadyLinked {
        account_id: AccountId,
        provider: ExternalIdentityProvider,
    },
    ManualReviewRequired {
        current_account_id: AccountId,
        existing_account_id: AccountId,
        provider: ExternalIdentityProvider,
    },
}

/// Plans a verified external-identity link without ever merging two accounts
/// automatically. The authentication boundary supplies the optional existing
/// binding after it has verified the provider credential.
pub fn plan_verified_identity_link(
    context: &VaultContext,
    provider: ExternalIdentityProvider,
    existing_account_id: Option<&AccountId>,
) -> Result<AccountLinkDecision> {
    context.validate()?;
    if provider == ExternalIdentityProvider::Guest {
        return Err(MemoryEngineError::Validation(
            "guest identities are created, not linked as verified providers".to_string(),
        ));
    }
    Ok(match existing_account_id {
        None => AccountLinkDecision::AttachToCurrentAccount {
            account_id: context.account_id.clone(),
            provider,
        },
        Some(account_id) if account_id == &context.account_id => {
            AccountLinkDecision::AlreadyLinked {
                account_id: context.account_id.clone(),
                provider,
            }
        }
        Some(account_id) => AccountLinkDecision::ManualReviewRequired {
            current_account_id: context.account_id.clone(),
            existing_account_id: account_id.clone(),
            provider,
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRequest {
    pub schema_version: String,
    pub client_request_id: ClientRequestId,
    pub session_id: String,
    pub request_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_turn_sequence: Option<u64>,
    pub submitted_at: String,
}

impl TurnRequest {
    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "turn request",
            &self.schema_version,
            TURN_REQUEST_SCHEMA_VERSION,
        )?;
        validate_storage_component("session_id", &self.session_id)?;
        validate_sha256("request_digest", &self.request_digest)?;
        validate_timestamp("submitted_at", &self.submitted_at)?;
        if self.expected_turn_sequence == Some(0) {
            return Err(MemoryEngineError::Validation(
                "expected_turn_sequence must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnTerminalOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnTerminalResult {
    pub outcome: TurnTerminalOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl TurnTerminalResult {
    pub fn succeeded(
        result_reference: impl Into<String>,
        result_digest: impl Into<String>,
    ) -> Self {
        Self {
            outcome: TurnTerminalOutcome::Succeeded,
            result_reference: Some(result_reference.into()),
            result_digest: Some(result_digest.into()),
            error_code: None,
        }
    }

    pub fn failed(error_code: impl Into<String>) -> Self {
        Self {
            outcome: TurnTerminalOutcome::Failed,
            result_reference: None,
            result_digest: None,
            error_code: Some(error_code.into()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self.outcome {
            TurnTerminalOutcome::Succeeded => {
                let reference = self.result_reference.as_deref().ok_or_else(|| {
                    MemoryEngineError::Validation(
                        "successful turn result requires result_reference".to_string(),
                    )
                })?;
                validate_storage_component("result_reference", reference)?;
                let digest = self.result_digest.as_deref().ok_or_else(|| {
                    MemoryEngineError::Validation(
                        "successful turn result requires result_digest".to_string(),
                    )
                })?;
                validate_sha256("result_digest", digest)?;
                if self.error_code.is_some() {
                    return Err(MemoryEngineError::Validation(
                        "successful turn result must not contain error_code".to_string(),
                    ));
                }
            }
            TurnTerminalOutcome::Failed => {
                let error = self.error_code.as_deref().ok_or_else(|| {
                    MemoryEngineError::Validation(
                        "failed turn result requires error_code".to_string(),
                    )
                })?;
                validate_storage_component("error_code", error)?;
                if self.result_reference.is_some() || self.result_digest.is_some() {
                    return Err(MemoryEngineError::Validation(
                        "failed turn result must not contain a success reference".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOperationStatus {
    Accepted,
    ReadyToCommit,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOperation {
    pub schema_version: String,
    pub operation_id: String,
    pub vault_id: VaultId,
    pub account_id: AccountId,
    pub membership_id: String,
    pub membership_revision: u64,
    pub client_request_id: ClientRequestId,
    pub session_id: String,
    pub request_digest: String,
    pub turn_sequence: u64,
    pub status: TurnOperationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_result: Option<TurnTerminalResult>,
    pub created_at: String,
    pub updated_at: String,
}

impl TurnOperation {
    fn validate(&self) -> Result<()> {
        expect_schema(
            "turn operation",
            &self.schema_version,
            TURN_OPERATION_SCHEMA_VERSION,
        )?;
        validate_storage_component("operation_id", &self.operation_id)?;
        validate_storage_component("membership_id", &self.membership_id)?;
        if self.membership_revision == 0 {
            return Err(MemoryEngineError::Validation(
                "turn operation membership_revision must be at least 1".to_string(),
            ));
        }
        validate_storage_component("session_id", &self.session_id)?;
        validate_sha256("request_digest", &self.request_digest)?;
        validate_timestamp("turn operation created_at", &self.created_at)?;
        validate_timestamp("turn operation updated_at", &self.updated_at)?;
        if self.turn_sequence == 0 {
            return Err(MemoryEngineError::Validation(
                "turn_sequence must be at least 1".to_string(),
            ));
        }
        match self.status {
            TurnOperationStatus::Accepted if self.terminal_result.is_some() => {
                Err(MemoryEngineError::Validation(
                    "accepted turn operation must not contain terminal_result".to_string(),
                ))
            }
            TurnOperationStatus::ReadyToCommit | TurnOperationStatus::Committed => self
                .terminal_result
                .as_ref()
                .ok_or_else(|| {
                    MemoryEngineError::Validation(
                        "ready or committed turn operation requires terminal_result".to_string(),
                    )
                })?
                .validate(),
            TurnOperationStatus::Accepted => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnAdmissionKind {
    AcceptedNew,
    Existing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnAdmission {
    pub kind: TurnAdmissionKind,
    pub operation: TurnOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionTurnCursor {
    next_assigned_sequence: u64,
    next_commit_sequence: u64,
}

impl Default for SessionTurnCursor {
    fn default() -> Self {
        Self {
            next_assigned_sequence: 1,
            next_commit_sequence: 1,
        }
    }
}

/// Durable aggregate for request deduplication and per-session commit order.
/// A production backend stores the same semantics transactionally in S2/S3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultTurnLedger {
    pub schema_version: String,
    pub vault_id: VaultId,
    pub account_id: AccountId,
    pub membership_id: String,
    sessions: BTreeMap<String, SessionTurnCursor>,
    operations: BTreeMap<ClientRequestId, TurnOperation>,
}

impl VaultTurnLedger {
    pub fn new(context: &VaultContext) -> Result<Self> {
        context.validate()?;
        Ok(Self {
            schema_version: VAULT_TURN_LEDGER_SCHEMA_VERSION.to_string(),
            vault_id: context.vault_id.clone(),
            account_id: context.account_id.clone(),
            membership_id: context.membership_id.clone(),
            sessions: BTreeMap::new(),
            operations: BTreeMap::new(),
        })
    }

    pub fn begin_turn(
        &mut self,
        context: &VaultContext,
        request: TurnRequest,
    ) -> Result<TurnAdmission> {
        self.validate_context(context)?;
        request.validate()?;

        if let Some(existing) = self.operations.get(&request.client_request_id) {
            if existing.session_id != request.session_id
                || existing.request_digest != request.request_digest
            {
                return Err(MemoryEngineError::Validation(format!(
                    "client_request_id {} was already used for different turn input",
                    request.client_request_id
                )));
            }
            return Ok(TurnAdmission {
                kind: TurnAdmissionKind::Existing,
                operation: existing.clone(),
            });
        }

        let cursor = self.sessions.entry(request.session_id.clone()).or_default();
        if let Some(expected) = request.expected_turn_sequence {
            if expected != cursor.next_assigned_sequence {
                return Err(MemoryEngineError::Validation(format!(
                    "turn sequence conflict for session {}: expected {}, next is {}",
                    request.session_id, expected, cursor.next_assigned_sequence
                )));
            }
        }
        let turn_sequence = cursor.next_assigned_sequence;
        cursor.next_assigned_sequence = cursor
            .next_assigned_sequence
            .checked_add(1)
            .ok_or_else(|| MemoryEngineError::Storage("turn sequence overflow".to_string()))?;

        let operation = TurnOperation {
            schema_version: TURN_OPERATION_SCHEMA_VERSION.to_string(),
            operation_id: deterministic_turn_operation_id(
                &self.vault_id,
                &request.client_request_id,
            ),
            vault_id: self.vault_id.clone(),
            account_id: self.account_id.clone(),
            membership_id: self.membership_id.clone(),
            membership_revision: context.membership_revision,
            client_request_id: request.client_request_id.clone(),
            session_id: request.session_id,
            request_digest: request.request_digest,
            turn_sequence,
            status: TurnOperationStatus::Accepted,
            terminal_result: None,
            created_at: request.submitted_at.clone(),
            updated_at: request.submitted_at,
        };
        operation.validate()?;
        self.operations
            .insert(request.client_request_id, operation.clone());
        Ok(TurnAdmission {
            kind: TurnAdmissionKind::AcceptedNew,
            operation,
        })
    }

    pub fn submit_terminal_result(
        &mut self,
        context: &VaultContext,
        client_request_id: &ClientRequestId,
        result: TurnTerminalResult,
        updated_at: impl Into<String>,
    ) -> Result<TurnOperation> {
        self.validate_context(context)?;
        result.validate()?;
        let updated_at = updated_at.into();
        validate_timestamp("turn result updated_at", &updated_at)?;
        let operation = self.operations.get_mut(client_request_id).ok_or_else(|| {
            MemoryEngineError::Storage(format!(
                "turn operation not found for client_request_id {client_request_id}"
            ))
        })?;

        if operation.status != TurnOperationStatus::Accepted {
            if operation.terminal_result.as_ref() == Some(&result) {
                return Ok(operation.clone());
            }
            return Err(MemoryEngineError::Validation(format!(
                "turn operation {} already has a different terminal result",
                operation.operation_id
            )));
        }
        operation.status = TurnOperationStatus::ReadyToCommit;
        operation.terminal_result = Some(result);
        operation.updated_at = updated_at;
        operation.validate()?;
        Ok(operation.clone())
    }

    pub fn commit_turn(
        &mut self,
        context: &VaultContext,
        client_request_id: &ClientRequestId,
        committed_at: impl Into<String>,
    ) -> Result<TurnOperation> {
        self.validate_context(context)?;
        let committed_at = committed_at.into();
        validate_timestamp("turn committed_at", &committed_at)?;
        let operation = self.operations.get_mut(client_request_id).ok_or_else(|| {
            MemoryEngineError::Storage(format!(
                "turn operation not found for client_request_id {client_request_id}"
            ))
        })?;
        if operation.status == TurnOperationStatus::Committed {
            return Ok(operation.clone());
        }
        if operation.status != TurnOperationStatus::ReadyToCommit {
            return Err(MemoryEngineError::Validation(format!(
                "turn operation {} has no terminal result to commit",
                operation.operation_id
            )));
        }
        let cursor = self
            .sessions
            .get_mut(&operation.session_id)
            .ok_or_else(|| {
                MemoryEngineError::Storage(format!(
                    "turn cursor missing for session {}",
                    operation.session_id
                ))
            })?;
        if operation.turn_sequence != cursor.next_commit_sequence {
            return Err(MemoryEngineError::Validation(format!(
                "turn {} cannot commit before sequence {} in session {}",
                operation.turn_sequence, cursor.next_commit_sequence, operation.session_id
            )));
        }
        operation.status = TurnOperationStatus::Committed;
        operation.updated_at = committed_at;
        cursor.next_commit_sequence =
            cursor.next_commit_sequence.checked_add(1).ok_or_else(|| {
                MemoryEngineError::Storage("turn commit sequence overflow".to_string())
            })?;
        operation.validate()?;
        Ok(operation.clone())
    }

    pub fn operation(&self, client_request_id: &ClientRequestId) -> Option<&TurnOperation> {
        self.operations.get(client_request_id)
    }

    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "vault turn ledger",
            &self.schema_version,
            VAULT_TURN_LEDGER_SCHEMA_VERSION,
        )?;
        validate_storage_component("membership_id", &self.membership_id)?;

        let mut sequences: BTreeMap<&str, BTreeSet<u64>> = BTreeMap::new();
        for (request_id, operation) in &self.operations {
            operation.validate()?;
            if request_id != &operation.client_request_id {
                return Err(MemoryEngineError::Storage(
                    "turn ledger request key does not match operation".to_string(),
                ));
            }
            if operation.vault_id != self.vault_id
                || operation.account_id != self.account_id
                || operation.membership_id != self.membership_id
            {
                return Err(MemoryEngineError::Storage(
                    "turn operation escaped its bound vault context".to_string(),
                ));
            }
            if operation.operation_id
                != deterministic_turn_operation_id(&self.vault_id, &operation.client_request_id)
            {
                return Err(MemoryEngineError::Storage(
                    "turn operation_id does not match its vault/request identity".to_string(),
                ));
            }
            if !self.sessions.contains_key(&operation.session_id) {
                return Err(MemoryEngineError::Storage(format!(
                    "turn cursor missing for session {}",
                    operation.session_id
                )));
            }
            if !sequences
                .entry(operation.session_id.as_str())
                .or_default()
                .insert(operation.turn_sequence)
            {
                return Err(MemoryEngineError::Storage(format!(
                    "duplicate turn sequence {} in session {}",
                    operation.turn_sequence, operation.session_id
                )));
            }
        }

        for (session_id, cursor) in &self.sessions {
            validate_storage_component("session_id", session_id)?;
            if cursor.next_assigned_sequence == 0 || cursor.next_commit_sequence == 0 {
                return Err(MemoryEngineError::Storage(format!(
                    "invalid zero turn cursor for session {session_id}"
                )));
            }
            if cursor.next_commit_sequence > cursor.next_assigned_sequence {
                return Err(MemoryEngineError::Storage(format!(
                    "commit cursor exceeds assigned cursor for session {session_id}"
                )));
            }
            let session_operations: Vec<&TurnOperation> = self
                .operations
                .values()
                .filter(|operation| operation.session_id == *session_id)
                .collect();
            let max_sequence = session_operations
                .iter()
                .map(|operation| operation.turn_sequence)
                .max()
                .unwrap_or(0);
            if cursor.next_assigned_sequence != max_sequence + 1 {
                return Err(MemoryEngineError::Storage(format!(
                    "assigned cursor does not follow stored operations for session {session_id}"
                )));
            }
            for operation in session_operations {
                let should_be_committed = operation.turn_sequence < cursor.next_commit_sequence;
                if should_be_committed != (operation.status == TurnOperationStatus::Committed) {
                    return Err(MemoryEngineError::Storage(format!(
                        "non-contiguous committed turn state in session {session_id}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_context(&self, context: &VaultContext) -> Result<()> {
        context.validate()?;
        if context.vault_id != self.vault_id
            || context.account_id != self.account_id
            || context.membership_id != self.membership_id
        {
            return Err(MemoryEngineError::Validation(
                "vault context does not match the bound turn ledger".to_string(),
            ));
        }
        if context.membership_revision == 0 {
            return Err(MemoryEngineError::Validation(
                "vault context membership revision is invalid".to_string(),
            ));
        }
        Ok(())
    }
}

/// Single-process/reference binding for local development and deterministic
/// S1 tests. It creates a dedicated directory per vault and never falls back
/// to a shared user. Production replicas must use the S2/S3 transactional
/// implementation instead of sharing this handle across processes.
#[derive(Debug)]
pub struct LocalVaultHandle {
    context: VaultContext,
    vault_root: PathBuf,
    memory_storage: FileStorage,
    ledger_path: PathBuf,
    turn_lock: Mutex<()>,
}

impl LocalVaultHandle {
    pub fn open(
        runtime_root: impl AsRef<Path>,
        context: VaultContext,
        host_id: impl Into<String>,
    ) -> Result<Self> {
        context.validate()?;
        let host_id = host_id.into();
        validate_storage_component("host_id", &host_id)?;
        let vault_root = runtime_root
            .as_ref()
            .join("vaults")
            .join(context.vault_id.as_str());
        let memory_root = vault_root.join("memory");
        let runtime_dir = vault_root.join("runtime");
        fs::create_dir_all(&runtime_dir)?;
        let memory_storage = FileStorage::with_host_id(&memory_root, host_id);
        memory_storage.ensure_layout()?;
        let handle = Self {
            context,
            vault_root,
            memory_storage,
            ledger_path: runtime_dir.join("turn_ledger.json"),
            turn_lock: Mutex::new(()),
        };
        {
            let _guard = handle.lock_turns()?;
            if handle.ledger_path.exists() {
                handle
                    .read_ledger_unlocked()?
                    .validate_context(&handle.context)?;
            } else {
                let ledger = VaultTurnLedger::new(&handle.context)?;
                handle.write_ledger_unlocked(&ledger)?;
            }
        }
        Ok(handle)
    }

    pub fn context(&self) -> &VaultContext {
        &self.context
    }

    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    pub fn memory_storage(&self) -> &FileStorage {
        &self.memory_storage
    }

    pub fn begin_turn(&self, request: TurnRequest) -> Result<TurnAdmission> {
        let _guard = self.lock_turns()?;
        let mut ledger = self.read_ledger_unlocked()?;
        let admission = ledger.begin_turn(&self.context, request)?;
        if admission.kind == TurnAdmissionKind::AcceptedNew {
            self.write_ledger_unlocked(&ledger)?;
        }
        Ok(admission)
    }

    pub fn submit_terminal_result(
        &self,
        client_request_id: &ClientRequestId,
        result: TurnTerminalResult,
        updated_at: impl Into<String>,
    ) -> Result<TurnOperation> {
        let _guard = self.lock_turns()?;
        let mut ledger = self.read_ledger_unlocked()?;
        let operation =
            ledger.submit_terminal_result(&self.context, client_request_id, result, updated_at)?;
        self.write_ledger_unlocked(&ledger)?;
        Ok(operation)
    }

    pub fn commit_turn(
        &self,
        client_request_id: &ClientRequestId,
        committed_at: impl Into<String>,
    ) -> Result<TurnOperation> {
        let _guard = self.lock_turns()?;
        let mut ledger = self.read_ledger_unlocked()?;
        let operation = ledger.commit_turn(&self.context, client_request_id, committed_at)?;
        self.write_ledger_unlocked(&ledger)?;
        Ok(operation)
    }

    pub fn load_turn_ledger(&self) -> Result<VaultTurnLedger> {
        let _guard = self.lock_turns()?;
        self.read_ledger_unlocked()
    }

    fn lock_turns(&self) -> Result<MutexGuard<'_, ()>> {
        self.turn_lock.lock().map_err(|_| {
            MemoryEngineError::Storage("local vault turn lock was poisoned".to_string())
        })
    }

    fn read_ledger_unlocked(&self) -> Result<VaultTurnLedger> {
        let content = fs::read_to_string(&self.ledger_path)?;
        let ledger: VaultTurnLedger = serde_json::from_str(&content)?;
        ledger.validate()?;
        ledger.validate_context(&self.context)?;
        Ok(ledger)
    }

    fn write_ledger_unlocked(&self, ledger: &VaultTurnLedger) -> Result<()> {
        ledger.validate()?;
        ledger.validate_context(&self.context)?;
        let content = format!("{}\n", serde_json::to_string_pretty(ledger)?);
        atomic_write(&self.ledger_path, content.as_bytes())
    }
}

fn deterministic_turn_operation_id(
    vault_id: &VaultId,
    client_request_id: &ClientRequestId,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(vault_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(client_request_id.as_str().as_bytes());
    format!("turn_{:x}", hasher.finalize())
}

fn validate_opaque_id(label: &str, value: &str) -> Result<()> {
    if !(8..=128).contains(&value.len()) {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must be 8..=128 characters"
        )));
    }
    if value == "."
        || value == ".."
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(MemoryEngineError::Validation(format!(
            "{label} contains unsafe characters"
        )));
    }
    Ok(())
}

fn validate_storage_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 160 {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must be 1..=160 characters"
        )));
    }
    if value == "."
        || value == ".."
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(MemoryEngineError::Validation(format!(
            "{label} contains unsafe characters"
        )));
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must start with sha256:"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must contain 64 hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_timestamp(label: &str, value: &str) -> Result<()> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
        MemoryEngineError::Validation(format!("{label} must be RFC3339: {error}"))
    })?;
    Ok(())
}

fn expect_schema(label: &str, actual: &str, expected: &str) -> Result<()> {
    if actual != expected {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    if actual.is_empty() {
        return Err(MemoryEngineError::Validation(format!(
            "{label} schema_version must not be empty"
        )));
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let counter = TURN_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("turn_ledger.json");
    let temporary = path.with_file_name(format!(".{name}.{}.{counter}.tmp", std::process::id()));
    {
        let mut file = File::create(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}
