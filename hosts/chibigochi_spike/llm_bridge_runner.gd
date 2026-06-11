extends SceneTree

const ChibigochiMemoryHostScript = preload("res://chibigochi_memory_host.gd")
const HttpLlmBridgeScript = preload("res://chibigochi_http_llm_bridge.gd")

var failed := false

func _init() -> void:
    var runtime_dir := _arg_value("--runtime-dir")
    var endpoint := _arg_value("--llm-endpoint")
    if runtime_dir == "":
        _fail("missing --runtime-dir")
        return
    if endpoint == "":
        _fail("missing --llm-endpoint")
        return

    var bridge: Variant = HttpLlmBridgeScript.new()
    bridge.configure(endpoint)

    var memory_dir := runtime_dir.path_join("memory")
    var host: Variant = ChibigochiMemoryHostScript.new()
    host.set_llm_bridge(bridge)
    if not host.open(memory_dir):
        _fail(host.last_error)
        return

    var first_reply: String = host.send_user_message("My name is Mykyta.")
    _assert_host_ok(host)
    if failed:
        return
    if not first_reply.contains("Mykyta"):
        _fail("HTTP bridge first reply did not acknowledge current user text")
        return

    host.send_user_message("I have a cat named Irzha.")
    _assert_host_ok(host)
    if failed:
        return
    host.send_user_message("I like space and want the HTTP bridge to preserve memory.")
    _assert_host_ok(host)
    if failed:
        return

    var outcome: Dictionary = host.run_sleep()
    _assert_host_ok(host)
    if failed:
        return
    var archive: Dictionary = outcome.get("archive_entry", {})
    _assert_archive(archive)
    if failed:
        return
    _assert_core_texts(host.core_fact_texts("What do you know about me?"))
    if failed:
        return

    var restarted: Variant = ChibigochiMemoryHostScript.new()
    restarted.set_llm_bridge(bridge)
    if not restarted.open(memory_dir):
        _fail(restarted.last_error)
        return
    var restart_reply: String = restarted.send_user_message("Do you remember my cat?")
    _assert_host_ok(restarted)
    if failed:
        return
    if not restart_reply.contains("Irzha"):
        _fail("HTTP bridge restart reply did not use persisted memory: %s" % restart_reply)
        return

    print("CHIBIGOCHI LLM BRIDGE PASSED")
    print("host=chibigochi-llm-bridge")
    print("archive_id=%s" % archive.get("archive_id", ""))
    print("memory_units=%s" % archive.get("memory_units", []).size())
    print("core_facts=%s" % restarted.context_package("What do you know about me?").get("core_facts", []).size())
    quit(0)

func _arg_value(name: String) -> String:
    var args := OS.get_cmdline_user_args()
    if args.is_empty():
        args = OS.get_cmdline_args()
    for index in range(args.size()):
        if args[index] == name and index + 1 < args.size():
            return args[index + 1]
    return ""

func _assert_archive(archive: Dictionary) -> void:
    if archive.get("status") != "complete":
        _fail("HTTP bridge sleep did not complete: %s" % archive.get("status"))
    var units: Array = archive.get("memory_units", [])
    if units.size() < 3:
        _fail("HTTP bridge expected at least three memory units, got %s" % units.size())

func _assert_core_texts(texts: String) -> void:
    if not texts.contains("Mykyta"):
        _fail("HTTP bridge Core facts missed player name:\n%s" % texts)
    if not texts.contains("Irzha"):
        _fail("HTTP bridge Core facts missed cat Irzha:\n%s" % texts)
    if not _contains_any(texts, ["space", "космос"]):
        _fail("HTTP bridge Core facts missed space interest:\n%s" % texts)

func _assert_host_ok(host: Node) -> void:
    if host.last_error != "":
        _fail(host.last_error)

func _contains_any(text: String, needles: Array) -> bool:
    for needle in needles:
        if text.contains(str(needle)):
            return true
    return false

func _fail(message: String) -> void:
    if failed:
        return
    failed = true
    push_error(message)
    printerr("CHIBIGOCHI LLM BRIDGE FAILED: %s" % message)
    quit(1)
