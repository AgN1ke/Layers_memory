extends Node

var endpoint_url := ""
var api_key := ""
var timeout_seconds := 60.0
var last_error := ""

func configure(endpoint: String, key: String = "", timeout: float = 60.0) -> void:
    endpoint_url = endpoint
    api_key = key
    timeout_seconds = timeout

func generate_chat_reply_async(text: String, memory_view: String) -> String:
    var response: Dictionary = await _post_json_async({
        "operation": "chat_reply",
        "input_text": text,
        "memory_view": memory_view,
    })
    if response.is_empty():
        return ""
    return str(response.get("text", ""))

func execute_memory_request_async(run: Dictionary, request: Dictionary) -> Dictionary:
    var response: Dictionary = await _post_json_async({
        "operation": "memory_request",
        "run": run,
        "request": request,
        "role_hint": request.get("role_hint", ""),
        "prompt_id": request.get("prompt_id", ""),
        "prompt_inputs": request.get("prompt_inputs", {}),
    })
    return _normalize_llm_response(response, request)

func execute_fidelity_request_async(request: Dictionary) -> Dictionary:
    var response: Dictionary = await _post_json_async({
        "operation": "memory_fidelity_pass",
        "request": request,
        "role_hint": request.get("role_hint", ""),
        "prompt_id": request.get("prompt_id", ""),
        "prompt_inputs": request.get("prompt_inputs", {}),
    })
    return _normalize_llm_response(response, request)

func _normalize_llm_response(response: Dictionary, request: Dictionary) -> Dictionary:
    if response.is_empty():
        return {}
    if not response.has("request_id"):
        response["request_id"] = request.get("request_id", "")
    if not response.has("status"):
        response["status"] = "ok"
    return response

func _post_json_async(payload: Dictionary) -> Dictionary:
    last_error = ""
    if endpoint_url == "":
        _set_error("LLM bridge endpoint is not configured")
        return {}

    var request := HTTPRequest.new()
    request.timeout = timeout_seconds
    add_child(request)

    var headers := PackedStringArray(["Content-Type: application/json"])
    if api_key != "":
        headers.append("Authorization: Bearer %s" % api_key)

    var err := request.request(endpoint_url, headers, HTTPClient.METHOD_POST, JSON.stringify(payload))
    if err != OK:
        request.queue_free()
        _set_error("LLM bridge request failed: %s" % err)
        return {}

    var completed: Array = await request.request_completed
    request.queue_free()
    if completed.size() < 4:
        _set_error("LLM bridge returned malformed HTTPRequest result")
        return {}

    var result_code: int = completed[0]
    var status_code: int = completed[1]
    var body: PackedByteArray = completed[3]
    if result_code != HTTPRequest.RESULT_SUCCESS:
        _set_error("LLM bridge request result: %s" % result_code)
        return {}
    if status_code < 200 or status_code >= 300:
        _set_error("LLM bridge HTTP status %s: %s" % [status_code, body.get_string_from_utf8()])
        return {}

    var parsed: Variant = JSON.parse_string(body.get_string_from_utf8())
    if not (parsed is Dictionary):
        _set_error("LLM bridge returned non-object JSON")
        return {}
    if parsed.has("error"):
        _set_error(str(parsed["error"]))
        return {}
    return parsed

func _set_error(message: String) -> void:
    if last_error == "":
        last_error = message
