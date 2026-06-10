use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn begin_reflection_analysis(
        &self,
        session_id: &str,
        core_scope: Option<String>,
    ) -> Result<ReflectionPassStart> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "reflection session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
            let now = now_rfc3339()?;
            let inputs = self.build_reflection_inputs_unlocked(session_id, core_scope.clone())?;
            let memory_unit_count = inputs
                .get("memory_units")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let core_fact_count = inputs
                .get("core_facts")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let task = PendingTask {
                schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
                task_id: new_id("task")?,
                task_type: TaskType::ReflectionAnalyze,
                state: TaskState::Pending,
                created_at: now.clone(),
                updated_at: now,
                prompt_id: "reflection_analyze".to_string(),
                prompt_version: 1,
                role_hint: ModelRole::Reasoning,
                expected_output_schema: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                inputs: inputs.clone(),
                attempts: Vec::new(),
                last_error: None,
            };
            self.storage.save_task(&task)?;
            let request = llm_request_from_task(
                &task,
                "reflection_analyze",
                json!({ "reflection_task": inputs }),
            )?;
            Ok(ReflectionPassStart {
                pending_task: task,
                request,
                source_session_id: session_id.to_string(),
                core_scope,
                memory_unit_count,
                core_fact_count,
            })
        })
    }

    pub fn submit_reflection_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<ReflectionCandidatesResult> {
        let request_id = llm_response_request_id(&response).to_string();
        match response {
            LlmResponse::Ok { text, .. } => {
                let result = (|| {
                    let value = parse_json_value_from_llm_text(&text)?;
                    let mut reflection: ReflectionAnalyzeResult = serde_json::from_value(value)?;
                    if reflection.schema_version.trim().is_empty() {
                        reflection.schema_version = REFLECTION_RESULT_SCHEMA_VERSION.to_string();
                    }
                    self.resume_reflection_analysis(task_id, reflection)
                })();
                if let Err(err) = &result {
                    self.mark_reflection_task_failed_best_effort(
                        task_id,
                        format!("{request_id} semantic error: {err}"),
                    );
                }
                result
            }
            LlmResponse::Err { kind, detail, .. } => {
                self.mark_reflection_task_failed_best_effort(
                    task_id,
                    format!("{request_id} {kind:?}: {detail}"),
                );
                Err(MemoryEngineError::Validation(format!(
                    "reflection_analyze failed: {kind:?}: {detail}"
                )))
            }
        }
    }

    pub fn resume_reflection_analysis(
        &self,
        task_id: &str,
        mut result: ReflectionAnalyzeResult,
    ) -> Result<ReflectionCandidatesResult> {
        if result.schema_version != REFLECTION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        if result.source_session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "reflection result source_session_id must not be empty".to_string(),
            ));
        }
        result.core_scope = normalize_optional_string(result.core_scope.as_deref());
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::ReflectionAnalyze {
            return Err(MemoryEngineError::Validation(format!(
                "task is not reflection_analyze: {task_id}"
            )));
        }
        let task_session_id = task
            .inputs
            .get("source_session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "reflection_analyze task has no source_session_id: {task_id}"
                ))
            })?;
        if task_session_id != result.source_session_id {
            return Err(MemoryEngineError::Validation(format!(
                "reflection source_session_id mismatch: task={task_session_id} result={}",
                result.source_session_id
            )));
        }
        let task_scope =
            normalize_optional_string(task.inputs.get("core_scope").and_then(Value::as_str));
        if task_scope != result.core_scope {
            result.core_scope = task_scope.clone();
        }

        self.with_resource_lock(session_lock_key(&result.source_session_id), || {
            let now = now_rfc3339()?;
            let known_units = self
                .reflection_memory_units_unlocked(&result.source_session_id)?
                .into_iter()
                .map(|unit| unit.memory_unit_id)
                .collect::<HashSet<_>>();
            let known_core_facts = task
                .inputs
                .get("core_facts")
                .and_then(Value::as_array)
                .map(|facts| {
                    facts
                        .iter()
                        .filter_map(|fact| fact.get("core_fact_id").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            let mut candidates = Vec::new();
            for draft in result.candidates {
                if let Some(candidate) = self.candidate_from_reflection_draft(
                    &result.source_session_id,
                    task_scope.clone(),
                    draft,
                    &known_units,
                    &known_core_facts,
                    &now,
                )? {
                    self.storage.write_candidate_belief(&candidate)?;
                    candidates.push(candidate);
                }
            }

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now.clone();
            task.last_error = None;
            self.storage.save_task(&task)?;

            Ok(ReflectionCandidatesResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: result.source_session_id,
                core_scope: task_scope,
                created_at: now,
                candidates,
            })
        })
    }

    pub fn list_candidates(&self) -> Result<Vec<CandidateBelief>> {
        self.ensure_manifest()?;
        Ok(self.storage.read_candidate_beliefs()?.into_items())
    }

    pub fn review_candidate(&self, input: CandidateReviewInput) -> Result<CandidateReviewResult> {
        validate_candidate_review_input(&input)?;
        self.ensure_manifest()?;

        self.with_resource_lock(candidate_lock_key(&input.candidate_id), || {
            let now = now_rfc3339()?;
            let mut candidate = self.storage.read_candidate_belief(&input.candidate_id)?;
            let review = ReviewRecord {
                reviewed_by: normalize_whitespace(&input.reviewed_by),
                reviewed_at: now.clone(),
                decision: input.decision,
                note: normalize_optional_string(input.note.as_deref()),
            };

            let mut promoted_fact = None;
            let mut contested_facts = Vec::new();
            match input.decision {
                ReviewDecision::Approved => {
                    let scope = normalize_optional_string(input.core_scope.as_deref())
                        .or_else(|| candidate.core_scope.clone());
                    let category = normalize_whitespace(&candidate.category);
                    let tags = {
                        let mut tags = candidate.tags.clone();
                        tags.push("reflection_candidate".to_string());
                        tags.push("manual_review".to_string());
                        unique_strings(tags)
                    };
                    let source_archive_ids =
                        unique_strings(candidate.supporting_archive_ids.clone());
                    contested_facts =
                        self.contest_candidate_core_conflicts(&candidate, scope.clone(), &now)?;
                    let upsert = self.with_resource_lock(core_lock_key(&category), || {
                        self.upsert_core_fact_unlocked(
                            CoreFactInput {
                                schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                                category: category.clone(),
                                scope,
                                text: candidate.text.clone(),
                                confidence: candidate.confidence,
                                tags,
                                source_archive_ids,
                                source_candidate_id: Some(candidate.candidate_id.clone()),
                            },
                            category.clone(),
                        )
                    })?;
                    candidate.status = CandidateStatus::Promoted;
                    candidate.promoted_core_fact_id = Some(upsert.fact.core_fact_id.clone());
                    promoted_fact = Some(upsert.fact);
                }
                ReviewDecision::Rejected => {
                    candidate.status = CandidateStatus::Rejected;
                }
                ReviewDecision::NeedsChanges => {
                    candidate.status = CandidateStatus::Draft;
                }
            }

            candidate.updated_at = now;
            candidate.review = Some(review);
            self.storage.write_candidate_belief(&candidate)?;
            Ok(CandidateReviewResult {
                schema_version: CANDIDATE_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                candidate,
                promoted_fact,
                contested_facts,
            })
        })
    }

    pub(super) fn contest_candidate_core_conflicts(
        &self,
        candidate: &CandidateBelief,
        scope: Option<String>,
        now: &str,
    ) -> Result<Vec<CoreFact>> {
        let target_ids = candidate
            .contradicted_core_fact_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }

        let candidate_id = candidate.candidate_id.clone();
        let category_names = self
            .storage
            .read_core_store_categories()?
            .into_items()
            .into_iter()
            .filter(|category| {
                category.facts.iter().any(|fact| {
                    target_ids.contains(&fact.core_fact_id)
                        && fact.status == CoreFactStatus::Active
                        && fact.scope == scope
                })
            })
            .map(|category| category.category)
            .collect::<BTreeSet<_>>();

        let mut contested = Vec::new();
        for category_name in category_names {
            let contested_for_category =
                self.with_resource_lock(core_lock_key(&category_name), || {
                    let mut category = self.storage.read_core_store_category(&category_name)?;
                    let mut changed = false;
                    let mut contested_for_category = Vec::new();
                    for fact in &mut category.facts {
                        if !target_ids.contains(&fact.core_fact_id)
                            || fact.status != CoreFactStatus::Active
                            || fact.scope != scope
                        {
                            continue;
                        }
                        fact.status = CoreFactStatus::Contested;
                        fact.updated_at = now.to_string();
                        merge_unique(
                            &mut fact.tags,
                            &[
                                "contested".to_string(),
                                "contested_by_reflection_candidate".to_string(),
                            ],
                        );
                        push_link_once(
                            &mut fact.links,
                            Link {
                                kind: "contested_by_candidate".to_string(),
                                target: candidate_id.clone(),
                                note: Some(candidate.text.clone()),
                            },
                        );
                        changed = true;
                        contested_for_category.push(fact.clone());
                    }
                    if changed {
                        category.updated_at = now.to_string();
                        self.storage.write_core_store_category(&category)?;
                    }
                    Ok(contested_for_category)
                })?;
            contested.extend(contested_for_category);
        }

        Ok(contested)
    }

    pub(super) fn build_reflection_inputs_unlocked(
        &self,
        session_id: &str,
        core_scope: Option<String>,
    ) -> Result<Value> {
        let core_scope = normalize_optional_string(core_scope.as_deref());
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_items()
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .filter(|entry| entry.status == ArchiveStatus::Complete)
            .collect::<Vec<_>>();

        let mut memory_units = Vec::new();
        for unit in self.reflection_memory_units_unlocked(session_id)? {
            memory_units.push(json!({
                "memory_unit_id": unit.memory_unit_id,
                "archive_id": unit.archive_id,
                "thesis": unit.thesis,
                "evidence": unit.evidence,
                "tags": unit.tags,
                "weight": unit.weight,
                "fidelity_status": unit.fidelity_status,
            }));
        }

        let archive_summaries = archives
            .iter()
            .map(|archive| {
                json!({
                    "archive_id": archive.archive_id,
                    "created_at": archive.created_at,
                    "gist": archive.gist,
                    "compact_memory": archive.compact_memory,
                    "tags": archive.tags,
                    "weight": archive.weight,
                    "recall_count": archive.recall_count,
                })
            })
            .collect::<Vec<_>>();

        let core_facts = self
            .storage
            .read_core_store_categories()?
            .into_items()
            .into_iter()
            .flat_map(|category| {
                let category_name = category.category.clone();
                let scope_filter = core_scope.clone();
                category.facts.into_iter().filter_map(move |fact| {
                    if !core_fact_visible_in_context(fact.status) {
                        return None;
                    }
                    if scope_filter.is_some() && fact.scope != scope_filter {
                        return None;
                    }
                    Some(json!({
                        "category": category_name,
                        "core_fact_id": fact.core_fact_id,
                        "text": fact.text,
                        "status": fact.status,
                        "confidence": fact.confidence,
                        "tags": fact.tags,
                    }))
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "schema_version": "reflection_task.v1",
            "source_session_id": session_id,
            "core_scope": core_scope,
            "memory_units": memory_units,
            "archives": archive_summaries,
            "core_facts": core_facts,
            "rules": {
                "agents_do_not_write_core": true,
                "first_iteration_requires_manual_review": true,
                "use_only_validated_memory_units": true
            }
        }))
    }

    pub(super) fn reflection_memory_units_unlocked(
        &self,
        session_id: &str,
    ) -> Result<Vec<MemoryUnit>> {
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_items()
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .filter(|entry| entry.status == ArchiveStatus::Complete)
            .collect::<Vec<_>>();
        let mut units = Vec::new();
        for archive in archives {
            for unit in self
                .storage
                .read_memory_units_for_archive(&archive.archive_id)?
                .into_items()
            {
                if unit.source_session_id != session_id {
                    continue;
                }
                if unit.status != MemoryUnitStatus::ActiveArchive {
                    continue;
                }
                if unit.fidelity_status != FidelityStatus::Valid {
                    continue;
                }
                units.push(unit);
            }
        }
        units.sort_by(|left, right| {
            right
                .weight
                .total_cmp(&left.weight)
                .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
        });
        Ok(units)
    }

    pub(super) fn candidate_from_reflection_draft(
        &self,
        source_session_id: &str,
        core_scope: Option<String>,
        draft: ReflectionCandidateDraft,
        known_units: &HashSet<String>,
        known_core_facts: &HashSet<String>,
        now: &str,
    ) -> Result<Option<CandidateBelief>> {
        let text = normalize_whitespace(&draft.text);
        let category = normalize_category_name(&draft.category);
        let evidence_summary = normalize_whitespace(&draft.evidence_summary);
        if text.is_empty() || category.is_empty() || evidence_summary.is_empty() {
            return Ok(None);
        }

        let source_memory_unit_ids = unique_strings(
            draft
                .source_memory_unit_ids
                .into_iter()
                .filter(|unit_id| known_units.contains(unit_id))
                .collect(),
        );
        if source_memory_unit_ids.is_empty() {
            return Ok(None);
        }

        let supporting_archive_ids = unique_strings(draft.supporting_archive_ids);
        let contradicting_archive_ids = unique_strings(draft.contradicting_archive_ids);
        let contradicted_core_fact_ids = unique_strings(
            draft
                .contradicted_core_fact_ids
                .into_iter()
                .filter(|fact_id| known_core_facts.contains(fact_id))
                .collect(),
        );
        let confidence = draft.confidence.clamp(0.0, 1.0);
        Ok(Some(CandidateBelief {
            schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            candidate_id: new_id("candidate")?,
            source_session_id: Some(source_session_id.to_string()),
            core_scope,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            text,
            category,
            status: CandidateStatus::ReadyForReview,
            confidence,
            supporting_archive_ids,
            contradicting_archive_ids: contradicting_archive_ids.clone(),
            contradicted_core_fact_ids: contradicted_core_fact_ids.clone(),
            evidence_summary,
            promotion_checks: PromotionChecks {
                min_sources_met: true,
                weight_threshold_met: confidence >= 0.70,
                no_recent_contradiction: contradicting_archive_ids.is_empty()
                    && contradicted_core_fact_ids.is_empty(),
                manual_review_required: true,
            },
            source_memory_unit_ids,
            tags: unique_strings(draft.tags),
            promoted_core_fact_id: None,
            review: None,
            links: Vec::new(),
        }))
    }

    pub(super) fn mark_reflection_task_failed_best_effort(&self, task_id: &str, detail: String) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::ReflectionAnalyze {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }
}
