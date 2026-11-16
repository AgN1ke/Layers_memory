use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;
use crate::server_runtime::VaultContext;
use crate::storage_v2::{RecordKey, TransactionalStorage, WorkUnitReceipt};

pub(crate) fn serializable_digest<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[derive(Serialize, Deserialize)]
pub(super) struct DecisionReceipt<T> {
    request_digest: String,
    result_digest: String,
    pub(super) result: T,
}

impl<T: Serialize> DecisionReceipt<T> {
    pub(super) fn new(request_digest: String, result: T) -> Result<Self> {
        Ok(Self {
            request_digest,
            result_digest: serializable_digest(&result)?,
            result,
        })
    }

    pub(super) fn replay(self, request_digest: &str) -> Result<T> {
        if self.request_digest != request_digest {
            return Err(MemoryEngineError::OperationConflict);
        }
        if self.result_digest != serializable_digest(&self.result)? {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        Ok(self.result)
    }
}

pub(super) fn read_transaction_result<S: TransactionalStorage, T: DeserializeOwned + Serialize>(
    storage: &S,
    context: &VaultContext,
    key: &RecordKey,
    receipt: &WorkUnitReceipt,
    expected_payload_schema: &str,
) -> Result<T> {
    if receipt.result_reference != key.reference() {
        return Err(MemoryEngineError::Storage(
            "committed receipt points to an unexpected result record".to_string(),
        ));
    }
    let record = storage
        .read_record(context, key)
        .map_err(MemoryEngineError::from)?
        .ok_or_else(|| {
            MemoryEngineError::Storage("committed receipt has no durable result record".to_string())
        })?;
    record.validate()?;
    if record.payload_schema_version != expected_payload_schema {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: expected_payload_schema.to_string(),
            actual: record.payload_schema_version,
        });
    }
    let result: T = serde_json::from_value(record.payload)?;
    if receipt.result_digest != serializable_digest(&result)? {
        return Err(MemoryEngineError::Storage(
            "committed result digest does not match its receipt".to_string(),
        ));
    }
    Ok(result)
}
