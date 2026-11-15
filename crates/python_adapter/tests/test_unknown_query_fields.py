"""Report ignored query fields while preserving the typed request behavior."""

import json

import pytest


WARNING_PREFIX = "ignored unknown request field: "
IGNORED_VALUE = "this value must not leak into the response"


def _assert_only_notes_changed(baseline, actual, *paths):
    for response in (baseline, actual):
        response.pop("created_at", None)
    existing_notes = baseline.pop("notes", [])
    expected_notes = [f"{WARNING_PREFIX}{json.dumps(path)}" for path in paths]
    assert not any(note.startswith(WARNING_PREFIX) for note in existing_notes)
    assert sorted(actual.pop("notes", [])) == sorted(existing_notes + expected_notes)
    assert actual == baseline


def test_context_reports_ignored_subject_and_budget_fields(tmp_path, engine_factory):
    engine = engine_factory(tmp_path)
    domain_state = {"host_extension": {"subject_id": "host-owned", "limit_hint": 8}}
    budget = {
        "total_tokens": 1000,
        "current_memory_tokens": 600,
        "compressed_memory_tokens": 300,
        "core_tokens": 100,
    }
    request = {
        "schema_version": "core_context_request.v1",
        "session_id": "context_session",
        "domain_state": domain_state,
        "token_budget": budget,
    }
    baseline = json.loads(engine.core_context_package(json.dumps(request)))
    assert baseline["domain_state"] == domain_state

    request["subject_id"] = IGNORED_VALUE
    budget["limit/hint~"] = {"private": IGNORED_VALUE}
    response = engine.core_context_package(json.dumps(request))
    assert IGNORED_VALUE not in response
    _assert_only_notes_changed(
        baseline, json.loads(response), "/subject_id", "/token_budget/limit~1hint~0"
    )


def _recall_query():
    return {
        "schema_version": "recall_query.v1",
        "session_id": "recall_session",
        "context": {"custom": {"limit_hint": 9}},
        "filters": {"source_layers": ["archive"]},
        "include_core": False,
        "limit": 4,
        "explain": True,
    }


def test_recall_reports_ignored_filter_fields(tmp_path, engine_factory):
    engine = engine_factory(tmp_path)
    request = _recall_query()
    baseline = json.loads(engine.recall(json.dumps(request)))
    request["filters"]["limit_hint"] = IGNORED_VALUE
    response = engine.recall(json.dumps(request))
    assert IGNORED_VALUE not in response
    actual = json.loads(response)
    expected_debug = baseline.pop("debug")
    actual_debug = actual.pop("debug")
    _assert_only_notes_changed(expected_debug, actual_debug, "/filters/limit_hint")
    _assert_only_notes_changed(baseline, actual, "/filters/limit_hint")


@pytest.mark.parametrize("method", ["core_context_package", "recall"])
def test_query_still_rejects_malformed_json(tmp_path, engine_factory, method):
    engine = engine_factory(tmp_path)
    query = getattr(engine, method)
    valid = _recall_query() if method == "recall" else {
        "schema_version": "core_context_request.v1",
        "session_id": "context_session",
    }
    with pytest.raises(ValueError, match="JSON"):
        query("{")
    with pytest.raises(ValueError, match="trailing characters"):
        query(json.dumps(valid) + " {}")


def test_recall_still_rejects_wrong_type_for_supported_field(tmp_path, engine_factory):
    engine = engine_factory(tmp_path)
    request = _recall_query()
    request["limit"] = "four"
    request["unknown"] = True
    with pytest.raises(ValueError, match="JSON"):
        engine.recall(json.dumps(request))


def test_recall_still_rejects_duplicate_supported_field(tmp_path, engine_factory):
    engine = engine_factory(tmp_path)
    request = json.dumps(_recall_query())[:-1] + ', "limit": 2}'
    with pytest.raises(ValueError, match="duplicate field"):
        engine.recall(request)
