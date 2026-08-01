use super::*;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REFLECTION_RELATION_LINK_WEIGHT: usize = 4;
const REFLECTION_RELATION_THEME_WEIGHT: usize = 3;
const REFLECTION_RELATION_TOPIC_WEIGHT: usize = 2;
const REFLECTION_RELATION_TAG_WEIGHT: usize = 1;

#[derive(Debug, Clone)]
struct ReflectionUnitContext {
    unit: MemoryUnit,
    archive: ArchiveEntry,
}

#[derive(Debug, Clone)]
struct RelatedReflectionUnit {
    context: ReflectionUnitContext,
    relation_score: usize,
    relation_reasons: Vec<String>,
}

struct ReflectionKnownEvidence<'a> {
    unit_sessions: &'a HashMap<String, String>,
    archive_ids: &'a HashSet<String>,
    core_fact_ids: &'a HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
enum PreparedReflectionAction {
    NewCandidate {
        action_id: String,
        candidate: CandidateBelief,
    },
    ReinforcedCandidate {
        action_id: String,
        candidate: CandidateBelief,
        previous_updated_at: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReflectionApplyOperationIntent {
    schema_version: String,
    task_id: String,
    reflection_scope: String,
    source_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    core_scope: Option<String>,
    result_digest: String,
    strict_typed_actions: bool,
    prepared_actions: Vec<PreparedReflectionAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    no_insight: Option<ReflectionNoInsight>,
    outcome: ReflectionCandidatesResult,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    applied_action_ids: Vec<String>,
    #[serde(default)]
    checkpoint_completed: bool,
    #[serde(default)]
    task_completed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CoreCategoryEffect {
    category: CoreStoreCategory,
    previous_updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CandidateReviewOperationIntent {
    schema_version: String,
    candidate_id: String,
    previous_candidate_updated_at: String,
    input_digest: String,
    desired_candidate: CandidateBelief,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    core_effects: Vec<CoreCategoryEffect>,
    result: CandidateReviewResult,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    applied_core_categories: Vec<String>,
    #[serde(default)]
    candidate_applied: bool,
}

impl<S: Storage> MemoryEngine<S> {
    pub fn bind_reflection_session(
        &self,
        session_id: &str,
        core_scope: Option<String>,
    ) -> Result<ReflectionState> {
        validate_reflection_session_id(session_id)?;
        self.ensure_manifest()?;
        let core_scope = normalize_optional_string(core_scope.as_deref());
        let reflection_scope = reflection_scope_id(session_id, core_scope.as_deref());
        self.with_resource_lock(reflection_lock_key(&reflection_scope), || {
            self.ensure_reflection_membership_unlocked(session_id, &reflection_scope)
        })
    }

    pub fn reflection_status(
        &self,
        session_id: &str,
        core_scope: Option<String>,
        opportunity: ReflectionOpportunity,
    ) -> Result<ReflectionTriggerStatus> {
        validate_reflection_session_id(session_id)?;
        self.ensure_manifest()?;
        let reflection_scope = reflection_scope_id(session_id, core_scope.as_deref());

        self.with_resource_lock(reflection_lock_key(&reflection_scope), || {
            self.evaluate_reflection_trigger_unlocked(session_id, core_scope, opportunity)
        })
    }

    pub fn begin_reflection_if_due(
        &self,
        session_id: &str,
        core_scope: Option<String>,
        opportunity: ReflectionOpportunity,
    ) -> Result<AutomaticReflectionStart> {
        validate_reflection_session_id(session_id)?;
        self.ensure_manifest()?;
        let reflection_scope = reflection_scope_id(session_id, core_scope.as_deref());

        self.with_resource_lock(reflection_lock_key(&reflection_scope), || {
            self.ensure_reflection_membership_unlocked(session_id, &reflection_scope)?;
            let mut status = self.evaluate_reflection_trigger_unlocked(
                session_id,
                core_scope.clone(),
                opportunity,
            )?;
            if !status.due {
                return Ok(AutomaticReflectionStart {
                    status,
                    reflection: None,
                });
            }

            let focus_ids = self
                .reflection_memory_units_for_sessions_unlocked(&status.source_session_ids)?
                .into_iter()
                .filter(|unit| unit.reflection_count == 0)
                .take(self.options.reflection.max_trigger_units.max(1))
                .map(|unit| unit.memory_unit_id)
                .collect::<HashSet<_>>();
            let start = self.begin_reflection_analysis_unlocked(
                session_id,
                core_scope,
                Some(&focus_ids),
                Some(status.reason),
            )?;
            status.pending_task_id = Some(start.pending_task.task_id.clone());

            Ok(AutomaticReflectionStart {
                status,
                reflection: Some(start),
            })
        })
    }

    pub fn begin_reflection_analysis(
        &self,
        session_id: &str,
        core_scope: Option<String>,
    ) -> Result<ReflectionPassStart> {
        validate_reflection_session_id(session_id)?;
        self.ensure_manifest()?;
        let reflection_scope = reflection_scope_id(session_id, core_scope.as_deref());

        self.with_resource_lock(reflection_lock_key(&reflection_scope), || {
            self.ensure_reflection_membership_unlocked(session_id, &reflection_scope)?;
            self.begin_reflection_analysis_unlocked(session_id, core_scope, None, None)
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
                    let schema_version = value
                        .get("schema_version")
                        .and_then(Value::as_str)
                        .unwrap_or(REFLECTION_RESULT_SCHEMA_VERSION);
                    if schema_version == REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION {
                        let reflection: ReflectionActionsResult = serde_json::from_value(value)?;
                        self.resume_reflection_actions(task_id, reflection)
                    } else {
                        let mut reflection: ReflectionAnalyzeResult =
                            serde_json::from_value(value)?;
                        if reflection.schema_version.trim().is_empty() {
                            reflection.schema_version =
                                REFLECTION_RESULT_SCHEMA_VERSION.to_string();
                        }
                        self.resume_reflection_analysis(task_id, reflection)
                    }
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
        let actions = result
            .candidates
            .into_iter()
            .map(|candidate| ReflectionActionDraft::NewCandidate { candidate })
            .collect();
        self.apply_reflection_actions(
            task_id,
            result.source_session_id,
            result.core_scope,
            actions,
            None,
            false,
        )
    }

    pub fn resume_reflection_actions(
        &self,
        task_id: &str,
        mut result: ReflectionActionsResult,
    ) -> Result<ReflectionCandidatesResult> {
        if result.schema_version != REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        if result.source_session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "reflection result source_session_id must not be empty".to_string(),
            ));
        }
        let has_actions = !result.actions.is_empty();
        let has_no_insight = result.no_insight.is_some();
        if has_actions == has_no_insight {
            return Err(MemoryEngineError::Validation(
                "reflection actions result must contain exactly one of actions or no_insight"
                    .to_string(),
            ));
        }
        if let Some(no_insight) = &mut result.no_insight {
            no_insight.explanation = normalize_whitespace(&no_insight.explanation);
            if no_insight.explanation.is_empty() {
                return Err(MemoryEngineError::Validation(
                    "reflection no_insight explanation must not be empty".to_string(),
                ));
            }
        }
        result.core_scope = normalize_optional_string(result.core_scope.as_deref());
        self.apply_reflection_actions(
            task_id,
            result.source_session_id,
            result.core_scope,
            result.actions,
            result.no_insight,
            true,
        )
    }

    fn apply_reflection_actions(
        &self,
        task_id: &str,
        source_session_id: String,
        mut core_scope: Option<String>,
        actions: Vec<ReflectionActionDraft>,
        no_insight: Option<ReflectionNoInsight>,
        strict_typed_actions: bool,
    ) -> Result<ReflectionCandidatesResult> {
        self.ensure_manifest()?;

        let result_digest = reflection_apply_result_digest(
            &source_session_id,
            &core_scope,
            &actions,
            &no_insight,
            strict_typed_actions,
        )?;
        let operation_id = format!("reflection_apply_{task_id}");

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
        if task_session_id != source_session_id {
            return Err(MemoryEngineError::Validation(format!(
                "reflection source_session_id mismatch: task={task_session_id} result={}",
                source_session_id
            )));
        }
        let task_scope =
            normalize_optional_string(task.inputs.get("core_scope").and_then(Value::as_str));
        if core_scope.is_some() && task_scope != core_scope {
            return Err(MemoryEngineError::Validation(format!(
                "reflection core_scope mismatch: task={task_scope:?} result={core_scope:?}"
            )));
        }
        core_scope = task_scope.clone();

        let reflection_scope = task
            .inputs
            .get("reflection_scope")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| reflection_scope_id(&source_session_id, task_scope.as_deref()));
        self.with_resource_lock(reflection_lock_key(&reflection_scope), || {
            if let Some(operation) = self.storage.load_journaled_operation(&operation_id)? {
                if operation.op_type != JournalOperationType::ReflectionApply {
                    return Err(MemoryEngineError::Validation(format!(
                        "journal operation type mismatch for {operation_id}"
                    )));
                }
                let intent: ReflectionApplyOperationIntent =
                    serde_json::from_value(operation.intent.clone())?;
                if intent.result_digest != result_digest {
                    return Err(MemoryEngineError::Validation(format!(
                        "reflection task already has a different durable result: {task_id}"
                    )));
                }
                return self.execute_reflection_apply_operation_unlocked(operation);
            }
            if !matches!(task.state, TaskState::Pending | TaskState::Submitted) {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection task is not resumable in state {:?}: {task_id}",
                    task.state
                )));
            }

            let now = now_rfc3339()?;
            let task_memory_units = task
                .inputs
                .get("memory_units")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let known_unit_sessions = task_memory_units
                .iter()
                .filter_map(|unit| {
                    Some((
                        unit.get("memory_unit_id")?.as_str()?.to_string(),
                        unit.get("source_session_id")?.as_str()?.to_string(),
                    ))
                })
                .collect::<HashMap<_, _>>();
            let known_archive_ids = task
                .inputs
                .get("archives")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|archive| archive.get("archive_id").and_then(Value::as_str))
                .map(str::to_string)
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
            let known_evidence = ReflectionKnownEvidence {
                unit_sessions: &known_unit_sessions,
                archive_ids: &known_archive_ids,
                core_fact_ids: &known_core_facts,
            };
            self.revalidate_reflection_task_evidence(
                &task,
                task_scope.as_deref(),
                &actions,
            )?;
            let known_candidate_ids = task
                .inputs
                .get("existing_candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|candidate| candidate.get("candidate_id").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<HashSet<_>>();
            let mut known_candidate_texts = task
                .inputs
                .get("existing_candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|candidate| candidate.get("text").and_then(Value::as_str))
                .map(normalize_whitespace)
                .collect::<HashSet<_>>();
            let mut reinforced_ids = HashSet::new();
            let mut prepared = Vec::new();
            for (action_index, action) in actions.into_iter().enumerate() {
                let action_id = format!("{operation_id}_action_{action_index}");
                match action {
                    ReflectionActionDraft::NewCandidate { candidate } => {
                        if strict_typed_actions
                            && (candidate
                                .source_memory_unit_ids
                                .iter()
                                .any(|unit_id| !known_unit_sessions.contains_key(unit_id))
                                || candidate
                                    .supporting_archive_ids
                                    .iter()
                                    .chain(candidate.contradicting_archive_ids.iter())
                                    .any(|archive_id| !known_archive_ids.contains(archive_id)))
                        {
                            return Err(MemoryEngineError::Validation(
                                "new_candidate cites evidence outside the reflection task"
                                    .to_string(),
                            ));
                        }
                        if strict_typed_actions
                            && !candidate.contradicted_core_fact_ids.is_empty()
                        {
                            return Err(MemoryEngineError::Validation(
                                "new_candidate cannot contradict Core; use challenge_core_fact"
                                    .to_string(),
                            ));
                        }
                        let normalized_text = normalize_whitespace(&candidate.text);
                        if strict_typed_actions && !known_candidate_texts.insert(normalized_text) {
                            return Err(MemoryEngineError::Validation(
                                "new_candidate duplicates an existing candidate; reinforce it"
                                    .to_string(),
                            ));
                        }
                        let candidate = self.candidate_from_reflection_draft(
                            &source_session_id,
                            task_scope.clone(),
                            candidate,
                            &known_evidence,
                            &now,
                        )?;
                        match candidate {
                            Some(candidate) => {
                                prepared.push(PreparedReflectionAction::NewCandidate {
                                    action_id,
                                    candidate,
                                });
                            }
                            None if strict_typed_actions => {
                                return Err(MemoryEngineError::Validation(
                                    "new_candidate must contain text, category, summary, and task evidence"
                                        .to_string(),
                                ));
                            }
                            None => {}
                        }
                    }
                    ReflectionActionDraft::ReinforceCandidate {
                        candidate_id,
                        confidence,
                        evidence_summary,
                        source_memory_unit_ids,
                        supporting_archive_ids,
                        tags,
                    } => {
                        if !known_candidate_ids.contains(&candidate_id) {
                            return Err(MemoryEngineError::Validation(format!(
                                "reinforcement candidate was not present in reflection task: {candidate_id}"
                            )));
                        }
                        if !reinforced_ids.insert(candidate_id.clone()) {
                            return Err(MemoryEngineError::Validation(format!(
                                "candidate may be reinforced only once per reflection task: {candidate_id}"
                            )));
                        }
                        if source_memory_unit_ids
                            .iter()
                            .any(|unit_id| !known_unit_sessions.contains_key(unit_id))
                            || supporting_archive_ids
                                .iter()
                                .any(|archive_id| !known_archive_ids.contains(archive_id))
                        {
                            return Err(MemoryEngineError::Validation(format!(
                                "reinforcement cites evidence outside the reflection task: {candidate_id}"
                            )));
                        }
                        let (candidate, previous_updated_at) = self.prepare_candidate_reinforcement(
                            &candidate_id,
                            confidence,
                            &evidence_summary,
                            source_memory_unit_ids,
                            supporting_archive_ids,
                            tags,
                            &known_evidence,
                            &now,
                        )?;
                        prepared.push(PreparedReflectionAction::ReinforcedCandidate {
                            action_id,
                            candidate,
                            previous_updated_at,
                        });
                    }
                    ReflectionActionDraft::ChallengeCoreFact { mut candidate } => {
                        if candidate
                            .source_memory_unit_ids
                            .iter()
                            .any(|unit_id| !known_unit_sessions.contains_key(unit_id))
                            || candidate
                                .supporting_archive_ids
                                .iter()
                                .chain(candidate.contradicting_archive_ids.iter())
                                .any(|archive_id| !known_archive_ids.contains(archive_id))
                            || candidate
                                .contradicted_core_fact_ids
                                .iter()
                                .any(|fact_id| !known_core_facts.contains(fact_id))
                        {
                            return Err(MemoryEngineError::Validation(
                                "challenge_core_fact cites evidence outside the reflection task"
                                    .to_string(),
                            ));
                        }
                        let normalized_text = normalize_whitespace(&candidate.text);
                        if !known_candidate_texts.insert(normalized_text) {
                            return Err(MemoryEngineError::Validation(
                                "challenge_core_fact duplicates an existing or new candidate"
                                    .to_string(),
                            ));
                        }
                        candidate.tags.push("core_challenge".to_string());
                        let Some(candidate) = self.candidate_from_reflection_draft(
                            &source_session_id,
                            task_scope.clone(),
                            candidate,
                            &known_evidence,
                            &now,
                        )? else {
                            return Err(MemoryEngineError::Validation(
                                "challenge_core_fact must contain text, category, summary, and task evidence"
                                    .to_string(),
                            ));
                        };
                        if candidate.contradicted_core_fact_ids.is_empty() {
                            return Err(MemoryEngineError::Validation(
                                "challenge_core_fact must cite a Core fact present in the task"
                                    .to_string(),
                            ));
                        }
                        prepared.push(PreparedReflectionAction::NewCandidate {
                            action_id,
                            candidate,
                        });
                    }
                }
            }

            // Existing candidates are guarded and written before new records.
            // If a concurrent review changed one, fail before creating a new
            // belief from the same reflection response.
            prepared.sort_by_key(|action| {
                matches!(action, PreparedReflectionAction::NewCandidate { .. })
            });
            let candidates = prepared
                .iter()
                .filter_map(|action| match action {
                    PreparedReflectionAction::NewCandidate { candidate, .. } => {
                        Some(candidate.clone())
                    }
                    PreparedReflectionAction::ReinforcedCandidate { .. } => None,
                })
                .collect::<Vec<_>>();
            let reinforced_candidates = prepared
                .iter()
                .filter_map(|action| match action {
                    PreparedReflectionAction::ReinforcedCandidate { candidate, .. } => {
                        Some(candidate.clone())
                    }
                    PreparedReflectionAction::NewCandidate { .. } => None,
                })
                .collect::<Vec<_>>();
            let outcome = ReflectionCandidatesResult {
                schema_version: if strict_typed_actions {
                    REFLECTION_APPLY_RESULT_SCHEMA_VERSION.to_string()
                } else {
                    REFLECTION_RESULT_SCHEMA_VERSION.to_string()
                },
                source_session_id: source_session_id.clone(),
                core_scope,
                created_at: now.clone(),
                candidates,
                reinforced_candidates,
                no_insight: no_insight.clone(),
                operation_id: Some(operation_id.clone()),
                result_digest: Some(result_digest.clone()),
            };
            let intent = ReflectionApplyOperationIntent {
                schema_version: REFLECTION_OPERATION_SCHEMA_VERSION.to_string(),
                task_id: task_id.to_string(),
                reflection_scope: reflection_scope.clone(),
                source_session_id: source_session_id.clone(),
                core_scope: task_scope.clone(),
                result_digest,
                strict_typed_actions,
                prepared_actions: prepared,
                no_insight,
                outcome,
                applied_action_ids: Vec::new(),
                checkpoint_completed: false,
                task_completed: false,
            };
            let operation = JournalOperation {
                schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
                op_id: operation_id,
                op_type: JournalOperationType::ReflectionApply,
                state: JournalState::Started,
                created_at: now.clone(),
                updated_at: now,
                target_files: Vec::new(),
                intent: serde_json::to_value(intent)?,
                recovery_policy: RecoveryPolicy::RetryOrManualReview,
                completed_at: None,
                error: None,
            };
            self.storage.begin_journaled_operation(&operation)?;
            self.execute_reflection_apply_operation_unlocked(operation)
        })
    }

    fn execute_reflection_apply_operation_unlocked(
        &self,
        operation: JournalOperation,
    ) -> Result<ReflectionCandidatesResult> {
        let operation_id = operation.op_id.clone();
        let result = self.execute_reflection_apply_operation_inner(operation);
        if let Err(error) = &result {
            self.record_memory_operation_error_best_effort(&operation_id, error);
        }
        result
    }

    fn execute_reflection_apply_operation_inner(
        &self,
        mut operation: JournalOperation,
    ) -> Result<ReflectionCandidatesResult> {
        if operation.schema_version != JOURNAL_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
                actual: operation.schema_version,
            });
        }
        if operation.op_type != JournalOperationType::ReflectionApply {
            return Err(MemoryEngineError::Validation(format!(
                "operation is not reflection_apply: {}",
                operation.op_id
            )));
        }
        let mut intent: ReflectionApplyOperationIntent =
            serde_json::from_value(operation.intent.clone())?;
        if intent.schema_version != REFLECTION_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_OPERATION_SCHEMA_VERSION.to_string(),
                actual: intent.schema_version,
            });
        }

        for effect in intent.prepared_actions.clone() {
            let action_id = match &effect {
                PreparedReflectionAction::NewCandidate { action_id, .. }
                | PreparedReflectionAction::ReinforcedCandidate { action_id, .. } => action_id,
            }
            .clone();
            match effect {
                PreparedReflectionAction::NewCandidate { candidate, .. } => {
                    self.with_resource_lock(candidate_lock_key(&candidate.candidate_id), || {
                        match self
                            .storage
                            .read_candidate_belief_optional(&candidate.candidate_id)?
                        {
                            Some(current) if current == candidate => {
                                // Candidate persistence now also materializes
                                // its hypothesis node and evidence links. A
                                // crash may leave that projection partial even
                                // when the compatibility read already matches.
                                self.storage.write_candidate_belief(&candidate)
                            }
                            Some(_) => Err(MemoryEngineError::Validation(format!(
                                "reflection operation candidate conflicts with stored value: {}",
                                candidate.candidate_id
                            ))),
                            None => self.storage.write_candidate_belief(&candidate),
                        }
                    })?;
                }
                PreparedReflectionAction::ReinforcedCandidate {
                    candidate,
                    previous_updated_at,
                    ..
                } => {
                    self.with_resource_lock(candidate_lock_key(&candidate.candidate_id), || {
                        let current = self
                            .storage
                            .read_candidate_belief(&candidate.candidate_id)?;
                        if current == candidate {
                            // Re-run the idempotent write to close a possibly
                            // partial hypothesis-graph projection.
                            return self.storage.write_candidate_belief(&candidate);
                        }
                        if current.updated_at != previous_updated_at {
                            return Err(MemoryEngineError::Validation(format!(
                                "candidate changed while durable reflection was being applied: {}",
                                candidate.candidate_id
                            )));
                        }
                        self.storage.write_candidate_belief(&candidate)
                    })?;
                }
            }
            if !intent.applied_action_ids.contains(&action_id) {
                intent.applied_action_ids.push(action_id);
                self.persist_reflection_apply_operation(&mut operation, &intent)?;
            }
        }

        let task = self.storage.load_task(&intent.task_id)?;
        self.complete_reflection_checkpoint_unlocked(
            &task,
            &intent.source_session_id,
            intent.core_scope.as_deref(),
            intent.outcome.candidates.len() + intent.outcome.reinforced_candidates.len(),
            &operation.op_id,
            &operation.created_at,
        )?;
        if !intent.checkpoint_completed {
            intent.checkpoint_completed = true;
            self.persist_reflection_apply_operation(&mut operation, &intent)?;
        }

        let durable_outcome = ReflectionOutcome {
            schema_version: REFLECTION_OUTCOME_SCHEMA_VERSION.to_string(),
            operation_id: operation.op_id.clone(),
            result_digest: intent.result_digest.clone(),
            completed_at: operation.created_at.clone(),
            new_candidate_ids: intent
                .outcome
                .candidates
                .iter()
                .map(|candidate| candidate.candidate_id.clone())
                .collect(),
            reinforced_candidate_ids: intent
                .outcome
                .reinforced_candidates
                .iter()
                .map(|candidate| candidate.candidate_id.clone())
                .collect(),
            no_insight_reason: intent
                .no_insight
                .as_ref()
                .map(|no_insight| no_insight.reason),
        };
        let expected_task_outcome = serde_json::to_value(&durable_outcome)?;
        let mut task = self.storage.load_task(&intent.task_id)?;
        if let Some(existing) = task.outcome.as_ref() {
            if existing != &expected_task_outcome {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection task has a different durable outcome: {}",
                    task.task_id
                )));
            }
        }
        task.state = TaskState::Completed;
        task.updated_at = operation.created_at.clone();
        task.last_error = None;
        task.outcome = Some(expected_task_outcome);
        self.storage.save_task(&task)?;
        if !intent.task_completed {
            intent.task_completed = true;
            self.persist_reflection_apply_operation(&mut operation, &intent)?;
        }

        operation.state = JournalState::Completed;
        operation.completed_at = Some(now_rfc3339()?);
        operation.error = None;
        operation.intent = serde_json::to_value(&intent)?;
        operation.updated_at = operation
            .completed_at
            .clone()
            .unwrap_or_else(|| operation.updated_at.clone());
        self.storage.begin_journaled_operation(&operation)?;
        Ok(intent.outcome)
    }

    fn persist_reflection_apply_operation(
        &self,
        operation: &mut JournalOperation,
        intent: &ReflectionApplyOperationIntent,
    ) -> Result<()> {
        operation.updated_at = now_rfc3339()?;
        operation.intent = serde_json::to_value(intent)?;
        self.storage.begin_journaled_operation(operation)
    }

    fn revalidate_reflection_task_evidence(
        &self,
        task: &PendingTask,
        core_scope: Option<&str>,
        actions: &[ReflectionActionDraft],
    ) -> Result<()> {
        let unit_snapshots = task
            .inputs
            .get("memory_units")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|unit| {
                Some((
                    unit.get("memory_unit_id")?.as_str()?.to_string(),
                    (
                        unit.get("source_session_id")?.as_str()?.to_string(),
                        unit.get("archive_id")?.as_str()?.to_string(),
                        unit.get("updated_at")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ),
                ))
            })
            .collect::<HashMap<_, _>>();
        let mut cited_unit_ids = task
            .inputs
            .get("new_memory_unit_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<HashSet<_>>();
        let mut cited_archive_ids = HashSet::new();
        let mut cited_candidate_ids = HashSet::new();
        let mut cited_core_fact_ids = HashSet::new();
        for action in actions {
            match action {
                ReflectionActionDraft::NewCandidate { candidate }
                | ReflectionActionDraft::ChallengeCoreFact { candidate } => {
                    cited_unit_ids.extend(candidate.source_memory_unit_ids.iter().cloned());
                    cited_archive_ids.extend(candidate.supporting_archive_ids.iter().cloned());
                    cited_archive_ids.extend(candidate.contradicting_archive_ids.iter().cloned());
                    cited_core_fact_ids
                        .extend(candidate.contradicted_core_fact_ids.iter().cloned());
                }
                ReflectionActionDraft::ReinforceCandidate {
                    candidate_id,
                    source_memory_unit_ids,
                    supporting_archive_ids,
                    ..
                } => {
                    cited_candidate_ids.insert(candidate_id.clone());
                    cited_unit_ids.extend(source_memory_unit_ids.iter().cloned());
                    cited_archive_ids.extend(supporting_archive_ids.iter().cloned());
                }
            }
        }

        for unit_id in cited_unit_ids {
            let Some((expected_session, expected_archive, expected_updated_at)) =
                unit_snapshots.get(&unit_id)
            else {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection evidence is outside the reflection task snapshot: {unit_id}"
                )));
            };
            let unit = self.storage.read_memory_unit_by_id(&unit_id)?;
            if unit.source_session_id != *expected_session
                || unit.archive_id != *expected_archive
                || expected_updated_at
                    .as_deref()
                    .is_some_and(|expected| unit.updated_at != expected)
                || unit.status != MemoryUnitStatus::ActiveArchive
                || unit.fidelity_status != FidelityStatus::Valid
            {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection evidence changed after task creation: {unit_id}"
                )));
            }
        }

        let task_archive_ids = task
            .inputs
            .get("archives")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|archive| archive.get("archive_id").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        for archive_id in cited_archive_ids {
            if !task_archive_ids.contains(archive_id.as_str()) {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection archive evidence is outside the reflection task snapshot: {archive_id}"
                )));
            }
            let archive = self.storage.read_archive_entry_by_id(&archive_id)?;
            if archive.status != ArchiveStatus::Complete {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection archive evidence is no longer complete: {archive_id}"
                )));
            }
        }

        for candidate_id in cited_candidate_ids {
            let candidate = self.storage.read_candidate_belief(&candidate_id)?;
            let expected_updated_at = task
                .inputs
                .get("existing_candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|snapshot| {
                    snapshot.get("candidate_id").and_then(Value::as_str)
                        == Some(candidate_id.as_str())
                })
                .and_then(|snapshot| snapshot.get("updated_at"))
                .and_then(Value::as_str);
            if candidate.core_scope.as_deref() != core_scope
                || expected_updated_at.is_some_and(|expected| candidate.updated_at != expected)
                || !matches!(
                    candidate.status,
                    CandidateStatus::Draft | CandidateStatus::ReadyForReview
                )
            {
                return Err(MemoryEngineError::Validation(format!(
                    "reflection candidate changed after task creation: {candidate_id}"
                )));
            }
        }

        if !cited_core_fact_ids.is_empty() {
            let facts = self
                .storage
                .read_core_store_categories()?
                .into_items()
                .into_iter()
                .flat_map(|category| category.facts)
                .filter(|fact| cited_core_fact_ids.contains(&fact.core_fact_id))
                .map(|fact| (fact.core_fact_id.clone(), fact))
                .collect::<HashMap<_, _>>();
            for fact_id in cited_core_fact_ids {
                let Some(fact) = facts.get(&fact_id) else {
                    return Err(MemoryEngineError::Validation(format!(
                        "reflection Core evidence disappeared after task creation: {fact_id}"
                    )));
                };
                let expected_updated_at = task
                    .inputs
                    .get("core_facts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|snapshot| {
                        snapshot.get("core_fact_id").and_then(Value::as_str)
                            == Some(fact_id.as_str())
                    })
                    .and_then(|snapshot| snapshot.get("updated_at"))
                    .and_then(Value::as_str);
                if fact.scope.as_deref() != core_scope
                    || expected_updated_at.is_some_and(|expected| fact.updated_at != expected)
                    || !core_fact_visible_in_context(fact.status)
                {
                    return Err(MemoryEngineError::Validation(format!(
                        "reflection Core evidence changed after task creation: {fact_id}"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn recover_pending_memory_operations(&self) -> Result<usize> {
        self.ensure_manifest_initialized()?;
        let recovered = self.recover_pending_memory_operations_unlocked()?;
        self.memory_operations_recovered
            .store(true, Ordering::Release);
        Ok(recovered)
    }

    pub(super) fn recover_pending_memory_operations_unlocked(&self) -> Result<usize> {
        let operations = self.storage.load_journaled_operations()?;
        if let Some(warning) = operations.warnings.first() {
            return Err(MemoryEngineError::Storage(warning.note()));
        }
        let mut recovered = 0usize;
        for operation in operations.items {
            if operation.state == JournalState::NeedsManualReview {
                return Err(MemoryEngineError::Validation(format!(
                    "journal operation requires manual review: {} ({})",
                    operation.op_id,
                    operation.error.as_deref().unwrap_or("no error recorded")
                )));
            }
            if operation.state != JournalState::Started {
                continue;
            }
            match operation.op_type {
                JournalOperationType::ReflectionApply => {
                    let intent: ReflectionApplyOperationIntent =
                        serde_json::from_value(operation.intent.clone())?;
                    self.with_resource_lock(reflection_lock_key(&intent.reflection_scope), || {
                        let current = self
                            .storage
                            .load_journaled_operation(&operation.op_id)?
                            .ok_or_else(|| {
                                MemoryEngineError::Storage(format!(
                                    "journal operation disappeared during recovery: {}",
                                    operation.op_id
                                ))
                            })?;
                        self.execute_reflection_apply_operation_unlocked(current)
                            .map(|_| ())
                    })?;
                    recovered += 1;
                }
                JournalOperationType::CandidateReview => {
                    let intent: CandidateReviewOperationIntent =
                        serde_json::from_value(operation.intent.clone())?;
                    self.with_resource_lock(candidate_lock_key(&intent.candidate_id), || {
                        let current = self
                            .storage
                            .load_journaled_operation(&operation.op_id)?
                            .ok_or_else(|| {
                                MemoryEngineError::Storage(format!(
                                    "journal operation disappeared during recovery: {}",
                                    operation.op_id
                                ))
                            })?;
                        self.execute_candidate_review_operation_unlocked(current)
                            .map(|_| ())
                    })?;
                    recovered += 1;
                }
                JournalOperationType::Migration => {
                    self.with_resource_lock("hypothesis_graph:migration".to_string(), || {
                        let current = self
                            .storage
                            .load_journaled_operation(&operation.op_id)?
                            .ok_or_else(|| {
                                MemoryEngineError::Storage(format!(
                                    "journal operation disappeared during recovery: {}",
                                    operation.op_id
                                ))
                            })?;
                        self.recover_hypothesis_graph_migration_unlocked(current)
                            .map(|_| ())
                    })?;
                    recovered += 1;
                }
                _ => {}
            }
        }
        Ok(recovered)
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
            let candidate = self.storage.read_candidate_belief(&input.candidate_id)?;
            if !matches!(
                candidate.status,
                CandidateStatus::Draft | CandidateStatus::ReadyForReview
            ) {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate is not reviewable in state {:?}: {}",
                    candidate.status, candidate.candidate_id
                )));
            }
            let requested_scope = normalize_optional_string(input.core_scope.as_deref());
            if requested_scope.is_some() && requested_scope != candidate.core_scope {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate review cannot change core_scope: candidate={:?} review={requested_scope:?}",
                    candidate.core_scope
                )));
            }
            if input.decision == ReviewDecision::Approved {
                self.revalidate_candidate_review_evidence(&candidate)?;
            }

            let review = ReviewRecord {
                reviewed_by: normalize_whitespace(&input.reviewed_by),
                reviewed_at: now.clone(),
                decision: input.decision,
                note: normalize_optional_string(input.note.as_deref()),
            };
            let input_digest = candidate_review_input_digest(
                &candidate.candidate_id,
                &review,
                &requested_scope,
            )?;
            let operation_digest = digest_json(&(
                candidate.candidate_id.as_str(),
                candidate.updated_at.as_str(),
                input_digest.as_str(),
            ))?;
            let operation_id = format!(
                "candidate_review_{}_{}",
                candidate.candidate_id,
                &operation_digest[..16]
            );

            let mut desired_candidate = candidate.clone();
            let mut core_effects = Vec::new();
            let mut promoted_fact = None;
            let mut contested_facts = Vec::new();
            match input.decision {
                ReviewDecision::Approved => {
                    let prepared = self.prepare_candidate_approval_effects(&candidate, &now)?;
                    core_effects = prepared.0;
                    promoted_fact = Some(prepared.1);
                    contested_facts = prepared.2;
                    desired_candidate.status = CandidateStatus::Promoted;
                    desired_candidate.promoted_core_fact_id = promoted_fact
                        .as_ref()
                        .map(|fact| fact.core_fact_id.clone());
                }
                ReviewDecision::Rejected => {
                    desired_candidate.status = CandidateStatus::Rejected;
                }
                ReviewDecision::NeedsChanges => {
                    desired_candidate.status = CandidateStatus::Draft;
                }
            }

            desired_candidate.updated_at = now.clone();
            desired_candidate.review = Some(review);
            let result = CandidateReviewResult {
                schema_version: CANDIDATE_REVIEW_RESULT_SCHEMA_VERSION.to_string(),
                candidate: desired_candidate.clone(),
                promoted_fact,
                contested_facts,
            };
            let intent = CandidateReviewOperationIntent {
                schema_version: REFLECTION_OPERATION_SCHEMA_VERSION.to_string(),
                candidate_id: candidate.candidate_id.clone(),
                previous_candidate_updated_at: candidate.updated_at.clone(),
                input_digest,
                desired_candidate,
                core_effects,
                result,
                applied_core_categories: Vec::new(),
                candidate_applied: false,
            };
            let operation = JournalOperation {
                schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
                op_id: operation_id,
                op_type: JournalOperationType::CandidateReview,
                state: JournalState::Started,
                created_at: now.clone(),
                updated_at: now,
                target_files: Vec::new(),
                intent: serde_json::to_value(intent)?,
                recovery_policy: RecoveryPolicy::RetryOrManualReview,
                completed_at: None,
                error: None,
            };
            self.storage.begin_journaled_operation(&operation)?;
            self.execute_candidate_review_operation_unlocked(operation)
        })
    }

    fn revalidate_candidate_review_evidence(&self, candidate: &CandidateBelief) -> Result<()> {
        for unit_id in &candidate.source_memory_unit_ids {
            let unit = self.storage.read_memory_unit_by_id(unit_id)?;
            if unit.status != MemoryUnitStatus::ActiveArchive
                || unit.fidelity_status != FidelityStatus::Valid
                || !candidate.supporting_archive_ids.contains(&unit.archive_id)
            {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate evidence is no longer valid: {unit_id}"
                )));
            }
        }
        for archive_id in candidate
            .supporting_archive_ids
            .iter()
            .chain(candidate.contradicting_archive_ids.iter())
        {
            let archive = self.storage.read_archive_entry_by_id(archive_id)?;
            if archive.status != ArchiveStatus::Complete {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate archive evidence is no longer complete: {archive_id}"
                )));
            }
        }

        let target_fact_ids = candidate
            .contradicted_core_fact_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if target_fact_ids.is_empty() {
            return Ok(());
        }
        let facts = self
            .storage
            .read_core_store_categories()?
            .into_items()
            .into_iter()
            .flat_map(|category| category.facts)
            .filter(|fact| target_fact_ids.contains(&fact.core_fact_id))
            .map(|fact| (fact.core_fact_id.clone(), fact))
            .collect::<HashMap<_, _>>();
        for fact_id in target_fact_ids {
            let Some(fact) = facts.get(&fact_id) else {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate Core evidence no longer exists: {fact_id}"
                )));
            };
            if fact.scope != candidate.core_scope || fact.status != CoreFactStatus::Active {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate Core evidence is no longer active in its scope: {fact_id}"
                )));
            }
        }
        Ok(())
    }

    fn prepare_candidate_approval_effects(
        &self,
        candidate: &CandidateBelief,
        now: &str,
    ) -> Result<(Vec<CoreCategoryEffect>, CoreFact, Vec<CoreFact>)> {
        let target_ids = candidate
            .contradicted_core_fact_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let stored_categories = self.storage.read_core_store_categories()?.into_items();
        let mut category_names = stored_categories
            .iter()
            .filter(|category| {
                category
                    .facts
                    .iter()
                    .any(|fact| target_ids.contains(&fact.core_fact_id))
            })
            .map(|category| category.category.clone())
            .collect::<BTreeSet<_>>();
        let promotion_category = normalize_whitespace(&candidate.category);
        category_names.insert(promotion_category.clone());

        let mut effects = Vec::new();
        let mut contested = Vec::new();
        let mut promoted = None;
        for category_name in category_names {
            let previous = self.storage.read_core_store_category(&category_name)?;
            let mut desired = previous.clone();
            desired.schema_version = CORE_STORE_SCHEMA_VERSION.to_string();
            desired.category = category_name.clone();

            for fact in &mut desired.facts {
                if target_ids.contains(&fact.core_fact_id)
                    && fact.status == CoreFactStatus::Active
                    && fact.scope == candidate.core_scope
                {
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
                            target: candidate.candidate_id.clone(),
                            note: Some(candidate.text.clone()),
                        },
                    );
                    contested.push(fact.clone());
                }
            }

            if category_name == promotion_category {
                let text = normalize_whitespace(&candidate.text);
                let needle = normalize_match_text(&text);
                let mut tags = candidate.tags.clone();
                tags.push("reflection_candidate".to_string());
                tags.push("manual_review".to_string());
                let tags = unique_strings(tags);
                let source_archive_ids = unique_strings(candidate.supporting_archive_ids.clone());
                let fact = if let Some(existing) = desired.facts.iter_mut().find(|fact| {
                    normalize_match_text(&fact.text) == needle && fact.scope == candidate.core_scope
                }) {
                    existing.text = text;
                    existing.status = CoreFactStatus::Active;
                    existing.confidence = existing
                        .confidence
                        .max(candidate.confidence)
                        .clamp(0.0, 1.0);
                    existing.updated_at = now.to_string();
                    merge_unique(&mut existing.tags, &tags);
                    merge_unique(&mut existing.source_archive_ids, &source_archive_ids);
                    if existing.source_candidate_id.is_none() {
                        existing.source_candidate_id = Some(candidate.candidate_id.clone());
                    }
                    existing.clone()
                } else {
                    let fact = CoreFact {
                        schema_version: CORE_FACT_SCHEMA_VERSION.to_string(),
                        core_fact_id: new_id("core_fact")?,
                        scope: candidate.core_scope.clone(),
                        text,
                        status: CoreFactStatus::Active,
                        confidence: candidate.confidence.clamp(0.0, 1.0),
                        created_at: now.to_string(),
                        updated_at: now.to_string(),
                        source_archive_ids,
                        source_candidate_id: Some(candidate.candidate_id.clone()),
                        tags,
                        links: Vec::new(),
                        review: None,
                    };
                    desired.facts.push(fact.clone());
                    fact
                };
                promoted = Some(fact);
            }

            if desired != previous {
                desired.updated_at = now.to_string();
                effects.push(CoreCategoryEffect {
                    category: desired,
                    previous_updated_at: previous.updated_at,
                });
            }
        }

        Ok((
            effects,
            promoted.ok_or_else(|| {
                MemoryEngineError::Validation(
                    "candidate approval did not prepare a promoted Core fact".to_string(),
                )
            })?,
            contested,
        ))
    }

    fn execute_candidate_review_operation_unlocked(
        &self,
        operation: JournalOperation,
    ) -> Result<CandidateReviewResult> {
        let operation_id = operation.op_id.clone();
        let result = self.execute_candidate_review_operation_inner(operation);
        if let Err(error) = &result {
            self.record_memory_operation_error_best_effort(&operation_id, error);
        }
        result
    }

    fn execute_candidate_review_operation_inner(
        &self,
        mut operation: JournalOperation,
    ) -> Result<CandidateReviewResult> {
        if operation.schema_version != JOURNAL_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
                actual: operation.schema_version,
            });
        }
        if operation.op_type != JournalOperationType::CandidateReview {
            return Err(MemoryEngineError::Validation(format!(
                "operation is not candidate_review: {}",
                operation.op_id
            )));
        }
        let mut intent: CandidateReviewOperationIntent =
            serde_json::from_value(operation.intent.clone())?;
        if intent.schema_version != REFLECTION_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_OPERATION_SCHEMA_VERSION.to_string(),
                actual: intent.schema_version,
            });
        }
        if operation.state == JournalState::Completed {
            return Ok(intent.result);
        }

        for effect in intent.core_effects.clone() {
            let category_name = effect.category.category.clone();
            self.with_resource_lock(core_lock_key(&category_name), || {
                let current = self.storage.read_core_store_category(&category_name)?;
                if current == effect.category {
                    return Ok(());
                }
                if current.updated_at != effect.previous_updated_at {
                    return Err(MemoryEngineError::Validation(format!(
                        "Core category changed while durable candidate review was being applied: {category_name}"
                    )));
                }
                self.storage.write_core_store_category(&effect.category)
            })?;
            if !intent.applied_core_categories.contains(&category_name) {
                intent.applied_core_categories.push(category_name);
                self.persist_candidate_review_operation(&mut operation, &intent)?;
            }
        }

        let current = self.storage.read_candidate_belief(&intent.candidate_id)?;
        if current == intent.desired_candidate {
            // Owner-review recovery must also finish the graph projection if
            // the earlier attempt stopped between its atomic files.
            self.storage
                .write_candidate_belief(&intent.desired_candidate)?;
        } else {
            if current.updated_at != intent.previous_candidate_updated_at {
                return Err(MemoryEngineError::Validation(format!(
                    "candidate changed while durable review was being applied: {}",
                    intent.candidate_id
                )));
            }
            self.storage
                .write_candidate_belief(&intent.desired_candidate)?;
        }
        if !intent.candidate_applied {
            intent.candidate_applied = true;
            self.persist_candidate_review_operation(&mut operation, &intent)?;
        }

        operation.state = JournalState::Completed;
        operation.completed_at = Some(now_rfc3339()?);
        operation.error = None;
        operation.updated_at = operation
            .completed_at
            .clone()
            .unwrap_or_else(|| operation.updated_at.clone());
        operation.intent = serde_json::to_value(&intent)?;
        self.storage.begin_journaled_operation(&operation)?;
        Ok(intent.result)
    }

    fn persist_candidate_review_operation(
        &self,
        operation: &mut JournalOperation,
        intent: &CandidateReviewOperationIntent,
    ) -> Result<()> {
        operation.updated_at = now_rfc3339()?;
        operation.intent = serde_json::to_value(intent)?;
        self.storage.begin_journaled_operation(operation)
    }

    pub(super) fn record_memory_operation_error_best_effort(
        &self,
        operation_id: &str,
        error: &MemoryEngineError,
    ) {
        let Ok(Some(mut operation)) = self.storage.load_journaled_operation(operation_id) else {
            return;
        };
        operation.error = Some(error.to_string());
        if let Ok(now) = now_rfc3339() {
            operation.updated_at = now;
        }
        if matches!(
            error,
            MemoryEngineError::Validation(_)
                | MemoryEngineError::IncompatibleSchema { .. }
                | MemoryEngineError::TaskNotFound(_)
                | MemoryEngineError::Json(_)
        ) {
            operation.state = JournalState::NeedsManualReview;
        }
        let _ = self.storage.begin_journaled_operation(&operation);
    }

    fn complete_reflection_checkpoint_unlocked(
        &self,
        task: &PendingTask,
        session_id: &str,
        core_scope: Option<&str>,
        candidate_count: usize,
        operation_id: &str,
        now: &str,
    ) -> Result<()> {
        let reflected_ids = task
            .inputs
            .get("new_memory_unit_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        let allowed_session_ids = task
            .inputs
            .get("source_session_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .chain(std::iter::once(session_id.to_string()))
            .collect::<HashSet<_>>();
        let provided_unit_ids = task
            .inputs
            .get("memory_units")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|unit| unit.get("memory_unit_id").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        let mut changed_archives = HashMap::<String, ArchiveEntry>::new();
        for memory_unit_id in reflected_ids {
            if !provided_unit_ids.contains(memory_unit_id.as_str()) {
                continue;
            }
            let mut unit = self.storage.read_memory_unit_by_id(&memory_unit_id)?;
            if !allowed_session_ids.contains(&unit.source_session_id) {
                continue;
            }
            if unit.last_reflection_operation_id.as_deref() != Some(operation_id) {
                unit.reflection_count = unit.reflection_count.saturating_add(1);
                unit.last_reflected_at = Some(now.to_string());
                unit.last_reflection_operation_id = Some(operation_id.to_string());
                unit.updated_at = now.to_string();
                self.storage.write_memory_unit(&unit)?;
            }

            let archive = if let Some(archive) = changed_archives.get_mut(&unit.archive_id) {
                archive
            } else {
                let archive = self.storage.read_archive_entry_by_id(&unit.archive_id)?;
                changed_archives.insert(unit.archive_id.clone(), archive);
                changed_archives
                    .get_mut(&unit.archive_id)
                    .expect("inserted reflection archive")
            };
            archive.updated_at = now.to_string();
            if let Some(archive_unit) = archive
                .memory_units
                .iter_mut()
                .find(|archive_unit| archive_unit.memory_unit_id == unit.memory_unit_id)
            {
                *archive_unit = unit;
            }
        }
        for archive in changed_archives.into_values() {
            self.storage
                .update_archive_entry(&archive.archive_id, &archive)?;
        }

        let reflection_scope = task
            .inputs
            .get("reflection_scope")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| reflection_scope_id(session_id, core_scope));
        let mut state = self
            .storage
            .read_reflection_state(&reflection_scope)?
            .unwrap_or(ReflectionState {
                schema_version: REFLECTION_STATE_SCHEMA_VERSION.to_string(),
                reflection_scope: reflection_scope.clone(),
                source_session_ids: vec![session_id.to_string()],
                source_session_id: session_id.to_string(),
                created_at: now.to_string(),
                updated_at: now.to_string(),
                last_completed_at: None,
                last_task_id: None,
                last_trigger_reason: None,
                last_candidate_count: 0,
            });
        state.updated_at = now.to_string();
        state.source_session_id = session_id.to_string();
        state.source_session_ids = unique_strings(
            state
                .source_session_ids
                .into_iter()
                .chain(allowed_session_ids)
                .collect(),
        );
        state.last_completed_at = Some(now.to_string());
        state.last_task_id = Some(task.task_id.clone());
        state.last_candidate_count = candidate_count;
        self.storage.write_reflection_state(&state)
    }

    fn begin_reflection_analysis_unlocked(
        &self,
        session_id: &str,
        core_scope: Option<String>,
        focus_memory_unit_ids: Option<&HashSet<String>>,
        trigger_reason: Option<ReflectionTriggerReason>,
    ) -> Result<ReflectionPassStart> {
        let core_scope = normalize_optional_string(core_scope.as_deref());
        let reflection_scope = reflection_scope_id(session_id, core_scope.as_deref());
        if let Some(task) = self.active_reflection_task_unlocked(session_id, &reflection_scope)? {
            return Err(MemoryEngineError::Validation(format!(
                "reflection task already active for scope {reflection_scope}: {}",
                task.task_id
            )));
        }
        let mut state =
            self.ensure_reflection_membership_unlocked(session_id, &reflection_scope)?;
        let source_session_ids = reflection_session_ids(Some(&state), session_id);

        let now = now_rfc3339()?;
        let mut inputs = self.build_reflection_inputs_unlocked(
            session_id,
            &source_session_ids,
            core_scope.clone(),
            focus_memory_unit_ids,
        )?;
        let memory_unit_count = inputs
            .get("memory_units")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let new_memory_unit_count = inputs
            .get("new_memory_unit_ids")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let related_memory_unit_count = inputs
            .get("related_memory_unit_ids")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let existing_candidate_count = inputs
            .get("existing_candidates")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let core_fact_count = inputs
            .get("core_facts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if let Some(object) = inputs.as_object_mut() {
            object.insert(
                "reflection_scope".to_string(),
                Value::String(reflection_scope.clone()),
            );
            object.insert(
                "trigger_reason".to_string(),
                serde_json::to_value(trigger_reason).unwrap_or(Value::Null),
            );
        }
        let task = PendingTask {
            schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
            task_id: new_id("task")?,
            task_type: TaskType::ReflectionAnalyze,
            state: TaskState::Pending,
            created_at: now.clone(),
            updated_at: now.clone(),
            prompt_id: self.options.reflection.prompt_id.clone(),
            prompt_version: self.options.reflection.prompt_version,
            role_hint: ModelRole::Reasoning,
            expected_output_schema: REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION.to_string(),
            inputs: inputs.clone(),
            attempts: Vec::new(),
            last_error: None,
            outcome: None,
        };
        self.storage.save_task(&task)?;

        state.updated_at = now.clone();
        state.source_session_id = session_id.to_string();
        state.source_session_ids = source_session_ids;
        state.last_task_id = Some(task.task_id.clone());
        state.last_trigger_reason = trigger_reason;
        self.storage.write_reflection_state(&state)?;

        let request = llm_request_from_task(
            &task,
            &self.options.reflection.prompt_id,
            json!({ "reflection_task": inputs }),
        )?;
        Ok(ReflectionPassStart {
            pending_task: task,
            request,
            source_session_id: session_id.to_string(),
            core_scope,
            memory_unit_count,
            new_memory_unit_count,
            related_memory_unit_count,
            existing_candidate_count,
            core_fact_count,
        })
    }

    fn evaluate_reflection_trigger_unlocked(
        &self,
        session_id: &str,
        core_scope: Option<String>,
        opportunity: ReflectionOpportunity,
    ) -> Result<ReflectionTriggerStatus> {
        validate_reflection_config(&self.options.reflection)?;
        let now = now_rfc3339()?;
        let core_scope = normalize_optional_string(core_scope.as_deref());
        let reflection_scope = reflection_scope_id(session_id, core_scope.as_deref());
        let state = self.storage.read_reflection_state(&reflection_scope)?;
        let source_session_ids = reflection_session_ids(state.as_ref(), session_id);
        let last_completed_at = state
            .as_ref()
            .and_then(|state| state.last_completed_at.clone());

        let eligible = self
            .reflection_memory_units_for_sessions_unlocked(&source_session_ids)?
            .into_iter()
            .filter(|unit| unit.reflection_count == 0)
            .collect::<Vec<_>>();
        let accumulated_salience = eligible
            .iter()
            .map(|unit| unit.weight.clamp(0.0, 1.0))
            .sum::<f64>();
        let oldest_eligible_memory_at = eligible.iter().map(|unit| unit.created_at.clone()).min();

        if let Some(task) = self.active_reflection_task_unlocked(session_id, &reflection_scope)? {
            return Ok(ReflectionTriggerStatus {
                schema_version: REFLECTION_TRIGGER_STATUS_SCHEMA_VERSION.to_string(),
                source_session_id: session_id.to_string(),
                source_session_ids,
                reflection_scope,
                opportunity,
                evaluated_at: now,
                eligible_memory_unit_count: eligible.len(),
                accumulated_salience,
                salience_threshold: self.options.reflection.salience_threshold,
                critical_unit_weight: self.options.reflection.critical_unit_weight,
                oldest_eligible_memory_at,
                last_completed_at,
                pending_task_id: Some(task.task_id),
                due: false,
                reason: ReflectionTriggerReason::AlreadyPending,
            });
        }

        let reason = if eligible.is_empty() {
            ReflectionTriggerReason::NoEligibleMemory
        } else if eligible
            .iter()
            .any(|unit| unit.weight >= self.options.reflection.critical_unit_weight)
        {
            ReflectionTriggerReason::CriticalMemory
        } else if accumulated_salience >= self.options.reflection.salience_threshold {
            ReflectionTriggerReason::AccumulatedSalience
        } else {
            let interval_reference = last_completed_at
                .as_deref()
                .or(oldest_eligible_memory_at.as_deref());
            let interval_elapsed = interval_reference
                .and_then(|reference| timestamp_age_days(reference, &now))
                .is_some_and(|days| days >= self.options.reflection.max_interval_days);
            if eligible.len() >= self.options.reflection.scheduled_min_units && interval_elapsed {
                ReflectionTriggerReason::ScheduledInterval
            } else {
                ReflectionTriggerReason::NotDue
            }
        };
        let due = matches!(
            reason,
            ReflectionTriggerReason::CriticalMemory
                | ReflectionTriggerReason::AccumulatedSalience
                | ReflectionTriggerReason::ScheduledInterval
        );

        Ok(ReflectionTriggerStatus {
            schema_version: REFLECTION_TRIGGER_STATUS_SCHEMA_VERSION.to_string(),
            source_session_id: session_id.to_string(),
            source_session_ids,
            reflection_scope,
            opportunity,
            evaluated_at: now,
            eligible_memory_unit_count: eligible.len(),
            accumulated_salience,
            salience_threshold: self.options.reflection.salience_threshold,
            critical_unit_weight: self.options.reflection.critical_unit_weight,
            oldest_eligible_memory_at,
            last_completed_at,
            pending_task_id: None,
            due,
            reason,
        })
    }

    fn active_reflection_task_unlocked(
        &self,
        session_id: &str,
        reflection_scope: &str,
    ) -> Result<Option<PendingTask>> {
        Ok(self
            .storage
            .load_tasks()?
            .into_items()
            .into_iter()
            .find(|task| {
                if task.task_type != TaskType::ReflectionAnalyze
                    || !matches!(task.state, TaskState::Pending | TaskState::Submitted)
                {
                    return false;
                }
                let task_scope = task
                    .inputs
                    .get("reflection_scope")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        reflection_scope_id(
                            session_id,
                            task.inputs.get("core_scope").and_then(Value::as_str),
                        )
                    });
                task_scope == reflection_scope
            }))
    }

    pub(super) fn build_reflection_inputs_unlocked(
        &self,
        session_id: &str,
        source_session_ids: &[String],
        core_scope: Option<String>,
        focus_memory_unit_ids: Option<&HashSet<String>>,
    ) -> Result<Value> {
        let core_scope = normalize_optional_string(core_scope.as_deref());
        let contexts = self.reflection_unit_contexts_for_sessions_unlocked(source_session_ids)?;
        let trigger_contexts = contexts
            .iter()
            .filter(|context| {
                focus_memory_unit_ids
                    .is_none_or(|focus_ids| focus_ids.contains(&context.unit.memory_unit_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        let related = focus_memory_unit_ids.map_or_else(Vec::new, |_| {
            select_related_reflection_units(
                &contexts,
                &trigger_contexts,
                self.options.reflection.max_related_units,
                self.options.reflection.min_related_score,
            )
        });

        let new_memory_unit_ids = trigger_contexts
            .iter()
            .filter(|context| focus_memory_unit_ids.is_some() || context.unit.reflection_count == 0)
            .map(|context| context.unit.memory_unit_id.clone())
            .collect::<Vec<_>>();
        let related_memory_unit_ids = related
            .iter()
            .map(|related| related.context.unit.memory_unit_id.clone())
            .collect::<Vec<_>>();

        let mut memory_units = trigger_contexts
            .iter()
            .map(|context| {
                reflection_unit_input(
                    context,
                    if focus_memory_unit_ids.is_some() {
                        "trigger"
                    } else {
                        "manual_scope"
                    },
                    None,
                    &[],
                )
            })
            .collect::<Vec<_>>();
        memory_units.extend(related.iter().map(|related| {
            reflection_unit_input(
                &related.context,
                "related_history",
                Some(related.relation_score),
                &related.relation_reasons,
            )
        }));
        let evidence_session_ids = unique_strings(
            memory_units
                .iter()
                .filter_map(|unit| unit.get("source_session_id").and_then(Value::as_str))
                .map(str::to_string)
                .collect(),
        );

        let selected_archive_ids = memory_units
            .iter()
            .filter_map(|unit| unit.get("archive_id").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        let mut archives = contexts
            .iter()
            .filter(|context| selected_archive_ids.contains(context.archive.archive_id.as_str()))
            .map(|context| context.archive.clone())
            .collect::<Vec<_>>();
        archives.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.archive_id.cmp(&right.archive_id))
        });
        archives.dedup_by(|left, right| left.archive_id == right.archive_id);
        let archive_summaries = archives
            .iter()
            .map(|archive| {
                json!({
                    "archive_id": archive.archive_id,
                    "source_session_id": archive.source_session_id,
                    "created_at": archive.created_at,
                    "theme": archive.theme,
                    "gist": archive.gist,
                    "compact_memory": archive.compact_memory,
                    "tags": archive.tags,
                    "links": archive.links,
                    "topic_thread": archive.topic_thread,
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
                    if fact.scope != scope_filter {
                        return None;
                    }
                    Some(json!({
                        "category": category_name,
                        "core_fact_id": fact.core_fact_id,
                        "updated_at": fact.updated_at,
                        "text": fact.text,
                        "status": fact.status,
                        "confidence": fact.confidence,
                        "tags": fact.tags,
                    }))
                })
            })
            .collect::<Vec<_>>();

        let existing_candidates = self
            .reflection_candidates_for_scope_unlocked(&core_scope, source_session_ids)?
            .into_iter()
            .map(|candidate| {
                json!({
                    "candidate_id": candidate.candidate_id,
                    "updated_at": candidate.updated_at,
                    "source_session_id": candidate.source_session_id,
                    "source_session_ids": candidate.source_session_ids,
                    "text": candidate.text,
                    "category": candidate.category,
                    "status": candidate.status,
                    "confidence": candidate.confidence,
                    "source_memory_unit_ids": candidate.source_memory_unit_ids,
                    "supporting_archive_ids": candidate.supporting_archive_ids,
                    "evidence_summary": candidate.evidence_summary,
                    "tags": candidate.tags,
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "schema_version": "reflection_task.v1",
            "source_session_id": session_id,
            "source_session_ids": evidence_session_ids,
            "personality_session_count": source_session_ids.len(),
            "core_scope": core_scope,
            "new_memory_unit_ids": new_memory_unit_ids,
            "related_memory_unit_ids": related_memory_unit_ids,
            "memory_units": memory_units,
            "archives": archive_summaries,
            "core_facts": core_facts,
            "existing_candidates": existing_candidates,
            "rules": {
                "agents_do_not_write_core": true,
                "first_iteration_requires_manual_review": true,
                "use_only_validated_memory_units": true,
                "trigger_units_are_new_evidence": true,
                "related_history_is_context_not_a_new_event": true,
                "avoid_duplicate_candidates": true,
                "reinforce_existing_candidates_with_new_evidence": true,
                "core_challenges_require_manual_review": true,
                "structured_no_insight_required_when_actions_are_empty": true
            }
        }))
    }

    fn ensure_reflection_membership_unlocked(
        &self,
        session_id: &str,
        reflection_scope: &str,
    ) -> Result<ReflectionState> {
        let now = now_rfc3339()?;
        let existing = self.storage.read_reflection_state(reflection_scope)?;
        let source_session_ids = reflection_session_ids(existing.as_ref(), session_id);
        let mut state = existing.unwrap_or(ReflectionState {
            schema_version: REFLECTION_STATE_SCHEMA_VERSION.to_string(),
            reflection_scope: reflection_scope.to_string(),
            source_session_ids: Vec::new(),
            source_session_id: session_id.to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
            last_completed_at: None,
            last_task_id: None,
            last_trigger_reason: None,
            last_candidate_count: 0,
        });
        let changed =
            state.source_session_ids != source_session_ids || state.source_session_id != session_id;
        state.source_session_ids = source_session_ids;
        state.source_session_id = session_id.to_string();
        if changed {
            state.updated_at = now;
            self.storage.write_reflection_state(&state)?;
        }
        Ok(state)
    }

    fn reflection_unit_contexts_for_sessions_unlocked(
        &self,
        source_session_ids: &[String],
    ) -> Result<Vec<ReflectionUnitContext>> {
        let allowed_sessions = source_session_ids.iter().cloned().collect::<HashSet<_>>();
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_items()
            .into_iter()
            .filter(|archive| allowed_sessions.contains(&archive.source_session_id))
            .filter(|archive| archive.status == ArchiveStatus::Complete)
            .collect::<Vec<_>>();
        let mut contexts = Vec::new();
        for archive in archives {
            for unit in self
                .storage
                .read_memory_units_for_archive(&archive.archive_id)?
                .into_items()
            {
                if unit.source_session_id != archive.source_session_id
                    || !allowed_sessions.contains(&unit.source_session_id)
                    || unit.status != MemoryUnitStatus::ActiveArchive
                    || unit.fidelity_status != FidelityStatus::Valid
                {
                    continue;
                }
                contexts.push(ReflectionUnitContext {
                    unit,
                    archive: archive.clone(),
                });
            }
        }
        contexts.sort_by(|left, right| {
            right
                .unit
                .weight
                .total_cmp(&left.unit.weight)
                .then_with(|| right.unit.created_at.cmp(&left.unit.created_at))
                .then_with(|| left.unit.memory_unit_id.cmp(&right.unit.memory_unit_id))
        });
        Ok(contexts)
    }

    pub(super) fn reflection_memory_units_for_sessions_unlocked(
        &self,
        source_session_ids: &[String],
    ) -> Result<Vec<MemoryUnit>> {
        Ok(self
            .reflection_unit_contexts_for_sessions_unlocked(source_session_ids)?
            .into_iter()
            .map(|context| context.unit)
            .collect())
    }

    fn reflection_candidates_for_scope_unlocked(
        &self,
        core_scope: &Option<String>,
        source_session_ids: &[String],
    ) -> Result<Vec<CandidateBelief>> {
        let allowed_sessions = source_session_ids.iter().cloned().collect::<HashSet<_>>();
        let mut candidates = self
            .storage
            .read_candidate_beliefs()?
            .into_items()
            .into_iter()
            .filter(|candidate| {
                matches!(
                    candidate.status,
                    CandidateStatus::Draft | CandidateStatus::ReadyForReview
                )
            })
            .filter(|candidate| {
                let belongs_to_bound_session = candidate
                    .source_session_id
                    .as_ref()
                    .is_some_and(|session| allowed_sessions.contains(session))
                    || candidate
                        .source_session_ids
                        .iter()
                        .any(|session| allowed_sessions.contains(session));
                match core_scope {
                    Some(scope) => {
                        candidate.core_scope.as_deref() == Some(scope.as_str())
                            || (candidate.core_scope.is_none() && belongs_to_bound_session)
                    }
                    None => candidate.core_scope.is_none() && belongs_to_bound_session,
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        candidates.truncate(self.options.reflection.max_existing_candidates);
        Ok(candidates)
    }

    fn candidate_from_reflection_draft(
        &self,
        source_session_id: &str,
        core_scope: Option<String>,
        draft: ReflectionCandidateDraft,
        known_evidence: &ReflectionKnownEvidence<'_>,
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
                .filter(|unit_id| known_evidence.unit_sessions.contains_key(unit_id))
                .collect(),
        );
        if source_memory_unit_ids.is_empty() {
            return Ok(None);
        }

        let source_session_ids = unique_strings(
            source_memory_unit_ids
                .iter()
                .filter_map(|unit_id| known_evidence.unit_sessions.get(unit_id).cloned())
                .collect(),
        );
        let supporting_archive_ids = unique_strings(
            draft
                .supporting_archive_ids
                .into_iter()
                .filter(|archive_id| known_evidence.archive_ids.contains(archive_id))
                .collect(),
        );
        let contradicting_archive_ids = unique_strings(
            draft
                .contradicting_archive_ids
                .into_iter()
                .filter(|archive_id| known_evidence.archive_ids.contains(archive_id))
                .collect(),
        );
        let contradicted_core_fact_ids = unique_strings(
            draft
                .contradicted_core_fact_ids
                .into_iter()
                .filter(|fact_id| known_evidence.core_fact_ids.contains(fact_id))
                .collect(),
        );
        let confidence = draft.confidence.clamp(0.0, 1.0);
        let min_sources_met = source_memory_unit_ids.len() >= 2;
        Ok(Some(CandidateBelief {
            schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            candidate_id: new_id("candidate")?,
            source_session_id: Some(source_session_id.to_string()),
            source_session_ids,
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
                min_sources_met,
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
            reinforcements: Vec::new(),
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_candidate_reinforcement(
        &self,
        candidate_id: &str,
        confidence: f64,
        evidence_summary: &str,
        source_memory_unit_ids: Vec<String>,
        supporting_archive_ids: Vec<String>,
        tags: Vec<String>,
        known_evidence: &ReflectionKnownEvidence<'_>,
        now: &str,
    ) -> Result<(CandidateBelief, String)> {
        let mut candidate = self.storage.read_candidate_belief(candidate_id)?;
        let previous_updated_at = candidate.updated_at.clone();
        if !matches!(
            candidate.status,
            CandidateStatus::Draft | CandidateStatus::ReadyForReview
        ) {
            return Err(MemoryEngineError::Validation(format!(
                "only draft or ready-for-review candidates can be reinforced: {candidate_id}"
            )));
        }

        let evidence_summary = normalize_whitespace(evidence_summary);
        if evidence_summary.is_empty() {
            return Err(MemoryEngineError::Validation(
                "candidate reinforcement evidence_summary must not be empty".to_string(),
            ));
        }
        let previous_unit_ids = candidate
            .source_memory_unit_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let new_unit_ids = unique_strings(
            source_memory_unit_ids
                .into_iter()
                .filter(|unit_id| known_evidence.unit_sessions.contains_key(unit_id))
                .filter(|unit_id| !previous_unit_ids.contains(unit_id))
                .collect(),
        );
        if new_unit_ids.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "candidate reinforcement must cite at least one new task memory unit: {candidate_id}"
            )));
        }
        let source_session_ids = unique_strings(
            new_unit_ids
                .iter()
                .filter_map(|unit_id| known_evidence.unit_sessions.get(unit_id).cloned())
                .collect(),
        );
        let supporting_archive_ids = unique_strings(
            supporting_archive_ids
                .into_iter()
                .filter(|archive_id| known_evidence.archive_ids.contains(archive_id))
                .collect(),
        );
        let tags = unique_strings(tags);
        let confidence = confidence.clamp(0.0, 1.0);
        candidate.updated_at = now.to_string();
        candidate.confidence = candidate.confidence.max(confidence);
        candidate.source_session_ids = unique_strings(
            candidate
                .source_session_ids
                .into_iter()
                .chain(source_session_ids.iter().cloned())
                .collect(),
        );
        candidate.source_memory_unit_ids = unique_strings(
            candidate
                .source_memory_unit_ids
                .into_iter()
                .chain(new_unit_ids.iter().cloned())
                .collect(),
        );
        candidate.supporting_archive_ids = unique_strings(
            candidate
                .supporting_archive_ids
                .into_iter()
                .chain(supporting_archive_ids.iter().cloned())
                .collect(),
        );
        candidate.tags = unique_strings(
            candidate
                .tags
                .into_iter()
                .chain(tags.iter().cloned())
                .collect(),
        );
        candidate.promotion_checks.min_sources_met = candidate.source_memory_unit_ids.len() >= 2;
        candidate.promotion_checks.weight_threshold_met = candidate.confidence >= 0.70;
        candidate.reinforcements.push(CandidateReinforcement {
            reinforcement_id: new_id("reinforcement")?,
            created_at: now.to_string(),
            source_session_ids,
            source_memory_unit_ids: new_unit_ids,
            supporting_archive_ids,
            evidence_summary,
            confidence,
            tags,
        });
        Ok((candidate, previous_updated_at))
    }

    pub(super) fn mark_reflection_task_failed_best_effort(&self, task_id: &str, detail: String) {
        if let Ok(mut task) = self.storage.load_task(task_id) {
            if task.task_type == TaskType::ReflectionAnalyze
                && matches!(task.state, TaskState::Pending | TaskState::Submitted)
            {
                task.state = TaskState::Failed;
                task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
                task.last_error = Some(detail);
                let _ = self.storage.save_task(&task);
            }
        }
    }
}

fn reflection_session_ids(
    state: Option<&ReflectionState>,
    current_session_id: &str,
) -> Vec<String> {
    let mut sessions = BTreeSet::new();
    if let Some(state) = state {
        sessions.extend(
            state
                .source_session_ids
                .iter()
                .map(|session| normalize_whitespace(session))
                .filter(|session| !session.is_empty()),
        );
        let legacy_session = normalize_whitespace(&state.source_session_id);
        if !legacy_session.is_empty() {
            sessions.insert(legacy_session);
        }
    }
    let current_session = normalize_whitespace(current_session_id);
    if !current_session.is_empty() {
        sessions.insert(current_session);
    }
    sessions.into_iter().collect()
}

fn reflection_unit_input(
    context: &ReflectionUnitContext,
    evidence_role: &str,
    relation_score: Option<usize>,
    relation_reasons: &[String],
) -> Value {
    json!({
        "memory_unit_id": context.unit.memory_unit_id,
        "archive_id": context.unit.archive_id,
        "source_session_id": context.unit.source_session_id,
        "created_at": context.unit.created_at,
        "updated_at": context.unit.updated_at,
        "thesis": context.unit.thesis,
        "evidence": context.unit.evidence,
        "tags": context.unit.tags,
        "weight": context.unit.weight,
        "fidelity_status": context.unit.fidelity_status,
        "reflection_count": context.unit.reflection_count,
        "evidence_role": evidence_role,
        "relation_score": relation_score,
        "relation_reasons": relation_reasons,
    })
}

fn select_related_reflection_units(
    contexts: &[ReflectionUnitContext],
    trigger_contexts: &[ReflectionUnitContext],
    limit: usize,
    min_score: usize,
) -> Vec<RelatedReflectionUnit> {
    if limit == 0 || trigger_contexts.is_empty() {
        return Vec::new();
    }
    let trigger_ids = trigger_contexts
        .iter()
        .map(|context| context.unit.memory_unit_id.as_str())
        .collect::<HashSet<_>>();
    let trigger_features = trigger_contexts
        .iter()
        .flat_map(reflection_unit_features)
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    if trigger_features.is_empty() {
        return Vec::new();
    }

    let mut related = contexts
        .iter()
        .filter(|context| !trigger_ids.contains(context.unit.memory_unit_id.as_str()))
        .filter(|context| context.unit.reflection_count > 0)
        .filter_map(|context| {
            let matches = reflection_unit_features(context)
                .into_iter()
                .filter(|(key, _)| trigger_features.contains(key))
                .map(|(_, value)| value)
                .collect::<Vec<_>>();
            let relation_score = matches.iter().map(|(weight, _)| *weight).sum::<usize>();
            if relation_score < min_score {
                return None;
            }
            let relation_reasons =
                unique_strings(matches.into_iter().map(|(_, reason)| reason).collect());
            Some(RelatedReflectionUnit {
                context: context.clone(),
                relation_score,
                relation_reasons,
            })
        })
        .collect::<Vec<_>>();
    related.sort_by(|left, right| {
        right
            .relation_score
            .cmp(&left.relation_score)
            .then_with(|| {
                right
                    .context
                    .unit
                    .weight
                    .total_cmp(&left.context.unit.weight)
            })
            .then_with(|| {
                right
                    .context
                    .unit
                    .created_at
                    .cmp(&left.context.unit.created_at)
            })
            .then_with(|| {
                left.context
                    .unit
                    .memory_unit_id
                    .cmp(&right.context.unit.memory_unit_id)
            })
    });
    related.truncate(limit);
    related
}

fn reflection_unit_features(context: &ReflectionUnitContext) -> HashMap<String, (usize, String)> {
    let mut features = HashMap::new();
    for tag in context.unit.tags.iter().chain(context.archive.tags.iter()) {
        insert_reflection_feature(&mut features, "tag", tag, REFLECTION_RELATION_TAG_WEIGHT);
    }
    if let Some(theme) = context.archive.theme.as_deref() {
        insert_reflection_feature(
            &mut features,
            "theme",
            theme,
            REFLECTION_RELATION_THEME_WEIGHT,
        );
    }
    for topic in &context.archive.topic_thread {
        insert_reflection_feature(
            &mut features,
            "topic",
            &topic.topic,
            REFLECTION_RELATION_TOPIC_WEIGHT,
        );
        for subtopic in &topic.subtopics {
            insert_reflection_feature(
                &mut features,
                "topic",
                subtopic,
                REFLECTION_RELATION_TOPIC_WEIGHT,
            );
        }
    }
    for link in &context.archive.links {
        let link_value = format!("{}:{}", link.kind, link.target);
        insert_reflection_feature(
            &mut features,
            "link",
            &link_value,
            REFLECTION_RELATION_LINK_WEIGHT,
        );
    }
    features
}

fn insert_reflection_feature(
    features: &mut HashMap<String, (usize, String)>,
    kind: &str,
    value: &str,
    weight: usize,
) {
    let normalized = normalize_match_text(value);
    if normalized.is_empty() {
        return;
    }
    let key = format!("{kind}:{normalized}");
    let reason = format!("shared_{kind}:{normalized}");
    features
        .entry(key)
        .and_modify(|existing| {
            if weight > existing.0 {
                *existing = (weight, reason.clone());
            }
        })
        .or_insert((weight, reason));
}

fn validate_reflection_session_id(session_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "reflection session_id must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn reflection_apply_result_digest(
    source_session_id: &str,
    core_scope: &Option<String>,
    actions: &[ReflectionActionDraft],
    no_insight: &Option<ReflectionNoInsight>,
    strict_typed_actions: bool,
) -> Result<String> {
    digest_json(&(
        source_session_id,
        core_scope,
        actions,
        no_insight,
        strict_typed_actions,
    ))
}

fn candidate_review_input_digest(
    candidate_id: &str,
    review: &ReviewRecord,
    requested_scope: &Option<String>,
) -> Result<String> {
    digest_json(&(
        candidate_id,
        review.reviewed_by.as_str(),
        review.decision,
        review.note.as_deref(),
        requested_scope,
    ))
}

fn digest_json<T: Serialize>(value: &T) -> Result<String> {
    let canonical = serde_json::to_vec(value)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn validate_reflection_config(config: &ReflectionConfig) -> Result<()> {
    if !config.salience_threshold.is_finite() || config.salience_threshold <= 0.0 {
        return Err(MemoryEngineError::Validation(
            "reflection salience_threshold must be finite and greater than zero".to_string(),
        ));
    }
    if !config.critical_unit_weight.is_finite()
        || !(0.0..=1.0).contains(&config.critical_unit_weight)
    {
        return Err(MemoryEngineError::Validation(
            "reflection critical_unit_weight must be finite and between 0.0 and 1.0".to_string(),
        ));
    }
    if !config.max_interval_days.is_finite() || config.max_interval_days <= 0.0 {
        return Err(MemoryEngineError::Validation(
            "reflection max_interval_days must be finite and greater than zero".to_string(),
        ));
    }
    if config.scheduled_min_units == 0
        || config.max_trigger_units == 0
        || config.max_related_units == 0
        || config.min_related_score == 0
        || config.max_existing_candidates == 0
    {
        return Err(MemoryEngineError::Validation(
            "reflection unit limits must be greater than zero".to_string(),
        ));
    }
    if config.prompt_id.trim().is_empty() || config.prompt_version == 0 {
        return Err(MemoryEngineError::Validation(
            "reflection prompt id must not be empty and version must be greater than zero"
                .to_string(),
        ));
    }
    Ok(())
}

fn reflection_scope_id(session_id: &str, core_scope: Option<&str>) -> String {
    normalize_optional_string(core_scope).unwrap_or_else(|| session_id.to_string())
}

fn reflection_lock_key(scope: &str) -> String {
    format!("reflection:{scope}")
}
