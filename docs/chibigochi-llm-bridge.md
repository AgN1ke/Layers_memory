# Chibigochi LLM Bridge

This document defines the current production-shaped LLM boundary for the
Chibigochi Godot spike.

The bridge is intentionally thin. Godot does not decide what should become
memory, what should be promoted to Core, or how forgetting/fidelity work. Godot
only sends prompt work to an external executor and returns text to the Rust
core.

## Implementations

- `hosts/chibigochi_spike/chibigochi_fake_llm_bridge.gd`
  - deterministic local bridge for headless tests;
  - no network;
  - returns stable fake chat/sleep/fidelity outputs.
- `hosts/chibigochi_spike/chibigochi_http_llm_bridge.gd`
  - HTTP bridge for a local or remote LLM executor;
  - uses Godot `HTTPClient`;
  - sends JSON requests and expects JSON response objects.
- `hosts/chibigochi_spike/chibigochi_gemini_proxy.py`
  - local Gemini-backed HTTP executor for development;
  - reads the existing gitignored secrets cache from
    `hosts/telegram_gemini_bot/runtime/state/secrets.local.json`;
  - owns provider keys, model selection, prompt loading, Gemini network I/O,
    and token telemetry;
  - can run as a long-lived local proxy or start a temporary proxy and execute
    the Godot bridge conformance scenario.

## Operations

### `chat_reply`

Request:

```json
{
  "operation": "chat_reply",
  "input_text": "Do you remember my cat?",
  "memory_view": "<memory_context>...</memory_context>"
}
```

Response:

```json
{
  "text": "I remember Irzha..."
}
```

### `memory_request`

This executes one core-owned `LlmRequest` from the sleep driver.

Request:

```json
{
  "operation": "memory_request",
  "run": { "...": "current SleepRun state" },
  "request": {
    "request_id": "...",
    "task_id": "...",
    "role_hint": "balanced",
    "prompt_id": "memory_unit_pass",
    "prompt_inputs": {}
  },
  "role_hint": "balanced",
  "prompt_id": "memory_unit_pass",
  "prompt_inputs": {}
}
```

Response:

```json
{
  "status": "ok",
  "request_id": "...",
  "text": "{...model output text...}"
}
```

`text` is the model output for the requested pass. The bridge may omit
`status` and `request_id`; the Godot bridge fills defaults before submitting to
the engine.

### `memory_fidelity_pass`

This executes the post-sleep fidelity validator request.

Request:

```json
{
  "operation": "memory_fidelity_pass",
  "request": {
    "request_id": "...",
    "task_id": "...",
    "role_hint": "reasoning",
    "prompt_id": "memory_fidelity_pass",
    "prompt_inputs": {}
  },
  "role_hint": "reasoning",
  "prompt_id": "memory_fidelity_pass",
  "prompt_inputs": {}
}
```

Response is the same shape as `memory_request`.

## Conformance

The bridge path is tested by a local HTTP proxy:

```powershell
crates\python_adapter\.venv\Scripts\python.exe tests\host_conformance\host_conformance.py --host chibigochi-llm-bridge
```

That scenario proves:

- Godot uses `chibigochi_http_llm_bridge.gd`, not in-process fake responses.
- Chat reply, sleep passes, and fidelity pass all cross the HTTP bridge.
- The Rust core still owns sleep orchestration, Core promotion, fidelity state,
  and persistence.
- Restart recall works from persisted Core/context memory.

## Local Gemini Proxy

The deterministic conformance proxy above proves the HTTP boundary without
calling a real model. To run the same Godot bridge against cached Gemini
credentials, use:

```powershell
crates\python_adapter\.venv\Scripts\python.exe hosts\chibigochi_spike\chibigochi_gemini_proxy.py --run-conformance --validate-key
```

The proxy does not print API keys. It logs only a fingerprint and token telemetry
under the gitignored Chibigochi runtime directory:

```text
hosts/chibigochi_spike/runtime/logs/chibigochi_gemini_proxy.log
hosts/chibigochi_spike/runtime/logs/chibigochi_gemini_proxy_token_usage.jsonl
```

To run it as a reusable local endpoint for the Godot scene:

```powershell
crates\python_adapter\.venv\Scripts\python.exe hosts\chibigochi_spike\chibigochi_gemini_proxy.py --host 127.0.0.1 --port 8765 --validate-key
```

Then configure `chibigochi_http_llm_bridge.gd` with:

```text
http://127.0.0.1:8765/llm
```

The proxy uses `hosts/chibigochi_spike/chibigochi_chat_system.md` for chat
replies and the shared `prompts/<prompt_id>.md` files for core-owned
sleep/fidelity tasks.
