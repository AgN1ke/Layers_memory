# HISTORY

Memory Engine canonical record of post-launch corrections, schema changes, behavior changes, retracted claims, and any public notices that affect trust in the project. Newest first.

This file is not a changelog of features. It is a document of trust. The format follows the practice we adopted from MemPalace's `HISTORY.md` (see `wiki/pages/research/`).

For day-to-day working notes, use `DEVLOG.md`.

## When to add an entry

Make an entry when one of these happens:

- breaking change in any public contract (events, archive entry, recall query/result, pending task, manifest, journal);
- schema or migration change in Core / Session / Archive / Tasks / Journal;
- recall, sleep, reflection, scoring, decay, or storage behavior change that may alter results host integrations already rely on;
- fix that could change memory results or prevent data damage;
- prompt change that affects stored memory or model behavior;
- adapter compatibility note;
- security or data integrity issue;
- retraction of any public claim about Memory Engine capability, performance, or behavior.

## Entry format

Each entry is a dated block. Newest first.

```
## YYYY-MM-DD — Short title of the change (issue/PR link if any)

Context. Why this change exists.

**What changed:**
- concrete files, fields, formats.

**What is retracted (if applicable):**
- exact prior claim being withdrawn and why.

**What is still true:**
- what survives this change unchanged. Protects the entry from over-correction.

**What we are doing:**
- next-step plan.

**Thanks:**
- specific people who flagged or fixed.
```

If the change involves any benchmark, performance number, or measurable claim, the entry must include a reproducibility-anchor: which tag the result was produced from, which dataset, which seed, where the result files live in the repository.

## 2026-08-01 — Storage v2 begins with an atomic, vault-bound ingest transition

Context. The local `Storage::append_event` call hid three sequential physical
writes: event JSONL, Session metadata and Markdown projection. Copying that API
shape into PostgreSQL would allow replicas or crashes to expose a new event
without its matching Session state.

**What changed:**
- Added versioned Storage v2 records, work-unit intents and commit receipts,
  bound to the server-derived `VaultContext` rather than a caller-selected
  vault key.
- Added point reads, mandatory optimistic expected revisions, atomic work-unit
  publication, typed retryable conflicts and same-input operation replay.
- Added an in-memory transactional conformance backend. Follow-up acceptance
  expanded its operation-bound fault hooks across every logical pre-publish
  edge and the post-publish/pre-ack edge; an unknown commit outcome is reconciled
  through the operation receipt before the engine reports failure.
- Added `MemoryEngine::ingest_transactional`: event and Session metadata now
  form one authoritative commit on the new path and reuse existing event
  validation/scoring policy.

**What is retracted:**
- The server track is no longer accurately described as “Storage v2 not
  started”. Only its foundation and ingest transition exist; this is not a
  production storage or scale claim.
- The initial test description implied arbitrary-point fault injection. Commit
  `04b0457` proved one deterministic failure immediately before publish. The
  expanded reference now proves logical commit edges and lost acknowledgement,
  but still does not prove physical database/process crash durability.

**What is still true:**
- Existing `Storage`, `FileStorage` and `MemoryEngine::ingest` behavior remains
  compatible for local/Python/Godot hosts.
- The in-memory backend is not durable or multi-process and PostgreSQL has not
  started.
- S2 remains open until sleep, reflection/review, forgetting, hypothesis
  migration and task transitions pass the same atomicity/retry gate.

**What we are doing:**
- Follow the recorded work-unit inventory: correctness state and small
  mutations next, then sleep/fidelity/forgetting, then reflection/graph/vector
  workflows. Provider calls remain outside transactions.

**Compatibility:**
The new public contracts use their own non-exhaustive `StorageV2Error`; the
existing public `MemoryEngineError` enum was deliberately left unchanged. This
is additive work for a future pre-1.0 minor release. Package versions remain
unchanged until adapter and release gates pass.

**External acceptance:**
The `af932a4` follow-up passed source-level review of all ten assertions: four
distinct pre-publish rollback edges, automatic committed-receipt reconciliation
after a lost acknowledgement, operation-targeted fault delivery, invalid-edge
cleanup, replay, concurrency and isolation. This closes the S2.A logical
reference gate only. No Storage v2 host adapter or physical durability claim is
included.

## 2026-08-01 — Hypothesis graph Phase 2 passes a non-degenerate host-path fixture

Context. The first Chibigochi storage-copy migration contained no legacy
candidates, so it could prove empty contracts and product regression but not
mapping, evidence cardinality, provenance or projection on actual records. The
required disposable fixture has now exercised every one of those branches
without seeding or migrating owner memory.

**What changed:**
- The v0.6 Phase 2 implementation gate is closed by four candidates across
  owner, other and unscoped namespaces, two resolved units from different
  archive/session sources, one missing unit and duplicate/new reinforcement.
- Migration produced four nodes and exactly seven evidence links; duplicate
  provenance did not become duplicate evidence.
- Exact-scope export, unresolved provenance, repeat idempotence, complete
  CandidateBelief equality and graph-only fallback all passed through the
  Chibigochi Python module path.
- Empty migration/export collections also passed direct key access as `[]`.

**What is retracted:**
- The Phase 2 gate is no longer open because of the first run's live
  `DATASET GAP`. The historical fact that owner storage had no candidates
  remains true and recorded.

**What is still true:**
- Owner storage was not migrated and no synthetic beliefs were added to it.
- Legacy migration creates no semantic relations or abstraction levels and
  does not implement hierarchical reflection.
- v0.5 live reflection calibration and normal release/version gates remain
  separate requirements.

**What we are doing:**
- Keep the build-provenance caveat visible: the external fixture ran from a
  worktree with seven uncommitted files. A source audit confirms graph,
  migration and adapter paths are identical from `e0ad0fa` to clean descendant
  `8875e9a`, so the behavior gate stands, but this is not represented as a
  dedicated clean-commit release build.
- Continue server S2 independently. Phase 3 cognitive behavior starts only
  under its own typed-action, maturation and live-reflection gates.

**Reproducibility anchor:**
The fixture geometry and all observed counts are recorded in
`wiki/pages/releases/v0.6-hypothesis-graph-storage-acceptance.md`; deterministic
fault/recovery coverage remains in `engine_hypothesis.rs`.

## 2026-08-01 — Server identity and turn ordering now have fail-closed reference contracts

Context. The accepted Google Play architecture could not safely begin with SQL
tables while account identity, retry semantics and same-session ordering were
still prose. A shared/default identity here would repeat the previously found
`local_user` disclosure class before PostgreSQL could protect anything.

**What changed:**
- Added versioned Rust contracts for authenticated principals, active
  account-to-vault membership, internal `VaultContext`, turn requests,
  operations and the vault turn ledger.
- Missing, forged, cross-account or inactive identity is rejected. Verified
  external identity conflict becomes manual review; memory is never merged
  automatically.
- A request ID is idempotent inside its bound vault, operation identity includes
  the vault, and terminal results cannot commit out of session sequence.
- Added a restart-safe `LocalVaultHandle` reference with one file-backed memory
  root and ledger per explicit vault. It never creates or selects `local_user`.
- Recorded a provisional S0 engineering envelope and its 2x S7 test geometry.

**What is retracted:**
- The statement that server-track implementation had not started at all. S1
  reference contracts now exist; the production server backend still does not.

**What is still true:**
- `FileStorage` is safe for one writer process per runtime directory, not for
  shared multi-replica production use.
- There is no PostgreSQL backend, Storage v2 transaction, public authenticated
  game API, durable worker fleet or production capacity proof yet.
- Existing local/Python/Godot APIs and cognitive Session/Archive/Core behavior
  are unchanged.

**What we are doing:**
- Finish S1 host/mobile/auth integration in S4 after the storage boundary is
  transactional. The next core server phase is S2 Storage v2 inventory and
  work-unit design.
- Replace the planning envelope with measured closed-test product behavior and
  explicit owner-approved retention/region/RPO/RTO before infrastructure or
  launch claims.

**Reproducibility anchor:**
`crates/memory_engine/tests/server_runtime_s1.rs` covers identity failure,
cross-vault isolation, replay, ordering, restart and shared-ID negative cases.

## 2026-08-01 — Empty graph admin responses keep their public collection shape

Context. The first Chibigochi graph-storage acceptance ran against a complete
copy of owner storage but found zero legacy candidates. That made the migration
semantically empty and exposed an adapter contract defect: Rust values
contained empty vectors, while serde omitted their JSON keys.

**What changed:**
- `hypothesis_graph_migration_result.v1` now always serializes
  `unresolved_memory_unit_ids` and `warnings`, including as `[]`.
- `hypothesis_graph_export.v1` now always serializes `nodes`,
  `evidence_links`, `relations` and `warnings`, including as `[]`.
- Core and Python-adapter regression tests use direct key access so a future
  omitted collection fails instead of being hidden by `.get(..., [])`.
- Acceptance now requires a disposable non-degenerate host-path fixture when
  live owner storage has no candidates; synthetic beliefs must not be inserted
  into owner memory.

**What is retracted:**
- The documented implication that those collection keys were already present
  in every v1 response. Before this correction, empty collections were omitted.
- The first live-copy run is not a full live migration PASS. It is a
  deterministic PASS and product-regression PASS with a live `DATASET GAP`.

**What is still true:**
- Migration counts, journal recovery, idempotence, exact-scope filtering and
  non-empty unresolved provenance behavior are unchanged.
- The live source storage was never migrated or modified by the acceptance.
- Phase 2 does not create semantic relations or invoke an LLM.

**What we are doing:**
- Rebuild the corrected module and execute the non-degenerate fixture through
  the Chibigochi Python path. A successful fixture can close implementation;
  absent live legacy candidates remain an honest dataset limitation.

**Reproducibility anchor:**
`crates/memory_engine/tests/engine_hypothesis.rs` covers empty and non-empty
core serialization; `crates/python_adapter/tests/test_basic.py` asserts the
empty JSON contract exposed to hosts. The live acceptance record is in
`wiki/pages/releases/v0.6-hypothesis-graph-storage-acceptance.md`.

## 2026-08-01 — Centralized game hosting requires an authenticated Memory Runtime

Context. Chibigochi is intended for Google Play with thousands of clients
using a central server. The existing deployment boundary is one writer process
per `FileStorage` runtime directory; in-process locks and local stress tests do
not establish safe multi-replica, multi-tenant operation.

**What changed:**
- Accepted a separate production deployment track around the same Rust
  cognitive core: server-derived per-account vaults, transaction-capable
  Storage v2, PostgreSQL/pgvector, durable workers, portable export/import and
  whole-product load/isolation acceptance.
- Clarified that `FileStorage` remains the local/reference/export backend. A
  server vault may use PostgreSQL as its operational source of truth while
  preserving versioned human-readable round trips and one authoritative
  backend at a time.
- Made authentication stronger than namespacing: Godot cannot select an
  authoritative vault, and missing identity cannot fall back to a shared
  `local_user` in server mode.
- Recorded that exact per-vault pgvector search is the starting design;
  separate vector/graph databases and approximate indexes require measured
  need.

**What is retracted:**
- Any reading of “production-shaped Chibigochi bridge”, the in-process
  concurrency model, or the historical 1,000-session stress run as proof that
  the current file backend is ready for thousands of centralized Google Play
  players.
- The unconditional public wording that files must be the operational source
  of truth in every deployment. That remains true for `FileStorage`; the
  project-level invariant is ownership, inspectability, portability,
  provenance and recoverability.

**What is still true:**
- Session, Archive, Core and all cognitive policy remain in one reusable Rust
  core. Server scale does not create a second memory system.
- `FileStorage` remains the implemented and supported local deployment; its
  one-writer-process rule is unchanged.
- Vectors remain optional, rebuildable data over atomic MemoryUnits and never
  become a fourth layer or source of truth.

**What we are doing:**
- Execute server phases S1-S7 in order: identity/idempotency, transactional
  storage contract, PostgreSQL/pgvector, authenticated runtime, durable
  workers, migration/operations, then whole-product acceptance and staged
  rollout.
- Replace provisional capacity assumptions with product forecasts before
  making or testing a launch-capacity claim.

**Research anchor:**
`wiki/pages/research/server-memory-runtime-2026-08-01.md` records the primary
PostgreSQL, Google Play/Cloud and GitHub sources. No production throughput
benchmark is claimed by this entry.

## 2026-08-01 — Hypothesis graph storage is additive and migration is recoverable

Context. Live v0.5 product acceptance is still pending, but the next storage
foundation can be implemented independently because it does not change the
reflection prompt, thresholds, chat context, or Core authority.

**What changed:**
- Added versioned `hypothesis_node.v1`, `hypothesis_evidence_link.v1` and
  `hypothesis_relation.v1` records under `memory/core/hypotheses/`, plus schema
  markers in backward-compatible `manifest.v1`.
- `migrate_hypothesis_graph()` freezes desired node/evidence records in a
  journaled `hypothesis_graph_migration_operation.v1`, resumes after restart,
  reports unresolved MemoryUnit IDs, and creates no semantic relations.
- One hypothesis/MemoryUnit/polarity tuple has one deterministic evidence edge.
  Legacy candidate and reinforcement records are origins of that edge, not
  independent copies of the same evidence.
- Existing reflection and owner review remain source-compatible through a
  lossless `CandidateBelief` projection. Candidate persistence also completes
  graph projection, including during journal replay; graph-only abstraction,
  contradiction, validity and directness fields are preserved.
- `export_hypothesis_graph()` exposes one exact personality scope for human
  inspection. Missing scope remains isolated unscoped data, never a wildcard.
  Relation writes fail closed across scopes.
- Python and Godot adapters expose the migration and versioned export request
  without taking ownership of graph policy.

**What is still true:**
- Session, Archive and Core remain the three memory layers. The hypothesis
  graph is internal Archive-to-Core reasoning state and is not chat context.
- Reflection still cannot promote itself into Core; owner review remains
  mandatory.
- No hierarchical LLM action, semantic merge, support propagation or
  maturation behavior is claimed by this storage phase.
- Live v0.5 acceptance and a migration/inspection run on a copy of real
  Chibigochi memory remain open gates.

**Reproducibility anchor:**
`crates/memory_engine/tests/engine_hypothesis.rs` covers legacy and repeated
migration, restart recovery, scope-exact export, unresolved provenance,
compatibility projection and cross-scope relation rejection. The reflection
fault-boundary fixture additionally verifies node/evidence recovery.

## 2026-08-01 — v0.5 reflection integrity gate is implemented

Context. The Phase C information-flow audit found three release-integrity
defects: reflection/review could stop between separately atomic files without
a replay plan, absent scope could expose named-scope Core facts, and review or
delayed apply could act on a mutable scope or stale evidence.

**What changed:**
- `journal_operation.v1` additively accepts `reflection_apply` and
  `candidate_review`. Both store versioned frozen intent and idempotent desired
  effects; started operations are recoverable without another LLM call.
- Reflection application revalidates cited unit/archive/candidate/Core records
  before the first write. New task snapshots carry additive record revisions;
  saved tasks without them remain compatible.
- `MemoryUnit.last_reflection_operation_id`, optional
  `ReflectionCandidatesResult.operation_id/result_digest`, and optional
  `PendingTask.outcome=reflection_outcome.v1` make checkpoint and terminal
  outcome replayable without persisting memory text in the task outcome.
- Missing `core_scope` now means only the unscoped namespace. Candidate review
  cannot replace scope, cannot revisit a terminal candidate, and validates
  evidence again before approval.
- Rust, Python and Godot expose explicit recovery; `pending_tasks()` performs
  it before returning provider work.

**What is retracted:**
- The immediately preceding statement that Phase C.1 is still an unimplemented
  release blocker is now historical. Its deterministic integrity gate is
  closed; live product acceptance is not.

**What is still true:**
- Core changes only through explicit review; autonomous reflection still
  cannot promote itself.
- Session, Archive and Core remain the only memory layers. Journal and task
  outcomes are processing/audit state, and vectors remain optional derived
  data.
- Flat candidate retrieval still cannot guarantee semantic continuity at
  scale; hierarchical hypotheses remain the next minor-line design.

**What we are doing:**
- Run multi-cycle live Chibigochi acceptance without tuning defaults mid-test.
- Only after live PASS synchronize manifests for `v0.5.0` under the documented
  SemVer policy.
- Start the v0.6 hypothesis graph as a separate schema/migration phase.

**Reproducibility anchor:** `crates/memory_engine/tests/engine_reflection.rs`
contains restart-boundary, reinforcement replay, review recovery, stale
evidence and scope-isolation fixtures; `cargo test -p memory_engine` passes on
the working tree dated 2026-08-01. Live acceptance is explicitly not claimed.

## 2026-08-01 — v0.5 reflection acceptance is reopened before hypothesis-graph work

Context. A full information-flow audit after Phase C found that the functional
typed-action path is not yet a complete integrity guarantee. Candidate and
reinforcement writes, unit/archive checkpoints, reflection state, task
completion, owner review and Core transitions span separate files without a
reflection-specific recovery operation. The same audit reproduced a Core-fact
scope leak when reflection is started without `core_scope`, and found that
review can accept a caller-supplied scope or revisit a terminal candidate.

The owner also corrected the flat-dedup direction: a narrow claim such as
interest in colonizing Mars, a broader claim about other planets, and a still
broader interest in space may all be valid hypotheses with different support.
They must not be collapsed merely because their wording is related.

**What is retracted:**
- The unqualified claim that deterministic happy-path tests make Phase C ready
  for live acceptance. Phase C remains functionally implemented, but v0.5
  acceptance now requires the Phase C.1 integrity/recovery gate.
- The claim that repeated evidence is already guaranteed to strengthen one
  traceable hypothesis. The current bounded recent-candidate context guarantees
  this only when the intended candidate is visible and selected.
- The implication that a scope lock and final checkpoint alone protect every
  mid-apply restart boundary.

**What is still true:**
- Typed new-candidate, reinforcement, Core-challenge and no-insight actions are
  implemented and validated against task-visible IDs.
- Reflection cannot write Core without explicit review.
- Personality membership is explicit; text similarity is not allowed to link
  identities.
- Non-vector reflection remains complete, and optional vectors remain a local
  derived sidecar rather than memory truth.

**What we are doing:**
- Close v0.5 with immutable scope, terminal-state guards, pre-commit evidence
  revalidation and journal-backed idempotent apply/review operations.
- After that release boundary, migrate flat candidates into an auditable graph
  of hypotheses, evidence links and generalization/contradiction relations.
- Treat confidence as explainable support rather than a claimed probability of
  truth; broad hypotheses require independent evidence branches.
- Add contradiction/time revision before any opt-in native clarification path.

**Documentation authority:**
- `wiki/pages/planning/autonomous-reflection-v0.5.md` for the reopened v0.5
  integrity gate;
- `wiki/pages/planning/reflective-belief-system.md` for the phased v0.5–v0.7
  direction;
- `wiki/pages/research/generative-agents-memory-stream-2026-07-06.md` for the
  external durable/sleep-time memory comparison.

No runtime behavior or persisted schema changed in this documentation
decision.

## 2026-08-01 — Reflection outcomes become typed and cumulative

Context. Autonomous reflection could see existing candidates but could only
return new candidate drafts. Repeated evidence therefore had no durable way to
mature one hypothesis, and abstention or Core contradiction had no explicit
result semantics.

**What changed:**
- New reflection tasks request prompt version 3 and
  `reflection_actions_result.v1`, with typed new-candidate, reinforcement,
  Core-challenge, and structured no-insight outcomes.
- Applied typed actions return `reflection_apply_result.v1`; the LLM output
  contract and the core application result are no longer conflated.
- `CandidateBelief` additively stores reinforcement records and merged
  cross-session evidence provenance. Reinforcement requires at least one task
  unit not already attached to that candidate and never lowers confidence.
- A Core challenge creates a reviewable tagged candidate. Core remains
  unchanged until the existing explicit approval path contests the cited fact
  and promotes the replacement.
- Typed application fails closed for out-of-task IDs, reused-only evidence,
  duplicate candidates/actions, malformed no-insight, and Core challenges
  without a visible conflicting fact. Failed application does not advance the
  reflection checkpoint.
- Python and Godot expose `resume_reflection_actions`. The generic submit path
  accepts both the new schema and legacy `reflection_result.v1`, so saved tasks
  and older hosts remain resumable.

**What is retracted:**
- The preceding Phase B entry says candidate reinforcement is not implemented.
  That statement is now historical; Phase C implements it.

**What is still true:**
- Reflection cannot write Core directly, vectors are not required, and
  Session/Archive/Core remain the only persisted memory layers.
- Deterministic implementation is not live product acceptance.

**What we are doing:**
- Run full workspace/adapters quality gates, then perform live Chibigochi
  acceptance across repeated evidence, temporary state, and a real Core
  contradiction before the v0.5 release decision.

**Reproducibility anchor:** deterministic tests on the eventual implementation
commit; live acceptance not yet claimed.

## 2026-08-01 — Reflection evidence becomes personality-scoped

Context. The first autonomous-reflection implementation stored its trigger
state under `core_scope`, but counted and supplied memory only from the current
source session. A stable pattern spread across several conversations therefore
could not become one reflection event.

**What changed:**
- `reflection_state.v1` additively records every source session explicitly
  bound to the reflection scope. Legacy state falls back to its singular
  `source_session_id`.
- `bind_reflection_session` lets a host register current or historical
  sessions without evaluating a trigger or spending an LLM call;
  `begin_reflection_if_due` binds idempotently as a safe default.
- Trigger eligibility now accumulates new validated units across those bound
  sessions. Sessions are never joined by text similarity or storage proximity.
- Automatic input separates new trigger evidence from bounded, already
  reflected related history. The non-vector selector uses exact structural
  overlap with explicit weights: link `4`, theme `3`, topic/subtopic `2`, tag
  `1`; the default acceptance score is `2`, so one weak tag is insufficient.
- In-scope draft/ready candidates are included for duplicate prevention.
  Related history and candidate context have explicit configurable limits.
- `CandidateBelief.source_session_ids` records the sessions behind accepted
  evidence, while the singular field remains compatible as the triggering
  session.
- `reflection_analyze` prompt version `2` distinguishes current trigger
  evidence from older context and forbids duplicate candidates.

**What is still true:**
- Reflection uses only fidelity-valid active Archive units and cannot write
  Core directly.
- Vectors are not required for personality-scope reflection and do not define
  identity membership.
- Reinforcement of an existing candidate is not implemented yet; it remains
  the typed-result lifecycle work of Phase C.

**What we are doing:**
- Run live multi-session Chibigochi acceptance for scope isolation, related
  history quality, and prompt behavior.
- Then implement typed new/reinforce/contradict/no-insight outcomes without
  relaxing manual Core review.

**Reproducibility anchor:** deterministic Rust/Python/workspace gates on the
eventual implementation commit; live acceptance not yet claimed.

## 2026-08-01 — Reflection gains a durable core-owned trigger cycle

Context. Reflection could already turn validated memory units into reviewable
Core candidates, but only after an explicit manual call. That left a gap in the
living-memory lifecycle: the engine could preserve experience yet could not
decide when enough new experience existed to justify interpretation.

**What changed:**
- `ReflectionConfig` makes accumulated salience, critical-unit weight, maximum
  interval, minimum scheduled evidence, and automatic batch size explicit.
- `reflection_status` explains whether reflection is due, and
  `begin_reflection_if_due` creates a reasoning task only for
  `critical_memory`, `accumulated_salience`, or `scheduled_interval`.
- Only new `ActiveArchive` + fidelity-valid memory units contribute. Automatic
  input is bounded and its archive summaries are limited to parent archives of
  those units.
- `reflection_state.v1` persists the per-scope checkpoint under
  `runs/reflection/`; `MemoryUnit` adds compatible `reflection_count` and
  `last_reflected_at` metadata.
- Successful zero-candidate analysis consumes its input; failed analysis does
  not. Per-scope locking and active-task checks prevent duplicate automatic
  reflection tasks.
- Python and Godot expose the core methods. The Telegram demo can execute the
  after-sleep path with `MEMORY_BOT_AUTO_REFLECTION=1`; it is opt-in because it
  may add a reasoning-model call.

**What is retracted:**
- The roadmap description of reflection as manual-only is no longer true for
  the Phase A library capability. Automatic execution is still opt-in and live
  acceptance is not yet claimed.

**What is still true:**
- Reflection cannot write Core. It only creates `CandidateBelief` records for
  review.
- Session, Archive, and Core remain the only persisted memory layers.
  Reflection state is orchestration metadata, and vectors remain optional.
- At the Phase A cutoff the automatic evidence set was source-session-backed.
  The Phase B entry above records its resolution; typed candidate reinforcement
  remains open.

**What we are doing:**
- Run Chibigochi acceptance over several sleep cycles and calibrate the initial
  defaults.
- Then add bounded personality-scope historical evidence and reinforcement of
  an existing candidate instead of duplicate candidate creation.

**Reproducibility anchor:** `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, Python adapter tests,
host syntax checks, and wiki link checks on the eventual implementation commit.

## 2026-08-01 — Associative recall learns to abstain at the live model boundary

Context. The live Chibigochi acceptance of `3c5aa74` proved that the separate
associative layer, topic switching, deduplication, Stage 1 fallback, and prompt
labeling work. Its guaranteed-negative dinosaur query also exposed a calibration
failure: with no dinosaur memory in the 30-row owner scope, the nearest planet
unit still passed the old `0.45` contextual threshold at cosine `0.455`.

**What changed:**
- The bundled multilingual MiniLM associative-memory default is `0.47`, between
  the strongest observed false positive (`0.455`) and weakest observed useful
  control (`0.497`). This changes only automatic current-topic association;
  explicit deep recall keeps its separate `0.30` default.
- The two defaults are named public constants instead of unexplained literals.
- Every accepted `AssociativeMemoryItem` exposes `activation_threshold`, and
  package notes state the threshold for both applied and below-threshold paths.
- The Python constructor accepts optional
  `associative_memory_min_similarity`; the thin adapter only forwards it into
  core policy. Telegram demo telemetry now records the threshold and the actual
  package notes without memory text.

**What is retracted:**
- The live report's statement that associative expansion had no relevance
  threshold. The gate existed; `0.45` was simply too low for this observed
  model/data boundary.

**What is still true:**
- Associative memory remains ephemeral, unit-focused, local-vector-backed, and
  separate from the three persisted layers and ordinary Archive ranking.
- One boundary sample does not establish universal embedding quality. Changing
  the model requires a new positive/negative calibration rather than copying
  `0.47` blindly.

**What we are doing:**
- Re-run the live positive controls and guaranteed-negative topic switch before
  closing the v0.4 acceptance or changing package versions.

**Reproducibility anchor:** library base `3c5aa74`; Chibigochi base `f0d986d`;
live owner scope `dev-1785028194.31988-42102`; 30 vector rows; model
`sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`, dimension 384.

## 2026-08-01 — Current-topic vector memories become an explicit associative layer

Context. Live Chibigochi verification of `d5d5f40` showed that ordinary Stage 2
correctly ranked Titan first, while `CoreContextPackage.items[0]` was a Mars
memory inserted by the older contextual-expansion path. The two items were
relevant, but Stage 2 and deep recall use different score formulas. Prepending
both into one `archive_relevant` list made the final package order disagree with
its recall debug and let the token budget describe unlike scores as one rank.

**What changed:**
- `core_context_package` keeps ordinary Archive recall structural and no longer
  sends its current-turn embedding through ordinary Stage 2 at the same time as
  contextual expansion.
- High-confidence deep-recall details now use the additive
  `associative_memory` field. Each item retains its `memory_unit_id`, parent
  `archive_id`, scope, timestamp, semantic similarity, deep-recall score, and
  activation explanation.
- The canonical prompt view renders a separate `<associative_memory>` section
  and identifies its contents as older memories activated by the current topic.
- Compressed-memory budgeting reports associative and ordinary Archive
  trimming separately. Empty associative fields and zero counters are omitted,
  preserving the non-vector serialized path.
- Explicit `engine.recall(...)` continues to support opt-in unit-focused Stage 2.

**What is retracted:**
- The Phase 1 implementation note that contextual vector details should be
  inserted into `archive_relevant` as if they shared its ranking semantics.

**What is still true:**
- Session, Archive, and Core remain the three persisted memory layers.
- Vector rows remain an optional local, purgeable, rebuildable sidecar over
  validated active memory units; files remain the source of truth.
- The Rust core owns selection, deduplication, budgeting, and prompt assembly;
  the host only supplies the current-turn embedding.
- Associative memory is rebuilt independently for every package and does not
  persist or carry a previous topic into the next turn.

**What we are doing:**
- Run a live Chibigochi topic-switch acceptance (Mars to dinosaurs), verify the
  explicit old-memory label in the actual prompt, and close `v0.4.0` only after
  that package behavior passes.

## 2026-08-01 — License clarifies the Licensor's retained rights

Context. The non-commercial public license already reserved the Licensor's
right to offer the Software under commercial, open-source, private, exclusive,
or custom terms. It did not state as directly that restrictions imposed on
licensees do not bind the Licensor's own lawful exercise of rights in the
Software.

**What changed:**
- The Memory Engine Non-Commercial Public License is identified as Version 0.3.
- Section 13 now explicitly states that restrictions and conditions imposed on
  `You` do not restrict the Licensor's own lawful use, copying, modification,
  distribution, commercialization, or licensing of the Software.
- The normative license body in root `LICENSE.md` and the governance wiki copy
  remains identical; the wiki page keeps only its separate navigation header.

**What did not change:**
- The public grant remains non-commercial.
- Commercial use by third parties still requires a separate written agreement.
- This clarification reserves the ability to license the Software in parallel;
  it does not by itself publish a second commercial license.
- The library/package version is unaffected by the license document's own
  version number.

## 2026-08-01 — Release versions adopt one SemVer policy

Context. Git tags `v0.2.0` and `v0.3.0` recorded accepted project milestones,
while the Cargo workspace and Python distribution remained at `0.1.0`. That
made the repository's three-number labels unsuitable as a reliable dependency
compatibility signal.

**What changed:**
- `wiki/pages/governance/versioning.md` is now the normative release policy.
- Before `1.0.0`, compatible fixes increment PATCH; public features and
  intentional breaking changes increment MINOR, with breaking changes called
  out explicitly and accompanied by migration notes.
- After `1.0.0`, ordinary SemVer applies: breaking change -> MAJOR, compatible
  feature -> MINOR, compatible fix -> PATCH.
- The root Cargo workspace version is the release source of truth. Rust crates,
  the Python package, immutable Git tag, acceptance record, and HISTORY must
  agree before a release is tagged.
- `tools/check_release_versions.py` now enforces the manifest boundary in the
  normal CI path. Its strict tag mode additionally requires an exact
  three-number accepted evidence page and matching HISTORY heading; GitHub
  Actions runs that mode automatically for future `v*` tags.
- Library SemVer is separate from named storage/wire schemas. Compatible
  optional fields may remain in `.v1`; incompatible schema changes require a
  new schema version and tested migration.

**What is retracted:**
- The implicit claim that historical `v0.2.0`/`v0.3.0` tags also represented
  synchronized Cargo and Python distribution versions.

**What is still true:**
- The historical tags and their acceptance evidence remain valid milestone
  records and will not be moved or rewritten.
- No package version changes in this policy/gate change.

**What we are doing:**
- After live acceptance of vector Stage 2 and unit-focused recall, prepare the
  first synchronized release as `0.4.0`, including an automated manifest/tag
  consistency gate. The gate is now implemented; the live acceptance,
  acceptance record, synchronized version bump, and tag remain intentionally
  open.

**Thanks:**
- The owner and Yevhen for forcing the project to distinguish actual dependency
  compatibility from merely writing three numbers in a tag.

## 2026-07-31 — Stage 2 preserves MemoryUnit provenance and focus

Context. The first Chibigochi verification of vector Stage 2 passed the main
planet-versus-routine case, but one broad query appeared to raise an unrelated
mixed daily archive. Code inspection corrected the initial diagnosis: vectors
were already stored per `MemoryUnit`; the actual gap was that ordinary recall
collapsed the winning unit to an archive cosine and then returned the whole
archive compact projection.

**What changed:**
- Stage 2 now retains the exact active vector-eligible `MemoryUnit` that produced
  each archive candidate's maximum cosine. Equal similarities use the smallest
  `memory_unit_id` as a deterministic provenance tie-breaker.
- `RecallItem` adds optional `matched_memory_unit_id`. For a matched Stage 2
  candidate, `gist` and `compact_memory` contain the winning unit thesis instead
  of the complete mixed archive compact. The parent archive remains the item ID
  and the boundary for Stage 1 metadata, time, ranking, and recall statistics.
- `RecallCandidateDebug` adds the same optional ID. Existing text-free host
  traces therefore expose `query hash -> MemoryUnit -> archive -> final rank`
  without logging query or memory text.
- The fields are additive within `recall_result.v1`; old payloads without them
  remain valid, and Stage 1 output omits them.
- Regression and host-conformance coverage proves that a mixed Europa/Titan
  archive returns only the winning Titan thesis through Rust and the Python
  adapter; Telegram-local Stage 1 conformance remains unchanged.

**What is retracted:**
- The live-test interpretation that Stage 2 embedded a whole daily archive. The
  implementation already embedded `MemoryUnit.thesis`; only the final ordinary
  recall projection was archive-wide.

**What is still true:**
- The Stage 2 candidate window, `0.5/0.5` default formula, opt-in/local vector
  boundary, and independent global `recall_deep` path are unchanged.
- No scoring weights were changed from a single weak broad-query example.

**What we are doing:**
- Use winning-unit provenance to inspect the weak "other worlds" query before
  deciding whether any model or scoring calibration is justified.

**Thanks:**
- The owner and Claude developer for providing the Chibigochi live ranking,
  score components, and mixed-archive symptom instead of asking for a blind
  weight adjustment.

## 2026-07-31 — Vector Stage 2 now re-ranks ordinary recall

Context. A live recall package contained the relevant planet memories, but
Stage 1 ranked an unrelated high-weight gym memory above them. The package also
did not expose enough structured evidence to distinguish a disabled/building
vector path from a scoring problem.

**What changed:**
- `recall_query.v1` accepts optional `query_embedding` (`model_id` +
  `query_vec`). A ready opt-in vector scope re-ranks the first configurable
  Stage 1 candidates with
  `stage1_score * (1 - w_vec) + max_memory_unit_cosine * w_vec`.
- Defaults are a 40-candidate window and `w_vec=0.5`. Rust callers configure
  them through `EngineOptions`; the Python adapter exposes both as optional
  constructor arguments.
- `core_context_request.v1` accepts `explain_recall` (default `false`) and now
  forwards its existing query embedding into ordinary recall as well as the
  independent contextual/deep expansion path.
- Opt-in `RecallDebug` now exposes Stage 1 score components, initial/final
  ranks, semantic similarity, final score, and vector status. The Telegram
  development host writes this payload without memory/query text to
  `runtime/logs/recall_trace.jsonl`.
- Regression tests cover both cases: semantic reordering when the correct
  archive is inside the Stage 2 window, and global `recall_deep` recovery when
  it is outside.

**What is retracted:**
- The contract text that described `stage2_embeddings` only as a future stage.

**What is still true:**
- Embeddings remain opt-in, local, host-computed derived data over validated
  `MemoryUnit` records. The Rust core does not load a model or call a provider.
- Without `query_embedding`, or while the vector scope is unavailable, ordinary
  recall stays on Stage 1.
- `recall_deep` remains an independent semantic candidate-generation path; it
  was not replaced by top-K re-ranking.

**What we are doing:**
- Use the new trace on live recall traffic before changing the default 0.5
  balance or deep-recall scoring weights.

**Thanks:**
- The owner and Claude developer for replacing the initial theory with the raw
  four-candidate live package that isolated the ordering failure.

## 2026-07-05 - Context packages can expand active-topic detail from vectors

Context. Vector storage already supported explicit distant recall. The owner
clarified the product goal: keep the ordinary memory context small while adding
specific detail when the active topic calls for it.

**What changed:**
- `core_context_request.v1` accepts optional `query_embedding` with `model_id`
  and `query_vec`.
- When `query_embedding` is present, `engine.core_context_package` can pull a
  small number of high-confidence memory-unit details from the ready vector
  scope into `archive_relevant`.
- Contextual expansion items are deduplicated against visible Core, recent
  session, trace, and ordinary archive memory, then passed through the existing
  context budget.
- The Telegram development host now supplies a query embedding when the chat
  vector scope is ready and the local embedder is available.

**What is retracted:**
- Nothing.

**What is still true:**
- The Rust core remains provider-agnostic. Hosts provide vectors; the core
  shapes memory.
- Without `query_embedding`, existing context-package behavior is unchanged.
- Vector storage remains opt-in and disabled by default.

**What we are doing:**
- Observe Phase 1 on real chat data before adding Core-fact vector anchors in
  contextual expansion Phase 2.

## 2026-07-05 - Project documentation moved into `wiki/`

Project knowledge was reorganized into an LLM-wiki structure. The old `docs/`
tree was moved into `wiki/pages/`, with `wiki/index.md` as the content index,
`wiki/log.md` as the chronological wiki maintenance log, and `wiki/AGENTS.md` as
the maintenance schema for future agents.

**What changed:**
- Strategy, architecture, contracts, roadmap, audit, acceptance, integration,
  governance, and research documents now live under `wiki/pages/`.
- Root README links now point to `wiki/index.md` and the moved foundation pages.
- Existing internal references to moved project documents were updated to the
  new paths.

**What is retracted:**
- The old public assumption that project knowledge lives under `docs/` is no
  longer current.

**What is still true:**
- Runtime behavior, storage schemas, prompt contracts, adapters, and model
  execution behavior are unchanged.
- Root `README.md`, `HISTORY.md`, `DEVLOG.md`, `LICENSE.md`, `SECURITY.md`,
  prompt files, runtime README files, crate README files, host README files, and
  config README files remain in their operational locations.

**How to verify:**
- Open `wiki/index.md` and confirm that project documents are discoverable from
  the wiki index.
- Run `git status --short` after the migration commit and confirm the old docs
  files are represented as moves into `wiki/pages/`.

## 2026-07-04 - Memory-unit repair for complete archives with missing theses

A long live Telegram conversation exposed a partial sleep result: one complete archive had source events and Core signals, but no `MemoryUnit` theses because `memory_unit_pass` repeatedly returned no usable candidates. That made the archive weaker for long-memory prompt use and excluded that slice from vector backfill.

**What changed:**
- The sleep driver now escalates the final `memory_unit_pass` retry to the `reasoning` role.
- New core/PyO3 APIs can inspect and repair complete archives whose `memory_units` are empty: `pending_memory_unit_repairs(session_id)` and `submit_memory_unit_repair_response(task_id, response)`.
- Repair tasks reuse `memory_unit_pass` over the archived source events, so crash-rotated events remain usable for later self-healing.
- If a repair response is blocked by the provider or otherwise fails as model output, the core creates conservative fallback memory units from already stored archive tracks (`personal_signals`, facts, topic-thread summaries, then gist as a last resort).
- Transport failures still mark the repair task failed so they can be retried later.
- Telegram host now runs memory-unit repair after sleep completion and backfills embeddings for repaired units.
- Telegram local embedder is now a module-level singleton, so the ONNX session is reused instead of being recreated for every deep recall or embedding batch.

**What is retracted (if applicable):**
- The assumption that retrying `memory_unit_pass` later is sufficient for every empty-unit archive. Some real prompts can keep failing through provider blocking, so the core needs a deterministic fallback from already validated tracks.

**What is still true:**
- The Rust core still does not call providers or compute embeddings.
- Vectors remain derived data over `MemoryUnit` theses.
- Core facts are not changed by memory-unit repair; repair only fills archive memory units and compact long-memory text.
- Archived source events remain the material for repair, but fallback units never invent new raw facts beyond existing archive tracks.

**Live repair result:**
- Owner Telegram scope `telegram_311422683`: archive `archive_1783159971443222600_98` was repaired after Gemini blocked the repair prompt.
- The archive gained 8 memory units, `compact_memory` became non-empty, vector rows increased from 9 to 17, and pending tasks returned to zero.

**Reproducibility anchor:**
- `cargo fmt`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts/telegram_gemini_bot/bot.py hosts/telegram_gemini_bot/local_harness.py tests/host_conformance/host_conformance.py`
- `crates/python_adapter/.venv/Scripts/python.exe -m pytest crates/python_adapter/tests -q`
- `crates/python_adapter/.venv/Scripts/python.exe tests/host_conformance/host_conformance.py --host direct-vectors`
- `crates/python_adapter/.venv/Scripts/python.exe tests/host_conformance/host_conformance.py --host direct-forced-recall`
- `crates/python_adapter/.venv/Scripts/python.exe tests/host_conformance/host_conformance.py --host telegram-distant-gate`

## 2026-07-03 - Vector embedder default changed to fastembed-supported multilingual MiniLM

Live calibration setup for vector recall installed `fastembed==0.8.0` and tried to load the model named in the vector-storage TZ. `TextEmbedding("intfloat/multilingual-e5-small")` failed because that model is not present in the installed fastembed registry. After switching to a supported 384-dimensional multilingual model, owner-scope calibration showed a lower cosine range than the original e5-family assumption.

**What changed:**
- The default vector model is now `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` with `dim=384`.
- The Rust default `DEFAULT_VECTOR_MODEL_ID`, Telegram `DEFAULT_EMBEDDING_MODEL`, vector conformance constants, contracts, and the implementation TZ now use the supported model.
- `VectorConfig.deep_recall_min_sim` is now `0.30` for this model family. Telegram's prompt-facing `vivid` threshold for `/recall_deep` is now `0.55`.
- Existing vector catalogs using the old model must be purged/rebuilt; vectors are derived data, so this does not affect archive or memory-unit truth.

**What is retracted (if applicable):**
- The claim that the v1 local fastembed path can use `intfloat/multilingual-e5-small` is retracted for the installed fastembed version.

**What is still true:**
- The Rust core still does not compute embeddings or load an embedding model.
- The model/dim boundary remains strict: submit and deep recall reject mismatched vectors instead of silently accepting them.
- The default dimension remains 384, so storage shape and deterministic tests stay small.
- `min_sim` still gates on raw cosine before recency/weight scoring; lowering it to `0.30` does not let recency rescue below-threshold rows.

**Reproducibility anchor:**
- `python -c "from fastembed import TextEmbedding; print(TextEmbedding.list_supported_models())"` shows `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` and not `intfloat/multilingual-e5-small` for fastembed 0.8.0.
- Owner Telegram scope `telegram_311422683`: 63 eligible units embedded; true-positive sims for cat/space/car queries were roughly `0.52-0.66`; a clear negative soup query topped out at about `0.26`; `min_sim=0.30` separates those cases. The location query "де я живу?" remains weak for vector-only recall without an explicit location cue and should be handled by Core/Stage 1.
- `cargo test -p memory_engine --test engine_vectors`

## 2026-07-03 - Vector storage Phase B slice: deep recall over host-provided vectors

Vector storage Phase A created the derived index and `ComputeEmbedding` task boundary. This slice adds the first read path over that index: `recall_deep`, plus a Telegram host-side local embedder/manual command path for calibration.

**What changed:**
- New public core/PyO3 API: `recall_deep(query_json) -> deep_recall.v1`.
- `DeepRecallQuery` carries `scope`, host-produced `query_vec`, `model_id`, `top_k`, `min_sim`, and optional deterministic `now`.
- The core normalizes the query vector, enforces model/dim match, searches only `memory/archive/vectors/<scope>/`, skips tombstoned/ineligible units, gates on raw cosine `min_sim`, ranks by `sim + recency_weight * recency + unit_weight * unit.weight`, and returns scarce top-K hits.
- Returned deep-recall hits update existing buffered archive recall stats; no synchronous archive write is added to the recall path.
- Telegram host gained `local_embedder.py` (fastembed, local CPU, lazy import), embedding task execution after sleep/backfill, `/vectors*` commands, and `/recall_deep text` for calibration.
- Host conformance gained `--host direct-vectors`, using deterministic fake vectors through the public Python adapter.

**What is retracted (if applicable):**
- Nothing is retracted. Phase A's statement that deep recall remained future work is superseded by this Phase B slice.

**What is still true:**
- The Rust core still does not compute embeddings, call a network, know provider keys, or load an embedding model.
- Vector storage remains opt-in and disabled by default. With vectors disabled, ordinary Stage 1 recall remains the active chat path.
- Vectors remain derived data; archives and memory units are still the source of truth.
- Automatic Gemini function-calling for `recall_distant_memory`, live `min_sim` calibration, and Stage 2 reranking remain open before Phase C.

**Reproducibility anchor:**
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m pytest crates/python_adapter/tests -q`
- `python tests/host_conformance/host_conformance.py --host direct-vectors`
- `python tests/host_conformance/host_conformance.py --host telegram-local`
- Full deterministic host conformance also passed for `direct`, `direct-multispeaker`, `godot-headless`, `chibigochi-spike`, `chibigochi-ui`, `chibigochi-llm-bridge`, and `chibigochi-product-loop`.

## 2026-07-03 - Vector storage Phase A: opt-in derived index and ComputeEmbedding tasks

The owner accepted `wiki/pages/research/vector-storage-tz-2026-07-03.md` as the implementation contract for vector storage. This first phase adds the derived storage/index and task boundary only; it does not add deep recall, Telegram fastembed, or Stage 2 reranking yet.

**What changed:**
- New derived vector catalog under `memory/archive/vectors/<scope>/`: `manifest.json`, `vectors.f32`, `rows.jsonl`, and `tombstones.jsonl`.
- New public core APIs: `vector_state`, `set_vector_scope`, `rebuild_vectors`, `pending_embedding_backfill`, and `resume_compute_embedding`.
- `TaskType::ComputeEmbedding` is now active. The core emits `LlmRequest { prompt_id: "embed_batch", role_hint: fast }` with `EmbedBatchInputs`; hosts return `embed_batch_result.v1`.
- `SleepOutcome` can include `embedding_requests` when a scope is enabled and new eligible `MemoryUnit` theses need embeddings.
- Vector rows index active archive memory units only. Forgotten/rejected units are tombstoned; compaction removes tombstoned rows. `remember_back` is picked up by backfill.
- Multi-speaker sessions are gated out of embedding until attributed memory units land.

**What is retracted (if applicable):**
- The reserved `ArchiveEntry.embedding_model_id` and `ArchiveEntry.embedding` fields are not the vector-storage path. They remain deprecated/reserved compatibility fields; Phase A indexes `MemoryUnit` theses instead.

**What is still true:**
- The Rust core still does not compute embeddings, call a network, know provider keys, or load an embedding model.
- Vector storage is disabled by default. With vectors disabled, no embedding tasks are emitted and ordinary Stage 1 recall is unchanged.
- Vectors are derived data; deleting or rebuilding `memory/archive/vectors/<scope>/` does not delete memory truth.
- Deep recall (`recall_deep`), local fastembed in Telegram, min-sim calibration, and Stage 2 rerank remain Phase B/C work.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_vectors`
- `cargo test -p memory_engine`
- `cargo check --workspace`

## 2026-07-03 - Multi-speaker branch 1: speaker contract, attributed transcript, Core gossip gate

First slice of the multi-speaker geometry plan (`wiki/pages/research/multi-speaker-geometry-2026-06-12.md`, DEVLOG Запис 117). Linear hosts are unaffected: every new field is optional with serde defaults, and behavior without `speaker` is byte-identical to before.

**What changed:**
- `IngestEvent`/`StoredEvent`/`CoreContextEvent` gain optional `speaker: { id, name }` (stable host identity + prompt-facing name).
- `render_memory_view` renders user-side events with a `speaker` under the speaker's name (`Жека: ...`) instead of `user:`; assistant rendering unchanged; current-message dedup now treats any non-assistant role as user-side.
- The `reply_to` link kind is documented in `wiki/pages/foundation/contracts.md` (the typed `links` slot already existed; reply structure lives in the raw session layer and is not persisted into Archive/MemoryUnit).
- Phase-1 Core protection, enforced in the core (not the host): when a session has >= 2 distinct `speaker.id` among user messages, the automatic Archive -> Core personal-signal bridge is disabled (signals are counted as skipped) and no unit is on the automatic Core fidelity path. Core still grows through explicit `/remember` and reflection candidates with manual review.
- New conformance scenario `--host direct-multispeaker`: proves `speaker` survives the PyO3 JSON boundary, the transcript is attributed, memory-unit theses stay attributed, and a high-confidence gossip signal ("У Жеки є мотоцикл" said in a two-speaker chat) does not become a Core fact.

**What is retracted (if applicable):**
- The implicit claim that the user-source guard alone protects Core in group chats is retracted: with several human speakers every message is `user_message`, so gossip passed the old guard. Until subject-aware facts land (branch 3), multi-speaker sessions get no automatic Core writes at all.

**What is still true:**
- Single-user hosts (Telegram private chat, Chibigochi, conformance drivers) behave exactly as before; no scenario expectations changed.
- Agents still cannot write Core truth directly; this change only narrows one automatic path.
- Sleep-material speaker/reply markers, evidence-pack speaker rendering, `recall_by_event_id`, and `CoreFact.subject` remain open roadmap work (branches 1b-3).

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_multi_speaker`
- `memory_view_attributes_speakers_and_drops_current_speaker_duplicate`
- `python tests/host_conformance/host_conformance.py --host direct-multispeaker`

## 2026-07-02 - Prompt view gains derived time labels; relative time is never stored

The rendered memory view previously carried no notion of time: the model saw neither "now" nor the age of memories, so questions like "when did we talk about this?" were unanswerable and any "yesterday/today" in replies was hallucinated. This change adds prompt-facing time perception while keeping storage unchanged.

**What changed:**
- `render_memory_view` now emits a `current_time:` line inside `<state>` (local date, time, weekday, UTC offset) as the model's reference point.
- Archive items in `<long_memory>` carry a derived age label from `RecallItem.time_range.end`: `- [yesterday | 0.88] ...`; ladder: today / yesterday / N days ago / earlier this month / last month / N months ago / over a year ago (calendar-day buckets, not 24h windows).
- Older active dialogue in `<short_memory>` gets one `[yesterday]`-style day marker per calendar-day group; the fresh recent tail stays unlabeled.
- `CoreContextRequest` and `CoreContextPackage` gain optional `utc_offset_minutes` (default 0) and `clock_untrusted` (default false); serde defaults, no migration. The Telegram host now passes its local offset.
- With an untrusted clock, an unparseable `created_at`, or a timestamp in the future relative to now, labels are omitted instead of rendered wrong; future timestamps add a package note.
- Relative labels are never persisted: they are computed at render time from stored absolute timestamps and package `created_at`, so a long-idle store needs no startup refresh (same derive-don't-store principle as query-time effective freshness).

**What is retracted (if applicable):**
- Nothing is retracted. Prior renders were time-blind by design gap, not by claim.

**What is still true:**
- Storage formats are unchanged; all timestamps were already absolute UTC.
- The core still performs no network I/O; time is the injected package `created_at` or the system UTC clock. Any network-time correction is a host concern (opportunistic `Date`-header comparison on traffic the host already makes), never a library one.
- Token budget stays honest automatically: budget estimators share the same prompt-line render functions, so label costs are counted.

**What we are doing:**
- Live check on the Telegram host ("when did I tell you about X?") is the owner's next step; deterministic behavior is covered by unit tests and host conformance.
- Design record: `wiki/pages/research/memory-time-perception-2026-07-02.md`.

**Reproducibility anchor:**
- `cargo test -p memory_engine` — `age_labels_follow_calendar_buckets_and_local_offset`, `memory_view_labels_archive_age_and_marks_older_dialogue_days`, `memory_view_relabels_same_package_when_rendered_later`, `memory_view_omits_labels_when_clock_is_untrusted`.
- All six host conformance scenarios pass unchanged.

## 2026-06-10 - v0.3 host conformance closed

v0.3 closes the reusable-host proof: Memory Engine now has automated conformance for the direct/local Python boundary, the Telegram bot host path without Telegram transport, and a real Godot 4.6 headless GDExtension host.

**What changed:**
- `wiki/pages/releases/v0.3-acceptance.md` now records the accepted v0.3 scope: direct/local, Telegram-local, and Godot-headless.
- `tests/host_conformance/host_conformance.py` provides the shared scenario for all accepted hosts.
- The Godot slice uses `crates/godot_adapter/` plus `hosts/godot_headless/` and was accepted with real Godot 4.6 stable console.
- `wiki/pages/research/vector-recall.md` is tracked as a research draft for opt-in vector recall, not an implementation-ready spec.

**What is retracted (if applicable):**
- Nothing is retracted. Telegram-live smoke and Telegram userbot support are clarified as deferred work, not v0.3 closure gates.

**What is still true:**
- The Rust core remains provider-agnostic and owns memory policy.
- Hosts may dispatch events, render/load prompts, execute `LlmRequest -> text`, and display results, but they do not decide memory extraction, validation, Core promotion, forgetting, or prompt-facing memory structure.
- v0.3 does not claim vector recall, benchmarked memory quality, polished Godot/Chibigochi integration, or Telegram userbot support.

**What we are doing:**
- Treat Telegram-live smoke as an operations/regression task, not a manual memory acceptance gate.
- Keep vector recall in research/alignment until it is rewritten around validated active `MemoryUnit`s, host-produced embeddings, core vector inputs, and buffered recall stats.
- Plan the next product-level work from the v0.3 menu: Chibigochi/Godot integration, diagnostics/backup, reviewer-pass, or vector-recall alignment.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check`
- `python -m pytest crates/python_adapter/tests -q`
- `python tests/host_conformance/host_conformance.py --host direct`
- `python tests/host_conformance/host_conformance.py --host telegram-local`
- `python tests/host_conformance/host_conformance.py --host godot-headless` with Godot 4.6 stable console

## 2026-06-10 - Ukrainian fallback literals and stop words restored

The post-audit review found that a small set of Ukrainian string literals had been mojibaked in an earlier Windows editing session. The broken literals affected Ukrainian stop-word filtering in recall/core near-duplicate checks and the human-readable fallback/preliminary archive prose used when LLM enrichment is unavailable.

**What changed:**
- `meaningful_tokens` now filters the intended Ukrainian stop words: `користувач`, `користувача`, `користувачу`, `дуже`, `любить`, and `цікавиться`.
- Preliminary and deterministic fallback archive text now uses valid Ukrainian prose instead of mojibake.
- Engine tests now assert that common Ukrainian user tokens are filtered and that fallback Ukrainian literals remain valid text.

**What is retracted (if applicable):**
- The implicit assumption that the Ukrainian stop-word list was active is retracted. The corrupted strings meant those words were not being filtered before this fix.

**What is still true:**
- The tokenizer and overlap algorithm are unchanged.
- English stop-word filtering is unchanged.
- LLM-enhanced archive records remain the preferred path; the restored fallback text only affects deterministic fallback/preliminary prose.
- This does not change storage schemas or public API contracts.

**What we are doing:**
- Keep Windows file edits on UTF-8-safe paths and protect critical non-ASCII literals with tests.

**Reproducibility anchor:**
- `ukrainian_stop_words_filter_common_user_tokens`
- `fallback_ukrainian_literals_are_valid_text`

## 2026-06-10 - Runtime sleep does not use the journal primitive yet

The post-v0.2 audit found that the architecture described `memory/journal/` as an active crash-safety mechanism for multi-file sleep operations, while the runtime implementation actually relies on atomic single-file writes, durable `SleepRun` checkpoints, and idempotent Complete-archive event coverage.

**What changed:**
- `wiki/pages/foundation/architecture.md` now states that the journal is a reserved storage primitive and future target for migration/recovery-heavy operations, not the active runtime sleep transaction layer.
- `wiki/pages/foundation/contracts.md` clarifies that `JournalOperation` is a reserved contract in v0.2: the schema and FileStorage methods exist, but runtime sleep/recovery do not consume journal records.
- `wiki/pages/planning/audit-2026-06-10.md` marks A3 closed as a scope decision rather than an open ambiguity.
- `wiki/pages/planning/roadmap.md` removes A3 from the pre-v0.3 open cleanup list.

**What is retracted (if applicable):**
- The earlier architecture claim that runtime sleep currently creates and replays `memory/journal/` records for crash safety is retracted.

**What is still true:**
- `JournalOperation` remains a valid schema and Storage primitive.
- `manifest.json` still tracks the journal schema version.
- Runtime sleep remains crash-resilient through persisted `SleepRun` cursors, pending tasks, atomic file writes, and idempotent event coverage.
- Journal-based recover/rollback remains appropriate for future schema migrations or operations that need explicit transaction policy.

**What we are doing:**
- Defer wiring journal replay until a concrete migration or recovery-heavy operation needs it.
- Avoid presenting unused primitives as active guarantees.

## 2026-06-10 - Session metadata now caches Complete-archive event coverage

The post-v0.2 audit found that `core_context_package` and sleep selection still scanned all archive files just to know which raw session events were already covered by Complete archives. That made every turn pay for the full archive history even after raw-event rotation moved old events out of active `events.jsonl`.

**What changed:**
- `SessionMetadata` now includes `archived_event_ids` and `archived_event_index_complete` alongside `archived_to`.
- Successful sleep completion records the Complete archive id and covered raw `event_id`s in `session.json`.
- `core_context_package` and sleep selection read the session metadata cache instead of scanning all archive entries on the hot path.
- Legacy or incomplete metadata self-heals: when `archived_event_index_complete` is false, the engine rebuilds the cache from Complete archive entries and writes it back.
- `Storage` exposes `read_session_metadata` and `write_session_metadata` so the engine, not hosts, owns this cache.

**What is retracted (if applicable):**
- Nothing is retracted. The raw event files and Complete archive entries remain the source of truth; the new fields are a rebuildable cache.

**What is still true:**
- `events.jsonl` remains the active raw-event tail.
- `archived/events-<NNN>.jsonl` remains the archived raw-event source for evidence/audit paths.
- Complete `ArchiveEntry.source_event_ids` remains the canonical coverage record.
- `session.md` remains the full human-readable session journal.

**What we are doing:**
- Treat missing or incomplete archived-event cache fields as legacy metadata and rebuild them from archive files.
- Keep future indexed storage as a separate scaling decision if archive volume demands it.

**Reproducibility anchor:**
- `engine_context_rebuilds_legacy_archived_event_index`
- `engine_sleep_run_driver_finishes_archive_and_seeds_core`

## 2026-06-10 - Architecture docs no longer claim Archive/Core Markdown dumps are current storage

The post-v0.2 audit found that `wiki/pages/foundation/architecture.md` still described `memory/archive/<YYYY>/<MM>/<spogad_id>.md` and `memory/core/store/<category>.md` as current storage files. The implementation writes pretty JSON for Archive/Core and only maintains `session.md` as a Markdown human log.

**What changed:**
- `wiki/pages/foundation/architecture.md` now states that Archive/Core use pretty-printed JSON as the current human-inspectable form.
- `wiki/pages/foundation/contracts.md` now lists only Archive/Core JSON files as the current storage contract.
- Archive/Core Markdown exports are explicitly deferred until a real workflow needs them.
- The architecture document was also synchronized with current v0.2 behavior for host-owned prompt loading, pull-based LLM requests, fine-grained in-process locking, raw-event rotation, tolerant collection reads, and the implemented reflection/forgetting lifecycle.
- `wiki/pages/planning/roadmap.md` now reflects the actual post-v0.2 state and marks implemented partial sleep / active-tail behavior as complete.

**What is retracted (if applicable):**
- The prior architecture claim that Archive entries have `<spogad_id>.md` files and Core categories have `<category>.md` files as part of the current storage layout is retracted.

**What is still true:**
- Files remain the source of truth.
- `session.md` remains the human-readable session journal.
- Archive/Core remain inspectable without a database through pretty JSON files.
- Rust core still does not know providers, models, keys, `prompts_dir`, or network I/O.

**What we are doing:**
- Keep Markdown exports as a deferred derived view, not as a second source of truth.
- Continue the post-v0.2 audit queue with the remaining scaling/modularity items.

## 2026-06-10 - Collection reads skip unreadable memory files with visible notes

The post-v0.2 audit found that one malformed JSON file in a collection directory such as `archive/`, `archive/units/`, `core/store/`, `core/candidates/`, `tasks/`, or `runs/sleep/` could make the whole collection read fail. That meant one bad archive file could break recall and context packaging until a human found and repaired it.

**What changed:**
- Storage collection reads now return `StorageCollection<T> { items, warnings }`.
- `FileStorage` skips unreadable files when scanning archive entries, memory units, core-store categories, candidate beliefs, pending tasks, and sleep runs.
- Point reads by id remain strict: if a command asks for a specific archive, unit, candidate, task, or sleep run, read/parse failure still fails that operation.
- `RecallResult.notes`, `RecallDebug.notes`, and `CoreContextPackage.notes` surface skipped-file warnings.
- Telegram `/recall` displays recall notes so the owner can see that a memory file needs manual repair.

**What is retracted (if applicable):**
- The previous implicit behavior "any unreadable collection member aborts the whole memory read" is retracted. Collection reads now degrade softly.

**What is still true:**
- The engine does not delete, rename, or auto-repair owner data.
- Strict point reads still preserve precise failures for operations that target one object.
- Healthy files continue to participate in recall, context packaging, reflection, and maintenance flows.

**What we are doing:**
- Continue the audit queue with the remaining scaling/documentation items before v0.3 adapter work.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_recall_and_context_skip_unreadable_archive_files_with_notes`
- `cargo test -p memory_engine --test engine_sleep_recall engine_context_skips_unreadable_core_store_files_with_notes`

## 2026-06-10 - Recall stats are buffered instead of written on every recall

The post-v0.2 audit found that `recall()` rewrote archive entry files on every returned memory just to increment `recall_count` and `last_recalled_at`. That made recall a write-heavy operation and created avoidable write-back races around archive files.

**What changed:**
- `MemoryEngine` now buffers recall stat deltas in memory.
- Recall scoring includes pending buffered deltas, so repeated recalls can still protect a memory before the next disk flush.
- `recall()` no longer calls `update_archive_entry` for every selected archive item.
- Added `flush_recall_stats()` to write buffered `recall_count` / `last_recalled_at` updates in batches.
- `finish_sleep_run` calls `flush_recall_stats()` as a natural consolidation point.
- `RecallStage1Config` now includes `stats_flush_interval` for periodic automatic flushing; `0` disables interval flushing.
- PyO3 exposes `flush_recall_stats()` for host shutdown or maintenance.

**What is retracted (if applicable):**
- Nothing is retracted, but the precision of recall counters is explicitly advisory. Unflushed deltas can be lost if the process crashes before a flush.

**What is still true:**
- Stored archive entries still contain `recall_count` and `last_recalled_at`.
- Recall ranking still uses the same count/recent-recall boost formula; it now uses persisted values plus pending deltas.
- Core memory, archive content, and memory units are not affected by losing an unflushed advisory counter delta.

**What we are doing:**
- Continue the audit queue with engine module split and tolerant collection reads before v0.3 adapter work.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_recall_buffers_stats_until_flush_and_scores_with_pending_counts`

## 2026-06-10 - Completed sleep rotates archived raw session events

The post-v0.2 audit found that long-lived host sessions kept all raw events in one active `sessions/<session_id>/events.jsonl` file. Even after those events were covered by Complete archives, every `read_session` call still parsed the full raw chat history.

**What changed:**
- `Storage` now exposes `read_session_archived_events(session_id)` and `rotate_session_events(session_id, covered_event_ids)`.
- `FileStorage` writes rotated raw events to `sessions/<session_id>/archived/events-<NNN>.jsonl`.
- `finish_sleep_run` rotates events covered by Complete archives after archive completion, memory-unit creation, Archive-to-Core bridge, and auto-fidelity routing.
- `read_session` remains active-file only; old source events remain available through archived segments.
- Core bridge and evidence-pack construction now read active + archived session events with `event_id` deduplication, so old units remain verifiable after rotation.

**What is retracted (if applicable):**
- Nothing is retracted. This is the first step of audit A4, not the full A4 closure: recall-counter isolation and the cached `archived_event_ids` session metadata path remain open.

**What is still true:**
- `session.md` remains append-only and human-readable; it is not rotated by this change.
- Archive entries and memory units remain the canonical compressed memory.
- If a process crashes after writing an archived segment but before rewriting the active file, duplicate raw events may temporarily exist across active and archived files; engine evidence reads deduplicate by `event_id`.

**What we are doing:**
- Continue the audit queue with recall-counter isolation (B3), then engine module split and tolerant collection reads.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall`

## 2026-06-10 - SleepRun recovery is persisted in the core

The post-v0.2 audit found that multi-pass sleep orchestration kept `SleepRun` only in process memory. If the host crashed mid-sleep, durable `PendingTask` files could remain pending while no public API could reconstruct the run cursor, blocking future sleep for that session.

**What changed:**
- `Storage` now persists `SleepRun` checkpoints under `memory/runs/sleep/<sleep_task_id>.json`.
- `begin_sleep_run`, `next_sleep_batch`, `submit_sleep_batch`, and `finish_sleep_run` checkpoint the run as it advances.
- The core exposes `pending_sleep_runs()` and `cancel_sleep_run(sleep_task_id)` through Rust and PyO3.
- The Telegram host queues recovered sleep runs at startup before polling.
- `resume_sleep_compression` and `resume_memory_unit_pass` are idempotent for already-completed archive/unit artifacts, so recovery can safely re-enter `finish_sleep_run`.

**What is retracted (if applicable):**
- The prior architecture wording implied that the journal was already active for sleep lifecycle recovery. It was not. Runtime sleep recovery now relies on durable `SleepRun` checkpoints plus idempotent event coverage; the journal remains a deferred primitive for future multi-file transactions.

**What is still true:**
- The Rust core still performs no network I/O and knows no provider/model/key.
- `PendingTask` remains the durable LLM work item; `SleepRun` is the durable orchestration cursor over those work items.
- `cancel_sleep_run` does not mark session events archived. A cancelled run leaves its preliminary archive non-complete, so the same source events remain eligible for the next sleep.

**What we are doing:**
- Continue the audit queue with raw-session scaling work (A4) and recall counter isolation (B3) before v0.3 adapter work.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_sleep_run_persists_and_recovers_after_restart`
- `cargo test -p memory_engine --test engine_sleep_recall engine_cancel_sleep_run_unblocks_unarchived_events`

## 2026-06-10 - Telegram background sleep uses the shared engine

The post-v0.2 audit found that the Telegram host created a second `MemoryEngine` inside `SleepRunner._run()` for background sleep. Because `LockRegistry` is per engine instance, that bypassed the concurrency guarantees added in v0.2 and could allow lost updates between normal chat commands and background sleep completion.

**What changed:**
- `hosts/telegram_gemini_bot/bot.py` now passes the main `MemoryEngine` into `SleepRunner`.
- Background sleep completion calls `complete_sleep_result()` with that shared engine instead of constructing a new one over the same `runtime/memory` directory.
- The Telegram host now constructs `memory_engine.MemoryEngine(...)` only once, in `main()`.

**What is retracted (if applicable):**
- The implied post-concurrency assumption that all Telegram host memory writes already shared one lock registry. They did not for background sleep until this fix.

**What is still true:**
- The Rust core lock model and lock ordering are unchanged.
- The core still supports many threads through one engine instance; it still does not provide multi-process file locking over the same memory directory.
- Manual `/sleep` already used the main engine and is unchanged.

**What we are doing:**
- Continue the audit queue with persistent `SleepRun` recovery and the explicit journal decision before starting v0.3 adapter work.

**Reproducibility anchor:**
- `Select-String -Path hosts\telegram_gemini_bot\bot.py -Pattern 'MemoryEngine\('` returns one constructor call.
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`

## 2026-06-01 — v0.2 living memory cycle acceptance

v0.2 is ready to close as an end-to-end living-memory cycle in the reusable Rust core. This entry records the acceptance anchor; it does not claim external benchmark quality.

**What changed:**
- Added `wiki/pages/releases/v0.2-acceptance.md`.
- Added deterministic integration test `crates/memory_engine/tests/living_memory_cycle.rs`.
- The test composes the full core lifecycle: `ingest -> sleep driver -> Archive + MemoryUnit -> recall/context -> fidelity -> reflection candidate -> manual Core promotion -> contested Core -> forget_review -> remember_back`.
- The acceptance path uses injected `LlmResponse` values, so the cycle is reproducible without a live provider.

**What is retracted (if applicable):**
- No quality or benchmark claim is made for v0.2. The release claim is only that the living-memory lifecycle is implemented and regression-tested end to end.

**What is still true:**
- The core still does no network I/O and knows no provider/model/key.
- Hosts still execute a single primitive: `LlmRequest -> text`, then submit the result back to the core.
- Agents still cannot write Core truth directly; Core mutation happens through explicit bridge/gating or manual review paths.

**Known limitations / deferred work:**
- Vector storage remains opt-in future work.
- Core candidate reviewer/formulation pass remains deferred.
- Unit-level recall counters are not implemented; forgetting v1 uses archive-level recall proxy.
- Per-scope Core store layout remains an optimization if shared category locks become a bottleneck.
- Partial sleep/session-tail strategy remains deferred.
- Public quality claims require a separate benchmark harness and reproducibility anchor.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test living_memory_cycle`
- Full release gate is documented in `wiki/pages/releases/v0.2-acceptance.md`.

## 2026-06-01 — Reversible forgetting review for low-signal memory units

Memory units can now be marked as forgotten through a conservative review path. This changes prompt-facing recall behavior, because forgotten units are excluded from compact memory projections, while the full archive and audit trail remain on disk.

**What changed:**
- Added `TaskType::ForgetReview`, `forget_review_pass`, and core APIs for `begin_forget_review`, `submit_forget_review_response`, `list_forgotten_memory_units`, and `remember_back`.
- Added `MemoryUnit.forget_review` audit metadata and reversible `MemoryUnitStatus::Forgotten` application.
- Forgetting uses a triple gate: structurally eligible old/low-signal unit, LLM recommendation, and a hard engine protection re-check at submit time.
- Protected units are not forgotten even if the model says `forget`: Core-linked, high-weight, emotionally strong, or recently recalled archive-linked units stay active.
- Parent `ArchiveEntry.compact_memory` is rebuilt after both forget and remember-back, so prompt-facing memory matches unit status.

**What is retracted (if applicable):**
- Nothing is retracted. The earlier `forgotten/` directory remains future scaffolding; v1 intentionally does not physically move archive files.

**What is still true:**
- Core facts are not touched by forgetting.
- Full archive entries remain stored and auditable.
- Unit-level recall counters do not exist yet; v1 uses archive-level `recall_count` / `last_recalled_at` as a conservative proxy, which may protect routine sibling units when the archive was recalled for a more important unit.

**What we are doing:**
- Review the feature branch before merge. Scratch live-checks passed with cached Gemini: one old routine unit was recommended as `forget` and became `Forgotten`; richer live-check confirmed routine forgotten, Core-linked unit protected at submit re-check, high-weight unit stayed active, and `remember_back` restored the forgotten thesis into `compact_memory`.
- Add unit-level recall counters later only if archive-level proxy proves too conservative.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest crates\python_adapter\tests -q`
- Scenario tests: `engine_forgetting.rs`
- Scratch live-check 1: `forget_review_pass`, role `balanced`, model `gemini-2.5-flash`, prompt 580 / output 110 / total 810, result `forgotten=1`.
- Scratch live-check 2: `forget_review_pass`, role `balanced`, model `gemini-2.5-flash`, prompt 758 / output 176 / total 1339, result `forgotten=1`, `protected=1`, routine compact removed after forget and restored after `remember_back`.

## 2026-06-01 — Approved reflection candidates can contest existing Core facts

The Core can now adapt without silently overwriting old truth. When a reviewed reflection candidate explicitly contradicts an existing Core fact, approving that candidate marks the older fact as `contested` and preserves it with provenance instead of deleting or replacing it.

**What changed:**
- `ReflectionCandidateDraft` and `CandidateBelief` now support `contradicted_core_fact_ids`.
- `CandidateReviewResult` now returns `contested_facts`.
- `review_candidate(approved)` marks active same-scope Core facts listed in `contradicted_core_fact_ids` as `CoreFactStatus::Contested`, tags them, and links them to the candidate through `contested_by_candidate`.
- The promoted candidate still becomes an active Core fact only after explicit review and keeps `source_candidate_id`.
- `CoreContextFact` now includes `status`; `core_context_package` includes active and contested Core facts, and `render_memory_view` marks contested facts in prompt-facing Core memory.
- `reflection_analyze.md` now instructs the model to use `contradicted_core_fact_ids` only for real contradictions, not harmless refinements.

**What is retracted (if applicable):**
- The prior HISTORY note that contested logic remained future work is now superseded for the manual candidate-review path. A standalone `engine.contest_core_fact`, automatic contested detection outside candidate review, and richer contested-resolution UX remain future work.

**What is still true:**
- Reviewer/reflection agents still cannot write Core directly. They can only propose candidates and contradiction references.
- Core mutation still requires explicit review (`review_candidate(approved)`) or an existing owner/admin patch path.
- Contested facts are preserved as audit trail and remain visible to prompt assembly with a contested marker.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_reflection`
- `cargo test -p memory_engine --test engine_sleep_recall`
- `cargo fmt --check`
- `cargo clippy -p memory_engine --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- Live Gemini check on 2026-06-01: scratch memory seeded `The user lives in Berlin.`, a validated memory unit said the user moved back to Kyiv and Berlin was outdated, `reflection_analyze` on `gemini-2.5-pro` returned a candidate with the old `core_fact_id` in `contradicted_core_fact_ids`, and `review_candidate(approved)` produced one `contested` old fact plus one active promoted Kyiv fact.

## 2026-06-01 — Reflection candidates require manual review before Core promotion

Reflection Phase C starts the controlled path from validated memory units to Core. The core can now ask a host to run `reflection_analyze`, store candidate beliefs, list them, and promote or reject them by explicit owner review. Agents still cannot write Core directly.

**What changed:**
- Added `reflection_result.v1`, `candidate_review_input.v1`, and `candidate_review_result.v1`.
- Added `TaskType::ReflectionAnalyze` execution path through `begin_reflection_analysis` and `submit_reflection_response`.
- Added candidate storage read/list methods for `memory/core/candidates/<candidate_id>.json`.
- `CandidateBelief` now records `source_session_id`, `core_scope`, `source_memory_unit_ids`, `tags`, and optional `promoted_core_fact_id`.
- Added manual candidate review via `review_candidate`: approved candidates are promoted to Core with `source_candidate_id`; rejected candidates stay rejected.
- Telegram host exposes `/reflect`, `/candidates`, `/confirm <id>`, and `/reject <id>`.

**What is retracted (if applicable):**
- The prior HISTORY notes that candidate beliefs and `/reflect` remained future work are now superseded for the first manual-review iteration. Core candidate reviewer/formulation pass, contested logic, auto-confirm, and forgetting remain future work.

**What is still true:**
- The core still has no provider, model, key, prompt directory, or network dependency.
- Reflection reads only validated `MemoryUnit` material for candidate generation.
- Core changes still require explicit review in this iteration; no LLM response directly mutates Core.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_reflection`
- `cargo test -p memory_engine --test engine_sleep_recall`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- Telegram live-check on 2026-06-01: `/candidates` returned no candidates before `/reflect`; `/reflect` scanned 7 memory units and 37 Core facts, then created `candidate_1780300762881461500_3` as `ready_for_review`; `/candidates` listed the same candidate, and Core remained unchanged without `/confirm`.

## 2026-05-31 — Fidelity validation is auto-routed after sleep

The first manual fidelity validator proved useful, but it still required a human/debug command. This change makes the core route only selected memory units to validation after sleep: high-weight units, configured high-risk tag classes, and units whose source events also feed Archive-to-Core personal signals.

**What changed:**
- `SleepOutcome` now includes `fidelity_requests: Vec<LlmRequest>`.
- `finish_sleep_run` creates `MemoryFidelityPass` tasks for selected memory units after memory-unit creation and Archive-to-Core bridge.
- `FidelityConfig` now has `auto_validate_after_sleep`, `auto_validate_weight_threshold`, and `auto_validate_tags`.
- Telegram host executes returned `fidelity_requests` through the same prompt -> text primitive and submits responses back to the core. It does not decide which units need validation.
- Tests cover both sides of the routing policy: low-weight routine units are not validated, while low-weight Core-path units are validated.

**What is retracted (if applicable):**
- The 2026-05-31 note that automatic high-risk/high-weight routing remained a later step is now superseded. Candidate beliefs and Core promotion are still not implemented.

**What is still true:**
- Reviewer agents still do not write truth directly into Core.
- The core still has no provider, model, key, prompt directory, or network dependency.
- Fidelity validation updates `MemoryUnit` status only; candidate beliefs, `/reflect`, `/confirm`, `/reject`, and forgetting remain future v0.2 work.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-31 — Memory fidelity validation uses evidence packs

Reflection Phase B begins with an explicit validation boundary: compressed memory units can now be checked against a small evidence pack before they are trusted for critical paths.

**What changed:**
- Added `evidence_pack.v1`, built by the core from a memory unit's `source_event_ids`, configured neighbor events, unit thesis/evidence, and a token budget.
- Added `fidelity_review.v1`, stored on `MemoryUnit` as `fidelity_review`.
- Added `TaskType::MemoryFidelityPass` with `role_hint: reasoning` and prompt `memory_fidelity_pass`.
- Added core methods to build evidence packs, begin fidelity validation, submit validator responses, and persist review status.
- Added Telegram debug commands `/evidence <memory_unit_id>` and `/fidelity <memory_unit_id>`.
- Fidelity results can mark memory units as active, rejected, or needing revision without writing directly to Core.

**What is retracted (if applicable):**
- Nothing. This implements the planned validator boundary; it does not claim full reflection or automatic Core promotion is complete.

**What is still true:**
- Reviewer agents do not write truth directly into Core.
- Core still has no provider, model, key, prompt directory, or network dependency.
- Automatic routing of only high-risk/high-weight units to validation remains a later step; the current Telegram path is manual/debug.
- Candidate beliefs, `/reflect`, `/confirm`, `/reject`, and full forgetting remain future v0.2 work.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Recall uses time decay and recall feedback

Archive recall now computes an effective freshness at query time instead of treating stored `freshness` as permanently current. Old archive memories sink in rank unless text/theme/tag relevance or recall feedback keeps them useful.

**What changed:**
- `RecallStage1Config` now has configurable freshness decay and recall feedback knobs: `freshness_half_life_days`, `recall_count_log_bonus`, `recent_recall_bonus`, `recent_recall_half_life_days`, and `max_recall_boost_factor`.
- Stage 1 archive scoring uses `effective_freshness = stored_freshness * time_decay(age)` and a bounded recall boost from `recall_count` and `last_recalled_at`.
- Recall scoring is now explicitly time-dependent: `query.created_at` is the deterministic reference time for decay and recent-recall feedback; when hosts omit it, the engine falls back to current UTC time.
- `RecallResult.items[].freshness` now reports effective prompt-time freshness for the recalled item.
- Added regression tests for old-memory decay and recall-feedback boosting.

**What is retracted (if applicable):**
- The previous claim that `freshness` affected recall was incomplete: it was only a stored scalar and did not decay with time.
- `recall_count` and `last_recalled_at` were recorded but did not influence ranking.

**What is still true:**
- This is not full forgetting. No archive entry is deleted or moved to `forgotten/`.
- Full agentic forgetting (`forget_review_pass`) remains a later v0.2 step with audit trail and restore path.
- The same recall query, including the same `created_at`, against the same archive data remains deterministic.
- The Rust core still uses deterministic Stage 1 scoring here; no provider, model, prompt, or network dependency is added.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_recall_`

## 2026-05-30 — Context budget uses prompt-shaped memory estimates

Context budgeting now estimates Core, Archive, and Session items by the compact prompt-facing lines produced by the core prompt-view renderer, not by their full storage/debug JSON structs. This removes storage-only overhead such as `core_fact_id`, `scope`, `tags`, JSON braces, and audit fields from prompt budget decisions.

**What changed:**
- `apply_context_token_budget` now uses prompt-shaped estimators for Core facts, archive memories, recent dialogue, and trace dialogue.
- Archive budget is aligned with the prompt-view archive display limit.
- Added a regression test proving short Core facts with very large storage-only tags are kept because those tags are not part of the rendered prompt memory.

**What is retracted (if applicable):**
- The previous budget estimate over-counted prompt memory by measuring storage JSON. That made Core appear much more expensive than the model-facing prompt actually was.

**What is still true:**
- The estimator remains deliberately conservative (`unicode_chars_div_2_ceil_json_v1` / rendered text chars divided by two).
- The 11k/7k/3k/1k budget split remains unchanged.
- Query-aware ranking from the previous entry still decides which facts should be kept first when the compact Core budget is still full.

**Reproducibility anchor:**
- Runtime check for query `А кішка?`: Core facts kept increased from 7 to 23, dropped Core facts decreased from 22 to 6, and the `pet` fact for Іржа remained first.
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Query-aware Core fact budgeting

Core Store facts are now ranked against the current query before the 1k Core memory budget is applied. Previously, active Core facts were sorted mostly by confidence and category, so when many facts had the same confidence, a directly relevant fact could be dropped only because its category sorted later.

**What changed:**
- `core_context_package` now preserves query-relevant Core facts ahead of unrelated same-confidence facts before token-budget trimming.
- Added a regression test proving that a relevant `pet` Core fact survives a tight Core budget for the query `А кішка?`.

**What is retracted (if applicable):**
- The implicit assumption that confidence-only Core ranking was enough for prompt budgeting. It is not enough once Core contains many equally confident facts.

**What is still true:**
- Core admission rules are unchanged.
- Core token budget remains 1k by default.
- This is not a hardcoded pet/cat rule; it is a general query-token relevance ranking for Core facts.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_core_context_package_keeps_query_relevant_core_fact_under_budget`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Core-owned prompt memory view

The ordinary chat prompt no longer relies on Telegram-specific Python code to decide how memory layers should be rendered. The Rust core now exposes `render_memory_view(package, current_user_message)` and returns the compact LLM-facing memory block with explicit layer geometry: `core_memory`, `long_memory`, `short_memory`, `current_user_message`, and `assistant_response_slot`.

**What changed:**
- Added `crates/memory_engine/src/prompt_view.rs` with canonical compact prompt rendering.
- Exposed `memory_engine::render_memory_view` through the PyO3 adapter.
- Changed the Telegram bot so `chat_prompt(...)` delegates to `engine.render_memory_view(...)` instead of owning memory projection helpers.
- Changed token telemetry so sleep metrics estimate the core-rendered prompt memory view rather than the old Python compact projection.
- Removed the old Telegram host compact projection helpers from the ordinary prompt/debug path.
- Added Rust and Python adapter tests for the prompt-view boundary.

**What is retracted (if applicable):**
- The previous open debt that prompt-view policy lived in Telegram host code. That policy now belongs to the core. The host still owns provider/model/key, prompt files, network execution, and Telegram UX.

**What is still true:**
- `core_context_package` remains the full API/debug shape.
- Debug/admin commands may still show IDs and audit metadata.
- Ordinary chat prompt memory should stay compact and should not carry long technical IDs unless a user command needs them.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --check`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Engine supports shared-reference calls and resource-scoped write locks

After the pull-based sleep driver moved memory orchestration into the core, the next strategic risk was concurrency. The engine still exposed many public calls as `&mut self`, the Python class was marked `unsendable`, and the file-backed Core store had a real lost-update risk because `core/store/<category>.json` is shared by all scopes.

**What changed:**
- `Storage`, `FileStorage`, and public `MemoryEngine` methods now use shared references for normal operations.
- `manifest_initialized` is now atomic and guarded by a manifest resource lock.
- Added an internal resource lock registry for `session:<id>`, `core:<category>`, and manifest operations.
- `ingest()` serializes writes only within the same session; different sessions can run independently.
- Core read-modify-write operations are serialized per category, including Archive-to-Core bridge writes.
- The Python adapter removed `unsendable` and wraps engine calls in `py.allow_threads(...)`.
- Added `concurrency_stress.rs`, including a quick parallel gate and an explicit 1000-session release stress test for Core lost-update detection.

**What is retracted (if applicable):**
- The earlier implicit assumption that a single mutable engine instance was acceptable for future adapters is no longer true. The reusable-library goal requires shared-reference calls and resource-scoped synchronization.

**What is still true:**
- The core still performs no network I/O and knows no provider, model, key, or prompt directory.
- The storage format remains file-based and human-inspectable.
- Core facts are still stored in per-category files for now; this change protects that layout with locks rather than migrating to per-scope directories.

**What we are doing:**
- Keep the explicit 1000-session stress test as the release gate for concurrency work.
- Revisit per-scope Core layout later if shared hot categories become a performance bottleneck.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --check`
- `cargo test -p memory_engine --release --test concurrency_stress -- --ignored` (1000-session stress)
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

**Thanks:**
- Mykyta Zagamula for pushing the project back toward a reusable, concurrent memory library before Phase B reflection added more complexity.

## 2026-05-30 — Consolidator now returns prose, not archive JSON

Live Telegram testing of the pull-based sleep driver showed that the core driver, fail-soft path, and Archive-to-Core bridge worked, but `sleep_consolidator` repeatedly failed schema validation. The root cause was the contract: the LLM was asked to return a full nested `sleep_compression_result.v1` JSON object even though the core already had validated track outputs.

**What changed:**
- `sleep_consolidator` now returns plain text using `GIST: ...` plus a narrative paragraph.
- Added `consolidator_text.v1` as the expected output schema for the consolidator request.
- `SleepRun` stores `consolidator_gist` and `consolidator_narrative` instead of a full `consolidated_result` JSON value.
- `finish_sleep_run` always assembles `SleepCompressionResult` deterministically from validated tracks, then overlays the LLM-provided gist/narrative when present.
- The fallback path now writes a neutral assembled narrative from available tracks instead of an error-like placeholder sentence.
- Added driver tests for both successful prose consolidation and fallback after three empty consolidator responses.

**What is retracted (if applicable):**
- The previous consolidator contract was too broad. Asking a prose model to reconstruct a full archive JSON object duplicated core responsibility and made rich sessions fragile.

**What is still true:**
- The core still owns archive structure, track validation, retry, fail-soft, task completion, and Archive-to-Core bridge.
- The host still only loads prompt text, calls a provider, and returns text or an error.
- If the consolidator fails, sleep still completes with full validated tracks and `consolidator_fallback` audit tags.

**What we are doing:**
- Re-run adapter tests and a short Telegram sleep check, then continue to the concurrency branch.

**Thanks:**
- Mykyta Zagamula for catching that "valid JSON from LLM" was the wrong abstraction at the final memory boundary.

## 2026-05-29 — Sleep orchestration moved into the core as a pull-based LLM driver

The owner challenged the project boundary: Memory Engine must be a reusable memory library, not a Telegram bot with memory logic embedded in the host. The Telegram host had accumulated sleep pass ordering, semantic retry, fail-soft fallback, JSON extraction, Archive-to-Core bridge, and task completion policy. That would force future Godot or third-party adapters to reimplement memory behavior.

**What changed:**
- Added core LLM boundary types in `crates/memory_engine/src/llm.rs`: `LlmRequest`, `LlmResponse`, `LlmBatch`, `SleepRun`, `SleepRunStep`, and `SleepOutcome`.
- Added pull-based driver methods: `begin_sleep_run`, `next_sleep_batch`, `submit_sleep_batch`, and `finish_sleep_run`.
- The core now owns sleep pass graph progression, JSON extraction from LLM text, schema validation, semantic retry state, fail-soft empty tracks, consolidator fallback, memory-unit completion, and Archive-to-Core seeding.
- The Python adapter exposes the new driver through JSON-string methods while keeping provider/model/API-key selection outside Rust.
- Telegram sleep execution now runs the generic driver loop and implements only the host primitive: load prompt text, call Gemini, return `{request_id, text}` or `{request_id, error}`.
- `seed_core_from_archives` moved Archive-to-Core bridge backfill into the core.
- `bot.py` dropped the old sleep pass orchestration helpers and shrank from roughly 2650 lines to roughly 1940 lines.

**What is retracted (if applicable):**
- The earlier practical shape of the host was too thick. The claim "hosts are thin adapters" was not structurally true while multi-pass sleep orchestration and Core seeding lived in `bot.py`.

**What is still true:**
- The core still does no network I/O and knows no provider, model, API key, or `prompts_dir`.
- Prompt files still live outside the Rust core; hosts load/render prompt text.
- Provider/network retry remains a host concern. The core handles semantic retry and memory-level fallback after the host returns text or an error.
- Existing direct `sleep()` and `resume_*` APIs remain available for compatibility while the driver becomes the preferred host integration path.

**What we are doing:**
- Next: concurrency hardening. Convert read paths toward `&self`, remove PyO3 `unsendable`, add per-session storage locks, and add a 1000-session stress test before Phase B reflection validation.

**Thanks:**
- Mykyta Zagamula for forcing the reusable-library boundary before Phase B added more host-owned memory intelligence.

## 2026-05-29 — File storage safety before core orchestration

Before moving orchestration, we hardened the file storage layer so the next refactor would not sit on known write and scan hazards.

**What changed:**
- Atomic writes now use unique temp paths in the target directory instead of a shared `.tmp` path.
- `session.md` is append-only and no longer rereads/replaces the full file on every event.
- Completed tasks move to `tasks/completed/`; pending task scans ignore completed tasks while `load_task(id)` can still find them for audit.
- High-frequency derived files avoid unnecessary fsync while durable event and archive/task/core writes remain durable.

**What is retracted (if applicable):**
- Nothing about the storage contract is retracted; this is a safety and scalability correction.

**What is still true:**
- `events.jsonl` remains the session source of truth.
- Completed tasks remain available for audit.

**What we are doing:**
- Use this as the storage baseline for the pull-based core orchestration branch.

**Thanks:**
- Mykyta Zagamula and Claude for identifying fixed temp paths, O(n²) session markdown writes, and unbounded hot task scans as early collapse risks.

## 2026-05-23 — Sleep pass failures no longer leave pending tasks stuck

A live Telegram test showed that a single specialized sleep pass can be blocked by Gemini safety/no-candidates while the other passes are fine. The failed `sleep_personal_signal_pass` left both `sleep_compression` and `compact_memory_pass` tasks pending, which blocked all future sleep for the session.

**What changed:**
- Telegram host now wraps `compact_memory_pass` and the four specialized sleep passes in fail-soft handlers.
- If one pass fails, the host logs the full error, records `pass_failed:<prompt_id>` in archive tags, and continues with an empty track for that pass.
- The sleep task can still complete through consolidator or fallback from remaining tracks.
- The stuck live pending task from `telegram_311422683` was repaired with the new code; all runtime tasks are completed again.

**What is retracted (if applicable):**
- The assumption that robust `sleep_consolidator` handling is enough. Individual upstream passes also need fail-soft handling because provider safety blocks can happen before consolidator runs.

**What is still true:**
- Core promotion still requires `personal_signals`; if `sleep_personal_signal_pass` fails, that archive will not seed Core from personal signals.
- Full traceback stays in `bot.log`; Telegram users should not see it directly.
- This is a host-side LLM reliability fix. Rust core still receives only valid completed results.

**What we are doing:**
- Continue long live testing and watch for `pass_failed:*` tags. If a pass fails often, adjust the prompt or provider/model policy for that pass.

**Thanks:**
- Mykyta Zagamula for running the long test that exposed the stuck pending task path.

## 2026-05-22 — Removed message-count sleep trigger

The owner rejected the message-count sleep path as a test-era shortcut that kept leaking into product planning. The product model has two sleep triggers: token/context budget pressure and scheduled idle sleep during a quiet time window.

**What changed:**
- Removed the message-count trigger configuration from `EngineOptions` and the Python adapter constructor.
- `engine.ingest()` now only stores the event and returns `IngestResult { schema_version, stored_event }`.
- Removed the GUI field and local harness argument for the old trigger.
- Telegram host now queues sleep from token pressure (`MEMORY_BOT_TOKEN_PRESSURE_RATIO`, default 0.80) or scheduled idle sleep (`MEMORY_BOT_IDLE_SLEEP_HOUR`, default 04:00; `MEMORY_BOT_IDLE_SLEEP_MIN_SECONDS`, default 1800).
- Current docs and README describe only `/sleep`, token-pressure sleep, and scheduled idle sleep.

**What is retracted (if applicable):**
- The prior claim that the message-count trigger should remain as a dev/test accelerator or emergency guard. It is removed from current behavior.

**What is still true:**
- `engine.sleep(session_id)` remains the only path that creates sleep tasks.
- Full archive storage, compact memory, token budget, and Archive -> Core bridge behavior are unchanged.
- Historical DEVLOG entries are not rewritten; they remain records of earlier decisions and are superseded by this entry.

**What we are doing:**
- Retest the Telegram host with wiped runtime memory and long human conversations so sleep is driven by token pressure or the idle schedule.

**Thanks:**
- Mykyta Zagamula for forcing the product sleep policy back to context limits and quiet-time consolidation instead of a convenient test counter.

## 2026-05-22 — Removed fixed compact-memory thesis quota

The first `compact_memory_pass` prompt incorrectly told the model to return 5-7 theses. That was a narrow quota, not a memory principle. It could force over-splitting a one-topic conversation or under-represent a session with many distinct memory units.

**What changed:**
- `prompts/compact_memory_pass.md` now tells the model to segment events into coherent memory units and return as many theses as the conversation actually supports.
- A one-topic conversation may produce one thesis.
- Multiple distinct episodes may produce multiple theses.
- Routine repetition can be omitted.
- Timestamps are explicitly available for ordering and time boundaries, but raw ISO timestamps should not be emitted unless useful.
- `wiki/pages/foundation/architecture.md` and `wiki/pages/foundation/contracts.md` no longer describe compact memory as a fixed 5-7 item output.

**What is retracted (if applicable):**
- The fixed "5-7 short theses" instruction and the related claim in the previous HISTORY entry.

**What is still true:**
- `compact_memory_pass` remains the prompt-facing compression layer.
- Full multi-track archive entries remain storage/debug records.
- Token budget and telemetry remain in force.

**What we are doing:**
- Treat numeric limits differently depending on their purpose: transport/UI/budget limits can be configured and measured; LLM memory reasoning should not be constrained by arbitrary thematic quotas.

**Thanks:**
- Mykyta Zagamula for catching the quota before it became another hidden architecture constraint.

## 2026-05-22 — Compact memory theses for archive recall

The owner flagged that the full multi-track archive is an audit/enrichment record, not the thing that should be carried back into every chat prompt. A real memory prompt needs short human theses: event -> conclusion. The archive now stores both forms separately.

**What changed:**
- Added `TaskType::CompactMemoryPass` and a persisted `compact_memory_pass` task alongside `sleep_compression` during sleep stage 1.
- Added `ArchiveEntry.compact_memory`, `SleepCompressionResult.compact_memory`, and `RecallItem.compact_memory`.
- Added `MemoryEngine::resume_compact_memory_pass(task_id, text)` and the Python adapter method with the same boundary.
- Added `prompts/compact_memory_pass.md`: plain-text compact theses, no JSON, no debug IDs.
- Telegram sleep now runs compact memory separately from the full multi-pass/consolidator archive enrichment.
- `archive_relevant` now prefers `compact_memory`: when present, recall returns compact theses instead of narrative/facts/quotes in the prompt-facing item.
- Token telemetry now distinguishes raw transcript, stored full archive, prompt archive payload, and compact memory ratios.

**What is retracted (if applicable):**
- The claim that a shortened JSON projection of `gist` plus selected `personal_signals` / `emotional_markers` is the right compressed memory for ordinary chat. It was smaller than full archive JSON, but it still mixed audit structure with prompt memory.

**What is still true:**
- Full multi-track archive entries remain canonical storage/debug records for audit, Core bridge, future reflection, and embeddings.
- The 11k/7k/3k/1k memory prompt budget still applies.
- Rust core still does not call any provider or choose a model.

**What we are doing:**
- Keep `compact_memory` as the ordinary prompt-facing archive layer.
- Keep full multi-track archive data out of normal chat prompts unless the user asks for debug/archive details.
- Defer `forget_review_pass` and vector recall to v0.2.

**Reproducibility anchor:**
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `maturin develop`
- `python -m pytest crates/python_adapter/tests`

**Thanks:**
- Mykyta Zagamula for separating real compression from audit enrichment and for insisting that token economy be measured, not assumed.

## 2026-05-22 — Local harness and session-scoped archive recall

The first local non-Telegram harness run exposed a real memory isolation defect: a brand-new local session could receive `archive_relevant` from older sessions. That made the bot answer with stale personal context such as the user's name before the new session had supplied it.

**What changed:**
- Added `hosts/telegram_gemini_bot/local_harness.py` for local conversation tests through the same Memory Engine, Gemini client, prompt builder, sleep flow, and Archive → Core bridge used by the Telegram host.
- Added `hosts/telegram_gemini_bot/run_local_harness.ps1`.
- `MemoryEngine::recall()` now treats `RecallQuery.session_id` as a session boundary for archive recall: when present, archive entries must have matching `source_session_id`.
- Global archive recall remains available only when `RecallQuery.session_id` is omitted or `null`.
- Added `engine_recall_with_session_id_does_not_leak_other_sessions` to prove a fresh session does not see another session's archive, while explicit global recall still can.

**What is retracted (if applicable):**
- The implicit assumption that archive recall in `core_context_package` could safely rank across all sessions. It could not: default chat context must be scoped to the current session to avoid cross-session contamination.

**What is still true:**
- Core facts remain isolated by `core_scope`.
- Archive entries still keep `source_session_id` and can be inspected globally for admin/debug flows.
- Host prompts still use compact prompt-facing projections rather than raw storage JSON.

**What we are doing:**
- Keep the local harness as a preflight tool, not a replacement for the real Telegram acceptance test.
- Add explicit acceptance coverage that a fresh session/chat must not inherit archive memories from another session/chat unless the caller deliberately requests global recall.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo fmt --check`
- `maturin develop`
- `python hosts/telegram_gemini_bot/local_harness.py --scenario mixed_short --turn-limit 4 --no-force-sleep-at-end`

**Thanks:**
- Mykyta Zagamula for insisting that tests must behave like real varied conversation and for challenging stale-memory behavior instead of accepting plausible replies.

## 2026-05-22 — Dialogue prompt geometry and free-category Core read path

The live Telegram logs showed repeated mid-dialog greetings and a Core context gap. The greeting issue was not stale memory: it happened before archive/Core were present in the prompt. The host was giving Gemini a compact JSON context package instead of a role-shaped dialogue, so the model treated turns too much like detached requests. Separately, Core facts were now written into free categories such as `name`, `pet`, and `age`, but Rust context assembly and patching still searched only the legacy `profile/preferences/relationship` list.

**What changed:**
- `Storage::read_core_store_categories()` now returns all Core Store category files.
- `FileStorage` reads every `memory/core/store/*.json` category file.
- `MemoryEngine::core_context_facts()` and `MemoryEngine::patch_core_fact()` now scan all stored categories, not `ContextPackageConfig.core_categories`.
- `ContextPackageConfig.core_categories` is retained only as a legacy seed list, not a whitelist.
- Telegram `chat_prompt()` now renders a role transcript (`user:` / `assistant:`), separates the current user message from prior context, and deduplicates `session_recent` from `session_trace` at the prompt boundary.
- Telegram archive prompt view is shorter: `gist`, a few personal signals, a few emotional markers, and compact topic/tone data instead of full narrative/quotes/debug payload.
- `prompts/telegram_chat_system.md` now explicitly forbids greetings inside an ongoing transcript unless the current user message is itself a greeting, and forbids confusing the assistant name with the user name.

**What is retracted (if applicable):**
- The claim that the previous compact JSON prompt was sufficient as the ordinary chat prompt. It was smaller than raw debug JSON, but it did not preserve dialogue geometry well enough.
- The implicit assumption that the legacy Core category list did not matter after free categories were introduced. It did matter: Core reads and patches still used it.

**What is still true:**
- `core_context_package.v1` remains the API/debug shape for hosts.
- Storage/debug files remain complete and auditable.
- Free Core categories remain allowed; trust still comes from user source, confidence, status, scope, and duplicate gates.

**What we are doing:**
- Retest a short live Telegram conversation for mid-dialog greeting regression.
- Verify `/core`, `/core_forget`, and normal chat answers now see facts stored in non-legacy categories.
- Keep the planned local non-Telegram conversation harness as the next step, after these fixes.

**Reproducibility anchor:**
- `python -m py_compile hosts/telegram_gemini_bot/bot.py`
- `cargo fmt`
- `cargo test --workspace`

**Thanks:**
- Mykyta Zagamula for noticing that the bot was losing conversation geometry and for pushing the token-economy requirement from architecture into actual prompt shape.

## 2026-05-21 — Live-test hardening and token telemetry

The live Telegram test proved the memory cycle works, but exposed concrete v0.1 defects: one invalid `sleep_consolidator` JSON left sleep unfinished, one Gemini safety block leaked a traceback to Telegram, personal-signal extraction still relied too much on narrow categories, and the host did not log enough token economics to prove compression savings.

**What changed:**
- `hosts/telegram_gemini_bot/bot.py` now logs Gemini provider `usageMetadata` for each model call into `runtime/logs/token_usage.jsonl` and short `token_usage` lines into `bot.log`.
- Chat turns now log estimated baseline tokens for raw-history-without-compression and estimated savings versus the compact Memory Engine prompt.
- Sleep now logs raw transcript estimated tokens versus compressed archive estimated tokens as `sleep_compression_metric` / `sleep_compression_tokens`.
- Chat prompts now use a compact prompt-facing projection without long storage/debug IDs.
- `sleep_consolidator` gets one JSON retry, then falls back to a complete archive assembled from the four successful specialized passes.
- Telegram error UX now hides tracebacks from users and categorizes Gemini no-candidates, safety, invalid key, rate/quota, and generic memory errors.
- `sleep_personal_signal_pass.md` was rewritten around criteria for stable user-grounded self-statements and free normalized categories, not a hardcoded category whitelist.
- `wiki/pages/foundation/architecture.md`, `wiki/pages/foundation/contracts.md`, `wiki/pages/integration/local-development.md`, and `wiki/pages/planning/roadmap.md` now describe the compact prompt, free categories, and token telemetry.

**What is retracted (if applicable):**
- The earlier category whitelist for Archive → Core bridge. Category mapping is now normalized free text; trust comes from source/confidence/duplicate gates, not from a closed topic list.
- The assumption that a successful provider call is guaranteed to return parseable JSON or user-visible text. The host must treat LLM output as unstable.

**What is still true:**
- Rust core still has no provider-specific tokenizer or network dependency.
- Storage/debug files stay full and auditable; compact projection only affects ordinary LLM prompt construction.
- Exact Gemini usage comes from provider `usageMetadata`; baseline-without-compression and raw→compressed sleep metrics are deterministic estimates unless we later add provider `countTokens`.

**What we are doing:**
- Retest the failure paths: invalid consolidator JSON fallback, Gemini safety/no-candidates UX, direct stable self-statements, and token telemetry in `token_usage.jsonl`.

**Reproducibility anchor:**
- `python -m py_compile hosts/telegram_gemini_bot/bot.py`
- `git diff --check`
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest crates\python_adapter\tests`

**Thanks:**
- Mykyta Zagamula for insisting that token economy must be measurable, not assumed, and for rejecting hardcoded category lists in prompts.

## 2026-05-21 — Compact prompt representation requirement

The owner flagged that long technical identifiers and verbose storage-shaped JSON can waste the same token budget the engine is supposed to preserve. The architecture now explicitly separates canonical storage/debug data from the compact representation that should be sent to an LLM prompt.

**What changed:**
- `wiki/pages/foundation/architecture.md` now states that storage/debug views may keep full IDs and metadata, but prompt-facing memory must be semantically sufficient and compact.
- `wiki/pages/foundation/contracts.md` now notes that `core_context_package.v1` is an API/debug shape, not necessarily the literal LLM prompt payload.
- `wiki/pages/planning/roadmap.md` adds an open v0.1 item for compact prompt representation.
- `wiki/pages/releases/v0.1-acceptance.md` adds acceptance checks that long IDs and debug metadata must not consume prompt budget unnecessarily.

**What is retracted (if applicable):**
- The implicit assumption that a full debug/API JSON package is acceptable as the literal prompt payload. It is useful for inspection, but too verbose for efficient live memory use.

**What is still true:**
- Canonical storage remains file-based, complete, human-inspectable, and audit-friendly.
- Technical IDs remain necessary for storage, linking, deduplication, commands such as `/core_update <id>`, and debugging.
- The 11k/7k/3k/1k budget still applies to the memory context that actually reaches the model.

**What we are doing:**
- Add a compact prompt projection or `prompt_view` mode before closing v0.1.
- Keep full IDs available in debug/admin paths, but remove them from ordinary chat prompt context unless required.

**Thanks:**
- Mykyta Zagamula for pointing out that verbose IDs directly conflict with the token-economy goal.

## 2026-05-20 — Core context package token budget

The owner defined the v0.1 memory prompt benchmark as a maximum of 11k tokens: 7k current memory, 3k compressed archive memory, and 1k Core. The engine now enforces that contract at the `core_context_package` boundary with a deterministic estimator and an explicit report.

**What changed:**
- `core_context_request.v1` accepts optional `token_budget` with `total_tokens`, `current_memory_tokens`, `compressed_memory_tokens`, and `core_tokens`.
- `core_context_package.v1` returns `budget`, including the estimator id, estimated layer usage, dropped item counts, and `budget_exceeded`.
- `ContextPackageConfig` now has a default `CoreContextTokenBudget` of 11k/7k/3k/1k.
- `core_context_package` trims active session context from the oldest side, archive memories by recall rank, and Core facts by confidence.

**What is retracted (if applicable):**
- Nothing is retracted, but the token count is not a provider-specific exact tokenizer result. It is a conservative deterministic estimate (`unicode_chars_div_2_ceil_json_v1`) until a host-level tokenizer is added.

**What is still true:**
- Rust core still has no provider, model, key, or network dependency.
- Hosts may still override event count limits and may pass their own token budget in the request.
- Stored session/archive/core files remain unchanged by this package-level trimming.

**What we are doing:**
- Keep the 11k/7k/3k/1k benchmark in acceptance tests.
- Add provider-specific exact token accounting later at host level if needed.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- Scenario test: `engine_core_context_package_enforces_token_budget_by_layer`

**Thanks:**
- Mykyta Zagamula for specifying the concrete 11k/7k/3k/1k memory benchmark and rejecting simple truncation as insufficient.

## 2026-05-17

- Added the first public `MemoryEngine` facade with `ingest()` for converting `IngestEvent` into `StoredEvent` and writing it through the configured `Storage`.
- Added deterministic event pre-scoring configuration through `EventScoringConfig`; no LLM provider, model, key, or prompt text is involved in this step.
- Added RFC3339 UTC timestamp generation for engine-owned `received_at` values.
- Added `MemoryEngine::sleep()` stage 1: selected session events now become preliminary `ArchiveEntry` records and `sleep_compression` pending tasks.
- Added `MemoryEngine::resume_sleep_compression()` for applying `sleep_compression_result.v1` to an existing archive entry.
- Added `MemoryEngine::recall()` stage 1 for archive recall by filters and text scoring.
- Added the local `memory_terminal` runner for manual live testing of ingest, sleep, tasks, and recall.
- Added the first real prompt file, `prompts/sleep_compression.md`, because `sleep_compression` is now a real pending LLM task.
