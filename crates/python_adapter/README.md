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

`memory_engine.build_info()` returns `version`, the full Git `commit`, and
`dirty` (whether tracked files had staged or unstaged changes at build time).
Unknown values are `None`. This function is available without creating a
`MemoryEngine`. For builds from `git archive`, the builder can supply
`MEMORY_ENGINE_BUILD_COMMIT` and `MEMORY_ENGINE_BUILD_DIRTY` before invoking
Maturin. `dirty` accepts `true` or `false`.

## Public API

```python
import json
import memory_engine

engine = memory_engine.MemoryEngine("memory", host_id="telegram_bot")

ingest_result = json.loads(engine.ingest(json.dumps(event_dict)))
stored = ingest_result["stored_event"]
session = json.loads(engine.read_session(session_id))
context = json.loads(engine.core_context_package(json.dumps(context_request_dict)))
sleep_result = json.loads(engine.sleep(session_id))
updated = json.loads(engine.resume_sleep_compression(task_id, json.dumps(llm_result)))
recall_result = json.loads(engine.recall(json.dumps(query_dict)))
pending = json.loads(engine.pending_tasks())
recovered_operations = engine.recover_pending_memory_operations()
```

`pending_tasks()` already performs reflection-operation recovery before it
returns host work. The explicit recovery method is available for startup
diagnostics and returns only the number of `started` journal operations it
completed; it does not call a provider.

Stage 2 recall tuning is optional and remains Rust-core policy:

```python
engine = memory_engine.MemoryEngine(
    "memory",
    host_id="telegram_bot",
    stage2_candidate_limit=40,
    stage2_vector_weight=0.5,
    associative_memory_min_similarity=0.47,
    reflection_salience_threshold=2.5,
    reflection_critical_unit_weight=0.95,
    reflection_max_interval_days=7.0,
    reflection_scheduled_min_units=2,
    reflection_max_trigger_units=40,
    reflection_max_related_units=20,
    reflection_min_related_score=2,
    reflection_max_existing_candidates=20,
)
```

The adapter only forwards these values into `EngineOptions`. It does not
compute embeddings or rank memories itself. When Stage 2 applies, the returned
JSON keeps the parent archive `id` but includes `matched_memory_unit_id` and a
unit-focused `gist`/`compact_memory`; `debug.candidates` carries the same ID as
text-free semantic provenance. Older Stage 1 payloads simply omit the optional
field.

`associative_memory_min_similarity` controls the separate, per-request
current-topic memory gate. Its default `0.47` is calibrated for the bundled
multilingual MiniLM model; weak matches below it are omitted instead of being
forced into the prompt.

Sleep is an explicit host decision in v0.1. A host calls `engine.sleep(session_id)`
manually, from token-budget pressure, or from an idle schedule, then executes the
returned tasks through its own LLM provider.

For the multi-pass driver, `begin_sleep_run(session_id)` returns the current
open run for that session, including after restart. The run keeps its original
options and saved progress. Continue from the returned state. To abandon it,
call `cancel_sleep_run(sleep_task_id)` before starting again. Share one engine
instance across workers that write to the same FileStorage directory.

Autonomous reflection is also an explicit host execution opportunity while
eligibility remains Rust-core policy:

```python
state = json.loads(engine.bind_reflection_session(session_id, core_scope))
status = json.loads(
    engine.reflection_status(session_id, core_scope, "after_sleep")
)
automatic = json.loads(
    engine.begin_reflection_if_due(session_id, core_scope, "after_sleep")
)
if automatic.get("reflection"):
    request = automatic["reflection"]["request"]
    # Execute request through the host's reasoning-model mapping, then call
    # submit_reflection_response as with manual reflection.
```

The method may omit `reflection`; that is the normal core abstention path.
`scheduled_maintenance` is the second accepted opportunity value. The
adapter does not sum memory weights or apply a second relevance rule. Calling
`begin_reflection_if_due` with the same explicit `core_scope` across several
sessions durably binds those sessions to one reflection personality scope.
`bind_reflection_session` performs the same membership update without trigger
evaluation or an LLM task and is suitable for session lifecycle/backfill.
Returned task inputs distinguish new `trigger` units from bounded
`related_history` and include bounded draft/ready candidates of that scope.
New tasks expect `reflection_actions_result.v1`. Hosts normally pass the LLM
response through `submit_reflection_response`; deterministic executors may call
`resume_reflection_actions`. Typed outcomes create a candidate, reinforce one
with new evidence, propose a reviewable Core challenge, or return structured
no-insight. The legacy `resume_reflection_analysis` method remains available
for saved `reflection_result.v1` work.

The v0.6 hypothesis graph storage surface is administrative and remains a
thin JSON boundary:

```python
migration = json.loads(engine.migrate_hypothesis_graph())
graph = json.loads(engine.export_hypothesis_graph(json.dumps({
    "schema_version": "hypothesis_graph_export_request.v1",
    "core_scope": "owner_scope",
})))
```

Migration is journaled and restart-safe. Export is exact-scope; `null` means
only the isolated unscoped namespace. Neither method invokes an LLM or adds the
graph to the chat prompt.

The response shape is stable for empty storage. Migration always includes
`unresolved_memory_unit_ids` and `warnings`; export always includes `nodes`,
`evidence_links`, `relations`, and `warnings`. Empty collections are `[]`, not
omitted keys.

All payloads follow the JSON contracts in `wiki/pages/foundation/contracts.md` at the
repository root.

## What lives here vs the host

The adapter contains no provider, model, or API key. It contains no prompt
text. See `examples/host_llm_config.py` for the shape a host project (a
Telegram bot, a Godot game, anything else) is expected to maintain on its
own side to map `role_hint` to a real `provider + model + api_key` and to
execute pending tasks.
