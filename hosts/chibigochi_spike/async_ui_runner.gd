extends SceneTree

const MainScene = preload("res://main_scene.tscn")

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

    var memory_dir := runtime_dir.path_join("memory")
    var scene: Variant = MainScene.instantiate()
    scene.auto_open = false
    root.add_child(scene)
    await process_frame
    scene.configure_http_bridge(endpoint)
    if not scene.open_memory(memory_dir):
        _fail(scene.last_error)
        return

    var first_reply: String = await scene.send_text_async("My name is Mykyta.")
    _assert_scene_ok(scene)
    if failed:
        return
    _assert_contains_any(first_reply, ["Mykyta", "Микит"], "async UI first reply missed player name: %s" % first_reply)
    if failed:
        return
    _assert_state(scene, "idle")

    await scene.send_text_async("I have a cat named Irzha.")
    _assert_scene_ok(scene)
    if failed:
        return
    await scene.send_text_async("I like space and want Chibigochi to remember me.")
    _assert_scene_ok(scene)
    if failed:
        return

    var outcome: Dictionary = await scene.run_sleep_now_async()
    _assert_scene_ok(scene)
    if failed:
        return
    _assert_state(scene, "idle")
    var archive: Dictionary = outcome.get("archive_entry", {})
    _assert_archive(archive)
    if failed:
        return
    _assert_core_texts(scene.core_fact_texts("What do you know about me?"))
    if failed:
        return

    var restarted: Variant = MainScene.instantiate()
    restarted.auto_open = false
    root.add_child(restarted)
    await process_frame
    restarted.configure_http_bridge(endpoint)
    if not restarted.open_memory(memory_dir):
        _fail(restarted.last_error)
        return
    var restart_reply: String = await restarted.send_text_async("Do you remember my cat?")
    _assert_scene_ok(restarted)
    if failed:
        return
    _assert_contains_any(restart_reply, ["Irzha", "Ірж"], "async UI restart reply did not use persisted memory: %s" % restart_reply)
    if failed:
        return
    var view: String = restarted.memory_snapshot.text
    _assert_contains_any(view, ["Mykyta", "Микит"], "async UI memory snapshot missed player name:\n%s" % view)
    _assert_contains_any(view, ["Irzha", "Ірж"], "async UI memory snapshot missed cat:\n%s" % view)
    if failed:
        return

    print("CHIBIGOCHI ASYNC UI PASSED")
    print("host=chibigochi-product-loop")
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
        _fail("async UI sleep did not complete: %s" % archive.get("status"))
    var units: Array = archive.get("memory_units", [])
    if units.size() < 3:
        _fail("async UI expected at least three memory units, got %s" % units.size())

func _assert_core_texts(texts: String) -> void:
    _assert_contains_any(texts, ["Mykyta", "Микит"], "async UI Core facts missed player name:\n%s" % texts)
    _assert_contains_any(texts, ["Irzha", "Ірж"], "async UI Core facts missed cat Irzha:\n%s" % texts)
    _assert_contains_any(texts, ["space", "космос"], "async UI Core facts missed space interest:\n%s" % texts)

func _assert_scene_ok(scene: Node) -> void:
    if scene.last_error != "":
        _fail(scene.last_error)

func _assert_state(scene: Node, expected: String) -> void:
    if scene.ui_state != expected:
        _fail("async UI expected state %s, got %s" % [expected, scene.ui_state])

func _contains_any(text: String, needles: Array) -> bool:
    for needle in needles:
        if text.contains(str(needle)):
            return true
    return false

func _assert_contains_any(text: String, needles: Array, message: String) -> void:
    if not _contains_any(text, needles):
        _fail(message)

func _fail(message: String) -> void:
    if failed:
        return
    failed = true
    push_error(message)
    printerr("CHIBIGOCHI ASYNC UI FAILED: %s" % message)
    quit(1)
