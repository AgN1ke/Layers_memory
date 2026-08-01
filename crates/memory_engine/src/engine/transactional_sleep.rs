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

pub const TRANSACTIONAL_SLEEP_START_REQUEST_SCHEMA_VERSION: &str =
    "transactional_sleep_start_request.v1";
pub const TRANSACTIONAL_SLEEP_START_RESULT_SCHEMA_VERSION: &str =
    "transactional_sleep_start_result.v1";
pub const SLEEP_START_WORK_UNIT_KIND: &str = "sleep_start";

const SESSION_COLLECTION: &str = "sessions";
// Storage v2 domain invariant: session events are append-only records created
// with ExpectedRevision::Absent by transactional ingest. Sleep freezes their
// full payloads into its tasks; later workflow stages must never rewrite them.
const EVENT_COLLECTION: &str = "session_events";
const ARCHIVE_COLLECTION: &str = "archives";
const TASK_COLLECTION: &str = "tasks";
const SLEEP_RUN_COLLECTION: &str = "sleep_runs";
const OPERATION_RESULT_COLLECTION: &str = "operation_results";
const SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION: &str = "sleep_start_operation_result.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepStartRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub session_id: String,
    pub created_at: String,
    /// Frozen cognitive policy. The engine convenience constructor snapshots
    /// EngineOptions; the low-level constructor accepts trusted runtime input.
    /// Either form survives retry/deployment boundaries unchanged.
    pub policy: SleepStage1Config,
}

impl TransactionalSleepStartRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        created_at: impl Into<String>,
        session_id: impl Into<String>,
        policy: SleepStage1Config,
    ) -> Result<Self> {
        let session_id = session_id.into();
        let created_at = created_at.into();
        let request_digest = sleep_start_request_digest(&session_id, &created_at, &policy)?;
        let request = Self {
            schema_version: TRANSACTIONAL_SLEEP_START_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: WorkUnitIntent {
                schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
                operation_id: operation_id.into(),
                operation_kind: SLEEP_START_WORK_UNIT_KIND.to_string(),
                request_digest,
                started_at: started_at.into(),
            },
            session_id,
            created_at,
            policy,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != TRANSACTIONAL_SLEEP_START_REQUEST_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: TRANSACTIONAL_SLEEP_START_REQUEST_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        self.work_unit.validate()?;
        if self.work_unit.operation_kind != SLEEP_START_WORK_UNIT_KIND {
            return Err(MemoryEngineError::Validation(format!(
                "transactional sleep start requires operation_kind {SLEEP_START_WORK_UNIT_KIND}"
            )));
        }
        if self.session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "transactional sleep start session_id must not be empty".to_string(),
            ));
        }
        if parse_rfc3339(&self.created_at).is_none() {
            return Err(MemoryEngineError::Validation(
                "transactional sleep start created_at must be RFC3339".to_string(),
            ));
        }
        validate_sleep_policy(&self.policy)?;
        let expected =
            sleep_start_request_digest(&self.session_id, &self.created_at, &self.policy)?;
        if self.work_unit.request_digest != expected {
            return Err(MemoryEngineError::Validation(
                "transactional sleep start request_digest does not match its frozen input"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepStartResult {
    pub schema_version: String,
    pub run: SleepRun,
    pub receipt: WorkUnitReceipt,
}

/// Immutable replay target for the sleep-start operation. `SleepRun` itself is
/// mutable once provider results begin to advance the workflow, so receipts
/// must never point at the live run record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SleepStartOperationResult {
    schema_version: String,
    run: SleepRun,
}

impl<S> MemoryEngine<S> {
    pub fn new_transactional_sleep_start_request(
        &self,
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        created_at: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Result<TransactionalSleepStartRequest> {
        TransactionalSleepStartRequest::new(
            operation_id,
            started_at,
            created_at,
            session_id,
            self.options.sleep.clone(),
        )
    }
}

impl<S: TransactionalStorage> MemoryEngine<S> {
    /// Atomically reserves live events and publishes preliminary Archive,
    /// reasoning tasks and the durable SleepRun. No provider call occurs here.
    pub fn begin_sleep_run_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalSleepStartRequest,
    ) -> Result<TransactionalSleepStartResult> {
        context.validate()?;
        request.validate()?;

        let session_key = RecordKey::new(SESSION_COLLECTION, request.session_id.clone())?;
        let archive_id = deterministic_sleep_id(context, &request.work_unit, "archive", "archive");
        let sleep_task_id =
            deterministic_sleep_id(context, &request.work_unit, "sleep_task", "task");
        let memory_unit_task_id =
            deterministic_sleep_id(context, &request.work_unit, "memory_unit_task", "task");
        let archive_key = RecordKey::new(ARCHIVE_COLLECTION, archive_id.clone())?;
        let sleep_task_key = RecordKey::new(TASK_COLLECTION, sleep_task_id.clone())?;
        let memory_task_key = RecordKey::new(TASK_COLLECTION, memory_unit_task_id.clone())?;
        let run_key = RecordKey::new(SLEEP_RUN_COLLECTION, sleep_task_id.clone())?;
        let result_key = RecordKey::new(
            OPERATION_RESULT_COLLECTION,
            request.work_unit.operation_id.clone(),
        )?;
        let request_ids = [
            deterministic_sleep_id(
                context,
                &request.work_unit,
                "memory_unit_request",
                "llm_req",
            ),
            deterministic_sleep_id(context, &request.work_unit, "emotional_request", "llm_req"),
            deterministic_sleep_id(context, &request.work_unit, "topic_request", "llm_req"),
            deterministic_sleep_id(context, &request.work_unit, "personal_request", "llm_req"),
            deterministic_sleep_id(context, &request.work_unit, "relational_request", "llm_req"),
        ];
        let session_id = request.session_id;
        let created_at = request.created_at;
        let policy = request.policy;
        let intent = request.work_unit;
        let result_reference = result_key.reference();

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let session_record = work_unit.get(&session_key)?.ok_or_else(|| {
                    MemoryEngineError::Validation(format!(
                        "session not found for transactional sleep: {session_id}"
                    ))
                })?;
                session_record.validate()?;
                if session_record.payload_schema_version != SESSION_SCHEMA_VERSION {
                    return Err(MemoryEngineError::IncompatibleSchema {
                        expected: SESSION_SCHEMA_VERSION.to_string(),
                        actual: session_record.payload_schema_version,
                    }
                    .into());
                }
                let mut metadata: SessionMetadata =
                    serde_json::from_value(session_record.payload.clone())?;
                validate_sleep_session_metadata(&metadata, &session_id)?;
                validate_sleep_start_eligibility(&metadata)?;

                let mut events = Vec::new();
                for record in work_unit.scan_collection(EVENT_COLLECTION)? {
                    record.validate()?;
                    if record.revision != 1 {
                        return Err(MemoryEngineError::Storage(format!(
                            "append-only session event {} must remain at revision 1",
                            record.key.record_id
                        ))
                        .into());
                    }
                    if record.payload_schema_version != EVENT_SCHEMA_VERSION {
                        return Err(MemoryEngineError::IncompatibleSchema {
                            expected: EVENT_SCHEMA_VERSION.to_string(),
                            actual: record.payload_schema_version,
                        }
                        .into());
                    }
                    let event: StoredEvent = serde_json::from_value(record.payload)?;
                    if event.schema_version != EVENT_SCHEMA_VERSION {
                        return Err(MemoryEngineError::IncompatibleSchema {
                            expected: EVENT_SCHEMA_VERSION.to_string(),
                            actual: event.schema_version,
                        }
                        .into());
                    }
                    if record.key.record_id != event.event_id {
                        return Err(MemoryEngineError::Storage(
                            "session event record key does not match payload event_id".to_string(),
                        )
                        .into());
                    }
                    if event.session_id == session_id {
                        events.push(event);
                    }
                }
                for event in &events {
                    if parse_rfc3339(&event.received_at).is_none() {
                        return Err(MemoryEngineError::Storage(format!(
                            "session event {} has invalid received_at",
                            event.event_id
                        ))
                        .into());
                    }
                }
                // File storage preserves append order. Storage v2 reconstructs
                // the same cognitive order from the server-controlled receive
                // time; client timestamps must not move a new turn into the
                // compactable head or an old turn into the protected live tail.
                events.sort_by(compare_sleep_receive_order);
                if events.is_empty() {
                    return Err(MemoryEngineError::Validation(format!(
                        "session has no events: {session_id}"
                    ))
                    .into());
                }
                if metadata.event_count != events.len() as u64 {
                    return Err(MemoryEngineError::Storage(format!(
                        "session {session_id} event_count {} does not match {} immutable event records",
                        metadata.event_count,
                        events.len()
                    ))
                    .into());
                }
                if !metadata.archived_event_index_complete {
                    return Err(MemoryEngineError::Validation(format!(
                        "session {session_id} requires archived-event index repair before transactional sleep"
                    ))
                    .into());
                }
                let archived = metadata
                    .archived_event_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>();
                let unarchived_events = events
                    .iter()
                    .filter(|event| !archived.contains(event.event_id.as_str()))
                    .collect::<Vec<_>>();
                if unarchived_events.is_empty() {
                    return Err(MemoryEngineError::Validation(format!(
                        "session has no unarchived events: {session_id}"
                    ))
                    .into());
                }

                let session = SessionRecord {
                    metadata: metadata.clone(),
                    events: events.clone(),
                };
                let stage1 = prepare_sleep_stage1(
                    &session,
                    &unarchived_events,
                    &policy,
                    &created_at,
                    archive_id.clone(),
                    sleep_task_id.clone(),
                    memory_unit_task_id.clone(),
                )?;
                let archive_entry = stage1.archive_entry.clone();
                let sleep_task = stage1.pending_task.clone();
                let memory_task = stage1.memory_unit_task.clone().ok_or_else(|| {
                    MemoryEngineError::Storage(
                        "shared sleep Stage 1 produced no memory-unit task".to_string(),
                    )
                })?;
                let run = sleep_run_from_stage1_with_request_ids(
                    stage1,
                    request_ids[0].clone(),
                    [
                        request_ids[1].clone(),
                        request_ids[2].clone(),
                        request_ids[3].clone(),
                        request_ids[4].clone(),
                    ],
                )?;
                validate_sleep_run(&run)?;
                let operation_result = SleepStartOperationResult {
                    schema_version: SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    run: run.clone(),
                };
                let result_digest =
                    transactional_support::serializable_digest(&operation_result)?;

                metadata.status = SessionStatus::SleepPending;
                metadata.updated_at = created_at.clone();
                metadata.active_sleep_task_id = Some(sleep_task_id.clone());
                metadata.sleep_pending_event_ids = archive_entry.source_event_ids.clone();
                work_unit.put(
                    session_key.clone(),
                    ExpectedRevision::Exact(session_record.revision),
                    SESSION_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&metadata)?,
                )?;
                work_unit.put(
                    archive_key.clone(),
                    ExpectedRevision::Absent,
                    ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&archive_entry)?,
                )?;
                work_unit.put(
                    sleep_task_key.clone(),
                    ExpectedRevision::Absent,
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&sleep_task)?,
                )?;
                work_unit.put(
                    memory_task_key.clone(),
                    ExpectedRevision::Absent,
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&memory_task)?,
                )?;
                work_unit.put(
                    run_key.clone(),
                    ExpectedRevision::Absent,
                    SLEEP_RUN_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&run)?,
                )?;
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&operation_result)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: created_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let operation_result =
            transactional_support::read_transaction_result::<S, SleepStartOperationResult>(
                &self.storage,
                context,
                &result_key,
                &receipt,
                SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION,
            )?;
        if operation_result.schema_version != SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                actual: operation_result.schema_version,
            });
        }
        validate_sleep_run(&operation_result.run)?;
        Ok(TransactionalSleepStartResult {
            schema_version: TRANSACTIONAL_SLEEP_START_RESULT_SCHEMA_VERSION.to_string(),
            run: operation_result.run,
            receipt,
        })
    }
}

#[derive(Serialize)]
struct SleepStartDigestInput<'a> {
    session_id: &'a str,
    created_at: &'a str,
    policy: &'a SleepStage1Config,
}

fn sleep_start_request_digest(
    session_id: &str,
    created_at: &str,
    policy: &SleepStage1Config,
) -> Result<String> {
    transactional_support::serializable_digest(&SleepStartDigestInput {
        session_id,
        created_at,
        policy,
    })
}

fn validate_sleep_policy(policy: &SleepStage1Config) -> Result<()> {
    if !policy.min_event_weight.is_finite()
        || !policy.active_tail_ratio.is_finite()
        || policy.max_events == 0
        || policy.prompt_id.trim().is_empty()
        || policy.prompt_version == 0
    {
        return Err(MemoryEngineError::Validation(
            "transactional sleep policy is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_sleep_session_metadata(metadata: &SessionMetadata, session_id: &str) -> Result<()> {
    if metadata.schema_version != SESSION_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: SESSION_SCHEMA_VERSION.to_string(),
            actual: metadata.schema_version.clone(),
        });
    }
    if metadata.session_id != session_id {
        return Err(MemoryEngineError::Storage(
            "Session record key does not match payload session_id".to_string(),
        ));
    }
    Ok(())
}

fn validate_sleep_start_eligibility(metadata: &SessionMetadata) -> Result<()> {
    let session_id = &metadata.session_id;
    match metadata.status {
        SessionStatus::Active
            if metadata.active_sleep_task_id.is_none()
                && metadata.sleep_pending_event_ids.is_empty() =>
        {
            Ok(())
        }
        SessionStatus::SleepPending
            if metadata.active_sleep_task_id.is_some()
                && !metadata.sleep_pending_event_ids.is_empty() =>
        {
            Err(MemoryEngineError::Validation(format!(
                "session {session_id} already has active sleep task {}",
                metadata.active_sleep_task_id.as_deref().unwrap_or_default()
            )))
        }
        SessionStatus::Active | SessionStatus::SleepPending => Err(MemoryEngineError::Storage(
            format!("session {session_id} has inconsistent sleep reservation state"),
        )),
        SessionStatus::Archived | SessionStatus::Failed => Err(MemoryEngineError::Validation(
            format!("session {session_id} is not active and cannot start sleep"),
        )),
    }
}

fn deterministic_sleep_id(
    context: &VaultContext,
    intent: &WorkUnitIntent,
    label: &str,
    prefix: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(context.vault_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(intent.operation_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.request_digest.as_bytes());
    hasher.update([0]);
    hasher.update(label.as_bytes());
    format!("{prefix}_{:x}", hasher.finalize())
}
