use super::erasure_flow::scope_erasure_lock_key;
use super::transactional_ingest::INGEST_WORK_UNIT_KIND;
use super::transactional_support::serializable_digest;
use super::*;
use crate::file_storage::{FileStorage, JournaledIngest};
use crate::storage_v2::{WorkUnitIntent, WORK_UNIT_INTENT_SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdempotentIngestResult {
    pub schema_version: String,
    pub event: StoredEvent,
    pub already_applied: bool,
}

impl MemoryEngine<FileStorage> {
    /// Accept one event under a stable operation ID in this logical host.
    /// Reuse that ID and the same input after an ambiguous result.
    pub fn ingest_idempotent(
        &self,
        operation_id: &str,
        event: IngestEvent,
    ) -> Result<IdempotentIngestResult> {
        validate_ingest_event(&event)?;
        let session = event.session_id.clone();
        let scope = event
            .personality_scope_id
            .as_deref()
            .unwrap_or(&session)
            .to_owned();
        self.storage.validate_idempotent_session_id(&session)?;
        let host = self.storage.host_id();
        let digest = serializable_digest(&(INGEST_WORK_UNIT_KIND, host, &event))?;
        let work = WorkUnitIntent {
            schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
            operation_id: operation_id.into(),
            operation_kind: INGEST_WORK_UNIT_KIND.into(),
            request_digest: digest,
            started_at: now_rfc3339()?,
        };
        work.validate()?;
        let key_digest = serializable_digest(&(INGEST_WORK_UNIT_KIND, host, operation_id))?;
        let key = format!("ingest_{}", &key_digest["sha256:".len()..]);
        self.ensure_manifest()?;

        self.with_ingest_locks(&scope, &session, &key, || {
            let (event, already_applied) =
                if let Some(receipt) = self.storage.ingest_receipt(&key)? {
                    if receipt.request_digest != work.request_digest {
                        return Err(MemoryEngineError::OperationConflict);
                    }
                    receipt.require_replayable()?;
                    self.storage
                        .validate_ingest_session(&session, &scope, host)?;
                    let event = self
                        .read_session_events_with_archived(&session)?
                        .into_iter()
                        .find(|candidate| candidate.event_id == receipt.commit.result_reference)
                        .ok_or(MemoryEngineError::OperationResultUnavailable)?;
                    if serializable_digest(&event)? != receipt.commit.result_digest {
                        return Err(MemoryEngineError::OperationResultUnavailable);
                    }
                    (event, true)
                } else {
                    self.storage
                        .validate_ingest_session(&session, &scope, host)?;
                    let event = self.prepare_ingest_event(event)?;
                    if let Err(error) = self.storage.start_journaled_ingest(&key, work, &event) {
                        self.memory_operations_recovered
                            .store(false, Ordering::Release);
                        return Err(error);
                    }
                    (event, false)
                };
            Ok(IdempotentIngestResult {
                schema_version: "idempotent_ingest_result.v1".into(),
                event,
                already_applied,
            })
        })
    }
}

impl<S: Storage> MemoryEngine<S> {
    fn with_ingest_locks<T>(
        &self,
        scope: &str,
        session: &str,
        key: &str,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        // Use a bounded set of admission locks. An operation ID can be
        // presented concurrently in different sessions or scopes.
        let stripe = key
            .as_bytes()
            .last()
            .ok_or_else(|| MemoryEngineError::Storage("empty ingest journal key".to_string()))?;
        self.with_resource_lock(scope_erasure_lock_key(scope), || {
            self.with_resource_lock(session_lock_key(session), || {
                self.with_resource_lock(format!("ingest:{stripe}"), f)
            })
        })
    }

    pub(super) fn recover_ingest_operation(&self, operation: JournalOperation) -> Result<bool> {
        let input: JournaledIngest = serde_json::from_value(operation.intent)?;
        self.with_ingest_locks(
            input.event.effective_personality_scope_id(),
            &input.event.session_id,
            &operation.op_id,
            || {
                // A concurrent recovery or erasure can finish after the scan.
                let Some(current) = self.storage.load_journaled_operation(&operation.op_id)? else {
                    return Ok(false);
                };
                self.storage.recover_journaled_ingest(&current)?;
                Ok(true)
            },
        )
    }
}
