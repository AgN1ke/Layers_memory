extends RefCounted

var endpoint_url := ""
var api_key := ""
var timeout_seconds := 60.0
var last_error := ""

func configure(endpoint: String, key: String = "", timeout: float = 60.0) -> void:
    endpoint_url = endpoint
    api_key = key
    timeout_seconds = timeout

func generate_chat_reply(text: String, memory_view: String) -> String:
    var response := _post_json({
        "operation": "chat_reply",
        "input_text": text,
        "memory_view": memory_view,
    })
    if response.is_empty():
        return ""
    return str(response.get("text", ""))

func execute_memory_request(run: Dictionary, request: Dictionary) -> Dictionary:
    var response := _post_json({
        "operation": "memory_request",
        "run": run,
        "request": request,
        "role_hint": request.get("role_hint", ""),
        "prompt_id": request.get("prompt_id", ""),
        "prompt_inputs": request.get("prompt_inputs", {}),
    })
    return _normalize_llm_response(response, request)

func execute_fidelity_request(request: Dictionary) -> Dictionary:
    var response := _post_json({
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

func _post_json(payload: Dictionary) -> Dictionary:
    last_error = ""
    if endpoint_url == "":
        _set_error("LLM bridge endpoint is not configured")
        return {}

    var endpoint := _parse_endpoint(endpoint_url)
    if endpoint.is_empty():
        return {}

    var http := HTTPClient.new()
    var tls_options: TLSOptions = TLSOptions.client() if endpoint["use_tls"] else null
    var err := http.connect_to_host(endpoint["host"], endpoint["port"], tls_options)
    if err != OK:
        _set_error("LLM bridge connect failed: %s" % err)
        return {}

    if not _poll_until_connected(http):
        return {}

    var headers := PackedStringArray(["Content-Type: application/json"])
    if api_key != "":
        headers.append("Authorization: Bearer %s" % api_key)
    err = http.request(HTTPClient.METHOD_POST, endpoint["path"], headers, JSON.stringify(payload))
    if err != OK:
        _set_error("LLM bridge request failed: %s" % err)
        return {}

    if not _poll_until_response(http):
        return {}
    var status_code := http.get_response_code()
    var body := _read_response_body(http)
    if last_error != "":
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

func _parse_endpoint(raw: String) -> Dictionary:
    var scheme_end := raw.find("://")
    if scheme_end <= 0:
        _set_error("LLM bridge endpoint must start with http:// or https://")
        return {}
    var scheme := raw.substr(0, scheme_end).to_lower()
    if scheme != "http" and scheme != "https":
        _set_error("unsupported LLM bridge endpoint scheme: %s" % scheme)
        return {}

    var rest := raw.substr(scheme_end + 3)
    var path_start := rest.find("/")
    var authority := rest if path_start < 0 else rest.substr(0, path_start)
    var path := "/" if path_start < 0 else rest.substr(path_start)
    if authority == "":
        _set_error("LLM bridge endpoint missed host")
        return {}

    var host := authority
    var port := 443 if scheme == "https" else 80
    var colon := authority.rfind(":")
    if colon > 0:
        host = authority.substr(0, colon)
        port = authority.substr(colon + 1).to_int()
    if host == "" or port <= 0:
        _set_error("invalid LLM bridge endpoint authority: %s" % authority)
        return {}
    return {
        "host": host,
        "port": port,
        "path": path,
        "use_tls": scheme == "https",
    }

func _poll_until_connected(http: HTTPClient) -> bool:
    var deadline := _deadline_ms()
    while http.get_status() == HTTPClient.STATUS_CONNECTING or http.get_status() == HTTPClient.STATUS_RESOLVING:
        if not _poll_once(http, deadline):
            return false
        OS.delay_msec(10)
    if http.get_status() != HTTPClient.STATUS_CONNECTED:
        _set_error("LLM bridge connection status: %s" % http.get_status())
        return false
    return true

func _poll_until_response(http: HTTPClient) -> bool:
    var deadline := _deadline_ms()
    while http.get_status() == HTTPClient.STATUS_REQUESTING:
        if not _poll_once(http, deadline):
            return false
        OS.delay_msec(10)
    if not http.has_response():
        _set_error("LLM bridge returned no HTTP response")
        return false
    return true

func _read_response_body(http: HTTPClient) -> PackedByteArray:
    var body := PackedByteArray()
    var deadline := _deadline_ms()
    while http.get_status() == HTTPClient.STATUS_BODY:
        if not _poll_once(http, deadline):
            return body
        var chunk := http.read_response_body_chunk()
        if not chunk.is_empty():
            body.append_array(chunk)
        else:
            OS.delay_msec(10)
    return body

func _deadline_ms() -> int:
    return Time.get_ticks_msec() + int(timeout_seconds * 1000.0)

func _poll_once(http: HTTPClient, deadline: int) -> bool:
    if Time.get_ticks_msec() > deadline:
        _set_error("LLM bridge timed out after %s seconds" % timeout_seconds)
        return false
    var err := http.poll()
    if err != OK:
        _set_error("LLM bridge poll failed: %s" % err)
        return false
    return true

func _set_error(message: String) -> void:
    if last_error == "":
        last_error = message
