# Smartest v2 Phase 1 Review - 2026-08-25

## Purpose

This page records the Memory Engine maintainer's independent review of the
first redacted Smartest v2 telemetry package. It separates evidence that is
already usable from instrumentation gaps that must be corrected before the
formal multi-day observation window begins.

> **Update 2026-11-15.** Smartest reported that its validation exporter has
> open privacy and provenance defects found by its own audit: a privacy PASS
> is not bound to the verified bytes, and an export is not a snapshot of one
> cohort and build. We have not verified those findings ourselves. Until
> Smartest fixes the exporter and returns a new qualification on exact bytes,
> the aggregates and the "privacy-safe" assessment below are a record of what
> was reviewed on 2026-08-25, not authoritative evidence. See the
> [Integration Hardening Plan](integration-hardening-plan-2026-11-15.md),
> stage H5 and decision D3.

## Related pages

- [Smartest v2 Production Validation](smartest-v2-production-validation-2026-08.md) -
  normative hypotheses, schemas, privacy boundary and acceptance sequence.
- [Smartest Phase 1 Correction TZ](smartest-v2-phase1-correction-tz-2026-08-25.md) -
  the standalone task to send to the Smartest engineer.
- [Production Integration Feedback](production-feedback-2026-08.md) - the
  eleven production findings that motivated the validation.
- [Roadmap](roadmap.md) - active ordering after this review.
- [Autonomous Reflection v0.5](autonomous-reflection-v0.5.md) - the live gate
  whose fidelity and reflection funnel this evidence must eventually close.
- [LLM Integration Resources](../integration/llm-integration-resources.md) -
  provider execution and model-role ownership.

## Review result

**Phase 1 status: `CHANGES_REQUIRED`.**

The returned package is privacy-safe, structurally useful and sufficient to
find real integration gaps. It does not yet qualify the instrumentation for a
formal Phase 2 observation window. Smartest should apply the correction
handoff below, start a new labelled cohort and return a short replacement
sample. The existing package remains an interim production observation.

The result does not identify a data-integrity defect in Memory Engine. It does
identify incomplete build provenance, incomplete physical-attempt telemetry,
insufficiently observed automatic fidelity coverage, missing chat-budget
fields, an unresolved Core scope path and inconsistent vector configuration.

Follow-up: the corrected
[Checkpoint A preflight passed on 2026-08-26](smartest-v2-checkpoint-a-review-2026-08-26.md).
This page retains the historical result for the first package; qualification
cohort `c` is now collecting the replacement sample.

## Reviewed artifact

- File: `smartest-validation-2026-08-24.tar.gz`
- SHA-256: `98ca4b28a3bf6f31192cbb3dedb3e7f5ca030f2f2f8041102173d0187c5c9393`
- Window: 2026-08-24 09:16-20:30 UTC
- Records: 232 provider calls, 9 Sleep runs, 9 reflection snapshots, 54 chat
  context turns and 9 health snapshots
- Usable cohort: `b`, runs `anon_run_002` through `anon_run_009`
- Discarded row: `anon_run_001`, whose exporter read output/pass fields from
  an obsolete shape

The README describes eight usable runs as a shortfall against the requested
approximately 20 runs. The 20-run target belongs to Phase 2. Phase 1 required
one short qualification sample, so volume was sufficient; record completeness
is the reason for `CHANGES_REQUIRED`.

The archive had safe relative member paths. Every JSONL row parsed and carried
the declared schema version. An independent scan of the returned structured
files found no four-character Cyrillic sequence, Google/OpenAI/Telegram key
shape, private-key header or long decimal identifier. IDs were anonymized and
private text was absent. The package itself is not committed to the project.

## Evidence already established

### Compact Sleep input works

Seven clean consolidated cohort-B runs contained 41,061 estimated source-text
tokens and produced 204,266 provider prompt tokens: **4.97x input
amplification**. Across all eight usable runs, including the filtered fallback
run, the values were 43,826 and 213,857 tokens: 4.88x.

This is a large directional improvement from the historical approximately 24x
sample. It is not a controlled percentage claim because Smartest also removed
its own repeated payload and the workloads differ. The new absolute baseline
is reproducible from this package.

Every usable run used one shared-context fingerprint, and every run-linked
request reported that the shared context was first. The seven clean runs each
needed six successful analytical calls. Their per-run amplification ranged
from 4.28x to 6.72x.

### Terminal fallback is observable

Seven usable runs completed through the consolidator. `anon_run_009` hit the
Gemini content filter on four tracks, retained the successful emotional and
relational outputs and finished as `fallback_from_tracks`. No Sleep run
remained pending. The archive was terminal and explainable, while memory
quality was degraded: it contained no MemoryUnits, and the subsequent repair
call was also filtered.

This is positive evidence for completion from available work. It also shows
why provider-attempt accounting and local quality review are required.

### Structured chat geometry works

All 54 chat records used `memory_prompt_plan.v1`, preserved role order and
contained the current message exactly once. The host payload event count
matched the dialogue event count on every row. This establishes the structural
part of H6.

### The package exposes live bottlenecks

The eight usable Sleep runs produced 21 MemoryUnits, 3 personal signals, 1
automatic Core creation and 2 review candidates. Active units grew from 41 to
62, complete archives from 57 to 86 and Core facts from 7 to 8 during the
returned health window. These counts prove that the telemetry sees real state
changes rather than a decorative fixture.

## Findings that block qualification

### 1. The loaded engine revision is not proven

`smartest_validation_build.json` records the extension SHA-256 but sets
`engine_commit` to `unknown`. The stated host revision range does not establish
the source ancestry of the Linux extension.

The build pipeline must emit an immutable mapping from extension hash to
Memory Engine commit. A server checkout is unnecessary; the producing CI job
already knows the source revision. The next sample must contain both values
and prove that the extension descends from the required acceptance commit.

### 2. Provider retries are not recorded per physical request

Run 9 reports two engine attempts for each failed pass. The provider ledger
contains two rows with `attempt: 3` and the same request ID for those attempts.
This means an inner provider retry loop made up to three physical requests but
only exported its final failure. Physical request count, latency and possible
billing cannot be reconstructed.

The next ledger needs separate fields for the engine pass attempt and provider
transport attempt, plus a unique physical-attempt ID. It must write one row
immediately after every actual provider request. Fidelity, reflection and
repair calls created by a Sleep run also need the originating run ID; four such
records currently have `run_id: null`.

One unlinked `reflection_analyze` provider call is present while every returned
reflection snapshot says `task_started: false`. The snapshots have no
timestamp, so the package cannot establish whether this was a later manual
call, an exporter omission or an ordering error.

### 3. Automatic fidelity coverage is too low and its configuration is opaque

The host executed the fidelity requests returned by Sleep. Memory Engine
auto-routed only one request across 21 new units in the usable runs. Unchecked
units therefore grew to 27 while only 5-6 units were valid in the observed
scopes. Reflection had at most two eligible units and never became due.

The package's anomaly note calls the absence of manual
`begin_memory_fidelity_pass` calls a host gap. That diagnosis conflicts with
the library boundary: the host executes requests, while the core selects units
through `auto_validate_weight_threshold`, risk tags and the Core path. The
manual method remains an operator/debug path and should not become a host-side
unit selector.

Before the next cohort, Smartest must export the actual automatic-fidelity
settings and text-free weight histograms for all new units and the units the
core selected. Smartest and the owner then choose one fixed threshold
for the cohort, with the expected provider cost stated. If a hard per-Sleep cap
is required, it belongs in a core-owned routing option rather than host logic.
Capture the reflection snapshot after all fidelity requests returned by the
engine are completed, with a timestamp and origin run ID.

H5 remains unavailable until a new cohort measures a declared automatic
coverage configuration.

### 4. Chat budget telemetry is missing

All 54 chat rows set `estimated_memory_tokens`, `dropped_by_budget` and
`budget_exceeded` to `null`. The current engine always returns a budget report
from `core_context_package`, so the host exporter is reading or mapping the
package incorrectly.

The next sample must export the actual budget report. It must also retain the
request's `include_core` value and anonymized fingerprints for the requested
Core scope and Session personality scope.

### 5. Core exists in storage and is absent from every chat plan

Health snapshots contain 7-8 Core facts, and one usable Sleep run reports a
new Core creation. Every one of the 54 chat plans contains zero Core facts.
This may be valid scope isolation, a request with Core disabled, a scope
mismatch or an exporter error. The present fields cannot distinguish them.

Smartest should locally compare the requested Core scope, Session personality
scope and package notes. Export only HMAC fingerprints, fixed note categories
and counts. A chat with available facts in the requested scope must either
contain those facts or carry an explicit engine reason.

### 6. Vector configuration and pending work disagree

The build record sets `vectors_enabled` to `null`, the README says vectors are
disabled, and health shows one chat scope in `building` state throughout the
window. Pending tasks grow from 2 to 6 and inspection identifies them as
`compute_embedding` work. Memory Engine emits those tasks only when the global
feature and a scope catalog are enabled.

Smartest must choose one state for the next cohort. It may execute embedding
tasks for an enabled scope, or run the validation scope with vectors disabled.
Health telemetry must report the global switch, per-scope state and pending
task counts grouped by type. Existing pending tasks must be resolved through
the public engine boundary or explicitly excluded as pre-cohort work; runtime
files must not be edited directly.

### 7. Restart evidence is not joinable

The README states that the bot was redeployed several times. Health rows lack
a process-instance ID and snapshot reason, so H7 cannot connect startup,
recovery and later terminal state.

Add a random anonymized process-instance ID and `startup|post_sleep|daily`
snapshot reason. Include collection-warning count, oldest pending-work age and
pending counts by task type. Mark health counts as global or scope-specific.

### 8. The cache finding needs a narrower statement

Memory-related Gemini calls had zero cached input tokens. Ten `paint_prompt`
calls through other providers reported 9,280 cached tokens, so the exported
cache field is functional and the package-wide statement "zero on every
workflow" is inaccurate.

Google documents implicit caching for Gemini 3.7 Flash with a 4,096-token
minimum, common content first, and no guaranteed hit. The engine estimator puts
the shared contexts near or above that boundary, but it is not the provider's
token count. Smartest should retain the protected raw Gemini usage metadata,
record the exact API surface, and measure the shared prefix with provider token
counting before choosing implicit or explicit caching.

References:

- [Google Gemini context caching](https://ai.google.dev/gemini-api/docs/caching)
- [Google Gemini token accounting](https://ai.google.dev/gemini-api/docs/tokens)

## Interim hypothesis assessment

| Hypothesis | Interim result | Evidence boundary |
|---|---|---|
| H1 shared-prefix economy | `PARTIAL` | Compact input improved the absolute amplification to 4.97x on seven clean runs. Gemini memory cache hits were zero and cache eligibility is unresolved. |
| H2 budget and recovery | `PARTIAL` | Eight usable runs reached terminal state and fallback preserved successful tracks. Physical provider attempts are undercounted. |
| H3 memory quality | `INSUFFICIENT_DATA` | Outputs exist, but the required local MemoryUnit/reply review and economy replay were not part of this sample. |
| H4 participant attribution | `INSUFFICIENT_DATA` | Core and review counts exist, but no local subject-quality labels were returned. |
| H5 reflection funnel | `BLOCKED_BY_CONFIGURATION` | Automatic fidelity selected 1 of 21 new units; the threshold and routing distribution were not exported well enough to calibrate coverage. |
| H6 structured chat context | `PARTIAL` | Dialogue geometry passes 54/54. Budget fields are absent and Core visibility is unresolved. |
| H7 long-run recovery | `PARTIAL` | No Sleep run remained pending. Vector tasks accumulated and restart instances cannot be joined. |

No v0.5 live-gate or v0.6 reasoning decision follows from this interim sample.

## Corrected Phase 1 handoff

Start a new cohort after the instrumentation and host-policy changes. Return a
small replacement sample with one to three natural terminal Sleep runs; the
multi-day minimum is still Phase 2 work.

Use the `v2` study-export schemas in the updated protocol and keep the existing
artifact file names.

The replacement sample must demonstrate:

1. known engine commit plus loaded extension hash;
2. one row per physical provider request with engine/provider attempt numbers;
3. origin-run joins for Sleep, fidelity, repair and reflection;
4. declared core-owned automatic-fidelity settings, weight/selection counts and a
   timestamped reflection snapshot after returned requests complete;
5. populated chat budget fields and explicit Core scope diagnostics;
6. explicit vector configuration and pending tasks grouped by type;
7. process-instance and snapshot-reason fields for restart evidence;
8. the same privacy scanner and anonymization boundary as this package.

When these records are complete and internally consistent, Phase 1 becomes
`QUALIFIED` and the unchanged configuration may continue directly into the
formal Phase 2 window.

Implementation and return steps are specified in the
[Phase 1 correction TZ](smartest-v2-phase1-correction-tz-2026-08-25.md).
