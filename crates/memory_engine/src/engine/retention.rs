use super::erasure_flow::{json_references_any, sleep_run_task_ids};
use super::*;
use crate::file_storage::FileStorage;
use std::collections::BTreeMap;
use time::Duration;

const REPLAY_WINDOW: Duration = Duration::days(30);
const COMPACTED_TASK_SCHEMA: &str = "compacted_task_inputs.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOperationPruneReport {
    pub schema_version: String,
    pub tasks_compacted: usize,
    pub sleep_runs_compacted: usize,
    pub journals_removed: usize,
    pub ingest_receipts_retired: usize,
    pub skipped_records: usize,
}

#[derive(Default, Serialize)]
struct TaskReferences {
    sessions: BTreeSet<String>,
    scopes: BTreeSet<String>,
    archives: BTreeSet<String>,
    memory_units: BTreeSet<String>,
    core_facts: BTreeSet<String>,
    candidates: BTreeSet<String>,
    events: BTreeSet<String>,
}

pub(super) fn task_payload_compacted(task: &PendingTask) -> bool {
    task.inputs["schema_version"].as_str() == Some(COMPACTED_TASK_SCHEMA)
}

impl MemoryEngine<FileStorage> {
    /// Compact expired FileStorage records. This performs local I/O only.
    pub fn prune_completed_memory_operations(&self) -> Result<MemoryOperationPruneReport> {
        self.prune_memory_operations_at(OffsetDateTime::now_utc())
    }

    fn prune_memory_operations_at(
        &self,
        now: OffsetDateTime,
    ) -> Result<MemoryOperationPruneReport> {
        self.ensure_manifest_initialized()?;
        self.with_resource_lock("memory_operation_retention".into(), || {
            let mut report = MemoryOperationPruneReport {
                schema_version: "memory_operation_prune_report.v1".into(),
                tasks_compacted: 0,
                sleep_runs_compacted: 0,
                journals_removed: 0,
                ingest_receipts_retired: 0,
                skipped_records: 0,
            };
            let journals = self.storage.load_journaled_operations()?;
            let tasks = self.storage.load_all_tasks()?;
            let runs = self.storage.load_sleep_runs()?;
            let archives = self.storage.read_archive_inventory()?;
            let sessions = self.storage.read_sessions()?;
            let warnings = journals.warnings.len()
                + tasks.warnings.len()
                + runs.warnings.len()
                + archives.warnings.len()
                + sessions.warnings.len();
            if warnings != 0 {
                // Unreadable history could be the only record of a live run
                // or a reference needed for recovery, so leave it intact.
                report.skipped_records = warnings;
                return Ok(report);
            }

            let open_runs: Vec<_> = runs
                .items
                .iter()
                .filter(|run| run.stage != SleepRunStage::Finished)
                .collect();
            let live_task_ids: HashSet<_> = open_runs
                .iter()
                .flat_map(|run| sleep_run_task_ids(run))
                .collect();
            let live_archive_ids: HashSet<_> = open_runs
                .iter()
                .map(|run| run.archive_id.as_str())
                .collect();
            let archives = archives
                .items
                .iter()
                .map(|archive| (archive.archive_id.as_str(), archive))
                .collect::<HashMap<_, _>>();
            let scopes = sessions
                .items
                .iter()
                .map(|session| {
                    (
                        session.session_id.as_str(),
                        session.effective_personality_scope_id(),
                    )
                })
                .collect::<HashMap<_, _>>();
            let tasks_by_id = tasks
                .items
                .iter()
                .map(|task| (task.task_id.as_str(), task))
                .collect::<HashMap<_, _>>();
            let recovery_journals: Vec<_> = journals
                .items
                .iter()
                .filter(|operation| {
                    operation.state != JournalState::Completed
                        || (operation.op_type == JournalOperationType::ReflectionApply
                            && !journal_expired(operation, now))
                })
                .collect();

            for task in &tasks.items {
                if !matches!(task.state, TaskState::Completed | TaskState::Cancelled)
                    || !expired(&task.updated_at, now)
                    || task_payload_compacted(task)
                {
                    continue;
                }
                if task.schema_version != PENDING_TASK_SCHEMA_VERSION
                    || !matches!(
                        task.task_type,
                        TaskType::SleepCompression
                            | TaskType::CompactMemoryPass
                            | TaskType::MemoryUnitPass
                            | TaskType::MemoryFidelityPass
                            | TaskType::ForgetReview
                            | TaskType::ReflectionAnalyze
                            | TaskType::ComputeEmbedding
                    )
                {
                    report.skipped_records += 1;
                    continue;
                }
                let mut refs = task_references(task);
                let ids = BTreeSet::from([task.task_id.clone()]);
                if live_task_ids.contains(&task.task_id)
                    || refs
                        .archives
                        .iter()
                        .any(|id| live_archive_ids.contains(id.as_str()))
                    || recovery_journals
                        .iter()
                        .any(|journal| json_references_any(&journal.intent, &ids))
                    || !refs.resolve_owners(&archives, &scopes)
                {
                    report.skipped_records += 1;
                    continue;
                }
                if self.compact_task(task, refs)? {
                    report.tasks_compacted += 1;
                } else {
                    report.skipped_records += 1;
                }
            }

            for run in &runs.items {
                if run.stage != SleepRunStage::Finished || !sleep_payload_present(run) {
                    continue;
                }
                if run.schema_version != SLEEP_RUN_SCHEMA_VERSION {
                    report.skipped_records += 1;
                    continue;
                }
                let Some(task) = tasks_by_id.get(run.sleep_task_id.as_str()) else {
                    report.skipped_records += 1;
                    continue;
                };
                if !expired(&task.updated_at, now) {
                    continue;
                }
                let ids = sleep_run_task_ids(run)
                    .into_iter()
                    .chain([run.archive_id.clone()])
                    .collect::<BTreeSet<_>>();
                if recovery_journals
                    .iter()
                    .any(|journal| json_references_any(&journal.intent, &ids))
                {
                    report.skipped_records += 1;
                    continue;
                }
                let Some(archive) = archives.get(run.archive_id.as_str()) else {
                    report.skipped_records += 1;
                    continue;
                };
                report.sleep_runs_compacted +=
                    usize::from(self.compact_finished_run(run, archive)?);
            }

            for operation in &journals.items {
                if journal_expired(operation, now) && self.remove_expired_journal(operation)? {
                    report.journals_removed += 1;
                }
            }
            let (retired, skipped) = self.storage.retire_expired_ingest_receipts(now)?;
            report.ingest_receipts_retired = retired;
            report.skipped_records += skipped;
            Ok(report)
        })
    }

    fn compact_task(&self, snapshot: &PendingTask, refs: TaskReferences) -> Result<bool> {
        let scopes = refs.scopes.iter().cloned().collect::<Vec<_>>();
        let sessions = refs.sessions.iter().map(String::as_str).collect::<Vec<_>>();
        self.with_memory_write_targets(&scopes, &sessions, || {
            let mut current = match self.storage.load_task(&snapshot.task_id) {
                Ok(task) if &task == snapshot => task,
                Ok(_) | Err(MemoryEngineError::TaskNotFound(_)) => return Ok(false),
                Err(error) => return Err(error),
            };
            if current.task_type == TaskType::MemoryUnitPass {
                for archive in &refs.archives {
                    self.storage
                        .retire_archive_replay_receipt(archive, Some(&current.task_id))?;
                }
            }
            current.inputs = json!({
                "schema_version": COMPACTED_TASK_SCHEMA,
                "references": refs,
            });
            current.attempts.clear();
            current.last_error = None;
            current.outcome = None;
            self.storage.save_task(&current)?;
            Ok(true)
        })
    }

    fn compact_finished_run(&self, snapshot: &SleepRun, archive: &ArchiveEntry) -> Result<bool> {
        self.with_memory_write(
            archive.effective_personality_scope_id(),
            &snapshot.session_id,
            || {
                if self.storage.was_erased_task(&snapshot.sleep_task_id)? {
                    return Ok(false);
                }
                let mut run = self.storage.load_sleep_run(&snapshot.sleep_task_id)?;
                if &run != snapshot {
                    return Ok(false);
                }
                if let Some(context) = run.shared_prompt_context.as_mut() {
                    context.content.clear();
                }
                for state in &mut run.requests {
                    state.request.prompt_inputs = Value::Null;
                    state.last_error = None;
                    state.attempt_errors.clear();
                }
                run.memory_unit_result = None;
                run.emotional_pass = None;
                run.topic_thread_pass = None;
                run.personal_signal_pass = None;
                run.relational_pass = None;
                run.consolidator_gist = None;
                run.consolidator_narrative = None;
                self.storage.save_sleep_run(&run)?;
                Ok(true)
            },
        )
    }

    fn remove_expired_journal(&self, operation: &JournalOperation) -> Result<bool> {
        match operation.op_type {
            JournalOperationType::ReflectionApply => {
                let Some(task_id) = operation.intent["task_id"].as_str() else {
                    return Ok(false);
                };
                match self.storage.load_task(task_id) {
                    Ok(task) if task_payload_compacted(&task) => {}
                    Err(MemoryEngineError::TaskNotFound(_)) => {}
                    Ok(_) => return Ok(false),
                    Err(error) => return Err(error),
                }
            }
            JournalOperationType::CandidateReview | JournalOperationType::Migration => {}
            _ => return Ok(false),
        }
        // Completed journal files are immutable after the effect is applied.
        self.storage.delete_journaled_operation(&operation.op_id)?;
        Ok(true)
    }
}

fn expired(timestamp: &str, now: OffsetDateTime) -> bool {
    OffsetDateTime::parse(timestamp, &Rfc3339).is_ok_and(|at| now - at >= REPLAY_WINDOW)
}

fn journal_expired(operation: &JournalOperation, now: OffsetDateTime) -> bool {
    operation.schema_version == JOURNAL_OPERATION_SCHEMA_VERSION
        && operation.state == JournalState::Completed
        && operation
            .completed_at
            .as_deref()
            .is_some_and(|at| expired(at, now))
}

fn sleep_payload_present(run: &SleepRun) -> bool {
    run.shared_prompt_context
        .as_ref()
        .is_some_and(|context| !context.content.is_empty())
        || run.requests.iter().any(|state| {
            !state.request.prompt_inputs.is_null()
                || state.last_error.is_some()
                || !state.attempt_errors.is_empty()
        })
        || run.memory_unit_result.is_some()
        || run.emotional_pass.is_some()
        || run.topic_thread_pass.is_some()
        || run.personal_signal_pass.is_some()
        || run.relational_pass.is_some()
        || run.consolidator_gist.is_some()
        || run.consolidator_narrative.is_some()
}

impl TaskReferences {
    fn resolve_owners(
        &mut self,
        archives: &HashMap<&str, &ArchiveEntry>,
        scopes: &HashMap<&str, &str>,
    ) -> bool {
        for id in &self.archives {
            let Some(archive) = archives.get(id.as_str()) else {
                return false;
            };
            self.sessions.insert(archive.source_session_id.clone());
            self.scopes
                .insert(archive.effective_personality_scope_id().to_owned());
        }
        for session in &self.sessions {
            self.scopes.insert(
                scopes
                    .get(session.as_str())
                    .copied()
                    .unwrap_or(session)
                    .to_owned(),
            );
        }
        !self.scopes.is_empty()
    }
}

fn task_references(task: &PendingTask) -> TaskReferences {
    let mut refs = TaskReferences::default();
    let input = &task.inputs;
    let mut fields = BTreeMap::from([
        ("source_session_id", &mut refs.sessions),
        ("scope", &mut refs.scopes),
        ("archive_id", &mut refs.archives),
        ("memory_unit_id", &mut refs.memory_units),
        ("core_fact_id", &mut refs.core_facts),
        ("candidate_id", &mut refs.candidates),
        ("event_id", &mut refs.events),
    ]);
    collect_references(input, &mut fields);
    if let Some(outcome) = &task.outcome {
        collect_references(outcome, &mut fields);
    }
    refs
}

fn collect_references(value: &Value, fields: &mut BTreeMap<&str, &mut BTreeSet<String>>) {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                let field = match key.as_str() {
                    "session_id" | "source_session_ids" => "source_session_id",
                    "core_scope" | "personality_scope_id" | "reflection_scope" => "scope",
                    "preliminary_archive_id"
                    | "supporting_archive_ids"
                    | "contradicting_archive_ids" => "archive_id",
                    "memory_unit_ids" | "source_memory_unit_ids" => "memory_unit_id",
                    "contradicted_core_fact_ids" => "core_fact_id",
                    "new_candidate_ids" | "reinforced_candidate_ids" => "candidate_id",
                    "source_event_ids" => "event_id",
                    name => name,
                };
                if let Some(ids) = fields.get_mut(field) {
                    ids.extend(value.as_str().into_iter().map(str::to_owned));
                    ids.extend(
                        value
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned),
                    );
                } else if key != "payload" {
                    collect_references(value, fields);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_references(value, fields);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::IngestEvent;
    use std::fs;

    #[test]
    fn expiry_redacts_finished_sleep_and_review_without_touching_active_work() {
        let storage =
            FileStorage::new(std::env::temp_dir().join(new_id("retention_fixture").unwrap()));
        let engine = MemoryEngine::new(storage.clone());
        let session = "retention_session";
        engine
            .ingest(event(session, "The review source is private."))
            .unwrap();
        let run = engine
            .begin_sleep_run_with_options(
                session,
                SleepRunOptions {
                    enabled_tracks: vec![SleepTrack::MemoryUnit],
                    ..SleepRunOptions::default()
                },
            )
            .unwrap();
        let step = engine.next_sleep_batch(run).unwrap();
        let request = &step.batch.unwrap().requests[0];
        let run = engine.submit_sleep_batch(step.run, vec![LlmResponse::Ok {
            request_id: request.request_id.clone(),
            text: json!({
                "schema_version": MEMORY_UNITS_RESULT_SCHEMA_VERSION,
                "archive_id": request.prompt_inputs["archive_id"],
                "memory_units": [{
                    "thesis": "The user mentioned a private matter.",
                    "source_event_ids": storage.read_session(session).unwrap().events.into_iter().map(|event| event.event_id).collect::<Vec<_>>(),
                    "weight": 0.2
                }]
            }).to_string(),
        }]).unwrap().run;
        let sleep_task_id = run.sleep_task_id.clone();
        let finished = engine.finish_sleep_run(run).unwrap();
        let unit = &finished.archive_entry.memory_units[0];
        let fidelity = engine
            .begin_memory_fidelity_pass(&unit.memory_unit_id)
            .unwrap();
        let review = FidelityReview {
            schema_version: FIDELITY_REVIEW_SCHEMA_VERSION.into(),
            memory_unit_id: unit.memory_unit_id.clone(),
            archive_id: unit.archive_id.clone(),
            status: FidelityStatus::Valid,
            confidence: 0.95,
            explanation: "The private source supports this conclusion.".into(),
            revised_thesis: None,
            missing_detail: None,
        };
        let reviewed = engine
            .resume_memory_fidelity_pass(&fidelity.pending_task.task_id, review.clone())
            .unwrap();
        assert_eq!(
            engine
                .resume_memory_fidelity_pass(&fidelity.pending_task.task_id, review.clone())
                .unwrap(),
            reviewed
        );

        engine
            .ingest(event("open_session", "Keep the pending source."))
            .unwrap();
        let pending = engine.begin_sleep_run("open_session").unwrap();
        let before = engine
            .prune_memory_operations_at(OffsetDateTime::now_utc())
            .unwrap();
        assert_eq!(before.tasks_compacted, 0);

        let expiry = OffsetDateTime::now_utc() + Duration::days(31);
        let pruned = engine.prune_memory_operations_at(expiry).unwrap();
        assert_eq!(pruned.tasks_compacted, 3);
        assert_eq!(pruned.sleep_runs_compacted, 1);
        let archive_receipt = storage
            .read_memory_unit_publication_receipt(&unit.archive_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            archive_receipt["memory_unit_ids"],
            json!([unit.memory_unit_id])
        );
        assert!(archive_receipt.get("task_id").is_none());
        assert!(archive_receipt.get("request_digest").is_none());
        let task = storage.load_task(&fidelity.pending_task.task_id).unwrap();
        assert!(task_payload_compacted(&task));
        assert_eq!(
            task.inputs["references"]["memory_units"],
            json!([unit.memory_unit_id])
        );
        assert!(task.outcome.is_none());
        let finished = storage.load_sleep_run(&sleep_task_id).unwrap();
        assert_eq!(finished.stage, SleepRunStage::Finished);
        assert!(!sleep_payload_present(&finished));
        assert_eq!(
            storage.load_sleep_run(&pending.sleep_task_id).unwrap(),
            pending
        );
        drop(engine);

        let reopened = MemoryEngine::new(storage.clone());
        assert!(matches!(
            reopened.resume_memory_fidelity_pass(&fidelity.pending_task.task_id, review),
            Err(MemoryEngineError::OperationResultUnavailable)
        ));
        assert_eq!(
            storage
                .read_memory_unit_by_id(&unit.memory_unit_id)
                .unwrap(),
            reviewed
        );
        assert!(reopened
            .pending_memory_unit_repairs(session)
            .unwrap()
            .is_empty());
        let again = reopened.prune_memory_operations_at(expiry).unwrap();
        assert_eq!(again.tasks_compacted + again.sleep_runs_compacted, 0);

        // The retained IDs identify the compact records when erasing a scope.
        assert!(reopened.purge_scope(session).unwrap().deleted.tasks >= 3);
        assert!(storage.load_task(&fidelity.pending_task.task_id).is_err());
        assert!(storage.load_sleep_run(&pending.sleep_task_id).is_ok());
        fs::remove_dir_all(storage.root()).unwrap();
    }

    #[test]
    fn recovery_intents_keep_their_dependencies_until_they_complete() {
        let storage =
            FileStorage::new(std::env::temp_dir().join(new_id("retention_journals").unwrap()));
        let engine = MemoryEngine::new(storage.clone());
        let session = "retention_journal_session";
        engine
            .ingest(event(session, "Keep source while recovery can need it."))
            .unwrap();
        let sleep = engine.sleep(session).unwrap();
        let mut task = sleep.pending_task;
        task.state = TaskState::Completed;
        storage.save_task(&task).unwrap();
        let mut journal = JournalOperation {
            schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.into(),
            op_id: "retention_test_journal".into(),
            op_type: JournalOperationType::ReflectionApply,
            state: JournalState::Started,
            created_at: task.created_at.clone(),
            updated_at: task.updated_at.clone(),
            target_files: Vec::new(),
            intent: json!({"task_id": task.task_id, "checkpoint": "private fixture"}),
            recovery_policy: RecoveryPolicy::Retry,
            completed_at: None,
            error: None,
        };
        storage.begin_journaled_operation(&journal).unwrap();
        let now = OffsetDateTime::now_utc();
        let expiry = now + Duration::days(31);
        let report = engine.prune_memory_operations_at(expiry).unwrap();
        assert_eq!(report.tasks_compacted, 0);
        assert_eq!(report.skipped_records, 1);
        assert_eq!(storage.load_task(&task.task_id).unwrap(), task);
        assert_eq!(
            storage.load_journaled_operation(&journal.op_id).unwrap(),
            Some(journal.clone())
        );

        journal.state = JournalState::Completed;
        journal.completed_at = Some(now.format(&Rfc3339).unwrap());
        storage.begin_journaled_operation(&journal).unwrap();
        let report = engine.prune_memory_operations_at(expiry).unwrap();
        assert_eq!(report.tasks_compacted, 1);
        assert_eq!(report.journals_removed, 1);
        assert!(storage
            .load_journaled_operation(&journal.op_id)
            .unwrap()
            .is_none());
        fs::remove_dir_all(storage.root()).unwrap();
    }

    fn event(session: &str, text: &str) -> IngestEvent {
        serde_json::from_value(json!({
            "schema_version": EVENT_SCHEMA_VERSION,
            "type": "user_message",
            "source": "fixture",
            "timestamp": "2026-11-16T12:00:00Z",
            "session_id": session,
            "payload": {"text": text}
        }))
        .unwrap()
    }
}
