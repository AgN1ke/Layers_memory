"""The installed adapter exposes budgeted, scope-checked quote projections."""

import json


TIMESTAMP = "2026-05-18T10:00:00Z"
SOURCE_SPEAKER = {"id": "source_author", "name": "Alex"}
REPLY_SPEAKER = {"id": "reply_author", "name": "Alex"}
SCOPE = "quote_room"
SESSION = "reply_session"


def _ingest(engine, session, speaker, text, quoted_event_ids=()):
    result = json.loads(engine.ingest(json.dumps({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "python_test",
        "timestamp": TIMESTAMP,
        "session_id": session,
        "personality_scope_id": SCOPE,
        "memory_text": text,
        "speaker": speaker,
        "payload": {},
        "tags": ["test"],
        "theme": "quoted_context",
        "quoted_event_ids": list(quoted_event_ids),
    })))
    return result["stored_event"]["event_id"]


def _package(engine):
    return engine.core_context_package(json.dumps({
        "schema_version": "core_context_request.v1",
        "session_id": SESSION,
        "domain_state": {},
        "session_recent_limit": 1,
        "session_trace_event_limit": 1,
    }))


def test_quotes_in_context_and_prompt_plan_after_reopen(tmp_path, engine_factory):
    engine = engine_factory(tmp_path)
    text = "Older <quoted> words."
    long_text = "A longer quoted sentence. " * 100
    source_id = _ingest(engine, "older_session", SOURCE_SPEAKER, text)
    long_id = _ingest(engine, "older_session", SOURCE_SPEAKER, long_text)
    removed_id = _ingest(engine, "removed_session", SOURCE_SPEAKER, "Deleted words.")
    reply_id = _ingest(
        engine, SESSION, REPLY_SPEAKER, "My own reply.", (source_id, long_id, removed_id)
    )
    engine.delete_session("removed_session")
    engine = engine_factory(tmp_path)

    raw = _package(engine)
    package = json.loads(raw)
    reply, = package["session_recent"]
    assert reply["event_id"] == reply_id
    assert reply["text"] == "My own reply."
    short, clipped, removed = reply["quoted_events"]
    assert short == {
        "event_id": source_id,
        "status": "available",
        "speaker": SOURCE_SPEAKER,
        "timestamp": TIMESTAMP,
        "text": text,
    }
    assert clipped["event_id"] == long_id
    assert clipped["status"] == "available"
    assert clipped["truncated"] is True
    assert len(clipped["text"]) < len(long_text)
    assert clipped["text"].endswith("...")
    assert long_text.startswith(clipped["text"].removesuffix("..."))
    assert removed == {"event_id": removed_id, "status": "unavailable"}
    assert package["session_trace"][0]["quoted_events"] == reply["quoted_events"]

    plan = json.loads(engine.render_memory_prompt_plan(raw, "A new turn."))
    assert plan["dialogue_events"] == [reply]
    view = engine.render_memory_view(raw, "A new turn.")
    assert "Older &lt;quoted&gt; words." in view
    assert 'status="unavailable"' in view
    assert "Deleted words." not in view

    engine.erase_subject(SCOPE, SOURCE_SPEAKER["id"], "erase_quote_author")
    engine = engine_factory(tmp_path)
    raw = _package(engine)
    reply, = json.loads(engine.render_memory_prompt_plan(raw, "A new turn."))["dialogue_events"]
    assert reply["speaker"] == REPLY_SPEAKER
    assert reply["text"] == "My own reply."
    assert text not in raw
    assert "Older &lt;quoted&gt; words." not in engine.render_memory_view(raw, "A new turn.")
