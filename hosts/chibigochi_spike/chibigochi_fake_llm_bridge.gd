extends RefCounted

var last_error := ""

func generate_chat_reply(text: String, memory_view: String) -> String:
    var lower := text.to_lower()
    if lower.contains("cat") and memory_view.contains("Irzha"):
        return "I remember Irzha: your cat is part of my long-term memory."
    if lower.contains("name") and memory_view.contains("Mykyta"):
        return "I remember your name is Mykyta."
    return "Chibigochi heard you: %s" % text.substr(0, 80)

func execute_memory_request(run: Dictionary, request: Dictionary) -> Dictionary:
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

func execute_fidelity_request(request: Dictionary) -> Dictionary:
    var inputs: Dictionary = request.get("prompt_inputs", {})
    var unit: Dictionary = inputs.get("memory_unit", {})
    if unit.is_empty() and inputs.get("evidence_pack") is Dictionary:
        unit = inputs["evidence_pack"]
    var memory_unit_id = unit.get("memory_unit_id")
    var archive_id = unit.get("archive_id")
    if not (memory_unit_id is String) or not (archive_id is String):
        _set_error("fidelity request missed memory_unit_id/archive_id")
        return {}
    return {
        "status": "ok",
        "request_id": request["request_id"],
        "text": _dumps({
            "schema_version": "fidelity_review.v1",
            "memory_unit_id": memory_unit_id,
            "archive_id": archive_id,
            "status": "valid",
            "confidence": 0.95,
            "explanation": "Chibigochi fake validator accepts source-backed unit.",
            "revised_thesis": null,
            "missing_detail": null,
        }),
    }

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

func _dumps(value: Variant) -> String:
    return JSON.stringify(value)

func _set_error(message: String) -> void:
    if last_error == "":
        last_error = message
