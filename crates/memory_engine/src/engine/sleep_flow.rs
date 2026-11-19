use super::transactional_support::serializable_digest;
use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn sleep(&self, session_id: &str) -> Result<SleepStage1Result> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_session_memory_write(session_id, || {
            if self.open_sleep_run_for_session(session_id)?.is_some() {
                return Err(MemoryEngineError::Validation(format!(
                    "session already has an open Sleep run; resume it with begin_sleep_run: {session_id}"
                )));
            }
            self.sleep_stage1_unlocked(session_id)
        })
    }

    pub fn begin_sleep_run(&self, session_id: &str) -> Result<SleepRun> {
        self.begin_sleep_run_with_options(session_id, SleepRunOptions::default())
    }

    pub fn begin_sleep_run_with_options(
        &self,
        session_id: &str,
        options: SleepRunOptions,
    ) -> Result<SleepRun> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_session_memory_write(session_id, || {
            if let Some(mut run) = self.open_sleep_run_for_session(session_id)? {
                self.recover_sleep_prompt_context(&mut run)?;
                return Ok(run);
            }
            let sleep_result = self.sleep_stage1_unlocked(session_id)?;
            let run = sleep_run_from_stage1_with_options(sleep_result, options)?;
            self.storage.save_sleep_run(&run)?;
            Ok(run)
        })
    }

    fn open_sleep_run_for_session(&self, session_id: &str) -> Result<Option<SleepRun>> {
        let runs = self
            .storage
            .load_sleep_runs()?
            .into_items()
            .into_iter()
            .filter(|run| run.session_id == session_id && run.stage != SleepRunStage::Finished)
            .collect();
        Ok(self
            .reconcile_sleep_runs_unlocked(session_id, runs)?
            .into_iter()
            .next())
    }

    fn reconcile_sleep_runs_unlocked(
        &self,
        session_id: &str,
        runs: Vec<SleepRun>,
    ) -> Result<Vec<SleepRun>> {
        if runs.is_empty() {
            return Ok(runs);
        }
        let (archives, covered) = self.sleep_archive_coverage_unlocked(session_id)?;
        let mut viable = Vec::new();
        for mut run in runs {
            validate_sleep_run(&run)?;
            let archive = sleep_run_archive(&run, &archives)?;
            if sleep_run_is_superseded(archive, &covered) {
                self.retire_superseded_sleep_run(&run, archive)?;
            } else {
                let recoverable = match self.recover_sleep_prompt_context(&mut run) {
                    Ok(()) => true,
                    Err(MemoryEngineError::NeedsNewReview) => false,
                    Err(error) => return Err(error),
                };
                viable.push((run, archive, recoverable));
            }
        }
        viable.sort_by(
            |(left, left_archive, left_recoverable), (right, right_archive, right_recoverable)| {
                right_recoverable
                    .cmp(left_recoverable)
                    .then_with(|| {
                        sleep_recovery_priority(right, right_archive)
                            .cmp(&sleep_recovery_priority(left, left_archive))
                    })
                    .then_with(|| left.sleep_task_id.cmp(&right.sleep_task_id))
            },
        );

        // Preserve disjoint legacy checkpoints so the host can finish already
        // paid work. Only the best progressed run of overlapping inputs goes
        // to the provider. An unverifiable run stays visible for explicit
        // cancellation, but next_sleep_batch will not issue a request for it.
        let mut claimed = HashSet::new();
        let mut selected = Vec::new();
        for (run, archive, _) in viable {
            if archive
                .source_event_ids
                .iter()
                .all(|id| !claimed.contains(id))
            {
                claimed.extend(archive.source_event_ids.iter().cloned());
                selected.push(run);
            }
        }
        Ok(selected)
    }

    fn sleep_archive_coverage_unlocked(
        &self,
        session_id: &str,
    ) -> Result<(Vec<ArchiveEntry>, HashSet<String>)> {
        let archives = self.storage.read_sleep_archives(session_id)?;
        let mut metadata = self.storage.read_session_metadata(session_id)?;
        let mut covered = metadata
            .archived_event_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut archive_ids = metadata
            .archived_to
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        // The Archive may already be durable after an interruption before the
        // session index was saved. The canonical records repair that gap.
        for archive in archives
            .iter()
            .filter(|entry| entry.status == ArchiveStatus::Complete)
        {
            covered.extend(archive.source_event_ids.iter().cloned());
            archive_ids.insert(archive.archive_id.clone());
        }
        let indexed_events = covered.iter().cloned().collect::<Vec<_>>();
        let indexed_archives = archive_ids.into_iter().collect::<Vec<_>>();
        if !metadata.archived_event_index_complete
            || metadata.archived_event_ids != indexed_events
            || metadata.archived_to != indexed_archives
        {
            metadata.archived_event_ids = indexed_events;
            metadata.archived_to = indexed_archives;
            metadata.archived_event_index_complete = true;
            self.storage.write_session_metadata(&metadata)?;
        }
        Ok((archives, covered.into_iter().collect()))
    }

    fn retire_superseded_sleep_run(&self, run: &SleepRun, archive: &ArchiveEntry) -> Result<()> {
        let reason = "sleep inputs are already covered in part or in full by another archive";
        if archive.status == ArchiveStatus::Preliminary {
            let mut superseded = archive.clone();
            superseded.status = ArchiveStatus::Superseded;
            superseded.updated_at = now_rfc3339()?;
            self.storage
                .update_archive_entry(&archive.archive_id, &superseded)?;
        }
        for task_id in
            std::iter::once(run.sleep_task_id.as_str()).chain(run.memory_unit_task_id.as_deref())
        {
            cancel_task_if_active(&self.storage, task_id, reason)?;
        }
        let mut finished = run.clone();
        finished.stage = SleepRunStage::Finished;
        finished.completion_mode = Some("superseded".to_string());
        self.storage.save_sleep_run(&finished)
    }

    fn require_current_sleep_coverage(&self, run: &SleepRun, verify_archive: bool) -> Result<()> {
        if !verify_archive {
            let covered = self.archived_event_ids_for_session_unlocked(&run.session_id)?;
            if run.shared_prompt_context.as_ref().is_some_and(|context| {
                !context.source_event_ids.is_empty()
                    && context
                        .source_event_ids
                        .iter()
                        .all(|id| !covered.contains(id))
            }) {
                return Ok(());
            }
        }
        let (archives, covered) = self.sleep_archive_coverage_unlocked(&run.session_id)?;
        let archive = sleep_run_archive(run, &archives)?;
        if sleep_run_is_superseded(archive, &covered) {
            self.retire_superseded_sleep_run(run, archive)?;
            return Err(MemoryEngineError::Validation(format!(
                "sleep run was superseded by committed archive coverage: {}",
                run.sleep_task_id
            )));
        }
        Ok(())
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

        let now = now_rfc3339()?;
        let archive_id = new_id("archive")?;
        let result = prepare_sleep_stage1(
            &session,
            &unarchived_events,
            &self.options.sleep,
            &now,
            archive_id,
            new_id("task")?,
            new_id("task")?,
        )?;
        self.storage.write_archive_entry(&result.archive_entry)?;
        self.storage.save_task(&result.pending_task)?;
        if let Some(memory_unit_task) = result.memory_unit_task.as_ref() {
            self.storage.save_task(memory_unit_task)?;
        }

        Ok(result)
    }

    fn current_sleep_run(&self, run: &SleepRun) -> Result<SleepRun> {
        let current = self.load_sleep_run_for_response(&run.sleep_task_id)?;
        validate_sleep_run(&current)?;
        if current.session_id != run.session_id
            || current.archive_id != run.archive_id
            || current.sleep_task_id != run.sleep_task_id
            || current.memory_unit_task_id != run.memory_unit_task_id
        {
            return Err(MemoryEngineError::Validation(
                "sleep run does not match its persisted identity".to_string(),
            ));
        }
        Ok(current)
    }

    fn require_open_sleep_run(&self, run: &SleepRun) -> Result<()> {
        if run.stage == SleepRunStage::Finished {
            return Err(MemoryEngineError::Validation(format!(
                "sleep run is already finished: {}",
                run.sleep_task_id
            )));
        }
        Ok(())
    }

    fn recover_sleep_prompt_context(&self, run: &mut SleepRun) -> Result<()> {
        match migrate_legacy_sleep_prompt_context(run, None, None) {
            Err(MemoryEngineError::NeedsNewReview) => {}
            result => return result,
        }

        let archive = self.storage.read_archive_entry_raw(&run.archive_id)?;
        match migrate_legacy_sleep_prompt_context(run, Some(&archive), None) {
            Err(MemoryEngineError::NeedsNewReview) => {}
            result => return result,
        }
        let task = match self.load_task_for_response(&run.sleep_task_id) {
            Ok(task) if task.task_type == TaskType::SleepCompression => task,
            Ok(_)
            | Err(MemoryEngineError::TaskNotFound(_))
            | Err(MemoryEngineError::OperationResultUnavailable) => {
                return Err(MemoryEngineError::NeedsNewReview);
            }
            Err(error) => return Err(error),
        };
        migrate_legacy_sleep_prompt_context(run, Some(&archive), Some(&task.inputs))
    }

    pub fn next_sleep_batch(&self, mut run: SleepRun) -> Result<SleepRunStep> {
        validate_sleep_run(&run)?;
        self.ensure_manifest()?;
        let session_id = run.session_id.clone();
        self.with_session_memory_write(&session_id, || {
            let mut current = self.current_sleep_run(&run)?;
            if current.stage == SleepRunStage::Finished && run.stage == SleepRunStage::Finished {
                return Ok(SleepRunStep {
                    run: current,
                    batch: None,
                });
            }
            self.require_open_sleep_run(&current)?;
            self.require_open_sleep_run(&run)?;
            self.require_current_sleep_coverage(&current, false)?;
            self.recover_sleep_prompt_context(&mut current)?;
            run.shared_prompt_context = current.shared_prompt_context;
            self.next_sleep_batch_unlocked(run)
        })
    }

    fn next_sleep_batch_unlocked(&self, mut run: SleepRun) -> Result<SleepRunStep> {
        migrate_legacy_sleep_prompt_context(&mut run, None, None)?;

        let batch = loop {
            advance_sleep_run_stage(&mut run)?;
            if matches!(
                run.stage,
                SleepRunStage::ReadyToFinish | SleepRunStage::Finished
            ) {
                break None;
            }

            let pending = run
                .requests
                .iter()
                .filter(|state| !state.completed && state_stage(state.track) == run.stage)
                .map(|state| state.request.clone())
                .collect::<Vec<_>>();
            if pending.is_empty() {
                continue;
            }

            let remaining = run
                .max_provider_calls
                .map(|limit| limit.saturating_sub(run.provider_calls_issued))
                .unwrap_or(u32::MAX) as usize;
            if remaining == 0 {
                run.budget_exhausted = true;
                fallback_unfinished_sleep_requests(&mut run, "budget_exhausted");
                continue;
            }

            let requests = pending.into_iter().take(remaining).collect::<Vec<_>>();
            run.provider_calls_issued = run
                .provider_calls_issued
                .saturating_add(requests.len() as u32);
            break Some(LlmBatch {
                shared_context: run.shared_prompt_context.clone(),
                requests,
            });
        };
        self.storage.save_sleep_run(&run)?;
        Ok(SleepRunStep { run, batch })
    }

    pub fn submit_sleep_batch(
        &self,
        mut run: SleepRun,
        responses: Vec<LlmResponse>,
    ) -> Result<SleepRunStep> {
        validate_sleep_run(&run)?;
        self.ensure_manifest()?;
        let session_id = run.session_id.clone();
        self.with_session_memory_write(&session_id, || {
            let mut current = self.current_sleep_run(&run)?;
            self.require_open_sleep_run(&current)?;
            self.require_open_sleep_run(&run)?;
            self.require_current_sleep_coverage(&current, false)?;
            self.recover_sleep_prompt_context(&mut current)?;
            run.shared_prompt_context = current.shared_prompt_context;
            migrate_legacy_sleep_prompt_context(&mut run, None, None)?;
            self.submit_sleep_batch_unlocked(run, responses)
        })
    }

    fn submit_sleep_batch_unlocked(
        &self,
        mut run: SleepRun,
        responses: Vec<LlmResponse>,
    ) -> Result<SleepRunStep> {
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
        self.next_sleep_batch_unlocked(run)
    }

    pub fn finish_sleep_run(&self, mut run: SleepRun) -> Result<SleepOutcome> {
        validate_sleep_run(&run)?;
        advance_sleep_run_stage(&mut run)?;
        if run.stage != SleepRunStage::ReadyToFinish {
            return Err(MemoryEngineError::Validation(
                "sleep run is not ready to finish".to_string(),
            ));
        }

        let session_id = run.session_id.clone();
        self.ensure_manifest()?;
        self.with_session_memory_write(&session_id, || {
            let mut current = self.current_sleep_run(&run)?;
            self.require_open_sleep_run(&current)?;
            self.require_current_sleep_coverage(&current, true)?;
            self.recover_sleep_prompt_context(&mut current)?;
            run.shared_prompt_context = current.shared_prompt_context;
            migrate_legacy_sleep_prompt_context(&mut run, None, None)?;
            self.storage.save_sleep_run(&run)?;
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
                self.resume_sleep_compression_unlocked(&run.sleep_task_id, sleep_result, false)?;

            if let Some(memory_unit_task_id) = run.memory_unit_task_id.clone() {
                let memory_unit_result = accepted_memory_unit_result(&run)?;
                let mut task = self.load_task_for_response(&memory_unit_task_id)?;
                let legacy_completion = task.state == TaskState::Completed
                    && task.outcome.is_none()
                    && !self.memory_unit_publication_exists(&run.archive_id)?;
                // Continue an interrupted legacy run from its existing
                // Archive without claiming to replay its unknown extraction.
                let accepted = memory_unit_result.filter(|_| !legacy_completion);
                if let Some(result) = accepted {
                    archive_entry = self.resume_memory_unit_pass_unlocked(&mut task, result)?;
                } else if task.state != TaskState::Completed {
                    // A failed or skipped pass has no accepted result. Keep
                    // repair available even if its local fallback was empty.
                    task.state = TaskState::Completed;
                    task.updated_at = now_rfc3339()?;
                    self.storage.save_task(&task)?;
                }
            }

            let core_summary = self.apply_archive_personal_signal_bridge(&archive_entry)?;
            let fidelity_requests = self.auto_route_memory_fidelity_requests(&archive_entry)?;
            let embedding_requests = self.pending_embedding_backfill_unlocked(
                archive_entry.effective_personality_scope_id(),
            )?;
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
                embedding_requests,
                failed_passes: run.failed_passes.clone(),
                skipped_passes: run.skipped_passes.clone(),
                pass_reports: sleep_pass_reports(&run),
                provider_calls_issued: run.provider_calls_issued,
                max_provider_calls: run.max_provider_calls,
                budget_exhausted: run.budget_exhausted,
                completion_mode: run
                    .completion_mode
                    .clone()
                    .unwrap_or_else(|| "consolidated".to_string()),
            })
        })
    }

    /// Finalizes a run with every result already obtained and deterministic
    /// fallbacks for the remainder. This is the safe host-side budget stop:
    /// paid work is retained and selected events still become archived.
    pub fn finish_sleep_run_with_available(&self, mut run: SleepRun) -> Result<SleepOutcome> {
        validate_sleep_run(&run)?;
        while !matches!(
            run.stage,
            SleepRunStage::ReadyToFinish | SleepRunStage::Finished
        ) {
            fallback_unfinished_sleep_requests(&mut run, "host_budget_stop");
            advance_sleep_run_stage(&mut run)?;
        }
        self.finish_sleep_run(run)
    }

    pub fn pending_sleep_runs(&self) -> Result<Vec<SleepRun>> {
        self.ensure_manifest()?;
        let mut by_session = std::collections::BTreeMap::<String, Vec<String>>::new();
        for run in self.storage.load_sleep_runs()?.into_items() {
            if run.stage != SleepRunStage::Finished {
                by_session
                    .entry(run.session_id)
                    .or_default()
                    .push(run.sleep_task_id);
            }
        }

        let mut runs = Vec::new();
        for (session_id, task_ids) in by_session {
            runs.extend(self.with_session_memory_write(&session_id, || {
                let mut current = Vec::new();
                for task_id in task_ids {
                    let run = match self.load_sleep_run_for_response(&task_id) {
                        Ok(run) => run,
                        Err(MemoryEngineError::OperationStale) => continue,
                        Err(error) => return Err(error),
                    };
                    if run.session_id == session_id && run.stage != SleepRunStage::Finished {
                        current.push(run);
                    }
                }
                self.reconcile_sleep_runs_unlocked(&session_id, current)
            })?);
        }
        runs.sort_by(|left, right| left.sleep_task_id.cmp(&right.sleep_task_id));
        Ok(runs)
    }

    pub fn pending_memory_unit_repairs(&self, session_id: &str) -> Result<Vec<LlmRequest>> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "repair session_id must not be empty".to_string(),
            ));
        }
        self.ensure_manifest()?;

        self.with_session_memory_write(session_id, || {
            let active_repair_archive_ids = self
                .storage
                .load_tasks()?
                .into_items()
                .into_iter()
                .filter(|task| {
                    task.task_type == TaskType::MemoryUnitPass
                        && matches!(task.state, TaskState::Pending | TaskState::Submitted)
                })
                .filter_map(|task| {
                    task.inputs
                        .get("preliminary_archive_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<HashSet<_>>();

            let mut archives = self
                .storage
                .read_archive(&ArchiveFilters::default())?
                .into_items()
                .into_iter()
                .filter(|archive| {
                    archive.source_session_id == session_id
                        && archive.status == ArchiveStatus::Complete
                        && archive.memory_units.is_empty()
                        && !archive.source_event_ids.is_empty()
                        && !active_repair_archive_ids.contains(&archive.archive_id)
                })
                .collect::<Vec<_>>();
            if archives.is_empty() {
                return Ok(Vec::new());
            }
            let repairable = self
                .storage
                .load_sleep_runs()?
                .into_items()
                .into_iter()
                .filter(|run| run.session_id == session_id && run.stage == SleepRunStage::Finished)
                .filter(|run| {
                    run.failed_passes
                        .iter()
                        .chain(&run.skipped_passes)
                        .any(|pass| pass == "memory_unit_pass")
                })
                .map(|run| run.archive_id)
                .collect::<HashSet<_>>();
            archives.retain(|archive| repairable.contains(&archive.archive_id));
            if archives.is_empty() {
                return Ok(Vec::new());
            }
            archives.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.archive_id.cmp(&right.archive_id))
            });

            let all_events = self.read_session_events_with_archived(session_id)?;
            let event_by_id = all_events
                .iter()
                .map(|event| (event.event_id.as_str(), event))
                .collect::<HashMap<_, _>>();
            let now = now_rfc3339()?;
            let mut requests = Vec::new();
            for archive in archives {
                if self.memory_unit_publication_exists(&archive.archive_id)? {
                    continue;
                }
                let events = archive
                    .source_event_ids
                    .iter()
                    .filter_map(|event_id| event_by_id.get(event_id.as_str()).copied())
                    .collect::<Vec<_>>();
                if events.len() != archive.source_event_ids.len() {
                    continue;
                }
                let task = build_memory_unit_repair_task(session_id, &events, &archive, &now)?;
                self.storage.save_task(&task)?;
                requests.push(llm_request_from_task(
                    &task,
                    "memory_unit_pass",
                    json!({ "sleep_task": task.inputs }),
                )?);
            }
            Ok(requests)
        })
    }

    pub fn submit_memory_unit_repair_response(
        &self,
        task_id: &str,
        response: LlmResponse,
    ) -> Result<Option<ArchiveEntry>> {
        if task_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "memory unit repair task_id must not be empty".to_string(),
            ));
        }
        self.with_archive_task_response(
            task_id,
            "preliminary_archive_id",
            TaskType::MemoryUnitPass,
            |mut task| self.submit_memory_unit_repair_unlocked(&mut task, response),
        )
    }

    fn submit_memory_unit_repair_unlocked(
        &self,
        task: &mut PendingTask,
        response: LlmResponse,
    ) -> Result<Option<ArchiveEntry>> {
        let archive_id = workflow_task_archive_id(task, "preliminary_archive_id")?.to_string();

        let result = match response {
            LlmResponse::Ok { text, .. } => {
                let parsed = parse_json_value_from_llm_text(&text)
                    .and_then(|value| {
                        serde_json::from_value::<MemoryUnitPassResult>(value).map_err(Into::into)
                    })
                    .map(|mut result| {
                        if result.schema_version.trim().is_empty() {
                            result.schema_version = MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string();
                        }
                        if result.archive_id != archive_id {
                            result.archive_id = archive_id.clone();
                        }
                        result
                    });
                match parsed {
                    Ok(result) => result,
                    Err(err) => {
                        self.mark_workflow_task_failed_best_effort(
                            task,
                            format!("memory unit repair parse failed: {err}"),
                        );
                        return Err(err);
                    }
                }
            }
            LlmResponse::Err { kind, detail, .. } => {
                if kind == LlmErrorKind::Transport {
                    self.mark_workflow_task_failed_best_effort(task, format!("{kind:?}: {detail}"));
                    return Ok(None);
                }
                return self
                    .repair_memory_units_from_archive_tracks_unlocked(
                        task,
                        &archive_id,
                        format!("{kind:?}: {detail}"),
                    )
                    .map(Some);
            }
        };

        self.resume_memory_unit_pass_unlocked(task, result)
            .map(Some)
    }

    fn repair_memory_units_from_archive_tracks_unlocked(
        &self,
        task: &mut PendingTask,
        archive_id: &str,
        failure_detail: String,
    ) -> Result<ArchiveEntry> {
        let archive_entry = self.storage.read_archive_entry_raw(archive_id)?;
        if archive_entry.status == ArchiveStatus::Superseded || task.state == TaskState::Cancelled {
            return Err(MemoryEngineError::Validation(format!(
                "memory unit task belongs to a retired sleep run: {}",
                task.task_id
            )));
        }
        let now = now_rfc3339()?;

        let mut units = Vec::new();
        let mut seen = HashSet::new();

        for signal in &archive_entry.personal_signals {
            if units.len() >= REPAIR_FALLBACK_MAX_UNITS {
                break;
            }
            push_repair_memory_unit(
                &mut units,
                &mut seen,
                &archive_entry,
                &now,
                RepairMemoryUnitInput {
                    thesis: &signal.text,
                    source_event_ids: signal.source_event_ids.clone(),
                    evidence: signal.evidence.clone(),
                    tags: vec![
                        "memory_unit_repair".to_string(),
                        "repair_from_personal_signal".to_string(),
                        format!("signal_category:{}", signal.category),
                    ],
                    weight: signal.confidence.clamp(0.55, 0.95),
                },
            )?;
        }

        for fact in &archive_entry.facts {
            if units.len() >= REPAIR_FALLBACK_MAX_UNITS {
                break;
            }
            push_repair_memory_unit(
                &mut units,
                &mut seen,
                &archive_entry,
                &now,
                RepairMemoryUnitInput {
                    thesis: &fact.text,
                    source_event_ids: fact.source_event_ids.clone(),
                    evidence: None,
                    tags: vec![
                        "memory_unit_repair".to_string(),
                        "repair_from_fact".to_string(),
                    ],
                    weight: fact.confidence.clamp(0.50, 0.90),
                },
            )?;
        }

        for item in &archive_entry.topic_thread {
            if units.len() >= REPAIR_FALLBACK_MAX_UNITS {
                break;
            }
            let thesis = item
                .summary
                .as_deref()
                .map(normalize_whitespace)
                .filter(|summary| !summary.is_empty())
                .unwrap_or_else(|| {
                    let mut parts = vec![item.topic.trim().to_string()];
                    parts.extend(
                        item.subtopics
                            .iter()
                            .map(|subtopic| subtopic.trim().to_string()),
                    );
                    normalize_whitespace(&parts.join(": "))
                });
            push_repair_memory_unit(
                &mut units,
                &mut seen,
                &archive_entry,
                &now,
                RepairMemoryUnitInput {
                    thesis: &thesis,
                    source_event_ids: item.source_event_ids.clone(),
                    evidence: None,
                    tags: vec![
                        "memory_unit_repair".to_string(),
                        "repair_from_topic_thread".to_string(),
                    ],
                    weight: archive_entry.weight.clamp(0.45, 0.75),
                },
            )?;
        }

        if units.is_empty() {
            push_repair_memory_unit(
                &mut units,
                &mut seen,
                &archive_entry,
                &now,
                RepairMemoryUnitInput {
                    thesis: &archive_entry.gist,
                    source_event_ids: archive_entry.source_event_ids.clone(),
                    evidence: Some(archive_entry.narrative.clone()),
                    tags: vec![
                        "memory_unit_repair".to_string(),
                        "repair_from_archive_gist".to_string(),
                    ],
                    weight: archive_entry.weight.clamp(0.40, 0.70),
                },
            )?;
        }

        self.publish_memory_units(
            task, archive_id, serializable_digest(&("repair_from_archive", archive_id))?, units,
            Some(format!("memory unit repair used archive-track fallback after LLM failure: {failure_detail}")),
        )
    }

    pub fn cancel_sleep_run(&self, sleep_task_id: &str) -> Result<SleepRun> {
        self.ensure_manifest()?;
        let run = self.load_sleep_run_for_response(sleep_task_id)?;
        validate_sleep_run(&run)?;
        let session_id = run.session_id.clone();
        self.with_session_memory_write(&session_id, || {
            let mut run = self.load_sleep_run_for_response(sleep_task_id)?;
            validate_sleep_run(&run)?;
            if run.stage == SleepRunStage::Finished {
                return Ok(run);
            }
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
        self.seed_core_from_archive_selection(None)
    }

    pub fn seed_core_from_archives_in_scope(&self, scope: &str) -> Result<CoreArchiveSeedSummary> {
        super::scope_inspection::validate_inspection_scope(scope)?;
        self.seed_core_from_archive_selection(Some(scope))
    }

    fn seed_core_from_archive_selection(
        &self,
        scope: Option<&str>,
    ) -> Result<CoreArchiveSeedSummary> {
        self.ensure_manifest()?;
        let mut summary = CoreArchiveSeedSummary::default();
        let archives = self
            .storage
            .read_archive_scoped(&ArchiveFilters::default(), scope, None)?
            .into_items();
        for archive in archives
            .into_iter()
            .filter(|archive| archive.status == ArchiveStatus::Complete)
        {
            summary.archives += 1;
            if archive.personal_signals.is_empty() {
                continue;
            }
            let scope = archive.effective_personality_scope_id();
            let archive_summary =
                self.with_memory_write(scope, &archive.source_session_id, || {
                    let Some(current) = self
                        .storage
                        .read_archive_detail_scoped(scope, &archive.archive_id)?
                        .and_then(|snapshot| snapshot.items.into_iter().next())
                    else {
                        return Ok(CoreSignalSummary::default());
                    };
                    if current.source_session_id != archive.source_session_id {
                        return Ok(CoreSignalSummary::default());
                    }
                    self.apply_archive_personal_signal_bridge(&current)
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
        if archive.status != ArchiveStatus::Complete || archive.personal_signals.is_empty() {
            return Ok(summary);
        }

        let scope = archive.effective_personality_scope_id();
        let session_events = self.read_session_events_with_archived(&archive.source_session_id)?;
        let attribution_events = session_events
            .iter()
            .filter(|event| {
                event.session_id == archive.source_session_id
                    && event.effective_personality_scope_id() == scope
            })
            .map(AttributionEvent::from)
            .collect::<Vec<_>>();
        let mut signals = archive.personal_signals.clone();
        attribute_personal_signals(&mut signals, &attribution_events);

        for signal in &signals {
            let text = normalize_whitespace(&signal.text);
            let category = normalize_category_name(&signal.category);
            let has_user_source = personal_signal_has_user_source(signal, &attribution_events);

            if text.is_empty() || category.is_empty() {
                record_core_signal_skip(&mut summary, "invalid_signal");
                continue;
            }
            if signal.confidence < 0.85 {
                record_core_signal_skip(&mut summary, "below_confidence_threshold");
                continue;
            }
            if !has_user_source {
                record_core_signal_skip(&mut summary, "no_user_authored_source");
                continue;
            }

            if !personal_signal_is_auto_core_eligible(signal, &attribution_events) {
                if self
                    .route_personal_signal_to_review_candidate(archive, signal, &category, &text)?
                {
                    summary.review_candidates += 1;
                }
                record_core_signal_skip(&mut summary, "requires_attribution_review");
                continue;
            }

            let incoming = CoreFactInput {
                schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                category: category.clone(),
                scope: Some(scope.to_string()),
                subject: signal.subject.clone(),
                text,
                confidence: signal.confidence,
                tags: vec![
                    "archive_signal".to_string(),
                    "signal_category".to_string(),
                    format!("signal_category:{category}"),
                ],
                source_archive_ids: vec![archive.archive_id.clone()],
                source_candidate_id: None,
            };
            let result = self.with_resource_lock(core_lock_key(&category), || {
                if let Some(reason) = self.core_signal_conflict(&incoming)? {
                    record_core_signal_skip(&mut summary, reason);
                    return Ok(None);
                }
                self.upsert_core_fact_unlocked(incoming, category.clone())
                    .map(Some)
            })?;
            let Some(result) = result else {
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

    fn route_personal_signal_to_review_candidate(
        &self,
        archive: &ArchiveEntry,
        signal: &crate::archive::PersonalSignal,
        category: &str,
        text: &str,
    ) -> Result<bool> {
        let core_scope = Some(archive.effective_personality_scope_id().to_string());
        let duplicate_exists = self
            .storage
            .read_candidate_beliefs()?
            .into_items()
            .into_iter()
            .any(|candidate| {
                matches!(
                    candidate.status,
                    CandidateStatus::Draft | CandidateStatus::ReadyForReview
                ) && candidate.core_scope == core_scope
                    && candidate.category == category
                    && same_subject(candidate.subject.as_ref(), signal.subject.as_ref())
                    && normalize_match_text(&candidate.text) == normalize_match_text(text)
            });
        if duplicate_exists {
            return Ok(false);
        }

        let now = now_rfc3339()?;
        let mut tags = vec![
            "archive_signal_review".to_string(),
            "attribution_review".to_string(),
            format!("signal_category:{category}"),
        ];
        if signal.subject_hint == Some(PersonalSignalSubjectHint::AboutOther) {
            tags.push("about_other".to_string());
        }
        let candidate = CandidateBelief {
            schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            candidate_id: new_id("candidate")?,
            source_session_id: Some(archive.source_session_id.clone()),
            source_session_ids: vec![archive.source_session_id.clone()],
            core_scope,
            subject: signal.subject.clone(),
            created_at: now.clone(),
            updated_at: now,
            text: text.to_string(),
            category: category.to_string(),
            status: CandidateStatus::ReadyForReview,
            confidence: signal.confidence.clamp(0.0, 1.0),
            supporting_archive_ids: vec![archive.archive_id.clone()],
            contradicting_archive_ids: Vec::new(),
            contradicted_core_fact_ids: Vec::new(),
            evidence_summary: signal
                .evidence
                .as_deref()
                .map(normalize_whitespace)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| text.to_string()),
            promotion_checks: PromotionChecks {
                min_sources_met: false,
                weight_threshold_met: signal.confidence >= 0.70,
                no_recent_contradiction: true,
                manual_review_required: true,
            },
            source_memory_unit_ids: Vec::new(),
            tags: unique_strings(tags),
            promoted_core_fact_id: None,
            review: None,
            links: Vec::new(),
            reinforcements: Vec::new(),
        };
        self.storage.write_candidate_belief(&candidate)?;
        Ok(true)
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
        let attribution_events = session_events
            .iter()
            .map(AttributionEvent::from)
            .collect::<Vec<_>>();
        let mut signals = archive.personal_signals.clone();
        attribute_personal_signals(&mut signals, &attribution_events);

        let mut ids = HashSet::new();
        for signal in &signals {
            let text = normalize_whitespace(&signal.text);
            let category = normalize_category_name(&signal.category);
            let has_user_source = personal_signal_has_user_source(signal, &attribution_events);
            if text.is_empty()
                || category.is_empty()
                || signal.confidence < 0.85
                || !has_user_source
                || !personal_signal_is_auto_core_eligible(signal, &attribution_events)
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

    fn core_signal_conflict(&self, incoming: &CoreFactInput) -> Result<Option<&'static str>> {
        let normalized_scope = normalize_optional_string(incoming.scope.as_deref());
        let needle = normalize_match_text(&incoming.text);
        let needle_tokens = meaningful_tokens(&incoming.text);
        let category = self.storage.read_core_store_category(&incoming.category)?;
        for fact in &category.facts {
            if core_policy::would_recreate_corrected_attribution(fact, incoming)? {
                return Ok(Some("corrected_attribution"));
            }
        }
        for fact in category.facts {
            if fact.status != CoreFactStatus::Active
                || fact.scope != normalized_scope
                || !same_subject(fact.subject.as_ref(), incoming.subject.as_ref())
            {
                continue;
            }
            if normalize_match_text(&fact.text) == needle {
                return Ok(None);
            }
            if token_overlap_sets(&needle_tokens, &meaningful_tokens(&fact.text)) >= 0.55 {
                return Ok(Some("near_duplicate"));
            }
        }

        Ok(None)
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

        self.with_archive_task_response(
            task_id,
            "preliminary_archive_id",
            TaskType::SleepCompression,
            |_| self.resume_sleep_compression_unlocked(task_id, result, true),
        )
    }

    fn resume_sleep_compression_unlocked(
        &self,
        task_id: &str,
        mut result: SleepCompressionResult,
        refresh_archive_coverage: bool,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;

        let mut task = self.load_task_for_response(task_id)?;

        if task.task_type != TaskType::SleepCompression {
            return Err(MemoryEngineError::Validation(format!(
                "task is not sleep_compression: {task_id}"
            )));
        }

        let archive_id = workflow_task_archive_id(&task, "preliminary_archive_id")?;
        if archive_id != result.archive_id {
            return Err(MemoryEngineError::Validation(format!(
                "sleep compression archive_id mismatch: task={archive_id} result={}",
                result.archive_id
            )));
        }
        let mut archive_entry = self.storage.read_archive_entry_by_id(archive_id)?;
        if archive_entry.status != ArchiveStatus::Complete {
            if refresh_archive_coverage && !sleep_task_covers_archive(&task.inputs, &archive_entry)
            {
                return Err(MemoryEngineError::NeedsNewReview);
            }
            let session_id = &archive_entry.source_session_id;
            let covered = if refresh_archive_coverage {
                self.sleep_archive_coverage_unlocked(session_id)?.1
            } else {
                self.archived_event_ids_for_session_unlocked(session_id)?
            };
            if task.state == TaskState::Cancelled
                || sleep_run_is_superseded(&archive_entry, &covered)
            {
                return Err(MemoryEngineError::Validation(format!(
                    "sleep compression no longer owns unpublished events: {task_id}"
                )));
            }
        }
        let attribution_events = self
            .read_session_events_with_archived(&archive_entry.source_session_id)?
            .iter()
            .map(AttributionEvent::from)
            .collect::<Vec<_>>();
        attribute_personal_signals(&mut result.personal_signals, &attribution_events);

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
        self.with_archive_task_response(
            task_id,
            "preliminary_archive_id",
            TaskType::CompactMemoryPass,
            |mut task| {
                let archive_id = workflow_task_archive_id(&task, "preliminary_archive_id")?;
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
            },
        )
    }

    pub fn resume_memory_unit_pass(
        &self,
        task_id: &str,
        result: MemoryUnitPassResult,
    ) -> Result<ArchiveEntry> {
        self.with_archive_task_response(
            task_id,
            "preliminary_archive_id",
            TaskType::MemoryUnitPass,
            |mut task| self.resume_memory_unit_pass_unlocked(&mut task, result),
        )
    }

    fn resume_memory_unit_pass_unlocked(
        &self,
        task: &mut PendingTask,
        result: MemoryUnitPassResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != MEMORY_UNITS_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;
        let task_id = &task.task_id;
        if task.task_type != TaskType::MemoryUnitPass {
            return Err(MemoryEngineError::Validation(format!(
                "task is not memory_unit_pass: {task_id}"
            )));
        }

        let archive_id = workflow_task_archive_id(task, "preliminary_archive_id")?;
        if archive_id != result.archive_id {
            return Err(MemoryEngineError::Validation(format!(
                "memory_unit_pass archive_id mismatch: task={archive_id} result={}",
                result.archive_id
            )));
        }
        if task.state == TaskState::Completed
            && task.outcome.is_none()
            && !self.memory_unit_publication_exists(archive_id)?
        {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        let frozen_events = task
            .inputs
            .get("events")
            .and_then(Value::as_array)
            .filter(|events| !events.is_empty())
            .ok_or(MemoryEngineError::NeedsNewReview)?
            .iter()
            .map(|event| serde_json::from_value::<SleepInputEvent>(event.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| MemoryEngineError::NeedsNewReview)?;
        let allowed_event_ids = frozen_events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<HashSet<_>>();
        result.validate_source_event_ids(&allowed_event_ids)?;
        let attribution_events = frozen_events
            .iter()
            .map(AttributionEvent::from)
            .collect::<Vec<_>>();

        let archive_entry = self.storage.read_archive_entry_raw(archive_id)?;
        if archive_entry.status == ArchiveStatus::Superseded || task.state == TaskState::Cancelled {
            return Err(MemoryEngineError::Validation(format!(
                "memory unit task belongs to a retired sleep run: {task_id}"
            )));
        }
        let now = now_rfc3339()?;
        let request_digest = serializable_digest(&result)?;
        let mut units = Vec::new();
        for draft in result.memory_units {
            if draft.thesis.trim().is_empty() {
                continue;
            }
            units.push(materialize_memory_unit(
                &archive_entry,
                draft,
                &attribution_events,
                &now,
                |_| new_id("mu"),
            )?);
        }

        self.publish_memory_units(task, archive_id, request_digest, units, None)
    }

    pub(super) fn with_archive_task_response<T>(
        &self,
        task_id: &str,
        archive_field: &str,
        task_type: TaskType,
        f: impl FnOnce(PendingTask) -> Result<T>,
    ) -> Result<T> {
        self.ensure_manifest()?;
        let snapshot = self.load_task_for_response(task_id)?;
        if snapshot.task_type != task_type {
            return Err(MemoryEngineError::Validation(format!(
                "task is not {task_type:?}: {task_id}"
            )));
        }
        let archive_id = workflow_task_archive_id(&snapshot, archive_field)?;
        let archive = self
            .storage
            .read_archive_entry_raw(archive_id)
            .map_err(|error| self.task_after_erasure_error(task_id, error))?;
        self.with_memory_write(
            archive.effective_personality_scope_id(),
            &archive.source_session_id,
            || f(self.load_task_for_response(task_id)?),
        )
    }

    pub(super) fn mark_workflow_task_failed_best_effort(
        &self,
        task: &mut PendingTask,
        detail: String,
    ) {
        if matches!(task.state, TaskState::Completed | TaskState::Cancelled) {
            return;
        }
        task.state = TaskState::Failed;
        task.updated_at = now_rfc3339().unwrap_or_else(|_| task.updated_at.clone());
        task.last_error = Some(detail);
        let _ = self.storage.save_task(task);
    }
}

fn workflow_task_archive_id<'a>(task: &'a PendingTask, field: &str) -> Result<&'a str> {
    task.inputs
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            MemoryEngineError::Validation(format!("task has no {field}: {}", task.task_id))
        })
}

struct RepairMemoryUnitInput<'a> {
    thesis: &'a str,
    source_event_ids: Vec<String>,
    evidence: Option<String>,
    tags: Vec<String>,
    weight: f64,
}

fn sleep_run_archive<'a>(run: &SleepRun, archives: &'a [ArchiveEntry]) -> Result<&'a ArchiveEntry> {
    archives
        .iter()
        .find(|entry| entry.archive_id == run.archive_id)
        .ok_or_else(|| {
            MemoryEngineError::Storage(format!(
                "Sleep run Archive is unavailable: {}",
                run.archive_id
            ))
        })
}

fn sleep_run_is_superseded(archive: &ArchiveEntry, covered: &HashSet<String>) -> bool {
    archive.status == ArchiveStatus::Superseded
        || (archive.status == ArchiveStatus::Preliminary
            && archive
                .source_event_ids
                .iter()
                .any(|id| covered.contains(id)))
}

fn sleep_recovery_priority(run: &SleepRun, archive: &ArchiveEntry) -> (bool, bool, usize) {
    (
        archive.status == ArchiveStatus::Complete,
        run.stage == SleepRunStage::ReadyToFinish,
        run.requests
            .iter()
            .filter(|state| state.completed && state.last_error.is_none())
            .count(),
    )
}

const REPAIR_FALLBACK_MAX_UNITS: usize = 8;

fn push_repair_memory_unit(
    units: &mut Vec<MemoryUnit>,
    seen: &mut HashSet<String>,
    archive_entry: &ArchiveEntry,
    now: &str,
    input: RepairMemoryUnitInput<'_>,
) -> Result<()> {
    let thesis = normalize_whitespace(input.thesis);
    if thesis.is_empty() || !seen.insert(thesis.to_lowercase()) {
        return Ok(());
    }

    units.push(MemoryUnit {
        schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
        memory_unit_id: new_id("mu")?,
        archive_id: archive_entry.archive_id.clone(),
        personality_scope_id: archive_entry.personality_scope_id.clone(),
        source_session_id: archive_entry.source_session_id.clone(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        thesis,
        subject: None,
        source_event_ids: input.source_event_ids,
        evidence: normalize_optional_string(input.evidence.as_deref()),
        tags: input.tags,
        weight: input.weight,
        status: MemoryUnitStatus::ActiveArchive,
        fidelity_status: FidelityStatus::Unchecked,
        fidelity_review: None,
        forget_review: None,
        reflection_count: 0,
        last_reflected_at: None,
        last_reflection_operation_id: None,
    });
    Ok(())
}

/// A session is multi-speaker when at least two distinct human speaker ids
/// appear among its user messages. Events without a `speaker` keep the legacy
/// single-user semantics, so a lone identified speaker (or none) is not
/// multi-speaker.
pub(super) fn session_is_multi_speaker(events: &[StoredEvent]) -> bool {
    let attribution_events = events
        .iter()
        .map(AttributionEvent::from)
        .collect::<Vec<_>>();
    attribution_session_is_multi_speaker(&attribution_events)
}
