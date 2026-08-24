# Autonomous Reflection v0.5

## Related pages

- [Strategy](../foundation/strategy.md) - why reflection exists in the living-memory model.
- [Architecture](../foundation/architecture.md) - Core/Session/Archive boundaries and PendingTask orchestration.
- [Contracts](../foundation/contracts.md) - public shapes and persisted reflection state.
- [Roadmap](roadmap.md) - release ordering and remaining phases.
- [Smartest v2 Production Validation](smartest-v2-production-validation-2026-08.md)
  - current live-calibration and reporting gate on large group chats.
- [Smartest Phase 1 Review 2026-08-25](smartest-v2-phase1-review-2026-08-25.md)
  - first production package, automatic-fidelity finding and correction handoff.
- [Reflective Belief System](reflective-belief-system.md) - post-v0.5 plan for
  reliable reflection, hierarchical hypotheses, contradiction, and native
  clarification.
- [Generative Agents Memory Stream review](../research/generative-agents-memory-stream-2026-07-06.md) - external pattern that surfaced the missing trigger.

## Purpose

This document is the implementation authority for v0.5 autonomous reflection.
The feature turns the existing manual `reflection_analyze` pass into a durable,
core-owned cycle that decides when enough validated experience exists to be
worth interpreting.

The objective is not another memory layer and not a permanently running agent.
It is the controlled transition from episodic Archive memory toward a stable
understanding in Core:

```text
Session events
    -> sleep / MemoryUnit extraction
    -> fidelity validation
    -> accumulated salience
    -> reflection
    -> reviewable CandidateBelief
    -> owner review
    -> Core
```

**Integrity closure update 2026-08-01.** The audit reopened acceptance after it
found non-recoverable multi-file apply/review, wildcard-like absent scope and
unsafe review transitions. Phase C.1 now closes those deterministic defects:
reflection apply and owner review are journal-backed and replayable, absent
scope is isolated, evidence is revalidated before the first side effect, and
review cannot change scope or revisit a terminal candidate. At the time of
that implementation, live acceptance remained open; the current host assignment
is recorded below.

**Live-host update 2026-08-24.** The owner paused Chibigochi product work and
assigned the open v0.5 live gate to Smartest v2. The deterministic reflection
contract is unchanged. Smartest's large production-shaped group chats now
provide the active → valid → eligible funnel, repeated evidence, attribution
pressure, provider failures and restart evidence required by the gate. The
redacted return format is normative in
[Smartest v2 Production Validation](smartest-v2-production-validation-2026-08.md).

**Instrumentation update 2026-08-25.** The first Smartest package showed that
automatic fidelity routed only one request across 21 new units. The host
correctly executed the returned work; it must not select additional units
through the manual API. Phase 1 is `CHANGES_REQUIRED`; the replacement cohort
records the core-owned routing threshold, text-free weight/selection counts and
reflection status after returned validations complete. Thresholds change only
as an explicit cohort configuration decision.

## Strategic Decision

Reflection remains a specialized reasoning pass executed through the existing
`PendingTask` boundary. Rust owns eligibility, trigger policy, bounded inputs,
checkpointing, deduplication, and application of results. The host owns only
the provider call and scheduling opportunity. The LLM may propose meaning; it
cannot write truth directly into Core.

Autonomous reflection works without embeddings. When vector memory is enabled,
future phases may use it to find non-obvious older supporting units, but vectors
remain a derived sidecar and never become a fourth source of truth.

## Trigger Semantics

Only `MemoryUnit` records with `status=active_archive` and
`fidelity_status=valid` contribute. Raw events, unchecked compression, rejected
units, and forgotten units do not.

Three conditions can make reflection due:

1. A single unit reaches the critical-memory boundary.
2. The sum of new validated unit weights reaches accumulated critical mass.
3. At a sleep or scheduled-maintenance opportunity, a maximum interval has
   elapsed and at least a minimum amount of new validated evidence exists.

The third condition is a safety net for slow patterns, not a daily forced LLM
call. No eligible evidence means no task. A successful zero-candidate result is
still a successful reflection and advances the checkpoint. A failed task does
not consume its input units.

Initial defaults are implementation hypotheses, not universal psychology:

- accumulated salience `2.5`;
- critical unit weight `0.95`;
- maximum interval `7 days`;
- minimum interval evidence `2 units`;
- maximum automatic input `40 units`.
- maximum related history `20 units`;
- minimum related-history structural score `2`;
- maximum existing candidate context `20 candidates`.

These values are explicit `ReflectionConfig` fields and require live
calibration before v0.5 acceptance.

## Sleep and Time Relationship

Sleep and reflection remain different cognitive operations. Sleep answers
"what happened" by turning live events into durable units. Reflection answers
"what does this accumulated experience mean" only after those units are
validated.

Therefore reflection does not run inside raw sleep compression. The host calls
`begin_reflection_if_due` after the sleep/fidelity pipeline reaches a safe
point, or from scheduled maintenance. The call is not a host-side decision:
the core returns a structured status and creates a reasoning task only when its
policy says the scope is due.

Exceptionally important raw experience still follows the same trust path. It
may cause an earlier partial sleep, then cross the critical reflection boundary
after fidelity; it is never interpreted into personality directly from an
unvalidated chat line.

## Durable State and Recovery

`runs/reflection/<scope>.json` stores processing state, not memory truth. It
records the last completed reflection, last task, last trigger reason, and
candidate count, plus source sessions explicitly bound to that personality
scope. Each processed `MemoryUnit` records `reflection_count`,
`last_reflected_at`, and the deterministic operation ID that applied the last
checkpoint.

The storage boundary rejects an unsupported reflection-state schema and a
stored scope that does not match the requested scope. Recovery must fail
visibly rather than silently applying another personality's checkpoint.

An active reflection task is unique per reflection scope. The scope lock and
task state prevent ordinary duplicate task creation, and a provider/parse
failure before apply leaves unit markers unchanged. Once a result passes live
evidence revalidation, the core first persists a frozen `reflection_apply`
intent. Recovery then reasserts candidate/reinforcement, unit/archive
checkpoint, reflection state and task outcome idempotently without another LLM
call. The first ordinary core call on a new engine instance performs recovery
once before serving data; thin adapters also expose an explicit recovery
method for startup diagnostics.

Persistent telemetry and trigger status contain identifiers, counts, weights,
times, and reasons, but no query text or memory text.

## Implementation Phases

### Phase A - Durable trigger cycle

Status: implemented, deterministic tests green; live acceptance pending.

Phase A provides `ReflectionConfig`, `reflection_status`,
`begin_reflection_if_due`, per-scope state, per-unit reflection markers,
critical-mass and interval triggers, bounded automatic input, pending-task
deduplication, failure-safe checkpoints, Python/Godot adapter access, and an
opt-in Telegram after-sleep executor.

The Telegram demo enables execution only with
`MEMORY_BOT_AUTO_REFLECTION=1`. Capability is first-class in the library, while
automatic provider spending remains an explicit host-owner choice.

### Phase B - Personality-scope evidence

Status: implemented, deterministic tests green; live acceptance pending.

`core_scope` is the explicit personality boundary. Reflection state additively
records sessions presented under that scope by `begin_reflection_if_due` or
manual reflection. `bind_reflection_session` exposes the same durable binding
without creating a task, so hosts can register lifecycle/history explicitly.
It never infers identity linkage from similar text. Trigger
eligibility and salience now span the bound sessions while every source session
remains provenance.

The reflection input contains:

- new validated units that caused the trigger, labeled `trigger`;
- a bounded set of already-reflected related units, labeled
  `related_history`;
- active and contested Core facts;
- existing draft/ready candidates for duplicate prevention and reinforcement.

The non-vector selector is implemented with explicit structural weights:
shared link `4`, archive theme `3`, topic/subtopic `2`, and unit/archive tag
`1`. A candidate needs score `2`, so one generic shared tag cannot activate
history by itself. Initial limits are 20 related units and 20 existing
candidates. Optional vectors may later enrich selection but cannot establish
scope membership or replace this baseline.

Candidate submit accepts provenance only from units, archives, and Core facts
actually present in the task. `CandidateBelief.source_session_ids` records the
cross-session evidence set compatibly. The reasoning task carries only session
IDs represented by selected units plus a total personality-session count; the
full durable membership is not copied into every LLM request.

### Phase C - Meaningful result actions

Status: implemented; deterministic functional and Phase C.1 integrity gates
green; live acceptance pending.

Reflection must distinguish four outcomes: propose a new belief, reinforce an
existing candidate, challenge an existing Core fact through the contested
path, or produce no durable insight. Repeated evidence must strengthen one
traceable hypothesis instead of creating duplicate candidate text.

New tasks use `reflection_actions_result.v1` and prompt version 3. The typed
actions are:

- `new_candidate` creates one reviewable hypothesis from task evidence;
- `reinforce_candidate` attaches at least one previously unused task unit to
  an existing draft/ready candidate, records a durable reinforcement event,
  merges provenance, and may raise but never lower confidence;
- `challenge_core_fact` creates a `core_challenge` candidate that cites the
  conflicting Core fact but leaves that fact active until owner approval;
- `no_insight` records a machine-readable reason and explanation when the pass
  should not change long-term understanding.

Exactly one of non-empty `actions` or `no_insight` is valid. Unknown candidate
IDs, evidence outside the task, reused-only reinforcement evidence, duplicate
actions, and malformed challenges fail closed and do not advance the trigger
checkpoint. Saved tasks and older hosts that return `reflection_result.v1`
remain accepted; their candidates are applied through the legacy compatible
path.

Core still changes only through explicit review. Auto-confirm is outside v0.5.

The current candidate context is a bounded recency slice. It is sufficient for
the Phase C functional path but does not guarantee semantic continuity once a
scope contains more candidates than the limit. The previous claim that every
repeated meaning must already strengthen exactly one hypothesis is narrowed:
Phase C guarantees reinforcement only when the intended candidate is in the
task and selected by the model. Trigger-related graph retrieval and multiple
levels of generalization belong to the
[Reflective Belief System](reflective-belief-system.md), not to a hotfix in
this schema.

### Phase C.1 - Integrity and recovery closure

Status: implemented and deterministic fault-boundary tests green; live v0.5
acceptance pending.

Phase C.1 does not add another cognitive action. It makes the existing actions
safe enough to become a release contract.

`core_scope` becomes an immutable operation boundary. Missing scope means an
isolated unscoped namespace, never a wildcard. Candidate review cannot replace
the candidate scope, and only an explicitly reviewable status may enter the
promotion/rejection transition.

Typed apply and owner review become journal-backed, idempotent operations with
deterministic IDs and frozen intent. Before the first side effect the core
revalidates every cited MemoryUnit, candidate, and Core fact. Recovery resumes
the same operation without another LLM call or duplicate reinforcement. The
completed task retains a text-free outcome summary and result digest.

The concrete design, dependency order, and fault-injection gate are defined in
[Reflective Belief System, Phase 1](reflective-belief-system.md#phase-1--reliable-reflection-foundation-v05-closure).

The implementation uses `JournalOperationType::ReflectionApply` and
`CandidateReview`. A completed reflection task stores a text-free
`reflection_outcome.v1` with operation ID, result digest, candidate IDs and an
optional structured no-insight reason. Deterministic tests replay the frozen
operation at the candidate, checkpoint, task and journal boundaries, verify
reinforcement deduplication, recover review after its Core write, reject stale
evidence, and cover unscoped/named-scope isolation. These tests establish
restart integrity, not product-quality calibration.

## Acceptance

Deterministic acceptance must prove:

- below-threshold memory creates no task;
- one critical unit or accumulated salience creates exactly one task;
- interval reflection requires both elapsed time and new evidence;
- restart before result application resumes the active task without consuming
  units;
- restart at every Phase C.1 multi-file write boundary completes the same
  journaled operation without duplicate candidate, reinforcement, checkpoint,
  review, or Core transition;
- provider/parse failure leaves units eligible;
- a zero-candidate success advances the checkpoint;
- two explicitly bound sessions accumulate one trigger while an unbound scope
  remains isolated;
- absent `core_scope` sees only unscoped facts/candidates and never behaves as
  a wildcard over named personalities;
- related history requires structural overlap, is separately labeled, and
  obeys its limit;
- only in-scope draft/ready candidates enter the task;
- typed new-candidate output creates one reviewable belief;
- later evidence reinforces the same candidate, preserves one candidate ID,
  records provenance, and never lowers confidence;
- reused-only or out-of-task evidence fails without consuming new units;
- a Core challenge leaves the old fact active until explicit approval, after
  which the ordinary contested lifecycle applies;
- typed no-insight is a successful checkpoint and has a structured reason;
- completed typed apply preserves a text-free durable outcome summary and
  digest;
- review rejects terminal candidates and any attempt to change candidate
  scope;
- saved `reflection_result.v1` tasks still complete after the upgrade;
- non-vector mode has identical trigger and lifecycle behavior;
- no memory text is written to persistent trigger telemetry.

Live production acceptance must span several sleep cycles. A repeated durable
interest should become one evidence-backed candidate; a temporary mood should
remain episodic; repeated support should not create duplicate beliefs; a real
contradiction should enter review without silently rewriting Core.

### Live Phase C procedure

Acceptance runs through Smartest v2 production-shaped owner-memory
orchestration. The report must identify the library commit, loaded release
module path and hash, anonymized `core_scope`/task/candidate/unit IDs,
expected/output schemas, action types, candidate counts before/after and Core
statuses. Persistent exported logs must be checked for absence of memory/query
text. Chibigochi remains a deterministic regression host while product work is
paused.

Use naturally occurring owner evidence. Do not inject a false personality fact
into the owner store merely to satisfy a scenario; when a real Core conflict is
not available, exercise that case on an isolated copy and label it non-owner
data. The required sequence is:

1. Let one stable pattern create a candidate, then let a later sleep cycle add
   independent validated evidence. The second pass must return
   `reinforce_candidate`, preserve the candidate ID and candidate count, append
   exactly one reinforcement, add the later unit/session provenance, and not
   lower confidence.
2. Present a one-off temporary state. The pass must return structured
   `no_insight`; no candidate may be created, while the trigger unit is
   checkpointed once.
3. Exercise a real contradiction when safely available. Reflection must create
   a `core_challenge` candidate while the cited Core fact remains active. On an
   isolated copy, explicit approval must move the old fact through the ordinary
   contested path; reflection submit alone must never do so.
4. Confirm that ordinary recall, associative memory, and non-vector fallback
   still operate. Phase C must not introduce a vector dependency or a new
   prompt-visible memory layer.

Any duplicate effect from replay, reused-only reinforcement, silent Core
mutation, cross-scope evidence, mutable-scope review, invalid terminal-state
review, partial unrecoverable operation, or text-bearing persistent telemetry
is an implementation failure. Model refusal or malformed JSON is a host/model
failure only if the core leaves evidence eligible and records the task failure
visibly.

## Release Boundary

Autonomous reflection is not part of v0.4 recall closure. It adds public
adapter methods, configurable behavior, persisted processing metadata, and a
new lifecycle capability, so it targets the next pre-1.0 minor release:
`v0.5.0`. The hierarchical hypothesis graph is deliberately not pulled into
that boundary; it begins after the Phase C.1 reliability gate and targets the
subsequent minor line described in the
[Reflective Belief System](reflective-belief-system.md).
