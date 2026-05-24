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
    result = _ingest(engine, "session_a", "Я живу в Берліні.")
    stored = result["stored_event"]

    assert result["schema_version"] == "ingest_result.v1"
    assert stored["event_id"].startswith("event_")
    assert stored["schema_version"] == "event.v1"
    assert stored["session_id"] == "session_a"
    assert stored["initial_weight"] >= 0.75
    assert "high importance floor" in stored["weight_reason"]


def test_read_session_returns_stored_events(engine: memory_engine.MemoryEngine):
    _ingest(engine, "session_recent", "Розкажи про МіГ-15.")
    _ingest(
        engine,
        "session_recent",
        "Розкажи про F-86.",
        timestamp="2026-05-18T10:01:00.000Z",
    )

    session = json.loads(engine.read_session("session_recent"))

    assert session["metadata"]["session_id"] == "session_recent"
    assert session["metadata"]["event_count"] == 2
    assert [event["payload"]["text"] for event in session["events"]] == [
        "Розкажи про МіГ-15.",
        "Розкажи про F-86.",
    ]


def test_explicit_sleep_preserves_active_tail(engine: memory_engine.MemoryEngine):
    last_result = None
    for index in range(50):
        last_result = _ingest(
            engine,
            "sleep_pressure_session",
            f"Подія {index}",
            timestamp=f"2026-05-18T10:{index:02}:00.000Z",
        )

    assert last_result is not None
    assert set(last_result) == {"schema_version", "stored_event"}

    package = json.loads(
        engine.core_context_package(
            json.dumps(
                {
                    "schema_version": "core_context_request.v1",
                    "session_id": "sleep_pressure_session",
                    "domain_state": {"current_text": "Що зараз активне?"},
                    "query_text": "Подія",
                    "recall_limit": 5,
                    "session_recent_limit": 50,
                    "session_trace_event_limit": 50,
                    "include_core": False,
                    "token_budget": {
                        "total_tokens": 50000,
                        "current_memory_tokens": 45000,
                        "compressed_memory_tokens": 4000,
                        "core_tokens": 1000,
                    },
                }
                )
            )
    )
    assert len(package["session_trace"]) == 50
    assert package["session_trace"][0]["text"] == "Подія 0"
    assert package["session_trace"][-1]["text"] == "Подія 49"
    assert package.get("archive_relevant", []) == []

    sleep_result = json.loads(engine.sleep("sleep_pressure_session"))
    assert sleep_result["archive_entry"]["source_session_id"] == "sleep_pressure_session"
    assert len(sleep_result["archive_entry"]["source_event_ids"]) == 35
    assert sleep_result["pending_task"]["task_type"] == "sleep_compression"
    assert sleep_result["memory_unit_task"]["task_type"] == "memory_unit_pass"

    engine.resume_sleep_compression(
        sleep_result["pending_task"]["task_id"],
        json.dumps(
            {
                "schema_version": "sleep_compression_result.v1",
                "archive_id": sleep_result["archive_entry"]["archive_id"],
                "gist": "Стиснені події 0-34.",
                "narrative": "Старша частина сесії була перенесена в архів.",
                "facts": [],
                "quotes": [],
                "tags": ["sleep"],
                "theme": "test_memory",
                "weight": 0.85,
                "links": [],
            }
        ),
    )
    engine.resume_memory_unit_pass(
        sleep_result["memory_unit_task"]["task_id"],
        json.dumps(
            {
                "schema_version": "memory_units_result.v1",
                "archive_id": sleep_result["archive_entry"]["archive_id"],
                "memory_units": [
                    {
                        "thesis": "Події 0-34 -> старша частина сесії стиснулась у коротку пам'ять.",
                        "source_event_ids": sleep_result["archive_entry"]["source_event_ids"][:3],
                        "tags": ["sleep"],
                        "weight": 0.85,
                    }
                ],
            }
        ),
    )

    completed_package = json.loads(
        engine.core_context_package(
            json.dumps(
                {
                    "schema_version": "core_context_request.v1",
                    "session_id": "sleep_pressure_session",
                    "domain_state": {"current_text": "Що зараз активне?"},
                    "query_text": "Подія",
                    "recall_limit": 5,
                    "session_recent_limit": 50,
                    "session_trace_event_limit": 50,
                    "include_core": False,
                    "token_budget": {
                        "total_tokens": 50000,
                        "current_memory_tokens": 45000,
                        "compressed_memory_tokens": 4000,
                        "core_tokens": 1000,
                    },
                }
                )
            )
    )
    assert len(completed_package["session_trace"]) == 15
    assert completed_package["session_trace"][0]["text"] == "Подія 35"
    assert completed_package["session_trace"][-1]["text"] == "Подія 49"


def test_full_cycle_ingest_sleep_resume_recall(engine: memory_engine.MemoryEngine):
    _ingest(engine, "session_b", "Я живу в Берліні з квітня 2026 року.")

    sleep_result = json.loads(engine.sleep("session_b"))
    archive = sleep_result["archive_entry"]
    task = sleep_result["pending_task"]
    memory_unit_task = sleep_result["memory_unit_task"]

    assert archive["status"] == "preliminary"
    assert archive["archive_id"].startswith("archive_")
    assert task["task_type"] == "sleep_compression"
    assert task["prompt_id"] == "sleep_compression"
    assert task["role_hint"] == "balanced"
    assert memory_unit_task["task_type"] == "memory_unit_pass"
    assert memory_unit_task["prompt_id"] == "memory_unit_pass"

    pending = json.loads(engine.pending_tasks())
    assert len(pending) == 2
    assert {item["task_id"] for item in pending} == {
        task["task_id"],
        memory_unit_task["task_id"],
    }

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

    unit_updated = json.loads(
        engine.resume_memory_unit_pass(
            memory_unit_task["task_id"],
            json.dumps(
                {
                    "schema_version": "memory_units_result.v1",
                    "archive_id": archive["archive_id"],
                    "memory_units": [
                        {
                            "thesis": "Берлін -> користувач повідомив стабільний контекст проживання.",
                            "source_event_ids": archive["source_event_ids"],
                            "evidence": "Користувач прямо повідомив, що живе в Берліні.",
                            "tags": ["location"],
                            "weight": 0.95,
                        }
                    ],
                }
            ),
        )
    )
    assert unit_updated["memory_units"][0]["memory_unit_id"].startswith("mu_")
    assert unit_updated["compact_memory"] == (
        "Берлін -> користувач повідомив стабільний контекст проживання."
    )
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
    assert (
        recall["items"][0]["compact_memory"]
        == "Берлін -> користувач повідомив стабільний контекст проживання."
    )
    assert recall["items"][0]["gist"] == recall["items"][0]["compact_memory"]
    assert "narrative" not in recall["items"][0]


def test_sleep_resume_persists_multi_track_fields(engine: memory_engine.MemoryEngine):
    _ingest(
        engine,
        "multi_track_session",
        "У мене є кішечка Іржа, і мені тепло про неї розповідати.",
        tags=["personal_story"],
        theme="personal_pet",
        importance_hint="high",
    )

    sleep_result = json.loads(engine.sleep("multi_track_session"))
    archive = sleep_result["archive_entry"]
    task = sleep_result["pending_task"]
    source_ids = archive["source_event_ids"]

    llm_result = {
        "schema_version": "sleep_compression_result.v1",
        "archive_id": archive["archive_id"],
        "gist": "Користувач тепло розповів про кішечку Іржу.",
        "narrative": "Користувач поділився особистим теплим епізодом про свою кішечку Іржу.",
        "facts": [],
        "quotes": [],
        "tags": ["personal_pet", "emotional_memory"],
        "theme": "personal_pet",
        "weight": 0.95,
        "links": [],
        "emotional_markers": [
            {
                "target": "cat_named_irzha",
                "affect": "fondness",
                "strength": 0.95,
                "source_event_ids": source_ids,
                "quote": "У мене є кішечка Іржа",
                "evidence": "Користувач прямо описав тепле ставлення.",
            }
        ],
        "topic_thread": [
            {
                "topic": "personal_pet",
                "subtopics": ["cat_named_irzha"],
                "energy": "warm",
                "source_event_ids": source_ids,
                "summary": "Користувач розповів про кішечку.",
            }
        ],
        "personal_signals": [
            {
                "text": "Користувач має кішечку на ім'я Іржа.",
                "category": "relationships_with_pets",
                "confidence": 0.95,
                "source_event_ids": source_ids,
                "evidence": "Пряма заява користувача.",
            }
        ],
        "relational_tone": {
            "warmth": 0.8,
            "intellectual_engagement": 0.2,
            "intimacy": 0.5,
            "trust": 0.4,
            "playfulness": 0.3,
            "tension": 0.0,
            "summary": "Користувач поділився теплим особистим фактом.",
            "source_event_ids": source_ids,
        },
    }

    updated = json.loads(
        engine.resume_sleep_compression(task["task_id"], json.dumps(llm_result))
    )

    assert updated["emotional_markers"][0]["target"] == "cat_named_irzha"
    assert updated["personal_signals"][0]["category"] == "relationships_with_pets"
    assert updated["relational_tone"]["warmth"] == 0.8


def test_core_context_package_combines_session_and_archive(engine: memory_engine.MemoryEngine):
    _ingest(engine, "context_session", "Ми говорили про МіГ-15.")
    sleep_result = json.loads(engine.sleep("context_session"))
    llm_result = {
        "schema_version": "sleep_compression_result.v1",
        "archive_id": sleep_result["archive_entry"]["archive_id"],
        "gist": "Розмова про МіГ-15.",
        "narrative": "Користувач питав про радянський винищувач МіГ-15.",
        "facts": [],
        "quotes": [],
        "tags": ["aircraft"],
        "theme": "aviation",
        "weight": 0.9,
        "links": [],
    }
    engine.resume_sleep_compression(
        sleep_result["pending_task"]["task_id"],
        json.dumps(llm_result),
    )
    engine.resume_memory_unit_pass(
        sleep_result["memory_unit_task"]["task_id"],
        json.dumps(
            {
                "schema_version": "memory_units_result.v1",
                "archive_id": sleep_result["archive_entry"]["archive_id"],
                "memory_units": [
                    {
                        "thesis": "Обговорили МіГ-15 -> користувач цікавиться військовою авіацією.",
                        "source_event_ids": sleep_result["archive_entry"]["source_event_ids"],
                        "tags": ["aviation"],
                        "weight": 0.9,
                    }
                ],
            }
        ),
    )
    _ingest(
        engine,
        "context_session",
        "А тепер говоримо про риболовлю.",
        timestamp="2026-05-18T10:01:00.000Z",
    )

    request = {
        "schema_version": "core_context_request.v1",
        "session_id": "context_session",
        "domain_state": {"current_text": "А про літаки?"},
        "query_text": "літаки МіГ-15",
        "recall_limit": 5,
        "session_recent_limit": 2,
        "session_trace_event_limit": 10,
        "include_core": False,
    }
    package = json.loads(engine.core_context_package(json.dumps(request)))

    assert package["schema_version"] == "core_context_package.v1"
    assert len(package["session_recent"]) == 1
    assert "риболовлю" in package["session_recent"][0].get("text", "")
    assert not any("МіГ-15" in event.get("text", "") for event in package["session_trace"])
    assert (
        package["archive_relevant"][0]["compact_memory"]
        == "Обговорили МіГ-15 -> користувач цікавиться військовою авіацією."
    )
    assert package["archive_relevant"][0]["gist"] == package["archive_relevant"][0]["compact_memory"]
    assert "narrative" not in package["archive_relevant"][0]


def test_upsert_core_fact_is_returned_in_context_package(engine: memory_engine.MemoryEngine):
    _ingest(engine, "core_session", "Мене звати Микита.")

    result = json.loads(
        engine.upsert_core_fact(
            json.dumps(
                {
                    "schema_version": "core_fact_input.v1",
                    "category": "profile",
                    "scope": "telegram_core_session",
                    "text": "Користувача звати Микита.",
                    "confidence": 0.95,
                    "tags": ["telegram", "name"],
                }
            )
        )
    )

    assert result["schema_version"] == "core_fact_upsert_result.v1"
    assert result["created"] is True
    assert result["fact"]["core_fact_id"].startswith("core_fact_")

    request = {
        "schema_version": "core_context_request.v1",
        "session_id": "core_session",
        "domain_state": {"current_text": "Як мене звати?"},
        "core_scope": "telegram_core_session",
        "query_text": "ім'я користувача",
        "recall_limit": 5,
        "session_recent_limit": 2,
        "session_trace_event_limit": 10,
        "include_core": True,
    }
    package = json.loads(engine.core_context_package(json.dumps(request)))

    assert any(
        fact["text"] == "Користувача звати Микита."
        for fact in package["core_facts"]
    )


def test_core_context_package_does_not_leak_facts_between_scopes(
    engine: memory_engine.MemoryEngine,
):
    _ingest(engine, "scoped_core_session", "Початок тесту.")
    for scope, name in [
        ("telegram_1", "Микита"),
        ("telegram_2", "Аліса"),
    ]:
        engine.upsert_core_fact(
            json.dumps(
                {
                    "schema_version": "core_fact_input.v1",
                    "category": "profile",
                    "scope": scope,
                    "text": f"Користувача звати {name}.",
                    "confidence": 0.95,
                    "tags": ["telegram", "name"],
                }
            )
        )

    request = {
        "schema_version": "core_context_request.v1",
        "session_id": "scoped_core_session",
        "domain_state": {"current_text": "Як мене звати?"},
        "core_scope": "telegram_2",
        "query_text": "ім'я користувача",
        "recall_limit": 5,
        "session_recent_limit": 2,
        "session_trace_event_limit": 10,
        "include_core": True,
    }
    package = json.loads(engine.core_context_package(json.dumps(request)))

    assert [fact["text"] for fact in package["core_facts"]] == [
        "Користувача звати Аліса."
    ]


def test_patch_core_fact_updates_and_deprecates_fact(engine: memory_engine.MemoryEngine):
    _ingest(engine, "patch_core_session", "Початок тесту.")
    upsert = json.loads(
        engine.upsert_core_fact(
            json.dumps(
                {
                    "schema_version": "core_fact_input.v1",
                    "category": "profile",
                    "scope": "telegram_patch",
                    "text": "Користувача звати Микита.",
                    "confidence": 0.95,
                    "tags": ["telegram", "name"],
                }
            )
        )
    )
    fact_id = upsert["fact"]["core_fact_id"]

    updated = json.loads(
        engine.patch_core_fact(
            json.dumps(
                {
                    "schema_version": "core_fact_patch_input.v1",
                    "core_fact_id": fact_id,
                    "scope": "telegram_patch",
                    "text": "Користувача звати Микита Загамула.",
                    "status": "active",
                }
            )
        )
    )
    assert updated["schema_version"] == "core_fact_patch_result.v1"
    assert updated["fact"]["text"] == "Користувача звати Микита Загамула."

    deprecated = json.loads(
        engine.patch_core_fact(
            json.dumps(
                {
                    "schema_version": "core_fact_patch_input.v1",
                    "core_fact_id": fact_id,
                    "scope": "telegram_patch",
                    "status": "deprecated",
                }
            )
        )
    )
    assert deprecated["fact"]["status"] == "deprecated"

    request = {
        "schema_version": "core_context_request.v1",
        "session_id": "patch_core_session",
        "domain_state": {"current_text": "Як мене звати?"},
        "core_scope": "telegram_patch",
        "query_text": "ім'я користувача",
        "recall_limit": 5,
        "session_recent_limit": 2,
        "session_trace_event_limit": 10,
        "include_core": True,
    }
    package = json.loads(engine.core_context_package(json.dumps(request)))
    assert package.get("core_facts", []) == []


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
