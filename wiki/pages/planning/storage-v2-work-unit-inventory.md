# Storage v2 Work-Unit Inventory

## Purpose

Status: **S2 inventory complete; transactional foundation and ingest slice implemented; S2 exit gate open**.

This page is the migration ledger between the method-by-method local `Storage`
trait and the vault-bound transactional storage required by the server runtime.
It prevents a PostgreSQL adapter from preserving today’s multi-record races
behind different method names. Every state-changing engine flow must receive an
explicit transaction boundary or an explicit durable-workflow boundary before
S2 can close.

## Related pages

- [Server-scale Memory Runtime](server-scale-memory-runtime.md) - normative
  S0-S7 plan and production invariants.
- [Roadmap](roadmap.md) - phase status and release gates.
- [Architecture](../foundation/architecture.md) - current local and server
  deployment forms.
- [Contracts](../foundation/contracts.md) - versioned Storage v2 envelopes.
- [Strategy](../foundation/strategy.md) - one reusable cognitive core and human
  control over memory.

## The unit we are protecting

A transaction is not “all work performed for one chat turn”. It is the smallest
domain transition that must become visible as one fact. Long provider work is a
durable workflow composed of several such transactions:

```mermaid
flowchart LR
    A["commit frozen intent and input revisions"] --> B["release transaction"]
    B --> C["LLM or embedding work"]
    C --> D["open new work unit"]
    D --> E["revalidate frozen input"]
    E --> F["commit result and terminal checkpoint together"]
```

This preserves the project’s existing distinction between deterministic Rust
policy and host/provider execution. Storage v2 does not create another memory
engine or another workflow orchestrator.

## Current call-chain inventory

| Domain path | Current records and state | Class | Required S2 boundary |
|---|---|---|---|
| Context package, ordinary recall, deep recall, graph export, candidate/task lists | Session, Core, Archive, MemoryUnit, vector and graph reads | Read-only query | Vault-bound point/index reads from one committed view; no mutation hidden in the query |
| Recall usage statistics | Replica-local deltas followed by Archive read/modify/write | Correctness-affecting mutation disguised as telemetry | Either durable atomic counters or explicitly lossy metrics removed from scoring; replica-local state may not alter ranking |
| Ingest | Event append, Session metadata and local Markdown projection | Transactional mutation | Event record and Session metadata commit together; human-readable projections are derived outside the source-of-truth commit |
| Core upsert/patch | Core category read/modify/write | Transactional mutation | Exact revision check and one category replacement in one work unit |
| Sleep Stage 1 | Preliminary Archive plus compression/unit tasks | Durable workflow start | Frozen source coverage, Archive intent and PendingTasks commit together |
| Sleep pass submit/finish | SleepRun, PendingTask, Archive, MemoryUnits and Session coverage | Durable workflow continuation | Provider call outside storage; each submit revalidates the run/task/input revisions and atomically advances all affected records |
| Fidelity review | PendingTask, MemoryUnit and Archive fidelity projection | Durable workflow | Begin commits task/input identity; submit atomically applies review and terminal task state |
| Forgetting and remember-back | Review task, units, Archive projection and vector tombstone/rebuild intent | Durable workflow | Decision and domain state commit together; vector work remains derived and replayable |
| Reflection analyze/apply/review | Reflection state, task, operation journal, candidates/nodes/evidence, units, archives and optional Core promotion | Durable workflow | Existing frozen intent/recovery semantics become transactional commit steps; no partial candidate/Core/review transition |
| Hypothesis graph migration | Legacy candidates, nodes, evidence, manifest and journal operation | Durable migration workflow | Idempotent batches with frozen scope, exact source revisions and atomic checkpoint/output commit |
| Vector backfill/rebuild | Vector manifest/index rows, tombstones and embed tasks | Durable derived-index workflow | Task/scope state is durable; embedding is outside transactions; structural memory remains authoritative |
| Manifest and isolated task/run checkpoints | One versioned record when no related domain write exists | Atomic single-record mutation | Exact revision write; fold into a wider work unit whenever it represents the same transition |

The inventory’s important result is that `Storage::append_event` was never a
true single-record operation. `FileStorage` appends JSONL, then updates Session
metadata, then updates Markdown. The new server path treats only the event and
Session metadata as authoritative; Markdown is a projection and cannot make the
commit fail halfway.

## Implemented S2 foundation

`storage_v2.rs` now defines an additive, backend-independent boundary:

- a work unit is born from a validated, server-derived `VaultContext` and can
  address only `RecordKey { collection, record_id }`; callers cannot smuggle a
  second vault into a record key;
- every persisted value has a `VaultRecord` envelope with vault, payload schema,
  monotonic revision and timestamps;
- writes require `ExpectedRevision::Absent` or `Exact(n)`, and all observed
  revisions are checked again at commit;
- `WorkUnitIntent` provides operation identity, kind, stable request digest and
  start time; a committed `WorkUnitReceipt` makes a same-input replay return the
  original result while conflicting reuse fails closed;
- `execute_work_unit_with_retry` restarts the entire deterministic closure on
  optimistic conflict. Storage/provider failures are not silently retried as
  conflicts;
- the in-memory reference builds the complete next vault state and publishes it
  once. Its failpoint proves that a failure immediately before publication
  leaves no records and no receipt.

The first migrated domain transition is `MemoryEngine::ingest_transactional`.
It derives a stable event ID from vault and operation identity, applies the same
event scoring policy as local ingest, and atomically creates the event record
and creates/updates Session metadata. Concurrent distinct operations against
one Session retry on revision conflict; a duplicate operation cannot increment
`event_count` twice.

This is a reference and conformance surface, not the production backend.
`FileStorage` and `MemoryEngine::ingest` remain unchanged for existing local,
Python and Godot hosts. No host has been switched automatically, and PostgreSQL
does not begin until S2’s domain migration gate closes.

## Migration sequence from here

### S2.A — Foundation and ingest (implemented, acceptance pending)

The code boundary and in-memory reference exist. Deterministic tests cover
atomic failure, idempotent replay, conflicting replay, concurrent same-session
ingest, optimistic rollback and vault isolation. Before this subphase is called
accepted, an external host-path test must verify the public serialization shape
and retry behavior from the built adapter that will actually call it.

### S2.B — Correctness state and small mutations

Next, move Core upsert/patch, manifest/task state transitions and Session
archive coverage. In the same subphase, remove the ambiguity around recall
statistics: either persist the counters transactionally because they influence
ranking, or stop using replica-local deltas as cognitive input and retain them
only as explicitly lossy telemetry.

### S2.C — Sleep, fidelity and forgetting

Express these as durable workflows whose short begin/submit/finalize commits
use Storage v2. Freeze source IDs and revisions before provider work, then
revalidate them on submit. Fault injection must interrupt every commit edge and
show either the old complete state or the new complete state, never a hybrid.

### S2.D — Reflection, review, hypothesis migration and vectors

Port the existing journaled reflection and graph recovery rules without
creating a competing transaction journal. Graph evidence and its terminal task
checkpoint commit together. Vector rows remain rebuildable derived data, while
their task and scope state become durable and idempotent.

### S2 exit and handoff to S3

S2 closes only when ingest, sleep completion, reflection apply/review,
forgetting, hypothesis migration and task transitions all pass the same
fault/retry suite on the reference backend, no provider call is inside a work
unit, and Storage v1 compatibility is verified. S3 may then implement these
contracts in PostgreSQL with transaction-local RLS; it must not reinterpret
their semantics.

## Known limits and deliberate non-goals

The in-memory backend snapshots a vault for deterministic conformance. It is not
a capacity model and does not claim cross-process durability. Membership
authentication remains the server runtime’s responsibility; Storage v2 binds a
seen vault to the authorized account/membership and rejects a later mismatch,
but it is not the identity registry. The retry closure cannot technically prove
that a caller avoided network I/O, so code review and workflow APIs retain that
architectural gate.

Adding public Storage v2 types, `ingest_transactional`, and a separate
non-exhaustive `StorageV2Error` is additive functionality planned for a future
pre-1.0 minor release. The existing public `MemoryEngineError` stays unchanged
so exhaustive downstream matches are not broken. Project package versions
remain unchanged until adapter and release acceptance gates pass.
