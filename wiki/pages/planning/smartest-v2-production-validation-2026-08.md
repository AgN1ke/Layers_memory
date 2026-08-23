# Smartest v2 Production Validation — 2026-08

## Related pages

- [Production Integration Feedback](production-feedback-2026-08.md) - the
  eleven production findings and the implemented library response.
- [Roadmap](roadmap.md) - active ordering from production validation into
  autonomous reflection and v0.6.
- [Autonomous Reflection v0.5](autonomous-reflection-v0.5.md) - trigger,
  evidence and live-acceptance rules that this run must calibrate.
- [Reflective Belief System](reflective-belief-system.md) - the v0.6 reasoning
  phase that opens after this evidence gate.
- [LLM Integration Resources](../integration/llm-integration-resources.md) -
  provider execution, model roles, keys and shared-prefix responsibilities.
- [Contracts](../foundation/contracts.md) - exact Sleep, context, reflection
  and telemetry shapes.

## Status and audience

Status: **approved production-validation specification; execution pending**.

This document is addressed to the Smartest v2 engineer or operator who can
inspect provider usage ledgers, Memory Engine runtime records, application
configuration and logs. Chat participants do not need to perform scripted
actions. The test uses naturally occurring large Ukrainian group chats.

The requested return is a reproducible operational report. It must contain
aggregated measurements and identifiers/hashes. It must not contain API keys,
Telegram tokens, full prompts, raw chat text, memory theses, participant names
or other personal data.

## Purpose

The run has three goals:

1. verify that the production integration uses the compact shared Sleep source
   and receives real provider-cache benefit where the provider supports it;
2. verify that cost controls preserve complete, attributable and observable
   memory on long group conversations;
3. collect the live reflection funnel and outcomes needed to close v0.5 and
   decide whether v0.6 hierarchical reasoning may begin.

This is a product measurement, not a public benchmark. Any result belongs to
the tested Smartest version, traffic window, provider configuration and model.

## Required tested build

Record all of the following before the first measured message:

- Memory Engine commit: `6dd6b68` or a descendant containing
  `Address production integration feedback`;
- Smartest host commit/build identifier;
- loaded Python extension path and SHA-256;
- prompt repository commit and multi-pass Sleep prompt version `3`;
- provider and exact model for `fast`, `balanced`, `reasoning` and chat roles;
- enabled Sleep tracks, total call limit and attempts per pass;
- fidelity and reflection configuration;
- vector state/model/dimension for each tested scope, when vectors are enabled;
- confirmation that Smartest's own repeated event payload has been removed or
  separated from the measurement.

Keep the full six-track historical profile for the first measurement window.
This isolates the compact-source/cache change from a memory-quality change.
An economy profile may be evaluated later on an isolated replay or a separately
labelled cohort.

## Privacy and handling

The operator may inspect private content locally to judge attribution and
fidelity. The package returned to Memory Engine maintainers contains only:

- randomly remapped chat, run, event, unit, candidate and fact identifiers;
- SHA-256 hashes of shared contexts and queries;
- counts, durations, token usage, statuses, reasons and local quality labels;
- short operator-written anomaly descriptions with personal details removed.

Do not upload `memory/`, `sessions/`, provider request bodies, bot logs with
message previews or unredacted archive/Core files. Keep the original evidence
under the Smartest product's own access controls.

## Observation window

Preferred minimum:

- at least 20 completed Sleep runs;
- at least three large group chats;
- at least two calendar days;
- at least 300 user/assistant events in total;
- enough natural traffic for several post-Sleep reflection evaluations.

If the product test ends earlier, return every available run and state the
shortfall. Do not manufacture medical, personal or contradictory dialogue to
fill a quota. Naturally occurring provider filtering is sufficient.

## What to verify before collecting metrics

For one Sleep run, inspect the engine-to-host boundary and record these boolean
checks:

- the batch has one `shared_context`;
- every selected pass references `source = shared_context`;
- pass-specific inputs contain no inline event array or full `sleep_task`;
- `shared_context.content` is placed before the pass instruction;
- every request in the run has the same shared-context ID and SHA-256;
- batch-level session/archive/scope values appear once in the compact source;
- the durable workflow input remains sufficient for provenance and recovery;
- role-aware chat assembly uses `memory_prompt_plan.v1` or an equivalent
  structured package path and performs no regular-expression parsing of
  `render_memory_view`.

Any failed check blocks cost conclusions. Return the failed item and the host
code location that assembled the request.

## Data to collect for every provider call

Write one machine-readable record per call. Use `null` when the provider does
not expose a field; never infer provider-reported numbers from character count.

```json
{
  "run_id": "anon_run_001",
  "request_id": "anon_request_001",
  "prompt_id": "memory_unit_pass",
  "attempt": 1,
  "model_role": "balanced",
  "provider": "gemini",
  "model": "exact-model-id",
  "shared_context_id": "sleep:anon_archive_001",
  "shared_context_sha256": "sha256:...",
  "shared_context_estimated_tokens": 1234,
  "prompt_tokens": 1500,
  "cached_input_tokens": 1200,
  "output_tokens": 220,
  "thoughts_tokens": 0,
  "total_tokens": 1720,
  "billable_input_tokens": 300,
  "latency_ms": 840,
  "status": "ok",
  "error_kind": null
}
```

`cached_input_tokens` and `billable_input_tokens` should come from provider
usage/billing metadata when available. If billing exposes price directly, add
`input_cost`, `cached_input_cost`, `output_cost`, currency and pricing-version
date. Preserve the raw provider usage object inside Smartest's protected logs;
return only the numeric fields above.

## Data to collect for every Sleep run

Write one record per run:

```json
{
  "run_id": "anon_run_001",
  "chat_id": "anon_chat_001",
  "started_at": "RFC3339",
  "finished_at": "RFC3339",
  "source_event_count": 16,
  "source_text_chars": 8081,
  "shared_context_chars": 8423,
  "shared_context_estimated_tokens": 4212,
  "enabled_tracks": ["memory_unit", "emotional", "topic_thread", "personal_signal", "relational", "consolidator"],
  "provider_calls_issued": 6,
  "max_provider_calls": null,
  "budget_exhausted": false,
  "completion_mode": "consolidated",
  "failed_passes": [],
  "skipped_passes": [],
  "recovered_passes": 0,
  "memory_units": 4,
  "core_created": 2,
  "core_updated": 0,
  "core_skipped": 1,
  "core_review_candidates": 1,
  "core_skip_reasons": {"requires_attribution_review": 1},
  "fidelity_requested": 3,
  "fidelity_valid": 3,
  "fidelity_rejected": 0,
  "fidelity_needs_revision": 0,
  "fidelity_failed": 0,
  "pending_tasks_after_run": 0
}
```

Also retain each `pass_report`: attempts, errors, terminal status and
`recovered_after_retry`. A run with a successful retry must still show the
earlier provider failure.

## Reflection record after every Sleep

Capture the reflection trigger status immediately after post-Sleep fidelity:

```json
{
  "run_id": "anon_run_001",
  "active_units": 12,
  "valid_units": 7,
  "unchecked_units": 4,
  "fidelity_excluded_units": 5,
  "eligible_units": 4,
  "accumulated_salience": 2.8,
  "salience_remaining": 0.0,
  "due": true,
  "reason": "accumulated_salience",
  "task_started": true,
  "task_status": "completed",
  "action_type": "new_candidate",
  "candidate_count_delta": 1
}
```

For completed reflection tasks record only action type, counts, anonymized IDs,
number of cited units/sessions and final candidate/Core statuses. Keep candidate
text and evidence text inside Smartest.

For v0.5 calibration, preserve the configured values and report their observed
distribution before proposing changes. A useful live gate includes several
Sleep cycles and at least:

- one evidence-backed new candidate or reinforcement from repeated stable
  evidence;
- one structured `no_insight` for episodic material;
- one attribution-sensitive case where another participant's statement stays
  outside automatic Core;
- a restart or process recovery with no duplicate reflection side effect.

When natural traffic does not produce one of these cases, mark it
`not_observed`. Use an isolated copied runtime only for a missing fault or
contradiction case and label it `synthetic_isolated`.

## Local quality review

The operator should inspect a sample of at least 30 produced MemoryUnits, or all
units when fewer exist. Return counts under these labels:

- `supported_and_attributed`;
- `supported_but_too_broad`;
- `wrong_speaker`;
- `unsupported`;
- `duplicate`;
- `routine_but_acceptable`.

Inspect every automatic Core promotion in the window and return counts for:

- correct self-statement and correct subject;
- stable fact that should remain in Archive;
- about-other claim incorrectly promoted;
- duplicate or conflicting active fact;
- correct review routing.

These are operator labels, not benchmark scores. Include the sampling rule and
reviewer identity/role, while omitting memory text.

## Required aggregate calculations

Provide totals and median/p95 by run for:

- source events and source text characters;
- shared-context tokens;
- provider prompt/output/thought tokens;
- cached input tokens and cache-hit ratio;
- provider calls, retries and latency;
- calls and tokens by pass;
- MemoryUnits per run and fidelity outcomes;
- Core promotions, review routes and skip reasons;
- reflection funnel counts and trigger reasons.

Calculate:

```text
input amplification = total provider prompt tokens / estimated source-text tokens
cache ratio = cached input tokens / provider prompt tokens
effective uncached amplification = (prompt tokens - cached input tokens) / estimated source-text tokens
```

Compare with the 22–23 August baseline only when the old host payload can be
removed using the same accounting rule. Otherwise label the monetary comparison
`not_comparable` and still report the new absolute measurements.

## Economy-profile experiment

Run this only after the full-profile window is complete.

Recommended comparison profile:

```json
{
  "enabled_tracks": ["memory_unit", "personal_signal", "consolidator"],
  "max_provider_calls": 5,
  "max_pass_attempts": 2
}
```

Use an isolated replay of the same input windows when possible. Compare token
cost, Core promotions, MemoryUnit quality, fidelity routing, repair activity and
later recall. State explicitly that this profile omits emotional forgetting
protection, topic structure and relational archive metadata. Adoption is a
product decision after the quality comparison.

## Failure and anomaly report

Create one redacted row for every occurrence of:

- terminal or recovered provider block;
- budget exhaustion or host-requested early finish;
- Complete archive with zero MemoryUnits;
- memory-unit repair;
- pending task left after completion/restart;
- empty Core with personal signals present;
- wrong or ambiguous participant attribution;
- duplicate current message or broken role order in the chat prompt;
- malformed Session or collection warning;
- reflection remaining permanently below threshold despite multiple valid
  units.

Each row needs run ID, stage, observable symptom, engine/host status, recovery
result and a short redacted operator diagnosis.

## What to return to Memory Engine maintainers

Return three artifacts:

1. `smartest_validation_summary.md` - human conclusion and configuration;
2. `smartest_sleep_runs.jsonl` - one redacted record per Sleep run;
3. `smartest_provider_calls.jsonl` - one redacted record per provider call.

Optional: `smartest_anomalies.csv` and a pricing export containing only usage
and cost fields.

In chat, send this exact compact handoff:

```text
Smartest production validation
Engine commit:
Host commit/build:
Prompt commit/version:
Observation window:
Chats / events / Sleep runs:
Provider and role models:
Sleep profile and limits:
Host payload fix active: yes/no

Shared-prefix contract: PASS/FAIL
Provider cache observed: yes/no/unavailable
Prompt tokens total:
Cached input tokens total:
Input amplification:
Effective uncached amplification:
Previous baseline comparable: yes/no

Complete / budget-finished / terminal-failed runs:
Recovered provider failures:
Pending tasks after restart:
Units reviewed / supported / too broad / wrong speaker / unsupported / duplicate:
Core correct / wrong-subject / about-other leak / review-routed:

Reflection evaluations / due / completed:
New candidate / reinforcement / no-insight / Core challenge:
v0.5 live gate: PASS/FAIL/INSUFFICIENT_DATA

Anomalies:
Artifact paths or attachment names:
Privacy check: PASS/FAIL
Operator conclusion:
```

## Acceptance and v0.6 entry gate

The production-feedback slice is accepted when:

- the shared-prefix contract passes;
- every run reaches a terminal, recoverable state and paid successful work is
  represented in the final archive;
- provider retries and Core skip decisions are observable;
- group attribution review finds no about-other automatic Core promotion;
- the report includes complete usage accounting or clearly marks unavailable
  provider fields;
- the returned artifacts contain no private text or secrets.

There is no predetermined percentage claim for cache savings. The measured
ledger becomes the evidence.

v0.6 Phase 3 may start after the Smartest report also closes the v0.5 live
gate: the valid-unit funnel is understood, at least one non-degenerate
reflection action is observed, replay/restart produces no duplicate effect and
no unresolved attribution or data-integrity defect remains. Vector enrichment
is optional for v0.6; non-vector structural selection remains complete.
