extends SceneTree

const ChibigochiMemoryHostScript = preload("res://chibigochi_memory_host.gd")

var failed := false

func _init() -> void:
    var runtime_dir := _arg_value("--runtime-dir")
    if runtime_dir == "":
        _fail("missing --runtime-dir")
        return

    var memory_dir := runtime_dir.path_join("memory")
    var host: Variant = ChibigochiMemoryHostScript.new()
    if not host.open(memory_dir):
        _fail(host.last_error)
        return

    var reply: String = host.send_user_message("My name is Mykyta.")
    _assert_no_error(host)
    if failed:
        return
    if not reply.contains("Mykyta"):
        _fail("first reply should acknowledge current user text")
        return

    host.send_user_message("I have a cat named Irzha.")
    _assert_no_error(host)
    if failed:
        return
    host.send_user_message("I like space and want this Godot character to remember me after restart.")
    _assert_no_error(host)
    if failed:
        return

    var outcome: Dictionary = host.run_sleep()
    _assert_no_error(host)
    if failed:
        return
    var archive: Dictionary = outcome.get("archive_entry", {})
    _assert_sleep_archive(archive)
    if failed:
        return
    var core_before: String = host.core_fact_texts("What do you know about me?")
    _assert_core_texts(core_before)
    if failed:
        return

    var restarted: Variant = ChibigochiMemoryHostScript.new()
    if not restarted.open(memory_dir):
        _fail(restarted.last_error)
        return
    var restart_reply: String = restarted.send_user_message("Do you remember my cat?")
    _assert_no_error(restarted)
    if failed:
        return
    if not restart_reply.contains("Irzha"):
        _fail("restart reply did not use persisted Core/context memory: %s" % restart_reply)
        return
    var view: String = restarted.memory_view("What is my name and what is my cat called?")
    if not view.contains("<core_memory>") or not view.contains("Mykyta") or not view.contains("Irzha"):
        _fail("restart memory view missed persisted Core facts:\n%s" % view)
        return

    print("CHIBIGOCHI SPIKE PASSED")
    print("host=chibigochi-spike")
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

func _assert_sleep_archive(archive: Dictionary) -> void:
    if archive.get("status") != "complete":
        _fail("sleep did not complete: %s" % archive.get("status"))
    var units: Array = archive.get("memory_units", [])
    if units.size() < 3:
        _fail("expected at least three memory units, got %s" % units.size())

func _assert_core_texts(texts: String) -> void:
    if not texts.contains("Mykyta"):
        _fail("Core facts missed player name:\n%s" % texts)
    if not texts.contains("Irzha"):
        _fail("Core facts missed cat Irzha:\n%s" % texts)
    if not texts.contains("space"):
        _fail("Core facts missed space interest:\n%s" % texts)

func _assert_no_error(host: Node) -> void:
    if host.last_error != "":
        _fail(host.last_error)

func _fail(message: String) -> void:
    if failed:
        return
    failed = true
    push_error(message)
    printerr("CHIBIGOCHI SPIKE FAILED: %s" % message)
    quit(1)
