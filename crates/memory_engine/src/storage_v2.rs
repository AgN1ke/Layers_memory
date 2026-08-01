//! Transaction-capable, vault-bound storage contracts.
//!
//! Storage v2 is additive while the existing [`crate::storage::Storage`]
//! implementations migrate one domain transition at a time. A work unit is a
//! short deterministic read/validate/write boundary. LLM, embedding, network,
//! or other provider work must happen before or after it, never inside the
//! retry closure.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::server_runtime::{AccountId, VaultContext, VaultId};
use crate::{MemoryEngineError, Result};

pub const VAULT_RECORD_SCHEMA_VERSION: &str = "vault_record.v1";
pub const WORK_UNIT_INTENT_SCHEMA_VERSION: &str = "storage_work_unit_intent.v1";
pub const WORK_UNIT_RECEIPT_SCHEMA_VERSION: &str = "storage_work_unit_receipt.v1";
pub const DEFAULT_WORK_UNIT_MAX_ATTEMPTS: usize = 4;

pub type StorageV2Result<T> = std::result::Result<T, StorageV2Error>;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageV2Error {
    #[error("storage conflict: {0}")]
    Conflict(String),

    #[error(transparent)]
    Engine(#[from] MemoryEngineError),
}

impl From<serde_json::Error> for StorageV2Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Engine(MemoryEngineError::Json(error))
    }
}

impl From<StorageV2Error> for MemoryEngineError {
    fn from(error: StorageV2Error) -> Self {
        match error {
            StorageV2Error::Conflict(message) => {
                Self::Storage(format!("retryable storage conflict: {message}"))
            }
            StorageV2Error::Engine(error) => error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RecordKey {
    pub collection: String,
    pub record_id: String,
}

impl RecordKey {
    pub fn new(collection: impl Into<String>, record_id: impl Into<String>) -> Result<Self> {
        let key = Self {
            collection: collection.into(),
            record_id: record_id.into(),
        };
        key.validate()?;
        Ok(key)
    }

    pub fn validate(&self) -> Result<()> {
        validate_component("record collection", &self.collection)?;
        validate_component("record id", &self.record_id)
    }

    pub fn reference(&self) -> String {
        format!("{}:{}", self.collection, self.record_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultRecord {
    pub schema_version: String,
    pub vault_id: VaultId,
    pub key: RecordKey,
    pub payload_schema_version: String,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
    pub payload: Value,
}

impl VaultRecord {
    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "vault record",
            &self.schema_version,
            VAULT_RECORD_SCHEMA_VERSION,
        )?;
        self.key.validate()?;
        validate_component("payload schema version", &self.payload_schema_version)?;
        if self.revision == 0 {
            return Err(MemoryEngineError::Validation(
                "vault record revision must be at least 1".to_string(),
            ));
        }
        validate_timestamp("vault record created_at", &self.created_at)?;
        validate_timestamp("vault record updated_at", &self.updated_at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "expect", content = "revision", rename_all = "snake_case")]
pub enum ExpectedRevision {
    Absent,
    Exact(u64),
}

impl ExpectedRevision {
    fn validate(self) -> Result<()> {
        if matches!(self, Self::Exact(0)) {
            return Err(MemoryEngineError::Validation(
                "expected revision must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkUnitIntent {
    pub schema_version: String,
    pub operation_id: String,
    pub operation_kind: String,
    pub request_digest: String,
    pub started_at: String,
}

impl WorkUnitIntent {
    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "storage work-unit intent",
            &self.schema_version,
            WORK_UNIT_INTENT_SCHEMA_VERSION,
        )?;
        validate_component("operation id", &self.operation_id)?;
        validate_component("operation kind", &self.operation_kind)?;
        validate_sha256("work-unit request digest", &self.request_digest)?;
        validate_timestamp("work-unit started_at", &self.started_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkUnitCommit {
    pub committed_at: String,
    pub result_reference: String,
    pub result_digest: String,
}

impl WorkUnitCommit {
    pub fn validate(&self) -> Result<()> {
        validate_timestamp("work-unit committed_at", &self.committed_at)?;
        validate_component("work-unit result reference", &self.result_reference)?;
        validate_sha256("work-unit result digest", &self.result_digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkUnitReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub operation_kind: String,
    pub vault_id: VaultId,
    pub account_id: AccountId,
    pub membership_id: String,
    pub membership_revision: u64,
    pub request_digest: String,
    pub result_reference: String,
    pub result_digest: String,
    pub mutation_count: usize,
    pub commit_sequence: u64,
    pub committed_at: String,
}

impl WorkUnitReceipt {
    pub fn validate(&self) -> Result<()> {
        expect_schema(
            "storage work-unit receipt",
            &self.schema_version,
            WORK_UNIT_RECEIPT_SCHEMA_VERSION,
        )?;
        validate_component("operation id", &self.operation_id)?;
        validate_component("operation kind", &self.operation_kind)?;
        validate_component("membership id", &self.membership_id)?;
        validate_sha256("work-unit request digest", &self.request_digest)?;
        validate_component("work-unit result reference", &self.result_reference)?;
        validate_sha256("work-unit result digest", &self.result_digest)?;
        validate_timestamp("work-unit committed_at", &self.committed_at)?;
        if self.membership_revision == 0 || self.commit_sequence == 0 {
            return Err(MemoryEngineError::Validation(
                "work-unit receipt revisions must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum BeginWorkUnit<W> {
    Started(W),
    AlreadyCommitted(Box<WorkUnitReceipt>),
}

pub trait StorageWorkUnit {
    fn get(&mut self, key: &RecordKey) -> StorageV2Result<Option<VaultRecord>>;

    fn put(
        &mut self,
        key: RecordKey,
        expected: ExpectedRevision,
        payload_schema_version: String,
        payload: Value,
    ) -> StorageV2Result<()>;

    fn delete(&mut self, key: RecordKey, expected: ExpectedRevision) -> StorageV2Result<()>;

    fn commit(self, commit: WorkUnitCommit) -> StorageV2Result<WorkUnitReceipt>;
}

pub trait TransactionalStorage {
    type WorkUnit<'a>: StorageWorkUnit
    where
        Self: 'a;

    fn begin_work_unit<'a>(
        &'a self,
        context: &VaultContext,
        intent: WorkUnitIntent,
    ) -> StorageV2Result<BeginWorkUnit<Self::WorkUnit<'a>>>;

    fn read_record(
        &self,
        context: &VaultContext,
        key: &RecordKey,
    ) -> StorageV2Result<Option<VaultRecord>>;
}

/// Runs only deterministic storage logic and retries the complete work unit on
/// optimistic conflict. Never perform provider or network work in `apply`.
pub fn execute_work_unit_with_retry<S, F>(
    storage: &S,
    context: &VaultContext,
    intent: WorkUnitIntent,
    max_attempts: usize,
    mut apply: F,
) -> StorageV2Result<WorkUnitReceipt>
where
    S: TransactionalStorage,
    F: for<'a> FnMut(&mut S::WorkUnit<'a>) -> StorageV2Result<WorkUnitCommit>,
{
    if max_attempts == 0 {
        return Err(MemoryEngineError::Validation(
            "work-unit max_attempts must be at least 1".to_string(),
        )
        .into());
    }
    intent.validate()?;
    let mut last_conflict = None;
    for _ in 0..max_attempts {
        match storage.begin_work_unit(context, intent.clone())? {
            BeginWorkUnit::AlreadyCommitted(receipt) => return Ok(*receipt),
            BeginWorkUnit::Started(mut work_unit) => {
                let commit = match apply(&mut work_unit) {
                    Ok(commit) => commit,
                    Err(StorageV2Error::Conflict(message)) => {
                        last_conflict = Some(message);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                match work_unit.commit(commit) {
                    Ok(receipt) => return Ok(receipt),
                    Err(StorageV2Error::Conflict(message)) => {
                        last_conflict = Some(message);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Err(StorageV2Error::Conflict(format!(
        "work unit {} exhausted {max_attempts} attempt(s): {}",
        intent.operation_id,
        last_conflict.unwrap_or_else(|| "unknown conflict".to_string())
    )))
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryTransactionalStorage {
    inner: Arc<InMemoryInner>,
}

#[derive(Debug, Default)]
struct InMemoryInner {
    vaults: RwLock<BTreeMap<String, VaultState>>,
    fail_before_publish: AtomicU64,
}

#[derive(Debug, Clone, Default)]
struct VaultState {
    binding: Option<VaultBinding>,
    records: BTreeMap<RecordKey, VaultRecord>,
    receipts: BTreeMap<String, WorkUnitReceipt>,
    next_commit_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultBinding {
    account_id: AccountId,
    membership_id: String,
}

impl InMemoryTransactionalStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deterministic test hook: the next commit builds and validates its full
    /// replacement state, then fails before publishing any mutation.
    pub fn inject_commit_failure_once(&self) {
        self.inner
            .fail_before_publish
            .fetch_add(1, Ordering::SeqCst);
    }

    pub fn committed_receipt(
        &self,
        context: &VaultContext,
        operation_id: &str,
    ) -> StorageV2Result<Option<WorkUnitReceipt>> {
        context.validate()?;
        validate_component("operation id", operation_id)?;
        let vaults = self.read_vaults()?;
        let Some(state) = vaults.get(context.vault_id.as_str()) else {
            return Ok(None);
        };
        validate_binding(context, state)?;
        Ok(state.receipts.get(operation_id).cloned())
    }

    pub fn record_count(&self, context: &VaultContext) -> StorageV2Result<usize> {
        context.validate()?;
        let vaults = self.read_vaults()?;
        let Some(state) = vaults.get(context.vault_id.as_str()) else {
            return Ok(0);
        };
        validate_binding(context, state)?;
        Ok(state.records.len())
    }

    fn read_vaults(&self) -> StorageV2Result<RwLockReadGuard<'_, BTreeMap<String, VaultState>>> {
        self.inner.vaults.read().map_err(|_| {
            MemoryEngineError::Storage("storage v2 read lock was poisoned".into()).into()
        })
    }

    fn write_vaults(&self) -> StorageV2Result<RwLockWriteGuard<'_, BTreeMap<String, VaultState>>> {
        self.inner.vaults.write().map_err(|_| {
            MemoryEngineError::Storage("storage v2 write lock was poisoned".into()).into()
        })
    }
}

impl TransactionalStorage for InMemoryTransactionalStorage {
    type WorkUnit<'a> = InMemoryWorkUnit<'a>;

    fn begin_work_unit<'a>(
        &'a self,
        context: &VaultContext,
        intent: WorkUnitIntent,
    ) -> StorageV2Result<BeginWorkUnit<Self::WorkUnit<'a>>> {
        context.validate()?;
        intent.validate()?;
        let snapshot = {
            let vaults = self.read_vaults()?;
            match vaults.get(context.vault_id.as_str()) {
                Some(state) => {
                    validate_binding(context, state)?;
                    if let Some(receipt) = state.receipts.get(&intent.operation_id) {
                        validate_replay(&intent, receipt)?;
                        return Ok(BeginWorkUnit::AlreadyCommitted(Box::new(receipt.clone())));
                    }
                    state.records.clone()
                }
                None => BTreeMap::new(),
            }
        };
        Ok(BeginWorkUnit::Started(InMemoryWorkUnit {
            storage: self,
            context: context.clone(),
            intent,
            snapshot,
            observed: BTreeMap::new(),
            staged: BTreeMap::new(),
        }))
    }

    fn read_record(
        &self,
        context: &VaultContext,
        key: &RecordKey,
    ) -> StorageV2Result<Option<VaultRecord>> {
        context.validate()?;
        key.validate()?;
        let vaults = self.read_vaults()?;
        let Some(state) = vaults.get(context.vault_id.as_str()) else {
            return Ok(None);
        };
        validate_binding(context, state)?;
        Ok(state.records.get(key).cloned())
    }
}

#[derive(Debug)]
pub struct InMemoryWorkUnit<'a> {
    storage: &'a InMemoryTransactionalStorage,
    context: VaultContext,
    intent: WorkUnitIntent,
    snapshot: BTreeMap<RecordKey, VaultRecord>,
    observed: BTreeMap<RecordKey, Option<u64>>,
    staged: BTreeMap<RecordKey, StagedMutation>,
}

#[derive(Debug, Clone)]
enum StagedMutation {
    Put {
        expected: ExpectedRevision,
        payload_schema_version: String,
        payload: Value,
    },
    Delete {
        expected: ExpectedRevision,
    },
}

impl StorageWorkUnit for InMemoryWorkUnit<'_> {
    fn get(&mut self, key: &RecordKey) -> StorageV2Result<Option<VaultRecord>> {
        key.validate()?;
        if self.staged.contains_key(key) {
            return Err(MemoryEngineError::Validation(format!(
                "record {} cannot be read after it was staged",
                key.reference()
            ))
            .into());
        }
        let value = self.snapshot.get(key).cloned();
        self.observed
            .insert(key.clone(), value.as_ref().map(|record| record.revision));
        Ok(value)
    }

    fn put(
        &mut self,
        key: RecordKey,
        expected: ExpectedRevision,
        payload_schema_version: String,
        payload: Value,
    ) -> StorageV2Result<()> {
        key.validate()?;
        expected.validate()?;
        validate_component("payload schema version", &payload_schema_version)?;
        self.stage_expected(&key, expected)?;
        self.staged.insert(
            key,
            StagedMutation::Put {
                expected,
                payload_schema_version,
                payload,
            },
        );
        Ok(())
    }

    fn delete(&mut self, key: RecordKey, expected: ExpectedRevision) -> StorageV2Result<()> {
        key.validate()?;
        expected.validate()?;
        if expected == ExpectedRevision::Absent {
            return Err(MemoryEngineError::Validation(
                "delete requires an exact existing revision".to_string(),
            )
            .into());
        }
        self.stage_expected(&key, expected)?;
        self.staged.insert(key, StagedMutation::Delete { expected });
        Ok(())
    }

    fn commit(self, commit: WorkUnitCommit) -> StorageV2Result<WorkUnitReceipt> {
        commit.validate()?;
        let mut vaults = self.storage.write_vaults()?;
        let current = vaults
            .get(self.context.vault_id.as_str())
            .cloned()
            .unwrap_or_default();
        validate_binding(&self.context, &current)?;

        if let Some(receipt) = current.receipts.get(&self.intent.operation_id) {
            validate_replay(&self.intent, receipt)?;
            return Ok(receipt.clone());
        }

        for (key, observed) in &self.observed {
            let current_revision = current.records.get(key).map(|record| record.revision);
            if &current_revision != observed {
                return Err(StorageV2Error::Conflict(format!(
                    "record {} changed from {:?} to {:?}",
                    key.reference(),
                    observed,
                    current_revision
                )));
            }
        }

        let mut next = current.clone();
        if next.binding.is_none() {
            next.binding = Some(VaultBinding {
                account_id: self.context.account_id.clone(),
                membership_id: self.context.membership_id.clone(),
            });
        }
        for (key, mutation) in &self.staged {
            match mutation {
                StagedMutation::Put {
                    expected,
                    payload_schema_version,
                    payload,
                } => {
                    ensure_expected_revision(key, *expected, next.records.get(key))?;
                    let previous = next.records.get(key);
                    let revision = match previous {
                        Some(record) => record.revision.checked_add(1).ok_or_else(|| {
                            MemoryEngineError::Storage(format!(
                                "record revision overflow for {}",
                                key.reference()
                            ))
                        })?,
                        None => 1,
                    };
                    let created_at = previous
                        .map(|record| record.created_at.clone())
                        .unwrap_or_else(|| commit.committed_at.clone());
                    let record = VaultRecord {
                        schema_version: VAULT_RECORD_SCHEMA_VERSION.to_string(),
                        vault_id: self.context.vault_id.clone(),
                        key: key.clone(),
                        payload_schema_version: payload_schema_version.clone(),
                        revision,
                        created_at,
                        updated_at: commit.committed_at.clone(),
                        payload: payload.clone(),
                    };
                    record.validate()?;
                    next.records.insert(key.clone(), record);
                }
                StagedMutation::Delete { expected } => {
                    ensure_expected_revision(key, *expected, next.records.get(key))?;
                    next.records.remove(key);
                }
            }
        }

        let commit_sequence = if next.next_commit_sequence == 0 {
            1
        } else {
            next.next_commit_sequence
        };
        next.next_commit_sequence = commit_sequence
            .checked_add(1)
            .ok_or_else(|| MemoryEngineError::Storage("commit sequence overflow".to_string()))?;
        let receipt = WorkUnitReceipt {
            schema_version: WORK_UNIT_RECEIPT_SCHEMA_VERSION.to_string(),
            operation_id: self.intent.operation_id,
            operation_kind: self.intent.operation_kind,
            vault_id: self.context.vault_id.clone(),
            account_id: self.context.account_id.clone(),
            membership_id: self.context.membership_id.clone(),
            membership_revision: self.context.membership_revision,
            request_digest: self.intent.request_digest,
            result_reference: commit.result_reference,
            result_digest: commit.result_digest,
            mutation_count: self.staged.len(),
            commit_sequence,
            committed_at: commit.committed_at,
        };
        receipt.validate()?;
        next.receipts
            .insert(receipt.operation_id.clone(), receipt.clone());

        if self
            .storage
            .inner
            .fail_before_publish
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                if remaining > 0 {
                    Some(remaining - 1)
                } else {
                    None
                }
            })
            .is_ok()
        {
            return Err(MemoryEngineError::Storage(
                "injected storage v2 failure before atomic publish".to_string(),
            )
            .into());
        }

        vaults.insert(self.context.vault_id.as_str().to_string(), next);
        Ok(receipt)
    }
}

impl InMemoryWorkUnit<'_> {
    fn stage_expected(
        &mut self,
        key: &RecordKey,
        expected: ExpectedRevision,
    ) -> StorageV2Result<()> {
        if self.staged.contains_key(key) {
            return Err(MemoryEngineError::Validation(format!(
                "record {} already has a staged mutation",
                key.reference()
            ))
            .into());
        }
        ensure_expected_revision(key, expected, self.snapshot.get(key))?;
        self.observed.insert(
            key.clone(),
            self.snapshot.get(key).map(|record| record.revision),
        );
        Ok(())
    }
}

fn ensure_expected_revision(
    key: &RecordKey,
    expected: ExpectedRevision,
    record: Option<&VaultRecord>,
) -> StorageV2Result<()> {
    let actual = record.map(|record| record.revision);
    let matches = match expected {
        ExpectedRevision::Absent => actual.is_none(),
        ExpectedRevision::Exact(revision) => actual == Some(revision),
    };
    if matches {
        Ok(())
    } else {
        Err(StorageV2Error::Conflict(format!(
            "record {} expected {:?}, found {:?}",
            key.reference(),
            expected,
            actual
        )))
    }
}

fn validate_binding(context: &VaultContext, state: &VaultState) -> StorageV2Result<()> {
    if let Some(binding) = &state.binding {
        if binding.account_id != context.account_id
            || binding.membership_id != context.membership_id
        {
            return Err(MemoryEngineError::Validation(
                "vault storage is bound to a different account membership".to_string(),
            )
            .into());
        }
    }
    Ok(())
}

fn validate_replay(intent: &WorkUnitIntent, receipt: &WorkUnitReceipt) -> StorageV2Result<()> {
    receipt.validate()?;
    if receipt.operation_kind != intent.operation_kind
        || receipt.request_digest != intent.request_digest
    {
        return Err(MemoryEngineError::Validation(format!(
            "operation id {} was already committed for different input",
            intent.operation_id
        ))
        .into());
    }
    Ok(())
}

fn validate_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 200 {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must be 1..=200 characters"
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
