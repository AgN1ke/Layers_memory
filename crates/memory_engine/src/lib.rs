//! Memory Engine core.
//!
//! This crate starts with the data contracts from `docs/contracts.md`.
//! The first implementation step is intentionally small: typed contracts,
//! storage boundaries, and serialization tests.

pub mod archive;
pub mod config;
pub mod core_store;
pub mod engine;
pub mod error;
pub mod event;
pub mod file_storage;
pub mod journal;
pub mod manifest;
pub mod recall;
pub mod session;
pub mod sleep;
pub mod storage;
pub mod tasks;
pub mod types;

pub use engine::{
    ContextPackageConfig, EngineOptions, EventScoringConfig, IngestResult, MemoryEngine,
    RecallStage1Config, SleepStage1Config, SleepStage1Result,
};
pub use error::{MemoryEngineError, Result};
pub use file_storage::FileStorage;
