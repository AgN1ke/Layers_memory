use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::archive::{FidelityStatus, MemoryUnit, MemoryUnitStatus};
use memory_engine::event::IngestEvent;
use memory_engine::sleep::{MemoryUnitDraft, MemoryUnitPassResult};
use memory_engine::storage::Storage;
use memory_engine::tasks::TaskState;
use memory_engine::types::{
    EVENT_SCHEMA_VERSION, MEMORY_UNITS_RESULT_SCHEMA_VERSION, MEMORY_UNIT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;

#[test]
fn restarting_after_a_partial_extraction_write_recovers_the_whole_result() {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let root = std::env::temp_dir().join(format!(
        "memory_engine_h2_partial_{}_{}",
        std::process::id(),
        stamp.as_nanos()
    ));
    let session_id = "partial_extraction_session";
    let storage = FileStorage::with_host_id(&root, "test_host");
    let engine = MemoryEngine::new(storage.clone());
    let event = engine
        .ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "test_user".to_string(),
            timestamp: "2026-05-17T16:00:00Z".to_string(),
            personality_scope_id: None,
            session_id: session_id.to_string(),
            memory_text: None,
            payload: json!({"text": "I bought a blue notebook and gave a red pen to Alex."}),
            tags: Vec::new(),
            theme: None,
            emotional_tone: None,
            speaker: None,
            links: Vec::new(),
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        })
        .expect("ingest source event")
        .stored_event;
    let sleep = engine.sleep(session_id).expect("prepare extraction");
    let task = sleep.memory_unit_task.expect("memory-unit task");
    let archive = sleep.archive_entry;
    let theses = [
        "The user bought a blue notebook.",
        "The user gave a red pen to Alex.",
    ];
    let result = MemoryUnitPassResult {
        schema_version: MEMORY_UNITS_RESULT_SCHEMA_VERSION.to_string(),
        archive_id: archive.archive_id.clone(),
        memory_units: theses
            .into_iter()
            .map(|thesis| MemoryUnitDraft {
                thesis: thesis.to_string(),
                subject_hint: None,
                speaker_id: None,
                source_event_ids: vec![event.event_id.clone()],
                evidence: None,
                tags: Vec::new(),
                weight: 0.5,
            })
            .collect(),
    };

    // Reconstruct the FileStorage state after the first standalone write and
    // before updating either the archive or task. No process crash is injected.
    assert!(archive.memory_units.is_empty());
    assert_eq!(task.state, TaskState::Pending);
    storage
        .write_memory_unit(&MemoryUnit {
            schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
            memory_unit_id: "mu_partial_extraction".to_string(),
            archive_id: archive.archive_id.clone(),
            personality_scope_id: archive.personality_scope_id.clone(),
            source_session_id: session_id.to_string(),
            created_at: archive.created_at.clone(),
            updated_at: archive.updated_at.clone(),
            thesis: theses[0].to_string(),
            subject: None,
            source_event_ids: vec![event.event_id],
            evidence: None,
            tags: Vec::new(),
            weight: 0.5,
            status: MemoryUnitStatus::ActiveArchive,
            fidelity_status: FidelityStatus::Unchecked,
            fidelity_review: None,
            forget_review: None,
            reflection_count: 0,
            last_reflected_at: None,
            last_reflection_operation_id: None,
        })
        .expect("persist just the first unit");
    drop(engine);

    let restarted = MemoryEngine::new(FileStorage::with_host_id(&root, "test_host"));
    let resumed = restarted
        .resume_memory_unit_pass(&task.task_id, result.clone())
        .expect("redeliver the complete extraction result after restart");
    let persisted = restarted
        .storage()
        .read_memory_units_for_archive(&archive.archive_id)
        .expect("read the published canonical units");
    assert!(persisted.warnings.is_empty());
    assert!(persisted
        .items
        .iter()
        .any(|unit| unit.memory_unit_id == "mu_partial_extraction" && unit.thesis == theses[0]));
    let mut published_theses = persisted
        .items
        .iter()
        .map(|unit| unit.thesis.as_str())
        .collect::<Vec<_>>();
    published_theses.sort_unstable();
    let final_state = restarted.storage().load_task(&task.task_id).unwrap().state;
    let replay = restarted
        .resume_memory_unit_pass(&task.task_id, result)
        .unwrap();
    assert_eq!(replay.memory_units, resumed.memory_units);
    drop(restarted);
    fs::remove_dir_all(root).ok();

    assert_eq!(
        published_theses, theses,
        "redelivery left an incomplete batch with task state {final_state:?}"
    );
    assert_eq!(resumed.memory_units.len(), theses.len());
    assert_eq!(final_state, TaskState::Completed);
}
