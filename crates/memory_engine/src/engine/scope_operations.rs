use super::*;
use crate::inspection::ScopedPendingOperation;

impl<S: Storage> MemoryEngine<S> {
    pub(super) fn scoped_pending_operations(
        &self,
        scope_id: &str,
        sessions: &HashSet<&str>,
        archives: &[ArchiveEntry],
    ) -> Result<Vec<ScopedPendingOperation>> {
        let sources = OperationSources::new(scope_id, sessions, archives);
        let tasks = self.storage.load_tasks()?.items;
        let task_ids = tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<HashSet<_>>();
        let mut pending = HashMap::new();
        let mut core_fact_ids = None;
        for task in tasks.iter().filter(|task| {
            matches!(task.state, TaskState::Pending | TaskState::Submitted)
                && (task.schema_version.is_empty()
                    || task.schema_version == PENDING_TASK_SCHEMA_VERSION)
        }) {
            let allowed = match task.task_type {
                TaskType::SleepCompression
                | TaskType::CompactMemoryPass
                | TaskType::MemoryUnitPass => sources.sleep_archive(&task.inputs).is_some(),
                TaskType::MemoryFidelityPass => sources.has_unit(&task.inputs),
                TaskType::ForgetReview => sources.has_forget_review(&task.inputs),
                TaskType::ReflectionAnalyze => {
                    sources.has_reflection(&task.inputs)
                        && self.scoped_reflection_core_facts(
                            &sources,
                            &task.inputs,
                            &mut core_fact_ids,
                        )?
                        && self.scoped_reflection_candidates(&sources, &task.inputs)
                }
                TaskType::ComputeEmbedding => sources.has_embedding_batch(&task.inputs),
                _ => false,
            };
            if !allowed {
                continue;
            }
            let needs_new_review = match task.task_type {
                TaskType::MemoryFidelityPass => {
                    !super::fidelity_flow::fidelity_task_has_basis(task)
                }
                TaskType::ForgetReview => !super::forgetting_flow::forget_task_has_basis(task),
                _ => false,
            };
            if let Some(operation) = pending_operation(
                &task.task_id,
                task.task_type,
                task.state,
                &task.created_at,
                &task.updated_at,
                needs_new_review,
            ) {
                pending.insert(operation.task_id.clone(), operation);
            }
        }

        // An interrupted finish can leave the compression task complete while
        // the run itself remains open. Its Archive still bounds the checkpoint.
        for run in self.storage.load_sleep_runs()?.items {
            if run.stage == SleepRunStage::Finished
                || task_ids.contains(run.sleep_task_id.as_str())
                || validate_sleep_run(&run).is_err()
            {
                continue;
            }
            let Some(archive) = sources.archive(&run.archive_id, &run.session_id) else {
                continue;
            };
            if let Some(operation) = pending_operation(
                &run.sleep_task_id,
                TaskType::SleepCompression,
                TaskState::Pending,
                &archive.created_at,
                &archive.updated_at,
                false,
            ) {
                pending.insert(operation.task_id.clone(), operation);
            }
        }

        let mut pending = pending.into_values().collect::<Vec<_>>();
        pending.sort_unstable_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        Ok(pending)
    }

    fn scoped_reflection_core_facts(
        &self,
        sources: &OperationSources<'_>,
        inputs: &Value,
        core_fact_ids: &mut Option<HashSet<String>>,
    ) -> Result<bool> {
        let Some(facts) = inputs["core_facts"].as_array() else {
            return Ok(false);
        };
        if facts.is_empty() {
            return Ok(true);
        }
        if inputs["core_scope"].as_str() != Some(sources.scope_id) {
            return Ok(false);
        }
        let ids = match core_fact_ids {
            Some(ids) => ids,
            None => core_fact_ids.insert(
                self.storage
                    .read_core_store_categories()?
                    .items
                    .into_iter()
                    .flat_map(|category| category.facts)
                    .filter(|fact| {
                        fact.scope.as_deref() == Some(sources.scope_id)
                            && fact
                                .source_archive_ids
                                .iter()
                                .all(|id| sources.archives.contains_key(id.as_str()))
                    })
                    .map(|fact| fact.core_fact_id)
                    .collect(),
            ),
        };
        Ok(facts.iter().all(|fact| {
            fact["core_fact_id"]
                .as_str()
                .is_some_and(|id| ids.contains(id))
        }))
    }

    fn scoped_reflection_candidates(&self, sources: &OperationSources<'_>, inputs: &Value) -> bool {
        inputs["existing_candidates"]
            .as_array()
            .is_some_and(|candidates| {
                candidates.iter().all(|reference| {
                    sources.has_candidate_references(reference)
                        && reference["candidate_id"]
                            .as_str()
                            .and_then(|id| {
                                self.storage
                                    .read_candidate_belief_optional(id)
                                    .ok()
                                    .flatten()
                                    .filter(|candidate| candidate.candidate_id == id)
                            })
                            .is_some_and(|candidate| {
                                candidate.core_scope.as_deref() == Some(sources.scope_id)
                                    && candidate
                                        .source_session_id
                                        .iter()
                                        .chain(&candidate.source_session_ids)
                                        .all(|id| sources.sessions.contains(id.as_str()))
                                    && candidate
                                        .supporting_archive_ids
                                        .iter()
                                        .chain(&candidate.contradicting_archive_ids)
                                        .all(|id| sources.archives.contains_key(id.as_str()))
                                    && candidate
                                        .source_memory_unit_ids
                                        .iter()
                                        .all(|id| sources.units.contains_key(id.as_str()))
                            })
                })
            })
    }
}

struct OperationSources<'a> {
    scope_id: &'a str,
    sessions: &'a HashSet<&'a str>,
    archives: HashMap<&'a str, &'a ArchiveEntry>,
    units: HashMap<&'a str, &'a MemoryUnit>,
}

impl<'a> OperationSources<'a> {
    fn new(
        scope_id: &'a str,
        sessions: &'a HashSet<&'a str>,
        archives: &'a [ArchiveEntry],
    ) -> Self {
        let archives = archives
            .iter()
            .filter(|archive| {
                archive.effective_personality_scope_id() == scope_id
                    && sessions.contains(archive.source_session_id.as_str())
                    && archive.status != ArchiveStatus::Superseded
            })
            .map(|archive| (archive.archive_id.as_str(), archive))
            .collect::<HashMap<_, _>>();
        let units = archives
            .values()
            .flat_map(|archive| {
                archive.memory_units.iter().filter(|unit| {
                    unit.archive_id == archive.archive_id
                        && unit.source_session_id == archive.source_session_id
                        && unit.effective_personality_scope_id() == scope_id
                })
            })
            .map(|unit| (unit.memory_unit_id.as_str(), unit))
            .collect();
        Self {
            scope_id,
            sessions,
            archives,
            units,
        }
    }

    fn archive(&self, archive_id: &str, session_id: &str) -> Option<&ArchiveEntry> {
        self.archives
            .get(archive_id)
            .copied()
            .filter(|archive| archive.source_session_id == session_id)
    }

    fn sleep_archive(&self, inputs: &Value) -> Option<&ArchiveEntry> {
        let archive = self.archive(
            inputs["preliminary_archive_id"].as_str()?,
            inputs["session_id"].as_str()?,
        )?;
        // Legacy tasks may lack the frozen inputs. Show their metadata when
        // the parent is proven, but refuse any explicit foreign relationship.
        if inputs.get("events").is_none_or(|events| {
            events.as_array().is_some_and(|events| {
                events.iter().all(|event| {
                    event["session_id"].as_str() == Some(&archive.source_session_id)
                        && match event.get("personality_scope_id") {
                            None | Some(Value::Null) => true,
                            Some(scope) => scope.as_str() == Some(self.scope_id),
                        }
                })
            })
        }) {
            Some(archive)
        } else {
            None
        }
    }

    fn has_unit(&self, reference: &Value) -> bool {
        reference["memory_unit_id"]
            .as_str()
            .and_then(|id| self.units.get(id))
            .is_some_and(|unit| {
                reference["archive_id"].as_str() == Some(&unit.archive_id)
                    && reference["source_session_id"].as_str() == Some(&unit.source_session_id)
            })
    }

    fn has_forget_review(&self, inputs: &Value) -> bool {
        let Some(session_id) = inputs["source_session_id"].as_str() else {
            return false;
        };
        self.sessions.contains(session_id)
            && inputs["candidates"].as_array().is_some_and(|candidates| {
                !candidates.is_empty()
                    && candidates.iter().all(|candidate| {
                        candidate["memory_unit_id"]
                            .as_str()
                            .and_then(|id| self.units.get(id))
                            .is_some_and(|unit| {
                                unit.source_session_id == session_id
                                    && candidate["archive_id"].as_str() == Some(&unit.archive_id)
                            })
                    })
            })
    }

    fn has_embedding_batch(&self, inputs: &Value) -> bool {
        inputs["scope"].as_str() == Some(self.scope_id)
            && inputs["kind"].as_str() == Some("embed_batch")
            && inputs["items"].as_array().is_some_and(|items| {
                !items.is_empty()
                    && items.iter().all(|item| {
                        item["memory_unit_id"]
                            .as_str()
                            .is_some_and(|id| self.units.contains_key(id))
                    })
            })
    }

    fn has_reflection(&self, inputs: &Value) -> bool {
        let Some(session_id) = inputs["source_session_id"].as_str() else {
            return false;
        };
        let reflection_scope = match inputs.get("core_scope") {
            None | Some(Value::Null) => session_id,
            Some(scope) if scope.as_str() == Some(self.scope_id) => self.scope_id,
            Some(_) => return false,
        };
        self.sessions.contains(session_id)
            && inputs["reflection_scope"].as_str() == Some(reflection_scope)
            && reference_ids_allowed(&inputs["source_session_ids"], |id| {
                self.sessions.contains(id)
            })
            && reference_ids_allowed(&inputs["new_memory_unit_ids"], |id| {
                self.units.contains_key(id)
            })
            && reference_ids_allowed(&inputs["related_memory_unit_ids"], |id| {
                self.units.contains_key(id)
            })
            && inputs["memory_units"]
                .as_array()
                .is_some_and(|units| units.iter().all(|unit| self.has_unit(unit)))
            && inputs["archives"].as_array().is_some_and(|archives| {
                archives.iter().all(|archive| {
                    archive["archive_id"]
                        .as_str()
                        .zip(archive["source_session_id"].as_str())
                        .is_some_and(|(id, session)| self.archive(id, session).is_some())
                })
            })
    }

    fn has_candidate_references(&self, reference: &Value) -> bool {
        let source_session_is_allowed = match reference.get("source_session_id") {
            None | Some(Value::Null) => true,
            Some(id) => id.as_str().is_some_and(|id| self.sessions.contains(id)),
        };
        source_session_is_allowed
            && reference_ids_allowed(&reference["source_session_ids"], |id| {
                self.sessions.contains(id)
            })
            && reference_ids_allowed(&reference["supporting_archive_ids"], |id| {
                self.archives.contains_key(id)
            })
            && reference_ids_allowed(&reference["source_memory_unit_ids"], |id| {
                self.units.contains_key(id)
            })
    }
}

fn reference_ids_allowed(values: &Value, includes: impl Fn(&str) -> bool) -> bool {
    values
        .as_array()
        .is_some_and(|ids| ids.iter().all(|id| id.as_str().is_some_and(&includes)))
}

fn pending_operation(
    id: &str,
    task_type: TaskType,
    state: TaskState,
    created_at: &str,
    updated_at: &str,
    needs_new_review: bool,
) -> Option<ScopedPendingOperation> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || OffsetDateTime::parse(created_at, &Rfc3339).is_err()
        || OffsetDateTime::parse(updated_at, &Rfc3339).is_err()
    {
        return None;
    }
    Some(ScopedPendingOperation {
        task_id: id.to_owned(),
        task_type,
        state,
        created_at: created_at.to_owned(),
        updated_at: updated_at.to_owned(),
        needs_new_review,
    })
}
