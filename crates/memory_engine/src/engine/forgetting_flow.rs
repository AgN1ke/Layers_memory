use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn begin_forget_review(&self, session_id: &str) -> Result<ForgetReviewStart> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "forget review session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
            let now = now_rfc3339()?;
            let inputs = self.build_forget_review_inputs_unlocked(session_id, &now)?;
            let task = PendingTask {
                schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
                task_id: new_id("task")?,
                task_type: TaskType::ForgetReview,
                state: if inputs.candidates.is_empty() {
                    TaskState::Completed
                } else {
                    TaskState::Pending
                },
                created_at: now.clone(),
                updated_at: now,
                prompt_id: self.options.forgetting.prompt_id.clone(),
                prompt_version: self.options.forgetting.prompt_version,
                role_hint: ModelRole::Balanced,
                expected_output_schema: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                inputs: serde_json::to_value(&inputs)?,
                attempts: Vec::new(),
                last_error: None,
            };
            self.storage.save_task(&task)?;
            let request = llm_request_from_task(
                &task,
                &self.options.forgetting.prompt_id,
                json!({ "forget_review": inputs.clone() }),
            )?;
            Ok(ForgetReviewStart {
                source_session_id: session_id.to_string(),
                candidate_count: inputs.candidates.len(),
                candidates: inputs.candidates,
                pending_task: task,
                request,
            })
        })
    }

    pub fn submit_forget_review_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<ForgetReviewApplyResult> {
        let request_id = llm_response_request_id(&response).to_string();
        match response {
            LlmResponse::Ok { text, .. } => {
                let result = (|| {
                    let value = parse_json_value_from_llm_text(&text)?;
                    let mut review: ForgetReviewResult = serde_json::from_value(value)?;
                    review.normalize_schema();
                    self.resume_forget_review(task_id, review)
                })();
                if let Err(err) = &result {
                    self.mark_forget_review_task_failed_best_effort(
                        task_id,
                        format!("{request_id} semantic error: {err}"),
                    );
                }
                result
            }
            LlmResponse::Err { kind, detail, .. } => {
                self.mark_forget_review_task_failed_best_effort(
                    task_id,
                    format!("{request_id} {kind:?}: {detail}"),
                );
                Err(MemoryEngineError::Validation(format!(
                    "forget_review_pass failed: {kind:?}: {detail}"
                )))
            }
        }
    }

    pub fn resume_forget_review(
        &self,
        task_id: &str,
        mut result: ForgetReviewResult,
    ) -> Result<ForgetReviewApplyResult> {
        result.normalize_schema();
        if result.schema_version != FORGET_REVIEW_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        if result.source_session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "forget review result source_session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::ForgetReview {
            return Err(MemoryEngineError::Validation(format!(
                "task is not forget_review: {task_id}"
            )));
        }
        let inputs: ForgetReviewInputs = serde_json::from_value(task.inputs.clone())?;
        if inputs.source_session_id != result.source_session_id {
            return Err(MemoryEngineError::Validation(format!(
                "forget review source_session_id mismatch: task={} result={}",
                inputs.source_session_id, result.source_session_id
            )));
        }

        self.with_resource_lock(session_lock_key(&inputs.source_session_id), || {
            let now = now_rfc3339()?;
            let candidate_ids = inputs
                .candidates
                .iter()
                .map(|candidate| candidate.memory_unit_id.clone())
                .collect::<HashSet<_>>();
            let mut reviewed = 0;
            let mut forgotten = 0;
            let mut kept = 0;
            let mut protected = 0;
            let mut ignored = 0;
            let mut changed_units = Vec::new();

            for recommendation in &mut result.recommendations {
                recommendation.reason = normalize_whitespace(&recommendation.reason);
                if recommendation.reason.is_empty() {
                    recommendation.reason = "No reason provided.".to_string();
                }
                reviewed += 1;
                if !candidate_ids.contains(&recommendation.memory_unit_id) {
                    ignored += 1;
                    continue;
                }

                match self.apply_forget_recommendation_unlocked(task_id, recommendation, &now)? {
                    ForgetApplyAction::Forgotten(unit) => {
                        forgotten += 1;
                        changed_units.push(unit);
                    }
                    ForgetApplyAction::Kept(unit) => {
                        kept += 1;
                        changed_units.push(unit);
                    }
                    ForgetApplyAction::Protected(unit) => {
                        protected += 1;
                        changed_units.push(unit);
                    }
                    ForgetApplyAction::Ignored => {
                        ignored += 1;
                    }
                }
            }

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            self.storage.save_task(&task)?;

            Ok(ForgetReviewApplyResult {
                schema_version: FORGET_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: inputs.source_session_id,
                reviewed,
                forgotten,
                kept,
                protected,
                ignored,
                changed_units,
            })
        })
    }

    pub fn list_forgotten_memory_units(&self, session_id: &str) -> Result<ForgottenMemoryUnits> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;
        self.with_resource_lock(session_lock_key(session_id), || {
            let mut units = self
                .storage
                .read_archive(&ArchiveFilters::default())?
                .into_items()
                .into_iter()
                .filter(|archive| archive.source_session_id == session_id)
                .flat_map(|archive| archive.memory_units.into_iter())
                .filter(|unit| unit.status == MemoryUnitStatus::Forgotten)
                .collect::<Vec<_>>();
            units.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
            });
            Ok(ForgottenMemoryUnits {
                schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
                source_session_id: session_id.to_string(),
                units,
            })
        })
    }

    pub fn remember_back(&self, memory_unit_id: &str) -> Result<MemoryUnit> {
        if memory_unit_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory_unit_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;
        let unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
        let session_id = unit.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            let now = now_rfc3339()?;
            let mut unit = self.storage.read_memory_unit_by_id(memory_unit_id)?;
            if unit.status != MemoryUnitStatus::Forgotten {
                return Err(MemoryEngineError::Validation(format!(
                    "memory unit is not forgotten: {memory_unit_id}"
                )));
            }
            unit.status = MemoryUnitStatus::ActiveArchive;
            unit.updated_at = now.clone();
            unit.tags.retain(|tag| tag != "forgotten");
            merge_unique(&mut unit.tags, &["remembered_back".to_string()]);
            self.storage.write_memory_unit(&unit)?;
            self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, &now)?;
            Ok(unit)
        })
    }

    pub(super) fn build_forget_review_inputs_unlocked(
        &self,
        session_id: &str,
        reference_at: &str,
    ) -> Result<ForgetReviewInputs> {
        let mut candidates = Vec::new();
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_items()
            .into_iter()
            .filter(|archive| {
                archive.source_session_id == session_id && archive.status == ArchiveStatus::Complete
            })
            .collect::<Vec<_>>();

        for archive in archives {
            let age_days = archive_age_days(&archive, reference_at).unwrap_or(0.0);
            if age_days < self.options.forgetting.min_age_days.max(0.0) {
                continue;
            }
            if archive.recall_count > self.options.forgetting.forget_recall_count_max {
                continue;
            }

            for unit in &archive.memory_units {
                if unit.status != MemoryUnitStatus::ActiveArchive {
                    continue;
                }
                if unit.weight >= self.options.forgetting.forget_weight_threshold {
                    continue;
                }
                let protection =
                    self.forget_protection_reasons_unlocked(unit, &archive, reference_at)?;
                if !protection.is_empty() {
                    continue;
                }

                candidates.push(ForgetReviewCandidate {
                    label: format!("m{}", candidates.len() + 1),
                    memory_unit_id: unit.memory_unit_id.clone(),
                    archive_id: archive.archive_id.clone(),
                    age_days,
                    weight: unit.weight,
                    archive_recall_count: archive.recall_count,
                    archive_last_recalled_days: archive.last_recalled_at.as_deref().and_then(
                        |last_recalled_at| timestamp_age_days(last_recalled_at, reference_at),
                    ),
                    fidelity_status: unit.fidelity_status,
                    has_core_link: false,
                    has_emotional: false,
                    thesis: truncate_chars(&unit.thesis, 240),
                });
            }
        }

        candidates.sort_by(|left, right| {
            right
                .age_days
                .total_cmp(&left.age_days)
                .then_with(|| left.weight.total_cmp(&right.weight))
                .then_with(|| left.archive_recall_count.cmp(&right.archive_recall_count))
                .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
        });
        candidates.truncate(self.options.forgetting.max_review_batch);
        for (index, candidate) in candidates.iter_mut().enumerate() {
            candidate.label = format!("m{}", index + 1);
        }

        Ok(ForgetReviewInputs {
            schema_version: FORGET_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            source_session_id: session_id.to_string(),
            created_at: reference_at.to_string(),
            candidates,
        })
    }

    pub(super) fn apply_forget_recommendation_unlocked(
        &self,
        task_id: &str,
        recommendation: &ForgetRecommendation,
        now: &str,
    ) -> Result<ForgetApplyAction> {
        let mut unit = match self
            .storage
            .read_memory_unit_by_id(&recommendation.memory_unit_id)
        {
            Ok(unit) => unit,
            Err(_) => return Ok(ForgetApplyAction::Ignored),
        };
        if unit.status != MemoryUnitStatus::ActiveArchive {
            return Ok(ForgetApplyAction::Ignored);
        }
        let archive = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
        let protection = self.forget_protection_reasons_unlocked(&unit, &archive, now)?;
        if !protection.is_empty() {
            unit.updated_at = now.to_string();
            unit.forget_review = Some(ForgetReviewRecord {
                reviewed_at: now.to_string(),
                task_id: Some(task_id.to_string()),
                decision: ForgetDecision::Protect,
                reason: format!(
                    "{} Protected by engine gate: {}",
                    recommendation.reason,
                    protection.join(", ")
                ),
            });
            merge_unique(
                &mut unit.tags,
                &[
                    "forget_reviewed".to_string(),
                    "forget_protected".to_string(),
                ],
            );
            self.storage.write_memory_unit(&unit)?;
            self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
            return Ok(ForgetApplyAction::Protected(unit));
        }

        match recommendation.decision {
            ForgetDecision::Forget => {
                unit.status = MemoryUnitStatus::Forgotten;
                unit.updated_at = now.to_string();
                unit.forget_review = Some(ForgetReviewRecord {
                    reviewed_at: now.to_string(),
                    task_id: Some(task_id.to_string()),
                    decision: ForgetDecision::Forget,
                    reason: recommendation.reason.clone(),
                });
                merge_unique(
                    &mut unit.tags,
                    &["forget_reviewed".to_string(), "forgotten".to_string()],
                );
                self.storage.write_memory_unit(&unit)?;
                self.tombstone_vector_unit_if_indexed_unlocked(&unit)?;
                self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
                Ok(ForgetApplyAction::Forgotten(unit))
            }
            ForgetDecision::Keep => {
                unit.updated_at = now.to_string();
                unit.forget_review = Some(ForgetReviewRecord {
                    reviewed_at: now.to_string(),
                    task_id: Some(task_id.to_string()),
                    decision: ForgetDecision::Keep,
                    reason: recommendation.reason.clone(),
                });
                merge_unique(&mut unit.tags, &["forget_reviewed".to_string()]);
                self.storage.write_memory_unit(&unit)?;
                self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
                Ok(ForgetApplyAction::Kept(unit))
            }
            ForgetDecision::Protect => {
                unit.updated_at = now.to_string();
                unit.forget_review = Some(ForgetReviewRecord {
                    reviewed_at: now.to_string(),
                    task_id: Some(task_id.to_string()),
                    decision: ForgetDecision::Protect,
                    reason: recommendation.reason.clone(),
                });
                merge_unique(
                    &mut unit.tags,
                    &[
                        "forget_reviewed".to_string(),
                        "forget_protected".to_string(),
                    ],
                );
                self.storage.write_memory_unit(&unit)?;
                self.rebuild_archive_units_and_compact_unlocked(&unit.archive_id, &unit, now)?;
                Ok(ForgetApplyAction::Protected(unit))
            }
        }
    }

    pub(super) fn rebuild_archive_units_and_compact_unlocked(
        &self,
        archive_id: &str,
        updated_unit: &MemoryUnit,
        now: &str,
    ) -> Result<()> {
        let mut archive = self.storage.read_archive_entry_by_id(archive_id)?;
        let mut replaced = false;
        for unit in &mut archive.memory_units {
            if unit.memory_unit_id == updated_unit.memory_unit_id {
                *unit = updated_unit.clone();
                replaced = true;
                break;
            }
        }
        if !replaced {
            archive.memory_units.push(updated_unit.clone());
        }
        archive.updated_at = now.to_string();
        archive.compact_memory = render_compact_memory_from_units(&archive.memory_units);
        self.storage
            .update_archive_entry(&archive.archive_id, &archive)
    }

    pub(super) fn forget_protection_reasons_unlocked(
        &self,
        unit: &MemoryUnit,
        archive: &ArchiveEntry,
        reference_at: &str,
    ) -> Result<Vec<String>> {
        let mut reasons = Vec::new();
        if unit.weight >= self.options.forgetting.protect_weight {
            reasons.push("high_weight".to_string());
        }
        if archive
            .last_recalled_at
            .as_deref()
            .and_then(|last_recalled_at| timestamp_age_days(last_recalled_at, reference_at))
            .is_some_and(|age_days| age_days <= self.options.forgetting.protect_recall_window_days)
        {
            reasons.push("recent_recall".to_string());
        }
        if self.unit_has_core_link_unlocked(unit)? {
            reasons.push("core_link".to_string());
        }
        if self.unit_has_emotional_marker(unit, archive) {
            reasons.push("emotional".to_string());
        }
        Ok(reasons)
    }

    pub(super) fn unit_has_core_link_unlocked(&self, unit: &MemoryUnit) -> Result<bool> {
        for category in self.storage.read_core_store_categories()?.into_items() {
            for fact in category.facts {
                if !core_fact_visible_in_context(fact.status) {
                    continue;
                }
                if fact
                    .source_archive_ids
                    .iter()
                    .any(|id| id == &unit.archive_id)
                {
                    return Ok(true);
                }
                if let Some(candidate_id) = fact.source_candidate_id {
                    if self
                        .storage
                        .read_candidate_belief(&candidate_id)
                        .ok()
                        .is_some_and(|candidate| {
                            candidate
                                .source_memory_unit_ids
                                .iter()
                                .any(|id| id == &unit.memory_unit_id)
                        })
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(self
            .storage
            .read_candidate_beliefs()?
            .into_items()
            .into_iter()
            .filter(|candidate| candidate.status == CandidateStatus::Promoted)
            .any(|candidate| {
                candidate
                    .source_memory_unit_ids
                    .iter()
                    .any(|id| id == &unit.memory_unit_id)
            }))
    }

    pub(super) fn unit_has_emotional_marker(
        &self,
        unit: &MemoryUnit,
        archive: &ArchiveEntry,
    ) -> bool {
        let source_ids = if unit.source_event_ids.is_empty() {
            archive.source_event_ids.iter().collect::<HashSet<_>>()
        } else {
            unit.source_event_ids.iter().collect::<HashSet<_>>()
        };
        archive.emotional_markers.iter().any(|marker| {
            marker.strength >= self.options.forgetting.protect_emotional_strength
                && (source_ids.is_empty()
                    || marker
                        .source_event_ids
                        .iter()
                        .any(|event_id| source_ids.contains(event_id)))
        })
    }

    pub(super) fn mark_forget_review_task_failed_best_effort(&self, task_id: &str, detail: String) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::ForgetReview {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }
}
