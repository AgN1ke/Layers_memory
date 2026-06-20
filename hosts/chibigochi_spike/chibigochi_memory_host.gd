extends Node

const FakeLlmBridgeScript = preload("res://chibigochi_fake_llm_bridge.gd")
const SESSION_ID := "chibigochi_spike"
const CORE_SCOPE := SESSION_ID

var engine: MemoryEngineGodot
var llm_bridge: Variant
var turn_index := 0
var last_error := ""

func set_llm_bridge(bridge: Variant) -> void:
    llm_bridge = bridge

func open(memory_dir: String) -> bool:
    var dir_error := DirAccess.make_dir_recursive_absolute(memory_dir)
    if dir_error != OK:
        _set_error("could not create memory directory: %s" % dir_error)
        return false
    if llm_bridge == null:
        llm_bridge = FakeLlmBridgeScript.new()
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

    var reply: String = llm_bridge.generate_chat_reply(text, view)
    if _bridge_failed():
        return ""
    _ingest("assistant_message", "chibigochi_heroine", reply, ["chibigochi_reply"])
    return reply

func send_user_message_async(text: String) -> String:
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

    var reply := ""
    if llm_bridge != null and llm_bridge.has_method("generate_chat_reply_async"):
        reply = await llm_bridge.generate_chat_reply_async(text, view)
    else:
        reply = llm_bridge.generate_chat_reply(text, view)
    if _bridge_failed():
        return ""
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
            var response: Dictionary = llm_bridge.execute_memory_request(run, request)
            if _bridge_failed():
                return {}
            responses.append(response)
        step = _loads(engine.submit_sleep_batch(_dumps(run), _dumps(responses)))
        if last_error != "":
            return {}
        run = step["run"]

    var outcome: Dictionary = _loads(engine.finish_sleep_run(_dumps(run)))
    if last_error != "":
        return {}
    for request in outcome.get("fidelity_requests", []):
        _submit_fidelity(request)
        if last_error != "":
            return {}
    return outcome

func run_sleep_async() -> Dictionary:
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
            var response: Dictionary
            if llm_bridge != null and llm_bridge.has_method("execute_memory_request_async"):
                response = await llm_bridge.execute_memory_request_async(run, request)
            else:
                response = llm_bridge.execute_memory_request(run, request)
            if _bridge_failed():
                return {}
            responses.append(response)
        step = _loads(engine.submit_sleep_batch(_dumps(run), _dumps(responses)))
        if last_error != "":
            return {}
        run = step["run"]

    var outcome: Dictionary = _loads(engine.finish_sleep_run(_dumps(run)))
    if last_error != "":
        return {}
    for request in outcome.get("fidelity_requests", []):
        await _submit_fidelity_async(request)
        if last_error != "":
            return {}
    return outcome

func core_fact_texts(current_text: String) -> String:
    var package: Dictionary = context_package(current_text)
    var texts: String = ""
    for fact in package.get("core_facts", []):
        texts += str(fact.get("text", "")) + "\n"
    return texts

func _submit_fidelity(request: Dictionary) -> void:
    var response: Dictionary = llm_bridge.execute_fidelity_request(request)
    if _bridge_failed():
        return
    _loads(engine.submit_memory_fidelity_response(request["task_id"], _dumps(response)))

func _submit_fidelity_async(request: Dictionary) -> void:
    var response: Dictionary
    if llm_bridge != null and llm_bridge.has_method("execute_fidelity_request_async"):
        response = await llm_bridge.execute_fidelity_request_async(request)
    else:
        response = llm_bridge.execute_fidelity_request(request)
    if _bridge_failed():
        return
    _loads(engine.submit_memory_fidelity_response(request["task_id"], _dumps(response)))

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

func _bridge_failed() -> bool:
    if llm_bridge != null and llm_bridge.get("last_error") is String and llm_bridge.last_error != "":
        _set_error(llm_bridge.last_error)
        return true
    return false

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
