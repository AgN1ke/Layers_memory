use serde::Serialize;
use sha2::{Digest, Sha256};

use super::*;
use crate::server_runtime::VaultContext;
use crate::session::{SessionMetadata, SessionStatus};
use crate::storage_v2::{
    execute_work_unit_with_retry, ExpectedRevision, RecordKey, StorageWorkUnit,
    TransactionalStorage, VaultSnapshotQuery, WorkUnitCommit, WorkUnitIntent, WorkUnitReceipt,
    DEFAULT_WORK_UNIT_MAX_ATTEMPTS, WORK_UNIT_INTENT_SCHEMA_VERSION,
};

pub const TRANSACTIONAL_SLEEP_START_REQUEST_SCHEMA_VERSION: &str =
    "transactional_sleep_start_request.v1";
pub const TRANSACTIONAL_SLEEP_START_RESULT_SCHEMA_VERSION: &str =
    "transactional_sleep_start_result.v1";
pub const TRANSACTIONAL_SLEEP_SUBMIT_REQUEST_SCHEMA_VERSION: &str =
    "transactional_sleep_submit_request.v1";
pub const TRANSACTIONAL_SLEEP_SUBMIT_RESULT_SCHEMA_VERSION: &str =
    "transactional_sleep_submit_result.v1";
pub const TRANSACTIONAL_SLEEP_FINALIZE_REQUEST_SCHEMA_VERSION: &str =
    "transactional_sleep_finalize_request.v1";
pub const TRANSACTIONAL_SLEEP_FINALIZE_RESULT_SCHEMA_VERSION: &str =
    "transactional_sleep_finalize_result.v1";
pub const TRANSACTIONAL_SLEEP_CANCEL_REQUEST_SCHEMA_VERSION: &str =
    "transactional_sleep_cancel_request.v1";
pub const TRANSACTIONAL_SLEEP_CANCEL_RESULT_SCHEMA_VERSION: &str =
    "transactional_sleep_cancel_result.v1";
pub const SLEEP_START_WORK_UNIT_KIND: &str = "sleep_start";
pub const SLEEP_SUBMIT_WORK_UNIT_KIND: &str = "sleep_submit";
pub const SLEEP_FINALIZE_WORK_UNIT_KIND: &str = "sleep_finalize";
pub const SLEEP_CANCEL_WORK_UNIT_KIND: &str = "sleep_cancel";
pub const SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION: &str = "sleep_workflow_input.v1";
pub const MAX_SLEEP_WORKFLOW_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SLEEP_SUBMIT_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SLEEP_MEMORY_UNITS_PER_RUN: usize = 256;
pub const TRANSACTIONAL_SLEEP_POST_FINALIZE_STATE_SCHEMA_VERSION: &str =
    "transactional_sleep_post_finalize_state.v1";

const SESSION_COLLECTION: &str = "sessions";
// Storage v2 domain invariant: session events are append-only records created
// with ExpectedRevision::Absent by transactional ingest. Sleep freezes their
// full payloads into its tasks; later workflow stages must never rewrite them.
const ARCHIVE_COLLECTION: &str = "archives";
const TASK_COLLECTION: &str = "tasks";
const SLEEP_RUN_COLLECTION: &str = "sleep_runs";
const SLEEP_INPUT_COLLECTION: &str = "sleep_inputs";
const MEMORY_UNIT_COLLECTION: &str = "memory_units";
const OPERATION_RESULT_COLLECTION: &str = "operation_results";
const SLEEP_START_OPERATION_RESULT_SCHEMA_VERSION: &str = "sleep_start_operation_result.v1";
const SLEEP_SUBMIT_OPERATION_RESULT_SCHEMA_VERSION: &str = "sleep_submit_operation_result.v1";
const SLEEP_FINALIZE_OPERATION_RESULT_SCHEMA_VERSION: &str = "sleep_finalize_operation_result.v1";
const SLEEP_CANCEL_OPERATION_RESULT_SCHEMA_VERSION: &str = "sleep_cancel_operation_result.v1";

/// The sole durable owner of private Sleep prompt material in Storage v2.
/// Tasks, runs and operation results contain only [`SleepInputReference`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepWorkflowInput {
    pub schema_version: String,
    pub sleep_task_id: String,
    pub archive_id: String,
    pub session_id: String,
    pub created_at: String,
    pub policy: SleepStage1Config,
    pub preliminary_archive_digest: String,
    #[serde(default)]
    pub events: Vec<SleepInputEvent>,
    #[serde(default)]
    pub sleep_hints: Value,
    #[serde(default)]
    pub memory_unit_hints: Value,
    pub content_digest: String,
}

struct SleepWorkflowInputSeed<'a> {
    sleep_task_id: String,
    archive_id: String,
    session_id: String,
    created_at: String,
    policy: SleepStage1Config,
    preliminary_archive_digest: String,
    sleep_task_inputs: &'a Value,
    memory_unit_task_inputs: &'a Value,
}

impl SleepWorkflowInput {
    fn new(seed: SleepWorkflowInputSeed<'_>) -> Result<Self> {
        let events =
            serde_json::from_value(seed.sleep_task_inputs.get("events").cloned().ok_or_else(
                || MemoryEngineError::Storage("sleep task did not freeze events".to_string()),
            )?)?;
        let mut input = Self {
            schema_version: SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION.to_string(),
            sleep_task_id: seed.sleep_task_id,
            archive_id: seed.archive_id,
            session_id: seed.session_id,
            created_at: seed.created_at,
            policy: seed.policy,
            preliminary_archive_digest: seed.preliminary_archive_digest,
            events,
            sleep_hints: seed
                .sleep_task_inputs
                .get("hints")
                .cloned()
                .unwrap_or_else(|| json!({})),
            memory_unit_hints: seed
                .memory_unit_task_inputs
                .get("hints")
                .cloned()
                .unwrap_or_else(|| json!({})),
            content_digest: String::new(),
        };
        input.content_digest = input.expected_digest()?;
        input.validate()?;
        Ok(input)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        if self.sleep_task_id.trim().is_empty()
            || self.archive_id.trim().is_empty()
            || self.session_id.trim().is_empty()
            || self.events.is_empty()
        {
            return Err(MemoryEngineError::Validation(
                "sleep workflow input identity and events must not be empty".to_string(),
            ));
        }
        if parse_rfc3339(&self.created_at).is_none() {
            return Err(MemoryEngineError::Validation(
                "sleep workflow input created_at must be RFC3339".to_string(),
            ));
        }
        validate_sleep_policy(&self.policy)?;
        validate_sha256_like(
            "preliminary Archive digest",
            &self.preliminary_archive_digest,
        )?;
        let mut ids = HashSet::new();
        for event in &self.events {
            if event.schema_version != crate::sleep::SLEEP_INPUT_EVENT_SCHEMA_VERSION
                || event.session_id != self.session_id
                || !ids.insert(event.event_id.clone())
            {
                return Err(MemoryEngineError::Validation(
                    "sleep workflow input contains invalid, foreign, or duplicate events"
                        .to_string(),
                ));
            }
        }
        if self.content_digest != self.expected_digest()? {
            return Err(MemoryEngineError::Validation(
                "sleep workflow input content_digest does not match frozen content".to_string(),
            ));
        }
        if serde_json::to_vec(self)?.len() > MAX_SLEEP_WORKFLOW_INPUT_BYTES {
            return Err(MemoryEngineError::Validation(format!(
                "sleep workflow input exceeds {MAX_SLEEP_WORKFLOW_INPUT_BYTES} serialized bytes"
            )));
        }
        Ok(())
    }

    fn expected_digest(&self) -> Result<String> {
        #[derive(Serialize)]
        struct DigestInput<'a> {
            schema_version: &'a str,
            sleep_task_id: &'a str,
            archive_id: &'a str,
            session_id: &'a str,
            created_at: &'a str,
            policy: &'a SleepStage1Config,
            preliminary_archive_digest: &'a str,
            events: &'a [SleepInputEvent],
            sleep_hints: &'a Value,
            memory_unit_hints: &'a Value,
        }
        transactional_support::serializable_digest(&DigestInput {
            schema_version: &self.schema_version,
            sleep_task_id: &self.sleep_task_id,
            archive_id: &self.archive_id,
            session_id: &self.session_id,
            created_at: &self.created_at,
            policy: &self.policy,
            preliminary_archive_digest: &self.preliminary_archive_digest,
            events: &self.events,
            sleep_hints: &self.sleep_hints,
            memory_unit_hints: &self.memory_unit_hints,
        })
    }

    fn reference(&self) -> SleepInputReference {
        SleepInputReference {
            schema_version: crate::sleep::SLEEP_INPUT_REFERENCE_SCHEMA_VERSION.to_string(),
            record_id: self.sleep_task_id.clone(),
            content_digest: self.content_digest.clone(),
        }
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<LlmBatch>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepSubmitRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub sleep_task_id: String,
    pub submitted_at: String,
    pub frozen_run_digest: String,
    #[serde(default)]
    pub responses: Vec<LlmResponse>,
}

impl TransactionalSleepSubmitRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        submitted_at: impl Into<String>,
        run: &SleepRun,
        responses: Vec<LlmResponse>,
    ) -> Result<Self> {
        let submitted_at = submitted_at.into();
        let frozen_run_digest = transactional_support::serializable_digest(run)?;
        let sleep_task_id = run.sleep_task_id.clone();
        let request_digest = sleep_submit_request_digest(
            &sleep_task_id,
            &submitted_at,
            &frozen_run_digest,
            &responses,
        )?;
        let request = Self {
            schema_version: TRANSACTIONAL_SLEEP_SUBMIT_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: continuation_intent(
                operation_id,
                SLEEP_SUBMIT_WORK_UNIT_KIND,
                request_digest,
                started_at,
            ),
            sleep_task_id,
            submitted_at,
            frozen_run_digest,
            responses,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        validate_continuation_request(
            &self.schema_version,
            TRANSACTIONAL_SLEEP_SUBMIT_REQUEST_SCHEMA_VERSION,
            &self.work_unit,
            SLEEP_SUBMIT_WORK_UNIT_KIND,
            &self.sleep_task_id,
            &self.submitted_at,
            &self.frozen_run_digest,
        )?;
        if self.responses.is_empty() {
            return Err(MemoryEngineError::Validation(
                "transactional sleep submit requires at least one response".to_string(),
            ));
        }
        if serde_json::to_vec(&self.responses)?.len() > MAX_SLEEP_SUBMIT_RESPONSE_BYTES {
            return Err(MemoryEngineError::Validation(format!(
                "transactional sleep responses exceed {MAX_SLEEP_SUBMIT_RESPONSE_BYTES} serialized bytes"
            )));
        }
        let mut request_ids = HashSet::new();
        for response in &self.responses {
            let request_id = llm_response_request_id(response);
            if request_id.trim().is_empty() || !request_ids.insert(request_id) {
                return Err(MemoryEngineError::Validation(
                    "transactional sleep submit response IDs must be non-empty and unique"
                        .to_string(),
                ));
            }
        }
        let expected = sleep_submit_request_digest(
            &self.sleep_task_id,
            &self.submitted_at,
            &self.frozen_run_digest,
            &self.responses,
        )?;
        ensure_request_digest(&self.work_unit, &expected, "sleep submit")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepSubmitResult {
    pub schema_version: String,
    pub run: SleepRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<LlmBatch>,
    pub receipt: WorkUnitReceipt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepFinalizeRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub sleep_task_id: String,
    pub finalized_at: String,
    pub frozen_run_digest: String,
}

impl TransactionalSleepFinalizeRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        finalized_at: impl Into<String>,
        run: &SleepRun,
    ) -> Result<Self> {
        let finalized_at = finalized_at.into();
        let frozen_run_digest = transactional_support::serializable_digest(run)?;
        let sleep_task_id = run.sleep_task_id.clone();
        let request_digest =
            sleep_terminal_request_digest(&sleep_task_id, &finalized_at, &frozen_run_digest, None)?;
        let request = Self {
            schema_version: TRANSACTIONAL_SLEEP_FINALIZE_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: continuation_intent(
                operation_id,
                SLEEP_FINALIZE_WORK_UNIT_KIND,
                request_digest,
                started_at,
            ),
            sleep_task_id,
            finalized_at,
            frozen_run_digest,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        validate_continuation_request(
            &self.schema_version,
            TRANSACTIONAL_SLEEP_FINALIZE_REQUEST_SCHEMA_VERSION,
            &self.work_unit,
            SLEEP_FINALIZE_WORK_UNIT_KIND,
            &self.sleep_task_id,
            &self.finalized_at,
            &self.frozen_run_digest,
        )?;
        let expected = sleep_terminal_request_digest(
            &self.sleep_task_id,
            &self.finalized_at,
            &self.frozen_run_digest,
            None,
        )?;
        ensure_request_digest(&self.work_unit, &expected, "sleep finalize")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepFinalizeResult {
    pub schema_version: String,
    pub run: SleepRun,
    pub outcome: SleepOutcome,
    pub post_finalize: TransactionalSleepPostFinalizeState,
    pub receipt: WorkUnitReceipt,
}

/// Explicit hand-off for cognition intentionally left outside this S2.C
/// transaction. Empty SleepOutcome collections must not be mistaken for
/// "evaluated and no work found" while these branches await Storage v2 ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionalSleepPostFinalizeState {
    pub schema_version: String,
    pub core_bridge_pending: bool,
    pub fidelity_routing_pending: bool,
    pub embedding_backfill_pending: bool,
}

impl TransactionalSleepPostFinalizeState {
    fn pending() -> Self {
        Self {
            schema_version: TRANSACTIONAL_SLEEP_POST_FINALIZE_STATE_SCHEMA_VERSION.to_string(),
            core_bridge_pending: true,
            fidelity_routing_pending: true,
            embedding_backfill_pending: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepCancelRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub sleep_task_id: String,
    pub cancelled_at: String,
    pub frozen_run_digest: String,
    pub reason: String,
}

impl TransactionalSleepCancelRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        cancelled_at: impl Into<String>,
        run: &SleepRun,
        reason: impl Into<String>,
    ) -> Result<Self> {
        let cancelled_at = cancelled_at.into();
        let frozen_run_digest = transactional_support::serializable_digest(run)?;
        let sleep_task_id = run.sleep_task_id.clone();
        let reason = normalize_whitespace(&reason.into());
        let request_digest = sleep_terminal_request_digest(
            &sleep_task_id,
            &cancelled_at,
            &frozen_run_digest,
            Some(&reason),
        )?;
        let request = Self {
            schema_version: TRANSACTIONAL_SLEEP_CANCEL_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: continuation_intent(
                operation_id,
                SLEEP_CANCEL_WORK_UNIT_KIND,
                request_digest,
                started_at,
            ),
            sleep_task_id,
            cancelled_at,
            frozen_run_digest,
            reason,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        validate_continuation_request(
            &self.schema_version,
            TRANSACTIONAL_SLEEP_CANCEL_REQUEST_SCHEMA_VERSION,
            &self.work_unit,
            SLEEP_CANCEL_WORK_UNIT_KIND,
            &self.sleep_task_id,
            &self.cancelled_at,
            &self.frozen_run_digest,
        )?;
        if self.reason.trim().is_empty() || self.reason != normalize_whitespace(&self.reason) {
            return Err(MemoryEngineError::Validation(
                "transactional sleep cancel reason must be normalized and non-empty".to_string(),
            ));
        }
        let expected = sleep_terminal_request_digest(
            &self.sleep_task_id,
            &self.cancelled_at,
            &self.frozen_run_digest,
            Some(&self.reason),
        )?;
        ensure_request_digest(&self.work_unit, &expected, "sleep cancel")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalSleepCancelResult {
    pub schema_version: String,
    pub run: SleepRun,
    pub receipt: WorkUnitReceipt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SleepSubmitOperationResult {
    schema_version: String,
    run: SleepRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SleepFinalizeOperationResult {
    schema_version: String,
    run: SleepRun,
    outcome: SleepOutcome,
    post_finalize: TransactionalSleepPostFinalizeState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SleepCancelOperationResult {
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
        let input_key = RecordKey::new(SLEEP_INPUT_COLLECTION, sleep_task_id.clone())?;
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
                for record in work_unit.query_snapshot(&VaultSnapshotQuery::SessionEvents {
                    session_id: session_id.clone(),
                })? {
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
                    event.validate()?;
                    if record.key.record_id != event.event_id {
                        return Err(MemoryEngineError::Storage(
                            "session event record key does not match payload event_id".to_string(),
                        )
                        .into());
                    }
                    if event.session_id != session_id {
                        return Err(MemoryEngineError::Storage(
                            "typed session-event query returned another session".to_string(),
                        )
                        .into());
                    }
                    events.push(event);
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
                let mut sleep_task = stage1.pending_task.clone();
                let mut memory_task = stage1.memory_unit_task.clone().ok_or_else(|| {
                    MemoryEngineError::Storage(
                        "shared sleep Stage 1 produced no memory-unit task".to_string(),
                    )
                })?;
                let mut run = sleep_run_from_stage1_with_request_ids(
                    stage1,
                    request_ids[0].clone(),
                    [
                        request_ids[1].clone(),
                        request_ids[2].clone(),
                        request_ids[3].clone(),
                        request_ids[4].clone(),
                    ],
                    SleepRunOptions::default(),
                )?;
                let workflow_input = SleepWorkflowInput::new(SleepWorkflowInputSeed {
                    sleep_task_id: sleep_task_id.clone(),
                    archive_id: archive_id.clone(),
                    session_id: session_id.clone(),
                    created_at: created_at.clone(),
                    policy: policy.clone(),
                    preliminary_archive_digest: transactional_support::serializable_digest(
                        &archive_entry,
                    )?,
                    sleep_task_inputs: &sleep_task.inputs,
                    memory_unit_task_inputs: &memory_task.inputs,
                })?;
                let input_reference = workflow_input.reference();
                sleep_task.inputs = task_input_reference(&input_reference, TaskType::SleepCompression);
                memory_task.inputs = task_input_reference(&input_reference, TaskType::MemoryUnitPass);
                normalize_run_input_references(&mut run, &input_reference);
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
                    input_key.clone(),
                    ExpectedRevision::Absent,
                    SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&workflow_input)?,
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
        let workflow_input =
            read_sleep_workflow_input(&self.storage, context, &operation_result.run.sleep_task_id)?;
        let batch = current_transactional_batch(&operation_result.run, &workflow_input)?;
        Ok(TransactionalSleepStartResult {
            schema_version: TRANSACTIONAL_SLEEP_START_RESULT_SCHEMA_VERSION.to_string(),
            run: operation_result.run,
            batch,
            receipt,
        })
    }
}

impl<S: TransactionalStorage> MemoryEngine<S> {
    /// Rehydrates the next provider batch from the immutable workflow input.
    /// This is read-only and is the recovery entry point after worker restart.
    pub fn resume_sleep_run_transactional(
        &self,
        context: &VaultContext,
        sleep_task_id: &str,
    ) -> Result<SleepRunStep> {
        context.validate()?;
        if sleep_task_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "transactional sleep task id must not be empty".to_string(),
            ));
        }
        let run = read_sleep_run(&self.storage, context, sleep_task_id)?;
        let input = read_sleep_workflow_input(&self.storage, context, sleep_task_id)?;
        validate_run_input_reference(&run, &input)?;
        let batch = current_transactional_batch(&run, &input)?;
        Ok(SleepRunStep { run, batch })
    }

    pub fn submit_sleep_batch_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalSleepSubmitRequest,
    ) -> Result<TransactionalSleepSubmitResult> {
        context.validate()?;
        request.validate()?;

        let run_key = RecordKey::new(SLEEP_RUN_COLLECTION, request.sleep_task_id.clone())?;
        let input_key = RecordKey::new(SLEEP_INPUT_COLLECTION, request.sleep_task_id.clone())?;
        let sleep_task_key = RecordKey::new(TASK_COLLECTION, request.sleep_task_id.clone())?;
        let result_key = operation_result_key(&request.work_unit.operation_id)?;
        let submitted_at = request.submitted_at;
        let frozen_run_digest = request.frozen_run_digest;
        let responses = request.responses;
        let submit_operation_id = request.work_unit.operation_id.clone();
        let intent = request.work_unit;
        let result_reference = result_key.reference();
        let consolidator_request_id = deterministic_workflow_id(
            context,
            &run_key.record_id,
            "consolidator_request",
            "llm_req",
        );

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let input_record = required_record(work_unit, &input_key, "sleep input")?;
                let input = decode_sleep_input(input_record.clone())?;
                let run_record = required_record(work_unit, &run_key, "sleep run")?;
                let mut run = decode_sleep_run(run_record.clone())?;
                validate_frozen_run(&run, &run_key.record_id, &frozen_run_digest)?;
                validate_run_input_reference(&run, &input)?;
                if !matches!(
                    run.stage,
                    SleepRunStage::Extraction | SleepRunStage::Consolidation
                ) {
                    return Err(MemoryEngineError::Validation(
                        "transactional sleep submit requires an active provider stage".to_string(),
                    )
                    .into());
                }

                let session_key = RecordKey::new(SESSION_COLLECTION, run.session_id.clone())?;
                let archive_key = RecordKey::new(ARCHIVE_COLLECTION, run.archive_id.clone())?;
                let memory_task_id = run.memory_unit_task_id.clone().ok_or_else(|| {
                    MemoryEngineError::Storage(
                        "transactional sleep run has no memory-unit task".to_string(),
                    )
                })?;
                let memory_task_key = RecordKey::new(TASK_COLLECTION, memory_task_id)?;
                let session_record = required_record(work_unit, &session_key, "Session")?;
                let session = decode_reserved_session(
                    session_record,
                    &run.session_id,
                    &run.sleep_task_id,
                    &input,
                )?;
                let archive_record = required_record(work_unit, &archive_key, "Archive")?;
                let archive = decode_preliminary_archive(archive_record, &run, &input)?;
                let sleep_task_record = required_record(work_unit, &sleep_task_key, "sleep task")?;
                let mut sleep_task = decode_workflow_task(
                    sleep_task_record.clone(),
                    TaskType::SleepCompression,
                    &run,
                    &input,
                )?;
                let memory_task_record =
                    required_record(work_unit, &memory_task_key, "memory-unit task")?;
                let mut memory_task = decode_workflow_task(
                    memory_task_record.clone(),
                    TaskType::MemoryUnitPass,
                    &run,
                    &input,
                )?;
                validate_tasks_active(&sleep_task, &memory_task)?;

                for response in responses.clone() {
                    let request_id = llm_response_request_id(&response).to_string();
                    let Some(index) = run
                        .requests
                        .iter()
                        .position(|state| state.request.request_id == request_id)
                    else {
                        return Err(MemoryEngineError::Validation(format!(
                            "LLM response does not match transactional sleep run: {request_id}"
                        ))
                        .into());
                    };
                    let mut state = run.requests[index].clone();
                    if state.completed || state_stage(state.track) != run.stage {
                        return Err(MemoryEngineError::Validation(format!(
                            "LLM response is stale or belongs to another sleep stage: {request_id}"
                        ))
                        .into());
                    }
                    let track = state.track;
                    state.attempts = state.attempts.saturating_add(1);
                    handle_sleep_response(&mut run, &mut state, response)?;
                    let task = if track == SleepTrack::MemoryUnit {
                        &mut memory_task
                    } else {
                        &mut sleep_task
                    };
                    task.state = TaskState::Submitted;
                    task.updated_at = submitted_at.clone();
                    task.attempts.push(TaskAttempt {
                        attempt_id: deterministic_workflow_id(
                            context,
                            &run.sleep_task_id,
                            &format!("attempt:{submit_operation_id}:{request_id}"),
                            "attempt",
                        ),
                        started_at: submitted_at.clone(),
                        finished_at: Some(submitted_at.clone()),
                        provider: None,
                        model: None,
                        status: if state.last_error.is_some() {
                            AttemptStatus::Failed
                        } else {
                            AttemptStatus::Completed
                        },
                        error: state.last_error.clone(),
                    });
                    run.requests[index] = state;
                }

                advance_transactional_sleep_stage(&mut run, consolidator_request_id.clone())?;
                normalize_run_input_references(&mut run, &input.reference());
                validate_sleep_run(&run)?;
                let operation_result = SleepSubmitOperationResult {
                    schema_version: SLEEP_SUBMIT_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    run: run.clone(),
                };
                let result_digest = transactional_support::serializable_digest(&operation_result)?;
                work_unit.put(
                    run_key.clone(),
                    ExpectedRevision::Exact(run_record.revision),
                    SLEEP_RUN_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&run)?,
                )?;
                work_unit.put(
                    sleep_task_key.clone(),
                    ExpectedRevision::Exact(sleep_task_record.revision),
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&sleep_task)?,
                )?;
                work_unit.put(
                    memory_task_key.clone(),
                    ExpectedRevision::Exact(memory_task_record.revision),
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&memory_task)?,
                )?;
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    SLEEP_SUBMIT_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&operation_result)?,
                )?;
                // Keep all reads in the optimistic snapshot even though this
                // transition changes only run/tasks.
                let _ = (session, archive, input_record.revision);
                Ok(WorkUnitCommit {
                    committed_at: submitted_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let operation_result =
            transactional_support::read_transaction_result::<S, SleepSubmitOperationResult>(
                &self.storage,
                context,
                &result_key,
                &receipt,
                SLEEP_SUBMIT_OPERATION_RESULT_SCHEMA_VERSION,
            )?;
        let input =
            read_sleep_workflow_input(&self.storage, context, &operation_result.run.sleep_task_id)?;
        let batch = current_transactional_batch(&operation_result.run, &input)?;
        Ok(TransactionalSleepSubmitResult {
            schema_version: TRANSACTIONAL_SLEEP_SUBMIT_RESULT_SCHEMA_VERSION.to_string(),
            run: operation_result.run,
            batch,
            receipt,
        })
    }

    pub fn finalize_sleep_run_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalSleepFinalizeRequest,
    ) -> Result<TransactionalSleepFinalizeResult> {
        context.validate()?;
        request.validate()?;

        let run_key = RecordKey::new(SLEEP_RUN_COLLECTION, request.sleep_task_id.clone())?;
        let input_key = RecordKey::new(SLEEP_INPUT_COLLECTION, request.sleep_task_id.clone())?;
        let sleep_task_key = RecordKey::new(TASK_COLLECTION, request.sleep_task_id.clone())?;
        let result_key = operation_result_key(&request.work_unit.operation_id)?;
        let finalized_at = request.finalized_at;
        let frozen_run_digest = request.frozen_run_digest;
        let intent = request.work_unit;
        let result_reference = result_key.reference();

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let input_record = required_record(work_unit, &input_key, "sleep input")?;
                let input = decode_sleep_input(input_record)?;
                let run_record = required_record(work_unit, &run_key, "sleep run")?;
                let mut run = decode_sleep_run(run_record.clone())?;
                validate_frozen_run(&run, &run_key.record_id, &frozen_run_digest)?;
                validate_run_input_reference(&run, &input)?;
                if run.stage != SleepRunStage::ReadyToFinish {
                    return Err(MemoryEngineError::Validation(
                        "transactional sleep run is not ready to finalize".to_string(),
                    )
                    .into());
                }

                let session_key = RecordKey::new(SESSION_COLLECTION, run.session_id.clone())?;
                let archive_key = RecordKey::new(ARCHIVE_COLLECTION, run.archive_id.clone())?;
                let memory_task_id = run.memory_unit_task_id.clone().ok_or_else(|| {
                    MemoryEngineError::Storage(
                        "transactional sleep run has no memory-unit task".to_string(),
                    )
                })?;
                let memory_task_key = RecordKey::new(TASK_COLLECTION, memory_task_id)?;
                let session_record = required_record(work_unit, &session_key, "Session")?;
                let mut session = decode_reserved_session(
                    session_record.clone(),
                    &run.session_id,
                    &run.sleep_task_id,
                    &input,
                )?;
                let archive_record = required_record(work_unit, &archive_key, "Archive")?;
                let mut archive =
                    decode_preliminary_archive(archive_record.clone(), &run, &input)?;
                let sleep_task_record =
                    required_record(work_unit, &sleep_task_key, "sleep task")?;
                let mut sleep_task = decode_workflow_task(
                    sleep_task_record.clone(),
                    TaskType::SleepCompression,
                    &run,
                    &input,
                )?;
                let memory_task_record =
                    required_record(work_unit, &memory_task_key, "memory-unit task")?;
                let mut memory_task = decode_workflow_task(
                    memory_task_record.clone(),
                    TaskType::MemoryUnitPass,
                    &run,
                    &input,
                )?;
                if sleep_task.state != TaskState::Submitted
                    || memory_task.state != TaskState::Submitted
                {
                    return Err(MemoryEngineError::Storage(
                        "ready SleepRun requires both workflow tasks to be submitted".to_string(),
                    )
                    .into());
                }

                let mut compression = assemble_sleep_compression_from_tracks(&run)?;
                if let Some(gist) = run
                    .consolidator_gist
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    compression.gist = gist.to_string();
                }
                if let Some(narrative) = run
                    .consolidator_narrative
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    compression.narrative = narrative.to_string();
                }
                apply_sleep_run_tags(&mut compression, &run);
                compression.validate_basic()?;
                let attribution_events = input
                    .events
                    .iter()
                    .map(AttributionEvent::from)
                    .collect::<Vec<_>>();
                attribute_personal_signals(
                    &mut compression.personal_signals,
                    &attribution_events,
                );
                apply_compression_to_archive(&mut archive, compression, &sleep_task, &finalized_at);
                let accepted = accepted_memory_unit_result(&run)?;
                let extraction_published = accepted.is_some();
                let units = build_transactional_memory_units(
                    context,
                    &run.sleep_task_id,
                    &archive,
                    &input,
                    &finalized_at,
                    accepted.unwrap_or_else(|| empty_memory_unit_result(&run.archive_id)),
                )?;
                archive.memory_units = units.clone();
                archive.compact_memory = render_compact_memory_from_units(&units);
                archive.updated_at = finalized_at.clone();

                for unit in &units {
                    let unit_key =
                        RecordKey::new(MEMORY_UNIT_COLLECTION, unit.memory_unit_id.clone())?;
                    work_unit.put(
                        unit_key,
                        ExpectedRevision::Absent,
                        MEMORY_UNIT_SCHEMA_VERSION.to_string(),
                        serde_json::to_value(unit)?,
                    )?;
                }
                sleep_task.state = TaskState::Completed;
                sleep_task.updated_at = finalized_at.clone();
                sleep_task.last_error = None;
                sleep_task.outcome = Some(json!({
                    "schema_version": "sleep_task_outcome.v1",
                    "archive_id": archive.archive_id,
                    "completion_mode": run.completion_mode.clone().unwrap_or_else(|| "consolidated".to_string()),
                }));
                memory_task.state = TaskState::Completed;
                memory_task.updated_at = finalized_at.clone();
                memory_task.last_error = None;
                memory_task.outcome = extraction_published.then(|| {
                    serde_json::to_value(MemoryUnitTaskOutcome::published(&archive.archive_id, &units))
                }).transpose().map_err(MemoryEngineError::from)?;
                merge_unique(&mut session.archived_to, [archive.archive_id.clone()]);
                merge_unique(
                    &mut session.archived_event_ids,
                    archive.source_event_ids.clone(),
                );
                session.archived_event_index_complete = true;
                session.status = SessionStatus::Active;
                session.active_sleep_task_id = None;
                session.sleep_pending_event_ids.clear();
                session.updated_at = finalized_at.clone();
                session.validate()?;
                run.stage = SleepRunStage::Finished;
                run.completion_mode = Some(
                    run.completion_mode
                        .clone()
                        .unwrap_or_else(|| "consolidated".to_string()),
                );
                normalize_run_input_references(&mut run, &input.reference());
                let outcome = SleepOutcome {
                    archive_entry: archive.clone(),
                    core_summary: CoreSignalSummary::default(),
                    fidelity_requests: Vec::new(),
                    embedding_requests: Vec::new(),
                    failed_passes: run.failed_passes.clone(),
                    skipped_passes: run.skipped_passes.clone(),
                    pass_reports: sleep_pass_reports(&run),
                    provider_calls_issued: run.provider_calls_issued,
                    max_provider_calls: run.max_provider_calls,
                    budget_exhausted: run.budget_exhausted,
                    completion_mode: run
                        .completion_mode
                        .clone()
                        .unwrap_or_else(|| "consolidated".to_string()),
                };
                let operation_result = SleepFinalizeOperationResult {
                    schema_version: SLEEP_FINALIZE_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    run: run.clone(),
                    outcome: outcome.clone(),
                    post_finalize: TransactionalSleepPostFinalizeState::pending(),
                };
                let result_digest =
                    transactional_support::serializable_digest(&operation_result)?;
                work_unit.put(
                    archive_key.clone(),
                    ExpectedRevision::Exact(archive_record.revision),
                    ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&archive)?,
                )?;
                work_unit.put(
                    sleep_task_key.clone(),
                    ExpectedRevision::Exact(sleep_task_record.revision),
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&sleep_task)?,
                )?;
                work_unit.put(
                    memory_task_key.clone(),
                    ExpectedRevision::Exact(memory_task_record.revision),
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&memory_task)?,
                )?;
                work_unit.put(
                    session_key.clone(),
                    ExpectedRevision::Exact(session_record.revision),
                    SESSION_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&session)?,
                )?;
                work_unit.put(
                    run_key.clone(),
                    ExpectedRevision::Exact(run_record.revision),
                    SLEEP_RUN_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&run)?,
                )?;
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    SLEEP_FINALIZE_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&operation_result)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: finalized_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let operation_result =
            transactional_support::read_transaction_result::<S, SleepFinalizeOperationResult>(
                &self.storage,
                context,
                &result_key,
                &receipt,
                SLEEP_FINALIZE_OPERATION_RESULT_SCHEMA_VERSION,
            )?;
        Ok(TransactionalSleepFinalizeResult {
            schema_version: TRANSACTIONAL_SLEEP_FINALIZE_RESULT_SCHEMA_VERSION.to_string(),
            run: operation_result.run,
            outcome: operation_result.outcome,
            post_finalize: operation_result.post_finalize,
            receipt,
        })
    }

    pub fn cancel_sleep_run_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalSleepCancelRequest,
    ) -> Result<TransactionalSleepCancelResult> {
        context.validate()?;
        request.validate()?;

        let run_key = RecordKey::new(SLEEP_RUN_COLLECTION, request.sleep_task_id.clone())?;
        let input_key = RecordKey::new(SLEEP_INPUT_COLLECTION, request.sleep_task_id.clone())?;
        let sleep_task_key = RecordKey::new(TASK_COLLECTION, request.sleep_task_id.clone())?;
        let result_key = operation_result_key(&request.work_unit.operation_id)?;
        let cancelled_at = request.cancelled_at;
        let frozen_run_digest = request.frozen_run_digest;
        let reason = request.reason;
        let intent = request.work_unit;
        let result_reference = result_key.reference();

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let input =
                    decode_sleep_input(required_record(work_unit, &input_key, "sleep input")?)?;
                let run_record = required_record(work_unit, &run_key, "sleep run")?;
                let mut run = decode_sleep_run(run_record.clone())?;
                validate_frozen_run(&run, &run_key.record_id, &frozen_run_digest)?;
                validate_run_input_reference(&run, &input)?;
                if run.stage == SleepRunStage::Finished {
                    return Err(MemoryEngineError::Validation(
                        "transactional sleep run is already terminal".to_string(),
                    )
                    .into());
                }
                let session_key = RecordKey::new(SESSION_COLLECTION, run.session_id.clone())?;
                let archive_key = RecordKey::new(ARCHIVE_COLLECTION, run.archive_id.clone())?;
                let memory_task_id = run.memory_unit_task_id.clone().ok_or_else(|| {
                    MemoryEngineError::Storage(
                        "transactional sleep run has no memory-unit task".to_string(),
                    )
                })?;
                let memory_task_key = RecordKey::new(TASK_COLLECTION, memory_task_id)?;
                let session_record = required_record(work_unit, &session_key, "Session")?;
                let mut session = decode_reserved_session(
                    session_record.clone(),
                    &run.session_id,
                    &run.sleep_task_id,
                    &input,
                )?;
                let archive_record = required_record(work_unit, &archive_key, "Archive")?;
                let mut archive = decode_preliminary_archive(archive_record.clone(), &run, &input)?;
                let sleep_task_record = required_record(work_unit, &sleep_task_key, "sleep task")?;
                let mut sleep_task = decode_workflow_task(
                    sleep_task_record.clone(),
                    TaskType::SleepCompression,
                    &run,
                    &input,
                )?;
                let memory_task_record =
                    required_record(work_unit, &memory_task_key, "memory-unit task")?;
                let mut memory_task = decode_workflow_task(
                    memory_task_record.clone(),
                    TaskType::MemoryUnitPass,
                    &run,
                    &input,
                )?;
                validate_tasks_active(&sleep_task, &memory_task)?;

                for task in [&mut sleep_task, &mut memory_task] {
                    if !matches!(task.state, TaskState::Completed | TaskState::Failed) {
                        task.state = TaskState::Cancelled;
                        task.updated_at = cancelled_at.clone();
                        task.last_error = Some(reason.clone());
                        task.outcome = Some(json!({
                            "schema_version": "sleep_cancelled_task_outcome.v1",
                            "reason": reason,
                        }));
                    }
                }
                archive.status = ArchiveStatus::Superseded;
                archive.updated_at = cancelled_at.clone();
                session.status = SessionStatus::Active;
                session.active_sleep_task_id = None;
                session.sleep_pending_event_ids.clear();
                session.updated_at = cancelled_at.clone();
                session.validate()?;
                run.stage = SleepRunStage::Finished;
                run.completion_mode = Some("cancelled".to_string());
                normalize_run_input_references(&mut run, &input.reference());
                let operation_result = SleepCancelOperationResult {
                    schema_version: SLEEP_CANCEL_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    run: run.clone(),
                };
                let result_digest = transactional_support::serializable_digest(&operation_result)?;
                work_unit.put(
                    archive_key.clone(),
                    ExpectedRevision::Exact(archive_record.revision),
                    ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&archive)?,
                )?;
                work_unit.put(
                    sleep_task_key.clone(),
                    ExpectedRevision::Exact(sleep_task_record.revision),
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&sleep_task)?,
                )?;
                work_unit.put(
                    memory_task_key.clone(),
                    ExpectedRevision::Exact(memory_task_record.revision),
                    PENDING_TASK_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&memory_task)?,
                )?;
                work_unit.put(
                    session_key.clone(),
                    ExpectedRevision::Exact(session_record.revision),
                    SESSION_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&session)?,
                )?;
                work_unit.put(
                    run_key.clone(),
                    ExpectedRevision::Exact(run_record.revision),
                    SLEEP_RUN_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&run)?,
                )?;
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    SLEEP_CANCEL_OPERATION_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&operation_result)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: cancelled_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let operation_result =
            transactional_support::read_transaction_result::<S, SleepCancelOperationResult>(
                &self.storage,
                context,
                &result_key,
                &receipt,
                SLEEP_CANCEL_OPERATION_RESULT_SCHEMA_VERSION,
            )?;
        Ok(TransactionalSleepCancelResult {
            schema_version: TRANSACTIONAL_SLEEP_CANCEL_RESULT_SCHEMA_VERSION.to_string(),
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
    metadata.validate()?;
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

fn continuation_intent(
    operation_id: impl Into<String>,
    operation_kind: &str,
    request_digest: String,
    started_at: impl Into<String>,
) -> WorkUnitIntent {
    WorkUnitIntent {
        schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
        operation_id: operation_id.into(),
        operation_kind: operation_kind.to_string(),
        request_digest,
        started_at: started_at.into(),
    }
}

fn validate_continuation_request(
    actual_schema: &str,
    expected_schema: &str,
    intent: &WorkUnitIntent,
    operation_kind: &str,
    sleep_task_id: &str,
    applied_at: &str,
    frozen_run_digest: &str,
) -> Result<()> {
    if actual_schema != expected_schema {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: expected_schema.to_string(),
            actual: actual_schema.to_string(),
        });
    }
    intent.validate()?;
    if intent.operation_kind != operation_kind {
        return Err(MemoryEngineError::Validation(format!(
            "transactional sleep operation requires operation_kind {operation_kind}"
        )));
    }
    if sleep_task_id.trim().is_empty() || parse_rfc3339(applied_at).is_none() {
        return Err(MemoryEngineError::Validation(
            "transactional sleep continuation requires task id and RFC3339 time".to_string(),
        ));
    }
    validate_sha256_like("frozen run digest", frozen_run_digest)
}

fn validate_sha256_like(label: &str, digest: &str) -> Result<()> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must use sha256"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MemoryEngineError::Validation(format!(
            "{label} must contain 64 hexadecimal characters"
        )));
    }
    Ok(())
}

fn ensure_request_digest(intent: &WorkUnitIntent, expected: &str, label: &str) -> Result<()> {
    if intent.request_digest != expected {
        return Err(MemoryEngineError::Validation(format!(
            "transactional {label} request_digest does not match its frozen input"
        )));
    }
    Ok(())
}

#[derive(Serialize)]
struct SleepSubmitDigestInput<'a> {
    sleep_task_id: &'a str,
    submitted_at: &'a str,
    frozen_run_digest: &'a str,
    responses: &'a [LlmResponse],
}

fn sleep_submit_request_digest(
    sleep_task_id: &str,
    submitted_at: &str,
    frozen_run_digest: &str,
    responses: &[LlmResponse],
) -> Result<String> {
    transactional_support::serializable_digest(&SleepSubmitDigestInput {
        sleep_task_id,
        submitted_at,
        frozen_run_digest,
        responses,
    })
}

#[derive(Serialize)]
struct SleepTerminalDigestInput<'a> {
    sleep_task_id: &'a str,
    applied_at: &'a str,
    frozen_run_digest: &'a str,
    reason: Option<&'a str>,
}

fn sleep_terminal_request_digest(
    sleep_task_id: &str,
    applied_at: &str,
    frozen_run_digest: &str,
    reason: Option<&str>,
) -> Result<String> {
    transactional_support::serializable_digest(&SleepTerminalDigestInput {
        sleep_task_id,
        applied_at,
        frozen_run_digest,
        reason,
    })
}

fn operation_result_key(operation_id: &str) -> Result<RecordKey> {
    RecordKey::new(OPERATION_RESULT_COLLECTION, operation_id)
}

fn deterministic_workflow_id(
    context: &VaultContext,
    sleep_task_id: &str,
    label: &str,
    prefix: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(context.vault_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(sleep_task_id.as_bytes());
    hasher.update([0]);
    hasher.update(label.as_bytes());
    format!("{prefix}_{:x}", hasher.finalize())
}

fn task_input_reference(reference: &SleepInputReference, task_type: TaskType) -> Value {
    json!({
        "sleep_input_ref": reference,
        "task_kind": task_type_name(task_type),
    })
}

fn task_type_name(task_type: TaskType) -> &'static str {
    match task_type {
        TaskType::SleepCompression => "sleep_compression",
        TaskType::MemoryUnitPass => "memory_unit_pass",
        _ => "unsupported",
    }
}

fn normalize_run_input_references(run: &mut SleepRun, reference: &SleepInputReference) {
    run.input_reference = Some(reference.clone());
    // Storage v2 keeps private source material in the single immutable input
    // record. The provider-facing compact projection is hydrated only when a
    // worker asks for the current batch.
    run.shared_prompt_context = None;
    for state in &mut run.requests {
        state.request.prompt_inputs = json!({
            "sleep_input_ref": reference,
            "track": format!("{:?}", state.track).to_lowercase(),
        });
    }
}

fn validate_run_input_reference(run: &SleepRun, input: &SleepWorkflowInput) -> Result<()> {
    input.validate()?;
    let expected = input.reference();
    let actual = run.input_reference.as_ref().ok_or_else(|| {
        MemoryEngineError::Storage(
            "transactional SleepRun has no immutable input reference".to_string(),
        )
    })?;
    actual.validate()?;
    if actual != &expected
        || run.sleep_task_id != input.sleep_task_id
        || run.archive_id != input.archive_id
        || run.session_id != input.session_id
    {
        return Err(MemoryEngineError::Storage(
            "transactional SleepRun does not match immutable workflow input".to_string(),
        ));
    }
    if run.shared_prompt_context.is_some() {
        return Err(MemoryEngineError::Storage(
            "transactional SleepRun must not persist a duplicate prompt context".to_string(),
        ));
    }
    validate_transactional_run_shape(run, input)?;
    for state in &run.requests {
        let state_reference: SleepInputReference = serde_json::from_value(
            state
                .request
                .prompt_inputs
                .get("sleep_input_ref")
                .cloned()
                .ok_or_else(|| {
                    MemoryEngineError::Storage(
                        "transactional sleep request contains inline or missing input".to_string(),
                    )
                })?,
        )?;
        if state_reference != expected {
            return Err(MemoryEngineError::Storage(
                "transactional sleep request points at another workflow input".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_transactional_run_shape(run: &SleepRun, input: &SleepWorkflowInput) -> Result<()> {
    let mut tracks = Vec::new();
    let mut request_ids = HashSet::new();
    for state in &run.requests {
        if tracks.contains(&state.track) || !request_ids.insert(state.request.request_id.as_str()) {
            return Err(MemoryEngineError::Storage(
                "transactional SleepRun contains duplicate track or request id".to_string(),
            ));
        }
        tracks.push(state.track);
        if state.attempts > run.max_pass_attempts {
            return Err(MemoryEngineError::Storage(
                "transactional SleepRun attempt count exceeds frozen policy".to_string(),
            ));
        }
        let (task_id, prompt_id, prompt_version, output_schema) = match state.track {
            SleepTrack::MemoryUnit => (
                run.memory_unit_task_id.as_deref().unwrap_or_default(),
                "memory_unit_pass",
                3,
                MEMORY_UNITS_RESULT_SCHEMA_VERSION,
            ),
            SleepTrack::Emotional => (
                run.sleep_task_id.as_str(),
                "sleep_emotional_pass",
                input.policy.prompt_version,
                "sleep_emotional_pass.v1",
            ),
            SleepTrack::TopicThread => (
                run.sleep_task_id.as_str(),
                "sleep_topic_thread_pass",
                input.policy.prompt_version,
                "sleep_topic_thread_pass.v1",
            ),
            SleepTrack::PersonalSignal => (
                run.sleep_task_id.as_str(),
                "sleep_personal_signal_pass",
                input.policy.prompt_version,
                "sleep_personal_signal_pass.v1",
            ),
            SleepTrack::Relational => (
                run.sleep_task_id.as_str(),
                "sleep_relational_pass",
                input.policy.prompt_version,
                "sleep_relational_pass.v1",
            ),
            SleepTrack::Consolidator => (
                run.sleep_task_id.as_str(),
                "sleep_consolidator",
                3,
                CONSOLIDATOR_TEXT_SCHEMA_VERSION,
            ),
        };
        let prompt_version_matches = match state.track {
            SleepTrack::MemoryUnit | SleepTrack::Consolidator => {
                matches!(state.request.prompt_version, 1..=3)
            }
            _ => state.request.prompt_version == prompt_version,
        };
        if state.request.task_id != task_id
            || state.request.prompt_id != prompt_id
            || !prompt_version_matches
            || state.request.expected_output_schema != output_schema
        {
            return Err(MemoryEngineError::Storage(
                "transactional SleepRun request contract changed outside workflow".to_string(),
            ));
        }
    }
    for required in [
        SleepTrack::MemoryUnit,
        SleepTrack::Emotional,
        SleepTrack::TopicThread,
        SleepTrack::PersonalSignal,
        SleepTrack::Relational,
    ] {
        if !tracks.contains(&required) {
            return Err(MemoryEngineError::Storage(
                "transactional SleepRun is missing a required extraction track".to_string(),
            ));
        }
    }
    if matches!(
        run.stage,
        SleepRunStage::Consolidation | SleepRunStage::ReadyToFinish
    ) && !tracks.contains(&SleepTrack::Consolidator)
    {
        return Err(MemoryEngineError::Storage(
            "transactional SleepRun is missing its consolidator track".to_string(),
        ));
    }
    if run.stage == SleepRunStage::ReadyToFinish
        && run.requests.iter().any(|state| !state.completed)
    {
        return Err(MemoryEngineError::Storage(
            "ready transactional SleepRun still has incomplete requests".to_string(),
        ));
    }
    Ok(())
}

fn hydrate_request(
    run: &SleepRun,
    input: &SleepWorkflowInput,
    state: &SleepRequestState,
) -> LlmRequest {
    let mut request = state.request.clone();
    request.prompt_inputs = match state.track {
        SleepTrack::MemoryUnit => sleep_request_inputs(&input.archive_id),
        SleepTrack::Emotional
        | SleepTrack::TopicThread
        | SleepTrack::PersonalSignal
        | SleepTrack::Relational => sleep_request_inputs(&input.archive_id),
        SleepTrack::Consolidator => json!({
            "source": "shared_context",
            "archive_id": input.archive_id,
            "emotional_pass": run.emotional_pass.clone().unwrap_or_else(|| json!({ "emotional_markers": [] })),
            "topic_thread_pass": run.topic_thread_pass.clone().unwrap_or_else(|| json!({ "topic_thread": [] })),
            "personal_signal_pass": run.personal_signal_pass.clone().unwrap_or_else(|| json!({ "personal_signals": [] })),
            "relational_pass": run.relational_pass.clone().unwrap_or_else(|| json!({ "relational_tone": null })),
        }),
    };
    if let Some(error) = state.last_error.as_deref().filter(|_| !state.completed) {
        request.prompt_inputs = add_retry_instruction(&request.prompt_inputs, error);
    }
    request
}

fn current_transactional_batch(
    run: &SleepRun,
    input: &SleepWorkflowInput,
) -> Result<Option<LlmBatch>> {
    validate_sleep_run(run)?;
    validate_run_input_reference(run, input)?;
    if matches!(
        run.stage,
        SleepRunStage::ReadyToFinish | SleepRunStage::Finished
    ) {
        return Ok(None);
    }
    let requests = run
        .requests
        .iter()
        .filter(|state| !state.completed && state_stage(state.track) == run.stage)
        .map(|state| hydrate_request(run, input, state))
        .collect::<Vec<_>>();
    Ok((!requests.is_empty()).then_some(LlmBatch {
        shared_context: Some(render_sleep_prompt_context(
            &input.session_id,
            &input.archive_id,
            &input.events,
        )),
        requests,
    }))
}

fn advance_transactional_sleep_stage(
    run: &mut SleepRun,
    consolidator_request_id: String,
) -> Result<()> {
    match run.stage {
        SleepRunStage::Extraction
            if run
                .requests
                .iter()
                .filter(|state| state_stage(state.track) == SleepRunStage::Extraction)
                .all(|state| state.completed) =>
        {
            run.stage = SleepRunStage::Consolidation;
            ensure_consolidator_request_with_id(run, consolidator_request_id)?;
        }
        SleepRunStage::Consolidation
            if run
                .requests
                .iter()
                .filter(|state| state_stage(state.track) == SleepRunStage::Consolidation)
                .all(|state| state.completed) =>
        {
            run.stage = SleepRunStage::ReadyToFinish;
        }
        _ => {}
    }
    Ok(())
}

fn required_record<W: StorageWorkUnit>(
    work_unit: &mut W,
    key: &RecordKey,
    label: &str,
) -> crate::storage_v2::StorageV2Result<crate::storage_v2::VaultRecord> {
    work_unit.get(key)?.ok_or_else(|| {
        MemoryEngineError::Storage(format!("{label} record not found: {}", key.reference())).into()
    })
}

fn decode_sleep_input(record: crate::storage_v2::VaultRecord) -> Result<SleepWorkflowInput> {
    record.validate()?;
    if record.payload_schema_version != SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: SLEEP_WORKFLOW_INPUT_SCHEMA_VERSION.to_string(),
            actual: record.payload_schema_version,
        });
    }
    if record.revision != 1 {
        return Err(MemoryEngineError::Storage(
            "immutable sleep workflow input must remain at revision 1".to_string(),
        ));
    }
    let input: SleepWorkflowInput = serde_json::from_value(record.payload)?;
    input.validate()?;
    if record.key.record_id != input.sleep_task_id {
        return Err(MemoryEngineError::Storage(
            "sleep workflow input record key does not match task id".to_string(),
        ));
    }
    Ok(input)
}

fn decode_sleep_run(record: crate::storage_v2::VaultRecord) -> Result<SleepRun> {
    record.validate()?;
    if record.payload_schema_version != SLEEP_RUN_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: SLEEP_RUN_SCHEMA_VERSION.to_string(),
            actual: record.payload_schema_version,
        });
    }
    let run: SleepRun = serde_json::from_value(record.payload)?;
    validate_sleep_run(&run)?;
    if record.key.record_id != run.sleep_task_id {
        return Err(MemoryEngineError::Storage(
            "sleep run record key does not match task id".to_string(),
        ));
    }
    Ok(run)
}

fn decode_reserved_session(
    record: crate::storage_v2::VaultRecord,
    session_id: &str,
    sleep_task_id: &str,
    input: &SleepWorkflowInput,
) -> Result<SessionMetadata> {
    record.validate()?;
    if record.payload_schema_version != SESSION_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: SESSION_SCHEMA_VERSION.to_string(),
            actual: record.payload_schema_version,
        });
    }
    let session: SessionMetadata = serde_json::from_value(record.payload)?;
    validate_sleep_session_metadata(&session, session_id)?;
    let input_event_ids = input
        .events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    if session.status != SessionStatus::SleepPending
        || session.active_sleep_task_id.as_deref() != Some(sleep_task_id)
        || session.sleep_pending_event_ids != input_event_ids
    {
        return Err(MemoryEngineError::Storage(
            "Session reservation does not match transactional sleep workflow".to_string(),
        ));
    }
    Ok(session)
}

fn decode_preliminary_archive(
    record: crate::storage_v2::VaultRecord,
    run: &SleepRun,
    input: &SleepWorkflowInput,
) -> Result<ArchiveEntry> {
    record.validate()?;
    if record.payload_schema_version != ARCHIVE_ENTRY_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
            actual: record.payload_schema_version,
        });
    }
    let archive: ArchiveEntry = serde_json::from_value(record.payload)?;
    let input_ids = input
        .events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    if record.key.record_id != archive.archive_id
        || archive.archive_id != run.archive_id
        || archive.source_session_id != run.session_id
        || archive.source_event_ids != input_ids
        || archive.status != ArchiveStatus::Preliminary
    {
        return Err(MemoryEngineError::Storage(
            "preliminary Archive does not match transactional sleep workflow".to_string(),
        ));
    }
    if transactional_support::serializable_digest(&archive)? != input.preliminary_archive_digest {
        return Err(MemoryEngineError::Storage(
            "preliminary Archive changed after Sleep input was frozen".to_string(),
        ));
    }
    Ok(archive)
}

fn decode_workflow_task(
    record: crate::storage_v2::VaultRecord,
    task_type: TaskType,
    run: &SleepRun,
    input: &SleepWorkflowInput,
) -> Result<PendingTask> {
    record.validate()?;
    if record.payload_schema_version != PENDING_TASK_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: PENDING_TASK_SCHEMA_VERSION.to_string(),
            actual: record.payload_schema_version,
        });
    }
    let task: PendingTask = serde_json::from_value(record.payload)?;
    let expected_id = match task_type {
        TaskType::SleepCompression => &run.sleep_task_id,
        TaskType::MemoryUnitPass => run.memory_unit_task_id.as_ref().ok_or_else(|| {
            MemoryEngineError::Storage("sleep run has no memory-unit task".to_string())
        })?,
        _ => {
            return Err(MemoryEngineError::Storage(
                "unsupported transactional sleep task type".to_string(),
            ))
        }
    };
    if task.task_type != task_type
        || task.task_id != *expected_id
        || record.key.record_id != *expected_id
    {
        return Err(MemoryEngineError::Storage(
            "workflow task identity/type does not match SleepRun".to_string(),
        ));
    }
    let (expected_prompt_id, expected_prompt_version, expected_output_schema) = match task_type {
        TaskType::SleepCompression => (
            input.policy.prompt_id.as_str(),
            input.policy.prompt_version,
            SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
        ),
        TaskType::MemoryUnitPass => ("memory_unit_pass", 2, MEMORY_UNITS_RESULT_SCHEMA_VERSION),
        _ => unreachable!("task type was checked above"),
    };
    let expected_attempts = run
        .requests
        .iter()
        .filter(|state| {
            (task_type == TaskType::MemoryUnitPass) == (state.track == SleepTrack::MemoryUnit)
        })
        .map(|state| state.attempts as usize)
        .sum::<usize>();
    let prompt_version_matches = if task_type == TaskType::MemoryUnitPass {
        matches!(task.prompt_version, 1..=3)
    } else {
        task.prompt_version == expected_prompt_version
    };
    if task.schema_version != PENDING_TASK_SCHEMA_VERSION
        || task.created_at != input.created_at
        || task.prompt_id != expected_prompt_id
        || !prompt_version_matches
        || task.role_hint != ModelRole::Balanced
        || task.expected_output_schema != expected_output_schema
        || task.attempts.len() != expected_attempts
        || task.outcome.is_some()
    {
        return Err(MemoryEngineError::Storage(
            "workflow task contract changed outside transactional Sleep".to_string(),
        ));
    }
    let reference: SleepInputReference =
        serde_json::from_value(task.inputs.get("sleep_input_ref").cloned().ok_or_else(|| {
            MemoryEngineError::Storage(
                "workflow task contains inline or missing sleep input".to_string(),
            )
        })?)?;
    if reference != input.reference()
        || task.inputs.get("task_kind").and_then(Value::as_str) != Some(task_type_name(task_type))
    {
        return Err(MemoryEngineError::Storage(
            "workflow task points at another immutable sleep input".to_string(),
        ));
    }
    Ok(task)
}

fn validate_tasks_active(sleep_task: &PendingTask, memory_task: &PendingTask) -> Result<()> {
    if !matches!(sleep_task.state, TaskState::Pending | TaskState::Submitted)
        || !matches!(memory_task.state, TaskState::Pending | TaskState::Submitted)
    {
        return Err(MemoryEngineError::Storage(
            "active SleepRun points at terminal workflow task".to_string(),
        ));
    }
    Ok(())
}

fn validate_frozen_run(run: &SleepRun, sleep_task_id: &str, expected_digest: &str) -> Result<()> {
    if run.sleep_task_id != sleep_task_id
        || transactional_support::serializable_digest(run)? != expected_digest
    {
        return Err(MemoryEngineError::Validation(
            "transactional sleep run changed after its digest was frozen".to_string(),
        ));
    }
    Ok(())
}

fn read_sleep_workflow_input<S: TransactionalStorage>(
    storage: &S,
    context: &VaultContext,
    sleep_task_id: &str,
) -> Result<SleepWorkflowInput> {
    let key = RecordKey::new(SLEEP_INPUT_COLLECTION, sleep_task_id)?;
    let record = storage
        .read_record(context, &key)
        .map_err(MemoryEngineError::from)?
        .ok_or_else(|| {
            MemoryEngineError::Storage(format!("sleep input record not found: {sleep_task_id}"))
        })?;
    decode_sleep_input(record)
}

fn read_sleep_run<S: TransactionalStorage>(
    storage: &S,
    context: &VaultContext,
    sleep_task_id: &str,
) -> Result<SleepRun> {
    let key = RecordKey::new(SLEEP_RUN_COLLECTION, sleep_task_id)?;
    let record = storage
        .read_record(context, &key)
        .map_err(MemoryEngineError::from)?
        .ok_or_else(|| {
            MemoryEngineError::Storage(format!("sleep run record not found: {sleep_task_id}"))
        })?;
    decode_sleep_run(record)
}

fn apply_compression_to_archive(
    archive: &mut ArchiveEntry,
    result: SleepCompressionResult,
    task: &PendingTask,
    now: &str,
) {
    archive.updated_at = now.to_string();
    archive.theme = result.theme;
    archive.tags = result.tags;
    archive.gist = result.gist;
    archive.narrative = result.narrative;
    if result.compact_memory.is_some() {
        archive.compact_memory = result.compact_memory;
    }
    archive.facts = result.facts;
    archive.quotes = result.quotes;
    archive.weight = result.weight;
    archive.links = result.links;
    archive.emotional_markers = result.emotional_markers;
    archive.topic_thread = result.topic_thread;
    archive.personal_signals = result.personal_signals;
    archive.relational_tone = result.relational_tone;
    archive.status = ArchiveStatus::Complete;
    archive.llm_enhanced = true;
    archive.prompt_id = Some(task.prompt_id.clone());
    archive.prompt_version = Some(task.prompt_version);
}

fn build_transactional_memory_units(
    context: &VaultContext,
    sleep_task_id: &str,
    archive: &ArchiveEntry,
    input: &SleepWorkflowInput,
    now: &str,
    result: MemoryUnitPassResult,
) -> Result<Vec<MemoryUnit>> {
    if result.schema_version != MEMORY_UNITS_RESULT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
            actual: result.schema_version,
        });
    }
    if result.archive_id != archive.archive_id {
        return Err(MemoryEngineError::Validation(
            "memory-unit result archive_id does not match SleepRun".to_string(),
        ));
    }
    result.validate_basic()?;
    if result.memory_units.len() > MAX_SLEEP_MEMORY_UNITS_PER_RUN {
        return Err(MemoryEngineError::Validation(format!(
            "memory-unit result exceeds {MAX_SLEEP_MEMORY_UNITS_PER_RUN} units"
        )));
    }
    let allowed = input
        .events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<HashSet<_>>();
    result.validate_source_event_ids(&allowed)?;
    let attribution_events = input
        .events
        .iter()
        .map(AttributionEvent::from)
        .collect::<Vec<_>>();
    result
        .memory_units
        .into_iter()
        .enumerate()
        .map(|(index, draft)| {
            materialize_memory_unit(archive, draft, &attribution_events, now, |thesis| {
                let label = format!("memory_unit:{index}:{thesis}");
                Ok(deterministic_workflow_id(
                    context,
                    sleep_task_id,
                    &label,
                    "mu",
                ))
            })
        })
        .collect()
}

fn merge_unique<I>(target: &mut Vec<String>, values: I)
where
    I: IntoIterator<Item = String>,
{
    let mut merged = target.iter().cloned().collect::<BTreeSet<_>>();
    merged.extend(values);
    *target = merged.into_iter().collect();
}
