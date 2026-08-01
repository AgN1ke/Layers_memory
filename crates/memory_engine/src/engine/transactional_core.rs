use serde::Serialize;
use sha2::{Digest, Sha256};

use super::*;
use crate::server_runtime::VaultContext;
use crate::storage_v2::{
    execute_work_unit_with_retry, ExpectedRevision, RecordKey, StorageWorkUnit,
    TransactionalStorage, WorkUnitCommit, WorkUnitIntent, WorkUnitReceipt,
    DEFAULT_WORK_UNIT_MAX_ATTEMPTS, WORK_UNIT_INTENT_SCHEMA_VERSION,
};

pub const TRANSACTIONAL_CORE_UPSERT_REQUEST_SCHEMA_VERSION: &str =
    "transactional_core_upsert_request.v1";
pub const TRANSACTIONAL_CORE_UPSERT_RESULT_SCHEMA_VERSION: &str =
    "transactional_core_upsert_result.v1";
pub const TRANSACTIONAL_CORE_PATCH_REQUEST_SCHEMA_VERSION: &str =
    "transactional_core_patch_request.v1";
pub const TRANSACTIONAL_CORE_PATCH_RESULT_SCHEMA_VERSION: &str =
    "transactional_core_patch_result.v1";
pub const CORE_UPSERT_WORK_UNIT_KIND: &str = "core_fact_upsert";
pub const CORE_PATCH_WORK_UNIT_KIND: &str = "core_fact_patch";

const CORE_CATEGORY_COLLECTION: &str = "core_categories";
const OPERATION_RESULT_COLLECTION: &str = "operation_results";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalCoreUpsertRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub applied_at: String,
    pub input: CoreFactInput,
}

impl TransactionalCoreUpsertRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        applied_at: impl Into<String>,
        input: CoreFactInput,
    ) -> Result<Self> {
        let applied_at = applied_at.into();
        let request_digest = transactional_core_digest(&applied_at, None, &input)?;
        let request = Self {
            schema_version: TRANSACTIONAL_CORE_UPSERT_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: WorkUnitIntent {
                schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
                operation_id: operation_id.into(),
                operation_kind: CORE_UPSERT_WORK_UNIT_KIND.to_string(),
                request_digest,
                started_at: started_at.into(),
            },
            applied_at,
            input,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        expect_request_schema(
            &self.schema_version,
            TRANSACTIONAL_CORE_UPSERT_REQUEST_SCHEMA_VERSION,
        )?;
        validate_transaction_intent(&self.work_unit, CORE_UPSERT_WORK_UNIT_KIND)?;
        validate_applied_at(&self.applied_at)?;
        validate_core_fact_input(&self.input)?;
        let expected = transactional_core_digest(&self.applied_at, None, &self.input)?;
        if self.work_unit.request_digest != expected {
            return Err(MemoryEngineError::Validation(
                "transactional Core upsert request_digest does not match its domain input"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalCoreUpsertResult {
    pub schema_version: String,
    pub upsert: CoreFactUpsertResult,
    pub receipt: WorkUnitReceipt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalCorePatchRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    /// Explicit category turns patch into one point mutation; server storage
    /// never scans every Core category to locate a fact.
    pub category: String,
    pub applied_at: String,
    pub input: CoreFactPatchInput,
}

impl TransactionalCorePatchRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        category: impl Into<String>,
        applied_at: impl Into<String>,
        input: CoreFactPatchInput,
    ) -> Result<Self> {
        let category = normalize_whitespace(&category.into());
        let applied_at = applied_at.into();
        let request_digest = transactional_core_digest(&applied_at, Some(&category), &input)?;
        let request = Self {
            schema_version: TRANSACTIONAL_CORE_PATCH_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: WorkUnitIntent {
                schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
                operation_id: operation_id.into(),
                operation_kind: CORE_PATCH_WORK_UNIT_KIND.to_string(),
                request_digest,
                started_at: started_at.into(),
            },
            category,
            applied_at,
            input,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        expect_request_schema(
            &self.schema_version,
            TRANSACTIONAL_CORE_PATCH_REQUEST_SCHEMA_VERSION,
        )?;
        validate_transaction_intent(&self.work_unit, CORE_PATCH_WORK_UNIT_KIND)?;
        validate_applied_at(&self.applied_at)?;
        validate_core_fact_patch_input(&self.input)?;
        if self.category != normalize_whitespace(&self.category) || self.category.is_empty() {
            return Err(MemoryEngineError::Validation(
                "transactional Core patch category must be normalized and non-empty".to_string(),
            ));
        }
        let expected =
            transactional_core_digest(&self.applied_at, Some(&self.category), &self.input)?;
        if self.work_unit.request_digest != expected {
            return Err(MemoryEngineError::Validation(
                "transactional Core patch request_digest does not match its domain input"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalCorePatchResult {
    pub schema_version: String,
    pub patch: CoreFactPatchResult,
    pub receipt: WorkUnitReceipt,
}

impl<S: TransactionalStorage> MemoryEngine<S> {
    pub fn upsert_core_fact_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalCoreUpsertRequest,
    ) -> Result<TransactionalCoreUpsertResult> {
        context.validate()?;
        request.validate()?;

        let category_name = normalize_whitespace(&request.input.category);
        let category_key = core_category_key(&category_name)?;
        let result_key = operation_result_key(&request.work_unit.operation_id)?;
        let new_fact_id = deterministic_core_fact_id(context, &request.work_unit);
        let applied_at = request.applied_at.clone();
        let input = request.input.clone();
        let intent = request.work_unit;
        let result_reference = result_key.reference();

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let (category, expected) =
                    read_core_category(work_unit, &category_key, &category_name, &applied_at)?;
                let (category, result) = core_policy::apply_core_fact_upsert(
                    category,
                    &input,
                    category_name.clone(),
                    || Ok(new_fact_id.clone()),
                    applied_at.clone(),
                )?;
                let result_digest = transactional_support::serializable_digest(&result)?;
                work_unit.put(
                    category_key.clone(),
                    expected,
                    CORE_STORE_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&category)?,
                )?;
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&result)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: applied_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let upsert = transactional_support::read_transaction_result::<S, CoreFactUpsertResult>(
            &self.storage,
            context,
            &result_key,
            &receipt,
            CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION,
        )?;
        Ok(TransactionalCoreUpsertResult {
            schema_version: TRANSACTIONAL_CORE_UPSERT_RESULT_SCHEMA_VERSION.to_string(),
            upsert,
            receipt,
        })
    }

    pub fn patch_core_fact_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalCorePatchRequest,
    ) -> Result<TransactionalCorePatchResult> {
        context.validate()?;
        request.validate()?;

        let category_name = request.category.clone();
        let category_key = core_category_key(&category_name)?;
        let result_key = operation_result_key(&request.work_unit.operation_id)?;
        let applied_at = request.applied_at.clone();
        let input = request.input.clone();
        let intent = request.work_unit;
        let result_reference = result_key.reference();

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let record = work_unit.get(&category_key)?.ok_or_else(|| {
                    MemoryEngineError::Validation(format!(
                        "Core category not found for patch: {category_name}"
                    ))
                })?;
                record.validate()?;
                let expected = ExpectedRevision::Exact(record.revision);
                let category: CoreStoreCategory = serde_json::from_value(record.payload)?;
                validate_stored_category(&category, &category_name)?;
                let (category, result) = core_policy::apply_core_fact_patch(
                    category,
                    &input,
                    category_name.clone(),
                    applied_at.clone(),
                )?;
                let result_digest = transactional_support::serializable_digest(&result)?;
                work_unit.put(
                    category_key.clone(),
                    expected,
                    CORE_STORE_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&category)?,
                )?;
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    CORE_FACT_PATCH_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&result)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: applied_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let patch = transactional_support::read_transaction_result::<S, CoreFactPatchResult>(
            &self.storage,
            context,
            &result_key,
            &receipt,
            CORE_FACT_PATCH_RESULT_SCHEMA_VERSION,
        )?;
        Ok(TransactionalCorePatchResult {
            schema_version: TRANSACTIONAL_CORE_PATCH_RESULT_SCHEMA_VERSION.to_string(),
            patch,
            receipt,
        })
    }
}

fn read_core_category<W: StorageWorkUnit>(
    work_unit: &mut W,
    key: &RecordKey,
    category_name: &str,
    applied_at: &str,
) -> crate::storage_v2::StorageV2Result<(CoreStoreCategory, ExpectedRevision)> {
    match work_unit.get(key)? {
        Some(record) => {
            record.validate()?;
            let expected = ExpectedRevision::Exact(record.revision);
            let category: CoreStoreCategory = serde_json::from_value(record.payload)?;
            validate_stored_category(&category, category_name)?;
            Ok((category, expected))
        }
        None => Ok((
            CoreStoreCategory {
                schema_version: CORE_STORE_SCHEMA_VERSION.to_string(),
                category: category_name.to_string(),
                updated_at: applied_at.to_string(),
                facts: Vec::new(),
            },
            ExpectedRevision::Absent,
        )),
    }
}

fn validate_stored_category(category: &CoreStoreCategory, expected_name: &str) -> Result<()> {
    if category.schema_version != CORE_STORE_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_STORE_SCHEMA_VERSION.to_string(),
            actual: category.schema_version.clone(),
        });
    }
    if category.category != expected_name {
        return Err(MemoryEngineError::Storage(
            "Core category record key does not match payload category".to_string(),
        ));
    }
    Ok(())
}

fn validate_transaction_intent(intent: &WorkUnitIntent, expected_kind: &str) -> Result<()> {
    intent.validate()?;
    if intent.operation_kind != expected_kind {
        return Err(MemoryEngineError::Validation(format!(
            "transactional Core operation requires operation_kind {expected_kind}"
        )));
    }
    Ok(())
}

fn validate_applied_at(value: &str) -> Result<()> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
        MemoryEngineError::Validation(format!(
            "transactional Core applied_at must be RFC3339: {error}"
        ))
    })?;
    Ok(())
}

fn expect_request_schema(actual: &str, expected: &str) -> Result<()> {
    if actual != expected {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

#[derive(Serialize)]
struct TransactionalCoreDigestInput<'a, T> {
    applied_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a str>,
    input: &'a T,
}

fn transactional_core_digest<T: Serialize>(
    applied_at: &str,
    category: Option<&str>,
    input: &T,
) -> Result<String> {
    transactional_support::serializable_digest(&TransactionalCoreDigestInput {
        applied_at,
        category,
        input,
    })
}

fn core_category_key(category: &str) -> Result<RecordKey> {
    let digest = Sha256::digest(category.as_bytes());
    RecordKey::new(CORE_CATEGORY_COLLECTION, format!("category_{digest:x}"))
}

fn operation_result_key(operation_id: &str) -> Result<RecordKey> {
    RecordKey::new(OPERATION_RESULT_COLLECTION, operation_id.to_string())
}

fn deterministic_core_fact_id(context: &VaultContext, intent: &WorkUnitIntent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(context.vault_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(intent.operation_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.request_digest.as_bytes());
    format!("core_fact_{:x}", hasher.finalize())
}
