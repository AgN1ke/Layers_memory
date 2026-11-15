//! Memory Engine core.
//!
//! This crate starts with the data contracts from `wiki/pages/foundation/contracts.md`.
//! The first implementation step is intentionally small: typed contracts,
//! storage boundaries, and serialization tests.

pub mod archive;
mod build_info;
pub mod config;
pub mod core_store;
pub mod engine;
pub mod erasure;
pub mod error;
pub mod event;
pub mod fidelity;
pub mod file_storage;
pub mod forgetting;
pub mod hypothesis;
pub mod journal;
pub mod llm;
pub mod manifest;
pub mod prompt_view;
pub mod query_json;
pub mod recall;
pub mod reflection;
pub mod server_runtime;
pub mod session;
pub mod sleep;
pub mod storage;
pub mod storage_v2;
pub mod tasks;
pub mod types;
pub mod vector;

pub use build_info::{build_info, BuildInfo};
pub use engine::{
    ContextPackageConfig, EngineOptions, EventScoringConfig, IngestResult, MemoryEngine,
    RecallStage1Config, ReflectionConfig, SleepStage1Config, SleepStage1Result, SleepWorkflowInput,
    TransactionalCorePatchRequest, TransactionalCorePatchResult, TransactionalCoreUpsertRequest,
    TransactionalCoreUpsertResult, TransactionalIngestRequest, TransactionalIngestResult,
    TransactionalRecallRequest, TransactionalRecallResult, TransactionalSleepCancelRequest,
    TransactionalSleepCancelResult, TransactionalSleepFinalizeRequest,
    TransactionalSleepFinalizeResult, TransactionalSleepPostFinalizeState,
    TransactionalSleepStartRequest, TransactionalSleepStartResult, TransactionalSleepSubmitRequest,
    TransactionalSleepSubmitResult, MAX_SLEEP_MEMORY_UNITS_PER_RUN,
    MAX_SLEEP_SUBMIT_RESPONSE_BYTES, MAX_SLEEP_WORKFLOW_INPUT_BYTES,
};
pub use erasure::{MemoryErasureCounts, MemoryErasureKind, MemoryErasureReport};
pub use error::{MemoryEngineError, Result};
pub use file_storage::FileStorage;
pub use llm::{
    CoreArchiveSeedSummary, CoreSignalSummary, LlmBatch, LlmErrorKind, LlmRequest, LlmResponse,
    LlmSharedContext, SleepOutcome, SleepPassReport, SleepRequestState, SleepRun, SleepRunOptions,
    SleepRunStage, SleepRunStep, SleepTrack,
};
pub use prompt_view::{render_memory_prompt_plan, render_memory_view, MemoryPromptPlan};
pub use server_runtime::{
    AccountId, AccountLinkDecision, AccountVaultMembership, AuthenticatedPrincipal,
    ClientRequestId, ExternalIdentityProvider, LocalVaultHandle, MembershipStatus, PrincipalKind,
    TurnAdmission, TurnAdmissionKind, TurnOperation, TurnOperationStatus, TurnRequest,
    TurnTerminalOutcome, TurnTerminalResult, VaultContext, VaultId, VaultTurnLedger,
};
pub use storage_v2::{
    execute_work_unit_with_retry, BeginWorkUnit, ExpectedRevision, InMemoryCommitFault,
    InMemoryTransactionalStorage, RecordKey, StorageV2Error, StorageV2Result, StorageWorkUnit,
    TransactionalStorage, VaultRecord, VaultSnapshotQuery, WorkUnitCommit, WorkUnitIntent,
    WorkUnitReceipt,
};
pub use vector::{
    DeepRecallHit, DeepRecallQuery, DeepRecallResult, EmbedBatchInputs, EmbedBatchItem,
    EmbedBatchResult, EmbedBatchVector, VectorScopeState, VectorScopeStatus,
};
