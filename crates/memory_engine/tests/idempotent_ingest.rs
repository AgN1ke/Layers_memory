use std::fs;
use std::path::PathBuf;
use std::sync::Barrier;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::event::IngestEvent;
use memory_engine::journal::{
    JournalOperation, JournalOperationType, JournalState, RecoveryPolicy,
};
use memory_engine::storage::Storage;
use memory_engine::types::JOURNAL_OPERATION_SCHEMA_VERSION;
use memory_engine::{FileStorage, MemoryEngine, MemoryEngineError};
use serde_json::json;

const OPERATION_ID: &str = "message-42";
const SESSION_ID: &str = "source_session";
const SCOPE_ID: &str = "source_scope";
const MESSAGE: &str = "A queued message was delivered.";

#[test]
fn repeat_after_lost_ack_and_restart_returns_the_original_event() {
    let vault = Vault::new("lost_ack");
    let mut request = event(SESSION_ID, SCOPE_ID, MESSAGE);
    request.payload["measurement"] = json!(51.248178375505404);

    // The host loses the acknowledgment. The test keeps it for comparison.
    let first = vault
        .open()
        .ingest_idempotent(OPERATION_ID, request.clone())
        .expect("apply the first delivery");
    assert!(!first.already_applied);

    let restarted = vault.open();
    let replay = restarted
        .ingest_idempotent(OPERATION_ID, request)
        .expect("retry the delivery after reopening");
    assert!(replay.already_applied);
    assert_eq!(replay.event, first.event);
    let session = restarted.storage().read_session(SESSION_ID).unwrap();
    assert_eq!(session.events, vec![first.event]);
    assert_eq!(session.metadata.event_count, 1);
    assert_eq!(vault.markdown().matches(MESSAGE).count(), 1);
}

#[test]
fn completed_receipt_cannot_restore_a_purged_event_after_restart() {
    let vault = Vault::new("purged_receipt");
    let request = event(SESSION_ID, SCOPE_ID, MESSAGE);
    let engine = vault.open();
    let first = engine
        .ingest_idempotent(OPERATION_ID, request.clone())
        .expect("apply the first delivery");
    engine
        .purge_scope(SCOPE_ID)
        .expect("purge the source scope");
    drop(engine);

    let restarted = vault.open();
    let error = restarted
        .ingest_idempotent(OPERATION_ID, request.clone())
        .expect_err("a completed receipt cannot resurrect a purged event");
    assert!(matches!(
        error,
        MemoryEngineError::OperationResultUnavailable
    ));
    assert!(restarted
        .storage()
        .read_sessions()
        .unwrap()
        .items
        .is_empty());

    let fresh = restarted
        .ingest_idempotent("message-43", request)
        .expect("accept a fresh operation after purge");
    assert!(!fresh.already_applied);
    assert_ne!(fresh.event.event_id, first.event.event_id);
    let session = restarted.storage().read_session(SESSION_ID).unwrap();
    assert_eq!(session.events, vec![fresh.event]);
    assert_eq!(session.metadata.event_count, 1);
    assert_eq!(vault.markdown().matches(MESSAGE).count(), 1);
}

#[test]
fn repeat_with_changed_payload_fails_after_restart() {
    let vault = Vault::new("payload_conflict");
    let request = event(SESSION_ID, SCOPE_ID, MESSAGE);
    let first = vault
        .open()
        .ingest_idempotent(OPERATION_ID, request.clone())
        .expect("apply the first delivery");

    let restarted = vault.open();
    let error = restarted
        .ingest_idempotent(OPERATION_ID, event(SESSION_ID, SCOPE_ID, "Changed payload"))
        .expect_err("the same operation cannot change its message");
    assert!(matches!(error, MemoryEngineError::OperationConflict));
    let original = restarted
        .ingest_idempotent(OPERATION_ID, request)
        .expect("the conflicting attempt preserves the original receipt");
    assert!(original.already_applied);
    assert_eq!(original.event, first.event);
    let session = restarted.storage().read_session(SESSION_ID).unwrap();
    assert_eq!(session.events, vec![first.event]);
    assert_eq!(session.metadata.event_count, 1);
    assert!(!vault.markdown().contains("Changed payload"));
}

#[test]
fn repeat_in_another_scope_creates_no_session_after_restart() {
    let vault = Vault::new("scope_conflict");
    let first = vault
        .open()
        .ingest_idempotent(OPERATION_ID, event(SESSION_ID, SCOPE_ID, MESSAGE))
        .expect("apply the first delivery");

    let restarted = vault.open();
    // A fresh session removes the usual session-scope consistency check from
    // the scenario. The operation ID itself must bind the replay to its scope.
    let error = restarted
        .ingest_idempotent(
            OPERATION_ID,
            event("another_session", "another_scope", MESSAGE),
        )
        .expect_err("an operation cannot replay into another scope");
    assert!(matches!(error, MemoryEngineError::OperationConflict));
    let sessions = restarted.storage().read_sessions().unwrap().into_items();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, SESSION_ID);
    let session = restarted.storage().read_session(SESSION_ID).unwrap();
    assert_eq!(session.events, vec![first.event]);
}

#[test]
fn concurrent_repeats_apply_once() {
    const DELIVERIES: usize = 6;

    let vault = Vault::new("concurrent_repeats");
    let engine = vault.open();
    let barrier = Barrier::new(DELIVERIES);
    let results = std::thread::scope(|scope| {
        (0..DELIVERIES)
            .map(|_| {
                scope.spawn(|| {
                    let request = event(SESSION_ID, SCOPE_ID, MESSAGE);
                    barrier.wait();
                    engine
                        .ingest_idempotent(OPERATION_ID, request)
                        .expect("apply or replay a concurrent delivery")
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(
        results
            .iter()
            .filter(|result| !result.already_applied)
            .count(),
        1
    );
    assert!(results
        .iter()
        .all(|result| result.event == results[0].event));
    let session = engine.storage().read_session(SESSION_ID).unwrap();
    assert_eq!(session.events, vec![results[0].event.clone()]);
    assert_eq!(session.metadata.event_count, 1);
    assert_eq!(vault.markdown().matches(MESSAGE).count(), 1);
}

#[test]
fn session_reads_see_complete_events_while_ingest_is_running() {
    let vault = Vault::new("concurrent_read");
    let engine = vault.open();
    let start = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            start.wait();
            for index in 0..24 {
                engine
                    .ingest_idempotent(
                        &format!("message-{index}"),
                        event(SESSION_ID, SCOPE_ID, MESSAGE),
                    )
                    .expect("publish a complete event");
            }
        });
        start.wait();
        for _ in 0..48 {
            let record = engine
                .read_session(SESSION_ID)
                .expect("read a session snapshot");
            assert_eq!(record.metadata.event_count, record.events.len() as u64);
            std::thread::yield_now();
        }
    });
    assert_eq!(engine.read_session(SESSION_ID).unwrap().events.len(), 24);
}

#[test]
fn pending_ingest_does_not_block_erasure_of_another_scope() {
    let vault = Vault::new("unrelated_purge");
    let engine = vault.open();
    engine.ingest(event(SESSION_ID, SCOPE_ID, MESSAGE)).unwrap();
    engine
        .ingest(event("other_session", "other_scope", "Erase this."))
        .unwrap();

    // Reproduce the interval between opening and committing a write to the
    // first session. The other scope's purge can use its own stable metadata.
    let marker = vault
        .0
        .join("sessions")
        .join(SESSION_ID)
        .join("pending-ingest.json");
    fs::write(&marker, "\"ingest_in_progress\"").unwrap();
    let pending = JournalOperation {
        schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.into(),
        op_id: "ingest_in_progress".into(),
        op_type: JournalOperationType::IngestEvent,
        state: JournalState::Started,
        created_at: "2026-05-17T16:00:00Z".into(),
        updated_at: "2026-05-17T16:00:00Z".into(),
        target_files: Vec::new(),
        intent: json!({"event": event(SESSION_ID, SCOPE_ID, "other_scope")}),
        recovery_policy: RecoveryPolicy::Retry,
        completed_at: None,
        error: None,
    };
    engine
        .storage()
        .begin_journaled_operation(&pending)
        .unwrap();
    assert!(matches!(
        engine.read_session(SESSION_ID),
        Err(MemoryEngineError::OperationIncomplete)
    ));

    let report = engine
        .purge_scope("other_scope")
        .expect("erase the independent scope");
    assert_eq!(report.deleted.sessions, 1);
    assert!(marker.exists());
    assert_eq!(
        engine
            .storage()
            .load_journaled_operation(&pending.op_id)
            .unwrap(),
        Some(pending)
    );
    assert!(engine
        .read_session("other_session")
        .unwrap()
        .events
        .is_empty());
    fs::remove_file(marker).unwrap();
    assert_eq!(engine.read_session(SESSION_ID).unwrap().events.len(), 1);
}

#[test]
fn repeat_after_raw_event_rotation_reads_the_canonical_event() {
    let vault = Vault::new("rotated_replay");
    let request = event(SESSION_ID, SCOPE_ID, MESSAGE);
    let engine = vault.open();
    let first = engine
        .ingest_idempotent(OPERATION_ID, request.clone())
        .expect("apply the first delivery");
    assert_eq!(
        engine
            .storage()
            .rotate_session_events(SESSION_ID, std::slice::from_ref(&first.event.event_id))
            .expect("rotate the stored event"),
        1
    );
    drop(engine);

    let restarted = vault.open();
    let replay = restarted
        .ingest_idempotent(OPERATION_ID, request)
        .expect("replay from the rotated segment");
    assert!(replay.already_applied);
    assert_eq!(replay.event, first.event);
    let session = restarted.storage().read_session(SESSION_ID).unwrap();
    assert!(session.events.is_empty());
    assert_eq!(session.metadata.event_count, 1);
    assert_eq!(
        restarted
            .storage()
            .read_session_archived_events(SESSION_ID)
            .unwrap(),
        vec![first.event]
    );
    assert_eq!(vault.markdown().matches(MESSAGE).count(), 1);
}

fn event(session: &str, scope: &str, text: &str) -> IngestEvent {
    serde_json::from_value(json!({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "source_user",
        "timestamp": "2026-05-17T16:00:00Z",
        "session_id": session,
        "personality_scope_id": scope,
        "payload": {"text": text}
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
        let root = std::env::temp_dir().join(format!("memory_engine_h2_ingest_{case}_{nanos}"));
        fs::create_dir(&root).expect("create disposable FileStorage directory");
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        MemoryEngine::new(FileStorage::with_host_id(&self.0, "h2_ingest_test"))
    }

    fn markdown(&self) -> String {
        fs::read_to_string(self.0.join("sessions").join(SESSION_ID).join("session.md"))
            .expect("read session Markdown")
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
