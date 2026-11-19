use thiserror::Error;

pub type Result<T> = std::result::Result<T, MemoryEngineError>;

#[derive(Debug, Error)]
pub enum MemoryEngineError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("operation ID was already used with different input")]
    OperationConflict,

    #[error("the stored result of this operation is no longer available")]
    OperationResultUnavailable,

    #[error("a journaled operation is incomplete")]
    OperationIncomplete,

    #[error("the operation belongs to an erased memory generation")]
    OperationStale,

    #[error("review basis changed or could not be verified; start a new review")]
    NeedsNewReview,

    #[error("incompatible schema version: expected {expected}, got {actual}")]
    IncompatibleSchema { expected: String, actual: String },

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
