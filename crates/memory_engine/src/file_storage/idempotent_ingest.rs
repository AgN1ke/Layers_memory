use std::io::{Read, Seek, SeekFrom};

use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use super::*;
use crate::engine::storage_digest;
use crate::erasure::{erasure_key, MemoryErasureKind, WriteGeneration};
use crate::journal::{JournalOperationType, RecoveryPolicy};
use crate::storage_v2::{WorkUnitCommit, WorkUnitIntent};
use crate::types::JOURNAL_OPERATION_SCHEMA_VERSION;

const INGEST_INTENT_SCHEMA: &str = "file_ingest_intent.v1";
const INGEST_REPLAY_WINDOW: Duration = Duration::days(30);

#[derive(Serialize, Deserialize)]
pub(crate) struct JournaledIngest {
    pub(crate) request_digest: String,
    pub(crate) host_id: String,
    pub(crate) event: StoredEvent,
    pub(crate) commit: WorkUnitCommit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) generation: Option<WriteGeneration>,
}

#[derive(Serialize, Deserialize)]
struct PreparedIngest {
    schema_version: String,
    #[serde(flatten)]
    input: JournaledIngest,
    metadata: SessionMetadata,
    events_offset: u64,
    markdown_offset: u64,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct IngestReceipt {
    pub(crate) request_digest: String,
    pub(crate) commit: WorkUnitCommit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<IngestReceiptOrigin>,
}

#[derive(Serialize, Deserialize)]
struct IngestReceiptOrigin {
    session_key: String,
    scope_key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "schema_version")]
enum IngestReceiptRecord {
    #[serde(rename = "file_ingest_receipt.v1")]
    Committed(IngestReceipt),
    #[serde(rename = "file_ingest_retired.v1")]
    Retired,
}

impl IngestReceipt {
    pub(crate) fn require_replayable(&self) -> Result<()> {
        if self.is_expired_at(OffsetDateTime::now_utc())? {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        Ok(())
    }

    fn is_expired_at(&self, now: OffsetDateTime) -> Result<bool> {
        let accepted_at = OffsetDateTime::parse(&self.commit.committed_at, &Rfc3339)
            .map_err(|error| MemoryEngineError::Storage(error.to_string()))?;
        Ok(now - accepted_at >= INGEST_REPLAY_WINDOW)
    }
}

impl FileStorage {
    pub(crate) fn host_id(&self) -> &str {
        &self.host_id
    }

    pub(crate) fn validate_idempotent_session_id(&self, session: &str) -> Result<()> {
        validate_erasure_path_component("session ID", session)?;
        if session.ends_with([' ', '.']) || session.contains(['<', '>', ':', '"', '|', '?', '*']) {
            return Err(MemoryEngineError::Validation(
                "invalid session path component".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_ingest_session(
        &self,
        session: &str,
        scope: &str,
        host: &str,
    ) -> Result<()> {
        let path = self.session_json_path(session);
        if path.try_exists()? {
            let metadata = self.read_session_metadata_file(session)?;
            metadata.validate()?;
            if metadata.session_id != session
                || metadata.host_id != host
                || metadata.effective_personality_scope_id() != scope
            {
                return Err(MemoryEngineError::Validation(
                    "session ID is bound to another event origin".into(),
                ));
            }
        }
        Ok(())
    }

    fn pending_ingest_path(&self, session: &str) -> PathBuf {
        self.session_dir(session).join("pending-ingest.json")
    }

    fn ingest_receipt_dir(&self) -> PathBuf {
        self.root.join("journal").join("ingest-receipts")
    }

    fn ingest_receipt_path(&self, key: &str) -> PathBuf {
        self.ingest_receipt_dir().join(format!("{key}.json"))
    }

    pub(super) fn require_complete_ingest(&self, session: &str) -> Result<()> {
        if self.pending_ingest_path(session).try_exists()? {
            return Err(MemoryEngineError::OperationIncomplete);
        }
        Ok(())
    }

    pub(crate) fn ingest_receipt(&self, key: &str) -> Result<Option<IngestReceipt>> {
        validate_erasure_path_component("ingest journal key", key)?;
        match read_json::<IngestReceiptRecord>(&self.ingest_receipt_path(key)) {
            Ok(IngestReceiptRecord::Committed(receipt)) => {
                receipt.commit.validate()?;
                Ok(Some(receipt))
            }
            Ok(IngestReceiptRecord::Retired) => Err(MemoryEngineError::OperationResultUnavailable),
            Err(MemoryEngineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn retire_expired_ingest_receipts(
        &self,
        now: OffsetDateTime,
    ) -> Result<(usize, usize)> {
        let mut paths = Vec::new();
        collect_json_files_shallow(&self.ingest_receipt_dir(), &mut paths)?;
        let mut retired = 0;
        let mut skipped = 0;
        for path in paths {
            let receipt = match read_json(&path) {
                Ok(IngestReceiptRecord::Committed(receipt)) => receipt,
                Ok(IngestReceiptRecord::Retired) => continue,
                Err(MemoryEngineError::Json(_)) => {
                    skipped += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if !receipt.is_expired_at(now)? {
                continue;
            }
            // Recovery can still need the receipt to verify that the append
            // finished before the intent and session marker were removed.
            let pending = self
                .root
                .join("journal")
                .join(path.file_name().ok_or_else(|| {
                    MemoryEngineError::Storage("ingest receipt path has no filename".to_string())
                })?)
                .try_exists()?;
            if !pending {
                atomic_write_json(&path, &IngestReceiptRecord::Retired)?;
                retired += 1;
            }
        }
        Ok((retired, skipped))
    }

    pub(super) fn retire_erased_ingest_receipts(&self, plan: &MemoryErasurePlan) -> Result<()> {
        let target = erasure_key(plan.operation, &plan.target_id);
        let mut paths = Vec::new();
        collect_json_files_shallow(&self.ingest_receipt_dir(), &mut paths)?;
        let mut legacy = Vec::new();
        for path in paths {
            let receipt = match read_json(&path) {
                Ok(IngestReceiptRecord::Committed(receipt)) => receipt,
                // A damaged receipt has no verified owner or replay result.
                // It must not prevent erasure of another scope.
                Ok(IngestReceiptRecord::Retired) | Err(MemoryEngineError::Json(_)) => continue,
                Err(error) => return Err(error),
            };
            let Some(origin) = receipt.origin else {
                legacy.push((path, receipt.commit));
                continue;
            };
            let key = match plan.operation {
                MemoryErasureKind::DeleteSession => origin.session_key,
                MemoryErasureKind::PurgeScope => origin.scope_key,
            };
            if key == target {
                atomic_write_json(&path, &IngestReceiptRecord::Retired)?;
            }
        }
        if !legacy.is_empty() {
            self.retire_legacy_ingest_receipts(plan, &target, &legacy)?;
        }
        Ok(())
    }

    fn retire_legacy_ingest_receipts(
        &self,
        plan: &MemoryErasurePlan,
        target: &str,
        receipts: &[(PathBuf, WorkUnitCommit)],
    ) -> Result<()> {
        // Existing receipts have no owner field. A match needs both the event
        // ID and its exact digest while the target's source is still present.
        let by_event: HashMap<_, _> = receipts
            .iter()
            .map(|(path, commit)| {
                (
                    commit.result_reference.as_str(),
                    (path, commit.result_digest.as_str()),
                )
            })
            .collect();
        let sessions = match plan.operation {
            MemoryErasureKind::DeleteSession => std::slice::from_ref(&plan.target_id),
            MemoryErasureKind::PurgeScope => &plan.session_ids,
        };
        let mut retired = HashSet::new();
        for session in sessions {
            let mut files = vec![self.events_path(session)];
            collect_jsonl_files_shallow(&self.archived_events_dir(session), &mut files)?;
            for path in files {
                for event in read_jsonl::<StoredEvent>(&path)? {
                    let Some(&(receipt_path, digest)) = by_event.get(event.event_id.as_str())
                    else {
                        continue;
                    };
                    let owner = match plan.operation {
                        MemoryErasureKind::DeleteSession => &event.session_id,
                        MemoryErasureKind::PurgeScope => event.effective_personality_scope_id(),
                    };
                    if erasure_key(plan.operation, owner) == target
                        && storage_digest(&event)? == digest
                        && retired.insert(receipt_path)
                    {
                        atomic_write_json(receipt_path, &IngestReceiptRecord::Retired)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn start_journaled_ingest(
        &self,
        key: &str,
        work: WorkUnitIntent,
        event: &StoredEvent,
        generation: WriteGeneration,
    ) -> Result<()> {
        self.require_complete_ingest(&event.session_id)?;
        // A failure may have occurred after the intent was written but before
        // the session marker existed. Keep the original event and offsets.
        if self.load_journaled_operation(key)?.is_some() {
            return Err(MemoryEngineError::OperationIncomplete);
        }
        let operation = self.prepare_journaled_ingest(key, work, event, Some(generation))?;
        self.begin_journaled_operation(&operation)?;
        self.finish_journaled_ingest(&operation)
    }

    fn prepare_journaled_ingest(
        &self,
        key: &str,
        work: WorkUnitIntent,
        event: &StoredEvent,
        generation: Option<WriteGeneration>,
    ) -> Result<JournalOperation> {
        let session = &event.session_id;
        let prepared = PreparedIngest {
            schema_version: INGEST_INTENT_SCHEMA.into(),
            metadata: self.session_metadata_after_event(session, event)?,
            events_offset: append_offset(&self.events_path(session))?,
            markdown_offset: append_offset(&self.session_md_path(session))?,
            input: JournaledIngest {
                request_digest: work.request_digest,
                host_id: self.host_id.clone(),
                event: event.clone(),
                generation,
                commit: WorkUnitCommit {
                    committed_at: event.received_at.clone(),
                    result_reference: event.event_id.clone(),
                    result_digest: storage_digest(event)?,
                },
            },
        };
        Ok(JournalOperation {
            schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.into(),
            op_id: key.to_owned(),
            op_type: JournalOperationType::IngestEvent,
            state: JournalState::Started,
            created_at: work.started_at.clone(),
            updated_at: work.started_at,
            target_files: Vec::new(),
            intent: serde_json::to_value(prepared)?,
            recovery_policy: RecoveryPolicy::Retry,
            completed_at: None,
            error: None,
        })
    }

    pub(super) fn finish_journaled_ingest(&self, operation: &JournalOperation) -> Result<()> {
        if operation.op_type != JournalOperationType::IngestEvent
            || operation.state != JournalState::Started
        {
            return Err(MemoryEngineError::Storage(
                "unexpected ingest journal operation".into(),
            ));
        }
        let prepared: PreparedIngest = serde_json::from_value(operation.intent.clone())?;
        if prepared.schema_version != INGEST_INTENT_SCHEMA {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: INGEST_INTENT_SCHEMA.into(),
                actual: prepared.schema_version,
            });
        }
        let JournaledIngest {
            request_digest,
            host_id,
            event,
            commit,
            ..
        } = prepared.input;
        let session = &event.session_id;
        self.validate_idempotent_session_id(session)?;
        event.validate()?;
        prepared.metadata.validate()?;
        commit.validate()?;
        if prepared.metadata.session_id != *session
            || prepared.metadata.host_id != host_id
            || prepared.metadata.effective_personality_scope_id()
                != event.effective_personality_scope_id()
            || event.event_id != commit.result_reference
            || storage_digest(&event)? != commit.result_digest
        {
            return Err(MemoryEngineError::Storage(
                "ingest journal does not match its result".into(),
            ));
        }
        let key = &operation.op_id;
        let marker_path = self.pending_ingest_path(session);
        let current_marker = if marker_path.try_exists()? {
            Some(read_json::<String>(&marker_path)?)
        } else {
            None
        };
        if let Some(receipt) = self.ingest_receipt(key)? {
            if receipt.request_digest != request_digest || receipt.commit != commit {
                return Err(MemoryEngineError::Storage(
                    "ingest receipt does not match its journal".into(),
                ));
            }
        } else {
            self.validate_ingest_session(
                session,
                event.effective_personality_scope_id(),
                &host_id,
            )?;
            match current_marker.as_deref() {
                Some(owner) if owner != key => return Err(MemoryEngineError::OperationIncomplete),
                Some(_) => {}
                None => atomic_write_json(&marker_path, key)?,
            }
            let line = format!("{}\n", serde_json::to_string(&event)?);
            finish_append(
                &self.events_path(session),
                prepared.events_offset,
                line.as_bytes(),
            )?;
            atomic_write_json(&self.session_json_path(session), &prepared.metadata)?;
            let markdown = event_markdown(session, &event, prepared.markdown_offset == 0);
            finish_append(
                &self.session_md_path(session),
                prepared.markdown_offset,
                markdown.as_bytes(),
            )?;
            let receipt = IngestReceipt {
                request_digest,
                commit,
                origin: Some(IngestReceiptOrigin {
                    session_key: erasure_key(MemoryErasureKind::DeleteSession, session),
                    scope_key: erasure_key(
                        MemoryErasureKind::PurgeScope,
                        event.effective_personality_scope_id(),
                    ),
                }),
            };
            atomic_write_json(
                &self.ingest_receipt_path(key),
                &IngestReceiptRecord::Committed(receipt),
            )?;
        }
        if current_marker.as_deref().is_none_or(|owner| owner == key) {
            remove_file_if_exists(&marker_path)?;
        }
        self.delete_journaled_operation(key)
    }

    pub(super) fn discard_ingest_intent(&self, key: &str, session: &str) -> Result<()> {
        self.validate_idempotent_session_id(session)?;
        let marker = self.pending_ingest_path(session);
        match read_json::<String>(&marker) {
            Ok(owner) if owner == key => remove_file_if_exists(&marker)?,
            Ok(_) => {}
            Err(MemoryEngineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.delete_journaled_operation(key)
    }
}

fn append_offset(path: &Path) -> Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(_) => Err(MemoryEngineError::Storage(
            "ingest target is not a file".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn finish_append(path: &Path, offset: u64, suffix: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)?;
    let written = file
        .metadata()?
        .len()
        .checked_sub(offset)
        .filter(|len| *len <= suffix.len() as u64)
        .ok_or_else(|| {
            MemoryEngineError::Storage("ingest target changed while an append was pending".into())
        })? as usize;
    if written > 0 {
        file.seek(SeekFrom::Start(offset))?;
        let mut existing = vec![0; written];
        file.read_exact(&mut existing)?;
        if existing != suffix[..written] {
            return Err(MemoryEngineError::Storage(
                "ingest target contains unexpected appended bytes".into(),
            ));
        }
    }
    file.write_all(&suffix[written..])?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::IngestEvent;
    use crate::storage_v2::WORK_UNIT_INTENT_SCHEMA_VERSION;
    use crate::MemoryEngine;

    #[test]
    fn restart_finishes_torn_appends_without_recounting_the_event() {
        let root = std::env::temp_dir().join(format!(
            "memory_engine_torn_ingest_{}_{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let storage = FileStorage::with_host_id(&root, "test");
        let request: IngestEvent = serde_json::from_value(serde_json::json!({
            "schema_version": "event.v1",
            "type": "user_message",
            "source": "test",
            "timestamp": "2026-11-16T12:00:00Z",
            "session_id": "test_session",
            "payload": {"text": "Доповни цей запис."}
        }))
        .unwrap();
        let engine = MemoryEngine::new(storage.clone());
        engine.ingest(request.clone()).unwrap();
        drop(engine);

        let accepted_at = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
        let event = StoredEvent::from_ingest(request, "event_pending", &accepted_at, 0.5, "test");
        let key = "ingest_torn_fixture";
        let work = WorkUnitIntent {
            schema_version: WORK_UNIT_INTENT_SCHEMA_VERSION.into(),
            operation_id: "torn_fixture".into(),
            operation_kind: "ingest_event".into(),
            request_digest: storage_digest(&event).unwrap(),
            started_at: accepted_at,
        };
        let operation = storage
            .prepare_journaled_ingest(key, work, &event, None)
            .unwrap();
        let prepared: PreparedIngest = serde_json::from_value(operation.intent.clone()).unwrap();
        let session = &event.session_id;
        storage.begin_journaled_operation(&operation).unwrap();
        atomic_write_json(&storage.pending_ingest_path(session), &key).unwrap();
        let raw = serde_json::to_string(&event).unwrap();
        let markdown = event_markdown(session, &event, false);
        for (path, bytes) in [
            (storage.events_path(session), raw.as_bytes()),
            (storage.session_md_path(session), markdown.as_bytes()),
        ] {
            OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(&bytes[..12])
                .unwrap();
        }
        // Metadata may already reflect the new event before Markdown is complete.
        atomic_write_json(&storage.session_json_path(session), &prepared.metadata).unwrap();
        assert!(matches!(
            storage.read_session(session),
            Err(MemoryEngineError::OperationIncomplete)
        ));

        let restarted = MemoryEngine::new(storage.clone());
        assert_eq!(restarted.recover_pending_memory_operations().unwrap(), 1);
        assert_eq!(restarted.recover_pending_memory_operations().unwrap(), 0);
        let record = storage.read_session(session).unwrap();
        assert_eq!(record.metadata.event_count, 2);
        assert_eq!(record.events.len(), 2);
        assert_eq!(record.events[1], event);
        assert_eq!(
            fs::read_to_string(storage.session_md_path(session))
                .unwrap()
                .matches("Доповни цей запис.")
                .count(),
            2
        );
        assert!(storage.ingest_receipt(key).unwrap().is_some());

        let stale_key = "ingest_old_generation_fixture";
        let generation = restarted.write_generation(session, None).unwrap();
        let mut obsolete = operation;
        obsolete.op_id = stale_key.into();
        obsolete.intent["generation"] = serde_json::to_value(generation).unwrap();
        restarted.delete_session(session).unwrap();
        // An obsolete intent in the recovery scan must never append again.
        storage.begin_journaled_operation(&obsolete).unwrap();
        drop(restarted);
        let restarted = MemoryEngine::new(storage.clone());
        assert_eq!(restarted.recover_pending_memory_operations().unwrap(), 0);
        assert!(storage
            .load_journaled_operation(stale_key)
            .unwrap()
            .is_none());
        assert!(storage.read_session(session).unwrap().events.is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_receipt_past_the_replay_window_cannot_return_a_result() {
        let accepted_at = OffsetDateTime::now_utc() - INGEST_REPLAY_WINDOW;
        let receipt = IngestReceipt {
            request_digest: "unused".into(),
            commit: WorkUnitCommit {
                committed_at: accepted_at.format(&Rfc3339).unwrap(),
                result_reference: "event_old".into(),
                result_digest: "unused".into(),
            },
            origin: None,
        };
        assert!(matches!(
            receipt.require_replayable(),
            Err(MemoryEngineError::OperationResultUnavailable)
        ));
    }

    #[test]
    fn expired_receipt_reserves_the_operation_after_retirement_and_restart() {
        let storage = fixture_storage("ingest_receipt_retention");
        let engine = MemoryEngine::new(storage.clone());
        let request = fixture_event("expiry_session", "expiry_scope");
        engine.ingest_idempotent("expiry", request.clone()).unwrap();
        let key = fixture_operation_key("expiry");
        let now = OffsetDateTime::now_utc() + INGEST_REPLAY_WINDOW + Duration::seconds(1);

        // Reproduce the window after the receipt has been written but before
        // the operation intent has been removed.
        atomic_write_json(&storage.journal_path(&key), &true).unwrap();
        assert_eq!(storage.retire_expired_ingest_receipts(now).unwrap(), (0, 0));
        assert!(storage.ingest_receipt(&key).unwrap().is_some());
        remove_file_if_exists(&storage.journal_path(&key)).unwrap();

        assert_eq!(storage.retire_expired_ingest_receipts(now).unwrap(), (1, 0));
        assert_eq!(storage.retire_expired_ingest_receipts(now).unwrap(), (0, 0));
        assert_receipt_retired(&storage, &key);
        drop(engine);
        let reopened = MemoryEngine::new(storage.clone());
        assert!(matches!(
            reopened.ingest_idempotent("expiry", request),
            Err(MemoryEngineError::OperationResultUnavailable)
        ));
        assert_eq!(
            reopened
                .read_session("expiry_session")
                .unwrap()
                .events
                .len(),
            1
        );
        fs::remove_dir_all(storage.root()).unwrap();
    }

    #[test]
    fn erasure_retires_exact_scope_receipts_including_legacy_rotated_events() {
        let storage = fixture_storage("ingest_receipt_erasure");
        let engine = MemoryEngine::new(storage.clone());
        let target = fixture_event("erase_session", "erase_scope");
        let sibling = fixture_event("sibling_session", "erase_scope");
        let unrelated = fixture_event("other_session", "other_scope");
        let old_event = engine
            .ingest_idempotent("legacy", target.clone())
            .unwrap()
            .event;
        engine.ingest_idempotent("modern", target.clone()).unwrap();
        engine
            .ingest_idempotent("sibling", sibling.clone())
            .unwrap();
        engine
            .ingest_idempotent("other", unrelated.clone())
            .unwrap();

        let legacy_key = fixture_operation_key("legacy");
        let path = storage.ingest_receipt_path(&legacy_key);
        let mut legacy: serde_json::Value = read_json(&path).unwrap();
        legacy.as_object_mut().unwrap().remove("origin");
        atomic_write_json(&path, &legacy).unwrap();
        storage
            .rotate_session_events(&target.session_id, &[old_event.event_id])
            .unwrap();

        engine.delete_session(&target.session_id).unwrap();
        assert_receipt_retired(&storage, &legacy_key);
        assert_receipt_retired(&storage, &fixture_operation_key("modern"));
        assert!(
            engine
                .ingest_idempotent("sibling", sibling.clone())
                .unwrap()
                .already_applied
        );
        engine.purge_scope("erase_scope").unwrap();
        assert_receipt_retired(&storage, &fixture_operation_key("sibling"));
        assert!(
            engine
                .ingest_idempotent("other", unrelated)
                .unwrap()
                .already_applied
        );
        assert!(matches!(
            engine.ingest_idempotent("legacy", target),
            Err(MemoryEngineError::OperationResultUnavailable)
        ));
        fs::remove_dir_all(storage.root()).unwrap();
    }

    fn fixture_storage(case: &str) -> FileStorage {
        let root = std::env::temp_dir().join(format!(
            "memory_engine_{}_{}_{}",
            case,
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        FileStorage::with_host_id(&root, "test")
    }

    fn fixture_event(session_id: &str, scope: &str) -> IngestEvent {
        serde_json::from_value(serde_json::json!({
            "schema_version": "event.v1",
            "type": "user_message",
            "source": "test",
            "timestamp": "2026-11-16T12:00:00Z",
            "session_id": session_id,
            "personality_scope_id": scope,
            "payload": {"text": "A private message."}
        }))
        .unwrap()
    }

    fn fixture_operation_key(operation_id: &str) -> String {
        let hash = storage_digest(&("ingest_event", "test", operation_id)).unwrap();
        format!("ingest_{}", hash.strip_prefix("sha256:").unwrap())
    }

    fn assert_receipt_retired(storage: &FileStorage, key: &str) {
        let record: serde_json::Value = read_json(&storage.ingest_receipt_path(key)).unwrap();
        assert_eq!(
            record,
            serde_json::json!({"schema_version": "file_ingest_retired.v1"})
        );
    }
}
