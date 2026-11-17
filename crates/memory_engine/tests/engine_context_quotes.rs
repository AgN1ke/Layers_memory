use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use memory_engine::{
    core_store::{CoreContextPackage, CoreContextRequest, CoreContextTokenBudget},
    event::{IngestEvent, StoredEvent},
    llm::SleepRunOptions,
    prompt_view::{render_memory_prompt_plan, render_memory_view},
    storage::Storage,
    FileStorage, MemoryEngine,
};
use serde_json::{json, Value};

const SCOPE: &str = "quote_scope";
const SESSION: &str = "current_session";
const PAST_SESSION: &str = "past_session";
const FOREIGN_SCOPE: &str = "foreign_scope";
const FOREIGN_SESSION: &str = "foreign_session";
const SOURCE_AUTHOR: &str = "first_alex";
const REPLY_AUTHOR: &str = "second_alex";
const TIMESTAMP: &str = "2026-05-08T10:00:00Z";
const REPLY_TEXT: &str = "That explains my question.";
const CURRENT_TEXT: &str = "Let's take a moment.";

#[test]
fn active_quote_preserves_source_identity_and_own_reply_in_both_prompt_formats() {
    let fixture = Fixture::new();
    let engine = fixture.open();
    let quote_text = "My telescope <Orion> needs a stand.";
    let nested = ingest(
        &engine,
        SCOPE,
        SESSION,
        REPLY_AUTHOR,
        "nested_source_must_not_be_expanded",
        &[],
    );
    let mut input = event(
        SCOPE,
        SESSION,
        SOURCE_AUTHOR,
        quote_text,
        &[&nested.event_id],
    );
    input.payload = json!({"text": "metadata_must_not_be_a_quote"});
    let source = engine.ingest(input).unwrap().stored_event;
    let reply = ingest(
        &engine,
        SCOPE,
        SESSION,
        REPLY_AUTHOR,
        REPLY_TEXT,
        &[&source.event_id],
    );

    let package = context(&engine, None);
    assert_eq!(
        reply_quotes(
            &serde_json::to_value(&package.session_recent).unwrap(),
            &reply
        ),
        json!([available(&source, quote_text)])
    );
    let plan = render_memory_prompt_plan(&package, CURRENT_TEXT);
    assert_eq!(
        reply_quotes(
            &serde_json::to_value(&plan.dialogue_events).unwrap(),
            &reply
        ),
        json!([available(&source, quote_text)])
    );
    let rendered = render_memory_view(&package, CURRENT_TEXT);
    // The source is outside the recent limit. Its words must reach the rendered
    // reply through the quote rather than through an independent recent event.
    assert!(rendered.contains("My telescope &lt;Orion&gt; needs a stand."));
    assert!(rendered.contains(REPLY_TEXT));
    assert!(!rendered.contains("metadata_must_not_be_a_quote"));
    assert!(!rendered.contains("nested_source_must_not_be_expanded"));
}

#[test]
fn quote_recovers_rotated_sources_in_current_and_previous_sessions_without_feedback() {
    let fixture = Fixture::new();
    let engine = fixture.open();
    let same_session = ingest(
        &engine,
        SCOPE,
        SESSION,
        SOURCE_AUTHOR,
        "The Orion telescope is here.",
        &[],
    );
    let other_session = ingest(
        &engine,
        SCOPE,
        PAST_SESSION,
        SOURCE_AUTHOR,
        "The Apollo telescope is packed.",
        &[],
    );
    let same_archive = finish_sleep(&engine, SESSION, &same_session);
    let other_archive = finish_sleep(&engine, PAST_SESSION, &other_session);
    let reply = ingest(
        &engine,
        SCOPE,
        SESSION,
        REPLY_AUTHOR,
        REPLY_TEXT,
        &[&same_session.event_id, &other_session.event_id],
    );
    drop(engine);

    let engine = fixture.open();
    let package = context(&engine, Some(2_000));
    assert!(package.archive_relevant.is_empty());
    assert_eq!(
        reply_quotes(
            &serde_json::to_value(&package.session_recent).unwrap(),
            &reply
        ),
        json!([
            available(&same_session, "The Orion telescope is here."),
            available(&other_session, "The Apollo telescope is packed.")
        ])
    );
    let rendered = render_memory_view(&package, CURRENT_TEXT);
    assert!(rendered.contains("The Orion telescope is here."));
    assert!(rendered.contains("The Apollo telescope is packed."));
    assert_eq!(engine.flush_recall_stats().unwrap(), 0);
    for archive_id in [same_archive, other_archive] {
        let archive = engine
            .storage()
            .read_archive_entry_by_id(&archive_id)
            .unwrap();
        assert_eq!(archive.recall_count, 0);
        assert!(archive.last_recalled_at.is_none());
    }
}

#[test]
fn context_masks_foreign_missing_suppressed_and_unattributed_sources() {
    let fixture = Fixture::new();
    let engine = fixture.open();
    let foreign = ingest(
        &engine,
        FOREIGN_SCOPE,
        FOREIGN_SESSION,
        SOURCE_AUTHOR,
        "foreign_secret",
        &[],
    );
    let target = ingest(
        &engine,
        SCOPE,
        PAST_SESSION,
        SOURCE_AUTHOR,
        "erased_secret",
        &[],
    );
    let mut legacy = event(
        SCOPE,
        PAST_SESSION,
        REPLY_AUTHOR,
        "legacy_copied_secret",
        &[],
    );
    legacy.quoted_event_ids = None;
    legacy.links =
        serde_json::from_value(json!([{"kind": "reply_to", "target": target.event_id}])).unwrap();
    let legacy = engine.ingest(legacy).unwrap().stored_event;
    let mut unverified = event(
        SCOPE,
        PAST_SESSION,
        REPLY_AUTHOR,
        "unverified_legacy_secret",
        &[],
    );
    unverified.quoted_event_ids = None;
    let unverified = engine.ingest(unverified).unwrap().stored_event;
    let report = engine
        .erase_subject(SCOPE, SOURCE_AUTHOR, "erase_fixture_author")
        .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap()["suppressed_events"],
        1
    );

    let ids = [
        foreign.event_id.as_str(),
        "missing_event",
        legacy.event_id.as_str(),
        unverified.event_id.as_str(),
    ];
    // Simulate an imported or corrupted record whose references bypass ingest validation.
    let reply = StoredEvent::from_ingest(
        event(SCOPE, SESSION, REPLY_AUTHOR, REPLY_TEXT, &ids),
        "fixture_reply",
        TIMESTAMP,
        0.5,
        "fixture",
    );
    engine.storage().append_event(SESSION, &reply).unwrap();

    let package = context(&engine, None);
    let expected = ids.map(|id| json!({"event_id": id, "status": "unavailable"}));
    assert_eq!(
        reply_quotes(
            &serde_json::to_value(&package.session_recent).unwrap(),
            &reply
        ),
        json!(expected)
    );
    let plan = render_memory_prompt_plan(&package, CURRENT_TEXT);
    assert_eq!(
        reply_quotes(
            &serde_json::to_value(&plan.dialogue_events).unwrap(),
            &reply
        ),
        json!(expected)
    );
    let rendered = render_memory_view(&package, CURRENT_TEXT);
    assert!(rendered.contains("unavailable"));
    assert!(rendered.contains(REPLY_TEXT));
    assert!(!rendered.contains("foreign_secret"));
    assert!(!rendered.contains("legacy_copied_secret"));
    assert!(!rendered.contains("unverified_legacy_secret"));
}

#[test]
fn subject_erase_removes_quote_text_after_restart_and_preserves_other_authorship() {
    let fixture = Fixture::new();
    let engine = fixture.open();
    let secret = "erase_the_sapphire_codeword";
    let target = ingest(&engine, SCOPE, PAST_SESSION, SOURCE_AUTHOR, secret, &[]);
    finish_sleep(&engine, PAST_SESSION, &target);
    let other = ingest(
        &engine,
        SCOPE,
        SESSION,
        REPLY_AUTHOR,
        "I organize the calendar.",
        &[],
    );
    let reply = ingest(
        &engine,
        SCOPE,
        SESSION,
        REPLY_AUTHOR,
        REPLY_TEXT,
        &[&target.event_id, &other.event_id],
    );
    let before = context(&engine, Some(2_000));
    assert!(render_memory_view(&before, CURRENT_TEXT).contains(secret));

    engine
        .erase_subject(SCOPE, SOURCE_AUTHOR, "erase_quote_author")
        .unwrap();
    drop(engine);

    let engine = fixture.open();
    let after = context(&engine, Some(2_000));
    let plan = render_memory_prompt_plan(&after, CURRENT_TEXT);
    let rendered = render_memory_view(&after, CURRENT_TEXT);
    for output in [
        serde_json::to_string(&after).unwrap(),
        serde_json::to_string(&plan).unwrap(),
        rendered,
    ] {
        assert!(!output.contains(secret));
        assert!(output.contains(REPLY_TEXT));
    }
    let quotes = reply_quotes(
        &serde_json::to_value(&plan.dialogue_events).unwrap(),
        &reply,
    );
    assert_eq!(
        quotes,
        json!([available(&other, "I organize the calendar.")])
    );
}

#[test]
fn quote_fits_current_memory_budget_without_displacing_own_reply() {
    let fixture = Fixture::new();
    let engine = fixture.open();
    let source_text = "Beginning of the archived quotation. ".repeat(64);
    let source = ingest(
        &engine,
        SCOPE,
        PAST_SESSION,
        SOURCE_AUTHOR,
        &source_text,
        &[],
    );
    ingest(&engine, SCOPE, SESSION, REPLY_AUTHOR, REPLY_TEXT, &[]);
    let baseline = context(&engine, Some(2_000));
    let reply = ingest(
        &engine,
        SCOPE,
        SESSION,
        REPLY_AUTHOR,
        REPLY_TEXT,
        &[&source.event_id],
    );
    let normal = context(&engine, Some(2_000));
    let tight = context(&engine, Some(96));
    let normal_events = serde_json::to_value(&normal.session_recent).unwrap();
    let tight_events = serde_json::to_value(&tight.session_recent).unwrap();
    let normal_quotes = reply_quotes(&normal_events, &reply);
    let tight_quotes = reply_quotes(&tight_events, &reply);
    let normal_text = normal_quotes[0]["text"]
        .as_str()
        .expect("quote shown in reasonable budget");
    let tight_text = tight_quotes[0]["text"].as_str().unwrap_or_default();

    assert!(normal_text.starts_with("Beginning of the archived quotation."));
    assert!(tight_text.chars().count() < normal_text.chars().count());
    assert!(
        normal.budget.unwrap().estimated_current_memory_tokens
            > baseline.budget.unwrap().estimated_current_memory_tokens
    );
    assert!(tight.budget.unwrap().estimated_current_memory_tokens <= 96);
    assert_eq!(reply_row(&tight_events, &reply)["text"], REPLY_TEXT);
}

fn event(scope: &str, session: &str, author: &str, text: &str, quotes: &[&str]) -> IngestEvent {
    serde_json::from_value(json!({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "quote_conformance",
        "timestamp": TIMESTAMP,
        "personality_scope_id": scope,
        "session_id": session,
        "memory_text": text,
        "payload": {},
        "speaker": {"id": author, "name": "Alex"},
        "quoted_event_ids": quotes,
    }))
    .unwrap()
}

fn ingest(
    engine: &MemoryEngine<FileStorage>,
    scope: &str,
    session: &str,
    author: &str,
    text: &str,
    quotes: &[&str],
) -> StoredEvent {
    engine
        .ingest(event(scope, session, author, text, quotes))
        .unwrap()
        .stored_event
}

fn finish_sleep(engine: &MemoryEngine<FileStorage>, session: &str, source: &StoredEvent) -> String {
    let run = engine
        .begin_sleep_run_with_options(
            session,
            SleepRunOptions {
                enabled_tracks: Vec::new(),
                ..SleepRunOptions::default()
            },
        )
        .unwrap();
    let step = engine.next_sleep_batch(run).unwrap();
    assert!(step.batch.is_none());
    let outcome = engine.finish_sleep_run(step.run).unwrap();
    assert!(engine
        .storage()
        .read_session(session)
        .unwrap()
        .events
        .is_empty());
    let archived_events = engine
        .storage()
        .read_session_archived_events(session)
        .unwrap();
    assert_eq!(archived_events, std::slice::from_ref(source));
    outcome.archive_entry.archive_id
}

fn context(
    engine: &MemoryEngine<FileStorage>,
    current_memory_limit: Option<usize>,
) -> CoreContextPackage {
    let mut request: CoreContextRequest = serde_json::from_value(json!({
        "schema_version": "core_context_request.v1",
        "session_id": SESSION,
        "core_scope": SCOPE,
        "session_recent_limit": 1,
        "session_trace_event_limit": 1,
        "query_text": CURRENT_TEXT,
    }))
    .unwrap();
    request.token_budget = current_memory_limit.map(|tokens| CoreContextTokenBudget {
        total_tokens: tokens,
        current_memory_tokens: tokens,
        compressed_memory_tokens: 0,
        core_tokens: 0,
    });
    engine.core_context_package(request).unwrap()
}

fn available(source: &StoredEvent, text: &str) -> Value {
    json!({
        "event_id": source.event_id,
        "status": "available",
        "speaker": source.speaker,
        "timestamp": source.timestamp,
        "text": text,
    })
}

fn reply_row<'a>(events: &'a Value, reply: &StoredEvent) -> &'a Value {
    events
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_id"] == reply.event_id)
        .expect("reply remains in context")
}

fn reply_quotes(events: &Value, reply: &StoredEvent) -> Value {
    reply_row(events, reply)["quoted_events"].clone()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let root = std::env::temp_dir().join(format!(
            "memory_engine_context_quotes_{}_{}",
            std::process::id(),
            stamp.as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn open(&self) -> MemoryEngine<FileStorage> {
        MemoryEngine::new(FileStorage::with_host_id(&self.0, "context_quotes_test"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
