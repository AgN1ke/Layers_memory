use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::core_store::{
    CoreFact, CoreFactAttributionInput, CoreFactInput, CoreFactSubjectChange,
};
use memory_engine::storage::Storage;
use memory_engine::types::Speaker;
use memory_engine::{FileStorage, MemoryEngine, MemoryEngineError};
use serde_json::json;

const SCOPE: &str = "attribution_scope";
const OTHER_SCOPE: &str = "foreign_scope";
const CATEGORY: &str = "personal_traits";
const FIRST: &str = "first_alex";
const SECOND: &str = "second_alex";
const TEXT: &str = "Alex collects copper buttons.";

#[test]
fn scoped_attribution_is_versioned_safe_to_retry_and_preserves_core_identity() {
    let fixture = Fixture::new("replay");
    let engine = fixture.open();
    let original = upsert(&engine, SCOPE, Some(FIRST), TEXT, "original_archive");
    let foreign = upsert(&engine, OTHER_SCOPE, Some(FIRST), TEXT, "foreign_archive");

    let input = correction(
        &original,
        SCOPE,
        "fix_subject",
        CoreFactSubjectChange::Assign {
            subject: speaker(SECOND),
        },
    );
    let mut denied = input.clone();
    denied.scope_id = OTHER_SCOPE.into();
    assert!(engine
        .correct_core_fact_attribution(denied)
        .unwrap()
        .is_none());
    let mut unknown = input.clone();
    unknown.core_fact_id = "unknown_fact".into();
    assert!(engine
        .correct_core_fact_attribution(unknown)
        .unwrap()
        .is_none());
    let mut stale = input.clone();
    stale.expected_updated_at = "2000-01-01T00:00:00Z".into();
    assert!(matches!(
        engine.correct_core_fact_attribution(stale),
        Err(MemoryEngineError::OperationStale)
    ));
    assert_eq!(stored_fact(&engine, &original.core_fact_id), original);
    assert_eq!(stored_fact(&engine, &foreign.core_fact_id), foreign);

    let applied = engine
        .correct_core_fact_attribution(input.clone())
        .unwrap()
        .unwrap();
    assert!(!applied.already_applied);
    assert_eq!(applied.category, CATEGORY);
    assert_eq!(applied.fact.core_fact_id, original.core_fact_id);
    assert_eq!(applied.fact.created_at, original.created_at);
    assert_eq!(applied.fact.updated_at, applied.applied_at);
    assert_eq!(applied.fact.scope, original.scope);
    assert_eq!(applied.fact.status, original.status);
    assert_eq!(applied.fact.text, original.text);
    assert_eq!(applied.fact.confidence, original.confidence);
    assert_eq!(applied.fact.tags, original.tags);
    assert_eq!(applied.fact.source_archive_ids, original.source_archive_ids);
    assert_eq!(applied.fact.subject, Some(speaker(SECOND)));
    assert_eq!(applied.fact.attribution_history.len(), 1);
    assert!(engine.pending_tasks().unwrap().is_empty());
    drop(engine);

    let engine = fixture.open();
    let duplicate = engine
        .correct_core_fact_attribution(input.clone())
        .unwrap()
        .unwrap();
    assert!(duplicate.already_applied);
    assert_eq!(duplicate.applied_at, applied.applied_at);
    assert_eq!(duplicate.fact, applied.fact);
    let mut reused_id = input.clone();
    reused_id.expected_updated_at = duplicate.fact.updated_at.clone();
    reused_id.change = CoreFactSubjectChange::Clear;
    assert!(matches!(
        engine.correct_core_fact_attribution(reused_id),
        Err(MemoryEngineError::OperationConflict)
    ));

    let old_source_delivery =
        engine.upsert_core_fact(fact_input(SCOPE, Some(FIRST), TEXT, "original_archive"));
    assert!(matches!(
        old_source_delivery,
        Err(MemoryEngineError::OperationStale)
    ));
    let independently_sourced = upsert(&engine, SCOPE, Some(FIRST), TEXT, "independent_archive");
    assert_ne!(independently_sourced.core_fact_id, original.core_fact_id);
    assert_eq!(independently_sourced.subject, Some(speaker(FIRST)));

    let clear = correction(
        &applied.fact,
        SCOPE,
        "clear_subject",
        CoreFactSubjectChange::Clear,
    );
    let cleared = engine
        .correct_core_fact_attribution(clear)
        .unwrap()
        .unwrap();
    assert_eq!(cleared.fact.core_fact_id, original.core_fact_id);
    assert!(cleared.fact.subject.is_none());
    let original_replay = engine
        .correct_core_fact_attribution(input)
        .unwrap()
        .unwrap();
    assert!(original_replay.already_applied);
    assert_eq!(original_replay.applied_at, applied.applied_at);
    assert_eq!(original_replay.fact, cleared.fact);
    assert_eq!(stored_fact(&engine, &foreign.core_fact_id), foreign);
    assert!(engine.pending_tasks().unwrap().is_empty());
}

#[test]
fn collision_rejects_correction_and_legacy_attribution_can_be_assigned() {
    let fixture = Fixture::new("legacy");
    let engine = fixture.open();
    let unidentified = upsert(&engine, SCOPE, None, TEXT, "legacy_archive");
    let first = upsert(&engine, SCOPE, Some(FIRST), TEXT, "other_archive");
    let collision = correction(
        &unidentified,
        SCOPE,
        "colliding_correction",
        CoreFactSubjectChange::Assign {
            subject: speaker(FIRST),
        },
    );
    assert!(matches!(
        engine.correct_core_fact_attribution(collision),
        Err(MemoryEngineError::OperationConflict)
    ));
    assert_eq!(
        stored_fact(&engine, &unidentified.core_fact_id),
        unidentified
    );
    assert_eq!(stored_fact(&engine, &first.core_fact_id), first);
    let assign = correction(
        &unidentified,
        SCOPE,
        "assign_legacy_subject",
        CoreFactSubjectChange::Assign {
            subject: speaker(SECOND),
        },
    );
    let attributed = engine
        .correct_core_fact_attribution(assign)
        .unwrap()
        .unwrap();
    assert_eq!(attributed.fact.core_fact_id, unidentified.core_fact_id);
    assert_eq!(attributed.fact.subject, Some(speaker(SECOND)));
    assert_eq!(attributed.fact.text, TEXT);
    assert_eq!(stored_fact(&engine, &first.core_fact_id), first);
    assert!(engine.pending_tasks().unwrap().is_empty());
}

#[test]
fn replaying_the_archive_signal_does_not_recreate_corrected_attribution() {
    let fixture = Fixture::new("signal_replay");
    let engine = fixture.open();
    let session = "archive_signal_session";
    let event = engine
        .ingest(
            serde_json::from_value(json!({
                "schema_version": "event.v1",
                "type": "user_message",
                "source": "core_attribution_test",
                "timestamp": "2026-05-01T12:00:00Z",
                "personality_scope_id": SCOPE,
                "session_id": session,
                "memory_text": "I collect copper buttons.",
                "payload": {"text": "I collect copper buttons."},
                "speaker": speaker(FIRST),
                "quoted_event_ids": []
            }))
            .unwrap(),
        )
        .unwrap()
        .stored_event;
    let sleep = engine.sleep(session).unwrap();
    engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            serde_json::from_value(json!({
                "schema_version": "sleep_compression_result.v1",
                "archive_id": sleep.archive_entry.archive_id,
                "gist": TEXT,
                "narrative": TEXT,
                "weight": 0.5,
                "personal_signals": [{
                    "text": TEXT,
                    "category": CATEGORY,
                    "confidence": 0.97,
                    "subject_hint": "self_statement",
                    "speaker_id": FIRST,
                    "source_event_ids": [event.event_id]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
    let first_seed = engine.seed_core_from_archives().unwrap();
    assert_eq!(first_seed.created, 1);
    let original = engine
        .storage()
        .read_core_store_category(CATEGORY)
        .unwrap()
        .facts
        .remove(0);
    assert_eq!(original.subject, Some(speaker(FIRST)));
    let input = correction(
        &original,
        SCOPE,
        "correct_signal_subject",
        CoreFactSubjectChange::Assign {
            subject: speaker(SECOND),
        },
    );
    let corrected = engine
        .correct_core_fact_attribution(input)
        .unwrap()
        .unwrap();
    let replay = engine.seed_core_from_archives().unwrap();
    assert_eq!(replay.created, 0);
    assert_eq!(replay.updated, 0);
    assert_eq!(
        engine
            .storage()
            .read_core_store_category(CATEGORY)
            .unwrap()
            .facts,
        [corrected.fact]
    );
}

#[test]
fn erasing_the_former_subject_scrubs_history_but_preserves_the_corrected_fact() {
    let fixture = Fixture::new("subject_erasure");
    let engine = fixture.open();
    let mut input = fact_input(SCOPE, Some(FIRST), TEXT, "unused_archive");
    input.source_archive_ids.clear();
    let original = engine.upsert_core_fact(input).unwrap().fact;
    let correction = correction(
        &original,
        SCOPE,
        "correct_before_erasure",
        CoreFactSubjectChange::Assign {
            subject: speaker(SECOND),
        },
    );
    let applied = engine
        .correct_core_fact_attribution(correction.clone())
        .unwrap()
        .unwrap();
    assert_eq!(applied.fact.attribution_history.len(), 1);
    let report = engine
        .erase_subject(SCOPE, FIRST, "erase_former_subject")
        .unwrap();
    assert_eq!(report.deleted_core_facts, 0);
    drop(engine);

    let engine = fixture.open();
    let surviving = stored_fact(&engine, &original.core_fact_id);
    assert_eq!(surviving.subject, Some(speaker(SECOND)));
    assert_eq!(surviving.text, TEXT);
    assert!(surviving.attribution_history.is_empty());
    assert!(matches!(
        engine.correct_core_fact_attribution(correction),
        Err(MemoryEngineError::OperationStale)
    ));
    assert_eq!(stored_fact(&engine, &original.core_fact_id), surviving);
}

fn correction(
    fact: &CoreFact,
    scope: &str,
    operation_id: &str,
    change: CoreFactSubjectChange,
) -> CoreFactAttributionInput {
    CoreFactAttributionInput {
        schema_version: "core_fact_attribution_input.v1".into(),
        scope_id: scope.into(),
        core_fact_id: fact.core_fact_id.clone(),
        operation_id: operation_id.into(),
        expected_updated_at: fact.updated_at.clone(),
        change,
    }
}

fn fact_input(scope: &str, subject: Option<&str>, text: &str, archive: &str) -> CoreFactInput {
    serde_json::from_value(json!({
        "schema_version": "core_fact_input.v1",
        "category": CATEGORY,
        "scope": scope,
        "subject": subject.map(speaker),
        "text": text,
        "confidence": 0.75,
        "tags": ["owner-entered"],
        "source_archive_ids": [archive]
    }))
    .unwrap()
}

fn upsert(
    engine: &MemoryEngine<FileStorage>,
    scope: &str,
    subject: Option<&str>,
    text: &str,
    archive: &str,
) -> CoreFact {
    engine
        .upsert_core_fact(fact_input(scope, subject, text, archive))
        .unwrap()
        .fact
}

fn stored_fact(engine: &MemoryEngine<FileStorage>, id: &str) -> CoreFact {
    engine
        .storage()
        .read_core_store_category(CATEGORY)
        .unwrap()
        .facts
        .into_iter()
        .find(|fact| fact.core_fact_id == id)
        .unwrap()
}

fn speaker(id: &str) -> Speaker {
    Speaker {
        id: id.into(),
        name: "Alex".into(),
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new(case: &str) -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let root = std::env::temp_dir().join(format!(
            "memory_engine_core_attribution_{case}_{}_{}",
            std::process::id(),
            stamp.as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        MemoryEngine::new(FileStorage::with_host_id(&self.0, "core_attribution_test"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
