use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use super::*;
use crate::server_runtime::VaultContext;
use crate::storage_v2::{
    execute_work_unit_with_retry, ExpectedRevision, RecordKey, StorageWorkUnit,
    TransactionalStorage, VaultSnapshotQuery, WorkUnitCommit, WorkUnitIntent, WorkUnitReceipt,
    DEFAULT_WORK_UNIT_MAX_ATTEMPTS, WORK_UNIT_INTENT_SCHEMA_VERSION,
};

pub const TRANSACTIONAL_RECALL_REQUEST_SCHEMA_VERSION: &str = "transactional_recall_request.v1";
pub const TRANSACTIONAL_RECALL_RESULT_SCHEMA_VERSION: &str = "transactional_recall_result.v1";
pub const RECALL_WORK_UNIT_KIND: &str = "recall_stage1";

const OPERATION_RESULT_COLLECTION: &str = "operation_results";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalRecallRequest {
    pub schema_version: String,
    pub work_unit: WorkUnitIntent,
    pub query: RecallQuery,
}

impl TransactionalRecallRequest {
    pub fn new(
        operation_id: impl Into<String>,
        started_at: impl Into<String>,
        created_at: impl Into<String>,
        mut query: RecallQuery,
    ) -> Result<Self> {
        query.created_at = Some(created_at.into());
        let request_digest = transactional_support::serializable_digest(&query)?;
        let request = Self {
            schema_version: TRANSACTIONAL_RECALL_REQUEST_SCHEMA_VERSION.to_string(),
            work_unit: WorkUnitIntent {
                schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.to_string(),
                operation_id: operation_id.into(),
                operation_kind: RECALL_WORK_UNIT_KIND.to_string(),
                request_digest,
                started_at: started_at.into(),
            },
            query,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != TRANSACTIONAL_RECALL_REQUEST_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: TRANSACTIONAL_RECALL_REQUEST_SCHEMA_VERSION.to_string(),
                actual: self.schema_version.clone(),
            });
        }
        self.work_unit.validate()?;
        if self.work_unit.operation_kind != RECALL_WORK_UNIT_KIND {
            return Err(MemoryEngineError::Validation(format!(
                "transactional recall requires operation_kind {RECALL_WORK_UNIT_KIND}"
            )));
        }
        validate_recall_query(&self.query)?;
        let created_at = self.query.created_at.as_deref().ok_or_else(|| {
            MemoryEngineError::Validation(
                "transactional recall query.created_at must be frozen".to_string(),
            )
        })?;
        if parse_rfc3339(created_at).is_none() {
            return Err(MemoryEngineError::Validation(
                "transactional recall query.created_at must be RFC3339".to_string(),
            ));
        }
        if self.query.query_embedding.is_some() {
            return Err(MemoryEngineError::Validation(
                "transactional recall Stage 2 waits for the Storage v2 vector slice".to_string(),
            ));
        }
        let expected = transactional_support::serializable_digest(&self.query)?;
        if self.work_unit.request_digest != expected {
            return Err(MemoryEngineError::Validation(
                "transactional recall request_digest does not match its query".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionalRecallResult {
    pub schema_version: String,
    pub recall: RecallResult,
    pub receipt: WorkUnitReceipt,
}

impl<S: TransactionalStorage> MemoryEngine<S> {
    /// Ranks one committed Archive snapshot and atomically records feedback for
    /// exactly the selected entries. Storage v1 keeps its buffered single-writer
    /// path until host migration; this method never consults replica-local deltas.
    pub fn recall_transactional(
        &self,
        context: &VaultContext,
        request: TransactionalRecallRequest,
    ) -> Result<TransactionalRecallResult> {
        context.validate()?;
        request.validate()?;

        let query = request.query;
        let created_at = query
            .created_at
            .clone()
            .expect("validated transactional recall timestamp");
        let intent = request.work_unit;
        let result_key = RecordKey::new(OPERATION_RESULT_COLLECTION, intent.operation_id.clone())?;
        let result_reference = result_key.reference();
        let recall_config = self.options.recall.clone();
        let stage2_candidate_limit = self.options.vectors.stage2_candidate_limit;

        let receipt = execute_work_unit_with_retry(
            &self.storage,
            context,
            intent,
            DEFAULT_WORK_UNIT_MAX_ATTEMPTS,
            |work_unit| {
                let archive_enabled = query.filters.source_layers.is_empty()
                    || query
                        .filters
                        .source_layers
                        .contains(&RecallSourceLayer::Archive);
                let records = if archive_enabled {
                    work_unit.query_snapshot(&VaultSnapshotQuery::Archives {
                        personality_scope_id: query.personality_scope_id.clone(),
                        source_session_id: query
                            .personality_scope_id
                            .is_none()
                            .then(|| query.session_id.clone())
                            .flatten(),
                    })?
                } else {
                    Vec::new()
                };
                let mut records_by_archive = BTreeMap::new();
                let mut archive_entries = Vec::with_capacity(records.len());
                for record in records {
                    record.validate()?;
                    if record.payload_schema_version != ARCHIVE_ENTRY_SCHEMA_VERSION {
                        return Err(MemoryEngineError::IncompatibleSchema {
                            expected: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                            actual: record.payload_schema_version,
                        }
                        .into());
                    }
                    let entry: ArchiveEntry = serde_json::from_value(record.payload.clone())?;
                    if entry.schema_version != ARCHIVE_ENTRY_SCHEMA_VERSION {
                        return Err(MemoryEngineError::IncompatibleSchema {
                            expected: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                            actual: entry.schema_version,
                        }
                        .into());
                    }
                    if record.key.record_id != entry.archive_id {
                        return Err(MemoryEngineError::Storage(
                            "Archive record key does not match payload archive_id".to_string(),
                        )
                        .into());
                    }
                    records_by_archive.insert(entry.archive_id.clone(), record);
                    archive_entries.push(entry);
                }

                let ranking =
                    rank_stage1_archives(archive_entries, &query, &created_at, &recall_config);
                let limit = if query.limit == 0 {
                    recall_config.default_limit
                } else {
                    query.limit
                };
                let stage1_ranks = ranking
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, (entry, _))| (entry.archive_id.clone(), index + 1))
                    .collect::<HashMap<_, _>>();
                let stage2_candidate_count =
                    ranking.entries.len().min(stage2_candidate_limit.max(1));
                let debug_limit = ranking.entries.len().min(stage2_candidate_count.max(limit));
                let candidates = if query.explain {
                    ranking
                        .entries
                        .iter()
                        .take(debug_limit)
                        .enumerate()
                        .map(|(index, (entry, scored))| RecallCandidateDebug {
                            archive_id: entry.archive_id.clone(),
                            stage1_rank: stage1_ranks[&entry.archive_id],
                            final_rank: index + 1,
                            selected: index < limit,
                            stage1_score: scored.stage1_score,
                            matched_memory_unit_id: None,
                            semantic_similarity: None,
                            final_score: scored.score,
                            components: scored.components.clone(),
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                let candidate_count = ranking.candidate_count;
                let filtered_count = ranking.filtered_count;
                let selected = ranking.entries.into_iter().take(limit).collect::<Vec<_>>();
                let items = selected
                    .iter()
                    .cloned()
                    .map(|(entry, scored)| recall_item_from_archive(entry, scored, query.explain))
                    .collect::<Vec<_>>();
                let recall = RecallResult {
                    schema_version: RECALL_RESULT_SCHEMA_VERSION.to_string(),
                    query_id: query.query_id.clone(),
                    created_at: created_at.clone(),
                    stage_used: RecallStage::Stage1,
                    items,
                    notes: Vec::new(),
                    debug: query.explain.then_some(RecallDebug {
                        stage_used: RecallStage::Stage1,
                        candidate_count,
                        filtered_count,
                        notes: Vec::new(),
                        candidates,
                        vector_stage: Some(RecallVectorStageDebug {
                            status: RecallVectorStageStatus::NotRequested,
                            model_id: None,
                            reason: Some("query_embedding was not provided".to_string()),
                            candidate_limit: stage2_candidate_limit,
                            candidate_count: stage2_candidate_count,
                            matched_archive_count: 0,
                            vector_weight: self.options.vectors.stage2_vector_weight.clamp(0.0, 1.0)
                                as f64,
                            stage1_weight: 1.0
                                - self.options.vectors.stage2_vector_weight.clamp(0.0, 1.0) as f64,
                        }),
                    }),
                };
                let result_digest = transactional_support::serializable_digest(&recall)?;

                for (mut entry, _) in selected {
                    let record = records_by_archive
                        .remove(&entry.archive_id)
                        .ok_or_else(|| {
                            MemoryEngineError::Storage(
                                "ranked Archive entry lost its source record".to_string(),
                            )
                        })?;
                    entry.recall_count = entry.recall_count.saturating_add(1);
                    entry.last_recalled_at =
                        newest_timestamp(entry.last_recalled_at.as_deref(), Some(&created_at));
                    work_unit.put(
                        record.key,
                        ExpectedRevision::Exact(record.revision),
                        ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
                        serde_json::to_value(&entry)?,
                    )?;
                }
                work_unit.put(
                    result_key.clone(),
                    ExpectedRevision::Absent,
                    RECALL_RESULT_SCHEMA_VERSION.to_string(),
                    serde_json::to_value(&recall)?,
                )?;
                Ok(WorkUnitCommit {
                    committed_at: created_at.clone(),
                    result_reference: result_reference.clone(),
                    result_digest,
                })
            },
        )
        .map_err(MemoryEngineError::from)?;

        let recall = transactional_support::read_transaction_result::<S, RecallResult>(
            &self.storage,
            context,
            &result_key,
            &receipt,
            RECALL_RESULT_SCHEMA_VERSION,
        )?;
        Ok(TransactionalRecallResult {
            schema_version: TRANSACTIONAL_RECALL_RESULT_SCHEMA_VERSION.to_string(),
            recall,
            receipt,
        })
    }
}
