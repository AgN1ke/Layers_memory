"""Exercise the installed adapter on deterministic, provider-free inputs."""

import json


def _start_archive(engine, session):
    engine.ingest(json.dumps({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "pytest_user",
        "timestamp": "2020-01-01T10:00:00Z",
        "session_id": session,
        "payload": {"text": "I saw the moon after lunch."},
        "importance_hint": "high",
    }))
    return json.loads(engine.sleep(session))


def _complete_archive(engine, sleep):
    archive = sleep["archive_entry"]
    return json.loads(engine.resume_sleep_compression(
        sleep["pending_task"]["task_id"],
        json.dumps({
            "schema_version": "sleep_compression_result.v1",
            "archive_id": archive["archive_id"],
            "gist": "The user saw the moon after lunch.",
            "narrative": "The user remembered seeing the moon after eating lunch.",
            "weight": 0.2,
        }),
    ))


def _units_payload(archive, weight):
    units = [] if weight is None else [{
        "thesis": "The user saw the moon after lunch.",
        "source_event_ids": archive["source_event_ids"],
        "weight": weight,
    }]
    return {
        "schema_version": "memory_units_result.v1",
        "archive_id": archive["archive_id"],
        "memory_units": units,
    }


def _archive_with_unit(engine, session, weight):
    sleep = _start_archive(engine, session)
    archive = _complete_archive(engine, sleep)
    updated = json.loads(engine.resume_memory_unit_pass(
        sleep["memory_unit_task"]["task_id"],
        json.dumps(_units_payload(archive, weight)),
    ))
    return updated["memory_units"][0]


def _ok(request, result):
    return json.dumps({
        "status": "ok",
        "request_id": request["request_id"],
        "text": json.dumps(result),
    })


def _verify_unit(unit):
    return {
        "schema_version": "fidelity_review.v1",
        "memory_unit_id": unit["memory_unit_id"],
        "archive_id": unit["archive_id"],
        "status": "valid",
        "confidence": 0.9,
        "explanation": "The source directly supports this memory.",
    }


def test_sleep_repairs_and_legacy_compact_task(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    session = "legacy_sleep"
    sleep = _start_archive(engine, session)
    archive = sleep["archive_entry"]
    # A saved v0.1 task is the only entry point into the legacy compact pass.
    legacy_task = {
        **sleep["pending_task"],
        "task_id": "task_legacy_compact",
        "task_type": "compact_memory_pass",
        "prompt_id": "compact_memory_pass",
        "expected_output_schema": "compact_memory_text.v1",
    }
    (tmp_path / "tasks" / "task_legacy_compact.json").write_text(
        json.dumps(legacy_task), encoding="utf-8"
    )
    compact = json.loads(engine.resume_compact_memory_pass(
        legacy_task["task_id"], "Remember: the moon was visible after lunch."
    ))
    assert compact["compact_memory"] == "Remember: the moon was visible after lunch."

    archive = _complete_archive(engine, sleep)
    empty = json.loads(engine.resume_memory_unit_pass(
        sleep["memory_unit_task"]["task_id"], json.dumps(_units_payload(archive, None))
    ))
    assert not empty.get("memory_units")
    repair, = json.loads(engine.pending_memory_unit_repairs(session))
    no_repair = json.loads(engine.submit_memory_unit_repair_response(
        repair["task_id"],
        json.dumps({"status": "err", "request_id": repair["request_id"],
                    "kind": "transport", "detail": "Synthetic timeout"}),
    ))
    assert no_repair is None
    repair, = json.loads(engine.pending_memory_unit_repairs(session))
    repaired = json.loads(engine.submit_memory_unit_repair_response(
        repair["task_id"], _ok(repair, _units_payload(archive, 0.3))
    ))
    assert repaired["memory_units"][0]["archive_id"] == archive["archive_id"]
    assert not json.loads(engine.pending_memory_unit_repairs(session))
    assert json.loads(engine.seed_core_from_archives())["archives"] == 1
    assert type(engine.flush_recall_stats()) is int


def test_cancel_sleep_run(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    session = "cancel_sleep"
    engine.ingest(json.dumps({
        "schema_version": "event.v1", "type": "user_message", "source": "pytest_user",
        "timestamp": "2020-01-01T10:00:00Z", "session_id": session,
        "payload": {"text": "Keep this event after cancellation."},
        "importance_hint": "high",
    }))
    run = json.loads(engine.begin_sleep_run(session))
    cancelled = json.loads(engine.cancel_sleep_run(run["sleep_task_id"]))
    assert cancelled["stage"] == "finished"
    assert cancelled["completion_mode"] == "cancelled"


def test_fidelity_and_forgetting(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    session = "fidelity_forget"
    unit = _archive_with_unit(engine, session, 0.2)
    unit_id = unit["memory_unit_id"]
    evidence = json.loads(engine.build_evidence_pack(unit_id))
    assert evidence["memory_unit_id"] == unit_id

    fidelity = json.loads(engine.begin_memory_fidelity_pass(unit_id))
    verified = json.loads(engine.resume_memory_fidelity_pass(
        fidelity["pending_task"]["task_id"], json.dumps(_verify_unit(unit))
    ))
    assert verified["fidelity_status"] == "valid"
    fidelity = json.loads(engine.begin_memory_fidelity_pass(unit_id))
    verified = json.loads(engine.submit_memory_fidelity_response(
        fidelity["pending_task"]["task_id"], _ok(fidelity["request"], _verify_unit(unit))
    ))
    assert verified["fidelity_status"] == "valid"

    review = {
        "schema_version": "forget_review_result.v1",
        "source_session_id": session,
        "recommendations": [],
    }
    start = json.loads(engine.begin_forget_review(session))
    assert start["candidate_count"] == 1
    unchanged = json.loads(engine.resume_forget_review(
        start["pending_task"]["task_id"], json.dumps(review)
    ))
    assert unchanged["forgotten"] == 0
    start = json.loads(engine.begin_forget_review(session))
    review["recommendations"] = [{
        "memory_unit_id": unit_id, "decision": "forget", "reason": "Routine detail."
    }]
    applied = json.loads(engine.submit_forget_review_response(
        start["pending_task"]["task_id"], _ok(start["request"], review)
    ))
    assert applied["forgotten"] == 1
    listed = json.loads(engine.list_forgotten_memory_units(session))
    assert [item["memory_unit_id"] for item in listed["units"]] == [unit_id]
    assert json.loads(engine.remember_back(unit_id))["status"] == "active_archive"


def test_candidate_submission_and_review(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    session = "candidate_review"
    unit = _archive_with_unit(engine, session, 0.9)
    fidelity = json.loads(engine.begin_memory_fidelity_pass(unit["memory_unit_id"]))
    engine.resume_memory_fidelity_pass(
        fidelity["pending_task"]["task_id"], json.dumps(_verify_unit(unit))
    )

    reflection = {
        "schema_version": "reflection_result.v1",
        "source_session_id": session,
        "core_scope": session,
        "candidates": [],
    }
    start = json.loads(engine.begin_reflection_analysis(session, session))
    empty = json.loads(engine.resume_reflection_analysis(
        start["pending_task"]["task_id"], json.dumps(reflection)
    ))
    assert not empty.get("candidates")
    start = json.loads(engine.begin_reflection_analysis(session, session))
    reflection["candidates"] = [{
        "text": "The user noticed the moon after lunch.",
        "category": "observation",
        "confidence": 0.9,
        "evidence_summary": "The source says when the user saw the moon.",
        "source_memory_unit_ids": [unit["memory_unit_id"]],
        "supporting_archive_ids": [unit["archive_id"]],
        "tags": ["moon"],
    }]
    submitted = json.loads(engine.submit_reflection_response(
        start["pending_task"]["task_id"], _ok(start["request"], reflection)
    ))
    candidate, = submitted["candidates"]
    assert json.loads(engine.list_candidates())[0]["candidate_id"] == candidate["candidate_id"]
    reviewed = json.loads(engine.review_candidate(json.dumps({
        "schema_version": "candidate_review_input.v1",
        "candidate_id": candidate["candidate_id"],
        "core_scope": session,
        "reviewed_by": "pytest_owner",
        "decision": "rejected",
    })))
    assert reviewed["candidate"]["status"] == "rejected"


def test_vector_backfill_and_deep_recall(engine_factory, tmp_path):
    engine = engine_factory(tmp_path)
    session = "vector_backfill"
    unit = _archive_with_unit(engine, session, 0.8)
    assert not json.loads(engine.pending_embedding_backfill(session))
    assert json.loads(engine.set_vector_scope(session, True))["status"] == "building"
    request, = json.loads(engine.pending_embedding_backfill(session))
    batch = request["prompt_inputs"]["embed_batch"]
    vector = [1.0] + [0.0] * (batch["dim"] - 1)
    embedding = {
        "schema_version": "embed_batch_result.v1",
        "model_id": batch["model_id"],
        "dim": batch["dim"],
        "results": [{"memory_unit_id": unit["memory_unit_id"], "vector": vector}],
    }
    assert engine.resume_compute_embedding(request["task_id"], json.dumps(embedding)) == 1
    result = json.loads(engine.recall_deep(json.dumps({
        "scope": session, "model_id": batch["model_id"], "query_vec": vector,
    })))
    assert result["found"] is True
    assert result["hits"][0]["memory_unit_id"] == unit["memory_unit_id"]
    assert json.loads(engine.rebuild_vectors(session))["rows"] == 0
