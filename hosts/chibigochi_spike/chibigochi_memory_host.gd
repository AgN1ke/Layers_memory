extends Node

const SESSION_ID := "chibigochi_spike"
const CORE_SCOPE := SESSION_ID

var engine: MemoryEngineGodot
var turn_index := 0
var last_error := ""

func open(memory_dir: String) -> bool:
    engine = MemoryEngineGodot.new()
    var opened: Variant = _loads(engine.open(memory_dir, "chibigochi_spike"))
    if _has_error(opened):
        return false
    return true

func send_user_message(text: String) -> String:
    turn_index += 1
    _ingest("user_message", "chibigochi_player", text, ["chibigochi_spike"])
    if last_error != "":
        return ""

    var package := context_package(text)
    if last_error != "":
        return ""
    var view := engine.render_memory_view(_dumps(package), text)
    if not view.contains("<current_user_message>"):
        _set_error("rendered memory view missed current_user_message")
        return ""

    var reply := _fake_assistant_reply(text, view)
    _ingest("assistant_message", "chibigochi_heroine", reply, ["chibigochi_reply"])
    return reply

func context_package(current_text: String) -> Dictionary:
    return _loads(engine.core_context_package(_dumps({
        "schema_version": "core_context_request.v1",
        "session_id": SESSION_ID,
        "domain_state": {"current_text": current_text, "host": "chibigochi_spike"},
        "core_scope": CORE_SCOPE,
        "query_text": current_text,
        "recall_limit": 5,
        "session_recent_limit": 8,
        "session_trace_event_limit": 20,
        "include_core": true,
    })))

func memory_view(current_text: String) -> String:
    return engine.render_memory_view(_dumps(context_package(current_text)), current_text)

func run_sleep() -> Dictionary:
    var run: Dictionary = _loads(engine.begin_sleep_run(SESSION_ID))
    if last_error != "":
        return {}

    while true:
        var step: Dictionary = _loads(engine.next_sleep_batch(_dumps(run)))
        if last_error != "":
            return {}
        run = step["run"]
        var batch: Variant = step.get("batch")
        if batch == null:
            break
        var responses: Array = []
        for request in batch["requests"]:
            responses.append(_response_for_request(run, request))
            if last_error != "":
                return {}
        step = _loads(engine.submit_sleep_batch(_dumps(run), _dumps(responses)))
        if last_error != "":
            return {}
        run = step["run"]

    var outcome: Dictionary = _loads(engine.finish_sleep_run(_dumps(run)))
    if last_error != "":
        return {}
    for request in outcome.get("fidelity_requests", []):
        _submit_valid_fidelity(request)
        if last_error != "":
            return {}
    return outcome

func core_fact_texts(current_text: String) -> String:
    var package: Dictionary = context_package(current_text)
    var texts: String = ""
    for fact in package.get("core_facts", []):
        texts += str(fact.get("text", "")) + "\n"
    return texts

func _fake_assistant_reply(text: String, view: String) -> String:
    var lower := text.to_lower()
    if lower.contains("cat") and view.contains("Irzha"):
        return "I remember Irzha: your cat is part of my long-term memory."
    if lower.contains("name") and view.contains("Mykyta"):
        return "I remember your name is Mykyta."
    return "Chibigochi heard you: %s" % text.substr(0, 80)

func _ingest(event_type: String, source: String, text: String, tags: Array) -> Dictionary:
    var timestamp := "2026-06-10T12:%02d:00.000Z" % turn_index
    return _loads(engine.ingest(_dumps({
        "schema_version": "event.v1",
        "type": event_type,
        "source": source,
        "timestamp": timestamp,
        "session_id": SESSION_ID,
        "payload": {"text": text},
        "tags": tags,
        "theme": "chibigochi_memory_spike",
        "importance_hint": "high" if event_type == "user_message" else "normal",
    })))

func _response_for_request(run: Dictionary, request: Dictionary) -> Dictionary:
    var prompt_id: String = request["prompt_id"]
    if prompt_id == "sleep_consolidator":
        return {
            "status": "ok",
            "request_id": request["request_id"],
            "text": (
                "GIST: Chibigochi learned the player's name, cat, and space interest.\n\n"
                + "The player introduced themselves as Mykyta, said they have a cat named Irzha, "
                + "and shared an interest in space. This should survive a Godot host restart."
            ),
        }

    var events := _sleep_events(request)
    var user_events := []
    for event in events:
        if _event_kind(event) == "user_message":
            user_events.append(event)
    if user_events.is_empty():
        user_events = events

    var payload := {}
    if prompt_id == "memory_unit_pass":
        payload = {
            "schema_version": "memory_units_result.v1",
            "archive_id": run["archive_id"],
            "memory_units": _memory_units_for_events(user_events),
        }
    elif prompt_id == "sleep_emotional_pass":
        payload = {"emotional_markers": _emotional_markers_for_events(user_events)}
    elif prompt_id == "sleep_topic_thread_pass":
        payload = {"topic_thread": [{
            "topic": "chibigochi_memory_spike",
            "summary": "The player introduced durable facts for a Godot product host spike.",
            "source_event_ids": _source_ids(user_events.slice(0, 3)),
        }]}
    elif prompt_id == "sleep_personal_signal_pass":
        payload = {"personal_signals": _personal_signals_for_events(user_events)}
    elif prompt_id == "sleep_relational_pass":
        payload = {"relational_tone": null}
    else:
        _set_error("unexpected LLM request prompt_id=%s" % prompt_id)
        return {}

    return {"status": "ok", "request_id": request["request_id"], "text": _dumps(payload)}

func _memory_units_for_events(events: Array) -> Array:
    var units := []
    for event in events:
        var text := _event_text(event)
        var source_id := _event_id(event)
        if text.contains("My name is Mykyta"):
            units.append({
                "thesis": "Name -> the player is named Mykyta.",
                "source_event_ids": [source_id],
                "evidence": text,
                "tags": ["name", "profile"],
                "weight": 0.95,
            })
        if text.contains("Irzha") or text.to_lower().contains("cat"):
            units.append({
                "thesis": "Cat -> the player has a cat named Irzha.",
                "source_event_ids": [source_id],
                "evidence": text,
                "tags": ["pet", "personal_memory"],
                "weight": 0.95,
            })
        if text.to_lower().contains("space"):
            units.append({
                "thesis": "Space -> the player likes space.",
                "source_event_ids": [source_id],
                "evidence": text,
                "tags": ["interest"],
                "weight": 0.9,
            })
    return units

func _personal_signals_for_events(events: Array) -> Array:
    var signals := []
    for event in events:
        var text := _event_text(event)
        var source_id := _event_id(event)
        if text.contains("My name is Mykyta"):
            signals.append({
                "text": "The player's name is Mykyta.",
                "category": "name",
                "confidence": 0.95,
                "source_event_ids": [source_id],
            })
        if text.contains("Irzha") or text.to_lower().contains("cat"):
            signals.append({
                "text": "The player has a cat named Irzha.",
                "category": "pet",
                "confidence": 0.95,
                "source_event_ids": [source_id],
            })
        if text.to_lower().contains("space"):
            signals.append({
                "text": "The player likes space.",
                "category": "interest",
                "confidence": 0.92,
                "source_event_ids": [source_id],
            })
    return signals

func _emotional_markers_for_events(events: Array) -> Array:
    var markers := []
    for event in events:
        var text := _event_text(event)
        if text.contains("Irzha") or text.to_lower().contains("cat"):
            markers.append({
                "target": "cat_irzha",
                "affect": "warmth",
                "strength": 0.9,
                "source_event_ids": [_event_id(event)],
                "quote": text,
            })
    return markers

func _submit_valid_fidelity(request: Dictionary) -> void:
    var unit: Dictionary = request.get("prompt_inputs", {}).get("memory_unit", {})
    var memory_unit_id = unit.get("memory_unit_id")
    var archive_id = unit.get("archive_id")
    if not (memory_unit_id is String) or not (archive_id is String):
        return
    var response := {
        "status": "ok",
        "request_id": request["request_id"],
        "text": _dumps({
            "schema_version": "fidelity_review.v1",
            "memory_unit_id": memory_unit_id,
            "archive_id": archive_id,
            "status": "valid",
            "confidence": 0.95,
            "explanation": "Chibigochi spike validator accepts source-backed unit.",
            "revised_thesis": null,
            "missing_detail": null,
        }),
    }
    _loads(engine.submit_memory_fidelity_response(request["task_id"], _dumps(response)))

func _sleep_events(request: Dictionary) -> Array:
    var inputs: Dictionary = request.get("prompt_inputs", {})
    var sleep_task: Variant = inputs.get("sleep_task")
    if sleep_task is Dictionary and sleep_task.get("events") is Array:
        return sleep_task["events"]
    if inputs.get("events") is Array:
        return inputs["events"]
    _set_error("request has no sleep events")
    return []

func _source_ids(events: Array) -> Array:
    var ids := []
    for event in events:
        ids.append(_event_id(event))
    return ids

func _event_text(event: Dictionary) -> String:
    var text: Variant = event.get("text")
    if text is String:
        return text
    var payload: Variant = event.get("payload")
    if payload is Dictionary and payload.get("text") is String:
        return payload["text"]
    return ""

func _event_kind(event: Dictionary) -> String:
    var value: Variant = event.get("event_type", event.get("type", ""))
    return value if value is String else ""

func _event_id(event: Dictionary) -> String:
    var value: Variant = event.get("event_id")
    if not (value is String) or value == "":
        _set_error("event has no event_id")
        return ""
    return value

func _loads(raw: String) -> Variant:
    var parsed: Variant = JSON.parse_string(raw)
    if parsed == null:
        _set_error("invalid JSON: %s" % raw)
        return {}
    if _has_error(parsed):
        return {}
    return parsed

func _dumps(value: Variant) -> String:
    return JSON.stringify(value)

func _has_error(value: Variant) -> bool:
    if value is Dictionary and value.has("error"):
        _set_error(str(value["error"]))
        return true
    return false

func _set_error(message: String) -> void:
    if last_error == "":
        last_error = message
