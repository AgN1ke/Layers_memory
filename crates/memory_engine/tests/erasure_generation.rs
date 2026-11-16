use std::fs;
use std::path::PathBuf;
use std::sync::Barrier;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::event::IngestEvent;
use memory_engine::{FileStorage, MemoryEngine, MemoryEngineError};
use serde_json::json;

const SCOPE: &str = "room_scope";
const SESSION: &str = "morning_session";
const SECOND_SESSION: &str = "afternoon_session";

#[test]
fn empty_scope_purge_rejects_delayed_first_deliveries_after_reopen() {
    let vault = Vault::new("empty_scope");
    let engine = vault.open();
    let admitted = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let admitted_other_session = engine
        .write_generation(SECOND_SESSION, Some(SCOPE))
        .unwrap();
    let durable_admission = serde_json::to_vec(&admitted).unwrap();
    assert!(engine.purge_scope(SCOPE).unwrap().deleted.is_empty());
    drop(engine);

    let engine = vault.open();
    assert!(matches!(
        engine.ingest_idempotent_at_generation(
            "delayed-first",
            event(SESSION, Some(SCOPE)),
            serde_json::from_slice(&durable_admission).unwrap(),
        ),
        Err(MemoryEngineError::OperationStale)
    ));
    assert!(matches!(
        engine.ingest_idempotent_at_generation(
            "delayed-second",
            event(SECOND_SESSION, Some(SCOPE)),
            admitted_other_session,
        ),
        Err(MemoryEngineError::OperationStale)
    ));
    assert!(engine.read_session(SESSION).unwrap().events.is_empty());
    assert!(engine
        .read_session(SECOND_SESSION)
        .unwrap()
        .events
        .is_empty());

    let fresh = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let first = engine
        .ingest_idempotent_at_generation("fresh", event(SESSION, Some(SCOPE)), fresh.clone())
        .expect("the scope accepts a newly admitted event");
    let replay = engine
        .ingest_idempotent_at_generation("fresh", event(SESSION, Some(SCOPE)), fresh)
        .expect("the normal protected retry remains idempotent");
    assert!(!first.already_applied);
    assert!(replay.already_applied);
    assert_eq!(first.event, replay.event);
    assert_eq!(
        engine.read_session(SESSION).unwrap().events,
        vec![first.event]
    );
}

#[test]
fn empty_session_deletion_invalidates_only_that_session_after_reopen() {
    let vault = Vault::new("empty_session");
    let engine = vault.open();
    let admitted = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let neighbor = engine
        .write_generation(SECOND_SESSION, Some(SCOPE))
        .unwrap();
    assert!(engine.delete_session(SESSION).unwrap().deleted.is_empty());
    drop(engine);

    let engine = vault.open();
    assert!(matches!(
        engine.ingest_idempotent_at_generation("delayed", event(SESSION, Some(SCOPE)), admitted),
        Err(MemoryEngineError::OperationStale)
    ));
    assert!(engine.read_session(SESSION).unwrap().events.is_empty());
    engine
        .ingest_idempotent_at_generation("neighbor", event(SECOND_SESSION, Some(SCOPE)), neighbor)
        .expect("a different session in the same scope keeps its admission");
    let fresh = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    engine
        .ingest_idempotent_at_generation("fresh", event(SESSION, Some(SCOPE)), fresh)
        .expect("a newly admitted event recreates the session");
    assert_eq!(engine.read_session(SESSION).unwrap().events.len(), 1);
    assert_eq!(engine.read_session(SECOND_SESSION).unwrap().events.len(), 1);
}

#[test]
fn admission_is_bound_to_both_scope_and_session_at_initial_generation() {
    let vault = Vault::new("binding");
    let engine = vault.open();
    let generation = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    assert!(matches!(
        engine.ingest_idempotent_at_generation(
            "other-scope",
            event(SESSION, Some("other_scope")),
            generation.clone(),
        ),
        Err(MemoryEngineError::OperationStale)
    ));
    assert!(matches!(
        engine.ingest_idempotent_at_generation(
            "other-session",
            event(SECOND_SESSION, Some(SCOPE)),
            generation,
        ),
        Err(MemoryEngineError::OperationStale)
    ));
    assert!(engine.read_session(SESSION).unwrap().events.is_empty());
    assert!(engine
        .read_session(SECOND_SESSION)
        .unwrap()
        .events
        .is_empty());
}

#[test]
fn purge_rejects_replay_of_a_committed_operation_from_an_older_generation() {
    let vault = Vault::new("committed_replay");
    let engine = vault.open();
    let generation = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let request = event(SESSION, Some(SCOPE));
    let first = engine
        .ingest_idempotent_at_generation("completed", request.clone(), generation.clone())
        .expect("publish the original event");
    assert_eq!(engine.purge_scope(SCOPE).unwrap().deleted.active_events, 1);
    drop(engine);

    let engine = vault.open();
    let fresh = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let second = engine
        .ingest_idempotent_at_generation("fresh", request.clone(), fresh)
        .expect("recreate the session at the current generation");
    assert!(matches!(
        engine.ingest_idempotent_at_generation("completed", request, generation),
        Err(MemoryEngineError::OperationStale)
    ));
    assert_ne!(first.event.event_id, second.event.event_id);
    assert_eq!(
        engine.read_session(SESSION).unwrap().events,
        vec![second.event]
    );
}

#[test]
fn concurrent_purge_cannot_leave_old_deliveries_from_its_scope() {
    let vault = Vault::new("concurrent_purge");
    let engine = vault.open();
    let first = engine.write_generation(SESSION, Some(SCOPE)).unwrap();
    let second = engine
        .write_generation(SECOND_SESSION, Some(SCOPE))
        .unwrap();
    let unrelated = engine
        .write_generation("neighbor_session", Some("neighbor_scope"))
        .unwrap();
    engine
        .ingest_idempotent_at_generation("seed", event(SESSION, Some(SCOPE)), first.clone())
        .unwrap();

    let start = Barrier::new(3);
    std::thread::scope(|threads| {
        threads.spawn(|| {
            start.wait();
            for index in 0..24 {
                let (session, generation) = if index % 2 == 0 {
                    (SESSION, &first)
                } else {
                    (SECOND_SESSION, &second)
                };
                let result = engine.ingest_idempotent_at_generation(
                    &format!("queued-{index}"),
                    event(session, Some(SCOPE)),
                    generation.clone(),
                );
                match result {
                    Ok(_) | Err(MemoryEngineError::OperationStale) => {}
                    Err(error) => panic!("unexpected result at erasure boundary: {error}"),
                }
                std::thread::yield_now();
            }
        });
        threads.spawn(|| {
            start.wait();
            engine.purge_scope(SCOPE).expect("purge racing writes");
        });
        threads.spawn(|| {
            start.wait();
            for index in 0..12 {
                engine
                    .ingest_idempotent_at_generation(
                        &format!("unrelated-{index}"),
                        event("neighbor_session", Some("neighbor_scope")),
                        unrelated.clone(),
                    )
                    .expect("purge does not invalidate another scope's admissions");
            }
        });
    });
    drop(engine);

    let engine = vault.open();
    assert!(engine.read_session(SESSION).unwrap().events.is_empty());
    assert!(engine
        .read_session(SECOND_SESSION)
        .unwrap()
        .events
        .is_empty());
    assert_eq!(
        engine
            .read_session("neighbor_session")
            .unwrap()
            .events
            .len(),
        12
    );
    assert!(matches!(
        engine.ingest_idempotent_at_generation("late", event(SESSION, Some(SCOPE)), first),
        Err(MemoryEngineError::OperationStale)
    ));
}

#[test]
fn original_idempotent_ingest_remains_usable_after_session_erasure() {
    let vault = Vault::new("legacy_retry");
    let engine = vault.open();
    let request = event(SESSION, None);
    let first = engine
        .ingest_idempotent("original", request.clone())
        .unwrap();
    let replay = engine
        .ingest_idempotent("original", request.clone())
        .unwrap();
    assert!(replay.already_applied);
    assert_eq!(first.event, replay.event);
    engine.delete_session(SESSION).unwrap();
    drop(engine);

    let engine = vault.open();
    assert!(matches!(
        engine.ingest_idempotent("original", request.clone()),
        Err(MemoryEngineError::OperationResultUnavailable)
    ));
    let fresh = engine.ingest_idempotent("legacy-fresh", request).unwrap();
    assert!(!fresh.already_applied);
    assert_ne!(fresh.event.event_id, first.event.event_id);
}

fn event(session: &str, scope: Option<&str>) -> IngestEvent {
    serde_json::from_value(json!({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "erasure_generation_test",
        "timestamp": "2026-05-17T16:00:00Z",
        "session_id": session,
        "personality_scope_id": scope,
        "payload": {"text": "A queued memory event."}
    }))
    .unwrap()
}

struct Vault(PathBuf);

impl Vault {
    fn new(case: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("memory_erasure_generation_{case}_{nanos}"));
        fs::create_dir(&root).expect("create disposable FileStorage directory");
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        MemoryEngine::new(FileStorage::with_host_id(
            &self.0,
            "erasure_generation_test",
        ))
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
