use super::transactional_support::{serializable_digest, DecisionReceipt};
use super::*;

#[derive(Serialize, Deserialize)]
struct FrozenFidelityBasis {
    digest: String,
    max_event_text_chars: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum FidelityOutcome {
    Applied(Box<DecisionReceipt<MemoryUnit>>),
    NeedsNewReview,
}

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
        self.with_memory_write(
            unit.effective_personality_scope_id(),
            &unit.source_session_id,
            || self.begin_memory_fidelity_pass_unlocked(memory_unit_id),
        )
    }

    pub(super) fn begin_memory_fidelity_pass_unlocked(
        &self,
        memory_unit_id: &str,
    ) -> Result<MemoryFidelityPassStart> {
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let source_ids = self.fidelity_source_ids(&unit)?;
        let events = self.read_session_events_with_archived(&unit.source_session_id)?;
        self.begin_memory_fidelity_pass_from(&unit, &source_ids, &events)
    }

    pub fn begin_memory_fidelity_pass_in_scope(
        &self,
        scope: &str,
        memory_unit_id: &str,
    ) -> Result<Option<MemoryFidelityPassStart>> {
        super::scope_inspection::validate_inspection_scope(scope)?;
        if !super::scope_inspection::inspection_id_is_valid(memory_unit_id) {
            return Err(MemoryEngineError::Validation(
                "invalid memory unit ID".into(),
            ));
        }
        let lookup = || {
            self.storage
                .read_memory_units_by_id(&[memory_unit_id])
                .map(|mut units| {
                    units.remove(memory_unit_id).filter(|unit| {
                        unit.effective_personality_scope_id() == scope
                            && super::scope_inspection::inspection_id_is_valid(
                                &unit.source_session_id,
                            )
                    })
                })
        };
        let Some(snapshot) = lookup()? else {
            return Ok(None);
        };
        self.ensure_manifest()?;
        self.with_memory_write(scope, &snapshot.source_session_id, || {
            let Some(unit) = lookup()? else {
                return Ok(None);
            };
            if unit.archive_id != snapshot.archive_id
                || unit.source_session_id != snapshot.source_session_id
            {
                return Err(MemoryEngineError::OperationStale);
            }
            let Some(parent) = self
                .storage
                .read_archive_detail_scoped(scope, &unit.archive_id)?
                .and_then(|projection| projection.items.into_iter().next())
            else {
                return Ok(None);
            };
            if parent.source_session_id != unit.source_session_id
                || !parent
                    .memory_units
                    .iter()
                    .any(|member| member.memory_unit_id == memory_unit_id)
            {
                return Ok(None);
            }
            let parent_ids = parent
                .source_event_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let source_ids: BTreeSet<String> = if unit.source_event_ids.is_empty() {
                parent.source_event_ids.iter().cloned().collect()
            } else {
                unit.source_event_ids.iter().cloned().collect()
            };
            let mut events = self.read_session_events_with_archived(&unit.source_session_id)?;
            events.retain(|event| {
                event.session_id == unit.source_session_id
                    && event.effective_personality_scope_id() == scope
                    && event.validate().is_ok()
            });
            let resolved = events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<HashSet<_>>();
            if source_ids.is_empty()
                || source_ids
                    .iter()
                    .any(|id| !parent_ids.contains(id.as_str()) || !resolved.contains(id.as_str()))
            {
                return Err(MemoryEngineError::OperationResultUnavailable);
            }
            self.begin_memory_fidelity_pass_from(&unit, &source_ids, &events)
                .map(Some)
        })
    }

    fn begin_memory_fidelity_pass_from(
        &self,
        unit: &MemoryUnit,
        source_ids: &BTreeSet<String>,
        events: &[StoredEvent],
    ) -> Result<MemoryFidelityPassStart> {
        let evidence_pack = self.build_evidence_pack_from(unit, source_ids, events)?;
        let basis = FrozenFidelityBasis {
            digest: fidelity_basis_digest(unit, source_ids, &evidence_pack.events)?,
            max_event_text_chars: self.options.fidelity.max_event_text_chars,
        };
        let mut inputs = serde_json::to_value(&evidence_pack)?;
        inputs["fidelity_basis"] = serde_json::to_value(basis)?;
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
            inputs,
            attempts: Vec::new(),
            last_error: None,
            outcome: None,
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
                self.with_fidelity_task(task_id, |mut task| {
                    if !matches!(
                        task.state,
                        TaskState::Pending | TaskState::Submitted | TaskState::Failed
                    ) {
                        return Ok(());
                    }
                    task.state = TaskState::Failed;
                    task.updated_at = now_rfc3339()?;
                    task.last_error = Some(format!("{request_id} {kind:?}: {detail}"));
                    self.storage.save_task(&task)
                })?;
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
        self.with_fidelity_task(task_id, |mut task| {
            let task_memory_unit_id = fidelity_task_unit_id(&task)?;
            if task_memory_unit_id != review.memory_unit_id {
                return Err(MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass memory_unit_id mismatch: task={task_memory_unit_id} result={}",
                    review.memory_unit_id
                )));
            }
            if task.inputs["archive_id"].as_str() != Some(review.archive_id.as_str()) {
                return Err(MemoryEngineError::Validation(format!(
                    "memory_fidelity_pass archive_id mismatch: task={} result={}",
                    task.inputs["archive_id"], review.archive_id
                )));
            }
            let response_digest = serializable_digest(&review)?;
            if let Some(outcome) = task.outcome.take() {
                return match serde_json::from_value(outcome)? {
                    FidelityOutcome::Applied(receipt) => receipt.replay(&response_digest),
                    FidelityOutcome::NeedsNewReview => Err(MemoryEngineError::NeedsNewReview),
                };
            }
            if task.state == TaskState::Completed {
                return Err(MemoryEngineError::OperationResultUnavailable);
            }
            if task.state == TaskState::Cancelled {
                return Err(MemoryEngineError::NeedsNewReview);
            }
            let now = now_rfc3339()?;
            let mut unit = self
                .storage
                .read_memory_unit_by_id(&review.memory_unit_id)?;
            if !self.fidelity_basis_matches(&task, &unit)? {
                task.state = TaskState::Cancelled;
                task.updated_at = now;
                task.last_error = Some(MemoryEngineError::NeedsNewReview.to_string());
                task.outcome = Some(serde_json::to_value(FidelityOutcome::NeedsNewReview)?);
                self.storage.save_task(&task)?;
                return Err(MemoryEngineError::NeedsNewReview);
            }

            unit.updated_at = now.clone();
            unit.fidelity_status = review.status;
            if unit.status != MemoryUnitStatus::Forgotten {
                unit.status = memory_unit_status_after_fidelity(review.status);
            }
            unit.fidelity_review = Some(review.clone());
            self.storage.write_memory_unit(&unit)?;
            if unit.status != MemoryUnitStatus::ActiveArchive {
                self.tombstone_vector_unit_if_indexed_unlocked(&unit)?;
            }

            self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, &now)?;

            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            let receipt = DecisionReceipt::new(response_digest, unit.clone())?;
            task.outcome = Some(serde_json::to_value(FidelityOutcome::Applied(Box::new(receipt)))?);
            self.storage.save_task(&task)?;

            Ok(unit)
        })
    }

    fn with_fidelity_task<T>(
        &self,
        task_id: &str,
        f: impl FnOnce(PendingTask) -> Result<T>,
    ) -> Result<T> {
        self.ensure_manifest()?;
        let task = self.load_task_for_response(task_id)?;
        let unit = self
            .storage
            .read_memory_unit_by_id(fidelity_task_unit_id(&task)?)
            .map_err(|error| self.task_after_erasure_error(task_id, error))?;
        self.with_memory_write(
            unit.effective_personality_scope_id(),
            &unit.source_session_id,
            || f(self.load_task_for_response(task_id)?),
        )
    }

    pub(super) fn build_evidence_pack_unlocked(
        &self,
        memory_unit_id: &str,
    ) -> Result<EvidencePack> {
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let source_ids = self.fidelity_source_ids(&unit)?;
        let session_events = self.read_session_events_with_archived(&unit.source_session_id)?;
        self.build_evidence_pack_from(&unit, &source_ids, &session_events)
    }

    fn fidelity_source_ids(&self, unit: &MemoryUnit) -> Result<BTreeSet<String>> {
        let ids = if unit.source_event_ids.is_empty() {
            self.storage
                .read_archive_entry_by_id(&unit.archive_id)?
                .source_event_ids
        } else {
            unit.source_event_ids.clone()
        };
        Ok(ids.into_iter().collect())
    }

    fn build_evidence_pack_from(
        &self,
        unit: &MemoryUnit,
        source_ids: &BTreeSet<String>,
        session_events: &[StoredEvent],
    ) -> Result<EvidencePack> {
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

        let source_indices = session_events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| source_ids.contains(&event.event_id).then_some(index))
            .collect::<Vec<_>>();
        if source_indices.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "memory unit {} has no resolvable source events for fidelity review",
                unit.memory_unit_id
            )));
        }

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

    fn fidelity_basis_matches(&self, task: &PendingTask, unit: &MemoryUnit) -> Result<bool> {
        let Some((frozen, pack)) = frozen_fidelity_inputs(task) else {
            return Ok(false);
        };
        let source_ids = self.fidelity_source_ids(unit)?;
        if pack.target_thesis != unit.thesis
            || pack.unit_evidence != unit.evidence
            || fidelity_basis_digest(unit, &source_ids, &pack.events)? != frozen.digest
        {
            return Ok(false);
        }
        let events = self.read_session_events_with_archived(&unit.source_session_id)?;
        let events_by_id = events
            .iter()
            .map(|event| (event.event_id.as_str(), event))
            .collect::<HashMap<_, _>>();
        Ok(pack.events.iter().all(|seen| {
            events_by_id
                .get(seen.event_id.as_str())
                .is_some_and(|event| {
                    evidence_event_from_stored(event, seen.role, frozen.max_event_text_chars)
                        == *seen
                })
        }))
    }

    pub(super) fn mark_memory_fidelity_task_failed_best_effort(
        &self,
        task_id: &str,
        detail: String,
    ) {
        let _ = self.with_fidelity_task(task_id, |mut task| {
            if matches!(task.state, TaskState::Completed | TaskState::Cancelled) {
                return Ok(());
            }
            task.state = TaskState::Failed;
            task.updated_at = now_rfc3339()?;
            task.last_error = Some(detail);
            self.storage.save_task(&task)
        });
    }
}

pub(super) fn fidelity_task_has_basis(task: &PendingTask) -> bool {
    frozen_fidelity_inputs(task).is_some()
}

fn frozen_fidelity_inputs(task: &PendingTask) -> Option<(FrozenFidelityBasis, EvidencePack)> {
    if task.schema_version != PENDING_TASK_SCHEMA_VERSION {
        return None;
    }
    let frozen: FrozenFidelityBasis =
        serde_json::from_value(task.inputs.get("fidelity_basis")?.clone()).ok()?;
    let digest = frozen.digest.strip_prefix("sha256:")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let pack: EvidencePack = serde_json::from_value(task.inputs.clone()).ok()?;
    pack.events
        .iter()
        .any(|event| event.role == EvidenceEventRole::Source)
        .then_some((frozen, pack))
}

fn fidelity_basis_digest(
    unit: &MemoryUnit,
    source_ids: &BTreeSet<String>,
    events: &[EvidenceEvent],
) -> Result<String> {
    serializable_digest(&(
        "fidelity_basis.v1",
        &unit.memory_unit_id,
        &unit.archive_id,
        &unit.source_session_id,
        unit.effective_personality_scope_id(),
        &unit.thesis,
        &unit.subject,
        &unit.evidence,
        source_ids,
        events,
    ))
}

fn fidelity_task_unit_id(task: &PendingTask) -> Result<&str> {
    if task.task_type != TaskType::MemoryFidelityPass {
        return Err(MemoryEngineError::Validation(format!(
            "task is not memory_fidelity_pass: {}",
            task.task_id
        )));
    }
    task.inputs["memory_unit_id"].as_str().ok_or_else(|| {
        MemoryEngineError::Validation(format!(
            "memory_fidelity_pass task has no memory_unit_id: {}",
            task.task_id
        ))
    })
}
