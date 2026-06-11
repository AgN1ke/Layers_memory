extends SceneTree

const MainScene = preload("res://main_scene.tscn")

var failed := false

func _init() -> void:
    var runtime_dir := _arg_value("--runtime-dir")
    if runtime_dir == "":
        _fail("missing --runtime-dir")
        return

    var memory_dir := runtime_dir.path_join("memory")
    var scene: Variant = MainScene.instantiate()
    scene.auto_open = false
    root.add_child(scene)
    if not scene.open_memory(memory_dir):
        _fail(scene.last_error)
        return

    var first_reply: String = scene.send_text("My name is Mykyta.")
    _assert_scene_ok(scene)
    if failed:
        return
    if not first_reply.contains("Mykyta"):
        _fail("UI first reply did not acknowledge current text")
        return

    scene.send_text("I have a cat named Irzha.")
    _assert_scene_ok(scene)
    if failed:
        return
    scene.send_text("I like space and want Chibigochi to remember me.")
    _assert_scene_ok(scene)
    if failed:
        return

    var outcome: Dictionary = scene.run_sleep_now()
    _assert_scene_ok(scene)
    if failed:
        return
    var archive: Dictionary = outcome.get("archive_entry", {})
    _assert_archive(archive)
    if failed:
        return
    _assert_core_texts(scene.core_fact_texts("What do you know about me?"))
    if failed:
        return
    if not scene.conversation_text().contains("Chibigochi:"):
        _fail("UI conversation log did not record assistant reply")
        return

    var restarted: Variant = MainScene.instantiate()
    restarted.auto_open = false
    root.add_child(restarted)
    if not restarted.open_memory(memory_dir):
        _fail(restarted.last_error)
        return
    var restart_reply: String = restarted.send_text("Do you remember my cat?")
    _assert_scene_ok(restarted)
    if failed:
        return
    if not restart_reply.contains("Irzha"):
        _fail("UI restart reply did not use persisted memory: %s" % restart_reply)
        return
    var view: String = restarted.memory_snapshot.text
    if not view.contains("<core_memory>") or not view.contains("Mykyta") or not view.contains("Irzha"):
        _fail("UI memory snapshot missed persisted Core facts:\n%s" % view)
        return

    print("CHIBIGOCHI UI PASSED")
    print("host=chibigochi-ui")
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
        _fail("UI sleep did not complete: %s" % archive.get("status"))
    var units: Array = archive.get("memory_units", [])
    if units.size() < 3:
        _fail("UI expected at least three memory units, got %s" % units.size())

func _assert_core_texts(texts: String) -> void:
    if not texts.contains("Mykyta"):
        _fail("UI Core facts missed player name:\n%s" % texts)
    if not texts.contains("Irzha"):
        _fail("UI Core facts missed cat Irzha:\n%s" % texts)
    if not texts.contains("space"):
        _fail("UI Core facts missed space interest:\n%s" % texts)

func _assert_scene_ok(scene: Node) -> void:
    if scene.last_error != "":
        _fail(scene.last_error)

func _fail(message: String) -> void:
    if failed:
        return
    failed = true
    push_error(message)
    printerr("CHIBIGOCHI UI FAILED: %s" % message)
    quit(1)
