"""End-to-end test for the Memory Engine Python adapter.

Runs the full ingest -> sleep -> resume -> recall cycle through the PyO3
binding. LLM execution is faked: the test directly constructs the
SleepCompressionResult that a host's LLM would normally produce.
"""

import json
from pathlib import Path

import pytest

import memory_engine


@pytest.fixture
def engine(tmp_path: Path) -> memory_engine.MemoryEngine:
    return memory_engine.MemoryEngine(str(tmp_path), host_id="pytest_host")


def _ingest(engine: memory_engine.MemoryEngine, session_id: str, text: str, **overrides) -> dict:
    event = {
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "pytest_user",
        "timestamp": "2026-05-18T10:00:00.000Z",
        "session_id": session_id,
        "payload": {"text": text},
        "tags": ["personal_fact", "location"],
        "theme": "personal_background",
        "importance_hint": "high",
    }
    event.update(overrides)
    return json.loads(engine.ingest(json.dumps(event)))


def test_ingest_creates_stored_event(engine: memory_engine.MemoryEngine):
    stored = _ingest(engine, "session_a", "Я живу в Берліні.")

    assert stored["event_id"].startswith("event_")
    assert stored["schema_version"] == "event.v1"
    assert stored["session_id"] == "session_a"
    assert stored["initial_weight"] >= 0.75
    assert "high importance floor" in stored["weight_reason"]


def test_full_cycle_ingest_sleep_resume_recall(engine: memory_engine.MemoryEngine):
    _ingest(engine, "session_b", "Я живу в Берліні з квітня 2026 року.")

    sleep_result = json.loads(engine.sleep("session_b"))
    archive = sleep_result["archive_entry"]
    task = sleep_result["pending_task"]

    assert archive["status"] == "preliminary"
    assert archive["archive_id"].startswith("archive_")
    assert task["task_type"] == "sleep_compression"
    assert task["prompt_id"] == "sleep_compression"
    assert task["role_hint"] == "balanced"

    pending = json.loads(engine.pending_tasks())
    assert len(pending) == 1
    assert pending[0]["task_id"] == task["task_id"]

    llm_result = {
        "schema_version": "sleep_compression_result.v1",
        "archive_id": archive["archive_id"],
        "gist": "Користувач живе в Берліні.",
        "narrative": "Користувач прямо повідомив, що проживає в Берліні з квітня 2026 року.",
        "facts": [],
        "quotes": [],
        "tags": ["personal_fact", "location"],
        "theme": "personal_background",
        "weight": 0.95,
        "links": [],
    }

    updated = json.loads(
        engine.resume_sleep_compression(task["task_id"], json.dumps(llm_result))
    )
    assert updated["status"] == "complete"
    assert updated["llm_enhanced"] is True
    assert updated["gist"] == llm_result["gist"]
    assert updated["prompt_id"] == "sleep_compression"

    assert json.loads(engine.pending_tasks()) == []

    recall_query = {
        "schema_version": "recall_query.v1",
        "context": {"recent_text": "Де живе користувач?"},
        "query_text": "Берлін",
        "filters": {"source_layers": ["archive"]},
        "limit": 5,
        "include_core": False,
        "explain": True,
    }
    recall = json.loads(engine.recall(json.dumps(recall_query)))

    assert recall["stage_used"] == "stage1"
    assert len(recall["items"]) == 1
    assert recall["items"][0]["gist"] == llm_result["gist"]
    assert recall["items"][0]["narrative"] == llm_result["narrative"]


def test_ingest_rejects_wrong_schema(engine: memory_engine.MemoryEngine):
    bad_event = json.dumps(
        {
            "schema_version": "event.v0",
            "type": "user_message",
            "source": "pytest_user",
            "timestamp": "2026-05-18T10:00:00.000Z",
            "session_id": "session_x",
            "payload": {"text": "hello"},
        }
    )
    with pytest.raises(ValueError, match="event.v1"):
        engine.ingest(bad_event)


def test_recall_zero_limit_uses_engine_default(engine: memory_engine.MemoryEngine):
    for index in range(7):
        _ingest(
            engine,
            "many_session",
            f"Факт номер {index} про Берлін.",
            timestamp=f"2026-05-18T10:0{index}:00.000Z",
        )
        sleep = json.loads(engine.sleep("many_session"))
        engine.resume_sleep_compression(
            sleep["pending_task"]["task_id"],
            json.dumps(
                {
                    "schema_version": "sleep_compression_result.v1",
                    "archive_id": sleep["archive_entry"]["archive_id"],
                    "gist": f"Стиснений факт {index} про Берлін.",
                    "narrative": f"Користувач повторив факт {index} про Берлін.",
                    "facts": [],
                    "quotes": [],
                    "tags": ["personal_fact", "location"],
                    "theme": "personal_background",
                    "weight": 0.85,
                    "links": [],
                }
            ),
        )

    query = json.dumps(
        {
            "schema_version": "recall_query.v1",
            "context": {"recent_text": "Берлін"},
            "query_text": "Берлін",
            "filters": {},
            "limit": 0,
            "include_core": False,
            "explain": False,
        }
    )
    result = json.loads(engine.recall(query))
    assert len(result["items"]) == 5
