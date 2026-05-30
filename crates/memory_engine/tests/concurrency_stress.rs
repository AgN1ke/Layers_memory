use memory_engine::core_store::CoreFactInput;
use memory_engine::event::IngestEvent;
use memory_engine::storage::Storage;
use memory_engine::types::{CORE_FACT_INPUT_SCHEMA_VERSION, EVENT_SCHEMA_VERSION};
use memory_engine::{FileStorage, MemoryEngine};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const QUICK_THREADS: usize = 64;
const FULL_THREADS: usize = 1_000;

#[test]
fn engine_handles_parallel_sessions_and_core_upserts_without_lost_updates() {
    run_parallel_session_and_core_stress(QUICK_THREADS, 2);
}

#[test]
#[ignore = "1000-session disk stress; run explicitly before release gates"]
fn engine_handles_1000_parallel_sessions_and_core_upserts_without_lost_updates() {
    run_parallel_session_and_core_stress(FULL_THREADS, 1);
}

fn run_parallel_session_and_core_stress(thread_count: usize, events_per_session: usize) {
    let root = unique_temp_dir(&format!(
        "parallel_sessions_and_core_upserts_{thread_count}"
    ));
    let storage = FileStorage::with_host_id(&root, "stress");
    let engine = Arc::new(MemoryEngine::new(storage));
    let barrier = Arc::new(Barrier::new(thread_count));

    let mut handles = Vec::with_capacity(thread_count);
    for index in 0..thread_count {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            let session_id = format!("stress_session_{index}");
            for event_index in 0..events_per_session {
                engine
                    .ingest(IngestEvent {
                        schema_version: EVENT_SCHEMA_VERSION.to_string(),
                        event_type: "user_message".to_string(),
                        source: format!("stress_user_{index}"),
                        timestamp: format!(
                            "2026-05-30T12:{:02}:{:02}.000Z",
                            index % 60,
                            event_index
                        ),
                        session_id: session_id.clone(),
                        payload: json!({
                            "text": format!("message {event_index} from session {index}")
                        }),
                        tags: vec!["stress".to_string()],
                        theme: Some("concurrency".to_string()),
                        emotional_tone: None,
                        links: vec![],
                        importance_hint: Default::default(),
                        processing_mode: Default::default(),
                    })
                    .expect("parallel ingest should succeed");
            }

            engine
                .upsert_core_fact(CoreFactInput {
                    schema_version: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
                    category: "name".to_string(),
                    scope: Some(session_id),
                    text: format!("Stress user {index} has a stable name fact."),
                    confidence: 0.95,
                    tags: vec!["stress".to_string()],
                    source_archive_ids: Vec::new(),
                    source_candidate_id: None,
                })
                .expect("parallel core upsert should succeed");
        }));
    }

    for handle in handles {
        handle.join().expect("stress thread should not panic");
    }

    for index in 0..thread_count {
        let session_id = format!("stress_session_{index}");
        let session = engine
            .storage()
            .read_session(&session_id)
            .expect("session should be readable");
        assert_eq!(
            session.events.len(),
            events_per_session,
            "session {session_id} event count"
        );
        assert!(
            session
                .events
                .iter()
                .all(|event| event.session_id == session_id),
            "session {session_id} must not contain events from another session"
        );
    }

    let name_category = engine
        .storage()
        .read_core_store_category("name")
        .expect("name core category should be readable");
    assert_eq!(
        name_category.facts.len(),
        thread_count,
        "core/store/name.json should contain every scope fact"
    );
    for index in 0..thread_count {
        let scope = format!("stress_session_{index}");
        assert!(
            name_category
                .facts
                .iter()
                .any(|fact| fact.scope.as_deref() == Some(scope.as_str())),
            "missing core fact for {scope}"
        );
    }

    let tmp_files = collect_tmp_files(&root);
    assert!(
        tmp_files.is_empty(),
        "atomic writes should not leave temp files behind: {tmp_files:?}"
    );

    fs::remove_dir_all(root).ok();
}

fn collect_tmp_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_tmp_files_inner(root, &mut found);
    found
}

fn collect_tmp_files_inner(path: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_tmp_files_inner(&path, found);
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".tmp"))
        {
            found.push(path);
        }
    }
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
