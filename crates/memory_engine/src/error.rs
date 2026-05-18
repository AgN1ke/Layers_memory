use thiserror::Error;

pub type Result<T> = std::result::Result<T, MemoryEngineError>;

#[derive(Debug, Error)]
pub enum MemoryEngineError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("incompatible schema version: expected {expected}, got {actual}")]
    IncompatibleSchema { expected: String, actual: String },

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
