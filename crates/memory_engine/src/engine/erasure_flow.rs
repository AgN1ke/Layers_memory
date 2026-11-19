use super::*;
use crate::erasure::erasure_key;

#[derive(Serialize, Deserialize)]
struct GenerationErasureIntent {
    #[serde(flatten)]
    erasure: MemoryErasureOperationIntent,
    #[serde(default)]
    generation: Option<u64>,
}

impl<S: Storage> MemoryEngine<S> {
    /// Remove the live conversation record while preserving memories already
    /// materialized in the personality scope.
    pub fn delete_session(&self, session_id: &str) -> Result<MemoryErasureReport> {
        validate_erasure_target("session_id", session_id)?;
        self.ensure_manifest()?;
        self.with_session_memory_write(session_id, || {
            let (plan, counts) = self.build_delete_session_plan_unlocked(session_id)?;
            self.begin_memory_erasure_unlocked(plan, counts, true)
        })
    }

    /// Irreversibly remove every engine-owned record belonging to one
    /// personality scope. Hosts remain responsible for their own logs,
    /// backups and provider-side data.
    pub fn purge_scope(&self, personality_scope_id: &str) -> Result<MemoryErasureReport> {
        validate_erasure_target("personality_scope_id", personality_scope_id)?;
        self.ensure_manifest()?;
        self.with_resource_lock(scope_erasure_lock_key(personality_scope_id), || {
            self.require_erasure_complete(MemoryErasureKind::PurgeScope, personality_scope_id)?;
            let (plan, counts) = self.build_purge_scope_plan_unlocked(personality_scope_id)?;
            self.begin_memory_erasure_unlocked(plan, counts, false)
        })
    }

    fn begin_memory_erasure_unlocked(
        &self,
        plan: MemoryErasurePlan,
        planned_counts: MemoryErasureCounts,
        archived_memory_preserved: bool,
    ) -> Result<MemoryErasureReport> {
        let key = erasure_key(plan.operation, &plan.target_id);
        let generation = self.next_erasure_generation(&key)?;
        let now = now_rfc3339()?;
        let intent = GenerationErasureIntent {
            erasure: MemoryErasureOperationIntent {
                schema_version: MEMORY_ERASURE_OPERATION_SCHEMA_VERSION.to_string(),
                plan,
                planned_counts,
                archived_memory_preserved,
            },
            generation: Some(generation),
        };
        let operation = JournalOperation {
            schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
            op_id: key,
            op_type: match intent.erasure.plan.operation {
                MemoryErasureKind::DeleteSession => JournalOperationType::DeleteSession,
                MemoryErasureKind::PurgeScope => JournalOperationType::PurgeScope,
            },
            state: JournalState::Started,
            created_at: now.clone(),
            updated_at: now,
            target_files: vec![intent.erasure.plan.target_id.clone()],
            intent: serde_json::to_value(intent)?,
            recovery_policy: RecoveryPolicy::Retry,
            completed_at: None,
            error: None,
        };
        self.storage.begin_journaled_operation(&operation)?;
        self.execute_memory_erasure_operation_unlocked(operation)
    }

    fn next_erasure_generation(&self, key: &str) -> Result<u64> {
        self.storage
            .read_erasure_generation(key)?
            .checked_add(1)
            .ok_or_else(|| MemoryEngineError::Storage("erasure generation exhausted".into()))
    }

    pub(super) fn execute_memory_erasure_operation_unlocked(
        &self,
        operation: JournalOperation,
    ) -> Result<MemoryErasureReport> {
        let operation_id = operation.op_id.clone();
        let result = self.execute_memory_erasure_operation_inner(operation);
        if let Err(error) = &result {
            self.memory_operations_recovered
                .store(false, Ordering::Release);
            self.record_memory_operation_error_best_effort(&operation_id, error);
        }
        result
    }

    fn execute_memory_erasure_operation_inner(
        &self,
        mut operation: JournalOperation,
    ) -> Result<MemoryErasureReport> {
        if operation.state != JournalState::Started {
            return Err(MemoryEngineError::Validation(format!(
                "memory erasure operation is not started: {}",
                operation.op_id
            )));
        }
        let mut journal: GenerationErasureIntent =
            serde_json::from_value(operation.intent.clone())?;
        let intent = &journal.erasure;
        if intent.schema_version != MEMORY_ERASURE_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: MEMORY_ERASURE_OPERATION_SCHEMA_VERSION.to_string(),
                actual: intent.schema_version.clone(),
            });
        }
        let expected_type = match intent.plan.operation {
            MemoryErasureKind::DeleteSession => JournalOperationType::DeleteSession,
            MemoryErasureKind::PurgeScope => JournalOperationType::PurgeScope,
        };
        if operation.op_type != expected_type {
            return Err(MemoryEngineError::Validation(format!(
                "memory erasure journal type does not match its plan: {}",
                operation.op_id
            )));
        }

        let key = erasure_key(intent.plan.operation, &intent.plan.target_id);
        let generation = match journal.generation {
            Some(generation) => generation,
            None => {
                // Older journal records did not carry a generation. Freeze it
                // first so another restart repeats the very same transition.
                let generation = self.next_erasure_generation(&key)?;
                journal.generation = Some(generation);
                operation.intent = serde_json::to_value(&journal)?;
                self.storage.begin_journaled_operation(&operation)?;
                generation
            }
        };
        let current = self.storage.read_erasure_generation(&key)?;
        if current > generation {
            return Err(MemoryEngineError::Storage(
                "erasure journal trails its saved generation".into(),
            ));
        }
        if current < generation {
            self.storage.write_erasure_generation(&key, generation)?;
        }
        let intent = journal.erasure;
        self.storage.apply_memory_erasure_plan(&intent.plan)?;
        if !intent.plan.archive_ids.is_empty() {
            let archive_ids = intent
                .plan
                .archive_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let mut stats = self.recall_stats.lock().map_err(|_| {
                MemoryEngineError::Storage("recall stats mutex was poisoned".to_string())
            })?;
            stats.retain(|archive_id, _| !archive_ids.contains(archive_id.as_str()));
        }

        // The operation record is the crash-recovery marker. Removing it is
        // the final step; replay before this point is idempotent.
        self.storage.delete_journaled_operation(&operation.op_id)?;
        Ok(MemoryErasureReport {
            schema_version: MEMORY_ERASURE_REPORT_SCHEMA_VERSION.to_string(),
            operation: intent.plan.operation,
            target_id: intent.plan.target_id,
            completed_at: now_rfc3339()?,
            deleted: intent.planned_counts,
            archived_memory_preserved: intent.archived_memory_preserved,
        })
    }

    fn build_delete_session_plan_unlocked(
        &self,
        session_id: &str,
    ) -> Result<(MemoryErasurePlan, MemoryErasureCounts)> {
        let sessions = clean_storage_collection(self.storage.read_sessions()?)?;
        let session_exists = sessions
            .iter()
            .any(|metadata| metadata.session_id == session_id);
        let mut counts = MemoryErasureCounts::default();
        if session_exists {
            let session = self.storage.read_session(session_id)?;
            counts.sessions = 1;
            counts.active_events = session.events.len();
            counts.archived_events = self.storage.read_session_archived_events(session_id)?.len();
        }

        let sleep_runs = clean_storage_collection(self.storage.load_sleep_runs()?)?
            .into_iter()
            .filter(|run| run.session_id == session_id)
            .collect::<Vec<_>>();
        let sleep_task_ids = sleep_runs
            .iter()
            .map(|run| run.sleep_task_id.clone())
            .collect::<BTreeSet<_>>();
        let run_task_ids = sleep_runs
            .iter()
            .flat_map(sleep_run_task_ids)
            .collect::<BTreeSet<_>>();
        let mut identifiers = BTreeSet::from([session_id.to_string()]);
        identifiers.extend(run_task_ids.iter().cloned());
        identifiers.extend(sleep_runs.iter().map(|run| run.archive_id.clone()));

        let task_ids = clean_storage_collection(self.storage.load_all_tasks()?)?
            .into_iter()
            .filter(|task| {
                run_task_ids.contains(&task.task_id)
                    || pending_task_references_any(task, &identifiers)
            })
            .map(|task| task.task_id)
            .collect::<BTreeSet<_>>();
        identifiers.extend(task_ids.iter().cloned());

        let journal_operations =
            clean_storage_collection(self.storage.load_journaled_operations()?)?
                .into_iter()
                .filter(|operation| {
                    journal_operation_matches_erasure(
                        operation,
                        MemoryErasureKind::DeleteSession,
                        session_id,
                        &identifiers,
                    )
                })
                .collect::<Vec<_>>();
        // This operation keeps derived memory. Let recovery finish an
        // accepted batch before removing the session or its journal entries.
        if journal_operations.iter().any(|operation| {
            operation.op_type == JournalOperationType::MemoryUnitPublish
                && operation.state == JournalState::Started
        }) {
            self.memory_operations_recovered
                .store(false, Ordering::Release);
            return Err(MemoryEngineError::OperationIncomplete);
        }
        let journal_operation_ids = journal_operations
            .into_iter()
            .map(|operation| operation.op_id)
            .collect::<BTreeSet<_>>();

        counts.tasks = task_ids.len();
        counts.sleep_runs = sleep_task_ids.len();
        counts.journal_operations = journal_operation_ids.len();
        let plan = MemoryErasurePlan {
            schema_version: MEMORY_ERASURE_PLAN_SCHEMA_VERSION.to_string(),
            operation: MemoryErasureKind::DeleteSession,
            target_id: session_id.to_string(),
            session_ids: if session_exists {
                vec![session_id.to_string()]
            } else {
                Vec::new()
            },
            archive_ids: Vec::new(),
            memory_unit_ids: Vec::new(),
            core_facts: Vec::new(),
            candidate_ids: Vec::new(),
            hypothesis_node_ids: Vec::new(),
            hypothesis_evidence_link_ids: Vec::new(),
            hypothesis_relation_ids: Vec::new(),
            task_ids: task_ids.into_iter().collect(),
            sleep_task_ids: sleep_task_ids.into_iter().collect(),
            reflection_scopes: Vec::new(),
            vector_scopes: Vec::new(),
            journal_operation_ids: journal_operation_ids.into_iter().collect(),
        };
        Ok((plan, counts))
    }

    fn build_purge_scope_plan_unlocked(
        &self,
        scope: &str,
    ) -> Result<(MemoryErasurePlan, MemoryErasureCounts)> {
        let mut counts = MemoryErasureCounts::default();
        let sessions = clean_storage_collection(self.storage.read_sessions()?)?
            .into_iter()
            .filter(|metadata| metadata.effective_personality_scope_id() == scope)
            .collect::<Vec<_>>();
        let session_ids = sessions
            .iter()
            .map(|metadata| metadata.session_id.clone())
            .collect::<BTreeSet<_>>();
        for session_id in &session_ids {
            let session = self.storage.read_session(session_id)?;
            counts.active_events += session.events.len();
            counts.archived_events += self.storage.read_session_archived_events(session_id)?.len();
        }

        let archives = clean_storage_collection(self.storage.read_archive_inventory()?)?
            .into_iter()
            .filter(|archive| archive.effective_personality_scope_id() == scope)
            .collect::<Vec<_>>();
        let archive_ids = archives
            .iter()
            .map(|archive| archive.archive_id.clone())
            .collect::<BTreeSet<_>>();
        let memory_unit_ids = clean_storage_collection(self.storage.read_memory_unit_inventory()?)?
            .into_iter()
            .filter(|unit| {
                unit.effective_personality_scope_id() == scope
                    || archive_ids.contains(&unit.archive_id)
            })
            .map(|unit| unit.memory_unit_id)
            .collect::<BTreeSet<_>>();

        let core_facts = clean_storage_collection(self.storage.read_core_store_categories()?)?
            .into_iter()
            .flat_map(|category| {
                category
                    .facts
                    .into_iter()
                    .filter(move |fact| fact.scope.as_deref() == Some(scope))
                    .map(move |fact| ErasureCoreFactRef {
                        category: category.category.clone(),
                        core_fact_id: fact.core_fact_id,
                    })
            })
            .collect::<Vec<_>>();

        let candidates = clean_storage_collection(self.storage.read_candidate_beliefs()?)?
            .into_iter()
            .filter(|candidate| candidate.core_scope.as_deref() == Some(scope))
            .collect::<Vec<_>>();
        let candidate_ids = candidates
            .iter()
            .map(|candidate| candidate.candidate_id.clone())
            .collect::<BTreeSet<_>>();
        let hypothesis_nodes = clean_storage_collection(self.storage.read_hypothesis_nodes()?)?
            .into_iter()
            .filter(|node| node.core_scope.as_deref() == Some(scope))
            .collect::<Vec<_>>();
        let hypothesis_node_ids = hypothesis_nodes
            .iter()
            .map(|node| node.hypothesis_id.clone())
            .collect::<BTreeSet<_>>();
        let graph_ids = candidate_ids
            .iter()
            .chain(hypothesis_node_ids.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        let hypothesis_evidence_link_ids =
            clean_storage_collection(self.storage.read_hypothesis_evidence_links()?)?
                .into_iter()
                .filter(|link| {
                    graph_ids.contains(&link.hypothesis_id)
                        || memory_unit_ids.contains(&link.memory_unit_id)
                        || link
                            .archive_id
                            .as_ref()
                            .is_some_and(|id| archive_ids.contains(id))
                        || link
                            .source_session_id
                            .as_ref()
                            .is_some_and(|id| session_ids.contains(id))
                })
                .map(|link| link.evidence_link_id)
                .collect::<BTreeSet<_>>();
        let hypothesis_relation_ids =
            clean_storage_collection(self.storage.read_hypothesis_relations()?)?
                .into_iter()
                .filter(|relation| {
                    graph_ids.contains(&relation.source_hypothesis_id)
                        || graph_ids.contains(&relation.target_hypothesis_id)
                })
                .map(|relation| relation.relation_id)
                .collect::<BTreeSet<_>>();

        let sleep_runs = clean_storage_collection(self.storage.load_sleep_runs()?)?
            .into_iter()
            .filter(|run| {
                session_ids.contains(&run.session_id) || archive_ids.contains(&run.archive_id)
            })
            .collect::<Vec<_>>();
        let sleep_task_ids = sleep_runs
            .iter()
            .map(|run| run.sleep_task_id.clone())
            .collect::<BTreeSet<_>>();
        let run_task_ids = sleep_runs
            .iter()
            .flat_map(sleep_run_task_ids)
            .collect::<BTreeSet<_>>();

        let reflection_scopes = clean_storage_collection(self.storage.read_reflection_states()?)?
            .into_iter()
            .filter(|state| {
                state.reflection_scope == scope
                    || state
                        .source_session_ids
                        .iter()
                        .any(|id| session_ids.contains(id))
                    || session_ids.contains(&state.source_session_id)
            })
            .map(|state| state.reflection_scope)
            .collect::<BTreeSet<_>>();

        let mut identifiers = BTreeSet::from([scope.to_string()]);
        identifiers.extend(session_ids.iter().cloned());
        identifiers.extend(archive_ids.iter().cloned());
        identifiers.extend(memory_unit_ids.iter().cloned());
        identifiers.extend(core_facts.iter().map(|fact| fact.core_fact_id.clone()));
        identifiers.extend(graph_ids.iter().cloned());
        identifiers.extend(hypothesis_evidence_link_ids.iter().cloned());
        identifiers.extend(hypothesis_relation_ids.iter().cloned());
        identifiers.extend(run_task_ids.iter().cloned());
        identifiers.extend(reflection_scopes.iter().cloned());

        let task_ids = clean_storage_collection(self.storage.load_all_tasks()?)?
            .into_iter()
            .filter(|task| {
                run_task_ids.contains(&task.task_id)
                    || pending_task_references_any(task, &identifiers)
            })
            .map(|task| task.task_id)
            .collect::<BTreeSet<_>>();
        identifiers.extend(task_ids.iter().cloned());
        let journal_operation_ids =
            clean_storage_collection(self.storage.load_journaled_operations()?)?
                .into_iter()
                .filter(|operation| {
                    journal_operation_matches_erasure(
                        operation,
                        MemoryErasureKind::PurgeScope,
                        scope,
                        &identifiers,
                    )
                })
                .map(|operation| operation.op_id)
                .collect::<BTreeSet<_>>();

        let vector_scopes = if self.storage.read_vector_index(scope)?.is_some() {
            BTreeSet::from([scope.to_string()])
        } else {
            BTreeSet::new()
        };

        counts.sessions = session_ids.len();
        counts.archives = archive_ids.len();
        counts.memory_units = memory_unit_ids.len();
        counts.core_facts = core_facts.len();
        counts.candidates = candidate_ids.len();
        counts.hypothesis_nodes = hypothesis_node_ids.len();
        counts.hypothesis_evidence_links = hypothesis_evidence_link_ids.len();
        counts.hypothesis_relations = hypothesis_relation_ids.len();
        counts.tasks = task_ids.len();
        counts.sleep_runs = sleep_task_ids.len();
        counts.reflection_states = reflection_scopes.len();
        counts.vector_scopes = vector_scopes.len();
        counts.journal_operations = journal_operation_ids.len();

        let plan = MemoryErasurePlan {
            schema_version: MEMORY_ERASURE_PLAN_SCHEMA_VERSION.to_string(),
            operation: MemoryErasureKind::PurgeScope,
            target_id: scope.to_string(),
            session_ids: session_ids.into_iter().collect(),
            archive_ids: archive_ids.into_iter().collect(),
            memory_unit_ids: memory_unit_ids.into_iter().collect(),
            core_facts,
            candidate_ids: candidate_ids.into_iter().collect(),
            hypothesis_node_ids: hypothesis_node_ids.into_iter().collect(),
            hypothesis_evidence_link_ids: hypothesis_evidence_link_ids.into_iter().collect(),
            hypothesis_relation_ids: hypothesis_relation_ids.into_iter().collect(),
            task_ids: task_ids.into_iter().collect(),
            sleep_task_ids: sleep_task_ids.into_iter().collect(),
            reflection_scopes: reflection_scopes.into_iter().collect(),
            vector_scopes: vector_scopes.into_iter().collect(),
            journal_operation_ids: journal_operation_ids.into_iter().collect(),
        };
        Ok((plan, counts))
    }
}

pub(super) fn scope_erasure_lock_key(scope: &str) -> String {
    format!("memory_erasure:scope:{scope}")
}

fn clean_storage_collection<T>(collection: StorageCollection<T>) -> Result<Vec<T>> {
    if let Some(warning) = collection.warnings.first() {
        return Err(MemoryEngineError::Storage(warning.note()));
    }
    Ok(collection.items)
}

pub(super) fn sleep_run_task_ids(run: &SleepRun) -> Vec<String> {
    let mut ids = vec![run.sleep_task_id.clone()];
    ids.extend(run.memory_unit_task_id.iter().cloned());
    ids.extend(
        run.requests
            .iter()
            .map(|request| request.request.task_id.clone()),
    );
    ids.sort();
    ids.dedup();
    ids
}

fn pending_task_references_any(task: &PendingTask, identifiers: &BTreeSet<String>) -> bool {
    json_references_any(&task.inputs, identifiers)
        || task
            .outcome
            .as_ref()
            .is_some_and(|outcome| json_references_any(outcome, identifiers))
}

fn journal_operation_matches_erasure(
    operation: &JournalOperation,
    kind: MemoryErasureKind,
    target: &str,
    identifiers: &BTreeSet<String>,
) -> bool {
    if operation.op_type == JournalOperationType::MemoryUnitPublish {
        let field = match kind {
            MemoryErasureKind::DeleteSession => "source_session_id",
            MemoryErasureKind::PurgeScope => "scope",
        };
        return operation.intent[field].as_str() == Some(target);
    }
    if operation.op_type == JournalOperationType::IngestEvent {
        let event = &operation.intent["event"];
        let session = event["session_id"].as_str();
        let origin = match kind {
            MemoryErasureKind::DeleteSession => session,
            MemoryErasureKind::PurgeScope => event["personality_scope_id"].as_str().or(session),
        };
        return origin == Some(target);
    }
    json_references_any(&operation.intent, identifiers)
        || operation.target_files.iter().any(|target| {
            identifiers
                .iter()
                .any(|identifier| target == identifier || target.contains(identifier))
        })
}

pub(super) fn json_references_any(value: &Value, identifiers: &BTreeSet<String>) -> bool {
    match value {
        Value::String(value) => identifiers.contains(value),
        Value::Array(values) => values
            .iter()
            .any(|value| json_references_any(value, identifiers)),
        Value::Object(values) => values
            .values()
            .any(|value| json_references_any(value, identifiers)),
        _ => false,
    }
}

pub(super) fn validate_erasure_target(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
        || value.contains("..")
        || value.chars().any(char::is_control)
    {
        return Err(MemoryEngineError::Validation(format!(
            "invalid {label} for memory erasure: {value}"
        )));
    }
    Ok(())
}
