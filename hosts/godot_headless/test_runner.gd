extends SceneTree

const SESSION_ID := "godot_headless_conformance"
const CORE_SCOPE := SESSION_ID

var engine: MemoryEngineGodot
var turn_index := 0

func _init() -> void:
    var runtime_dir := _arg_value("--runtime-dir")
    if runtime_dir == "":
        _fail("missing --runtime-dir")

    engine = MemoryEngineGodot.new()
    _expect_ok(engine.open(runtime_dir.path_join("memory"), "godot_headless_conformance"))

    _send_user_message("Мене звати Микита.")
    _send_user_message("У мене є кішка Іржа.")
    _send_user_message("Я люблю космос і хочу, щоб Godot host не мав власної memory logic.")

    var outcome := _run_sleep()
    var archive: Dictionary = outcome.get("archive_entry", {})
    _assert_sleep_archive(archive)

    var current_text := "Що ти пам'ятаєш про Іржу?"
    var view := _render_memory_view(current_text)
    _assert_memory_view(view, current_text)

    var package := _context_package(current_text)
    _assert_core(package)

    print("HOST CONFORMANCE PASSED")
    print("host=godot-headless")
    print("archive_id=%s" % archive.get("archive_id", ""))
    print("memory_units=%s" % archive.get("memory_units", []).size())
    print("core_facts=%s" % package.get("core_facts", []).size())
    quit(0)

func _arg_value(name: String) -> String:
    var args := OS.get_cmdline_user_args()
    if args.is_empty():
        args = OS.get_cmdline_args()
    for index in range(args.size()):
        if args[index] == name and index + 1 < args.size():
            return args[index + 1]
    return ""

func _send_user_message(text: String) -> String:
    turn_index += 1
    _ingest("user_message", "godot_user", text, ["host_conformance"])
    var package := _context_package(text)
    var view := engine.render_memory_view(_dumps(package), text)
    if not view.contains("<current_user_message>"):
        _fail("rendered memory view missed current_user_message")
    var reply := "ACK %s: %s" % [turn_index, text.substr(0, 48)]
    _ingest("assistant_message", "godot_assistant", reply, ["host_conformance_reply"])
    return reply

func _run_sleep() -> Dictionary:
    var run := _loads(engine.begin_sleep_run(SESSION_ID))
    while true:
        var step := _loads(engine.next_sleep_batch(_dumps(run)))
        run = step["run"]
        var batch = step.get("batch")
        if batch == null:
            break
        var responses := []
        for request in batch["requests"]:
            responses.append(_response_for_request(run, request))
        step = _loads(engine.submit_sleep_batch(_dumps(run), _dumps(responses)))
        run = step["run"]
    var outcome := _loads(engine.finish_sleep_run(_dumps(run)))
    for request in outcome.get("fidelity_requests", []):
        _submit_valid_fidelity(request)
    return outcome

func _context_package(current_text: String) -> Dictionary:
    return _loads(engine.core_context_package(_dumps({
        "schema_version": "core_context_request.v1",
        "session_id": SESSION_ID,
        "domain_state": {"current_text": current_text},
        "core_scope": CORE_SCOPE,
        "query_text": current_text,
        "recall_limit": 5,
        "session_recent_limit": 8,
        "session_trace_event_limit": 20,
        "include_core": true,
    })))

func _render_memory_view(current_text: String) -> String:
    return engine.render_memory_view(_dumps(_context_package(current_text)), current_text)

func _ingest(event_type: String, source: String, text: String, tags: Array) -> Dictionary:
    var timestamp := "2026-06-10T10:%02d:00.000Z" % turn_index
    return _loads(engine.ingest(_dumps({
        "schema_version": "event.v1",
        "type": event_type,
        "source": source,
        "timestamp": timestamp,
        "session_id": SESSION_ID,
        "payload": {"text": text},
        "tags": tags,
        "theme": "host_conformance",
        "importance_hint": "high" if event_type == "user_message" else "normal",
    })))

func _response_for_request(run: Dictionary, request: Dictionary) -> Dictionary:
    var prompt_id: String = request["prompt_id"]
    if prompt_id == "sleep_consolidator":
        return {
            "status": "ok",
            "request_id": request["request_id"],
            "text": (
                "GIST: Godot host зберіг ім'я Микити, кішку Іржу і космічний інтерес.\n\n"
                + "Headless Godot host пройшов sleep через GDExtension adapter і fake LLM."
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
        var source_ids := []
        for event in user_events.slice(0, 3):
            source_ids.append(_event_id(event))
        payload = {
            "topic_thread": [{
                "topic": "godot_headless_conformance",
                "summary": "Godot host перевіряє ім'я, Іржу і космос.",
                "source_event_ids": source_ids,
            }]
        }
    elif prompt_id == "sleep_personal_signal_pass":
        payload = {"personal_signals": _personal_signals_for_events(user_events)}
    elif prompt_id == "sleep_relational_pass":
        payload = {"relational_tone": null}
    else:
        _fail("unexpected LLM request prompt_id=%s" % prompt_id)

    return {"status": "ok", "request_id": request["request_id"], "text": _dumps(payload)}

func _sleep_events(request: Dictionary) -> Array:
    var inputs: Dictionary = request.get("prompt_inputs", {})
    var sleep_task = inputs.get("sleep_task")
    if sleep_task is Dictionary and sleep_task.get("events") is Array:
        return sleep_task["events"]
    if inputs.get("events") is Array:
        return inputs["events"]
    _fail("request has no sleep events")
    return []

func _memory_units_for_events(events: Array) -> Array:
    var units := []
    for event in events:
        var text := _event_text(event)
        var source_id := _event_id(event)
        if text.contains("Мене звати Микита"):
            units.append({
                "thesis": "Ім'я -> користувача звати Микита.",
                "source_event_ids": [source_id],
                "evidence": text,
                "tags": ["name", "profile"],
                "weight": 0.95,
            })
        if text.contains("Іржа") or text.to_lower().contains("кішк"):
            units.append({
                "thesis": "Кішка Іржа -> у користувача є кішка Іржа.",
                "source_event_ids": [source_id],
                "evidence": text,
                "tags": ["pet", "personal_memory"],
                "weight": 0.95,
            })
        if text.to_lower().contains("космос"):
            units.append({
                "thesis": "Космос -> користувач любить тему космосу.",
                "source_event_ids": [source_id],
                "evidence": text,
                "tags": ["interest"],
                "weight": 0.9,
            })
    if units.is_empty() and not events.is_empty():
        units.append({
            "thesis": "Conformance dialogue -> коротка перевірка Godot memory path.",
            "source_event_ids": [_event_id(events[0])],
            "evidence": _event_text(events[0]),
            "tags": ["host_conformance"],
            "weight": 0.7,
        })
    return units

func _personal_signals_for_events(events: Array) -> Array:
    var signals := []
    for event in events:
        var text := _event_text(event)
        var source_id := _event_id(event)
        if text.contains("Мене звати Микита"):
            signals.append({
                "text": "Користувача звати Микита.",
                "category": "name",
                "confidence": 0.95,
                "source_event_ids": [source_id],
            })
        if text.contains("Іржа") or text.to_lower().contains("кішк"):
            signals.append({
                "text": "У користувача є кішка Іржа.",
                "category": "pet",
                "confidence": 0.95,
                "source_event_ids": [source_id],
            })
        if text.to_lower().contains("люблю космос"):
            signals.append({
                "text": "Користувач любить космос.",
                "category": "interest",
                "confidence": 0.92,
                "source_event_ids": [source_id],
            })
    return signals

func _emotional_markers_for_events(events: Array) -> Array:
    var markers := []
    for event in events:
        var text := _event_text(event)
        if text.contains("Іржа") or text.to_lower().contains("кішк"):
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
            "explanation": "Godot fake validator accepts the source-backed unit.",
            "revised_thesis": null,
            "missing_detail": null,
        }),
    }
    var raw := engine.submit_memory_fidelity_response(request["task_id"], _dumps(response))
    var parsed := _loads(raw)
    if parsed.has("error"):
        _fail("fidelity submit failed: %s" % raw)

func _assert_sleep_archive(archive: Dictionary) -> void:
    if archive.get("status") != "complete":
        _fail("sleep did not complete: %s" % archive.get("status"))
    var units: Array = archive.get("memory_units", [])
    if units.size() < 2:
        _fail("expected at least two memory units, got %s" % units.size())

func _assert_memory_view(view: String, current_text: String) -> void:
    for marker in ["<memory_context>", "<core_memory>", "<long_memory>", "<short_memory>", "<current_user_message>", current_text]:
        if not view.contains(marker):
            _fail("memory view missing %s\n%s" % [marker, view])
    if view.contains("user: %s" % current_text):
        _fail("current user message leaked into short_memory duplicate")

func _assert_core(package: Dictionary) -> void:
    var texts := ""
    for fact in package.get("core_facts", []):
        texts += str(fact.get("text", "")) + "\n"
    if not texts.contains("Микита"):
        _fail("core facts do not include user name:\n%s" % texts)
    if not texts.contains("Іржа"):
        _fail("core facts do not include Irzha:\n%s" % texts)

func _event_text(event: Dictionary) -> String:
    var text = event.get("text")
    if text is String:
        return text
    var payload = event.get("payload")
    if payload is Dictionary and payload.get("text") is String:
        return payload["text"]
    return ""

func _event_kind(event: Dictionary) -> String:
    var value = event.get("event_type", event.get("type", ""))
    return value if value is String else ""

func _event_id(event: Dictionary) -> String:
    var value = event.get("event_id")
    if not (value is String) or value == "":
        _fail("event has no event_id")
    return value

func _expect_ok(raw: String) -> void:
    var parsed := _loads(raw)
    if parsed.has("error"):
        _fail(raw)

func _loads(raw: String):
    var parsed = JSON.parse_string(raw)
    if parsed == null:
        _fail("invalid JSON: %s" % raw)
    return parsed

func _dumps(value) -> String:
    return JSON.stringify(value)

func _fail(message: String) -> void:
    push_error(message)
    printerr("HOST CONFORMANCE FAILED: %s" % message)
    quit(1)
