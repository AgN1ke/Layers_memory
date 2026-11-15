use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::archive::{FidelityReview, FidelityStatus, MemoryUnit, MemoryUnitStatus};
use memory_engine::event::{IngestEvent, StoredEvent};
use memory_engine::forgetting::ForgetReviewStart;
use memory_engine::llm::LlmResponse;
use memory_engine::sleep::{MemoryUnitPassResult, SleepCompressionResult};
use memory_engine::storage::Storage;
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;

const SESSION: &str = "h2_routine_session";
const THESIS: &str = "The user had soup for lunch.";

#[test]
#[ignore = "C1 baseline: late fidelity currently restores forgotten memory; enable in H2"]
fn delayed_fidelity_preserves_forgetting_after_restart() {
    let fixture = Fixture::new("delayed_fidelity");
    let engine = fixture.open();
    let unit = create_routine_unit(&engine);
    let fidelity = engine
        .begin_memory_fidelity_pass(&unit.memory_unit_id)
        .expect("begin fidelity while available");
    let forgetting = start_forgetting(&engine, &unit);
    forget(&engine, &forgetting, &unit);
    drop(engine);

    let engine = fixture.open();
    assert_eq!(
        stored_unit(&engine, &unit).status,
        MemoryUnitStatus::Forgotten
    );
    let evaluated = engine
        .resume_memory_fidelity_pass(&fidelity.pending_task.task_id, valid_review(&unit))
        .expect("valid review of unchanged evidence");

    assert_eq!(evaluated.fidelity_status, FidelityStatus::Valid);
    assert_eq!(
        stored_unit(&engine, &unit).status,
        MemoryUnitStatus::Forgotten,
        "a valid fidelity verdict cannot independently restore availability"
    );
    let restored = engine
        .remember_back(&unit.memory_unit_id)
        .expect("explicit restoration remains possible");
    assert_eq!(restored.status, MemoryUnitStatus::ActiveArchive);
    assert_eq!(restored.fidelity_status, FidelityStatus::Valid);
}

#[test]
#[ignore = "H2.3 must check frozen source evidence even if the thesis has not changed"]
fn fidelity_verdict_for_replaced_source_content_does_not_apply_after_restart() {
    let fixture = Fixture::new("edited_fidelity_source");
    let engine = fixture.open();
    let unit = create_routine_unit(&engine);
    let fidelity = engine
        .begin_memory_fidelity_pass(&unit.memory_unit_id)
        .unwrap();
    drop(engine);

    // Model an offline source correction under the same event ID. The thesis
    // and ID list stay identical; a thesis-only or ID-only digest misses it.
    let events = fixture
        .0
        .join("sessions")
        .join(SESSION)
        .join("events.jsonl");
    let mut source: StoredEvent =
        serde_json::from_str(&std::fs::read_to_string(&events).unwrap()).unwrap();
    assert_eq!(unit.source_event_ids, [source.event_id.clone()]);
    source.payload = json!({"text": "I had a sandwich for lunch."});
    std::fs::write(
        &events,
        format!("{}\n", serde_json::to_string(&source).unwrap()),
    )
    .unwrap();

    let engine = fixture.open();
    let submission =
        engine.resume_memory_fidelity_pass(&fidelity.pending_task.task_id, valid_review(&unit));
    assert_eq!(
        stored_unit(&engine, &unit),
        unit,
        "a verdict for a replaced source must not change the unit: {submission:?}"
    );
}

#[test]
#[ignore = "C1 baseline: completed forgetting reapplies after restoration; enable in H2"]
fn completed_forgetting_replay_preserves_later_restore() {
    let fixture = Fixture::new("forget_replay");
    let engine = fixture.open();
    let unit = create_routine_unit(&engine);
    let original = start_forgetting(&engine, &unit);
    forget(&engine, &original, &unit);
    let restored = engine
        .remember_back(&unit.memory_unit_id)
        .expect("explicitly restore the forgotten unit");
    assert_eq!(restored.status, MemoryUnitStatus::ActiveArchive);
    drop(engine);

    let engine = fixture.open();
    assert_eq!(stored_unit(&engine, &unit), restored);
    // The replay may return the original receipt or an explicit rejection on
    // this legacy entry point. Its effect on the newer state is the invariant.
    let replay = engine.submit_forget_review_response(
        &original.pending_task.task_id,
        forgetting_response(&original, &unit),
    );
    assert_eq!(
        stored_unit(&engine, &unit),
        restored,
        "replay of a completed decision must preserve the later restoration: {replay:?}"
    );

    let fresh = start_forgetting(&engine, &unit);
    assert_ne!(fresh.pending_task.task_id, original.pending_task.task_id);
    forget(&engine, &fresh, &unit);
    assert_eq!(
        stored_unit(&engine, &unit).status,
        MemoryUnitStatus::Forgotten
    );
}

fn create_routine_unit(engine: &MemoryEngine<FileStorage>) -> MemoryUnit {
    let event: IngestEvent = serde_json::from_value(json!({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "h2_fixture_user",
        "timestamp": "2020-01-01T10:00:00Z",
        "session_id": SESSION,
        "payload": {"text": "I had soup for lunch."},
        "tags": ["routine"]
    }))
    .expect("event fixture");
    engine.ingest(event).expect("ingest the source event");
    let sleep = engine.sleep(SESSION).expect("start Sleep");
    let units: MemoryUnitPassResult = serde_json::from_value(json!({
        "schema_version": "memory_units_result.v1",
        "archive_id": sleep.archive_entry.archive_id,
        "memory_units": [{
            "thesis": THESIS,
            "source_event_ids": sleep.archive_entry.source_event_ids,
            "weight": 0.2
        }]
    }))
    .expect("unit fixture");
    let unit_task = sleep.memory_unit_task.expect("unit task");
    engine
        .resume_memory_unit_pass(&unit_task.task_id, units)
        .expect("extract the routine unit");

    let compression: SleepCompressionResult = serde_json::from_value(json!({
        "schema_version": "sleep_compression_result.v1",
        "archive_id": sleep.archive_entry.archive_id,
        "gist": "The user mentioned their lunch.",
        "narrative": "The user described an ordinary lunch with soup.",
        "weight": 0.2
    }))
    .expect("compression fixture");
    let archive = engine
        .resume_sleep_compression(&sleep.pending_task.task_id, compression)
        .expect("complete the Archive");
    assert_eq!(archive.memory_units.len(), 1);
    archive
        .memory_units
        .into_iter()
        .next()
        .expect("routine unit")
}

fn stored_unit(engine: &MemoryEngine<FileStorage>, unit: &MemoryUnit) -> MemoryUnit {
    engine
        .storage()
        .read_memory_unit_by_id(&unit.memory_unit_id)
        .expect("read canonical memory unit from disk")
}

fn valid_review(unit: &MemoryUnit) -> FidelityReview {
    FidelityReview {
        schema_version: "fidelity_review.v1".into(),
        memory_unit_id: unit.memory_unit_id.clone(),
        archive_id: unit.archive_id.clone(),
        status: FidelityStatus::Valid,
        confidence: 0.95,
        explanation: "The original event supports the unchanged thesis.".into(),
        revised_thesis: None,
        missing_detail: None,
    }
}

fn start_forgetting(engine: &MemoryEngine<FileStorage>, unit: &MemoryUnit) -> ForgetReviewStart {
    let start = engine
        .begin_forget_review(SESSION)
        .expect("start forgetting");
    assert!(start
        .candidates
        .iter()
        .any(|candidate| candidate.memory_unit_id == unit.memory_unit_id));
    start
}

fn forgetting_response(start: &ForgetReviewStart, unit: &MemoryUnit) -> LlmResponse {
    LlmResponse::Ok {
        request_id: start.request.request_id.clone(),
        text: json!({
            "schema_version": "forget_review_result.v1",
            "source_session_id": SESSION,
            "recommendations": [{
                "memory_unit_id": unit.memory_unit_id,
                "decision": "forget",
                "reason": "A routine detail without lasting importance."
            }]
        })
        .to_string(),
    }
}

fn forget(engine: &MemoryEngine<FileStorage>, start: &ForgetReviewStart, unit: &MemoryUnit) {
    let applied = engine
        .submit_forget_review_response(
            &start.pending_task.task_id,
            forgetting_response(start, unit),
        )
        .expect("apply current forgetting decision");
    assert_eq!(applied.forgotten, 1);
}

struct Fixture(PathBuf);

impl Fixture {
    fn new(case: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("memory_engine_h2_{case}_{nanos}"));
        std::fs::create_dir(&root).expect("create disposable FileStorage directory");
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        MemoryEngine::new(FileStorage::with_host_id(&self.0, "h2_contract_test"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
