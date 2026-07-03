use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive::MemoryUnit;
use crate::types::{Id, Timestamp};
use crate::{MemoryEngineError, Result};

pub const VECTOR_INDEX_SCHEMA_VERSION: &str = "vector_index.v1";
pub const EMBED_BATCH_RESULT_SCHEMA_VERSION: &str = "embed_batch_result.v1";
pub const DEEP_RECALL_RESULT_SCHEMA_VERSION: &str = "deep_recall.v1";
pub const DEFAULT_VECTOR_MODEL_ID: &str =
    "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2";
pub const DEFAULT_VECTOR_DIM: usize = 384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorScopeStatus {
    Disabled,
    Building,
    Ready,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorMetric {
    Cosine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorIndexManifest {
    pub schema_version: String,
    pub model_id: String,
    pub dim: usize,
    pub metric: VectorMetric,
    pub normalized: bool,
    pub rows: usize,
    pub state: VectorScopeStatus,
    pub built_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backfill_cursor: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorRow {
    pub row: usize,
    pub memory_unit_id: Id,
    pub archive_id: Id,
    pub created_at: Timestamp,
    pub thesis_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorTombstone {
    pub memory_unit_id: Id,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorIndexData {
    pub manifest: VectorIndexManifest,
    pub rows: Vec<VectorRow>,
    pub vectors: Vec<Vec<f32>>,
    pub tombstones: Vec<VectorTombstone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorScopeState {
    pub schema_version: String,
    pub scope: String,
    pub status: VectorScopeStatus,
    pub rows: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dim: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepRecallQuery {
    pub scope: String,
    pub query_vec: Vec<f32>,
    pub model_id: String,
    #[serde(default)]
    pub top_k: usize,
    #[serde(default)]
    pub min_sim: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepRecallHit {
    pub memory_unit_id: Id,
    pub archive_id: Id,
    pub thesis: String,
    pub created_at: Timestamp,
    pub sim: f32,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepRecallResult {
    pub schema_version: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<DeepRecallHit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedBatchInputs {
    pub kind: String,
    pub scope: String,
    pub model_id: String,
    pub dim: usize,
    pub items: Vec<EmbedBatchItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedBatchItem {
    pub memory_unit_id: Id,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedBatchResult {
    pub schema_version: String,
    pub model_id: String,
    pub dim: usize,
    pub results: Vec<EmbedBatchVector>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedBatchVector {
    pub memory_unit_id: Id,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorAppendRecord {
    pub row: VectorRow,
    pub vector: Vec<f32>,
}

pub fn default_vector_manifest(scope_model_id: &str, dim: usize, now: &str) -> VectorIndexManifest {
    VectorIndexManifest {
        schema_version: VECTOR_INDEX_SCHEMA_VERSION.to_string(),
        model_id: scope_model_id.to_string(),
        dim,
        metric: VectorMetric::Cosine,
        normalized: true,
        rows: 0,
        state: VectorScopeStatus::Building,
        built_at: now.to_string(),
        updated_at: now.to_string(),
        backfill_cursor: None,
    }
}

pub fn thesis_hash(thesis: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(thesis.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

pub fn normalize_vector(mut vector: Vec<f32>, expected_dim: usize) -> Result<Vec<f32>> {
    if vector.len() != expected_dim {
        return Err(MemoryEngineError::Validation(format!(
            "embedding dim mismatch: expected {expected_dim}, got {}",
            vector.len()
        )));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(MemoryEngineError::Validation(
            "embedding vector contains non-finite value".to_string(),
        ));
    }
    let norm = vector
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return Err(MemoryEngineError::Validation(
            "embedding vector must not be zero".to_string(),
        ));
    }
    for value in &mut vector {
        *value = (*value as f64 / norm) as f32;
    }
    Ok(vector)
}

pub fn memory_unit_is_vector_eligible(unit: &MemoryUnit) -> bool {
    use crate::archive::{FidelityStatus, MemoryUnitStatus};

    unit.status == MemoryUnitStatus::ActiveArchive
        && !matches!(
            unit.fidelity_status,
            FidelityStatus::Distorted | FidelityStatus::Unsupported | FidelityStatus::NeedsRevision
        )
}
