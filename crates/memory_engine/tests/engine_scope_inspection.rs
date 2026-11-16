use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::archive::{ArchiveEntry, MemoryUnit, MemoryUnitStatus};
use memory_engine::event::StoredEvent;
use memory_engine::storage::Storage;
use memory_engine::types::{Quote, WeightedFact};
use memory_engine::{FileStorage, MemoryEngine, MemoryEngineError};
use serde_json::{json, Value};

const OUR_SCOPE: &str = "shared:scope";
const OTHER_SCOPE: &str = "other_scope";
const FIRST_SESSION: &str = "first_session";
const SECOND_SESSION: &str = "second_session";
const OTHER_SESSION: &str = "other_session";
const PRIVATE_TEXT: &str = "FOREIGN PRIVATE TEXT";

#[test]
fn inspection_isolates_known_ids_and_sources_across_scopes_after_restart() {
    let fixture = Fixture::new("scope_boundary");
    let engine = fixture.open();
    let first_source = ingest(
        &engine,
        OUR_SCOPE,
        FIRST_SESSION,
        "I bought a blue notebook.",
    );
    let second_source = ingest(&engine, OUR_SCOPE, SECOND_SESSION, "I saved a recipe.");
    let foreign_source = ingest(&engine, OTHER_SCOPE, OTHER_SESSION, PRIVATE_TEXT);
    let (first_archive, first_unit) = publish_archived_unit(&engine, &first_source);
    let (second_archive, _) = publish_archived_unit(&engine, &second_source);
    let (foreign_archive, foreign_unit) = publish_archived_unit(&engine, &foreign_source);

    let our_review = engine
        .begin_memory_fidelity_pass(&first_unit.memory_unit_id)
        .unwrap();
    let foreign_review = engine
        .begin_memory_fidelity_pass(&foreign_unit.memory_unit_id)
        .unwrap();
    add_core_fact(&engine, Some(OUR_SCOPE), "Fact in our scope");
    add_core_fact(&engine, Some(OTHER_SCOPE), "Fact in the foreign scope");
    add_core_fact(&engine, None, "Shared Core fact");

    // Reconstruct a foreign-scope source within the same physical session.
    // Session identity alone cannot authorize each event in it.
    let mut mixed_session_source = foreign_source.clone();
    mixed_session_source.event_id = "foreign_event_in_shared_session".into();
    mixed_session_source.session_id = FIRST_SESSION.into();
    let mut events = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(
            fixture
                .0
                .join("sessions")
                .join(FIRST_SESSION)
                .join("events.jsonl"),
        )
        .unwrap();
    writeln!(
        events,
        "{}",
        serde_json::to_string(&mixed_session_source).unwrap()
    )
    .unwrap();
    drop(events);

    let mut linked_archive = first_archive.clone();
    let foreign_ids = [
        foreign_source.event_id.clone(),
        mixed_session_source.event_id.clone(),
    ];
    linked_archive.source_event_ids.extend(foreign_ids.clone());
    linked_archive.gist = format!("A notebook purchase and {PRIVATE_TEXT}.");
    linked_archive.narrative = format!("The combined conversation included {PRIVATE_TEXT}.");
    linked_archive.compact_memory = Some(PRIVATE_TEXT.into());
    linked_archive.facts = vec![
        WeightedFact {
            text: "Verified notebook purchase.".into(),
            confidence: 1.0,
            source_event_ids: vec![first_source.event_id.clone()],
        },
        WeightedFact {
            text: PRIVATE_TEXT.into(),
            confidence: 1.0,
            source_event_ids: foreign_ids.to_vec(),
        },
    ];
    linked_archive.quotes = vec![
        Quote {
            text: "I bought a blue notebook.".into(),
            source_event_id: Some(first_source.event_id.clone()),
        },
        Quote {
            text: PRIVATE_TEXT.into(),
            source_event_id: Some(mixed_session_source.event_id.clone()),
        },
    ];
    engine
        .storage()
        .update_archive_entry(&linked_archive.archive_id, &linked_archive)
        .unwrap();
    let mut linked_unit = first_unit.clone();
    linked_unit.source_event_ids.extend(foreign_ids);
    linked_unit.thesis = format!("A notebook purchase and {PRIVATE_TEXT}.");
    linked_unit.evidence = Some(format!("Part of the evidence is {PRIVATE_TEXT}."));
    engine.storage().write_memory_unit(&linked_unit).unwrap();
    drop(engine);

    let engine = fixture.open();
    let summary = serde_json::to_value(engine.inspect_scope(OUR_SCOPE).unwrap()).unwrap();
    let counts = &summary["counts"];
    assert_eq!(counts["sessions"], 2);
    assert_eq!(counts["complete_archives"], 2);
    assert_eq!(counts["active_memory_units"], 2);
    assert_eq!(counts["core_facts"], 1);
    assert_eq!(counts["pending_by_type"]["memory_fidelity_pass"], 1);
    let sessions = summary["session_ids"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|session| session == FIRST_SESSION));
    assert!(sessions.iter().any(|session| session == SECOND_SESSION));
    let archives = summary["archives"].as_array().unwrap();
    assert_eq!(archives.len(), 2);
    for archive in [&first_archive, &second_archive] {
        assert!(archives
            .iter()
            .any(|entry| entry["archive_id"] == archive.archive_id));
    }
    let operations = summary["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["task_id"], our_review.pending_task.task_id);
    let serialized_summary = summary.to_string();
    assert!(!serialized_summary.contains(&foreign_archive.archive_id));
    assert!(!serialized_summary.contains(&foreign_review.pending_task.task_id));
    assert!(!serialized_summary.contains(PRIVATE_TEXT));

    let archive = engine
        .inspect_archive(OUR_SCOPE, &first_archive.archive_id)
        .unwrap()
        .unwrap();
    let archive = serde_json::to_value(archive).unwrap();
    assert_only_own_source(
        &archive,
        &first_source,
        &foreign_source,
        &mixed_session_source,
    );
    assert_eq!(
        archive["archive"]["source_event_ids"],
        json!([first_source.event_id])
    );
    assert_eq!(
        archive["archive"]["memory_units"][0]["source_event_ids"],
        json!([first_source.event_id])
    );
    assert_eq!(
        archive["archive"]["facts"][0]["text"],
        "Verified notebook purchase."
    );
    assert_eq!(
        archive["archive"]["quotes"][0]["text"],
        "I bought a blue notebook."
    );
    let unit = engine
        .inspect_memory_unit(OUR_SCOPE, &first_unit.memory_unit_id)
        .unwrap()
        .unwrap();
    let unit = serde_json::to_value(unit).unwrap();
    assert_only_own_source(&unit, &first_source, &foreign_source, &mixed_session_source);
    assert_eq!(
        unit["unit"]["source_event_ids"],
        json!([first_source.event_id])
    );
    assert_eq!(unit["unit"]["status"], "active_archive");
    let second = engine
        .inspect_archive(OUR_SCOPE, &second_archive.archive_id)
        .unwrap()
        .unwrap();
    assert_eq!(second.sources[0].event_id, second_source.event_id);
    assert_eq!(second.archive.unwrap().gist, "I saved a recipe.");

    assert!(engine
        .inspect_archive(OUR_SCOPE, &foreign_archive.archive_id)
        .unwrap()
        .is_none());
    assert!(engine
        .inspect_archive(OUR_SCOPE, "unknown_archive")
        .unwrap()
        .is_none());
    assert!(engine
        .inspect_memory_unit(OUR_SCOPE, &foreign_unit.memory_unit_id)
        .unwrap()
        .is_none());
    assert!(engine
        .inspect_memory_unit(OUR_SCOPE, "unknown_unit")
        .unwrap()
        .is_none());
    assert!(engine.inspect_scope("").is_err());

    std::fs::remove_file(
        fixture
            .0
            .join("archive")
            .join("units")
            .join(format!("{}.json", first_unit.memory_unit_id)),
    )
    .unwrap();
    let damaged = engine
        .inspect_archive(OUR_SCOPE, &first_archive.archive_id)
        .unwrap()
        .unwrap();
    assert!(damaged.archive.is_none());
    assert!(!damaged.warnings.is_empty());
    assert!(!engine.inspect_scope(OUR_SCOPE).unwrap().warnings.is_empty());
    assert!(engine
        .inspect_scope(OTHER_SCOPE)
        .unwrap()
        .warnings
        .is_empty());
    assert!(engine
        .inspect_archive(OTHER_SCOPE, &first_archive.archive_id)
        .unwrap()
        .is_none());
}

#[test]
fn forgotten_memory_is_auditable_and_requires_scoped_explicit_restoration() {
    let fixture = Fixture::new("forgotten_inspection");
    let engine = fixture.open();
    let source = ingest(&engine, OUR_SCOPE, FIRST_SESSION, "I had soup for lunch.");
    let (archive, unit) = publish_archived_unit(&engine, &source);
    let review = engine.begin_forget_review(FIRST_SESSION).unwrap();
    let result = engine
        .resume_forget_review(
            &review.pending_task.task_id,
            serde_json::from_value(json!({
                "schema_version": "forget_review_result.v1",
                "source_session_id": FIRST_SESSION,
                "recommendations": [{
                    "memory_unit_id": unit.memory_unit_id,
                    "decision": "forget",
                    "reason": "This is a brief routine detail."
                }]
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(result.forgotten, 1);
    let archived_before = engine
        .storage()
        .read_archive_entry_by_id(&archive.archive_id)
        .unwrap();
    let forgotten_before = engine
        .storage()
        .read_memory_unit_by_id(&unit.memory_unit_id)
        .unwrap();
    assert_eq!(forgotten_before.status, MemoryUnitStatus::Forgotten);
    drop(engine);

    let engine = fixture.open();
    let detail = engine
        .inspect_memory_unit(OUR_SCOPE, &unit.memory_unit_id)
        .unwrap()
        .unwrap();
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(detail["unit"]["status"], "forgotten");
    assert_eq!(detail["unit"]["forget_review"]["decision"], "forget");
    assert_eq!(detail["unit"]["thesis"], "I had soup for lunch.");
    assert_eq!(detail["sources"][0]["event_id"], source.event_id);
    let summary = serde_json::to_value(engine.inspect_scope(OUR_SCOPE).unwrap()).unwrap();
    assert_eq!(summary["counts"]["active_memory_units"], 0);
    let archive_detail = engine
        .inspect_archive(OUR_SCOPE, &archive.archive_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(archive_detail).unwrap()["archive"]["recall_count"],
        archived_before.recall_count
    );
    assert!(engine
        .remember_back_in_scope(OTHER_SCOPE, &unit.memory_unit_id)
        .unwrap()
        .is_none());
    assert_eq!(
        engine
            .storage()
            .read_memory_unit_by_id(&unit.memory_unit_id)
            .unwrap(),
        forgotten_before
    );
    assert_eq!(
        engine
            .storage()
            .read_archive_entry_by_id(&archive.archive_id)
            .unwrap(),
        archived_before
    );
    let package = engine
        .core_context_package(
            serde_json::from_value(json!({
                "schema_version": "core_context_request.v1",
                "core_scope": OUR_SCOPE,
                "session_id": FIRST_SESSION,
                "query_text": "What did the user eat?"
            }))
            .unwrap(),
        )
        .unwrap();
    assert!(package.archive_relevant.is_empty());

    let foreign_source = ingest(&engine, OTHER_SCOPE, OTHER_SESSION, PRIVATE_TEXT);
    let mut mixed = forgotten_before.clone();
    mixed.source_event_ids.push(foreign_source.event_id);
    mixed.thesis = format!("The lunch conversation included {PRIVATE_TEXT}.");
    mixed.evidence = Some(PRIVATE_TEXT.into());
    engine.storage().write_memory_unit(&mixed).unwrap();
    assert!(matches!(
        engine.remember_back_in_scope(OUR_SCOPE, &unit.memory_unit_id),
        Err(MemoryEngineError::OperationResultUnavailable)
    ));
    assert_eq!(
        engine
            .storage()
            .read_memory_unit_by_id(&unit.memory_unit_id)
            .unwrap(),
        mixed
    );
    engine
        .storage()
        .write_memory_unit(&forgotten_before)
        .unwrap();

    let restored = engine
        .remember_back_in_scope(OUR_SCOPE, &unit.memory_unit_id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.status, MemoryUnitStatus::ActiveArchive);
    assert_eq!(
        engine
            .storage()
            .read_memory_unit_by_id(&unit.memory_unit_id)
            .unwrap()
            .status,
        restored.status
    );
}

fn assert_only_own_source(
    detail: &Value,
    own: &StoredEvent,
    foreign: &StoredEvent,
    mixed_session_source: &StoredEvent,
) {
    let sources = detail["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["event_id"], own.event_id);
    assert_eq!(sources[0]["type"], "user_message");
    assert_eq!(sources[0]["timestamp"], own.timestamp);
    assert_eq!(sources[0]["memory_text"], "I bought a blue notebook.");
    assert!(!detail["warnings"].as_array().unwrap().is_empty());
    let serialized_detail = detail.to_string();
    assert!(!serialized_detail.contains(&foreign.event_id));
    assert!(!serialized_detail.contains(&mixed_session_source.event_id));
    assert!(!serialized_detail.contains(PRIVATE_TEXT));
}

fn ingest(
    engine: &MemoryEngine<FileStorage>,
    scope: &str,
    session: &str,
    text: &str,
) -> StoredEvent {
    engine
        .ingest(
            serde_json::from_value(json!({
                "schema_version": "event.v1",
                "type": "user_message",
                "source": "scope_test",
                "timestamp": "2020-01-01T12:00:00Z",
                "personality_scope_id": scope,
                "session_id": session,
                "memory_text": text,
                "payload": { "text": text },
                "tags": ["routine"]
            }))
            .unwrap(),
        )
        .unwrap()
        .stored_event
}

fn publish_archived_unit(
    engine: &MemoryEngine<FileStorage>,
    source: &StoredEvent,
) -> (ArchiveEntry, MemoryUnit) {
    let sleep = engine.sleep(&source.session_id).unwrap();
    let archive_id = &sleep.archive_entry.archive_id;
    let thesis = source.payload["text"].as_str().unwrap();
    engine
        .resume_memory_unit_pass(
            &sleep.memory_unit_task.unwrap().task_id,
            serde_json::from_value(json!({
                "schema_version": "memory_units_result.v1",
                "archive_id": archive_id,
                "memory_units": [{
                    "thesis": thesis,
                    "source_event_ids": [source.event_id],
                    "weight": 0.2
                }]
            }))
            .unwrap(),
        )
        .unwrap();
    let archive = engine
        .resume_sleep_compression(
            &sleep.pending_task.task_id,
            serde_json::from_value(json!({
                "schema_version": "sleep_compression_result.v1",
                "archive_id": archive_id,
                "gist": thesis,
                "narrative": thesis,
                "weight": 0.2
            }))
            .unwrap(),
        )
        .unwrap();
    let unit = archive.memory_units[0].clone();
    (archive, unit)
}

fn add_core_fact(engine: &MemoryEngine<FileStorage>, scope: Option<&str>, text: &str) {
    engine
        .upsert_core_fact(
            serde_json::from_value(json!({
                "schema_version": "core_fact_input.v1",
                "category": "notes",
                "scope": scope,
                "text": text,
                "confidence": 1.0
            }))
            .unwrap(),
        )
        .unwrap();
}

struct Fixture(PathBuf);

impl Fixture {
    fn new(case: &str) -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let root = std::env::temp_dir().join(format!(
            "memory_engine_inspect_{case}_{}_{}",
            std::process::id(),
            stamp.as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        MemoryEngine::new(FileStorage::with_host_id(&self.0, "scope_inspection_test"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
