use super::erasure_flow::{scope_erasure_lock_key, validate_erasure_target};
use super::scope_inspection::inspection_id_is_valid;
use super::subject_inspection::validate_subject_target;
use super::transactional_support::serializable_digest;
use super::*;
use crate::erasure::{
    erasure_key, SubjectErasurePlan, SubjectErasureReport, SUBJECT_ERASURE_REPORT_SCHEMA,
};
use crate::file_storage::JournaledIngest;

const SUBJECT_ERASURE_INTENT_SCHEMA: &str = "subject_erasure_intent.v1";

#[derive(Serialize, Deserialize)]
struct SubjectEraseIntent {
    schema_version: String,
    receipt_key: String,
    target_digest: String,
    generation: u64,
    plan: SubjectErasurePlan,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_storage::FileStorage;

    #[test]
    fn interrupted_subject_erasure_replays_the_frozen_plan() {
        let root = std::env::temp_dir().join(new_id("recover_subject").unwrap());
        let storage = FileStorage::new(&root);
        let engine = MemoryEngine::new(storage.clone());
        let scope = "scope";
        let session = "session";
        let subject = "person";
        let op_id = "erase_person";
        let event: IngestEvent = serde_json::from_value(json!({
            "schema_version": crate::types::EVENT_SCHEMA_VERSION,
            "type": "message",
            "source": "test",
            "timestamp": "2026-09-10T09:00:00Z",
            "session_id": session,
            "personality_scope_id": scope,
            "memory_text": "An old message.",
            "payload": null,
            "speaker": {"id": subject, "name": "Sam"},
            "quoted_event_ids": []
        }))
        .unwrap();
        let old = engine.ingest(event).unwrap().stored_event;
        let plan = engine.build_subject_erasure_plan(scope, subject).unwrap();
        let key = erasure_key(MemoryErasureKind::PurgeScope, scope);
        let operation_digest = serializable_digest(&("erase_subject", scope, op_id)).unwrap();
        let now = now_rfc3339().unwrap();
        storage
            .begin_journaled_operation(&JournalOperation {
                schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.into(),
                op_id: key.clone(),
                op_type: JournalOperationType::EraseSubject,
                state: JournalState::Started,
                created_at: now.clone(),
                updated_at: now,
                target_files: vec![],
                intent: serde_json::to_value(SubjectEraseIntent {
                    schema_version: SUBJECT_ERASURE_INTENT_SCHEMA.into(),
                    receipt_key: format!("subject_{}", &operation_digest["sha256:".len()..]),
                    target_digest: serializable_digest(&(scope, subject)).unwrap(),
                    generation: 1,
                    plan: plan.clone(),
                })
                .unwrap(),
                recovery_policy: RecoveryPolicy::Retry,
                completed_at: None,
                error: None,
            })
            .unwrap();
        storage.write_erasure_generation(&key, 1).unwrap();
        storage.apply_subject_erasure_plan(&plan).unwrap();
        assert!(storage.has_erasure_intent(&key).unwrap());
        assert!(engine.inspect_subject(scope, subject).is_err());
        drop(engine);

        let restarted = MemoryEngine::new(storage.clone());
        assert_eq!(restarted.recover_pending_memory_operations().unwrap(), 1);
        let report = restarted.erase_subject(scope, subject, op_id).unwrap();
        assert_eq!(report.deleted_events, 1);
        assert_eq!(
            restarted.erase_subject(scope, subject, op_id).unwrap(),
            report
        );
        assert_eq!(storage.read_erasure_generation(&key).unwrap(), 1);
        assert!(!storage.has_erasure_intent(&key).unwrap());
        assert!(restarted
            .read_session(session)
            .unwrap()
            .events
            .iter()
            .all(|event| event.event_id != old.event_id));
        drop(restarted);
        std::fs::remove_dir_all(root).ok();
    }
}

#[derive(Serialize, Deserialize)]
struct SubjectEraseReceipt {
    target_digest: String,
    report: SubjectErasureReport,
}

impl<S: Storage> MemoryEngine<S> {
    pub fn erase_subject(
        &self,
        scope_id: &str,
        subject_id: &str,
        operation_id: &str,
    ) -> Result<SubjectErasureReport> {
        validate_subject_target(scope_id, subject_id)?;
        validate_erasure_target("scope_id", scope_id)?;
        if operation_id.len() > 200 || !inspection_id_is_valid(operation_id) {
            return Err(MemoryEngineError::Validation(
                "invalid operation_id for subject erasure".into(),
            ));
        }
        let receipt_key = serializable_digest(&("erase_subject", scope_id, operation_id))?;
        let target_digest = serializable_digest(&(scope_id, subject_id))?;
        let receipt_key = format!("subject_{}", &receipt_key["sha256:".len()..]);
        self.ensure_manifest()?;
        self.with_scope_memory_write(scope_id, || {
            if let Some(value) = self.storage.read_subject_erasure_receipt(&receipt_key)? {
                let receipt: SubjectEraseReceipt = serde_json::from_value(value)?;
                if receipt.target_digest != target_digest {
                    return Err(MemoryEngineError::OperationConflict);
                }
                if receipt.report.schema_version != SUBJECT_ERASURE_REPORT_SCHEMA {
                    return Err(MemoryEngineError::OperationResultUnavailable);
                }
                return Ok(receipt.report);
            }
            for operation in self.storage.load_journaled_operations()?.items {
                let event = &operation.intent["event"];
                if operation.op_type != JournalOperationType::IngestEvent
                    || operation.state != JournalState::Started
                    || event["personality_scope_id"]
                        .as_str()
                        .or(event["session_id"].as_str())
                        != Some(scope_id)
                {
                    continue;
                }
                let input: JournaledIngest = serde_json::from_value(operation.intent)?;
                self.with_resource_lock(session_lock_key(&input.event.session_id), || {
                    self.require_erasure_complete(
                        MemoryErasureKind::DeleteSession,
                        &input.event.session_id,
                    )?;
                    self.replay_ingest_unlocked(&operation.op_id, &input)
                })?;
            }
            let plan = self.build_subject_erasure_plan(scope_id, subject_id)?;
            let key = erasure_key(MemoryErasureKind::PurgeScope, scope_id);
            let generation = self
                .storage
                .read_erasure_generation(&key)?
                .checked_add(u64::from(plan.has_changes()))
                .ok_or_else(|| MemoryEngineError::Storage("erasure generation exhausted".into()))?;
            let now = now_rfc3339()?;
            let operation = JournalOperation {
                schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.into(),
                op_id: key,
                op_type: JournalOperationType::EraseSubject,
                state: JournalState::Started,
                created_at: now.clone(),
                updated_at: now,
                target_files: vec![],
                intent: serde_json::to_value(SubjectEraseIntent {
                    schema_version: SUBJECT_ERASURE_INTENT_SCHEMA.into(),
                    receipt_key,
                    target_digest,
                    generation,
                    plan,
                })?,
                recovery_policy: RecoveryPolicy::Retry,
                completed_at: None,
                error: None,
            };
            self.storage.begin_journaled_operation(&operation)?;
            self.execute_subject_erasure(operation)
        })
    }

    pub(super) fn recover_subject_erasure(&self, operation: JournalOperation) -> Result<bool> {
        let snapshot: SubjectEraseIntent = serde_json::from_value(operation.intent)?;
        self.with_resource_lock(scope_erasure_lock_key(&snapshot.plan.scope_id), || {
            let Some(current) = self.storage.load_journaled_operation(&operation.op_id)? else {
                return Ok(false);
            };
            // Another erasure can reuse the scope marker after the old one
            // completed while we were waiting for the lock.
            if current.op_type != JournalOperationType::EraseSubject
                || current.created_at != operation.created_at
            {
                return Ok(false);
            }
            self.execute_subject_erasure(current)?;
            Ok(true)
        })
    }

    fn execute_subject_erasure(&self, operation: JournalOperation) -> Result<SubjectErasureReport> {
        let result = self.execute_subject_erasure_inner(&operation);
        if let Err(error) = &result {
            self.memory_operations_recovered
                .store(false, Ordering::Release);
            self.record_memory_operation_error_best_effort(&operation.op_id, error);
        }
        result
    }

    fn execute_subject_erasure_inner(
        &self,
        operation: &JournalOperation,
    ) -> Result<SubjectErasureReport> {
        let intent: SubjectEraseIntent = serde_json::from_value(operation.intent.clone())?;
        let plan = &intent.plan;
        let key = erasure_key(MemoryErasureKind::PurgeScope, &plan.scope_id);
        if intent.schema_version != SUBJECT_ERASURE_INTENT_SCHEMA
            || operation.op_id != key
            || operation.state != JournalState::Started
            || operation.op_type != JournalOperationType::EraseSubject
            || plan.records.target_id != plan.scope_id
            || plan.records.operation != MemoryErasureKind::PurgeScope
        {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        let current = self.storage.read_erasure_generation(&key)?;
        if current > intent.generation {
            return Err(MemoryEngineError::OperationStale);
        }
        let session_keys = plan
            .event_ids_by_session
            .keys()
            .chain(plan.suppressed_event_ids_by_session.keys())
            .chain(plan.detached_quote_event_ids_by_session.keys())
            .map(|session| session_lock_key(session))
            .collect::<BTreeSet<_>>();
        let mut lock_keys = session_keys.into_iter().collect::<Vec<_>>();
        lock_keys.push(format!("archive_scope:{}", plan.scope_id));
        lock_keys.extend(
            plan.records
                .archive_ids
                .iter()
                .chain(&plan.suppressed_archive_ids)
                .map(|id| archive_lock_key(id))
                .collect::<BTreeSet<_>>(),
        );
        lock_keys.extend(
            plan.records
                .core_facts
                .iter()
                .chain(plan.attribution_scrub.iter().flat_map(|scrub| &scrub.facts))
                .map(|fact| core_lock_key(&fact.category))
                .collect::<BTreeSet<_>>(),
        );
        lock_keys.push(vectors_lock_key(&plan.scope_id));
        let locks = lock_keys
            .iter()
            .map(|key| self.locks.resource(key))
            .collect::<Result<Vec<_>>>()?;
        let _guards = lock_keys
            .iter()
            .zip(&locks)
            .map(|(key, lock)| lock_resource(lock, key))
            .collect::<Result<Vec<_>>>()?;
        if current < intent.generation {
            self.storage
                .write_erasure_generation(&key, intent.generation)?;
        }
        let report = match self
            .storage
            .read_subject_erasure_receipt(&intent.receipt_key)?
        {
            Some(value) => {
                let receipt: SubjectEraseReceipt = serde_json::from_value(value)?;
                if receipt.target_digest != intent.target_digest {
                    return Err(MemoryEngineError::OperationConflict);
                }
                receipt.report
            }
            None => {
                self.storage.apply_subject_erasure_plan(plan)?;
                let changed_archives = plan
                    .records
                    .archive_ids
                    .iter()
                    .chain(&plan.suppressed_archive_ids)
                    .collect::<HashSet<_>>();
                self.recall_stats
                    .lock()
                    .map_err(|_| {
                        MemoryEngineError::Storage("recall stats mutex was poisoned".into())
                    })?
                    .retain(|id, _| !changed_archives.contains(id));
                let report = SubjectErasureReport {
                    schema_version: SUBJECT_ERASURE_REPORT_SCHEMA.into(),
                    completed_at: now_rfc3339()?,
                    deleted_events: plan.event_ids_by_session.values().map(Vec::len).sum(),
                    suppressed_events: plan
                        .suppressed_event_ids_by_session
                        .values()
                        .map(Vec::len)
                        .sum(),
                    detached_quotes: plan
                        .detached_quote_event_ids_by_session
                        .values()
                        .map(Vec::len)
                        .sum(),
                    suppressed_archives: plan.suppressed_archive_ids.len(),
                    deleted_archives: plan.records.archive_ids.len(),
                    deleted_memory_units: plan.records.memory_unit_ids.len(),
                    deleted_core_facts: plan.records.core_facts.len(),
                    deleted_candidates: plan.records.candidate_ids.len(),
                    deleted_hypotheses: plan.records.hypothesis_node_ids.len(),
                    retired_tasks: plan.records.task_ids.len(),
                    incomplete: plan.incomplete,
                };
                self.storage.write_subject_erasure_receipt(
                    &intent.receipt_key,
                    &serde_json::to_value(SubjectEraseReceipt {
                        target_digest: intent.target_digest,
                        report: report.clone(),
                    })?,
                )?;
                report
            }
        };
        self.storage.delete_journaled_operation(&operation.op_id)?;
        Ok(report)
    }
}
