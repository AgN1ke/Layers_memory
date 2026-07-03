use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn build_evidence_pack(&self, memory_unit_id: &str) -> Result<EvidencePack> {
        if memory_unit_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory_unit_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let session_id = unit.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            self.build_evidence_pack_unlocked(memory_unit_id)
        })
    }

    pub fn begin_memory_fidelity_pass(
        &self,
        memory_unit_id: &str,
    ) -> Result<MemoryFidelityPassStart> {
        if memory_unit_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory_unit_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let session_id = unit.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            self.begin_memory_fidelity_pass_unlocked(memory_unit_id)
        })
    }

    pub(super) fn begin_memory_fidelity_pass_unlocked(
        &self,
        memory_unit_id: &str,
    ) -> Result<MemoryFidelityPassStart> {
        let evidence_pack = self.build_evidence_pack_unlocked(memory_unit_id)?;
        let now = now_rfc3339()?;
        let task = PendingTask {
            schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
            task_id: new_id("task")?,
            task_type: TaskType::MemoryFidelityPass,
            state: TaskState::Pending,
            created_at: now.clone(),
            updated_at: now,
            prompt_id: self.options.fidelity.prompt_id.clone(),
            prompt_version: self.options.fidelity.prompt_version,
            role_hint: ModelRole::Reasoning,
            expected_output_schema: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
            inputs: serde_json::to_value(&evidence_pack)?,
            attempts: Vec::new(),
            last_error: None,
        };
        self.storage.save_task(&task)?;
        let request = llm_request_from_task(
            &task,
            &self.options.fidelity.prompt_id,
            json!({ "evidence_pack": evidence_pack.clone() }),
        )?;
        Ok(MemoryFidelityPassStart {
            evidence_pack,
            pending_task: task,
            request,
        })
    }

    pub fn submit_memory_fidelity_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<MemoryUnit> {
        let request_id = llm_response_request_id(&response).to_string();
        match response {
            LlmResponse::Ok { text, .. } => {
                let result = (|| {
                    let value = parse_json_value_from_llm_text(&text)?;
                    let mut review: FidelityReview = serde_json::from_value(value)?;
                    if review.schema_version.trim().is_empty() {
                        review.schema_version = FIDELITY_REVIEW_SCHEMA_VERSION.to_string();
                    }
                    self.resume_memory_fidelity_pass(task_id, review)
                })();
                if let Err(err) = &result {
                    self.mark_memory_fidelity_task_failed_best_effort(
                        task_id,
                        format!("{request_id} semantic error: {err}"),
                    );
                }
                result
            }
            LlmResponse::Err { kind, detail, .. } => {
                self.mark_memory_fidelity_task_failed_best_effort(
                    task_id,
                    format!("{request_id} {kind:?}: {detail}"),
                );
                Err(MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass failed: {kind:?}: {detail}"
                )))
            }
        }
    }

    pub fn resume_memory_fidelity_pass(
        &self,
        task_id: &str,
        mut review: FidelityReview,
    ) -> Result<MemoryUnit> {
        if review.schema_version != FIDELITY_REVIEW_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                actual: review.schema_version.clone(),
            });
        }
        review.explanation = normalize_whitespace(&review.explanation);
        review.revised_thesis = normalize_optional_string(review.revised_thesis.as_deref());
        review.missing_detail = normalize_optional_string(review.missing_detail.as_deref());
        review.validate_basic()?;
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::MemoryFidelityPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not memory_fidelity_pass: {task_id}"
            )));
        }

        let task_memory_unit_id = task
            .inputs
            .get("memory_unit_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass task has no memory_unit_id: {task_id}"
                ))
            })?;
        if task_memory_unit_id != review.memory_unit_id {
            return Err(MemoryEngineError::Validation(format!(
                "memory_fidelity_pass memory_unit_id mismatch: task={task_memory_unit_id} result={}",
                review.memory_unit_id
            )));
        }

        let session_id = task
            .inputs
            .get("source_session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass task has no source_session_id: {task_id}"
                ))
            })?
            .to_string();

        self.with_resource_lock(session_lock_key(&session_id), || {
            let now = now_rfc3339()?;
            let mut unit = self
                .storage
                .read_memory_unit_by_id(&review.memory_unit_id)?;
            if unit.archive_id != review.archive_id {
                return Err(MemoryEngineError::Validation(format!(
                    "fidelity review archive_id mismatch: unit={} review={}",
                    unit.archive_id, review.archive_id
                )));
            }

            unit.updated_at = now.clone();
            unit.fidelity_status = review.status;
            unit.status = memory_unit_status_after_fidelity(review.status);
            unit.fidelity_review = Some(review.clone());
            self.storage.write_memory_unit(&unit)?;
            if unit.status != MemoryUnitStatus::ActiveArchive {
                self.tombstone_vector_unit_if_indexed_unlocked(&unit)?;
            }

            let mut archive_entry = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
            archive_entry.updated_at = now.clone();
            for archive_unit in &mut archive_entry.memory_units {
                if archive_unit.memory_unit_id == unit.memory_unit_id {
                    *archive_unit = unit.clone();
                }
            }
            archive_entry.compact_memory =
                render_compact_memory_from_units(&archive_entry.memory_units);
            self.storage
                .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            self.storage.save_task(&task)?;

            Ok(unit)
        })
    }

    pub(super) fn build_evidence_pack_unlocked(
        &self,
        memory_unit_id: &str,
    ) -> Result<EvidencePack> {
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let archive = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
        let session_events = self.read_session_events_with_archived(&unit.source_session_id)?;
        let now = now_rfc3339()?;
        let mut pack = EvidencePack::empty_for(
            new_id("evidence_pack")?,
            now,
            unit.memory_unit_id.clone(),
            unit.archive_id.clone(),
            unit.source_session_id.clone(),
            unit.thesis.clone(),
            self.options.fidelity.max_evidence_tokens,
        );
        pack.unit_evidence = unit.evidence.clone();

        let source_ids = if unit.source_event_ids.is_empty() {
            archive.source_event_ids.iter().collect::<HashSet<_>>()
        } else {
            unit.source_event_ids.iter().collect::<HashSet<_>>()
        };
        let source_indices = session_events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| source_ids.contains(&event.event_id).then_some(index))
            .collect::<Vec<_>>();

        let mut selected = HashSet::new();
        for index in &source_indices {
            if let Some(event) = session_events.get(*index) {
                let event = evidence_event_from_stored(
                    event,
                    EvidenceEventRole::Source,
                    self.options.fidelity.max_event_text_chars,
                );
                add_evidence_event(&mut pack, event, &mut selected, true);
            }
        }

        let mut neighbor_indices = BTreeSet::new();
        for source_index in &source_indices {
            for offset in 1..=self.options.fidelity.neighbor_events {
                if let Some(left) = source_index.checked_sub(offset) {
                    neighbor_indices.insert(left);
                }
                let right = source_index + offset;
                if right < session_events.len() {
                    neighbor_indices.insert(right);
                }
            }
        }

        for index in neighbor_indices {
            let Some(event) = session_events.get(index) else {
                continue;
            };
            let event = evidence_event_from_stored(
                event,
                EvidenceEventRole::Neighbor,
                self.options.fidelity.max_event_text_chars,
            );
            add_evidence_event(&mut pack, event, &mut selected, false);
        }

        pack.estimated_tokens = estimate_evidence_pack_tokens(&pack);
        Ok(pack)
    }

    pub(super) fn mark_memory_fidelity_task_failed_best_effort(
        &self,
        task_id: &str,
        detail: String,
    ) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::MemoryFidelityPass {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }
}
