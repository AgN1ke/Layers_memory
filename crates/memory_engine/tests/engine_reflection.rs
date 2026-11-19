use memory_engine::archive::{ArchiveStatus, FidelityReview, FidelityStatus, RelationalTone};
use memory_engine::core_store::{
    CandidateReviewInput, CandidateStatus, CoreContextRequest, CoreFactInput, CoreFactStatus,
    ReviewDecision,
};
use memory_engine::event::IngestEvent;
use memory_engine::journal::{JournalOperation, JournalOperationType, JournalState};
use memory_engine::llm::LlmResponse;
use memory_engine::reflection::{
    ReflectionActionDraft, ReflectionActionsResult, ReflectionAnalyzeResult,
    ReflectionCandidateDraft, ReflectionNoInsight, ReflectionNoInsightReason,
    ReflectionOpportunity, ReflectionState, ReflectionTriggerReason,
};
use memory_engine::sleep::{MemoryUnitDraft, MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::storage::Storage;
use memory_engine::tasks::TaskState;
use memory_engine::types::{
    CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION, CORE_CONTEXT_REQUEST_SCHEMA_VERSION,
    CORE_FACT_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, FIDELITY_REVIEW_SCHEMA_VERSION,
    MEMORY_UNITS_RESULT_SCHEMA_VERSION, REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
    REFLECTION_APPLY_RESULT_SCHEMA_VERSION, REFLECTION_RESULT_SCHEMA_VERSION,
    REFLECTION_STATE_SCHEMA_VERSION, SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use memory_engine::{EngineOptions, FileStorage, MemoryEngine, MemoryEngineError};
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn reflection_creates_reviewable_candidate_and_confirm_promotes_to_core() {
    let root = unique_temp_dir("reflection_candidate_confirm_promotes");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    ingest_text(
        &engine,
        "2026-05-31T20:00:00.000Z",
        "I keep returning to space because it calms me down.",
    );
    let sleep = engine.sleep("live_session").expect("sleep stage1");
    let unit_archive = engine
        .resume_memory_unit_pass(
            &sleep
                .memory_unit_task
                .as_ref()
                .expect("memory unit task")
                .task_id,
            MemoryUnitPassResult {
                schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                memory_units: vec![MemoryUnitDraft {
                    thesis: "Space interest -> the user returns to space because it calms them."
                        .to_string(),
                    subject_hint: None,
                    speaker_id: None,
                    source_event_ids: sleep.archive_entry.source_event_ids.clone(),
                    evidence: Some("The user directly said space calms them down.".to_string()),
                    tags: vec!["interest".to_string(), "values".to_string()],
                    weight: 0.9,
                }],
            },
        )
        .expect("resume memory unit pass");
    let unit_id = unit_archive.memory_units[0].memory_unit_id.clone();

    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                gist: "The user said space calms them.".to_string(),
                narrative: "The user framed space as a recurring topic that helps them feel calm."
                    .to_string(),
                compact_memory: None,
                facts: Vec::new(),
                quotes: Vec::new(),
                tags: vec!["space".to_string(), "personal_pattern".to_string()],
                theme: Some("space_interest".to_string()),
                weight: 0.9,
                links: Vec::new(),
                emotional_markers: Vec::new(),
                topic_thread: Vec::new(),
                personal_signals: Vec::new(),
                relational_tone: Some(RelationalTone {
                    warmth: None,
                    intellectual_engagement: Some(0.7),
                    intimacy: None,
                    trust: None,
                    playfulness: None,
                    tension: None,
                    summary: Some("The exchange was reflective.".to_string()),
                    source_event_ids: sleep.archive_entry.source_event_ids.clone(),
                }),
            },
        )
        .expect("resume sleep compression");

    let fidelity = engine
        .begin_memory_fidelity_pass(&unit_id)
        .expect("begin fidelity");
    engine
        .resume_memory_fidelity_pass(
            &fidelity.pending_task.task_id,
            FidelityReview {
                schema_version: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                memory_unit_id: unit_id.clone(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                status: FidelityStatus::Valid,
                confidence: 0.95,
                explanation: "The thesis is directly supported by the source event.".to_string(),
                revised_thesis: None,
                missing_detail: None,
            },
        )
        .expect("mark unit valid");

    let start = engine
        .begin_reflection_analysis("live_session", Some("live_session".to_string()))
        .expect("begin reflection");
    assert_eq!(start.memory_unit_count, 1);
    assert_eq!(
        start.request.role_hint,
        memory_engine::types::ModelRole::Reasoning
    );

    let candidates = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id.clone(),
                text: json!({
                    "schema_version": "reflection_result.v1",
                    "source_session_id": "live_session",
                    "core_scope": "live_session",
                    "candidates": [{
                        "text": "The user uses space as a calming recurring interest.",
                        "category": "interest",
                        "confidence": 0.9,
                        "evidence_summary": "A validated memory unit says the user returns to space because it calms them.",
                        "source_memory_unit_ids": [unit_id.clone()],
                        "supporting_archive_ids": [sleep.archive_entry.archive_id.clone()],
                        "contradicting_archive_ids": [],
                        "tags": ["reflection", "space"]
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit reflection");
    assert_eq!(candidates.candidates.len(), 1);
    assert_eq!(
        candidates.candidates[0].status,
        CandidateStatus::ReadyForReview
    );
    let scope_error = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: candidates.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: None,
            core_scope: Some("another_owner".to_string()),
        })
        .expect_err("review cannot move a candidate into another scope");
    assert!(scope_error.to_string().contains("cannot change core_scope"));
    let before_review = storage_probe
        .read_candidate_belief(&candidates.candidates[0].candidate_id)
        .expect("candidate remains reviewable after rejected scope override");

    let review = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: candidates.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: Some("Accurate and stable enough.".to_string()),
            core_scope: Some("live_session".to_string()),
        })
        .expect("confirm candidate");
    assert_eq!(review.candidate.status, CandidateStatus::Promoted);
    assert!(review.promoted_fact.is_some());

    let mut review_operation = storage_probe
        .load_journaled_operations()
        .expect("load review journal")
        .into_items()
        .into_iter()
        .find(|operation| operation.op_type == JournalOperationType::CandidateReview)
        .expect("candidate review is journaled");
    assert_eq!(review_operation.state, JournalState::Completed);
    storage_probe
        .write_candidate_belief(&before_review)
        .expect("simulate restart after Core write and before candidate write");
    review_operation.state = JournalState::Started;
    review_operation.completed_at = None;
    storage_probe
        .begin_journaled_operation(&review_operation)
        .expect("reopen review journal fixture");
    let restored = MemoryEngine::new(FileStorage::with_host_id(&root, "reflection_test"));
    assert_eq!(
        restored
            .recover_pending_memory_operations()
            .expect("recover candidate review"),
        1
    );
    assert_eq!(
        storage_probe
            .read_candidate_belief(&candidates.candidates[0].candidate_id)
            .expect("read recovered candidate")
            .status,
        CandidateStatus::Promoted
    );
    let terminal_error = restored
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: candidates.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Rejected,
            note: None,
            core_scope: Some("live_session".to_string()),
        })
        .expect_err("terminal candidate cannot be reviewed twice");
    assert!(terminal_error.to_string().contains("not reviewable"));

    let core = storage_probe
        .read_core_store_category("interest")
        .expect("read core category");
    assert_eq!(core.facts.len(), 1);
    assert_eq!(
        core.facts[0].source_candidate_id.as_deref(),
        Some(candidates.candidates[0].candidate_id.as_str())
    );
    assert_eq!(core.facts[0].scope.as_deref(), Some("live_session"));
    assert_eq!(
        core.facts[0].status,
        memory_engine::core_store::CoreFactStatus::Active
    );

    let archive = storage_probe
        .read_archive_entry_by_id(&sleep.archive_entry.archive_id)
        .expect("read archive");
    assert_eq!(archive.status, ArchiveStatus::Complete);

    fs::remove_dir_all(root).ok();
}

#[test]
fn typed_reflection_no_insight_is_an_explicit_successful_checkpoint() {
    let root = unique_temp_dir("typed_reflection_no_insight");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (unit_id, _) = seed_valid_unit(
        &engine,
        "2026-08-01T08:00:00.000Z",
        "I felt tired once today.",
        "Temporary state -> the user felt tired once.",
        0.95,
    );

    let start = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin typed reflection");
    assert_eq!(start.pending_task.prompt_version, 3);
    assert_eq!(
        start.pending_task.expected_output_schema,
        REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION
    );
    let applied = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id,
                text: json!({
                    "schema_version": REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "core_scope": "owner_scope",
                    "actions": [],
                    "no_insight": {
                        "reason": "temporary_state",
                        "explanation": "One tired day is not a stable trait."
                    }
                })
                .to_string(),
            },
        )
        .expect("apply explicit no-insight result");

    let operation_id = applied
        .operation_id
        .clone()
        .expect("typed apply exposes its durable operation");
    let result_digest = applied
        .result_digest
        .clone()
        .expect("typed apply exposes its result digest");

    assert!(applied.candidates.is_empty());
    assert_eq!(
        applied.schema_version,
        REFLECTION_APPLY_RESULT_SCHEMA_VERSION
    );
    assert!(applied.reinforced_candidates.is_empty());
    assert_eq!(
        applied.no_insight.expect("no insight").explanation,
        "One tired day is not a stable trait."
    );
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&unit_id)
            .expect("read checkpointed unit")
            .reflection_count,
        1
    );
    let checkpointed_unit = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read operation marker");
    assert_eq!(
        checkpointed_unit.last_reflection_operation_id.as_deref(),
        Some(operation_id.as_str())
    );
    let completed_task = storage_probe
        .load_task(&start.pending_task.task_id)
        .expect("read durable reflection outcome");
    let task_outcome = completed_task.outcome.expect("completed task outcome");
    assert_eq!(task_outcome["operation_id"], operation_id);
    assert_eq!(task_outcome["result_digest"], result_digest);
    assert!(!task_outcome.to_string().contains("One tired day"));

    let mut operation = storage_probe
        .load_journaled_operation(&operation_id)
        .expect("load reflection journal")
        .expect("reflection journal exists");
    assert_eq!(operation.state, JournalState::Completed);
    operation.state = JournalState::Started;
    operation.completed_at = None;
    storage_probe
        .begin_journaled_operation(&operation)
        .expect("simulate restart before journal completion");
    let restored = MemoryEngine::new(FileStorage::with_host_id(&root, "reflection_test"));
    assert_eq!(
        restored
            .recover_pending_memory_operations()
            .expect("recover completed reflection effects"),
        1
    );
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&unit_id)
            .expect("checkpoint stays idempotent")
            .reflection_count,
        1
    );
    engine
        .resume_reflection_actions(
            &start.pending_task.task_id,
            ReflectionActionsResult {
                schema_version: REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "live_session".to_string(),
                core_scope: Some("owner_scope".to_string()),
                actions: Vec::new(),
                no_insight: Some(ReflectionNoInsight {
                    reason: ReflectionNoInsightReason::TemporaryState,
                    explanation: "Replay must not checkpoint twice.".to_string(),
                }),
            },
        )
        .expect_err("completed reflection task cannot be replayed");
    assert_eq!(
        storage_probe
            .load_task(&start.pending_task.task_id)
            .expect("completed task remains readable")
            .state,
        TaskState::Completed
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn journaled_reflection_recovers_each_logical_apply_boundary_without_duplicates() {
    let root = unique_temp_dir("reflection_apply_boundary_recovery");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (unit_id, archive_id) = seed_valid_unit(
        &engine,
        "2026-08-01T08:05:00.000Z",
        "Space keeps returning as a long-term interest.",
        "Stable interest -> the user repeatedly returns to space.",
        0.95,
    );
    let start = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin journaled reflection");
    let task_before = storage_probe
        .load_task(&start.pending_task.task_id)
        .expect("capture pending task");
    let unit_before = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("capture unit before checkpoint");
    let archive_before = storage_probe
        .read_archive_entry_by_id(&archive_id)
        .expect("capture archive before checkpoint");
    let state_before = storage_probe
        .read_reflection_state("owner_scope")
        .expect("read reflection state")
        .expect("reflection state exists");

    let applied = engine
        .resume_reflection_actions(
            &start.pending_task.task_id,
            ReflectionActionsResult {
                schema_version: REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "live_session".to_string(),
                core_scope: Some("owner_scope".to_string()),
                actions: vec![ReflectionActionDraft::NewCandidate {
                    candidate: ReflectionCandidateDraft {
                        text: "The user has a stable interest in space.".to_string(),
                        category: "interest".to_string(),
                        subject: None,
                        confidence: 0.9,
                        evidence_summary: "Validated evidence repeats the same interest."
                            .to_string(),
                        source_memory_unit_ids: vec![unit_id.clone()],
                        supporting_archive_ids: vec![archive_id.clone()],
                        contradicting_archive_ids: Vec::new(),
                        contradicted_core_fact_ids: Vec::new(),
                        tags: vec!["space".to_string()],
                    },
                }],
                no_insight: None,
            },
        )
        .expect("complete reference reflection operation");
    let candidate_id = applied.candidates[0].candidate_id.clone();
    let operation_id = applied.operation_id.expect("operation id");
    let operation = storage_probe
        .load_journaled_operation(&operation_id)
        .expect("load reflection operation")
        .expect("reflection operation exists");

    // Boundary 1: durable intent exists, but no candidate/checkpoint/task effect survived.
    storage_probe
        .write_memory_unit(&unit_before)
        .expect("restore unit before checkpoint");
    storage_probe
        .update_archive_entry(&archive_id, &archive_before)
        .expect("restore archive before checkpoint");
    storage_probe
        .write_reflection_state(&state_before)
        .expect("restore reflection state");
    storage_probe
        .save_task(&task_before)
        .expect("restore pending task");
    fs::remove_file(
        root.join("core")
            .join("candidates")
            .join(format!("{candidate_id}.json")),
    )
    .expect("remove candidate fixture");
    fs::remove_dir_all(root.join("core").join("hypotheses"))
        .expect("remove hypothesis projection fixture");
    reopen_journal_operation(&storage_probe, &operation);
    recover_one_operation(&root);
    assert_reflection_apply_recovered(
        &storage_probe,
        &unit_id,
        &candidate_id,
        &start.pending_task.task_id,
        &operation_id,
    );

    // Boundary 2: candidate survived, checkpoint and task completion did not.
    storage_probe
        .write_memory_unit(&unit_before)
        .expect("restore unit before checkpoint");
    storage_probe
        .update_archive_entry(&archive_id, &archive_before)
        .expect("restore archive before checkpoint");
    storage_probe
        .write_reflection_state(&state_before)
        .expect("restore reflection state");
    storage_probe
        .save_task(&task_before)
        .expect("restore pending task");
    fs::remove_dir_all(root.join("core").join("hypotheses").join("evidence"))
        .expect("remove evidence projection fixture");
    reopen_journal_operation(&storage_probe, &operation);
    recover_one_operation(&root);
    assert_reflection_apply_recovered(
        &storage_probe,
        &unit_id,
        &candidate_id,
        &start.pending_task.task_id,
        &operation_id,
    );

    // Boundary 3: candidate and checkpoint survived, task completion did not.
    storage_probe
        .save_task(&task_before)
        .expect("restore pending task only");
    reopen_journal_operation(&storage_probe, &operation);
    recover_one_operation(&root);
    assert_reflection_apply_recovered(
        &storage_probe,
        &unit_id,
        &candidate_id,
        &start.pending_task.task_id,
        &operation_id,
    );

    // Boundary 4: every domain effect survived; only journal completion did not.
    reopen_journal_operation(&storage_probe, &operation);
    let restored = MemoryEngine::new(FileStorage::with_host_id(&root, "reflection_test"));
    assert!(restored
        .pending_tasks()
        .expect("ordinary startup API recovers before returning host work")
        .is_empty());
    assert_reflection_apply_recovered(
        &storage_probe,
        &unit_id,
        &candidate_id,
        &start.pending_task.task_id,
        &operation_id,
    );

    // A real conflicting write is never overwritten by replay.
    let mut conflicting_candidate = storage_probe
        .read_candidate_belief(&candidate_id)
        .expect("read candidate for conflict fixture");
    conflicting_candidate.text = "A human changed this candidate after the crash.".to_string();
    conflicting_candidate.updated_at = "2026-08-01T09:00:00.000Z".to_string();
    storage_probe
        .write_candidate_belief(&conflicting_candidate)
        .expect("write conflicting candidate fixture");
    reopen_journal_operation(&storage_probe, &operation);
    let conflicted = MemoryEngine::new(FileStorage::with_host_id(&root, "reflection_test"));
    let conflict = conflicted
        .pending_tasks()
        .expect_err("conflicting replay must fail closed");
    assert!(conflict.to_string().contains("candidate conflicts"));
    let manual_operation = storage_probe
        .load_journaled_operation(&operation_id)
        .expect("read conflicted journal")
        .expect("conflicted journal exists");
    assert_eq!(manual_operation.state, JournalState::NeedsManualReview);
    assert!(manual_operation
        .error
        .as_deref()
        .is_some_and(|error| error.contains("candidate conflicts")));

    fs::remove_dir_all(root).ok();
}

#[test]
fn typed_reflection_reinforces_one_candidate_with_later_evidence() {
    let root = unique_temp_dir("typed_reflection_reinforcement");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (first_unit, first_archive) = seed_valid_unit(
        &engine,
        "2026-08-01T08:10:00.000Z",
        "Space keeps returning as a long-term interest.",
        "Stable interest -> the user repeatedly returns to space.",
        0.95,
    );
    let first = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin first reflection");
    let created = engine
        .submit_reflection_response(
            &first.pending_task.task_id,
            LlmResponse::Ok {
                request_id: first.request.request_id,
                text: json!({
                    "schema_version": REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "core_scope": "owner_scope",
                    "actions": [{
                        "action": "new_candidate",
                        "candidate": {
                            "text": "The user has a stable interest in space.",
                            "category": "interests",
                            "confidence": 0.72,
                            "evidence_summary": "Validated evidence describes a recurring interest.",
                            "source_memory_unit_ids": [first_unit],
                            "supporting_archive_ids": [first_archive],
                            "tags": ["space"]
                        }
                    }]
                })
                .to_string(),
            },
        )
        .expect("create typed candidate");
    let candidate_id = created.candidates[0].candidate_id.clone();
    assert!(!created.candidates[0].promotion_checks.min_sources_met);

    let (later_unit, later_archive) = seed_valid_unit(
        &engine,
        "2026-08-01T08:20:00.000Z",
        "I came back to space again while discussing future plans.",
        "Stable interest -> space recurred in a later conversation.",
        0.95,
    );
    let later = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin later reflection");
    assert_eq!(later.existing_candidate_count, 1);
    let reinforced = engine
        .submit_reflection_response(
            &later.pending_task.task_id,
            LlmResponse::Ok {
                request_id: later.request.request_id,
                text: json!({
                    "schema_version": REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "core_scope": "owner_scope",
                    "actions": [{
                        "action": "reinforce_candidate",
                        "candidate_id": candidate_id,
                        "confidence": 0.88,
                        "evidence_summary": "A later validated unit independently repeats the same interest.",
                        "source_memory_unit_ids": [later_unit],
                        "supporting_archive_ids": [later_archive],
                        "tags": ["recurring"]
                    }]
                })
                .to_string(),
            },
        )
        .expect("reinforce candidate");

    assert!(reinforced.candidates.is_empty());
    assert_eq!(reinforced.reinforced_candidates.len(), 1);
    let candidate = &reinforced.reinforced_candidates[0];
    assert_eq!(candidate.candidate_id, candidate_id);
    assert_eq!(candidate.confidence, 0.88);
    assert_eq!(candidate.source_memory_unit_ids.len(), 2);
    assert_eq!(candidate.reinforcements.len(), 1);
    assert!(candidate.promotion_checks.min_sources_met);
    assert_eq!(
        candidate.reinforcements[0].source_memory_unit_ids,
        vec![later_unit.clone()]
    );
    assert_eq!(
        storage_probe.read_candidate_beliefs().unwrap().items.len(),
        1
    );
    let operation_id = reinforced
        .operation_id
        .as_deref()
        .expect("reinforcement operation id")
        .to_string();
    let operation = storage_probe
        .load_journaled_operation(&operation_id)
        .expect("load reinforcement journal")
        .expect("reinforcement journal exists");
    reopen_journal_operation(&storage_probe, &operation);
    recover_one_operation(&root);
    let replayed = storage_probe
        .read_candidate_belief(&candidate_id)
        .expect("read replayed candidate");
    assert_eq!(replayed.reinforcements.len(), 1);
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&later_unit)
            .expect("read replayed checkpoint")
            .reflection_count,
        1
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn typed_reinforcement_rejects_reused_evidence_without_checkpointing_new_memory() {
    let root = unique_temp_dir("typed_reflection_reused_evidence");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (first_unit, first_archive) = seed_valid_unit(
        &engine,
        "2026-08-01T08:40:00.000Z",
        "Space is a recurring interest.",
        "Stable interest -> the user repeatedly returns to space.",
        0.95,
    );
    let first = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin first reflection");
    let created = engine
        .resume_reflection_analysis(
            &first.pending_task.task_id,
            ReflectionAnalyzeResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "live_session".to_string(),
                core_scope: Some("owner_scope".to_string()),
                candidates: vec![ReflectionCandidateDraft {
                    text: "The user has a stable interest in space.".to_string(),
                    category: "interests".to_string(),
                    subject: None,
                    confidence: 0.75,
                    evidence_summary: "Validated recurring interest.".to_string(),
                    source_memory_unit_ids: vec![first_unit.clone()],
                    supporting_archive_ids: vec![first_archive.clone()],
                    contradicting_archive_ids: Vec::new(),
                    contradicted_core_fact_ids: Vec::new(),
                    tags: vec!["space".to_string()],
                }],
            },
        )
        .expect("create legacy-compatible candidate");
    let candidate_id = created.candidates[0].candidate_id.clone();
    let (new_unit, _) = seed_valid_unit(
        &engine,
        "2026-08-01T08:50:00.000Z",
        "Space appeared again in a later conversation.",
        "Stable interest -> space recurred later.",
        0.95,
    );
    let later = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin later reflection");
    let error = engine
        .submit_reflection_response(
            &later.pending_task.task_id,
            LlmResponse::Ok {
                request_id: later.request.request_id,
                text: json!({
                    "schema_version": REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "core_scope": "owner_scope",
                    "actions": [{
                        "action": "reinforce_candidate",
                        "candidate_id": candidate_id,
                        "confidence": 0.90,
                        "evidence_summary": "This cites only evidence the candidate already had.",
                        "source_memory_unit_ids": [first_unit],
                        "supporting_archive_ids": [first_archive]
                    }]
                })
                .to_string(),
            },
        )
        .expect_err("reused evidence must fail closed");
    assert!(error
        .to_string()
        .contains("at least one new task memory unit"));
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&new_unit)
            .expect("new unit remains eligible")
            .reflection_count,
        0
    );
    assert_eq!(
        storage_probe
            .read_candidate_belief(&candidate_id)
            .expect("candidate remains unchanged")
            .confidence,
        0.75
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn typed_reflection_rejects_mixed_in_task_and_foreign_evidence() {
    let root = unique_temp_dir("typed_reflection_foreign_evidence");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (unit_id, archive_id) = seed_valid_unit(
        &engine,
        "2026-08-01T08:55:00.000Z",
        "A validated in-scope event.",
        "Stable pattern -> one in-scope unit.",
        0.95,
    );
    let start = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin typed reflection");
    let error = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id,
                text: json!({
                    "schema_version": REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "core_scope": "owner_scope",
                    "actions": [{
                        "action": "new_candidate",
                        "candidate": {
                            "text": "The user has a stable pattern.",
                            "category": "pattern",
                            "confidence": 0.80,
                            "evidence_summary": "The response mixes visible and foreign evidence.",
                            "source_memory_unit_ids": [unit_id, "mu_foreign"],
                            "supporting_archive_ids": [archive_id],
                            "tags": ["pattern"]
                        }
                    }]
                })
                .to_string(),
            },
        )
        .expect_err("any foreign evidence must reject the whole typed action");
    assert!(error.to_string().contains("outside the reflection task"));
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&unit_id)
            .expect("trigger remains eligible")
            .reflection_count,
        0
    );
    assert!(storage_probe
        .read_candidate_beliefs()
        .unwrap()
        .items
        .is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn typed_core_challenge_stays_reviewable_until_owner_approval() {
    let root = unique_temp_dir("typed_reflection_core_challenge");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let old_fact = engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "location".to_string(),
            scope: Some("owner_scope".to_string()),
            subject: None,
            text: "The user lives in Berlin.".to_string(),
            confidence: 0.95,
            tags: vec!["manual".to_string()],
            source_archive_ids: Vec::new(),
            source_candidate_id: None,
        })
        .expect("seed core fact")
        .fact;
    let (unit_id, archive_id) = seed_valid_unit(
        &engine,
        "2026-08-01T08:30:00.000Z",
        "I moved back to Kyiv; Berlin is outdated.",
        "Location update -> the user now lives in Kyiv.",
        0.95,
    );
    let start = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin challenge reflection");
    let applied = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id,
                text: json!({
                    "schema_version": REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION,
                    "source_session_id": "live_session",
                    "core_scope": "owner_scope",
                    "actions": [{
                        "action": "challenge_core_fact",
                        "candidate": {
                            "text": "The user lives in Kyiv now.",
                            "category": "location",
                            "confidence": 0.95,
                            "evidence_summary": "A validated update says Berlin is outdated.",
                            "source_memory_unit_ids": [unit_id],
                            "supporting_archive_ids": [archive_id],
                            "contradicting_archive_ids": [archive_id],
                            "contradicted_core_fact_ids": [old_fact.core_fact_id],
                            "tags": ["location_update"]
                        }
                    }]
                })
                .to_string(),
            },
        )
        .expect("create reviewable challenge");
    let challenge = &applied.candidates[0];
    assert!(challenge.tags.iter().any(|tag| tag == "core_challenge"));
    let before_review = storage_probe
        .read_core_store_category("location")
        .expect("read unchanged Core");
    assert_eq!(before_review.facts[0].status, CoreFactStatus::Active);

    let review = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: challenge.candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: Some("Confirmed location change.".to_string()),
            core_scope: Some("owner_scope".to_string()),
        })
        .expect("approve challenge");
    assert_eq!(review.contested_facts.len(), 1);
    assert_eq!(review.contested_facts[0].status, CoreFactStatus::Contested);
    fs::remove_dir_all(root).ok();
}

#[test]
fn confirming_contradicting_candidate_marks_existing_core_fact_contested() {
    let root = unique_temp_dir("reflection_candidate_contests_core");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    let old_fact = engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "location".to_string(),
            scope: Some("live_session".to_string()),
            subject: None,
            text: "The user lives in Berlin.".to_string(),
            confidence: 0.95,
            tags: vec!["manual".to_string()],
            source_archive_ids: Vec::new(),
            source_candidate_id: None,
        })
        .expect("seed old core fact")
        .fact;

    ingest_text(
        &engine,
        "2026-05-31T21:00:00.000Z",
        "I moved back to Kyiv, so Berlin is outdated now.",
    );
    let sleep = engine.sleep("live_session").expect("sleep stage1");
    let unit_archive = engine
        .resume_memory_unit_pass(
            &sleep
                .memory_unit_task
                .as_ref()
                .expect("memory unit task")
                .task_id,
            MemoryUnitPassResult {
                schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                memory_units: vec![MemoryUnitDraft {
                    thesis:
                        "Location update -> the user moved back to Kyiv and Berlin is outdated."
                            .to_string(),
                    subject_hint: None,
                    speaker_id: None,
                    source_event_ids: sleep.archive_entry.source_event_ids.clone(),
                    evidence: Some("The user directly said Berlin is outdated now.".to_string()),
                    tags: vec!["location".to_string(), "core_path".to_string()],
                    weight: 0.95,
                }],
            },
        )
        .expect("resume memory unit pass");
    let unit_id = unit_archive.memory_units[0].memory_unit_id.clone();

    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                gist: "The user updated their location.".to_string(),
                narrative: "The user said they moved back to Kyiv and Berlin is outdated."
                    .to_string(),
                compact_memory: None,
                facts: Vec::new(),
                quotes: Vec::new(),
                tags: vec!["location".to_string()],
                theme: Some("location_update".to_string()),
                weight: 0.95,
                links: Vec::new(),
                emotional_markers: Vec::new(),
                topic_thread: Vec::new(),
                personal_signals: Vec::new(),
                relational_tone: None,
            },
        )
        .expect("resume sleep compression");

    let fidelity = engine
        .begin_memory_fidelity_pass(&unit_id)
        .expect("begin fidelity");
    engine
        .resume_memory_fidelity_pass(
            &fidelity.pending_task.task_id,
            FidelityReview {
                schema_version: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                memory_unit_id: unit_id.clone(),
                archive_id: sleep.archive_entry.archive_id.clone(),
                status: FidelityStatus::Valid,
                confidence: 0.98,
                explanation: "The source directly supports the location update.".to_string(),
                revised_thesis: None,
                missing_detail: None,
            },
        )
        .expect("mark unit valid");

    let start = engine
        .begin_reflection_analysis("live_session", Some("live_session".to_string()))
        .expect("begin reflection");
    let candidates = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id.clone(),
                text: json!({
                    "schema_version": "reflection_result.v1",
                    "source_session_id": "live_session",
                    "core_scope": "live_session",
                    "candidates": [{
                        "text": "The user lives in Kyiv now.",
                        "category": "location",
                        "confidence": 0.95,
                        "evidence_summary": "A validated memory unit says the user moved back to Kyiv and Berlin is outdated.",
                        "source_memory_unit_ids": [unit_id.clone()],
                        "supporting_archive_ids": [sleep.archive_entry.archive_id.clone()],
                        "contradicting_archive_ids": [sleep.archive_entry.archive_id.clone()],
                        "contradicted_core_fact_ids": [old_fact.core_fact_id.clone()],
                        "tags": ["reflection", "location_update"]
                    }]
                })
                .to_string(),
            },
        )
        .expect("submit reflection");
    assert_eq!(candidates.candidates.len(), 1);
    assert!(
        !candidates.candidates[0]
            .promotion_checks
            .no_recent_contradiction
    );

    let review = engine
        .review_candidate(CandidateReviewInput {
            schema_version: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            candidate_id: candidates.candidates[0].candidate_id.clone(),
            reviewed_by: "owner".to_string(),
            decision: ReviewDecision::Approved,
            note: Some("The update supersedes the old location.".to_string()),
            core_scope: Some("live_session".to_string()),
        })
        .expect("confirm candidate");
    assert_eq!(review.candidate.status, CandidateStatus::Promoted);
    assert_eq!(review.contested_facts.len(), 1);
    assert_eq!(review.contested_facts[0].status, CoreFactStatus::Contested);

    let core = storage_probe
        .read_core_store_category("location")
        .expect("read core category");
    let old = core
        .facts
        .iter()
        .find(|fact| fact.core_fact_id == old_fact.core_fact_id)
        .expect("old fact");
    assert_eq!(old.status, CoreFactStatus::Contested);
    assert!(old
        .links
        .iter()
        .any(|link| link.kind == "contested_by_candidate"
            && link.target == candidates.candidates[0].candidate_id));

    let promoted = review.promoted_fact.expect("promoted fact");
    assert_eq!(promoted.status, CoreFactStatus::Active);
    assert_eq!(
        promoted.source_candidate_id.as_deref(),
        Some(candidates.candidates[0].candidate_id.as_str())
    );

    let context = engine
        .core_context_package(CoreContextRequest {
            schema_version: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            session_id: "live_session".to_string(),
            domain_state: json!({}),
            core_scope: Some("live_session".to_string()),
            query_text: Some("where does the user live?".to_string()),
            query_embedding: None,
            explain_recall: false,
            recall_limit: 0,
            session_recent_limit: 0,
            session_trace_event_limit: 0,
            include_core: true,
            utc_offset_minutes: 0,
            clock_untrusted: false,
            token_budget: None,
        })
        .expect("context package");
    assert!(context
        .core_facts
        .iter()
        .any(|fact| fact.core_fact_id == old_fact.core_fact_id
            && fact.status == CoreFactStatus::Contested));

    fs::remove_dir_all(root).ok();
}

#[test]
fn automatic_reflection_is_due_for_critical_memory_and_checkpoints_zero_candidates() {
    let root = unique_temp_dir("automatic_reflection_critical_checkpoint");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (unit_id, _) = seed_valid_unit(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "This changes how I see my future.",
        "Future outlook changed -> the user described a major personal shift.",
        0.95,
    );

    let status = engine
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("evaluate reflection trigger");
    assert!(status.due);
    assert_eq!(status.reason, ReflectionTriggerReason::CriticalMemory);
    assert_eq!(status.eligible_memory_unit_count, 1);

    let automatic = engine
        .begin_reflection_if_due(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin automatic reflection");
    let start = automatic.reflection.expect("reflection should start");

    let duplicate = engine
        .begin_reflection_if_due(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("duplicate trigger check");
    assert!(duplicate.reflection.is_none());
    assert_eq!(
        duplicate.status.reason,
        ReflectionTriggerReason::AlreadyPending
    );
    assert_eq!(
        duplicate.status.pending_task_id.as_deref(),
        Some(start.pending_task.task_id.as_str())
    );

    let result = engine
        .submit_reflection_response(
            &start.pending_task.task_id,
            LlmResponse::Ok {
                request_id: start.request.request_id,
                text: json!({
                    "schema_version": "reflection_result.v1",
                    "source_session_id": "live_session",
                    "core_scope": "live_session",
                    "candidates": []
                })
                .to_string(),
            },
        )
        .expect("complete zero-candidate reflection");
    assert!(result.candidates.is_empty());

    let unit = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read reflected memory unit");
    assert_eq!(unit.reflection_count, 1);
    assert!(unit.last_reflected_at.is_some());
    let state = storage_probe
        .read_reflection_state("live_session")
        .expect("read reflection state")
        .expect("reflection state exists");
    assert!(state.last_completed_at.is_some());
    assert_eq!(state.last_candidate_count, 0);

    let after = engine
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("evaluate after checkpoint");
    assert!(!after.due);
    assert_eq!(after.reason, ReflectionTriggerReason::NoEligibleMemory);

    fs::remove_dir_all(root).ok();
}

#[test]
fn reflection_status_explains_units_excluded_by_fidelity() {
    let root = unique_temp_dir("reflection_fidelity_visibility");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    seed_valid_unit(
        &engine,
        "2026-07-01T10:00:00.000Z",
        "A validated memory.",
        "Validated detail -> supported by source.",
        0.40,
    );
    let (unchecked_id, _) = seed_valid_unit(
        &engine,
        "2026-07-01T11:00:00.000Z",
        "An unchecked memory.",
        "Unchecked detail -> waiting for validation.",
        0.70,
    );
    let mut unchecked = storage_probe
        .read_memory_unit_by_id(&unchecked_id)
        .expect("read second unit");
    unchecked.fidelity_status = FidelityStatus::Unchecked;
    unchecked.fidelity_review = None;
    storage_probe
        .write_memory_unit(&unchecked)
        .expect("restore unchecked fixture state");

    let status = engine
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("reflection status");
    assert_eq!(status.active_memory_unit_count, 2);
    assert_eq!(status.valid_memory_unit_count, 1);
    assert_eq!(status.unchecked_memory_unit_count, 1);
    assert_eq!(status.fidelity_excluded_memory_unit_count, 1);
    assert_eq!(status.eligible_memory_unit_count, 1);
    assert_eq!(status.accumulated_salience, 0.40);
    assert_eq!(status.salience_remaining, 2.10);
    assert_eq!(status.reason, ReflectionTriggerReason::NotDue);

    fs::remove_dir_all(root).ok();
}

#[test]
fn automatic_reflection_uses_accumulated_salience_and_scheduled_safety_net() {
    let root = unique_temp_dir("automatic_reflection_accumulated_and_scheduled");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);

    for (timestamp, thesis) in [
        (
            "2026-07-02T10:00:00.000Z",
            "Creative project -> the user returned to the same long-running work.",
        ),
        (
            "2026-07-02T11:00:00.000Z",
            "Creative project -> the user described another sustained effort.",
        ),
        (
            "2026-07-02T12:00:00.000Z",
            "Creative project -> the user connected the work to future plans.",
        ),
    ] {
        seed_valid_unit(&engine, timestamp, thesis, thesis, 0.84);
    }

    let accumulated = engine
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("evaluate accumulated salience");
    assert!(accumulated.due);
    assert_eq!(
        accumulated.reason,
        ReflectionTriggerReason::AccumulatedSalience
    );

    for unit in storage_probe
        .read_archive(&Default::default())
        .expect("read archives")
        .into_items()
        .into_iter()
        .flat_map(|archive| {
            storage_probe
                .read_memory_units_for_archive(&archive.archive_id)
                .expect("read units")
                .into_items()
        })
    {
        let mut unit = unit;
        unit.reflection_count = 1;
        unit.last_reflected_at = Some("2026-07-02T13:00:00.000Z".to_string());
        storage_probe
            .write_memory_unit(&unit)
            .expect("mark prior unit reflected");
    }

    seed_valid_unit(
        &engine,
        "2026-07-20T10:00:00.000Z",
        "A small new preference appeared.",
        "Preference signal -> the user mentioned a modest recurring preference.",
        0.40,
    );
    seed_valid_unit(
        &engine,
        "2026-07-20T11:00:00.000Z",
        "The preference appeared again.",
        "Preference signal -> the user repeated the modest preference.",
        0.40,
    );
    storage_probe
        .write_reflection_state(&ReflectionState {
            schema_version: REFLECTION_STATE_SCHEMA_VERSION.to_string(),
            reflection_scope: "live_session".to_string(),
            source_session_ids: vec!["live_session".to_string()],
            source_session_id: "live_session".to_string(),
            created_at: "2026-07-01T00:00:00.000Z".to_string(),
            updated_at: "2026-07-01T00:00:00.000Z".to_string(),
            last_completed_at: Some("2026-07-01T00:00:00.000Z".to_string()),
            last_task_id: None,
            last_trigger_reason: None,
            last_candidate_count: 0,
        })
        .expect("seed old reflection checkpoint");

    let scheduled = engine
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::ScheduledMaintenance,
        )
        .expect("evaluate scheduled reflection");
    assert!(scheduled.due);
    assert_eq!(scheduled.reason, ReflectionTriggerReason::ScheduledInterval);

    fs::remove_dir_all(root).ok();
}

#[test]
fn automatic_reflection_survives_restart_and_failure_keeps_units_eligible() {
    let root = unique_temp_dir("automatic_reflection_restart_failure");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (unit_id, _) = seed_valid_unit(
        &engine,
        "2026-07-03T10:00:00.000Z",
        "A critical event should remain eligible after provider failure.",
        "Critical event -> the user described a major lasting change.",
        0.95,
    );
    let automatic = engine
        .begin_reflection_if_due(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin automatic reflection");
    let start = automatic.reflection.expect("reflection should start");
    let task_id = start.pending_task.task_id.clone();
    drop(engine);

    let restored = MemoryEngine::new(FileStorage::with_host_id(&root, "reflection_test"));
    let status = restored
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::ScheduledMaintenance,
        )
        .expect("read reflection status after restart");
    assert_eq!(status.reason, ReflectionTriggerReason::AlreadyPending);
    assert_eq!(status.pending_task_id.as_deref(), Some(task_id.as_str()));

    restored
        .submit_reflection_response(
            &task_id,
            LlmResponse::Err {
                request_id: start.request.request_id,
                kind: memory_engine::llm::LlmErrorKind::Transport,
                detail: "simulated provider failure".to_string(),
            },
        )
        .expect_err("provider failure should fail reflection");
    let unit = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read failed reflection unit");
    assert_eq!(unit.reflection_count, 0);
    assert!(unit.last_reflected_at.is_none());

    let retry_status = restored
        .reflection_status(
            "live_session",
            Some("live_session".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("evaluate failed reflection retry");
    assert!(retry_status.due);
    assert_eq!(retry_status.reason, ReflectionTriggerReason::CriticalMemory);

    fs::remove_dir_all(root).ok();
}

#[test]
fn automatic_reflection_accumulates_new_evidence_across_bound_sessions_only() {
    let root = unique_temp_dir("reflection_personality_scope_accumulation");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let mut options = EngineOptions::default();
    options.reflection.salience_threshold = 0.75;
    options.reflection.critical_unit_weight = 1.0;
    let engine = MemoryEngine::with_options(storage, options.clone());

    let (unit_a, _) = seed_valid_unit_for_session(
        &engine,
        "session_a",
        "2026-07-25T10:00:00.000Z",
        "The user returned to the space topic.",
        "Space interest -> the user returned to the topic in one conversation.",
        0.40,
        &["space", "recurring_interest"],
        "space",
    );
    let state = engine
        .bind_reflection_session("session_a", Some("owner_scope".to_string()))
        .expect("register first personality session without a reasoning task");
    assert_eq!(state.source_session_ids, vec!["session_a".to_string()]);
    let first = engine
        .reflection_status(
            "session_a",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("evaluate first personality session");
    assert_eq!(first.reason, ReflectionTriggerReason::NotDue);
    drop(engine);
    let engine =
        MemoryEngine::with_options(FileStorage::with_host_id(&root, "reflection_test"), options);

    let (outside_unit, _) = seed_valid_unit_for_session(
        &engine,
        "outside_session",
        "2026-07-25T11:00:00.000Z",
        "Another user discussed space.",
        "Space interest -> another scope has its own memory.",
        0.90,
        &["space"],
        "space",
    );
    let (unit_b, _) = seed_valid_unit_for_session(
        &engine,
        "session_b",
        "2026-07-25T12:00:00.000Z",
        "The same user returned to space in a later conversation.",
        "Space interest -> the user repeated the interest in another session.",
        0.40,
        &["space", "recurring_interest"],
        "space",
    );

    let automatic = engine
        .begin_reflection_if_due(
            "session_b",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin cross-session reflection");
    assert_eq!(
        automatic.status.reason,
        ReflectionTriggerReason::AccumulatedSalience
    );
    assert_eq!(
        automatic.status.source_session_ids,
        vec!["session_a".to_string(), "session_b".to_string()]
    );
    assert_eq!(automatic.status.eligible_memory_unit_count, 2);
    let start = automatic.reflection.expect("cross-session reflection task");
    assert_eq!(start.new_memory_unit_count, 2);
    assert_eq!(start.related_memory_unit_count, 0);
    let input_ids = start.pending_task.inputs["new_memory_unit_ids"]
        .as_array()
        .expect("new unit ids")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    assert!(input_ids.contains(unit_a.as_str()));
    assert!(input_ids.contains(unit_b.as_str()));
    assert!(!input_ids.contains(outside_unit.as_str()));

    engine
        .resume_reflection_analysis(
            &start.pending_task.task_id,
            ReflectionAnalyzeResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "session_b".to_string(),
                core_scope: Some("owner_scope".to_string()),
                candidates: Vec::new(),
            },
        )
        .expect("complete cross-session reflection");
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&unit_a)
            .expect("read session a unit")
            .reflection_count,
        1
    );
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&unit_b)
            .expect("read session b unit")
            .reflection_count,
        1
    );
    assert_eq!(
        storage_probe
            .read_memory_unit_by_id(&outside_unit)
            .expect("read outside unit")
            .reflection_count,
        0
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn reflection_adds_bounded_related_history_and_existing_candidates() {
    let root = unique_temp_dir("reflection_related_history_candidates");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let mut options = EngineOptions::default();
    options.reflection.critical_unit_weight = 0.85;
    options.reflection.max_related_units = 1;
    options.reflection.max_existing_candidates = 1;
    let engine = MemoryEngine::with_options(storage, options);

    let (strong_history_id, strong_archive_id) = seed_valid_unit_for_session(
        &engine,
        "history_a",
        "2026-07-20T10:00:00.000Z",
        "Space is a lasting personal interest.",
        "Space interest -> the user treats space as a lasting interest.",
        0.95,
        &["space", "stable_interest"],
        "space",
    );
    let first = engine
        .begin_reflection_if_due(
            "history_a",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin first reflection")
        .reflection
        .expect("first reflection task");
    let first_result = engine
        .resume_reflection_analysis(
            &first.pending_task.task_id,
            ReflectionAnalyzeResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "history_a".to_string(),
                core_scope: Some("owner_scope".to_string()),
                candidates: vec![ReflectionCandidateDraft {
                    text: "The user has a stable interest in space.".to_string(),
                    category: "interests".to_string(),
                    subject: None,
                    confidence: 0.82,
                    evidence_summary: "The validated memory describes a lasting interest."
                        .to_string(),
                    source_memory_unit_ids: vec![strong_history_id.clone()],
                    supporting_archive_ids: vec![strong_archive_id],
                    contradicting_archive_ids: Vec::new(),
                    contradicted_core_fact_ids: Vec::new(),
                    tags: vec!["space".to_string(), "stable_interest".to_string()],
                }],
            },
        )
        .expect("create existing candidate");
    let existing_candidate_id = first_result.candidates[0].candidate_id.clone();
    assert_eq!(
        first_result.candidates[0].source_session_ids,
        vec!["history_a".to_string()]
    );
    let mut legacy_candidate = storage_probe
        .read_candidate_belief(&existing_candidate_id)
        .expect("read candidate for legacy-scope compatibility");
    legacy_candidate.core_scope = None;
    legacy_candidate.source_session_ids.clear();
    storage_probe
        .write_candidate_belief(&legacy_candidate)
        .expect("write legacy unscoped candidate fixture");

    let (weaker_history_id, _) = seed_valid_unit_for_session(
        &engine,
        "history_b",
        "2026-07-21T10:00:00.000Z",
        "The user mentioned space again.",
        "Space interest -> the user returned to space once more.",
        0.90,
        &["space"],
        "space",
    );
    let second = engine
        .begin_reflection_if_due(
            "history_b",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin second reflection")
        .reflection
        .expect("second reflection task");
    engine
        .resume_reflection_analysis(
            &second.pending_task.task_id,
            ReflectionAnalyzeResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "history_b".to_string(),
                core_scope: Some("owner_scope".to_string()),
                candidates: Vec::new(),
            },
        )
        .expect("checkpoint second historical unit");

    let (outside_id, _) = seed_valid_unit_for_session(
        &engine,
        "outside_session",
        "2026-07-22T10:00:00.000Z",
        "An unrelated personality also discussed space.",
        "Space interest -> this belongs to another personality scope.",
        0.95,
        &["space", "stable_interest"],
        "space",
    );
    let (trigger_id, _) = seed_valid_unit_for_session(
        &engine,
        "current_session",
        "2026-07-23T10:00:00.000Z",
        "The owner connected space with long-term plans.",
        "Space interest -> the user connected the recurring interest with future plans.",
        0.95,
        &["space", "stable_interest"],
        "space",
    );
    let current = engine
        .begin_reflection_if_due(
            "current_session",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin personality reflection")
        .reflection
        .expect("personality reflection task");

    assert_eq!(current.new_memory_unit_count, 1);
    assert_eq!(current.related_memory_unit_count, 1);
    assert_eq!(current.existing_candidate_count, 1);
    assert_eq!(
        current.pending_task.inputs["personality_session_count"],
        json!(3)
    );
    assert_eq!(
        current.pending_task.inputs["new_memory_unit_ids"][0],
        json!(trigger_id)
    );
    assert_eq!(
        current.pending_task.inputs["related_memory_unit_ids"][0],
        json!(strong_history_id)
    );
    assert_ne!(
        current.pending_task.inputs["related_memory_unit_ids"][0],
        json!(weaker_history_id)
    );
    let related = current.pending_task.inputs["memory_units"]
        .as_array()
        .expect("memory units")
        .iter()
        .find(|unit| unit["evidence_role"] == "related_history")
        .expect("related history input");
    assert!(related["relation_reasons"]
        .as_array()
        .expect("relation reasons")
        .iter()
        .any(|reason| reason == "shared_tag:space"));
    assert_eq!(
        current.pending_task.inputs["existing_candidates"][0]["candidate_id"],
        json!(existing_candidate_id)
    );
    assert!(!current.pending_task.inputs["source_session_ids"]
        .as_array()
        .expect("source sessions")
        .iter()
        .any(|session| session == "history_b"));
    assert!(!current.pending_task.inputs["source_session_ids"]
        .as_array()
        .expect("source sessions")
        .iter()
        .any(|session| session == "outside_session"));
    assert!(!current.pending_task.inputs["memory_units"]
        .as_array()
        .expect("memory units")
        .iter()
        .any(|unit| unit["memory_unit_id"] == outside_id));

    fs::remove_dir_all(root).ok();
}

#[test]
fn reflection_does_not_activate_history_from_one_weak_shared_tag() {
    let root = unique_temp_dir("reflection_weak_tag_gate");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let mut options = EngineOptions::default();
    options.reflection.critical_unit_weight = 0.85;
    let engine = MemoryEngine::with_options(storage, options);

    let (history_id, _) = seed_valid_unit_for_session(
        &engine,
        "history_session",
        "2026-07-20T10:00:00.000Z",
        "The user described a work routine.",
        "Work routine -> the user described a recurring work habit.",
        0.90,
        &["personal"],
        "work",
    );
    let history = engine
        .begin_reflection_if_due(
            "history_session",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin historical reflection")
        .reflection
        .expect("historical reflection task");
    engine
        .resume_reflection_analysis(
            &history.pending_task.task_id,
            ReflectionAnalyzeResult {
                schema_version: REFLECTION_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "history_session".to_string(),
                core_scope: Some("owner_scope".to_string()),
                candidates: Vec::new(),
            },
        )
        .expect("checkpoint historical unit");

    seed_valid_unit_for_session(
        &engine,
        "current_session",
        "2026-07-21T10:00:00.000Z",
        "The user discussed a planet.",
        "Space topic -> the user discussed a planet.",
        0.90,
        &["personal"],
        "space",
    );
    let current = engine
        .begin_reflection_if_due(
            "current_session",
            Some("owner_scope".to_string()),
            ReflectionOpportunity::AfterSleep,
        )
        .expect("begin current reflection")
        .reflection
        .expect("current reflection task");
    assert_eq!(current.related_memory_unit_count, 0);
    assert!(!current.pending_task.inputs["related_memory_unit_ids"]
        .as_array()
        .expect("related ids")
        .iter()
        .any(|unit_id| unit_id == history_id.as_str()));

    fs::remove_dir_all(root).ok();
}

#[test]
fn unscoped_reflection_does_not_treat_named_core_scopes_as_a_wildcard() {
    let root = unique_temp_dir("reflection_unscoped_core_isolation");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let engine = MemoryEngine::new(storage);
    engine
        .upsert_core_fact(CoreFactInput {
            schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            category: "private_fact".to_string(),
            scope: Some("another_owner".to_string()),
            subject: None,
            text: "A fact from another personality scope.".to_string(),
            confidence: 0.95,
            tags: vec!["scope_test".to_string()],
            source_archive_ids: Vec::new(),
            source_candidate_id: None,
        })
        .expect("seed a named-scope Core fact");
    seed_valid_unit(
        &engine,
        "2026-08-01T12:00:00.000Z",
        "I keep returning to astronomy.",
        "Interest -> the user returns to astronomy.",
        0.95,
    );

    let start = engine
        .begin_reflection_analysis("live_session", None)
        .expect("begin isolated unscoped reflection");
    assert_eq!(start.core_fact_count, 0);
    assert!(start.pending_task.inputs["core_facts"]
        .as_array()
        .expect("core facts snapshot")
        .is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn reflection_revalidates_live_evidence_before_starting_durable_apply() {
    let root = unique_temp_dir("reflection_live_evidence_revalidation");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let storage_probe = storage.clone();
    let engine = MemoryEngine::new(storage);
    let (unit_id, archive_id) = seed_valid_unit(
        &engine,
        "2026-08-01T12:10:00.000Z",
        "Space is still a recurring interest.",
        "Interest -> the user repeatedly returns to space.",
        0.95,
    );
    let start = engine
        .begin_reflection_analysis("live_session", Some("owner_scope".to_string()))
        .expect("begin reflection before evidence changes");
    let mut changed_unit = storage_probe
        .read_memory_unit_by_id(&unit_id)
        .expect("read evidence unit");
    changed_unit.thesis = "Revised evidence that the reasoning task never saw.".to_string();
    changed_unit.updated_at = "2026-08-01T12:11:00.000Z".to_string();
    storage_probe
        .write_memory_unit(&changed_unit)
        .expect("invalidate evidence after task creation");

    let error = engine
        .resume_reflection_actions(
            &start.pending_task.task_id,
            ReflectionActionsResult {
                schema_version: REFLECTION_ACTIONS_RESULT_SCHEMA_VERSION.to_string(),
                source_session_id: "live_session".to_string(),
                core_scope: Some("owner_scope".to_string()),
                actions: vec![ReflectionActionDraft::NewCandidate {
                    candidate: ReflectionCandidateDraft {
                        text: "The user has a stable interest in space.".to_string(),
                        category: "interest".to_string(),
                        subject: None,
                        confidence: 0.9,
                        evidence_summary: "The task snapshot contained a supporting unit."
                            .to_string(),
                        source_memory_unit_ids: vec![unit_id.clone()],
                        supporting_archive_ids: vec![archive_id],
                        contradicting_archive_ids: Vec::new(),
                        contradicted_core_fact_ids: Vec::new(),
                        tags: vec!["space".to_string()],
                    },
                }],
                no_insight: None,
            },
        )
        .expect_err("changed evidence must stop apply before its first side effect");
    assert!(error.to_string().contains("changed after task creation"));
    assert!(storage_probe
        .read_candidate_beliefs()
        .expect("read candidates")
        .items
        .is_empty());
    assert!(!storage_probe
        .load_journaled_operations()
        .expect("read journals")
        .items
        .iter()
        .any(|operation| operation.op_type == JournalOperationType::ReflectionApply));
    assert_eq!(
        storage_probe
            .load_task(&start.pending_task.task_id)
            .expect("task remains resumable")
            .state,
        TaskState::Pending
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn reflection_state_rejects_incompatible_schema_and_scope() {
    let root = unique_temp_dir("reflection_state_contract_validation");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let state = ReflectionState {
        schema_version: REFLECTION_STATE_SCHEMA_VERSION.to_string(),
        reflection_scope: "personality_scope".to_string(),
        source_session_ids: vec!["live_session".to_string()],
        source_session_id: "live_session".to_string(),
        created_at: "2026-08-01T00:00:00.000Z".to_string(),
        updated_at: "2026-08-01T00:00:00.000Z".to_string(),
        last_completed_at: None,
        last_task_id: None,
        last_trigger_reason: None,
        last_candidate_count: 0,
    };
    storage
        .write_reflection_state(&state)
        .expect("write valid reflection state");
    let state_path = root
        .join("runs")
        .join("reflection")
        .join("personality_scope.json");

    let mut raw = serde_json::to_value(&state).expect("serialize reflection state");
    raw["schema_version"] = json!("reflection_state.v999");
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&raw).expect("serialize incompatible state"),
    )
    .expect("write incompatible state fixture");
    assert!(matches!(
        storage.read_reflection_state("personality_scope"),
        Err(MemoryEngineError::IncompatibleSchema { .. })
    ));

    raw["schema_version"] = json!(REFLECTION_STATE_SCHEMA_VERSION);
    raw["reflection_scope"] = json!("another_scope");
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&raw).expect("serialize mismatched scope"),
    )
    .expect("write mismatched scope fixture");
    assert!(matches!(
        storage.read_reflection_state("personality_scope"),
        Err(MemoryEngineError::Validation(_))
    ));

    fs::remove_dir_all(root).ok();
}

#[test]
fn reflection_recovery_fails_closed_on_an_unreadable_journal() {
    let root = unique_temp_dir("reflection_unreadable_journal");
    let storage = FileStorage::with_host_id(&root, "reflection_test");
    let engine = MemoryEngine::new(storage);
    engine
        .bind_reflection_session("live_session", Some("owner_scope".to_string()))
        .expect("initialize storage layout");
    fs::write(root.join("journal").join("broken.json"), b"{not-json")
        .expect("write corrupt journal fixture");

    let error = engine
        .recover_pending_memory_operations()
        .expect_err("recovery must not silently skip an unreadable operation");
    assert!(error
        .to_string()
        .contains("skipped unreadable journal operation"));

    fs::remove_dir_all(root).ok();
}

fn seed_valid_unit(
    engine: &MemoryEngine<FileStorage>,
    timestamp: &str,
    text: &str,
    thesis: &str,
    weight: f64,
) -> (String, String) {
    seed_valid_unit_for_session(
        engine,
        "live_session",
        timestamp,
        text,
        thesis,
        weight,
        &["reflection_test"],
        "reflection_test",
    )
}

#[allow(clippy::too_many_arguments)]
fn seed_valid_unit_for_session(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    timestamp: &str,
    text: &str,
    thesis: &str,
    weight: f64,
    tags: &[&str],
    theme: &str,
) -> (String, String) {
    ingest_text_for_session(engine, session_id, timestamp, text, tags, theme);
    let sleep = engine.sleep(session_id).expect("sleep stage1");
    let archive_id = sleep.archive_entry.archive_id.clone();
    let unit_archive = engine
        .resume_memory_unit_pass(
            &sleep
                .memory_unit_task
                .as_ref()
                .expect("memory unit task")
                .task_id,
            MemoryUnitPassResult {
                schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: archive_id.clone(),
                memory_units: vec![MemoryUnitDraft {
                    thesis: thesis.to_string(),
                    subject_hint: None,
                    speaker_id: None,
                    source_event_ids: sleep.archive_entry.source_event_ids.clone(),
                    evidence: Some(text.to_string()),
                    tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                    weight,
                }],
            },
        )
        .expect("resume memory unit pass");
    let unit_id = unit_archive.memory_units[0].memory_unit_id.clone();
    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            SleepCompressionResult {
                schema_version: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                archive_id: archive_id.clone(),
                gist: thesis.to_string(),
                narrative: thesis.to_string(),
                compact_memory: None,
                facts: Vec::new(),
                quotes: Vec::new(),
                tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                theme: Some(theme.to_string()),
                weight,
                links: Vec::new(),
                emotional_markers: Vec::new(),
                topic_thread: Vec::new(),
                personal_signals: Vec::new(),
                relational_tone: None,
            },
        )
        .expect("resume sleep compression");
    let fidelity = engine
        .begin_memory_fidelity_pass(&unit_id)
        .expect("begin fidelity");
    engine
        .resume_memory_fidelity_pass(
            &fidelity.pending_task.task_id,
            FidelityReview {
                schema_version: FIDELITY_REVIEW_SCHEMA_VERSION.to_string(),
                memory_unit_id: unit_id.clone(),
                archive_id: archive_id.clone(),
                status: FidelityStatus::Valid,
                confidence: 0.98,
                explanation: "The thesis is supported by the source event.".to_string(),
                revised_thesis: None,
                missing_detail: None,
            },
        )
        .expect("mark unit valid");
    (unit_id, archive_id)
}

fn ingest_text(engine: &MemoryEngine<FileStorage>, timestamp: &str, text: &str) {
    ingest_text_for_session(
        engine,
        "live_session",
        timestamp,
        text,
        &["reflection_test"],
        "space",
    );
}

fn ingest_text_for_session(
    engine: &MemoryEngine<FileStorage>,
    session_id: &str,
    timestamp: &str,
    text: &str,
    tags: &[&str],
    theme: &str,
) {
    engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test_user".to_string(),
            timestamp: timestamp.to_string(),
            personality_scope_id: None,
            session_id: session_id.to_string(),
            memory_text: None,
            payload: json!({ "text": text }),
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            theme: Some(theme.to_string()),
            emotional_tone: None,
            speaker: None,
            quoted_event_ids: None,
            links: vec![],
            importance_hint: memory_engine::types::ImportanceHint::High,
            processing_mode: Default::default(),
        })
        .expect("ingest text");
}

fn reopen_journal_operation(storage: &FileStorage, completed: &JournalOperation) {
    let mut operation = completed.clone();
    operation.state = JournalState::Started;
    operation.completed_at = None;
    storage
        .begin_journaled_operation(&operation)
        .expect("reopen journal operation fixture");
}

fn recover_one_operation(root: &std::path::Path) {
    let restored = MemoryEngine::new(FileStorage::with_host_id(root, "reflection_test"));
    assert_eq!(
        restored
            .recover_pending_memory_operations()
            .expect("recover one reflection operation"),
        1
    );
}

fn assert_reflection_apply_recovered(
    storage: &FileStorage,
    unit_id: &str,
    candidate_id: &str,
    task_id: &str,
    operation_id: &str,
) {
    let unit = storage
        .read_memory_unit_by_id(unit_id)
        .expect("read recovered unit");
    assert_eq!(unit.reflection_count, 1);
    assert_eq!(
        unit.last_reflection_operation_id.as_deref(),
        Some(operation_id)
    );
    let candidates = storage
        .read_candidate_beliefs()
        .expect("read recovered candidates")
        .into_items();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].candidate_id, candidate_id);
    let nodes = storage
        .read_hypothesis_nodes()
        .expect("read recovered hypothesis nodes")
        .into_items();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].hypothesis_id, candidate_id);
    let evidence = storage
        .read_hypothesis_evidence_links()
        .expect("read recovered hypothesis evidence")
        .into_items();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].hypothesis_id, candidate_id);
    let task = storage.load_task(task_id).expect("read recovered task");
    assert_eq!(task.state, TaskState::Completed);
    assert_eq!(
        task.outcome
            .as_ref()
            .and_then(|outcome| outcome.get("operation_id"))
            .and_then(serde_json::Value::as_str),
        Some(operation_id)
    );
    assert_eq!(
        storage
            .load_journaled_operation(operation_id)
            .expect("read recovered journal")
            .expect("recovered journal exists")
            .state,
        JournalState::Completed
    );
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
