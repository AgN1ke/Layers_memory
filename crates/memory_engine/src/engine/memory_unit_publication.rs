use super::*;
use crate::storage::memory_unit_publication_key;

const PUBLICATION_SCHEMA: &str = "memory_unit_task_outcome.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PublicationReceipt {
    schema_version: String,
    archive_id: String,
    task_id: String,
    request_digest: String,
    memory_unit_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PublicationIntent {
    receipt: PublicationReceipt,
    source_session_id: String,
    scope: String,
    units: Vec<MemoryUnit>,
    last_error: Option<String>,
}

impl<S: Storage> MemoryEngine<S> {
    pub(super) fn memory_unit_publication_exists(&self, archive_id: &str) -> Result<bool> {
        let Some(value) = self
            .storage
            .read_memory_unit_publication_receipt(archive_id)?
        else {
            return Ok(false);
        };
        let receipt: PublicationReceipt = serde_json::from_value(value)?;
        if receipt.schema_version != PUBLICATION_SCHEMA || receipt.archive_id != archive_id {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        Ok(true)
    }

    pub(super) fn publish_memory_units(
        &self,
        task: &PendingTask,
        archive_id: &str,
        request_digest: String,
        units: Vec<MemoryUnit>,
        last_error: Option<String>,
    ) -> Result<ArchiveEntry> {
        self.with_resource_lock(archive_lock_key(archive_id), || {
            let key = memory_unit_publication_key(archive_id);
            if let Some(operation) = self.storage.load_journaled_operation(&key)? {
                let intent = publication_intent(&operation)?;
                intent.receipt.require_request(task, &request_digest)?;
                return self.finish_memory_unit_publication(&operation);
            }

            if let Some(stored) = self
                .storage
                .read_memory_unit_publication_receipt(archive_id)?
                .or_else(|| task.outcome.clone())
            {
                let receipt: PublicationReceipt = serde_json::from_value(stored)?;
                receipt.require_request(task, &request_digest)?;
                return self.read_published_memory_units(&receipt);
            }
            if task.state == TaskState::Completed {
                // A legacy completion lacks an exact batch receipt. Preserve
                // the old read API and leave classification to migration.
                return self.storage.read_archive_entry_by_id(archive_id);
            }

            let archive = self.storage.read_archive_entry_raw(archive_id)?;
            if archive.status == ArchiveStatus::Superseded || task.state == TaskState::Cancelled {
                return Err(MemoryEngineError::Validation(format!(
                    "memory unit task belongs to a retired sleep run: {}",
                    task.task_id
                )));
            }
            let units = self.adopt_legacy_memory_units(&archive, units)?;
            let operation = prepare_publication(task, &archive, request_digest, units, last_error)?;
            self.storage.begin_journaled_operation(&operation)?;
            self.finish_memory_unit_publication(&operation)
        })
    }

    fn adopt_legacy_memory_units(
        &self,
        archive: &ArchiveEntry,
        mut units: Vec<MemoryUnit>,
    ) -> Result<Vec<MemoryUnit>> {
        // Old storage could write one or more units before completing the
        // task. Match exact accepted content and preserve each existing ID.
        let mut legacy = archive
            .memory_units
            .iter()
            .cloned()
            .map(|unit| (unit.memory_unit_id.clone(), unit))
            .collect::<std::collections::BTreeMap<_, _>>();
        let canonical = self
            .storage
            .read_memory_units_for_archive_raw(&archive.archive_id)?;
        if let Some(warning) = canonical.warnings.iter().find(|warning| {
            let filename = warning.path.rsplit('/').next().unwrap_or(&warning.path);
            filename
                .strip_suffix(".json")
                .is_some_and(|id| legacy.contains_key(id))
        }) {
            return Err(MemoryEngineError::Storage(warning.note()));
        }
        for unit in canonical.items {
            legacy.insert(unit.memory_unit_id.clone(), unit);
        }
        let mut available = (0..units.len()).collect::<BTreeSet<_>>();
        for previous in legacy.into_values() {
            let Some(index) = available
                .iter()
                .copied()
                .find(|index| same_unit_basis(&units[*index], &previous))
            else {
                return Err(MemoryEngineError::OperationConflict);
            };
            available.remove(&index);
            units[index] = previous;
        }
        Ok(units)
    }

    pub(super) fn recover_memory_unit_publication(
        &self,
        operation: JournalOperation,
    ) -> Result<bool> {
        let intent = publication_intent(&operation)?;
        self.with_memory_write(&intent.scope, &intent.source_session_id, || {
            self.with_resource_lock(archive_lock_key(&intent.receipt.archive_id), || {
                let Some(current) = self.storage.load_journaled_operation(&operation.op_id)? else {
                    return Ok(false);
                };
                self.finish_memory_unit_publication(&current)?;
                Ok(true)
            })
        })
    }

    fn finish_memory_unit_publication(&self, operation: &JournalOperation) -> Result<ArchiveEntry> {
        let result = self.apply_memory_unit_publication(operation);
        if result.is_err() {
            self.memory_operations_recovered
                .store(false, Ordering::Release);
        }
        result
    }

    fn apply_memory_unit_publication(&self, operation: &JournalOperation) -> Result<ArchiveEntry> {
        let intent = publication_intent(operation)?;
        let receipt = intent.receipt;
        let mut task = self.load_task_for_response(&receipt.task_id)?;
        let mut archive = self.storage.read_archive_entry_raw(&receipt.archive_id)?;
        if task.task_type != TaskType::MemoryUnitPass
            || task.inputs["preliminary_archive_id"].as_str() != Some(receipt.archive_id.as_str())
            || task.state == TaskState::Cancelled
            || archive.status == ArchiveStatus::Superseded
            || archive.source_session_id != intent.source_session_id
            || archive.effective_personality_scope_id() != intent.scope
        {
            return Err(MemoryEngineError::OperationStale);
        }
        let expected = serde_json::to_value(&receipt)?;
        if let Some(stored) = &task.outcome {
            if task.state != TaskState::Completed || *stored != expected {
                return Err(MemoryEngineError::OperationConflict);
            }
            archive = self.read_published_memory_units(&receipt)?;
        } else {
            for unit in &intent.units {
                self.storage.write_memory_unit(unit)?;
            }
            archive.updated_at = operation.created_at.clone();
            archive.memory_units = intent.units;
            archive.compact_memory = render_compact_memory_from_units(&archive.memory_units);
            self.storage
                .update_archive_entry(&receipt.archive_id, &archive)?;
            task.state = TaskState::Completed;
            task.updated_at = operation.created_at.clone();
            task.last_error = intent.last_error;
            task.outcome = Some(expected.clone());
            self.storage.save_task(&task)?;
        }
        self.storage
            .write_memory_unit_publication_receipt(&archive.archive_id, &expected)?;
        self.storage.delete_journaled_operation(&operation.op_id)?;
        Ok(archive)
    }

    fn read_published_memory_units(&self, receipt: &PublicationReceipt) -> Result<ArchiveEntry> {
        let mut archive = self.storage.read_archive_entry_raw(&receipt.archive_id)?;
        let units = self
            .storage
            .read_memory_units_for_archive_raw(&receipt.archive_id)?;
        if units.items.len() != receipt.memory_unit_ids.len() {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        let mut by_id = units
            .items
            .into_iter()
            .map(|unit| (unit.memory_unit_id.clone(), unit))
            .collect::<HashMap<_, _>>();
        archive.memory_units = receipt
            .memory_unit_ids
            .iter()
            .map(|id| {
                by_id
                    .remove(id)
                    .ok_or(MemoryEngineError::OperationResultUnavailable)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(archive)
    }
}

impl PublicationReceipt {
    fn require_request(&self, task: &PendingTask, request_digest: &str) -> Result<()> {
        if self.schema_version != PUBLICATION_SCHEMA
            || self.task_id != task.task_id
            || self.request_digest != request_digest
        {
            return Err(MemoryEngineError::OperationConflict);
        }
        Ok(())
    }
}

fn prepare_publication(
    task: &PendingTask,
    archive: &ArchiveEntry,
    request_digest: String,
    units: Vec<MemoryUnit>,
    last_error: Option<String>,
) -> Result<JournalOperation> {
    let now = now_rfc3339()?;
    let intent = PublicationIntent {
        receipt: PublicationReceipt {
            schema_version: PUBLICATION_SCHEMA.into(),
            archive_id: archive.archive_id.clone(),
            task_id: task.task_id.clone(),
            request_digest,
            memory_unit_ids: units
                .iter()
                .map(|unit| unit.memory_unit_id.clone())
                .collect(),
        },
        source_session_id: archive.source_session_id.clone(),
        scope: archive.effective_personality_scope_id().into(),
        units,
        last_error,
    };
    Ok(JournalOperation {
        schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.into(),
        op_id: memory_unit_publication_key(&archive.archive_id),
        op_type: JournalOperationType::MemoryUnitPublish,
        state: JournalState::Started,
        created_at: now.clone(),
        updated_at: now,
        target_files: vec![archive.archive_id.clone()],
        intent: serde_json::to_value(intent)?,
        recovery_policy: RecoveryPolicy::Retry,
        completed_at: None,
        error: None,
    })
}

fn publication_intent(operation: &JournalOperation) -> Result<PublicationIntent> {
    if operation.op_type != JournalOperationType::MemoryUnitPublish
        || operation.state != JournalState::Started
    {
        return Err(MemoryEngineError::OperationIncomplete);
    }
    let intent: PublicationIntent = serde_json::from_value(operation.intent.clone())?;
    let receipt = &intent.receipt;
    if receipt.schema_version != PUBLICATION_SCHEMA
        || operation.op_id != memory_unit_publication_key(&receipt.archive_id)
        || !receipt
            .memory_unit_ids
            .iter()
            .eq(intent.units.iter().map(|unit| &unit.memory_unit_id))
        || receipt.memory_unit_ids.iter().collect::<HashSet<_>>().len() != intent.units.len()
        || intent.units.iter().any(|unit| {
            unit.archive_id != receipt.archive_id
                || unit.source_session_id != intent.source_session_id
                || unit.effective_personality_scope_id() != intent.scope
        })
    {
        return Err(MemoryEngineError::Storage(
            "invalid memory unit publication journal".into(),
        ));
    }
    Ok(intent)
}

fn same_unit_basis(left: &MemoryUnit, right: &MemoryUnit) -> bool {
    left.archive_id == right.archive_id
        && left.source_session_id == right.source_session_id
        && left.effective_personality_scope_id() == right.effective_personality_scope_id()
        && left.thesis == right.thesis
        && left.subject == right.subject
        && left.source_event_ids == right.source_event_ids
        && left.evidence == right.evidence
        && left.tags == right.tags
        && left.weight == right.weight
}

#[cfg(test)]
mod tests {
    use super::super::transactional_support::serializable_digest;
    use super::*;
    use crate::FileStorage;
    use std::fs;

    #[test]
    fn recovery_finishes_every_persisted_publication_cut() {
        // Journal; first unit; all units; Archive mirror; task; receipt.
        for cut in 0..=5 {
            let (engine, mut task, mut archive) = start_extraction("publication_cut");
            let storage = engine.storage().clone();
            let units = vec![
                fixture_unit(&archive, "mu_first", "The user bought a notebook."),
                fixture_unit(&archive, "mu_second", "The user visited the station."),
            ];
            let operation = prepare_publication(
                &task,
                &archive,
                serializable_digest(&units).unwrap(),
                units.clone(),
                None,
            )
            .unwrap();
            let receipt = publication_intent(&operation).unwrap().receipt;
            let receipt_json = serde_json::to_value(&receipt).unwrap();
            storage.begin_journaled_operation(&operation).unwrap();

            for unit in units.iter().take(cut.min(units.len())) {
                storage.write_memory_unit(unit).unwrap();
            }
            if cut >= 3 {
                archive.memory_units = units.clone();
                archive.updated_at = operation.created_at.clone();
                archive.compact_memory = render_compact_memory_from_units(&units);
                storage
                    .update_archive_entry(&archive.archive_id, &archive)
                    .unwrap();
            }
            if cut >= 4 {
                task.state = TaskState::Completed;
                task.outcome = Some(receipt_json.clone());
                storage.save_task(&task).unwrap();
            }
            if cut >= 5 {
                storage
                    .write_memory_unit_publication_receipt(&archive.archive_id, &receipt_json)
                    .unwrap();
            }
            assert!(
                matches!(
                    storage.read_archive_entry_by_id(&archive.archive_id),
                    Err(MemoryEngineError::OperationIncomplete)
                ),
                "cut {cut}"
            );
            drop(engine);

            let reopened = MemoryEngine::new(storage.clone());
            assert_eq!(
                reopened.recover_pending_memory_operations().unwrap(),
                1,
                "cut {cut}"
            );
            assert_eq!(
                reopened.recover_pending_memory_operations().unwrap(),
                0,
                "cut {cut}"
            );
            assert_eq!(
                storage
                    .read_archive_entry_by_id(&archive.archive_id)
                    .unwrap()
                    .memory_units,
                units,
                "cut {cut}"
            );
            let published = storage.load_task(&task.task_id).unwrap();
            assert_eq!(published.state, TaskState::Completed);
            assert_eq!(published.outcome, Some(receipt_json));
            fs::remove_dir_all(storage.root()).unwrap();
        }
    }

    #[test]
    fn purge_removes_incomplete_publication_in_its_scope_only() {
        let (engine, task, archive) = start_extraction("publication_purge");
        let storage = engine.storage().clone();
        let unit = fixture_unit(&archive, "mu_pending", "The user bought a notebook.");
        let operation = prepare_publication(
            &task,
            &archive,
            serializable_digest(&unit).unwrap(),
            vec![unit.clone()],
            None,
        )
        .unwrap();
        storage.begin_journaled_operation(&operation).unwrap();
        storage.write_memory_unit(&unit).unwrap();

        engine.ingest(test_event("other_session")).unwrap();
        assert_eq!(
            engine
                .purge_scope("other_session")
                .unwrap()
                .deleted
                .sessions,
            1
        );
        assert!(storage
            .load_journaled_operation(&operation.op_id)
            .unwrap()
            .is_some());
        assert!(matches!(
            engine.delete_session(&archive.source_session_id),
            Err(MemoryEngineError::OperationIncomplete)
        ));
        // An already initialized caller can erase the incomplete scope without
        // relying on the visible read projection for its deletion inventory.
        engine
            .memory_operations_recovered
            .store(true, Ordering::Release);
        assert_eq!(
            engine
                .purge_scope(archive.effective_personality_scope_id())
                .unwrap()
                .deleted
                .memory_units,
            1
        );
        let restarted = MemoryEngine::new(storage.clone());
        assert_eq!(restarted.recover_pending_memory_operations().unwrap(), 0);
        assert!(storage
            .read_memory_unit_by_id(&unit.memory_unit_id)
            .is_err());
        assert!(storage
            .read_archive_entry_by_id(&archive.archive_id)
            .is_err());
        fs::remove_dir_all(storage.root()).unwrap();
    }

    fn start_extraction(case: &str) -> (MemoryEngine<FileStorage>, PendingTask, ArchiveEntry) {
        let root = std::env::temp_dir().join(new_id(case).unwrap());
        let engine = MemoryEngine::new(FileStorage::new(root));
        engine.ingest(test_event("publication_session")).unwrap();
        let sleep = engine.sleep("publication_session").unwrap();
        (engine, sleep.memory_unit_task.unwrap(), sleep.archive_entry)
    }

    fn test_event(session: &str) -> IngestEvent {
        serde_json::from_value(json!({
            "schema_version": EVENT_SCHEMA_VERSION,
            "type": "user_message",
            "source": "fixture",
            "timestamp": "2026-11-16T12:00:00Z",
            "session_id": session,
            "payload": {"text": "I bought a notebook at the station."}
        }))
        .unwrap()
    }

    fn fixture_unit(archive: &ArchiveEntry, id: &str, thesis: &str) -> MemoryUnit {
        MemoryUnit {
            schema_version: MEMORY_UNIT_SCHEMA_VERSION.into(),
            memory_unit_id: id.into(),
            archive_id: archive.archive_id.clone(),
            personality_scope_id: archive.personality_scope_id.clone(),
            source_session_id: archive.source_session_id.clone(),
            created_at: archive.created_at.clone(),
            updated_at: archive.updated_at.clone(),
            thesis: thesis.into(),
            subject: None,
            source_event_ids: archive.source_event_ids.clone(),
            evidence: None,
            tags: Vec::new(),
            weight: 0.5,
            status: MemoryUnitStatus::ActiveArchive,
            fidelity_status: FidelityStatus::Unchecked,
            fidelity_review: None,
            forget_review: None,
            reflection_count: 0,
            last_reflected_at: None,
            last_reflection_operation_id: None,
        }
    }
}
