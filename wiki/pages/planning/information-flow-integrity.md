# Information Flow Integrity Plan

## Related pages

- [Strategy](../foundation/strategy.md) - invariants this repair preserves.
- [Architecture](../foundation/architecture.md) - implemented information and
  storage boundaries.
- [Contracts](../foundation/contracts.md) - canonical schemas used along the
  event-to-prompt path.
- [Roadmap](roadmap.md) - implementation status and remaining server phases.
- [Server-scale Memory Runtime](server-scale-memory-runtime.md) - Storage v2
  migration and production acceptance track.

Status: **architectural repair implemented in the local/reference path;
server lifecycle migration continues through S2.C–S2.D**

This plan is the answer to the end-to-end information-flow audit. It does not
add a fourth memory system and it does not move cognition into Telegram,
Godot, or a backend. It makes the existing three-layer memory keep one meaning
as information moves from a host event to Session, Sleep, atomic MemoryUnit,
Archive/Core retrieval, the final context plan, and the model prompt.

## Strategic decision

Memory Engine keeps three durable cognitive layers: the live Session, the
episodic Archive, and the stable Core. Vectors are not another source of truth
and not another durable layer. They are an optional, rebuildable associative
index over eligible atomic MemoryUnits. When vectors are disabled, Session,
Archive, Core, structural recall, Sleep, fidelity, reflection, and forgetting
must remain complete. When vectors are enabled, they may activate an older
detail for the current topic, but they may not rewrite that detail, carry it
into the next topic, or bypass scope/fidelity/forgetting policy.

The unit of personality continuity is not an interaction session. A Session
is one live conversation or gameplay interval; `personality_scope_id` is the
long-lived memory identity that may contain many such Sessions. Account/vault
authorization remains the security boundary. The hierarchy is therefore:

```text
account membership -> vault -> personality scope -> interaction sessions
                                      |                    |
                                      |                    +-> live events
                                      +-> Archive / Core / optional vector index
```

No API is allowed to replace a missing identity with a shared global user.
Legacy records without `personality_scope_id` use their own `session_id` as a
private fallback; that preserves old isolation without turning old data into a
wildcard.

## Canonical information path

### 1. Admission: the host states what is memorable

The host sends a versioned event with two deliberately separate payloads.
`payload` is lossless domain data owned by the host. `memory_text` is the
host-approved textual projection that memory policy, Sleep, recall, and
evidence may interpret. Chat hosts normally put the message text in both;
game hosts can keep rich state in `payload` while exposing only the narrative
meaning in `memory_text`.

The engine validates event identity and RFC3339 time, binds the event to one
Session and one stable personality scope, and stores both forms. An existing
Session cannot silently change scope. This removes the old implicit contract
that every meaningful payload happens to contain `payload.text`.

### 2. Consolidation: Sleep freezes the complete source projection

Sleep chooses a bounded event window using one shared policy for FileStorage
and Storage v2. Every reasoning task receives the same versioned
`sleep_input_event.v1` projection: identity, source, client and receive time,
Session and personality scope, `memory_text`, payload, tags, theme, affect,
speaker, links, importance, processing mode, weight, and weight reason.

The model is not allowed to cite an event outside that frozen input. A result
with foreign provenance fails closed. The preliminary Archive derives its
theme and content from the exact selected events, not from stale Session
metadata. Conversational `reply_to` edges help interpret the source but do not
leak into lifelong Archive links as if they were semantic relationships.

### 3. Durable meaning: MemoryUnit is the canonical prompt truth

Archive is the episode and provenance container; MemoryUnit is the atomic
meaning. Once standalone MemoryUnits exist for an Archive, recall and prompt
projection hydrate and use those canonical records. They do not fall back to
an old embedded copy or the parent Archive gist merely because that text is
convenient.

Only active/core-candidate units with prompt-eligible fidelity state can enter
recall. Forgotten, rejected, distorted, unsupported, or revision-pending
units remain auditable on disk but cannot leak through cached parent text.
This makes fidelity and forgetting real state transitions rather than labels
that one retrieval path can ignore.

### 4. Retrieval: structure chooses; optional association remembers

Ordinary context assembly uses deterministic structural Archive recall.
`personality_scope_id` allows continuity across interaction Sessions of the
same person while preserving exact vault isolation. Explicit Stage 2 recall
may re-rank structural candidates with MemoryUnit vectors. Separately, the
current-turn embedding may activate a small `associative_memory` block from
deep recall. That block is rebuilt for every turn and is marked as older
memory, so switching from Mars to dinosaurs removes Mars unless the new turn
itself activates it.

Both vector paths use the same personality scope as structural recall. A
similarity threshold may return no association; “nearest” is not synonymous
with “relevant.” With no embedding or disabled vector scope, the complete
non-vector path remains available.

### 5. Context plan: one final decision before the prompt

`CoreContextPackage` is the canonical context plan. It brings together stable
Core facts, the active Session tail, structural Archive memories, optional
current-topic associations, and transient `domain_state`. Budgeting and
cross-layer deduplication happen before the plan is returned. Recall feedback
is recorded only for Archive records that survive that final plan, once per
Archive even if an atomic detail was found through more than one route.

`domain_state` is rendered as explicit present state under
`current_state_json`; it is neither stored as memory nor described to the
model as something the user said in the past. The shared renderer, not each
host, owns the separation between Core, long memory, associative older memory,
short memory, present state, and the current user turn.

## Delivery phases

### Phase I — Repair semantic continuity (implemented)

The event contract now carries `memory_text` and stable personality scope.
Sleep uses the full shared input projection, validates source provenance, and
derives the preliminary episode from the selected events. Session, Archive,
MemoryUnit, recall, vector backfill, and current-topic association carry the
same scope with a legacy exact-session fallback.

### Phase II — Enforce one prompt truth (implemented)

FileStorage hydrates Archives from standalone MemoryUnit records. Recall text,
Stage 1 eligibility, Stage 2 projection, and prompt projection obey one
eligibility policy. Present domain state is rendered explicitly. Context
feedback moved behind budget and deduplication.

### Phase III — Make server reads match domain boundaries (implemented in the
in-memory Storage v2 reference)

Storage v2 now exposes typed snapshot queries for one Session’s events and for
scope/session-bounded Archives. The reference backend tracks membership and
revisions for the exact query, retaining phantom protection without requiring
a full-vault hot-path scan. Production PostgreSQL must map these contracts to
bounded composite indexes and prove query plans.

### Phase IV — Finish the transactional Sleep lifecycle (implemented in the
in-memory reference; external acceptance pending)

Sleep is now one restartable state machine rather than a start transaction
followed by best-effort local writes. `sleep_workflow_input.v1` owns the only
durable copy of the selected private events, frozen policy and preliminary
Archive digest. Tasks and SleepRun persist typed references; the provider batch
is rehydrated only when a worker asks for it. Submit freezes the current run,
records task attempts, advances extraction/consolidation and stores an exact
replay result in one commit. Provider execution remains outside transactions.

Finalize is the sole transition allowed to publish a Complete Archive,
deterministic canonical MemoryUnits and Session coverage. A live turn admitted
after Sleep started is preserved for the next cycle. Cancel marks preliminary
work Superseded and clears the Session reservation without hiding any source
event. Every terminal or continuation operation revalidates the immutable
input, Archive digest, task contract and run topology.

Transactional ingest also assigns a monotonic per-Session
`admission_sequence`; Sleep uses it before the legacy receive-time/event-ID
fallback. Serialized input/response limits and a MemoryUnit-count cap bound the
reference work unit before PostgreSQL handoff.

### Phase V — Port the remaining cognition and derived indexes

Fidelity, forgetting, reflection/review, hypothesis graph changes, vector task
state, and vector-scope lifecycle must pass the same idempotent transactional
gate. Vector rows remain rebuildable derived data; their scope/task state is
durable. No host switches to Storage v2 until its required flows have parity
with the compatible FileStorage path.

### Phase VI — Prove the production runtime

PostgreSQL/pgvector, transaction-local RLS, authenticated account-to-vault
resolution, durable workers, portable export/restore, multi-process failure
tests, and whole-turn load tests close production readiness. Passing the
in-memory reference proves semantics only; it does not prove WAL, network,
pool, or thousands-of-users behavior.

## Acceptance invariants

An implementation is accepted only if one admitted fact can be followed from
the host event to its frozen Sleep source, MemoryUnit provenance, eligible
recall result, final context plan, and rendered layer without changing scope or
meaning. A rejected or forgotten unit must disappear from every prompt path.
A topic switch must rebuild associations. Disabling vectors must preserve the
three durable layers. A concurrent or replayed server operation must either
publish the whole transition once or publish none of it. Debug output may
explain IDs, scores, thresholds, and stages, but persistent logs must not copy
private memory text.

## Remaining known boundaries

The compatible Python/Godot APIs still use FileStorage; the new fields are
already populated by repository hosts, but there is no Storage v2 host cutover.
External acceptance of transactional Sleep, post-finalize Core/derived work,
fidelity/forgetting, Storage v2 vector workflows, and PostgreSQL remain open.
Finalize зберігає versioned `post_finalize` pending-state, тому ці гілки не
губляться за порожнім compatibility outcome.
These are planned architectural phases. They are not hidden behind a claim
that the current reference is production-ready.
