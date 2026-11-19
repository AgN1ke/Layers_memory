"""Check the scope, subject and attribution contracts against the native extension."""

import json

import memory_engine
import pytest


def test_scoped_inspection_and_restoration(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    scope_id = "owner_group"
    session_id = "lunch"
    event = json.loads(engine.ingest(json.dumps({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "pytest_user",
        "timestamp": "2020-01-01T10:00:00Z",
        "session_id": session_id,
        "personality_scope_id": scope_id,
        "speaker": {"id": "person_1", "name": "Alex"},
        "memory_text": "I ate red lentil soup for lunch.",
        "payload": {"text": "I ate red lentil soup for lunch."},
        "quoted_event_ids": [],
    })))["stored_event"]
    assert json.loads(engine.list_local_sessions()) == [session_id]
    assert json.loads(engine.read_unarchived_session(session_id))["events"] == [event]
    sleep = json.loads(engine.sleep(session_id))
    archive_id = sleep["archive_entry"]["archive_id"]
    engine.resume_sleep_compression(sleep["pending_task"]["task_id"], json.dumps({
        "schema_version": "sleep_compression_result.v1",
        "archive_id": archive_id,
        "gist": "Alex ate red lentil soup for lunch.",
        "narrative": "Alex reported eating red lentil soup at lunch.",
        "weight": 0.2,
    }))
    archive = json.loads(engine.resume_memory_unit_pass(
        sleep["memory_unit_task"]["task_id"],
        json.dumps({
            "schema_version": "memory_units_result.v1",
            "archive_id": archive_id,
            "memory_units": [{
                "thesis": "Alex ate red lentil soup for lunch.",
                "source_event_ids": [event["event_id"]],
                "subject_hint": "self_statement",
                "speaker_id": event["speaker"]["id"],
                "weight": 0.2,
            }],
        }),
    ))
    unit_id = archive["memory_units"][0]["memory_unit_id"]
    assert not json.loads(engine.read_unarchived_session(session_id)).get("events")
    assert json.loads(engine.seed_core_from_archives_in_scope("unrelated"))["archives"] == 0
    assert json.loads(engine.seed_core_from_archives_in_scope(scope_id))["archives"] == 1
    forget = json.loads(engine.begin_forget_review(session_id))

    summary = json.loads(engine.inspect_scope(scope_id))
    assert summary["session_ids"] == [session_id]
    assert summary["counts"]["active_memory_units"] == 1
    assert summary["counts"]["pending_by_type"] == {"forget_review": 1}
    assert summary["operations"][0]["task_id"] == forget["pending_task"]["task_id"]
    assert "inputs" not in summary["operations"][0]
    assert json.loads(engine.inspect_scope("unrelated"))["archives"] == []

    detail = json.loads(engine.inspect_archive(scope_id, archive_id))
    assert detail["sources"][0]["event_id"] == event["event_id"]
    assert detail["archive"]["recall_count"] == 0
    assert json.loads(engine.inspect_archive("unrelated", archive_id)) is None
    assert json.loads(engine.inspect_memory_unit("unrelated", unit_id)) is None

    applied = json.loads(engine.resume_forget_review(forget["pending_task"]["task_id"], json.dumps({
        "schema_version": "forget_review_result.v1",
        "source_session_id": session_id,
        "recommendations": [{
            "memory_unit_id": unit_id,
            "decision": "forget",
            "reason": "A routine detail.",
        }],
    })))
    assert applied["forgotten"] == 1
    assert json.loads(engine.remember_back_in_scope("unrelated", unit_id)) is None
    unit = json.loads(engine.inspect_memory_unit(scope_id, unit_id))
    assert unit["unit"]["status"] == "forgotten"
    assert unit["sources"][0]["memory_text"] == event["memory_text"]
    restored = json.loads(engine.remember_back_in_scope(scope_id, unit_id))
    assert restored["status"] == "active_archive"
    assert json.loads(engine.inspect_archive(scope_id, archive_id))["archive"]["recall_count"] == 0
    assert json.loads(engine.begin_memory_fidelity_pass_in_scope("unrelated", unit_id)) is None
    fidelity = json.loads(engine.begin_memory_fidelity_pass_in_scope(scope_id, unit_id))
    assert fidelity["evidence_pack"]["memory_unit_id"] == unit_id

    unit_file = tmp_path / "archive" / "units" / f"{unit_id}.json"
    unit_file.write_text("broken JSON", encoding="utf-8")
    degraded = json.loads(engine.inspect_archive(scope_id, archive_id))
    assert "archive" not in degraded
    assert degraded["warnings"]
    assert json.loads(engine.inspect_scope(scope_id))["warnings"]


def test_subject_erasure_and_core_correction_survive_reopen(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    scope_id = "group"
    fact = json.loads(engine.upsert_core_fact(json.dumps({
        "schema_version": "core_fact_input.v1",
        "category": "hobby",
        "scope": scope_id,
        "subject": {"id": "person_1", "name": "Alex"},
        "text": "Alex keeps a field notebook.",
        "confidence": 0.9,
    })))["fact"]
    request = {
        "schema_version": "core_fact_attribution_input.v1",
        "scope_id": scope_id,
        "core_fact_id": fact["core_fact_id"],
        "operation_id": "fix_owner",
        "expected_updated_at": fact["updated_at"],
        "change": {"action": "assign", "subject": {"id": "person_2", "name": "Alex"}},
    }
    foreign_request = {**request, "scope_id": "unrelated"}
    assert json.loads(engine.correct_core_fact_attribution(json.dumps(foreign_request))) is None
    correction = json.loads(engine.correct_core_fact_attribution(json.dumps(request)))
    assert correction["fact"]["core_fact_id"] == fact["core_fact_id"]
    assert correction["fact"]["subject"]["id"] == "person_2"
    assert not correction["already_applied"]

    engine = engine_factory(tmp_path)
    replay = json.loads(engine.correct_core_fact_attribution(json.dumps(request)))
    assert replay["already_applied"]
    assert replay["applied_at"] == correction["applied_at"]
    with pytest.raises(memory_engine.OperationStaleError):
        engine.correct_core_fact_attribution(json.dumps({**request, "operation_id": "stale_edit"}))
    assert json.loads(engine.inspect_subject(scope_id, "person_1"))["items"] == []
    view = json.loads(engine.inspect_subject(scope_id, "person_2"))
    assert [(item["kind"], item["text"]) for item in view["items"]] == [
        ("core_fact", fact["text"]),
    ]
    assert json.loads(engine.inspect_subject("unrelated", "person_2"))["items"] == []

    report = json.loads(engine.erase_subject(scope_id, "person_2", "erase_owner"))
    assert report["deleted_core_facts"] == 1
    engine = engine_factory(tmp_path)
    assert json.loads(engine.erase_subject(scope_id, "person_2", "erase_owner")) == report
    with pytest.raises(memory_engine.OperationConflictError):
        engine.erase_subject(scope_id, "person_1", "erase_owner")
    assert json.loads(engine.inspect_subject(scope_id, "person_2"))["items"] == []
