# Smartest v2 Production Validation Protocol — 2026-08

## Related pages

- [Production Integration Feedback](production-feedback-2026-08.md) - the
  eleven production findings and the implemented library response.
- [Smartest Phase 1 Review 2026-08-25](smartest-v2-phase1-review-2026-08-25.md) -
  independent review of the first returned package and the correction handoff.
- [Smartest Phase 1 Correction TZ](smartest-v2-phase1-correction-tz-2026-08-25.md) -
  standalone implementation and return instructions for the Smartest engineer.
- [Smartest Checkpoint A Review 2026-08-26](smartest-v2-checkpoint-a-review-2026-08-26.md) -
  qualification decision for the corrected preflight and cohort `c`.
- [Roadmap](roadmap.md) - active ordering from production validation into
  autonomous reflection and v0.6.
- [Autonomous Reflection v0.5](autonomous-reflection-v0.5.md) - trigger,
  evidence and live-acceptance rules calibrated by this run.
- [Reflective Belief System](reflective-belief-system.md) - the v0.6 reasoning
  phase that opens after this evidence gate.
- [LLM Integration Resources](../integration/llm-integration-resources.md) -
  provider execution, model roles and key ownership.
- [Contracts](../foundation/contracts.md) - exact Sleep, context, reflection
  and telemetry shapes.

## Status, audience and current phase

Status: **owner-approved production protocol; corrected checkpoint A reviewed
2026-08-26 as `QUALIFIED`; checkpoint B cohort is collecting**.

**Update 2026-11-15: the study is on hold.** Smartest reported open privacy and
provenance defects in its exporter and asked that no conclusions be drawn from
the August numbers. No checkpoint B package was recorded in this repository. A
new export and Phase 2 follow only after the exporter is fixed and a new
`QUALIFIED` is issued on exact bytes. Production acceptance stays a separate
open item. Whether development may meanwhile proceed on an isolated real-model
replay is owner decision D3 in the
[Integration Hardening Plan](integration-hardening-plan-2026-11-15.md).

This document is addressed to the Smartest v2 engineer or operator who can
change application logging and inspect provider usage, Memory Engine outputs,
runtime state and configuration. Chat participants continue using the product
normally. They do not need to perform scripted conversations or know that a
measurement is running.

The work is split into four phases:

1. add and qualify logging on one short run;
2. collect several days of passive production evidence;
3. analyze the full profile and run controlled replays where useful;
4. decide which integration settings are safe and whether v0.6 may begin.

Smartest should complete only Phase 1 first and return its small redacted
sample. Long measurement starts after the Memory Engine maintainer confirms
that the records are complete and internally consistent.

### Original assignment for 2026-08-25

For the first handoff, Smartest implements only **Phase 1** below: install the required build,
add five structured log streams, run one short sample and return the seven
redacted artifacts. Phases 2–4 explain why these fields are collected and what
will happen after qualification; they are not first-day implementation work.

### Returned sample review - 2026-08-25

Smartest returned eight usable cohort-B Sleep runs, provider/chat/reflection/
health ledgers and a privacy-safe export. The package proved that compact shared
Sleep input is active and exposed several real host-observability gaps. Phase 1
remains open because engine build provenance, physical provider attempts,
automatic fidelity coverage, chat budget fields, Core scope diagnostics and
vector-task state are incomplete.

The exact evidence, interim H1-H7 assessment and replacement-sample checklist
are recorded in the
[Phase 1 review](smartest-v2-phase1-review-2026-08-25.md). The next handoff is a
small new cohort with corrected instrumentation. The formal multi-day Phase 2
window starts only after that cohort is `QUALIFIED`.

The directly assignable work package is the
[Phase 1 correction TZ](smartest-v2-phase1-correction-tz-2026-08-25.md).
Qualification cohort `c` explicitly uses
`fidelity_auto_validate_weight_threshold=0.85`; lower-weight units may still
be routed by configured risk tags or Core-path evidence.

The corrected preflight passed
[checkpoint A review](smartest-v2-checkpoint-a-review-2026-08-26.md).
Smartest now collects 1-3 natural terminal Sleep runs without changing the
cohort configuration. Phase 2 remains closed until checkpoint B receives the
final Phase 1 `QUALIFIED` decision.

The corrected cohort uses study-export schema version `v2` for all six JSON
record types below. The file names stay unchanged. This prevents v1 rows with
ambiguous attempts or missing scope/process fields from being mixed into the
formal cohort.

## Why this validation exists

The 22–23 August Smartest sample exposed the largest real cost and observability
gaps in Memory Engine. One measured Sleep run compressed 16 messages containing
8,081 characters of live text but issued six calls with 87,913 input tokens.
The effective amplification was about 24 times. Across that traffic window,
memory work accounted for 61% of the bot's provider input. Smartest identified
that 46% of the repeated event block came from its own host payload and is
correcting that part separately.

Memory Engine then implemented eleven production-facing corrections. The main
changes are one compact shared Sleep source, provider-cache-friendly ordering,
selectable tracks, call and retry budgets, completion from available results,
structured role-aware chat context, empty-session semantics, explanatory Core
bridge reports, a visible fidelity/reflection funnel and durable recovered
attempt telemetry.

Laboratory tests prove the contracts and failure handling. Smartest is the
first environment that can show how they behave under long Ukrainian group
conversations, real provider filtering, multiple speakers, restarts and actual
billing metadata.

The project question is:

> Does the complete memory lifecycle preserve useful, correctly attributed
> long-term memory while reducing the token and provider cost paid by the
> integrating product?

## Hypotheses and decisions

The operator may change how a metric is collected to match Smartest's
infrastructure. The meaning of each hypothesis and measurement must remain the
same.

| ID | Hypothesis | Evidence that answers it | Decision supported |
|---|---|---|---|
| H1 | One compact shared source materially reduces repeated Sleep input when provider prefix caching is available. | Provider-reported prompt and cached tokens, shared-context identity, source size and billable input. | Keep the shared-prefix integration and establish a real cost baseline. |
| H2 | Call budgets, retries and early completion preserve successful paid work and leave every run terminal and explainable. | Per-attempt records, pass reports, completion modes, pending tasks and restart evidence. | Set production budgets and retry limits without silent Archive loss. |
| H3 | Compact input and an economy track profile can preserve the useful memory produced by the full six-track profile. | Per-track output, MemoryUnit quality, Core routing, later recall and controlled replay. | Choose the least expensive profile that preserves required product behavior. |
| H4 | Signal-level attribution keeps group memory attached to the correct participant and routes hearsay to review. | Local review of units and every automatic Core promotion, grouped by subject and bridge reason. | Accept group memory or return a concrete attribution defect to the engine. |
| H5 | The active → fidelity-valid → eligible funnel produces enough reliable evidence for meaningful autonomous reflection. | Reflection snapshots and outcomes across several Sleep cycles. | Close v0.5 live acceptance, tune a measured bottleneck, or record insufficient data. |
| H6 | Structured prompt plans preserve role order, current-message uniqueness and host payload while keeping memory within its budget. | Per-turn prompt-plan diagnostics and a local response sample. | Adopt the structured path without host-side string parsing. |
| H7 | The runtime remains self-healing over real provider failures and process restarts. | Recovered errors, repairs, task backlog, collection warnings and storage health snapshots. | Accept long-running operation or isolate a recovery defect. |

A failed hypothesis is a useful result. Thresholds and profiles are changed
only after the failing stage is identified. The protocol has no predetermined
cache percentage, reflection rate or preferred model.

## Investigator latitude

The fields below define evidence semantics, not a mandatory logging framework.
Smartest may use its existing structured logger, database, tracing platform or
provider ledger.

The operator should:

- preserve stable join keys between provider call, Sleep run, Session and
  reflection evaluation;
- preserve exact provider-reported usage separately from engine estimates;
- use `null` for unavailable values and state why they are unavailable;
- document any renamed field or equivalent calculation;
- pause the long collection when joins, units or accounting are clearly wrong;
- create a new labelled cohort after changing models, prompts, track profile,
  budgets or instrumentation semantics;
- report an unexpected bottleneck even when this document did not predict it.

Literal compliance that produces misleading data is a failure of the test.
The hypotheses, privacy boundary and comparability rules are the authority.

## Privacy and handling

The operator may inspect private content locally to judge support and
attribution. The package returned to Memory Engine maintainers contains only:

- randomly remapped chat, run, event, unit, candidate and fact identifiers;
- keyed HMAC-SHA-256 fingerprints for private contexts or queries, using a
  secret salt retained by Smartest;
- counts, durations, token usage, statuses, reasons and local quality labels;
- short operator-written anomaly descriptions with personal details removed.

Do not return API keys, Telegram tokens, participant names, raw chat text,
memory theses, provider request bodies, full prompts, runtime directories or
unredacted Archive/Core files. A plain unsalted hash of a short user query is
also excluded because it can be guessed. Keep source evidence under Smartest's
own access controls.

## Version and comparison boundary

The acceptance window must use:

- a Python extension built from production-feedback runtime commit `6dd6b68`
  or a later compatible descendant, with the exact source commit, loaded path
  and SHA-256 recorded;
- protocol revision `1e3101f` or a compatible later revision recorded
  separately from the extension source commit; documentation-only revisions
  do not require rebuilding an otherwise identical extension;
- multi-pass Sleep prompt version `3` or a compatible later version;
- Smartest's own repeated event payload fix enabled or separately measured.

The older 22–23 August build remains a historical baseline. It cannot serve as
the acceptance build because it lacks parts of the new control and reporting
surface. Monetary comparison with that baseline is valid only after removing
Smartest's own payload using the same accounting rule. Otherwise the report
states `not_comparable` and gives new absolute measurements.

Record exact providers and models assigned to chat, `fast`, `balanced` and
`reasoning` roles. One model may fill several roles; the protocol evaluates the
actual configuration rather than prescribing a provider.

---

## Phase 1 — Instrumentation qualification

### Phase 1 goal

Prove that Smartest can produce a complete, privacy-safe and joinable record
before several days of traffic are measured. One natural Sleep run or one
isolated replay is sufficient.

Phase 1 ends with a small sample returned to the Memory Engine maintainer. Do
not start the formal observation window until this sample is reviewed.

### Step 1. Record the tested build and configuration

Create `smartest_validation_build.json` containing:

```json
{
  "schema_version": "smartest_validation_build.v2",
  "engine_commit": "6dd6b68-or-compatible-descendant",
  "engine_build_source": "ci-artifact-manifest-or-equivalent",
  "host_commit": "...",
  "loaded_extension_path": "redacted-or-relative-path",
  "loaded_extension_sha256": "sha256:...",
  "prompt_commit": "...",
  "sleep_prompt_version": 3,
  "provider_models": {
    "chat": "exact-provider/model",
    "fast": "exact-provider/model",
    "balanced": "exact-provider/model",
    "reasoning": "exact-provider/model"
  },
  "provider_api_surfaces": {
    "gemini": "generateContent|interactions|vertex"
  },
  "sleep_tracks": [
    "memory_unit",
    "emotional",
    "topic_thread",
    "personal_signal",
    "relational",
    "consolidator"
  ],
  "max_provider_calls": null,
  "max_pass_attempts": 3,
  "host_payload_fix_active": true,
  "vectors_enabled": false,
  "vector_model": null,
  "fidelity_auto_validation": {
    "enabled": true,
    "weight_threshold": 0.85,
    "routing_owner": "memory_engine",
    "configured_tags": ["identity", "profile", "personal", "..."]
  },
  "fidelity_config_digest": "hmac-sha256:...",
  "reflection_config_digest": "hmac-sha256:..."
}
```

Configuration digests prove that a cohort kept one configuration without
exporting private or operational secrets. Preserve the full configuration
inside Smartest's protected records. Keep the selected source-event IDs and
their protected runtime data available for a later isolated replay; only
anonymized IDs leave Smartest.

`engine_commit` must identify the source that produced the loaded extension.
When the runtime server has no Git checkout, the build pipeline writes the
commit into an artifact manifest and the exporter joins it by extension
SHA-256. `unknown` does not qualify the build.

### Step 2. Add five structured log streams

The names are recommendations. Equivalent tables or tracing events are valid
when they can be exported into these shapes. These are study-export schemas,
not new public Memory Engine API contracts.

| Stream | Write point | Canonical source inside Smartest |
|---|---|---|
| `provider_calls` | Immediately after every physical provider request, including inner transport retries and failures. | Host provider executor plus exact provider usage metadata. |
| `sleep_runs` | Once after a terminal Sleep outcome. | `SleepOutcome`, its Archive entry, Core summary, pass reports and post-processing summaries. |
| `reflection_snapshots` | After post-Sleep fidelity and after any reflection submit. | `reflection_status` plus the structured reflection outcome. |
| `chat_context_turns` | Once for every ordinary chat-generation call. | `core_context_package`, its budget report, `memory_prompt_plan.v1` and provider usage. |
| `memory_health` | At startup, after Sleep and once daily. | Engine/storage status, warnings, pending work and directory sizes where available. |

Fields that do not apply to a workflow are `null`. For example, an ordinary
chat call has no Sleep `run_id` or shared context.

#### 2.1 Provider calls

Write one record for every physical chat, Sleep, fidelity and reflection model
request. Local embedding work may use the same stream with `provider = local`.

```json
{
  "schema_version": "smartest_provider_call.v2",
  "timestamp": "RFC3339",
  "workflow": "sleep",
  "chat_id": "anon_chat_001",
  "run_id": "anon_run_001",
  "origin_run_id": "anon_run_001",
  "request_id": "anon_physical_request_001",
  "engine_request_id": "anon_engine_request_001",
  "task_id": "anon_task_001",
  "prompt_id": "memory_unit_pass",
  "engine_attempt": 1,
  "provider_attempt": 1,
  "model_role": "balanced",
  "provider": "gemini",
  "model": "exact-model-id",
  "shared_context_id": "sleep:anon_archive_001",
  "shared_context_fingerprint": "hmac-sha256:...",
  "shared_context_first": true,
  "shared_context_chars": 8423,
  "shared_context_estimated_tokens": 4212,
  "instruction_chars": 1400,
  "prompt_tokens": 1500,
  "cached_input_tokens": 1200,
  "billable_input_tokens": 300,
  "output_tokens": 220,
  "thoughts_tokens": 0,
  "total_tokens": 1720,
  "input_cost": null,
  "cached_input_cost": null,
  "output_cost": null,
  "currency": null,
  "pricing_version_date": null,
  "latency_ms": 840,
  "status": "ok",
  "error_kind": null
}
```

Take token and cache fields from provider usage metadata. Keep character-based
engine estimates in their separate `*_estimated_tokens` fields. When the
provider does not expose cache or billing fields, write `null`; do not estimate
them silently. Preserve raw provider usage inside Smartest and return only the
normalized numeric fields.

Map provider errors to stable categories such as `rate_limit`, `timeout`,
`content_filter`, `invalid_response`, `transport` and `unknown`. Raw provider
error text stays in protected logs.

`engine_attempt` counts Memory Engine semantic/pass attempts.
`provider_attempt` counts transport retries made while executing one engine
request. `request_id` is unique for every actual provider request, while
`engine_request_id` groups its transport retries. Write failed physical
requests as separate rows instead of exporting only the final retry.

Post-Sleep fidelity, repair and reflection requests inherit `origin_run_id`
from the Sleep outcome that scheduled them. A genuinely manual or scheduled
request without a parent run leaves the field `null` and records its origin
category separately.

#### 2.2 Sleep runs

Write one record after every terminal Sleep outcome:

```json
{
  "schema_version": "smartest_sleep_run.v2",
  "run_id": "anon_run_001",
  "chat_id": "anon_chat_001",
  "session_id": "anon_session_001",
  "started_at": "RFC3339",
  "finished_at": "RFC3339",
  "source_event_count": 16,
  "source_memory_text_chars": 8081,
  "source_memory_text_estimated_tokens": 4041,
  "source_token_estimator": "unicode_chars_div_2_ceil_text_v1",
  "shared_context_chars": 8423,
  "shared_context_estimated_tokens": 4212,
  "enabled_tracks": [
    "memory_unit",
    "emotional",
    "topic_thread",
    "personal_signal",
    "relational",
    "consolidator"
  ],
  "provider_calls_issued": 6,
  "max_provider_calls": null,
  "budget_exhausted": false,
  "completion_mode": "consolidated",
  "failed_passes": [],
  "skipped_passes": [],
  "pass_reports": [],
  "output_counts": {
    "memory_units": 4,
    "emotional_markers": 1,
    "topic_items": 2,
    "personal_signals": 2,
    "relational_tone": 1,
    "gist_present": 1,
    "narrative_present": 1
  },
  "core_created": 1,
  "core_updated": 0,
  "core_skipped": 1,
  "core_review_candidates": 1,
  "core_skip_reasons": {"requires_attribution_review": 1},
  "fidelity_requested": 3,
  "new_unit_weight_histogram": {
    "below_0_65": 1,
    "0_65_to_0_84": 2,
    "0_85_and_above": 1
  },
  "auto_fidelity_weight_histogram": {
    "below_0_65": 0,
    "0_65_to_0_84": 1,
    "0_85_and_above": 1
  },
  "fidelity_valid": 3,
  "fidelity_rejected": 0,
  "fidelity_needs_revision": 0,
  "fidelity_failed": 0,
  "memory_unit_repairs": 0,
  "embedding_requests": 0,
  "pending_tasks_after_run": 0
}
```

`source_memory_text_chars` counts only the memory text supplied for the
conversation, excluding Smartest's operational payload.
`unicode_chars_div_2_ceil_text_v1` means `ceil(Unicode scalar count / 2)` over
that concatenated text and is intentionally separate from the engine's JSON
budget estimator. Keep every
`pass_report`: attempts, errors by stable category, terminal status and
`recovered_after_retry`. A successful retry still records the earlier failure.
Record text-free weight histograms for all new units and units selected by
automatic fidelity. Keep the same documented bins for the whole cohort. This
shows coverage without asking the host to reproduce the core's routing policy.

#### 2.3 Reflection snapshots

Capture `reflection_status` immediately after post-Sleep fidelity and record
the optional reflection result when a task runs:

```json
{
  "schema_version": "smartest_reflection_snapshot.v2",
  "timestamp": "RFC3339",
  "run_id": "anon_run_001",
  "origin_run_id": "anon_run_001",
  "snapshot_phase": "after_auto_fidelity",
  "reflection_scope": "anon_scope_001",
  "active_units": 12,
  "valid_units": 7,
  "unchecked_units": 4,
  "fidelity_excluded_units": 5,
  "eligible_units": 4,
  "accumulated_salience": 2.8,
  "salience_remaining": 0.0,
  "salience_threshold": 2.5,
  "due": true,
  "reason": "accumulated_salience",
  "task_started": true,
  "task_status": "completed",
  "action_types": ["new_candidate"],
  "new_candidate_count": 1,
  "reinforcement_count": 0,
  "core_challenge_count": 0,
  "no_insight_reason": null,
  "cited_unit_count": 3,
  "source_session_count": 2
}
```

Keep candidate and evidence text inside Smartest. Returned records contain
only action types, counts, anonymized IDs and final statuses.

Complete every fidelity request returned by the Sleep outcome before capturing
the snapshot. The host does not select additional units. Record the core-owned
automatic routing threshold and text-free counts by routing reason. A second
snapshot after reflection submit uses `snapshot_phase: after_reflection`. This
ordering makes the active -> valid -> eligible funnel reproducible.

#### 2.4 Chat context turns

Write one record for ordinary response generation. This measures the actual
token-facing product path as well as Sleep:

```json
{
  "schema_version": "smartest_chat_context_turn.v2",
  "timestamp": "RFC3339",
  "turn_id": "anon_turn_001",
  "chat_id": "anon_chat_001",
  "session_id": "anon_session_001",
  "prompt_plan_schema": "memory_prompt_plan.v1",
  "conversation_state": "ongoing",
  "include_core": true,
  "requested_core_scope_fingerprint": "hmac-sha256:...",
  "session_personality_scope_fingerprint": "hmac-sha256:...",
  "core_notes": [],
  "core_fact_count": 12,
  "archive_item_count": 4,
  "associative_item_count": 0,
  "dialogue_event_count": 18,
  "dialogue_payload_event_count": 18,
  "current_message_occurrences": 1,
  "role_order_valid": true,
  "estimated_memory_tokens": {
    "total": 2100,
    "current": 1300,
    "compressed": 600,
    "core": 200,
    "associative": 0,
    "domain_state": 0
  },
  "dropped_by_budget": {
    "session_recent": 0,
    "session_trace": 0,
    "archive": 0,
    "core": 0,
    "associative": 0
  },
  "budget_exceeded": false,
  "provider_prompt_tokens": 3400,
  "provider_output_tokens": 250,
  "latency_ms": 930,
  "response_status": "ok"
}
```

Counts and estimates come from `core_context_package`, its budget report and
`memory_prompt_plan.v1`. `current_message_occurrences` must be exactly one.
`dialogue_payload_event_count` proves that role-aware assembly preserved host
payload without a second Session read. No dialogue text is exported.

Scope fingerprints use the export HMAC salt and reveal no raw scope IDs.
Convert package notes into fixed diagnostic categories before export. This
lets the maintainer distinguish an intentionally empty Core from a disabled or
mismatched Core request without receiving private text.

#### 2.5 Runtime health snapshots

At process start, after every Sleep and once daily, record:

```json
{
  "schema_version": "smartest_memory_health.v2",
  "timestamp": "RFC3339",
  "process_instance_id": "anon_process_001",
  "snapshot_reason": "startup",
  "measurement_scope": "global",
  "chat_id": "anon_chat_001",
  "sessions": 4,
  "complete_archives": 12,
  "active_memory_units": 31,
  "core_facts": 14,
  "review_candidates": 2,
  "pending_tasks": 0,
  "pending_tasks_by_type": {},
  "oldest_pending_task_age_seconds": null,
  "pending_sleep_runs": 0,
  "collection_warnings": 0,
  "global_embeddings_enabled": false,
  "vector_state": "disabled",
  "vector_rows": 0,
  "storage_bytes": {
    "sessions": 0,
    "archive": 0,
    "core": 0,
    "vectors": 0,
    "tasks_and_runs": 0
  }
}
```

Filesystem sizes are optional when Smartest stores or mounts runtime data in a
way that makes them expensive to calculate. Mark unavailable fields `null`.

Use a new anonymized `process_instance_id` on each process start. Health
records use `startup`, `post_sleep` or `daily` as `snapshot_reason`. Pending
task counts are grouped by task type so fidelity, embedding and workflow work
cannot be confused. `global_embeddings_enabled` and the per-scope
`vector_state` must describe the same runtime configuration. State whether
each count is global or personality-scoped; add an HMAC scope fingerprint for
scope-specific counts.

### Step 3. Verify joins and privacy on one run

Before returning the Phase 1 sample, confirm:

- the loaded extension maps by artifact manifest and SHA-256 to accepted
  runtime commit `6dd6b68` or a compatible descendant;
- one Sleep batch has one `shared_context`;
- every selected pass references that context and has the same fingerprint;
- the shared source is placed before each pass instruction;
- pass-specific input contains no inline event array or full `sleep_task`;
- every physical provider request has a unique row and its engine/provider
  attempt numbers are independently reconstructable;
- provider-call records join to exactly one Sleep run and pass report, or carry
  an explicit non-Sleep origin;
- the Sleep record joins to a timestamped reflection snapshot after every
  core-routed fidelity request completes;
- one chat turn uses `memory_prompt_plan.v1` or an equivalent structured plan;
- the current message occurs once and dialogue roles are ordered;
- the chat row contains the package budget report and Core request/scope
  diagnostics;
- health distinguishes task types, process instances and global/per-scope
  vector state;
- the exported sample contains no secrets or private text.

A natural retry is not required for Phase 1 qualification. The logging path
must support it; observed retry behavior is measured in Phase 2.

### Step 4. Return the Phase 1 sample

Return:

1. `smartest_validation_build.json`;
2. 3–10 redacted `smartest_provider_call.v2` records;
3. one `smartest_sleep_run.v2` record;
4. one `smartest_reflection_snapshot.v2` record;
5. one `smartest_chat_context_turn.v2` record;
6. one `smartest_memory_health.v2` record;
7. `smartest_instrumentation_notes.md` describing field sources, unavailable
   fields, deviations and the local privacy check.

Send this compact handoff in chat:

```text
Smartest Phase 1 instrumentation sample
Engine commit:
Host build:
Loaded extension hash:
Prompt version:
Provider/models by role:

One shared Sleep context: PASS/FAIL
Shared context first: PASS/FAIL
Provider usage available: yes/partial/no
Cached-token field available: yes/no
Call -> run -> reflection joins: PASS/FAIL
Structured chat plan: PASS/FAIL
Current message exactly once: PASS/FAIL
Privacy check: PASS/FAIL

Unavailable fields:
Deviations from the proposed schemas:
Sample artifact names or paths:
Operator questions:
```

Phase 1 result is `QUALIFIED`, `CHANGES_REQUIRED` or `BLOCKED`. Begin Phase 2
only after `QUALIFIED`.

---

## Phase 2 — Passive production observation

### Observation window

Preferred minimum after Phase 1 qualification:

- at least 20 terminal Sleep runs;
- at least three large group chats;
- at least two calendar days;
- at least 300 user/assistant events in total;
- enough natural traffic for several post-Sleep reflection evaluations.

If the window ends earlier, return all evidence and state the shortfall. Do not
manufacture medical, personal or contradictory dialogue. Natural provider
filtering and ordinary group traffic are the relevant evidence.

Keep the full six-track profile during this window. This isolates the compact
source/cache correction from a change in memory semantics. Every configuration
or model change starts a separately labelled cohort.

### Local quality review

The operator reviews at least 30 produced MemoryUnits, or all units when fewer
exist. Return counts under these labels:

- `supported_and_attributed`;
- `supported_but_too_broad`;
- `wrong_speaker`;
- `unsupported`;
- `duplicate`;
- `routine_but_acceptable`.

Review every automatic Core promotion in the window and count:

- correct self-statement and correct subject;
- stable fact that should have remained in Archive;
- about-other claim incorrectly promoted;
- duplicate or conflicting active fact;
- correct review routing.

Also sample up to 30 ordinary replies that visibly rely on remembered
information. Count `correct_memory_use`, `wrong_person`, `unsupported_memory`,
`relevant_memory_ignored` and `memory_not_needed`. This is the product-facing
check that token savings preserved useful conversation behavior.

Record the sampling rule and reviewer role. Private evidence and prose stay
inside Smartest.

### What else to debug during the window

Create one redacted anomaly row for every occurrence of:

- terminal or recovered provider block;
- budget exhaustion or host-requested early finish;
- Complete Archive with zero MemoryUnits;
- memory-unit repair;
- pending task or Sleep run left after completion/restart;
- empty Core with personal signals present;
- wrong or ambiguous participant attribution;
- duplicate current message or broken role order;
- missing host payload in a structured dialogue event;
- malformed Session or collection warning;
- reflection remaining below threshold despite multiple valid units;
- unexpected growth in memory files or latency;
- vector index mismatch, corruption or irrelevant high-similarity result when
  vectors are enabled.

Each row needs anonymized run/turn ID, stage, symptom, engine/host status,
recovery result and a short operator diagnosis.

### Reflection evidence required for v0.5

Across several Sleep cycles, seek natural evidence of:

- an evidence-backed new candidate or reinforcement from repeated stable
  material;
- structured `no_insight` for episodic or already represented material;
- an attribution-sensitive case where another participant's statement stays
  outside automatic Core;
- a process restart with no duplicate reflection effect.

When natural traffic does not produce one of these cases, mark it
`not_observed`. An isolated copied runtime may fill a missing restart or
contradiction check and must be labelled `synthetic_isolated`.

---

## Phase 3 — Analysis and controlled replay

### Required aggregate calculations

Provide totals and median/p95 by run for:

- source events and source memory-text characters;
- shared-context tokens;
- provider prompt/output/thought tokens;
- cached and billable input tokens;
- provider calls, retries and latency;
- calls and tokens by pass;
- per-track output counts;
- MemoryUnits and fidelity outcomes;
- Core promotions, review routes and skip reasons;
- reflection funnel counts and trigger reasons;
- chat memory-context tokens and their share of total prompt tokens;
- storage growth and pending-work high-water marks.

Calculate:

```text
input amplification = total provider prompt tokens / estimated source-text tokens
cache ratio = cached input tokens / provider prompt tokens
effective uncached amplification = (prompt tokens - cached input tokens) / estimated source-text tokens
memory context share = estimated memory-context tokens / provider prompt tokens for chat calls
cost per terminal Sleep = total memory provider cost / terminal Sleep runs
```

When provider billing fields are unavailable, report token quantities and mark
money-based calculations unavailable.

### Interpretation map

Use the first matching explanation as a starting point, then inspect the joined
records:

- identical shared fingerprints with low cache use: provider cache policy,
  request ordering or model-level cache support;
- large shared source before provider calls: host payload or compact rendering
  integration problem;
- many units with few valid units: thesis quality or fidelity configuration;
- valid units with no eligible units: checkpoint/session membership or status
  filtering;
- eligible salience that never triggers: reflection scheduling or threshold
  configuration;
- reflection triggers with only duplicates: selection or candidate-comparison
  quality;
- personal signals with no Core and explicit skip reasons: evaluate the stated
  reasons before changing thresholds;
- wrong-subject Core promotion: engine attribution defect and immediate stop
  condition;
- successful retries with missing attempt history: host telemetry defect;
- budget-finished runs with missing successful outputs: completion defect;
- clean stored memory with poor remembered replies: chat-context assembly or
  response-model use of memory.

This map is guidance. The operator should report a better evidence-backed
explanation when the data points elsewhere.

### Economy-profile replay

Run after the full-profile production window, preferably on an isolated copy of
the same inputs:

```json
{
  "enabled_tracks": ["memory_unit", "personal_signal", "consolidator"],
  "max_provider_calls": 5,
  "max_pass_attempts": 2
}
```

Compare provider cost, MemoryUnit quality, Core routing, fidelity, repair,
reflection input and later recall. This profile omits emotional forgetting
protection, topic structure and relational archive metadata. Adoption requires
an explicit product decision after the quality comparison.

### Isolated product integration checks

These checks use disposable copied data and do not require participant action:

- `delete_session` makes the live Session empty while preserving durable scope
  memory;
- `purge_scope` removes one disposable scope and leaves a neighbouring scope
  intact;
- a scheduled process restart leaves no duplicate Sleep/reflection outcome;
- vectors, when enabled, return only the tested scope and survive rebuild.

Erasure and vector checks are useful integration evidence. They do not block
the v0.6 cognitive decision when Smartest does not expose those product actions
in this test window.

---

## Phase 4 — Return package and decisions

Return:

1. `smartest_validation_summary.md` - context, hypotheses, configuration,
   conclusions and deviations;
2. `smartest_validation_build.json`;
3. `smartest_provider_calls.jsonl`;
4. `smartest_sleep_runs.jsonl`;
5. `smartest_reflection_snapshots.jsonl`;
6. `smartest_chat_context_turns.jsonl`;
7. `smartest_memory_health.jsonl`;
8. optional `smartest_anomalies.csv` and pricing export containing only usage
   and cost fields.

Use this final chat handoff:

```text
Smartest production validation
Engine commit:
Host build:
Prompt version:
Observation window:
Chats / events / terminal Sleep runs:
Provider and role models:
Sleep profile and limits:

H1 shared-prefix economy: CONFIRMED/REJECTED/INSUFFICIENT_DATA
H2 budget/recovery: CONFIRMED/REJECTED/INSUFFICIENT_DATA
H3 memory quality under tested profile: CONFIRMED/REJECTED/INSUFFICIENT_DATA
H4 participant attribution: CONFIRMED/REJECTED/INSUFFICIENT_DATA
H5 reflection funnel: CONFIRMED/REJECTED/INSUFFICIENT_DATA
H6 structured chat context: CONFIRMED/REJECTED/INSUFFICIENT_DATA
H7 long-run recovery: CONFIRMED/REJECTED/INSUFFICIENT_DATA

Prompt tokens / cached input / billable input:
Input amplification / effective uncached amplification:
Complete / budget-finished / failed runs:
Recovered provider failures / repairs / pending tasks:
Units reviewed and quality counts:
Core attribution and routing counts:
Reflection evaluations / due / completed / action types:
Reply memory-use sample counts:
Storage growth and latency summary:

v0.5 live gate: PASS/FAIL/INSUFFICIENT_DATA
Recommended production profile:
Blocking defects:
Unexpected findings:
Artifact names or paths:
Privacy check: PASS/FAIL
Operator conclusion:
```

## Acceptance and v0.6 entry gate

Production integration receives PASS when:

- Phase 1 instrumentation is qualified;
- the shared-prefix contract is present and usage is measured or explicitly
  unavailable from the provider;
- every run reaches a terminal recoverable state and successful paid work is
  represented in the final Archive;
- retries, Core bridge decisions and memory repairs are observable;
- local review finds no about-other automatic Core promotion;
- structured chat context preserves roles, payload and one current message;
- returned artifacts contain no private text or secrets.

There is no universal cache-savings or accuracy percentage attached to this
PASS. The report states measured values and confidence for this Smartest build.

v0.6 Phase 3 may begin after the report also closes the v0.5 live gate: the
valid-unit funnel is understood, at least one non-degenerate reflection action
is observed, restart produces no duplicate effect and no unresolved
attribution or data-integrity defect remains. Vector enrichment is optional;
the structural non-vector path is the required baseline.

If the natural window lacks enough repeated evidence, the result is
`INSUFFICIENT_DATA`. Collection can continue without changing thresholds. A
failing stage is repaired and re-measured as a new labelled cohort.
