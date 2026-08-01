use serde::Serialize;
use sha2::{Digest, Sha256};

use super::*;
use crate::server_runtime::VaultContext;
use crate::session::{SessionMetadata, SessionStatus};
use crate::storage_v2::{
    execute_work_unit_with_retry, ExpectedRevision, RecordKey, StorageWorkUnit,
    TransactionalStorage, WorkUnitCommit, WorkUnitIntent, WorkUnitReceipt,
    DEFAULT_WORK_UNIT_MAX_ATTEMPTS, WORK_UNIT_INTENT_SCHEMA_VERSION,
};

pub const TRANSACTIONAL_INGEST_REQUEST_SCHEMA_VERSION: &str = "transactional_ingest_request.v1";
pub const TRANSACTIONAL_INGEST_RESULT_SCHEMA_VERSION: &str = "transactional_ingest_result.v1";
pub const INGEST_WORK_UNIT_KIND: &str = "ingest_event";
const SESSION_COLLECTION: &str = "sessions";
const EVENT_COLLECTION: &str = "session_events";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalIngestRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub host_id: String,
    pub received_at: String,
    pub event: IngestEvent,
}

impl TransactionalIngestRequest {
    pub fn new(
        operation_id: impl Into<String>,
        host_id: impl Into<String>,
        started_at: impl Into<String>,
        received_at: impl Into<String>,
        event: IngestEvent,
    ) -> Result<Self> {
        let host_id = host_id.into();
        let started_at = started_at.into();
        let received_at = received_at.into();
        let request_digest = ingest_request_digest(&host_id, &received_at, &event)?;
        let request = Self {
            schema_version: TRANSACTIONAL_INGEST_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: WorkUnitIntent {
                schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
                operation_id: operation_id.into(),
                operation_kind: INGEST_WORK_UNIT_KIND.to_string(),
                request_digest,
                started_at,
            },
            host_id,
            received_at,
            event,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != TRANSACTIONAL_INGEST_REQUEST_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: TRANSACTIONAL_INGEST_REQUEST_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        self.work_unit.validate()?;
        if self.work_unit.operation_kind != INGEST_WORK_UNIT_KIND {
            return Err(MemoryEngineError::Validation(format!(
                "transactional ingest requires operation_kind {INGEST_WORK_UNIT_KIND}"
            )));
        }
        validate_ingest_component("host_id", &self.host_id)?;
        OffsetDateTime::parse(&self.received_at, &Rfc3339).map_err(|error| {
            MemoryEngineError::Validation(format!(
                "transactional ingest received_at must be RFC3339: {error}"
            ))
        })?;
        self.event.validate()?;
        let expected_digest = ingest_request_digest(&self.host_id, &self.received_at, &self.event)?;
        if self.work_unit.request_digest != expected_digest {
            return Err(MemoryEngineError::Validation(
                "transactional ingest request_digest does not match its domain input".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalIngestResult {
    pub schema_version: String,
    pub ingest: IngestResult,
    pub receipt: WorkUnitReceipt,
}

impl<S: TransactionalStorage> MemoryEngine<S> {
    /// Atomically writes one stored event and its session metadata.
    ///
    /// Existing `ingest` on Storage v1 remains available during migration.
    /// Server runtimes use this operation with a server-derived VaultContext.
    pub fn ingest_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalIngestRequest,
    ) -> Result<TransactionalIngestResult> {
        context.validate()?;
        request.validate()?;

        let event_id = deterministic_event_id(context, &request.work_unit);
        let (initial_weight, weight_reason) = self
            .options
            .event_scoring
            .score_ingest_event(&request.event);
        let stored_event = StoredEvent::from_ingest(
            request.event.clone(),
            event_id,
            request.received_at.clone(),
            initial_weight,
            weight_reason,
        );
        let ingest = IngestResult {
            schema_version: INGEST_RESULT_SCHEMA_VERSION.to_string(),
            stored_event: stored_event.clone(),
        };
        let result_digest = serializable_digest(&ingest)?;
        let session_key = RecordKey::new(SESSION_COLLECTION, stored_event.session_id.clone())?;
        let event_key = RecordKey::new(EVENT_COLLECTION, stored_event.event_id.clone())?;
        let host_id = request.host_id.clone();
        let committed_at = request.received_at.clone();
        let intent = request.work_unit;
        let result_reference = event_key.reference();

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let existing_session = work_unit.get(&session_key)?;
                let (mut metadata, expected_session) = match existing_session {
                    Some(record) => {
                        record.validate()?;
                        let metadata: SessionMetadata = serde_json::from_value(record.payload)?;
                        if metadata.session_id != stored_event.session_id {
                            return Err(MemoryEngineError::Storage(
                                "session record key does not match payload session_id".to_string(),
                            )
                            .into());
                        }
                        if metadata.host_id != host_id {
                            return Err(MemoryEngineError::Validation(format!(
                                "session {} belongs to logical host {}, not {}",
                                metadata.session_id, metadata.host_id, host_id
                            ))
                            .into());
                        }
                        (metadata, ExpectedRevision::Exact(record.revision))
                    }
                    None => (
                        SessionMetadata {
                            schema_version: SESSION_SCHEMA_VERSION.to_string(),
                            session_id: stored_event.session_id.clone(),
                            host_id: host_id.clone(),
                            status: SessionStatus::Active,
                            created_at: stored_event.received_at.clone(),
                            updated_at: stored_event.received_at.clone(),
                            closed_at: None,
                            event_count: 0,
                            summary: None,
                            active_theme: stored_event.theme.clone(),
                            tags: Vec::new(),
                            archived_to: Vec::new(),
                            archived_event_ids: Vec::new(),
                            archived_event_index_complete: true,
                            notes: Vec::new(),
                        },
                        ExpectedRevision::Absent,
                    ),
                };

                metadata.event_count = metadata.event_count.checked_add(1).ok_or_else(|| {
                    MemoryEngineError::Storage("session event_count overflow".to_string())
                })?;
                metadata.updated_at = stored_event.received_at.clone();
                if metadata.active_theme.is_none() {
                    metadata.active_theme = stored_event.theme.clone();
                }
                for tag in &stored_event.tags {
                    if !metadata.tags.contains(tag) {
                        metadata.tags.push(tag.clone());
                    }
                }

                work_unit.put(
                    event_key.clone(),
                    ExpectedRevision::Absent,
                    EVENT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&stored_event)?,
                )?;
                work_unit.put(
                    session_key.clone(),
                    expected_session,
                    SESSION_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&metadata)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: committed_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest: result_digest.clone(),
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        if receipt.result_reference != result_reference || receipt.result_digest != result_digest {
            return Err(MemoryEngineError::Storage(
                "committed ingest receipt does not match deterministic result".to_string(),
            ));
        }
        Ok(TransactionalIngestResult {
            schema_version: TRANSACTIONAL_INGEST_RESULT_SCHEMA_VERSION.to_string(),
            ingest,
            receipt,
        })
    }
}

#[derive(Serialize)]
struct IngestDigestInput<'a> {
    host_id: &'a str,
    received_at: &'a str,
    event: &'a IngestEvent,
}

fn ingest_request_digest(host_id: &str, received_at: &str, event: &IngestEvent) -> Result<String> {
    serializable_digest(&IngestDigestInput {
        host_id,
        received_at,
        event,
    })
}

pub(super) fn serializable_digest<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn deterministic_event_id(context: &VaultContext, intent: &WorkUnitIntent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(context.vault_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(intent.operation_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.request_digest.as_bytes());
    format!("event_{:x}", hasher.finalize())
}

fn validate_ingest_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || value == "."
        || value == ".."
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(MemoryEngineError::Validation(format!(
            "transactional ingest {label} contains unsafe characters"
        )));
    }
    Ok(())
}
