# Memory Engine — Python Adapter

PyO3 bindings for the Memory Engine Rust core. Built with `maturin`.

## Status

v0.1 thin wrapper. The boundary is JSON strings in, JSON strings out. The
Python caller is responsible for executing any `PendingTask` returned by
the engine, using whatever LLM provider it chooses, and submitting the
result back. The engine itself never touches the network.

## Build and develop

From `crates/python_adapter/`, inside a Python virtual environment:

```bash
pip install maturin pytest
maturin develop
pytest tests/
```

`maturin develop` builds the Rust extension and installs it into the
active virtual environment under the import name `memory_engine`.

## Public API

```python
import json
import memory_engine

engine = memory_engine.MemoryEngine("memory", host_id="telegram_bot")

stored = json.loads(engine.ingest(json.dumps(event_dict)))
session = json.loads(engine.read_session(session_id))
sleep_result = json.loads(engine.sleep(session_id))
updated = json.loads(engine.resume_sleep_compression(task_id, json.dumps(llm_result)))
recall_result = json.loads(engine.recall(json.dumps(query_dict)))
pending = json.loads(engine.pending_tasks())
```

All payloads follow the JSON contracts in `docs/contracts.md` at the
repository root.

## What lives here vs the host

The adapter contains no provider, model, or API key. It contains no prompt
text. See `examples/host_llm_config.py` for the shape a host project (a
Telegram bot, a Godot game, anything else) is expected to maintain on its
own side to map `role_hint` to a real `provider + model + api_key` and to
execute pending tasks.
