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
