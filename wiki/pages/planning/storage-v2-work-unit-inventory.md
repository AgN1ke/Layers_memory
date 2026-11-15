# Storage v2 Work-Unit Inventory

## Purpose

Status: **S2 inventory complete; S2.A and S2.B externally accepted; S2.C
transactional Sleep lifecycle has external logical acceptance at `c57e3cc`;
S2 exit gate open**.

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
| Sleep Stage 1 | Session workflow claim, preliminary Archive, compression/unit tasks, immutable workflow input, initial SleepRun and immutable operation result | Durable workflow start | Frozen policy/source projection/Archive digest, one active Session claim and seven atomic mutations; preliminary selection is not Complete archive coverage |
| Sleep pass submit/finish | SleepRun, PendingTask, immutable input, Archive, MemoryUnits and Session coverage | Durable workflow continuation | Implemented reference: provider call outside storage; submit revalidates frozen topology; finalize atomically publishes deterministic units and Complete coverage; cancel releases sources without coverage |
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
  once. Operation-bound failpoints cover before apply, each staged mutation,
  receipt creation before publish, and publish before acknowledgement. Every
  pre-publish failure leaves no records/receipt; a lost acknowledgement is
  classified as `CommitOutcomeUnknown`, then the executor reopens the same
  operation and returns its committed receipt. A later caller replay returns
  that same result without another mutation.

The first migrated domain transition is `MemoryEngine::ingest_transactional`.
It derives a stable event ID from vault and operation identity, applies the same
event scoring policy as local ingest, and atomically creates the event record
and creates/updates Session metadata. Concurrent distinct operations against
one Session retry on revision conflict; a duplicate operation cannot increment
`event_count` twice.

The next migrated transition is transactional Core upsert/patch. It does not
fork Core policy: local Storage v1 and Storage v2 call one shared deterministic
fact-mutation policy. The server path replaces one category under an exact
record revision and commits a durable operation-result record beside it.
Keeping the result in the same transaction matters because `created` and the
returned fact depend on the state observed by the first attempt; a replay after
later reinforcement must return that original result, not recompute history.
New Core IDs are deterministic from vault and operation identity. Patch carries
the normalized category explicitly, so a server backend performs one point
mutation instead of scanning every category.

Committed-view Stage 1 recall is the other completed S2.B transition. Ranking,
selected-archive feedback and the exact result commit together, so replay
cannot increment counters twice or silently see a newer ranking. The Core and
recall slices passed independent source-level acceptance at `2bdc0ae` and
`7314833`, including concurrency, replay, vault isolation, phantom protection
and Storage v1/adapters regression checks.

S2.C now contains the full transactional Sleep lifecycle reference. It reuses the existing
deterministic Stage 1 selection policy, uses server-controlled `received_at`
for the active-tail boundary and equal-weight selection, freezes the policy and
selected sources, and atomically commits the Session sleep claim, preliminary
Archive, two PendingTasks, initial
SleepRun, immutable workflow input and operation result. Only one workflow may claim a Session
at a time. The preliminary Archive reserves work but does **not** add its
sources to Complete archive coverage; those events remain live until a future
finalize transaction validates provider output and commits completion. The
focused suite now covers the complete workflow, all mutation edges, lost ack,
replay, stale/concurrent workers, immutable dependency rewrites, source
provenance, late live turns, limits and cancellation. The
[recorded external review](client-request-memory-integrity-2026-09-29.md#приймання-яке-не-можна-загубити)
accepted the logical lifecycle at `c57e3cc`. Full S2.C parity remains open.

This is a reference and conformance surface, not the production backend.
`FileStorage` and `MemoryEngine::ingest` remain unchanged for existing local,
Python and Godot hosts. No host has been switched automatically, and PostgreSQL
does not begin until S2’s domain migration gate closes.

## Migration sequence from here

### S2.A — Foundation and ingest (logical reference accepted; host adapter pending)

The code boundary and in-memory reference exist. Ten deterministic tests cover
all logical pre-publish edges, automatic receipt reconciliation after a lost
commit acknowledgement, idempotent and conflicting replay, frozen-input tamper, concurrent same-session ingest,
optimistic rollback, operation-targeted failpoints, invalid-edge cleanup,
versioned serialization and vault isolation. An external source-level
acceptance of `af932a4` independently inspected and passed these assertions.
This closes the library-level logical reference gate. Before this subphase is
called host-ready, the built adapter that will actually call Storage v2 must
still verify the same public serialization and retry behavior.

### S2.B — Correctness state and small mutations

**Core and committed-view recall sub-slices externally accepted.** Transactional
upsert/patch now has frozen request digests, durable results, exact revision
checks, deterministic IDs, scope/vault isolation and the same logical
fault/replay contract as ingest. Seven tests cover same-category contention,
replay after later state change, reinforcement, scope isolation, point patch,
all pre-publish edges, lost acknowledgement and cross-vault separation.
Storage v1 and both existing host adapters remain unchanged. Independent
acceptance of `2bdc0ae` and `7314833` passed both focused suites, the shared
policy audit, workspace tests, clippy, documentation links and adapter
regression checks. This closes the S2.B logical-reference gate only.

The recall-statistics ambiguity is resolved as a design decision: Archive
`recall_count` and `last_recalled_at` are **correctness state**, because Stage 1
reads them to alter ranking and forgetting uses their archive-level proxy.
They are not allowed to remain replica-local on the server path and cannot be
downgraded to telemetry while those policies consume them.

**Committed-view Stage 1 recall is now implemented as the second S2.B
sub-slice.** `recall_transactional` ranks a vault-bound Archive snapshot with
the same Stage 1 policy as Storage v1, increments feedback on exactly the
selected entries and stores the exact `RecallResult` in one work unit. Replay
cannot count twice or return a newly recomputed rank. Snapshot collection reads
track membership and revisions, so an insert/delete/update during the operation
causes whole-unit retry instead of a phantom result. Eight tests cover every
pre-publish mutation edge, lost acknowledgement, tamper, replay, concurrent
feedback, cross-replica ranking visibility, phantom insertion and vault
isolation.

This reference intentionally supports Stage 1 only. A request carrying
`query_embedding` fails clearly until S2.D ports the vector scope/index; it
does not silently degrade a caller that explicitly requested Stage 2. Domain
flows now request typed `Archives`/`SessionEvents` snapshots. The in-memory
reference may implement them by scanning one vault, but tracks phantoms only
for the exact bounded result. S3 must implement the same contracts with indexed
queries and prove query plans; this is not a production scan approval. The
next migration boundary is the rest of the S2.C Sleep workflow, not an
isolated manifest or coverage write.

### S2.C — Sleep, fidelity and forgetting

**Transactional Sleep lifecycle has external logical acceptance at
`c57e3cc`.** Its short begin transaction makes seven changes visible together:
Session workflow claim, preliminary Archive, compression task, MemoryUnit task,
immutable workflow input, initial SleepRun and immutable replay result. It
shares Stage 1 selection with Storage v1 and uses
server receive order for the active-tail partition and equal-weight truncation,
so untrusted client timestamps cannot move new turns or win a selection tie.
The Session keeps a separate pending-event reservation;
`archived_event_ids` and `archived_to` continue to mean coverage by Complete
archives only.

The information-flow repair also makes both Storage paths freeze the complete
versioned `sleep_input_event.v1` projection, including `memory_text`, payload,
personality scope, receive provenance and policy fields. A typed SessionEvents
snapshot means a malformed/revised event in another Session cannot poison the
target workflow; a malformed target event still fails closed.

Submit, finalize and cancel/recovery are now implemented without wrapping the
sequential Storage v1 helpers. Provider responses and current run digest are
frozen in the operation; consolidator/attempt/MemoryUnit IDs are deterministic;
every continuation revalidates immutable input, preliminary Archive digest,
task contract, run topology and Session reservation. Finalize is the only path
that writes Complete coverage; cancel marks preliminary work Superseded and
leaves events available.

The repeated raw payload is normalized into one revision-1
`sleep_workflow_input.v1`; tasks/run/results carry
`sleep_input_reference.v1`. Provider requests are expanded ephemerally at the
worker boundary. Input and submit payloads are capped at 2 MiB, with at most
256 MemoryUnits per finalize. Transactional ingest assigns a monotonic
per-Session admission sequence; receive-time/event-ID ordering remains only a
legacy compatibility fallback.

Next within S2.C, port post-finalize Core/derived routing, fidelity and
forgetting through the same durable-workflow pattern rather than adding a
manifest-only checkpoint path. Until then
`transactional_sleep_post_finalize_state.v1` preserves these branches as
explicit pending work instead of letting empty compatibility outcome fields
silently terminate them.

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

The in-memory backend snapshots a vault for deterministic conformance. Logical
failpoints are not OS process kills, filesystem tears, PostgreSQL crash tests or
network-proxy faults. It is not a capacity model and does not claim
cross-process durability. Membership
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

The Session reservation fields are additive for serialized `session.v1` data
through serde defaults, but source-breaking for downstream Rust code that uses
`SessionMetadata` struct literals. Release governance therefore classifies the
eventual published change as pre-1.0 minor; the implementation branch does not
silently relabel it as a patch.

The reference Sleep lifecycle now has bounded normalized prompt input, but it
still proves logical semantics rather than PostgreSQL storage density or query
plans. Physical row/WAL/network behavior remains a hard S3 gate.
