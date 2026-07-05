use super::*;

pub(super) fn compactable_sleep_events<'a>(
    events: &[&'a StoredEvent],
    config: &SleepStage1Config,
) -> Vec<&'a StoredEvent> {
    let tail_count = active_tail_event_count(events.len(), config);
    let compactable_len = events.len().saturating_sub(tail_count);
    events[..compactable_len].to_vec()
}

pub(super) fn active_tail_event_count(total_events: usize, config: &SleepStage1Config) -> usize {
    if total_events <= 1 || total_events < config.partial_sleep_min_events {
        return 0;
    }

    if !config.active_tail_ratio.is_finite() || config.active_tail_ratio <= 0.0 {
        return 0;
    }

    let ratio = config.active_tail_ratio.min(0.95);
    let tail_count = ((total_events as f64) * ratio).ceil() as usize;
    tail_count.min(total_events.saturating_sub(1))
}

pub(super) fn select_sleep_events<'a>(
    events: &[&'a StoredEvent],
    config: &SleepStage1Config,
) -> Vec<&'a StoredEvent> {
    let mut selected = events
        .iter()
        .copied()
        .filter(|event| {
            event.initial_weight >= config.min_event_weight
                || matches!(
                    event.importance_hint,
                    ImportanceHint::High | ImportanceHint::Critical
                )
        })
        .collect::<Vec<_>>();

    if selected.is_empty() {
        selected = events.to_vec();
    }

    selected.sort_by(|left, right| {
        right
            .initial_weight
            .total_cmp(&left.initial_weight)
            .then_with(|| right.timestamp.cmp(&left.timestamp))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    selected.truncate(config.max_events);
    selected.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    selected
}

pub(super) fn build_preliminary_archive(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_id: &str,
    now: &str,
) -> ArchiveEntry {
    let tags = collect_tags(events);
    let theme = session
        .metadata
        .active_theme
        .clone()
        .or_else(|| events.iter().find_map(|event| event.theme.clone()));
    let quotes = events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| Quote {
                text: truncate_chars(&text, 240),
                source_event_id: Some(event.event_id.clone()),
            })
        })
        .collect::<Vec<_>>();

    ArchiveEntry {
        schema_version: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
        archive_id: archive_id.to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        source_session_id: session.metadata.session_id.clone(),
        source_event_ids: events.iter().map(|event| event.event_id.clone()).collect(),
        time_range: time_range_from_events(events),
        theme,
        tags,
        gist: preliminary_gist(events),
        narrative: preliminary_narrative(&session.metadata.session_id, events),
        compact_memory: None,
        memory_units: Vec::new(),
        facts: preliminary_facts(events),
        quotes,
        weight: archive_weight(events),
        freshness: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        links: events
            .iter()
            .flat_map(|event| event.links.iter().cloned())
            .collect(),
        emotional_markers: Vec::new(),
        topic_thread: Vec::new(),
        personal_signals: Vec::new(),
        relational_tone: None,
        status: ArchiveStatus::Preliminary,
        llm_enhanced: false,
        prompt_id: None,
        prompt_version: None,
        embedding_model_id: None,
        embedding: None,
    }
}

pub(super) fn build_sleep_compression_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    config: &SleepStage1Config,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::SleepCompression,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: config.prompt_id.clone(),
        prompt_version: config.prompt_version,
        role_hint: ModelRole::Balanced,
        expected_output_schema: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": &session.metadata.session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "compact_human_readable_memory",
                "preserve_quotes": true,
                "do_not_invent_facts": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

#[allow(dead_code)]
pub(super) fn build_compact_memory_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::CompactMemoryPass,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: "compact_memory_pass".to_string(),
        prompt_version: 1,
        role_hint: ModelRole::Balanced,
        expected_output_schema: COMPACT_MEMORY_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": &session.metadata.session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "short_human_memory_theses",
                "plain_text_only": true,
                "do_not_invent_facts": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

pub(super) fn build_memory_unit_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::MemoryUnitPass,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: "memory_unit_pass".to_string(),
        prompt_version: 1,
        role_hint: ModelRole::Balanced,
        expected_output_schema: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": &session.metadata.session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "atomic_human_memory_units",
                "return_json": true,
                "do_not_invent_facts": true,
                "use_source_event_ids": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

pub(super) fn build_memory_unit_repair_task(
    session_id: &str,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::MemoryUnitPass,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: "memory_unit_pass".to_string(),
        prompt_version: 1,
        role_hint: ModelRole::Reasoning,
        expected_output_schema: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "repair_reason": "complete_archive_without_memory_units",
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "atomic_human_memory_units",
                "return_json": true,
                "do_not_invent_facts": true,
                "use_source_event_ids": true,
                "repair_missing_memory_units": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

pub(super) fn render_compact_memory_from_units(units: &[MemoryUnit]) -> Option<String> {
    let lines = units
        .iter()
        .filter(|unit| unit.status == MemoryUnitStatus::ActiveArchive)
        .map(|unit| unit.thesis.trim())
        .filter(|thesis| !thesis.is_empty())
        .collect::<Vec<_>>();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

pub(super) fn memory_unit_status_after_fidelity(status: FidelityStatus) -> MemoryUnitStatus {
    match status {
        FidelityStatus::Valid | FidelityStatus::SelfChecked => MemoryUnitStatus::ActiveArchive,
        FidelityStatus::Unsupported | FidelityStatus::Distorted => MemoryUnitStatus::Rejected,
        FidelityStatus::Unchecked
        | FidelityStatus::TooBroad
        | FidelityStatus::MissingKeyDetail
        | FidelityStatus::NeedsRevision => MemoryUnitStatus::NeedsRevision,
    }
}

pub(super) fn evidence_event_from_stored(
    event: &StoredEvent,
    role: EvidenceEventRole,
    max_text_chars: usize,
) -> EvidenceEvent {
    EvidenceEvent {
        event_id: event.event_id.clone(),
        timestamp: event.timestamp.clone(),
        event_type: event.event_type.clone(),
        source: event.source.clone(),
        role,
        text: truncate_chars(
            &event_text(event).unwrap_or_else(|| event.payload.to_string()),
            max_text_chars,
        ),
        tags: event.tags.clone(),
    }
}

pub(super) fn add_evidence_event(
    pack: &mut EvidencePack,
    event: EvidenceEvent,
    selected: &mut HashSet<String>,
    force: bool,
) {
    if !selected.insert(event.event_id.clone()) {
        return;
    }
    let mut candidate = pack.clone();
    candidate.events.push(event.clone());
    let candidate_tokens = estimate_evidence_pack_tokens(&candidate);
    if force || candidate_tokens <= pack.max_estimated_tokens {
        pack.events.push(event);
        pack.estimated_tokens = candidate_tokens;
    } else {
        pack.truncated = true;
    }
}

pub(super) fn estimate_evidence_pack_tokens(pack: &EvidencePack) -> usize {
    let mut chars = pack.target_thesis.chars().count()
        + pack
            .unit_evidence
            .as_deref()
            .map(|value| value.chars().count())
            .unwrap_or(0);
    for event in &pack.events {
        chars += event.event_id.chars().count()
            + event.timestamp.chars().count()
            + event.event_type.chars().count()
            + event.source.chars().count()
            + event.text.chars().count()
            + event
                .tags
                .iter()
                .map(|tag| tag.chars().count())
                .sum::<usize>()
            + 32;
    }
    chars.div_ceil(2)
}

pub(super) fn time_range_from_events(events: &[&StoredEvent]) -> TimeRange {
    let start = events
        .iter()
        .map(|event| event.timestamp.as_str())
        .min()
        .unwrap_or("unknown")
        .to_string();
    let end = events
        .iter()
        .map(|event| event.timestamp.as_str())
        .max()
        .unwrap_or("unknown")
        .to_string();

    TimeRange { start, end }
}

pub(super) fn collect_tags(events: &[&StoredEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| event.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn preliminary_gist(events: &[&StoredEvent]) -> String {
    let texts = events
        .iter()
        .filter_map(|event| event_text(event))
        .take(3)
        .map(|text| truncate_chars(&text, 120))
        .collect::<Vec<_>>();

    if texts.is_empty() {
        format!("Попередній спогад із {} події(й).", events.len())
    } else {
        format!("Попередній спогад: {}.", texts.join(" / "))
    }
}
pub(super) fn preliminary_narrative(session_id: &str, events: &[&StoredEvent]) -> String {
    let lines = events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| {
                format!(
                    "{} {}: {}",
                    event.timestamp,
                    event.event_type,
                    truncate_chars(&text, 180)
                )
            })
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        format!(
            "Попередній архівний спогад із сесії {session_id}, створений алгоритмічно з {} події(й).",
            events.len()
        )
    } else {
        format!(
            "Попередній архівний спогад із сесії {session_id}. Ключові події: {}",
            lines.join(" | ")
        )
    }
}
pub(super) fn preliminary_facts(events: &[&StoredEvent]) -> Vec<WeightedFact> {
    events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| WeightedFact {
                text: truncate_chars(&text, 240),
                confidence: event.initial_weight.clamp(0.0, 1.0),
                source_event_ids: vec![event.event_id.clone()],
            })
        })
        .collect()
}

pub(super) fn archive_weight(events: &[&StoredEvent]) -> f64 {
    events
        .iter()
        .map(|event| event.initial_weight)
        .fold(0.0, f64::max)
        .clamp(0.0, 1.0)
}

pub(super) fn sleep_run_from_stage1(stage1: SleepStage1Result) -> Result<SleepRun> {
    let mut requests = Vec::new();
    let sleep_task = stage1.pending_task;

    if let Some(memory_unit_task) = stage1.memory_unit_task.clone() {
        requests.push(SleepRequestState {
            track: SleepTrack::MemoryUnit,
            request: llm_request_from_task(
                &memory_unit_task,
                "memory_unit_pass",
                json!({ "sleep_task": memory_unit_task.inputs }),
            )?,
            attempts: 0,
            completed: false,
            last_error: None,
        });
    }

    for (track, prompt_id, fallback_schema) in [
        (
            SleepTrack::Emotional,
            "sleep_emotional_pass",
            "sleep_emotional_pass.v1",
        ),
        (
            SleepTrack::TopicThread,
            "sleep_topic_thread_pass",
            "sleep_topic_thread_pass.v1",
        ),
        (
            SleepTrack::PersonalSignal,
            "sleep_personal_signal_pass",
            "sleep_personal_signal_pass.v1",
        ),
        (
            SleepTrack::Relational,
            "sleep_relational_pass",
            "sleep_relational_pass.v1",
        ),
    ] {
        requests.push(SleepRequestState {
            track,
            request: LlmRequest {
                request_id: new_id("llm_req")?,
                task_id: sleep_task.task_id.clone(),
                role_hint: sleep_task.role_hint,
                prompt_id: prompt_id.to_string(),
                prompt_version: sleep_task.prompt_version,
                prompt_inputs: json!({ "sleep_task": sleep_task.inputs }),
                expected_output_schema: fallback_schema.to_string(),
            },
            attempts: 0,
            completed: false,
            last_error: None,
        });
    }

    Ok(SleepRun {
        schema_version: SLEEP_RUN_SCHEMA_VERSION.to_string(),
        session_id: stage1.archive_entry.source_session_id.clone(),
        archive_id: stage1.archive_entry.archive_id,
        sleep_task_id: sleep_task.task_id,
        memory_unit_task_id: stage1.memory_unit_task.map(|task| task.task_id),
        stage: SleepRunStage::Extraction,
        max_pass_attempts: DEFAULT_SLEEP_PASS_MAX_ATTEMPTS,
        requests,
        failed_passes: Vec::new(),
        memory_unit_result: None,
        emotional_pass: None,
        topic_thread_pass: None,
        personal_signal_pass: None,
        relational_pass: None,
        consolidator_gist: None,
        consolidator_narrative: None,
        completion_mode: None,
    })
}

pub(super) fn llm_request_from_task(
    task: &PendingTask,
    prompt_id: &str,
    prompt_inputs: Value,
) -> Result<LlmRequest> {
    Ok(LlmRequest {
        request_id: new_id("llm_req")?,
        task_id: task.task_id.clone(),
        role_hint: task.role_hint,
        prompt_id: prompt_id.to_string(),
        prompt_version: task.prompt_version,
        prompt_inputs,
        expected_output_schema: task.expected_output_schema.clone(),
    })
}

pub(super) fn validate_sleep_run(run: &SleepRun) -> Result<()> {
    if run.schema_version != SLEEP_RUN_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: SLEEP_RUN_SCHEMA_VERSION.to_string(),
            actual: run.schema_version.clone(),
        });
    }
    if run.session_id.trim().is_empty()
        || run.archive_id.trim().is_empty()
        || run.sleep_task_id.trim().is_empty()
    {
        return Err(MemoryEngineError::Validation(
            "sleep run must include session_id, archive_id, and sleep_task_id".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn cancel_task_if_active<S: Storage>(
    storage: &S,
    task_id: &str,
    reason: &str,
) -> Result<()> {
    let mut task = storage.load_task(task_id)?;
    if matches!(
        task.state,
        TaskState::Completed | TaskState::Failed | TaskState::Cancelled
    ) {
        return Ok(());
    }

    task.state = TaskState::Cancelled;
    task.updated_at = now_rfc3339()?;
    task.last_error = Some(reason.to_string());
    storage.save_task(&task)
}

pub(super) fn advance_sleep_run_stage(run: &mut SleepRun) -> Result<()> {
    match run.stage {
        SleepRunStage::Extraction => {
            if run
                .requests
                .iter()
                .filter(|state| state_stage(state.track) == SleepRunStage::Extraction)
                .all(|state| state.completed)
            {
                run.stage = SleepRunStage::Consolidation;
                ensure_consolidator_request(run)?;
            }
        }
        SleepRunStage::Consolidation => {
            if run
                .requests
                .iter()
                .filter(|state| state_stage(state.track) == SleepRunStage::Consolidation)
                .all(|state| state.completed)
            {
                run.stage = SleepRunStage::ReadyToFinish;
            }
        }
        SleepRunStage::ReadyToFinish | SleepRunStage::Finished => {}
    }
    Ok(())
}

pub(super) fn ensure_consolidator_request(run: &mut SleepRun) -> Result<()> {
    if run
        .requests
        .iter()
        .any(|state| state.track == SleepTrack::Consolidator)
        || run.completion_mode.as_deref() == Some("fallback_from_tracks")
    {
        return Ok(());
    }

    run.requests.push(SleepRequestState {
        track: SleepTrack::Consolidator,
        request: LlmRequest {
            request_id: new_id("llm_req")?,
            task_id: run.sleep_task_id.clone(),
            role_hint: ModelRole::Balanced,
            prompt_id: "sleep_consolidator".to_string(),
            prompt_version: 1,
            prompt_inputs: json!({
                "sleep_task": sleep_task_input_from_run(run),
                "emotional_pass": run.emotional_pass.clone().unwrap_or_else(|| json!({ "emotional_markers": [] })),
                "topic_thread_pass": run.topic_thread_pass.clone().unwrap_or_else(|| json!({ "topic_thread": [] })),
                "personal_signal_pass": run.personal_signal_pass.clone().unwrap_or_else(|| json!({ "personal_signals": [] })),
                "relational_pass": run.relational_pass.clone().unwrap_or_else(|| json!({ "relational_tone": null })),
            }),
            expected_output_schema: CONSOLIDATOR_TEXT_SCHEMA_VERSION.to_string(),
        },
        attempts: 0,
        completed: false,
        last_error: None,
    });
    Ok(())
}

pub(super) fn sleep_task_input_from_run(run: &SleepRun) -> Value {
    run.requests
        .iter()
        .find(|state| {
            matches!(
                state.track,
                SleepTrack::Emotional
                    | SleepTrack::TopicThread
                    | SleepTrack::PersonalSignal
                    | SleepTrack::Relational
            )
        })
        .and_then(|state| state.request.prompt_inputs.get("sleep_task").cloned())
        .unwrap_or_else(|| json!({}))
}

pub(super) fn state_stage(track: SleepTrack) -> SleepRunStage {
    match track {
        SleepTrack::Consolidator => SleepRunStage::Consolidation,
        SleepTrack::MemoryUnit
        | SleepTrack::Emotional
        | SleepTrack::TopicThread
        | SleepTrack::PersonalSignal
        | SleepTrack::Relational => SleepRunStage::Extraction,
    }
}

pub(super) fn handle_sleep_response(
    run: &mut SleepRun,
    state: &mut SleepRequestState,
    response: LlmResponse,
) -> Result<()> {
    match response {
        LlmResponse::Ok { text, .. } => match parse_sleep_response_text(state.track, &text, run) {
            Ok(value) => {
                assign_sleep_track_result(run, state.track, value);
                state.completed = true;
                state.last_error = None;
            }
            Err(err) => handle_sleep_pass_error(run, state, err.to_string())?,
        },
        LlmResponse::Err { kind, detail, .. } => {
            handle_sleep_pass_error(run, state, format!("{kind:?}: {detail}"))?
        }
    }
    Ok(())
}

pub(super) fn handle_sleep_pass_error(
    run: &mut SleepRun,
    state: &mut SleepRequestState,
    error: String,
) -> Result<()> {
    state.last_error = Some(error.clone());
    if state.attempts < run.max_pass_attempts {
        state.request.prompt_inputs = add_retry_instruction(&state.request.prompt_inputs, &error);
        if state.track == SleepTrack::MemoryUnit && state.attempts + 1 >= run.max_pass_attempts {
            state.request.role_hint = ModelRole::Reasoning;
        }
        return Ok(());
    }

    push_unique(&mut run.failed_passes, state.request.prompt_id.clone());
    let fallback = fallback_value_for_track(state.track, run);
    if state.track != SleepTrack::Consolidator {
        assign_sleep_track_result(run, state.track, fallback);
    }
    state.completed = true;
    Ok(())
}

pub(super) fn parse_sleep_response_text(
    track: SleepTrack,
    text: &str,
    run: &SleepRun,
) -> Result<Value> {
    if track == SleepTrack::Consolidator {
        let (gist, narrative) = parse_consolidator_text(text)?;
        return Ok(json!({
            "gist": gist,
            "narrative": narrative,
        }));
    }

    let value = parse_json_value_from_llm_text(text)?;
    match track {
        SleepTrack::MemoryUnit => {
            let mut result: MemoryUnitPassResult = serde_json::from_value(value)?;
            if result.schema_version.trim().is_empty() {
                result.schema_version = MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string();
            }
            if result.archive_id != run.archive_id {
                result.archive_id = run.archive_id.clone();
            }
            result.validate_basic()?;
            Ok(serde_json::to_value(result)?)
        }
        SleepTrack::Consolidator => unreachable!("consolidator text is parsed before JSON tracks"),
        SleepTrack::Emotional
        | SleepTrack::TopicThread
        | SleepTrack::PersonalSignal
        | SleepTrack::Relational => Ok(value),
    }
}

pub(super) fn parse_json_value_from_llm_text(text: &str) -> Result<Value> {
    let mut candidate = text.trim().to_string();
    if candidate.starts_with("```") {
        candidate = candidate
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim()
            .to_string();
        if candidate.ends_with("```") {
            candidate.truncate(candidate.len().saturating_sub(3));
        }
    }

    match serde_json::from_str::<Value>(candidate.trim()) {
        Ok(value) => Ok(value),
        Err(original_err) => {
            let Some(start) = candidate.find('{') else {
                return Err(original_err.into());
            };
            let Some(end) = candidate.rfind('}') else {
                return Err(original_err.into());
            };
            serde_json::from_str::<Value>(&candidate[start..=end]).map_err(Into::into)
        }
    }
}

pub(super) fn parse_consolidator_text(text: &str) -> Result<(String, String)> {
    let candidate = strip_markdown_fence(text.trim()).trim();

    if candidate.is_empty() {
        return Err(MemoryEngineError::Validation(
            "consolidator returned empty text".to_string(),
        ));
    }

    if let Some(decoded) = parse_consolidator_json_string(candidate)? {
        return parse_consolidator_text(&decoded);
    }

    if let Some((gist, narrative)) = parse_consolidator_json_object(candidate)? {
        return validate_consolidator_overlay(gist, narrative);
    }

    let mut lines = candidate.lines();
    let first_nonempty = lines
        .by_ref()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .ok_or_else(|| {
            MemoryEngineError::Validation("consolidator returned empty text".to_string())
        })?;

    let gist = strip_consolidator_gist_prefix(first_nonempty).to_string();

    let narrative = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let narrative = if narrative.is_empty() {
        gist.clone()
    } else {
        narrative
    };

    validate_consolidator_overlay(gist, narrative)
}

pub(super) fn strip_markdown_fence(text: &str) -> &str {
    let Some(stripped) = text.strip_prefix("```") else {
        return text;
    };

    let body = stripped
        .find('\n')
        .map(|index| &stripped[index + 1..])
        .unwrap_or("");
    body.strip_suffix("```").unwrap_or(body).trim()
}

pub(super) fn parse_consolidator_json_object(candidate: &str) -> Result<Option<(String, String)>> {
    if !candidate.starts_with('{') {
        return Ok(None);
    }

    let value = serde_json::from_str::<Value>(candidate).map_err(|err| {
        MemoryEngineError::Validation(format!(
            "consolidator returned JSON-shaped text that could not be parsed: {err}"
        ))
    })?;

    let gist = value
        .get("gist")
        .and_then(Value::as_str)
        .map(strip_consolidator_gist_prefix)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let narrative = value
        .get("narrative")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    match (gist, narrative) {
        (Some(gist), Some(narrative)) => Ok(Some((gist, narrative))),
        (Some(gist), None) => Ok(Some((gist.clone(), gist))),
        (None, Some(narrative)) => Ok(Some((narrative.clone(), narrative))),
        (None, None) => Err(MemoryEngineError::Validation(
            "consolidator returned JSON without gist or narrative strings".to_string(),
        )),
    }
}

pub(super) fn parse_consolidator_json_string(candidate: &str) -> Result<Option<String>> {
    if !candidate.starts_with('"') {
        return Ok(None);
    }

    serde_json::from_str::<String>(candidate)
        .map(Some)
        .map_err(|err| {
            MemoryEngineError::Validation(format!(
                "consolidator returned quoted text that could not be decoded: {err}"
            ))
        })
}

pub(super) fn strip_consolidator_gist_prefix(text: &str) -> &str {
    text.trim()
        .strip_prefix("GIST:")
        .or_else(|| text.trim().strip_prefix("gist:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| text.trim())
}

pub(super) fn validate_consolidator_overlay(
    gist: String,
    narrative: String,
) -> Result<(String, String)> {
    if !gist_looks_valid(&gist) {
        return Err(MemoryEngineError::Validation(format!(
            "{CONSOLIDATOR_GIST_REJECTED_MARKER}: consolidator gist is not a compact single-line summary"
        )));
    }
    if !narrative_looks_valid(&narrative) {
        return Err(MemoryEngineError::Validation(
            "consolidator_narrative_rejected: consolidator narrative looks like a raw structured blob"
                .to_string(),
        ));
    }
    Ok((gist, narrative))
}

pub(super) fn gist_looks_valid(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 200 {
        return false;
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }
    if starts_with_structural_wrapper(trimmed) {
        return false;
    }
    serde_json::from_str::<Value>(trimmed).is_err()
}

pub(super) fn narrative_looks_valid(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if matches!(trimmed.chars().next(), Some('{') | Some('[') | Some('`')) {
        return false;
    }
    if trimmed.starts_with('"') && serde_json::from_str::<String>(trimmed).is_ok() {
        return false;
    }
    true
}

pub(super) fn starts_with_structural_wrapper(value: &str) -> bool {
    matches!(
        value.chars().next(),
        Some('{') | Some('[') | Some('"') | Some('`')
    )
}

pub(super) fn assign_sleep_track_result(run: &mut SleepRun, track: SleepTrack, value: Value) {
    match track {
        SleepTrack::MemoryUnit => run.memory_unit_result = Some(value),
        SleepTrack::Emotional => run.emotional_pass = Some(value),
        SleepTrack::TopicThread => run.topic_thread_pass = Some(value),
        SleepTrack::PersonalSignal => run.personal_signal_pass = Some(value),
        SleepTrack::Relational => run.relational_pass = Some(value),
        SleepTrack::Consolidator => {
            run.consolidator_gist = value
                .get("gist")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            run.consolidator_narrative = value
                .get("narrative")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            run.completion_mode = Some("consolidated".to_string());
        }
    }
}

pub(super) fn fallback_value_for_track(track: SleepTrack, run: &mut SleepRun) -> Value {
    match track {
        SleepTrack::MemoryUnit => serde_json::to_value(empty_memory_unit_result(&run.archive_id))
            .unwrap_or_else(|_| json!({ "memory_units": [] })),
        SleepTrack::Emotional => json!({ "emotional_markers": [] }),
        SleepTrack::TopicThread => json!({ "topic_thread": [] }),
        SleepTrack::PersonalSignal => json!({ "personal_signals": [] }),
        SleepTrack::Relational => json!({ "relational_tone": null }),
        SleepTrack::Consolidator => {
            run.completion_mode = Some("fallback_from_tracks".to_string());
            json!(null)
        }
    }
}

pub(super) fn add_retry_instruction(prompt_inputs: &Value, error: &str) -> Value {
    let mut value = prompt_inputs.clone();
    if let Value::Object(ref mut object) = value {
        object.insert(
            "retry_instruction".to_string(),
            json!({
                "previous_response_error": error,
                "instruction": "Your previous response was not accepted. Return only the requested valid output schema. No prose, no markdown, no comments."
            }),
        );
    }
    value
}

pub(super) fn llm_response_request_id(response: &LlmResponse) -> &str {
    match response {
        LlmResponse::Ok { request_id, .. } | LlmResponse::Err { request_id, .. } => request_id,
    }
}

pub(super) fn empty_memory_unit_result(archive_id: &str) -> MemoryUnitPassResult {
    MemoryUnitPassResult {
        schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
        archive_id: archive_id.to_string(),
        memory_units: Vec::new(),
    }
}

pub(super) fn assemble_sleep_compression_from_tracks(
    run: &SleepRun,
) -> Result<SleepCompressionResult> {
    let emotional_markers = run
        .emotional_pass
        .as_ref()
        .and_then(|value| value.get("emotional_markers"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let topic_thread = run
        .topic_thread_pass
        .as_ref()
        .and_then(|value| value.get("topic_thread"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let personal_signals = run
        .personal_signal_pass
        .as_ref()
        .and_then(|value| value.get("personal_signals"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let relational_tone = run
        .relational_pass
        .as_ref()
        .and_then(|value| value.get("relational_tone"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut result = SleepCompressionResult {
        schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
        archive_id: run.archive_id.clone(),
        gist: "Сесія збережена як набір важливих спогадів.".to_string(),
        narrative: neutral_narrative_from_tracks(
            &serde_json::from_value::<Vec<crate::archive::EmotionalMarker>>(
                emotional_markers.clone(),
            )?,
            &serde_json::from_value::<Vec<crate::archive::TopicThreadItem>>(topic_thread.clone())?,
            &serde_json::from_value::<Vec<crate::archive::PersonalSignal>>(
                personal_signals.clone(),
            )?,
        ),
        compact_memory: None,
        facts: Vec::new(),
        quotes: Vec::new(),
        tags: vec!["multi_pass_sleep".to_string()],
        theme: None,
        weight: 0.55,
        links: Vec::new(),
        emotional_markers: serde_json::from_value(emotional_markers)?,
        topic_thread: serde_json::from_value(topic_thread)?,
        personal_signals: serde_json::from_value(personal_signals)?,
        relational_tone: serde_json::from_value(relational_tone)?,
    };

    if let Some(signal) = result.personal_signals.first() {
        result.gist = signal.text.clone();
    } else if let Some(topic) = result.topic_thread.first() {
        result.gist = topic.summary.clone().unwrap_or_else(|| topic.topic.clone());
        result.theme = Some(topic.topic.clone());
    } else if let Some(marker) = result.emotional_markers.first() {
        result.gist = format!("{}: {}", marker.target, marker.affect);
    }
    if run.completion_mode.as_deref() == Some("fallback_from_tracks") {
        result.tags.push("consolidator_fallback".to_string());
    }
    result.weight = fallback_archive_weight(&result);
    result.validate_basic()?;
    Ok(result)
}

pub(super) fn neutral_narrative_from_tracks(
    emotional_markers: &[crate::archive::EmotionalMarker],
    topic_thread: &[crate::archive::TopicThreadItem],
    personal_signals: &[crate::archive::PersonalSignal],
) -> String {
    let mut parts = Vec::new();
    if !personal_signals.is_empty() {
        let signals = personal_signals
            .iter()
            .take(3)
            .map(|signal| signal.text.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!("Особисті сигнали: {signals}."));
    }
    if !emotional_markers.is_empty() {
        let markers = emotional_markers
            .iter()
            .take(3)
            .map(|marker| format!("{} ({})", marker.target, marker.affect))
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!("Емоційно помітні моменти: {markers}."));
    }
    if !topic_thread.is_empty() {
        let topics = topic_thread
            .iter()
            .take(4)
            .map(|topic| {
                topic
                    .summary
                    .as_deref()
                    .filter(|summary| !summary.trim().is_empty())
                    .unwrap_or(&topic.topic)
            })
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!("Теми розмови: {topics}."));
    }

    if parts.is_empty() {
        "Сесія була стиснута у структуровані треки, але без виразних довготривалих сигналів."
            .to_string()
    } else {
        parts.join(" ")
    }
}
pub(super) fn fallback_archive_weight(result: &SleepCompressionResult) -> f64 {
    let strongest_emotion = result
        .emotional_markers
        .iter()
        .map(|marker| marker.strength)
        .fold(0.0, f64::max);
    let strongest_signal = result
        .personal_signals
        .iter()
        .map(|signal| signal.confidence)
        .fold(0.0, f64::max);
    strongest_emotion.max(strongest_signal).clamp(0.55, 1.0)
}

pub(super) fn apply_sleep_run_tags(result: &mut SleepCompressionResult, run: &SleepRun) {
    if let Some(mode) = &run.completion_mode {
        result.tags.push(format!("completion_mode:{mode}"));
    }
    if run
        .failed_passes
        .iter()
        .any(|prompt_id| prompt_id == "sleep_consolidator")
        && run.requests.iter().any(|state| {
            state.track == SleepTrack::Consolidator
                && state
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains(CONSOLIDATOR_GIST_REJECTED_MARKER))
        })
    {
        result
            .tags
            .push(CONSOLIDATOR_GIST_REJECTED_MARKER.to_string());
    }
    for prompt_id in &run.failed_passes {
        result.tags.push(format!("pass_failed:{prompt_id}"));
    }
    result.tags = unique_strings(std::mem::take(&mut result.tags));
}

pub(super) fn push_unique(target: &mut Vec<String>, value: String) {
    if !target.iter().any(|existing| existing == &value) {
        target.push(value);
    }
}
