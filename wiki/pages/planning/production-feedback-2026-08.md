# Production Integration Feedback — 2026-08

## Related pages

- [Architecture](../foundation/architecture.md) - ownership boundaries and the
  three-layer memory flow.
- [Contracts](../foundation/contracts.md) - exact sleep, context and telemetry
  shapes.
- [LLM integration resources](../integration/llm-integration-resources.md) -
  provider roles, keys and host execution requirements.
- [Information Flow Integrity](information-flow-integrity.md) - canonical path
  from a host event to prompt-facing memory.
- [Roadmap](roadmap.md) - current implementation state.
- [Smartest v2 Production Validation](smartest-v2-production-validation-2026-08.md)
  - operator-facing measurement and reporting specification for large chats.

## Purpose

This page records the response to production observations from a commercial
Ukrainian Telegram bot on 22-23 August 2026. The report covered twelve Sleep
runs, persisted run/session/archive records and provider usage ledgers. Its
measurements belong to that product workload. Memory Engine uses them as design
evidence and does not publish them as a library-wide performance benchmark.

The implementation keeps the existing default behavior for older callers and
adds explicit controls and structured outputs for integrations that need a
different cost or chat-API profile.

## Resolution

### Shared and compact Sleep source

Every Sleep workflow exposes one `sleep_prompt_context.v1` through its
`LlmBatch`. It contains batch-level session, archive and scope fields once,
followed by compact event lines with event ID, time, type, source, speaker,
weight, useful markers and `memory_text`. FileStorage can persist that compact
view with the run. Storage v2 hydrates it from the immutable lossless workflow
input at the worker boundary instead of persisting a second copy of private
text.

`LlmBatch.shared_context` carries this source separately from pass-specific
`LlmRequest.prompt_inputs`. A provider executor places `shared_context.content`
first, then the prompt instruction, then the small request inputs. All Sleep
passes therefore start with the same exact prefix. The reference Gemini hosts
follow this order and expose the context ID and estimated size in telemetry.

This resolves findings 1, 3 and 4. Provider-specific explicit cache creation is
still a host capability; the core now provides the stable prefix needed for
implicit or explicit prefix caching.

### Selectable work and bounded cost

`begin_sleep_run_with_options(session_id, options)` accepts:

```json
{
  "enabled_tracks": ["memory_unit", "personal_signal", "consolidator"],
  "max_provider_calls": 5,
  "max_pass_attempts": 2
}
```

An omitted `enabled_tracks` field preserves all six historical tracks. A host
can choose an economy profile for its product. `max_provider_calls` covers
initial attempts, retries and consolidation. Reaching the limit applies
deterministic fallbacks and still produces a Complete archive. A host that must
stop for its own external budget can call `finish_sleep_run_with_available`;
already accepted results remain in the final archive.

Each pass has a concrete consumer:

| Track | Current consumers |
|---|---|
| `memory_unit` | canonical compact memory, ordinary recall, vectors, fidelity and reflection |
| `emotional` | archive audit, consolidator input, forgetting protection and repair fallback |
| `topic_thread` | archive audit, consolidator input, reflection relations and repair fallback |
| `personal_signal` | Archive-to-Core bridge, attribution review routing and Core-path fidelity |
| `relational` | archive audit and consolidator input |
| `consolidator` | prompt/recall gist, lexical recall narrative and repair evidence fallback |

This resolves findings 2 and 11. Track selection changes memory quality and is
a product policy decision made explicitly at run start.

### Structured chat context

`CoreContextEvent` now carries the original event `payload`. One call to
`core_context_package` therefore provides both memory text and host-owned
message/tool metadata.

`render_memory_prompt_plan(package, current_user_message)` returns
`memory_prompt_plan.v1`: state, Core, long memory and optional associative
memory as separate core-rendered sections, plus chronological typed dialogue
events and the current message. A role-aware Chat API can construct native
turns directly. `render_memory_view` remains available for text-only callers.

This resolves findings 5 and 6 without moving memory projection policy into a
host or requiring regex parsing.

### Empty sessions

Reading a session whose directory has never existed returns a virtual empty
active session. `core_context_package` works before the first ingest and can use
an explicitly supplied Core scope. The virtual record is not persisted.

A materialized session directory with missing metadata remains a storage
error. This distinguishes a new conversation from damaged data. It resolves
finding 7 and removes the need for synthetic `session_started` events.

### Core promotion visibility

`finish_sleep_run` already executes the Archive-to-Core personal-signal bridge.
The production symptom in finding 8 therefore indicated absent or rejected
signals rather than a missing bridge call.

`SleepOutcome.core_summary` now reports created, updated, skipped and routed-to-
review counts. Skips are grouped by reason:

- `invalid_signal`;
- `below_confidence_threshold`;
- `no_user_authored_source`;
- `requires_attribution_review`;
- `near_duplicate`.

The reference Telegram host logs these reasons. An empty Core is now
diagnosable from the run outcome.

### Reflection visibility and tuning

Reflection continues to reason only over fidelity-valid active MemoryUnits.
`ReflectionTriggerStatus` now separates:

- all active units;
- valid units;
- unchecked units;
- units excluded by fidelity;
- eligible unreflected units;
- accumulated and remaining salience.

The PyO3 constructor exposes `fidelity_auto_validate_weight_threshold`. A
product may validate a wider share of units before reflection by lowering that
threshold and accepting the added model cost. This resolves the ambiguity in
finding 9 while retaining the fidelity gate.

### Recovered provider failures

Each Sleep request retains all attempt errors. `SleepOutcome.pass_reports`
shows attempts, final completion, errors and whether a later retry recovered
the pass. `failed_passes` remains the list of terminal pass failures;
`skipped_passes` records disabled or budget-fallback work.

This resolves finding 10 and makes transient provider filtering visible even
when the final archive succeeds.

## Compatibility

- `begin_sleep_run(session_id)` still runs the full historical profile.
- Existing persisted Sleep runs deserialize through additive defaults; legacy
  inline event inputs are migrated to a shared prompt context when resumed.
- `render_memory_view` remains stable for text-only integrations.
- Stored events, PendingTasks and immutable Storage v2 workflow input remain
  lossless. Compact formatting applies only to provider-facing Sleep source.
- JSON responses from analytical passes remain unchanged.

## Reference host controls

The Telegram reference host demonstrates the public options through:

- `MEMORY_BOT_SLEEP_TRACKS` - comma-separated track names;
- `MEMORY_BOT_SLEEP_MAX_PROVIDER_CALLS` - whole-run provider-call ceiling;
- `MEMORY_BOT_SLEEP_MAX_PASS_ATTEMPTS` - retry ceiling per pass;
- `MEMORY_BOT_FIDELITY_WEIGHT_THRESHOLD` - automatic fidelity routing
  threshold.

Keys, providers and concrete model names remain owned by the integrating
program, as described in [LLM integration resources](../integration/llm-integration-resources.md).

## Acceptance anchors

- `crates/memory_engine/tests/production_feedback.rs` covers compact shared
  source, pass selection, budget completion, early host completion, retry
  reporting, empty sessions, payload-preserving prompt plans and Core skip
  reasons.
- `crates/memory_engine/tests/engine_reflection.rs` covers the fidelity count
  breakdown.
- `crates/memory_engine/tests/engine_erasure.rs` confirms that deleted sessions
  read as empty while durable scope memory follows the erasure contract.
- Full workspace, adapter and host-conformance gates remain required before
  merge.
