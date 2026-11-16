extends SceneTree

const SESSION_ID := "godot_headless_conformance"
const CORE_SCOPE := SESSION_ID

var engine: MemoryEngineGodot
var turn_index := 0
var failed := false

func _init() -> void:
    var runtime_dir := _arg_value("--runtime-dir")
    if runtime_dir == "":
        _fail("missing --runtime-dir")
        return

    engine = MemoryEngineGodot.new()
    var build: Variant = _loads(engine.build_info())
    if not (build is Dictionary) or not build.has_all(["version", "commit", "dirty"]):
        _fail("build_info must return version, commit and dirty before open")
        return
    if not (build.version is String) or build.version.is_empty():
        _fail("build_info is missing the package version")
        return
    if build.commit != null and (not (build.commit is String) or build.commit.length() not in [40, 64]):
        _fail("build_info must report a full SHA or null")
        return
    if build.dirty != null and not (build.dirty is bool):
        _fail("build_info dirty must be a boolean or null")
        return
    var expected_commit := OS.get_environment("MEMORY_ENGINE_BUILD_COMMIT")
    var expected_dirty := OS.get_environment("MEMORY_ENGINE_BUILD_DIRTY")
    if not expected_commit.is_empty() and build.commit != expected_commit.to_lower():
        _fail("binary commit does not match the build input")
        return
    if not expected_dirty.is_empty() and build.dirty != (expected_dirty == "true"):
        _fail("binary dirty flag does not match the build input")
        return
    _expect_ok(engine.open(runtime_dir.path_join("memory"), "godot_headless_conformance"))
    if failed:
        return

    var pruned: Variant = _loads(engine.prune_completed_memory_operations())
    if not (pruned is Dictionary) or pruned.get("schema_version") != "memory_operation_prune_report.v1" or pruned.get("tasks_compacted") != 0:
        _fail("pruning a fresh FileStorage must return an empty maintenance report")
        return

    _assert_idempotent_ingest(runtime_dir.path_join("memory"))
    if failed:
        return

    _assert_guarded_ingest(runtime_dir.path_join("memory"))
    if failed:
        return

    _send_user_message("Мене звати Микита.")
    if failed:
        return
    _send_user_message("У мене є кішка Іржа.")
    if failed:
        return
    _send_user_message("Я люблю космос і хочу, щоб Godot host не мав власної memory logic.")
    if failed:
        return

    var outcome: Dictionary = _run_sleep()
    if failed:
        return
    var archive: Dictionary = outcome.get("archive_entry", {})
    _assert_sleep_archive(archive)
    if failed:
        return

    var current_text := "Що ти пам'ятаєш про Іржу?"
    var view := _render_memory_view(current_text)
    _assert_memory_view(view, current_text)
    if failed:
        return

    var package: Dictionary = _context_package(current_text)
    if failed:
        return
    _assert_core(package)
    if failed:
        return
    _assert_unknown_request_fields(current_text, archive["archive_id"])
    if failed:
        return

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

func _assert_idempotent_ingest(memory_dir: String) -> void:
    var event := _dumps({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "godot_user",
        "timestamp": "2026-11-16T12:00:00Z",
        "session_id": "godot_idempotent_ingest",
        "payload": {"text": "Записати один раз."},
    })
    var first: Variant = _loads(engine.ingest_idempotent("retry-1", event))
    if not (first is Dictionary) or first.get("already_applied") != false or not first.has("event"):
        _fail("idempotent ingest must apply a new operation")
        return
    _expect_ok(engine.open(memory_dir, "godot_headless_conformance"))
    if failed:
        return
    var replay: Variant = _loads(engine.ingest_idempotent("retry-1", event))
    if not (replay is Dictionary) or replay.get("already_applied") != true or replay.get("event") != first["event"]:
        _fail("idempotent ingest must return the original event after reopening")

func _assert_guarded_ingest(memory_dir: String) -> void:
    var session := "godot_guarded_ingest"
    var generation := engine.write_generation(session, "")
    var prior: Variant = _loads(generation)
    if not (prior is Dictionary) or prior.get("schema_version") != "write_generation.v1":
        _fail("write_generation must return a generation snapshot")
        return
    var erased: Variant = _loads(engine.delete_session(session))
    if not (erased is Dictionary) or erased.has("error"):
        _fail("delete_session must erase the captured generation")
        return
    _expect_ok(engine.open(memory_dir, "godot_headless_conformance"))
    if failed:
        return
    var event := _dumps({
        "schema_version": "event.v1",
        "type": "user_message",
        "source": "godot_user",
        "timestamp": "2026-11-16T12:00:00Z",
        "session_id": session,
        "payload": {"text": "Затримана подія."},
    })
    var stale: Variant = _loads(engine.ingest_idempotent_at_generation("guarded-old", event, generation))
    if not (stale is Dictionary) or stale.get("code") != "operation_stale":
        _fail("guarded ingest must refuse an event from before erasure")
        return
    var current := engine.write_generation(session, "")
    var accepted: Variant = _loads(engine.ingest_idempotent_at_generation("guarded-fresh", event, current))
    if not (accepted is Dictionary) or accepted.get("already_applied") != false:
        _fail("guarded ingest must accept a fresh event after erasure")

func _send_user_message(text: String) -> String:
    turn_index += 1
    _ingest("user_message", "godot_user", text, ["host_conformance"])
    if failed:
        return ""
    var package: Dictionary = _context_package(text)
    if failed:
        return ""
    var view := engine.render_memory_view(_dumps(package), text)
    if not view.contains("<current_user_message>"):
        _fail("rendered memory view missed current_user_message")
        return ""
    var reply := "ACK %s: %s" % [turn_index, text.substr(0, 48)]
    _ingest("assistant_message", "godot_assistant", reply, ["host_conformance_reply"])
    return reply

func _run_sleep() -> Dictionary:
    var run: Dictionary = _loads(engine.begin_sleep_run(SESSION_ID))
    if failed:
        return {}
    while true:
        var step: Dictionary = _loads(engine.next_sleep_batch(_dumps(run)))
        if failed:
            return {}
        run = step["run"]
        var batch = step.get("batch")
        if batch == null:
            break
        var responses := []
        for request in batch["requests"]:
            responses.append(_response_for_request(run, request))
            if failed:
                return {}
        step = _loads(engine.submit_sleep_batch(_dumps(run), _dumps(responses)))
        if failed:
            return {}
        run = step["run"]
    var outcome: Dictionary = _loads(engine.finish_sleep_run(_dumps(run)))
    if failed:
        return {}
    for request in outcome.get("fidelity_requests", []):
        _submit_valid_fidelity(request)
        if failed:
            return {}
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
        "personality_scope_id": CORE_SCOPE,
        "memory_text": text,
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

    var events := _sleep_events(run, request)
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
        return {}

    return {"status": "ok", "request_id": request["request_id"], "text": _dumps(payload)}

func _sleep_events(run: Dictionary, request: Dictionary) -> Array:
    var shared_context: Variant = run.get("shared_prompt_context")
    if shared_context is Dictionary:
        var shared_events := _events_from_shared_context(shared_context)
        if not shared_events.is_empty():
            return shared_events
    var inputs: Dictionary = request.get("prompt_inputs", {})
    var sleep_task = inputs.get("sleep_task")
    if sleep_task is Dictionary and sleep_task.get("events") is Array:
        return sleep_task["events"]
    if inputs.get("events") is Array:
        return inputs["events"]
    _fail("request has no sleep events")
    return []

func _events_from_shared_context(shared_context: Dictionary) -> Array:
    var content: Variant = shared_context.get("content")
    if not (content is String):
        return []
    var events := []
    for line in content.split("\n"):
        if not line.begins_with("["):
            continue
        var closing: int = line.find("] ")
        if closing <= 1:
            continue
        var fields: PackedStringArray = line.substr(1, closing - 1).split("|")
        if fields.size() < 4:
            continue
        events.append({
            "event_id": fields[0],
            "timestamp": fields[1],
            "type": fields[2],
            "source": fields[3],
            "text": line.substr(closing + 2),
        })
    return events

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
    var parsed: Dictionary = _loads(raw)
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

func _assert_unknown_request_fields(current_text: String, archive_id: String) -> void:
    var domain_state := {"subject_id": "host-owned", "nested": {"custom_key": true}}
    var package: Dictionary = _loads(engine.core_context_package(_dumps({
        "schema_version": "core_context_request.v1",
        "session_id": SESSION_ID,
        "core_scope": CORE_SCOPE,
        "query_text": current_text,
        "include_core": true,
        "recall_limit": 5.0,
        "subject_id": "ignored",
        "domain_state": domain_state,
    })))
    if failed:
        return
    if package.get("notes", []) != ['ignored unknown request field: "/subject_id"']:
        _fail("context must report only the unknown request field: %s" % str(package.get("notes")))
        return
    if package.get("domain_state") != domain_state:
        _fail("context must preserve the host's domain_state")
        return
    _assert_core(package)
    if failed:
        return

    var recall: Dictionary = _loads(engine.recall(_dumps({
        "schema_version": "recall_query.v1",
        "session_id": SESSION_ID,
        "query_text": current_text,
        "context": {"host_data": {"custom_key": true}},
        "filters": {
            "source_layers": ["archive"],
            "time_range": {
                "start": "2026-01-01T00:00:00Z",
                "end": "2099-01-01T00:00:00Z",
                "unexpected": true,
            },
        },
        "limit": 1.0,
        "include_core": false,
        "explain": true,
    })))
    if failed:
        return
    var expected := ['ignored unknown request field: "/filters/time_range/unexpected"']
    if recall.get("notes", []) != expected or recall.get("debug", {}).get("notes", []) != expected:
        _fail("recall must report the nested unknown field in notes and debug.notes: %s" % str(recall.get("notes")))
        return
    var items: Array = recall.get("items", [])
    if items.size() != 1 or items[0].get("id") != archive_id:
        _fail("recall must still return the expected archive")

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
        return ""
    return value

func _expect_ok(raw: String) -> void:
    var parsed: Dictionary = _loads(raw)
    if failed:
        return
    if parsed.has("error"):
        _fail(raw)

func _loads(raw: String) -> Variant:
    var parsed = JSON.parse_string(raw)
    if parsed == null:
        _fail("invalid JSON: %s" % raw)
        return null
    return parsed

func _dumps(value: Variant) -> String:
    return JSON.stringify(value)

func _fail(message: String) -> void:
    if failed:
        return
    failed = true
    push_error(message)
    printerr("HOST CONFORMANCE FAILED: %s" % message)
    quit(1)
