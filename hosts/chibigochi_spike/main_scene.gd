extends Control

const ChibigochiMemoryHostScript = preload("res://chibigochi_memory_host.gd")
const AsyncHttpLlmBridgeScript = preload("res://chibigochi_async_http_llm_bridge.gd")

@export var auto_open := true
@export var llm_endpoint := ""
@export var llm_timeout_seconds := 60.0

var memory_host: Node
var chat_log: RichTextLabel
var memory_snapshot: TextEdit
var input_line: LineEdit
var send_button: Button
var sleep_button: Button
var status_label: Label
var conversation_lines: Array[String] = []
var last_error := ""
var last_reply := ""
var ui_state := "starting"
var ui_built := false
var configured_llm_bridge: Variant

func _ready() -> void:
    _ensure_ui()
    if auto_open:
        open_memory(OS.get_user_data_dir().path_join("memory"))

func open_memory(memory_dir: String) -> bool:
    _ensure_ui()
    last_error = ""
    memory_host = ChibigochiMemoryHostScript.new()
    _ensure_configured_bridge()
    if configured_llm_bridge != null:
        memory_host.set_llm_bridge(configured_llm_bridge)
    if not memory_host.open(memory_dir):
        _set_error(memory_host.last_error)
        return false
    _set_ui_state("idle", "Memory ready")
    memory_snapshot.text = ""
    return true

func send_text(text: String) -> String:
    var cleaned := text.strip_edges()
    if cleaned == "":
        return ""
    if memory_host == null:
        _set_error("memory host is not open")
        return ""

    _append_line("Player", cleaned)
    var reply: String = memory_host.send_user_message(cleaned)
    if memory_host.last_error != "":
        _set_error(memory_host.last_error)
        return ""
    last_reply = reply
    _append_line("Chibigochi", reply)
    _set_ui_state("idle", "Turn saved")
    refresh_memory_view(cleaned)
    return reply

func send_text_async(text: String) -> String:
    var cleaned := text.strip_edges()
    if cleaned == "":
        return ""
    if memory_host == null:
        _set_error("memory host is not open")
        return ""

    _set_ui_state("sending", "Sending")
    _append_line("Player", cleaned)
    var reply: String = await memory_host.send_user_message_async(cleaned)
    if memory_host.last_error != "":
        _set_error(memory_host.last_error)
        return ""
    last_reply = reply
    _append_line("Chibigochi", reply)
    _set_ui_state("idle", "Turn saved")
    refresh_memory_view(cleaned)
    return reply

func run_sleep_now() -> Dictionary:
    if memory_host == null:
        _set_error("memory host is not open")
        return {}
    _set_ui_state("sleeping", "Sleeping")
    var outcome: Dictionary = memory_host.run_sleep()
    if memory_host.last_error != "":
        _set_error(memory_host.last_error)
        return {}
    var archive: Dictionary = outcome.get("archive_entry", {})
    var unit_count: int = archive.get("memory_units", []).size()
    _set_ui_state("idle", "Sleep complete: %s memory units" % unit_count)
    refresh_memory_view("What do you know about me?")
    return outcome

func run_sleep_now_async() -> Dictionary:
    if memory_host == null:
        _set_error("memory host is not open")
        return {}
    _set_ui_state("sleeping", "Sleeping")
    var outcome: Dictionary = await memory_host.run_sleep_async()
    if memory_host.last_error != "":
        _set_error(memory_host.last_error)
        return {}
    var archive: Dictionary = outcome.get("archive_entry", {})
    var unit_count: int = archive.get("memory_units", []).size()
    _set_ui_state("idle", "Sleep complete: %s memory units" % unit_count)
    refresh_memory_view("What do you know about me?")
    return outcome

func refresh_memory_view(current_text: String) -> void:
    if memory_host == null or memory_snapshot == null:
        return
    memory_snapshot.text = memory_host.memory_view(current_text)

func conversation_text() -> String:
    return "\n".join(conversation_lines)

func memory_view(current_text: String) -> String:
    if memory_host == null:
        return ""
    return memory_host.memory_view(current_text)

func context_package(current_text: String) -> Dictionary:
    if memory_host == null:
        return {}
    return memory_host.context_package(current_text)

func set_llm_bridge(bridge: Variant) -> void:
    configured_llm_bridge = bridge
    if configured_llm_bridge is Node and configured_llm_bridge.get_parent() == null:
        add_child(configured_llm_bridge)
    if memory_host != null:
        memory_host.set_llm_bridge(bridge)

func configure_http_bridge(endpoint: String, timeout: float = 60.0) -> void:
    llm_endpoint = endpoint
    llm_timeout_seconds = timeout
    configured_llm_bridge = null
    _ensure_configured_bridge()
    if memory_host != null and configured_llm_bridge != null:
        memory_host.set_llm_bridge(configured_llm_bridge)

func core_fact_texts(current_text: String) -> String:
    if memory_host == null:
        return ""
    return memory_host.core_fact_texts(current_text)

func _build_ui() -> void:
    set_anchors_preset(Control.PRESET_FULL_RECT)

    var root := HBoxContainer.new()
    root.set_anchors_preset(Control.PRESET_FULL_RECT)
    root.add_theme_constant_override("separation", 12)
    root.offset_left = 12
    root.offset_top = 12
    root.offset_right = -12
    root.offset_bottom = -12
    add_child(root)

    var conversation_panel := VBoxContainer.new()
    conversation_panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
    conversation_panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
    conversation_panel.add_theme_constant_override("separation", 8)
    root.add_child(conversation_panel)

    var header := HBoxContainer.new()
    header.add_theme_constant_override("separation", 10)
    conversation_panel.add_child(header)

    var portrait := ColorRect.new()
    portrait.custom_minimum_size = Vector2(48, 48)
    portrait.color = Color(0.18, 0.43, 0.53)
    header.add_child(portrait)

    var title_box := VBoxContainer.new()
    title_box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
    header.add_child(title_box)

    var title := Label.new()
    title.text = "Chibigochi"
    title.add_theme_font_size_override("font_size", 22)
    title_box.add_child(title)

    status_label = Label.new()
    status_label.text = "Starting"
    title_box.add_child(status_label)

    chat_log = RichTextLabel.new()
    chat_log.fit_content = false
    chat_log.scroll_following = true
    chat_log.size_flags_vertical = Control.SIZE_EXPAND_FILL
    chat_log.size_flags_horizontal = Control.SIZE_EXPAND_FILL
    conversation_panel.add_child(chat_log)

    var input_bar := HBoxContainer.new()
    input_bar.add_theme_constant_override("separation", 8)
    conversation_panel.add_child(input_bar)

    input_line = LineEdit.new()
    input_line.placeholder_text = "Message"
    input_line.size_flags_horizontal = Control.SIZE_EXPAND_FILL
    input_line.text_submitted.connect(_on_text_submitted)
    input_bar.add_child(input_line)

    send_button = Button.new()
    send_button.text = "Send"
    send_button.pressed.connect(_on_send_pressed)
    input_bar.add_child(send_button)

    sleep_button = Button.new()
    sleep_button.text = "Sleep"
    sleep_button.pressed.connect(_on_sleep_pressed)
    input_bar.add_child(sleep_button)

    memory_snapshot = TextEdit.new()
    memory_snapshot.editable = false
    memory_snapshot.wrap_mode = TextEdit.LINE_WRAPPING_BOUNDARY
    memory_snapshot.custom_minimum_size = Vector2(380, 0)
    memory_snapshot.size_flags_vertical = Control.SIZE_EXPAND_FILL
    root.add_child(memory_snapshot)

func _ensure_ui() -> void:
    if ui_built:
        return
    ui_built = true
    _build_ui()

func _on_send_pressed() -> void:
    var text := input_line.text
    input_line.clear()
    await send_text_async(text)

func _on_text_submitted(text: String) -> void:
    input_line.clear()
    await send_text_async(text)

func _on_sleep_pressed() -> void:
    await run_sleep_now_async()

func _append_line(speaker: String, text: String) -> void:
    var line := "%s: %s" % [speaker, text]
    conversation_lines.append(line)
    chat_log.append_text(line + "\n")

func _set_status(text: String) -> void:
    if status_label != null:
        status_label.text = text

func _set_ui_state(state: String, status_text: String) -> void:
    ui_state = state
    _set_status(status_text)
    var busy := state == "sending" or state == "sleeping"
    if input_line != null:
        input_line.editable = not busy
    if send_button != null:
        send_button.disabled = busy
    if sleep_button != null:
        sleep_button.disabled = busy

func _set_error(message: String) -> void:
    last_error = message
    _set_ui_state("error", "Error")

func _ensure_configured_bridge() -> void:
    if configured_llm_bridge != null:
        return
    if llm_endpoint == "":
        return
    var bridge: Node = AsyncHttpLlmBridgeScript.new()
    bridge.configure(llm_endpoint, "", llm_timeout_seconds)
    add_child(bridge)
    configured_llm_bridge = bridge
