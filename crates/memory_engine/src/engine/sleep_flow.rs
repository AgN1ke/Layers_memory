use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn sleep(&self, session_id: &str) -> Result<SleepStage1Result> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
            self.sleep_stage1_unlocked(session_id)
        })
    }

    pub fn begin_sleep_run(&self, session_id: &str) -> Result<SleepRun> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_resource_lock(session_lock_key(session_id), || {
            let sleep_result = self.sleep_stage1_unlocked(session_id)?;
            let run = sleep_run_from_stage1(sleep_result)?;
            self.storage.save_sleep_run(&run)?;
            Ok(run)
        })
    }

    pub(super) fn sleep_stage1_unlocked(&self, session_id: &str) -> Result<SleepStage1Result> {
        let session = self.storage.read_session(session_id)?;
        if session.events.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "session has no events: {session_id}"
            )));
        }

        let archived_event_ids =
            self.archived_event_ids_for_session_unlocked(&session.metadata.session_id)?;
        let unarchived_events = session
            .events
            .iter()
            .filter(|event| !archived_event_ids.contains(&event.event_id))
            .collect::<Vec<_>>();
        if unarchived_events.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "session has no unarchived events: {session_id}"
            )));
        }

        let compactable_events = compactable_sleep_events(&unarchived_events, &self.options.sleep);
        let selected_events = select_sleep_events(&compactable_events, &self.options.sleep);
        let now = now_rfc3339()?;
        let archive_id = new_id("archive")?;
        let archive_entry =
            build_preliminary_archive(&session, &selected_events, &archive_id, &now);
        self.storage.write_archive_entry(&archive_entry)?;

        let pending_task = build_sleep_compression_task(
            &session,
            &selected_events,
            &archive_entry,
            &self.options.sleep,
            &now,
        )?;
        let memory_unit_task =
            build_memory_unit_task(&session, &selected_events, &archive_entry, &now)?;
        self.storage.save_task(&pending_task)?;
        self.storage.save_task(&memory_unit_task)?;

        Ok(SleepStage1Result {
            archive_entry,
            pending_task,
            memory_unit_task: Some(memory_unit_task),
            compact_memory_task: None,
        })
    }

    pub fn next_sleep_batch(&self, mut run: SleepRun) -> Result<SleepRunStep> {
        validate_sleep_run(&run)?;
        advance_sleep_run_stage(&mut run)?;
        self.storage.save_sleep_run(&run)?;

        let requests = run
            .requests
            .iter()
            .filter(|state| !state.completed && state_stage(state.track) == run.stage)
            .map(|state| state.request.clone())
            .collect::<Vec<_>>();

        let batch = (!requests.is_empty()).then_some(LlmBatch { requests });
        Ok(SleepRunStep { run, batch })
    }

    pub fn submit_sleep_batch(
        &self,
        mut run: SleepRun,
        responses: Vec<LlmResponse>,
    ) -> Result<SleepRunStep> {
        validate_sleep_run(&run)?;

        for response in responses {
            let request_id = llm_response_request_id(&response).to_string();
            let Some(index) = run
                .requests
                .iter()
                .position(|state| state.request.request_id == request_id)
            else {
                return Err(MemoryEngineError::Validation(format!(
                    "LLM response does not match any request in sleep run: {request_id}"
                )));
            };

            let mut state = run.requests[index].clone();
            if state.completed {
                continue;
            }
            state.attempts += 1;
            handle_sleep_response(&mut run, &mut state, response)?;
            run.requests[index] = state;
        }

        self.storage.save_sleep_run(&run)?;
        self.next_sleep_batch(run)
    }

    pub fn finish_sleep_run(&self, mut run: SleepRun) -> Result<SleepOutcome> {
        validate_sleep_run(&run)?;
        advance_sleep_run_stage(&mut run)?;
        self.storage.save_sleep_run(&run)?;
        if run.stage != SleepRunStage::ReadyToFinish {
            return Err(MemoryEngineError::Validation(
                "sleep run is not ready to finish".to_string(),
            ));
        }

        let session_id = run.session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            let mut sleep_result = assemble_sleep_compression_from_tracks(&run)?;
            if let Some(gist) = run
                .consolidator_gist
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sleep_result.gist = gist.to_string();
            }
            if let Some(narrative) = run
                .consolidator_narrative
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sleep_result.narrative = narrative.to_string();
            }
            apply_sleep_run_tags(&mut sleep_result, &run);

            let mut archive_entry =
                self.resume_sleep_compression_unlocked(&run.sleep_task_id, sleep_result)?;

            if let Some(memory_unit_task_id) = run.memory_unit_task_id.clone() {
                let memory_unit_result = run
                    .memory_unit_result
                    .clone()
                    .map(serde_json::from_value::<MemoryUnitPassResult>)
                    .transpose()?
                    .unwrap_or_else(|| empty_memory_unit_result(&run.archive_id));
                archive_entry =
                    self.resume_memory_unit_pass(&memory_unit_task_id, memory_unit_result)?;
            }

            let core_summary = self.apply_archive_personal_signal_bridge(&archive_entry)?;
            let fidelity_requests = self.auto_route_memory_fidelity_requests(&archive_entry)?;
            let covered_event_ids = self
                .record_completed_archive_in_session_metadata_unlocked(&session_id, &archive_entry)?
                .into_iter()
                .collect::<Vec<_>>();
            let _ = self
                .storage
                .rotate_session_events(&session_id, &covered_event_ids)?;
            self.flush_recall_stats()?;
            run.stage = SleepRunStage::Finished;
            run.completion_mode = Some(
                run.completion_mode
                    .clone()
                    .unwrap_or_else(|| "consolidated".to_string()),
            );
            self.storage.save_sleep_run(&run)?;

            Ok(SleepOutcome {
                archive_entry,
                core_summary,
                fidelity_requests,
                failed_passes: run.failed_passes.clone(),
                completion_mode: run
                    .completion_mode
                    .clone()
                    .unwrap_or_else(|| "consolidated".to_string()),
            })
        })
    }

    pub fn pending_sleep_runs(&self) -> Result<Vec<SleepRun>> {
        self.ensure_manifest()?;
        let mut runs = self
            .storage
            .load_sleep_runs()?
            .into_items()
            .into_iter()
            .filter(|run| run.stage != SleepRunStage::Finished)
            .collect::<Vec<_>>();
        runs.sort_by(|left, right| left.sleep_task_id.cmp(&right.sleep_task_id));
        Ok(runs)
    }

    pub fn cancel_sleep_run(&self, sleep_task_id: &str) -> Result<SleepRun> {
        let run = self.storage.load_sleep_run(sleep_task_id)?;
        validate_sleep_run(&run)?;
        let session_id = run.session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            let mut run = self.storage.load_sleep_run(sleep_task_id)?;
            validate_sleep_run(&run)?;
            cancel_task_if_active(&self.storage, &run.sleep_task_id, "sleep run cancelled")?;
            if let Some(memory_unit_task_id) = run.memory_unit_task_id.as_deref() {
                cancel_task_if_active(&self.storage, memory_unit_task_id, "sleep run cancelled")?;
            }
            run.stage = SleepRunStage::Finished;
            run.completion_mode = Some("cancelled".to_string());
            self.storage.save_sleep_run(&run)?;
            Ok(run)
        })
    }

    pub fn seed_core_from_archives(&self) -> Result<CoreArchiveSeedSummary> {
        self.ensure_manifest()?;
        let mut summary = CoreArchiveSeedSummary::default();
        let archives = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_items();
        for archive in archives
            .into_iter()
            .filter(|archive| archive.status == ArchiveStatus::Complete)
        {
            summary.archives += 1;
            let archive_summary = self
                .with_resource_lock(session_lock_key(&archive.source_session_id), || {
                    self.apply_archive_personal_signal_bridge(&archive)
                })?;
            summary.created += archive_summary.created;
            summary.updated += archive_summary.updated;
            summary.skipped += archive_summary.skipped;
        }
        Ok(summary)
    }

    pub(super) fn apply_archive_personal_signal_bridge(
        &self,
        archive: &ArchiveEntry,
    ) -> Result<CoreSignalSummary> {
        let mut summary = CoreSignalSummary::default();
        if archive.status != ArchiveStatus::Complete {
            return Ok(summary);
        }

        let session_events = self.read_session_events_with_archived(&archive.source_session_id)?;
        let user_event_ids = session_events
            .iter()
            .filter(|event| event.event_type == "user_message")
            .map(|event| event.event_id.clone())
            .collect::<HashSet<_>>();
        let scope = Some(archive.source_session_id.as_str());

        for signal in &archive.personal_signals {
            let text = normalize_whitespace(&signal.text);
            let category = normalize_category_name(&signal.category);
            let has_user_source = !user_event_ids.is_empty()
                && signal
                    .source_event_ids
                    .iter()
                    .any(|event_id| user_event_ids.contains(event_id));

            if text.is_empty()
                || category.is_empty()
                || signal.confidence < 0.85
                || !has_user_source
            {
                summary.skipped += 1;
                continue;
            }

            let result = self.with_resource_lock(core_lock_key(&category), || {
                if self.is_near_duplicate_core_fact(&category, scope, &text)? {
                    return Ok(None);
                }
                self.upsert_core_fact_unlocked(
                    CoreFactInput {
                        schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                        category: category.clone(),
                        scope: scope.map(str::to_string),
                        text,
                        confidence: signal.confidence,
                        tags: vec![
                            "archive_signal".to_string(),
                            "signal_category".to_string(),
                            format!("signal_category:{category}"),
                        ],
                        source_archive_ids: vec![archive.archive_id.clone()],
                        source_candidate_id: None,
                    },
                    category.clone(),
                )
                .map(Some)
            })?;
            let Some(result) = result else {
                summary.skipped += 1;
                continue;
            };
            if result.created {
                summary.created += 1;
            } else {
                summary.updated += 1;
            }
        }

        Ok(summary)
    }

    pub(super) fn auto_route_memory_fidelity_requests(
        &self,
        archive: &ArchiveEntry,
    ) -> Result<Vec<LlmRequest>> {
        if !self.options.fidelity.auto_validate_after_sleep
            || archive.status != ArchiveStatus::Complete
            || archive.memory_units.is_empty()
        {
            return Ok(Vec::new());
        }

        let core_path_event_ids = self.core_path_signal_event_ids(archive)?;
        let mut requests = Vec::new();
        for unit in &archive.memory_units {
            if !self.should_auto_validate_memory_unit(unit, &core_path_event_ids) {
                continue;
            }
            if self.pending_fidelity_task_exists_unlocked(&unit.memory_unit_id)? {
                continue;
            }
            let start = self.begin_memory_fidelity_pass_unlocked(&unit.memory_unit_id)?;
            requests.push(start.request);
        }
        Ok(requests)
    }

    pub(super) fn should_auto_validate_memory_unit(
        &self,
        unit: &MemoryUnit,
        core_path_event_ids: &HashSet<String>,
    ) -> bool {
        if unit.status != MemoryUnitStatus::ActiveArchive
            || unit.fidelity_status != FidelityStatus::Unchecked
        {
            return false;
        }

        if unit.weight >= self.options.fidelity.auto_validate_weight_threshold {
            return true;
        }

        if unit
            .source_event_ids
            .iter()
            .any(|id| core_path_event_ids.contains(id))
        {
            return true;
        }

        unit.tags.iter().any(|tag| {
            let tag = normalize_category_name(tag);
            self.options
                .fidelity
                .auto_validate_tags
                .iter()
                .any(|configured| configured == &tag)
        })
    }

    pub(super) fn core_path_signal_event_ids(
        &self,
        archive: &ArchiveEntry,
    ) -> Result<HashSet<String>> {
        if archive.personal_signals.is_empty() {
            return Ok(HashSet::new());
        }
        let session_events = self.read_session_events_with_archived(&archive.source_session_id)?;
        let user_event_ids = session_events
            .iter()
            .filter(|event| event.event_type == "user_message")
            .map(|event| event.event_id.clone())
            .collect::<HashSet<_>>();
        if user_event_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let mut ids = HashSet::new();
        for signal in &archive.personal_signals {
            let text = normalize_whitespace(&signal.text);
            let category = normalize_category_name(&signal.category);
            let has_user_source = signal
                .source_event_ids
                .iter()
                .any(|event_id| user_event_ids.contains(event_id));
            if text.is_empty()
                || category.is_empty()
                || signal.confidence < 0.85
                || !has_user_source
            {
                continue;
            }
            ids.extend(signal.source_event_ids.iter().cloned());
        }
        Ok(ids)
    }

    pub(super) fn pending_fidelity_task_exists_unlocked(
        &self,
        memory_unit_id: &str,
    ) -> Result<bool> {
        let tasks = self.storage.load_tasks()?.into_items();
        Ok(tasks.into_iter().any(|task| {
            task.task_type == TaskType::MemoryFidelityPass
                && task
                    .inputs
                    .get("memory_unit_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == memory_unit_id)
        }))
    }

    pub(super) fn is_near_duplicate_core_fact(
        &self,
        category_name: &str,
        scope: Option<&str>,
        text: &str,
    ) -> Result<bool> {
        let normalized_scope = normalize_optional_string(scope);
        let needle = normalize_match_text(text);
        let needle_tokens = meaningful_tokens(text);
        let category = self.storage.read_core_store_category(category_name)?;

        for fact in category.facts {
            if fact.status != CoreFactStatus::Active || fact.scope != normalized_scope {
                continue;
            }
            if normalize_match_text(&fact.text) == needle {
                return Ok(false);
            }
            if token_overlap_sets(&needle_tokens, &meaningful_tokens(&fact.text)) >= 0.55 {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub fn resume_sleep_compression(
        &self,
        task_id: &str,
        result: SleepCompressionResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;
        self.ensure_manifest()?;

        let archive_entry = self.storage.read_archive_entry_by_id(&result.archive_id)?;
        let session_id = archive_entry.source_session_id.clone();
        self.with_resource_lock(session_lock_key(&session_id), || {
            self.resume_sleep_compression_unlocked(task_id, result)
        })
    }

    pub(super) fn resume_sleep_compression_unlocked(
        &self,
        task_id: &str,
        result: SleepCompressionResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;
        self.ensure_manifest()?;

        let mut task = self.storage.load_task(task_id)?;

        if task.task_type != TaskType::SleepCompression {
            return Err(MemoryEngineError::Validation(format!(
                "task is not sleep_compression: {task_id}"
            )));
        }

        let mut archive_entry = self.storage.read_archive_entry_by_id(&result.archive_id)?;

        let now = now_rfc3339()?;
        if archive_entry.status == ArchiveStatus::Complete {
            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            self.storage.save_task(&task)?;
            self.record_completed_archive_in_session_metadata_unlocked(
                &archive_entry.source_session_id,
                &archive_entry,
            )?;
            return Ok(archive_entry);
        }

        archive_entry.updated_at = now.clone();
        archive_entry.theme = result.theme;
        archive_entry.tags = result.tags;
        archive_entry.gist = result.gist;
        archive_entry.narrative = result.narrative;
        if result.compact_memory.is_some() {
            archive_entry.compact_memory = result.compact_memory;
        }
        archive_entry.facts = result.facts;
        archive_entry.quotes = result.quotes;
        archive_entry.weight = result.weight;
        archive_entry.links = result.links;
        archive_entry.emotional_markers = result.emotional_markers;
        archive_entry.topic_thread = result.topic_thread;
        archive_entry.personal_signals = result.personal_signals;
        archive_entry.relational_tone = result.relational_tone;
        archive_entry.status = ArchiveStatus::Complete;
        archive_entry.llm_enhanced = true;
        archive_entry.prompt_id = Some(task.prompt_id.clone());
        archive_entry.prompt_version = Some(task.prompt_version);

        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;
        self.record_completed_archive_in_session_metadata_unlocked(
            &archive_entry.source_session_id,
            &archive_entry,
        )?;

        Ok(archive_entry)
    }

    pub fn resume_compact_memory_pass(
        &self,
        task_id: &str,
        compact_memory: &str,
    ) -> Result<ArchiveEntry> {
        let compact_memory = normalize_optional_string(Some(compact_memory)).ok_or_else(|| {
            MemoryEngineError::Validation(
                "compact memory pass result must not be empty".to_string(),
            )
        })?;
        self.ensure_manifest()?;

        let mut task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::CompactMemoryPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not compact_memory_pass: {task_id}"
            )));
        }

        let archive_id = task
            .inputs
            .get("preliminary_archive_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "compact_memory_pass task has no preliminary_archive_id: {task_id}"
                ))
            })?;
        let mut archive_entry = self.storage.read_archive_entry_by_id(archive_id)?;

        let now = now_rfc3339()?;
        archive_entry.updated_at = now.clone();
        archive_entry.compact_memory = Some(compact_memory);
        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;

        Ok(archive_entry)
    }

    pub fn resume_memory_unit_pass(
        &self,
        task_id: &str,
        result: MemoryUnitPassResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != MEMORY_UNITS_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;
        self.ensure_manifest()?;

        let mut task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::MemoryUnitPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not memory_unit_pass: {task_id}"
            )));
        }

        let archive_id = task
            .inputs
            .get("preliminary_archive_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "memory_unit_pass task has no preliminary_archive_id: {task_id}"
                ))
            })?;
        if archive_id != result.archive_id {
            return Err(MemoryEngineError::Validation(format!(
                "memory_unit_pass archive_id mismatch: task={archive_id} result={}",
                result.archive_id
            )));
        }

        let mut archive_entry = self.storage.read_archive_entry_by_id(archive_id)?;
        let now = now_rfc3339()?;
        if task.state == TaskState::Completed || !archive_entry.memory_units.is_empty() {
            task.state = TaskState::Completed;
            task.updated_at = now;
            task.last_error = None;
            self.storage.save_task(&task)?;
            return Ok(archive_entry);
        }

        let mut units = Vec::new();
        for draft in result.memory_units {
            let thesis = normalize_whitespace(&draft.thesis);
            if thesis.is_empty() {
                continue;
            }
            units.push(MemoryUnit {
                schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
                memory_unit_id: new_id("mu")?,
                archive_id: archive_entry.archive_id.clone(),
                source_session_id: archive_entry.source_session_id.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
                thesis,
                source_event_ids: draft.source_event_ids,
                evidence: normalize_optional_string(draft.evidence.as_deref()),
                tags: draft.tags,
                weight: draft.weight,
                status: MemoryUnitStatus::ActiveArchive,
                fidelity_status: FidelityStatus::Unchecked,
                fidelity_review: None,
                forget_review: None,
            });
        }

        for unit in &units {
            self.storage.write_memory_unit(unit)?;
        }

        archive_entry.updated_at = now.clone();
        archive_entry.memory_units = units;
        archive_entry.compact_memory =
            render_compact_memory_from_units(&archive_entry.memory_units);
        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;

        Ok(archive_entry)
    }
}
