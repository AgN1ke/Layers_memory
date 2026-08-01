# Memory Engine Wiki Log

## Related pages

- [Wiki index](index.md) - main map of project knowledge pages.
- [Wiki rules](AGENTS.md) - maintenance schema this log follows.
- [DEVLOG](../DEVLOG.md) - working development diary for broader context.

This is the chronological maintenance log for the project wiki.

## [2026-08-01] acceptance | Non-degenerate graph fixture closes Phase 2

Recorded the completed Chibigochi host-path fixture that replaced the first
empty owner-data run as the implementation proof. Four candidates across three
scope forms, resolved and missing units, duplicate/new reinforcement, seven
deduplicated evidence links, exact-scope export, repeat idempotence, full
CandidateBelief equality and graph-only fallback all passed. Owner storage
remained unmigrated and unseeded.

The acceptance, roadmap and reflective-belief plan now mark Phase 2 closed
while preserving the original live `DATASET GAP` as history. They also retain
the dirty-worktree build provenance caveat and explicitly avoid turning this
storage PASS into a claim that Phase 3 reasoning or v0.5 live calibration is
complete.

## [2026-08-01] implementation | Server S1 identity and ordered-turn reference

Updated the accepted Google Play production track with its first implemented
reference slice. The wiki now distinguishes an internal authorized
`VaultContext` from any client-supplied identifier, documents vault-scoped
idempotency and monotonic same-session commit, and records the local
single-process handle used to prove restart and cross-vault isolation before
Storage v2/PostgreSQL.

Added a provisional S0 planning envelope and explicit owner-decision boundary.
The numbers drive engineering and the future 2x S7 workload but are not a
capacity or launch claim; retention, region and recovery objectives remain
subject to owner approval and measured closed-test replacement.

## [2026-08-01] acceptance | Empty graph contract fix and non-degenerate rerun

Recorded the first Chibigochi Phase 2 run as deterministic/product PASS with a
live dataset gap, not a full migration PASS: the complete owner copy contained
zero legacy candidates and reinforcements. The source remained untouched.

The run found that serde omitted empty public collection keys. Contracts now
require stable arrays for migration and export, and tests use direct key access.
The acceptance procedure now routes an empty live dataset to a separate
disposable fixture with multiple scopes, resolved/missing units and duplicate/
new reinforcement provenance. It explicitly forbids seeding synthetic beliefs
into owner memory.

## [2026-08-01] strategy | Server-scale Memory Runtime for Google Play

Added a primary-source research note and an accepted S0-S7 production plan for
centralized Chibigochi hosting. The wiki now separates the implemented local
`FileStorage` boundary from the planned authenticated multi-replica runtime:
server-derived per-account vaults, transactional Storage v2,
PostgreSQL/pgvector, durable workers, deterministic export/import and
whole-product load/isolation acceptance.

Foundation pages, roadmap, Chibigochi bridge documentation and navigation now
state that namespacing is not authorization, no server path may fall back to a
shared default user, provider calls happen outside database transactions, and
one vault has one authoritative backend. The update changes no runtime code and
makes no production throughput claim.

## [2026-08-01] acceptance | Chibigochi graph-storage migration gate

Added the v0.6 Phase 2 acceptance procedure for the implemented hypothesis
graph storage foundation. The procedure runs migration only on a stopped,
complete copy of Chibigochi owner storage; audits every legacy candidate and
MemoryUnit evidence edge; checks exact-scope export, idempotence, compatibility
projection, deterministic restart recovery, privacy and absence from the chat
prompt.

The gate deliberately does not test Phase 3 semantic hierarchy quality and
does not authorize migration of the live source storage, a version bump or a
release tag. The roadmap now links directly to this acceptance authority.

## [2026-08-01] architecture | Hypothesis graph Phase 2 storage foundation

Updated strategy-facing project knowledge after implementing the first v0.6
graph slice. Architecture and contracts now describe separate hypothesis node,
MemoryUnit evidence and relation records, deterministic evidence identity,
journaled/recoverable flat-candidate migration, exact-scope export and the
CandidateBelief compatibility projection.

The roadmap and phased plan distinguish deterministic implementation from the
still-open migration of a real Chibigochi storage copy. They also preserve the
behavioral boundary: migration creates no semantic relations, graph data does
not enter chat context, and hierarchical reflection/maturation remains Phase 3.

## [2026-08-01] integrity | Reflection Phase C.1 is recoverable

Implemented and documented the deterministic integrity closure that the prior
audit made mandatory. Reflection apply and candidate review now persist frozen
journal intent, revalidate live evidence, replay each desired side effect
idempotently after restart, and leave a text-free durable task outcome. Absent
scope is isolated, review scope is immutable, and terminal candidates cannot
re-enter review.

The wiki now distinguishes the closed deterministic C.1 gate from still-open
live Chibigochi acceptance. Roadmap Phase C/C.1 items are complete; the v0.6
hypothesis graph remains a separate next schema phase rather than being folded
into recovery work.

## [2026-08-01] strategy | Reliable reflection and hierarchical belief plan

Added `pages/planning/reflective-belief-system.md` as the phased authority for
closing v0.5 integrity before moving to a v0.6 hypothesis graph and v0.7
contradiction/clarification work. The plan records the target information
model, agent/core/host boundary, end-to-end flow, release dependencies,
acceptance gates, risks, and explicit non-goals.

Reopened autonomous reflection Phase C acceptance after the information-flow
audit found missing multi-file recovery, unsafe absent-scope behavior, and
review transition/scope gaps. Updated strategy, architecture, contracts,
roadmap, the Generative Agents research note, root/wiki navigation, HISTORY,
and DEVLOG. The research note now connects Generative Agents and Letta
sleep-time memory work with LangGraph/Temporal durable execution and Mem0's
comparison-before-write pattern.

The accepted cognitive direction preserves narrow and broad hypotheses as
separate related claims with explainable evidence strength, rather than
forcing every related wording into one flat candidate. No runtime schema or
behavior is claimed by this documentation-only decision.

## [2026-08-01] living memory | Autonomous reflection v0.5 Phase C

Implemented the typed result lifecycle from the normative v0.5 plan. New
reflection tasks can create one hypothesis, reinforce an existing candidate
with genuinely new evidence, open a reviewable Core challenge, or return a
structured no-insight outcome. Candidate reinforcement accumulates provenance
without duplicating beliefs, while Core remains owner-reviewed. The new schema
coexists with legacy saved-task responses. Deterministic acceptance is green;
live Chibigochi acceptance remains pending.

## [2026-08-01] living memory | Autonomous reflection v0.5 Phase B

Documented the shift from session-scoped reflection evidence to an explicit
personality boundary. `core_scope` now owns a durable list of host-bound source
sessions; trigger salience may accumulate across them without inferring account
links from text similarity.

The architecture and contracts now distinguish new trigger evidence from
bounded related history, define the non-vector structural selector and its
weights, include existing in-scope candidates, and preserve cross-session
candidate provenance. Phase B is deterministic-green but remains live-pending;
typed candidate reinforcement is explicitly Phase C.

## [2026-08-01] living memory | Autonomous reflection v0.5 Phase A

Added `pages/planning/autonomous-reflection-v0.5.md` as the implementation
authority for the next cognitive-memory capability. The page defines
critical-mass, sleep, and interval semantics; preserves the rule that agents do
not write Core; separates processing state from the three memory layers; and
records Phase A implementation versus the still-open personality-scope and
reinforcement phases.

Updated strategy, architecture, contracts, roadmap, the Generative Agents
review, wiki home/index, root README, prompt documentation, HISTORY, and DEVLOG.
The documentation records the new `reflection_state.v1`, compatible per-unit
reflection markers, structured trigger status, opt-in host execution, initial
calibration values, and the v0.5 release boundary without claiming live
acceptance.

## [2026-08-01] recall calibration | Associative memory may abstain

Recorded the live boundary discovered after the associative-layer acceptance:
an unrelated dinosaur query produced a nearest planet unit at cosine `0.455`,
while the weakest observed useful training association was `0.497`. The bundled
MiniLM default is now `0.47`, accepted items expose the applied threshold, and
the contracts state explicitly that an empty associative section is preferable
to forcing the nearest unrelated memory. The roadmap remains in progress until
the live negative and positive controls are repeated on the rebuilt adapter.

## [2026-08-01] context architecture | Ephemeral associative memory

Corrected the Phase 1 contextual-expansion package role after live Chibigochi
verification showed a valid Stage 2 Titan winner and a separately prepended
deep-recall Mars detail in one flat list. The wiki now distinguishes explicit
semantic `recall()` from chat context assembly: `archive_relevant` keeps its
structural rank, while current-turn vector hits are exposed as per-request
`associative_memory` with unit/archive provenance and an explicit old-memory
prompt section. Session, Archive, and Core remain the persisted layers; the
vector catalog remains a rebuildable sidecar. The roadmap requires Mars to
dinosaurs topic-switch acceptance before v0.4 closure.

## [2026-08-01] governance | Licensor rights clarification

Updated the governance copy of the Memory Engine Non-Commercial Public License
to Version 0.3 and kept its normative license body identical to root
`LICENSE.md`; the wiki page retains only a separate navigation header. Section
13 now states explicitly that restrictions imposed on licensees do not restrict
the Licensor's own lawful use, distribution, commercialization, or parallel
licensing of the Software. The roadmap and project HISTORY record that this is
a clarification of retained rights, not a second published commercial license
and not a library SemVer change.

## [2026-08-01] governance | SemVer and schema-version policy

Added `pages/governance/versioning.md` as the normative policy for library,
adapter, Git-tag, and schema versions.

The page defines the public compatibility surface, pre-1.0 rules, strict SemVer
after 1.0, separation between release versions and named `*.v1` schemas, the
root Cargo workspace as release source of truth, and the release checklist.
It also records the historical mismatch between milestone tags
`v0.2.0`/`v0.3.0` and `0.1.0` package manifests without rewriting old tags.

Linked the policy from the wiki index and root README. The decision and public
compatibility correction are recorded in DEVLOG and HISTORY.

Follow-up in the same governance pass added the executable release gate,
documented its normal and strict tag modes, and updated the roadmap with an
explicit v0.4 closure milestone. The roadmap now distinguishes implemented
unit-level Stage 2 from its still-open Chibigochi acceptance and retracts the
obsolete plan to populate archive-level embedding fields.

## [2026-07-31] semantic provenance | Unit-focused Stage 2 recall

Updated the accepted recall documents after Chibigochi live verification showed
that unit-level vector ranking still returned an archive-wide prompt projection.

- `foundation/architecture.md` now distinguishes unit-level embedding/search
  from the parent archive's structural and statistics boundary.
- `foundation/contracts.md` adds optional `matched_memory_unit_id` to
  `RecallItem` and candidate debug, and specifies winning-unit projection.
- `planning/roadmap.md` records semantic provenance as the completed foundation
  for the future Recall Inspector while leaving the CLI itself open.
- `research/vector-storage-tz-2026-07-03.md` records the live follow-up and
  mixed-archive regression requirement.
- `index.md` now calls out the unit-level recall boundary in foundation page
  descriptions.

No scoring weights, vector model, or Stage 1 behavior changed in this wiki pass.

## [2026-07-31] contracts + roadmap | Recall Stage 2 implemented

Updated the stable contracts and accepted vector-storage spec after implementing
Recall Stage 2 in the Rust core. Documented `RecallQuery.query_embedding`, the
top-40 hybrid formula, `CoreContextRequest.explain_recall`, structured candidate
diagnostics, and the division of responsibility between Stage 2 re-ranking and
global unit-level `recall_deep` fallback.

Marked Phase C, incremental `ComputeEmbedding` backfill, and the no-vector
Stage 1 fallback complete in the roadmap. Updated the wiki index descriptions;
no new page was added.

## [2026-07-06] research note | Generative Agents Memory Stream external review

Added `research/generative-agents-memory-stream-2026-07-06.md` after reviewing
the agentpatterns.ai summary of the Park et al. generative-agents memory-stream
pattern at the owner's request.

The note compares the external retrieval-scoring formula
(`recency*0.5 + relevance*3.0 + importance*2.0`, recency from last access,
threshold-triggered reflection) against our actual `recall_deep` code, not
just our docs. Four findings recorded: importance is under-weighted in deep
recall relative to cosine similarity; our own declared M4 reinforcement
principle (`vector-recall.md`) is not wired into `recall_deep` scoring;
reflection has no accumulated-signal trigger the way sleep does; Stage 1
recall's multiplicative formula makes the vector path the only place
old-and-important memories can currently surface, which raises the stakes of
the first two findings. Also records what we deliberately do not take from the
pattern (per-observation LLM scoring, top-30 context, retrieval-as-ceiling)
and why our Core lifecycle is structurally stronger on the one axis the source
itself calls a ceiling.

Linked from `research/contextual-memory-expansion-2026-07-05.md` (the scoring
fixes should land inside that page's Phase 2 live-calibration pass) and from
`planning/roadmap.md`, which gained two new items: a `recall_deep` recency-gap
item next to the existing Stage 1 recall-boost item, and a new reflection
auto-trigger item. Also added to `index.md` under Research.

No code changed. See DEVLOG for the working-session record.

## [2026-07-05] content update | Contextual memory expansion Phase 1

Updated wiki pages for the implemented first phase of contextual memory
expansion:

- `foundation/contracts.md` documents optional
  `CoreContextRequest.query_embedding`;
- `research/contextual-memory-expansion-2026-07-05.md` marks Step 1 and Step 2
  as implemented and leaves Core-fact anchors as the next step;
- `planning/roadmap.md` marks vector Phase B and contextual expansion Phase 1 as
  complete.

The matching behavior and contract change is recorded in root `HISTORY.md`.

## [2026-07-05] page added | Wiki home

Added `README.md` as the first page to read inside the wiki. The page explains
what Memory Engine is, what state the project is in, how to navigate the wiki,
and which operational files still live outside the wiki.

## [2026-07-05] research note | LLM wiki operating patterns

Added `research/llm-wiki-operating-patterns-2026-07-05.md` after reviewing a
local Ukrainian transcript of a podcast about Karpathy-style LLM wikis.

The note records reusable operating patterns for this repository: plain Markdown
files, semantic cross-links, incremental maintenance, and repeatable wiki health
checks. Added `tools/check_wiki_links.py` as the first repository-native wiki
health check.

## [2026-07-05] link pass | Wiki shell and governance

Added navigation around the wiki shell and governance page:

- `index.md` now links to wiki rules and the wiki log;
- `AGENTS.md` links back to index, log, HISTORY, DEVLOG, and strategy;
- `log.md` links back to index, rules, and DEVLOG;
- `pages/governance/licensing.md` links to root license, README, strategy,
  HISTORY, and SECURITY.

This completes the first top-level wiki navigation pass.

## [2026-07-05] link pass | Remaining research pages

Added first-pass `Related pages` navigation blocks to the remaining research
pages:

- `research/agentic-architectures.md`;
- `research/multimodal-media.md`;
- `research/multi-speaker-geometry-2026-06-12.md`;
- `research/memory-time-perception-2026-07-02.md`;
- `research/history-discipline-reference.md`.

This completes the first navigation pass over the current research cluster.

## [2026-07-05] link pass | Active vector and provider research

Added first-pass `Related pages` navigation blocks to active research pages:

- `research/vector-recall.md`;
- `research/vector-storage-tz-2026-07-03.md`;
- `research/contextual-memory-expansion-2026-07-05.md`;
- `research/provider-landscape.md`.

This pass connects the original vector draft, the implementation TZ, the current
contextual expansion plan, and the provider-resource research.

## [2026-07-05] link pass | Release and integration pages

Added first-pass `Related pages` navigation blocks to release and integration
pages:

- `releases/v0.1-acceptance.md`;
- `releases/v0.2-acceptance.md`;
- `releases/v0.3-acceptance.md`;
- `integration/local-development.md`;
- `integration/llm-integration-resources.md`;
- `integration/chibigochi-llm-bridge.md`.

This pass connects acceptance documents, local setup, model resources, and the
Godot/Chibigochi bridge without changing their underlying content.

## [2026-07-05] link pass | Foundation and planning pages

Added first-pass `Related pages` navigation blocks to the core foundation and
planning pages:

- `foundation/strategy.md`;
- `foundation/architecture.md`;
- `foundation/contracts.md`;
- `planning/roadmap.md`;
- `planning/audit-2026-06-10.md`.

This pass focused on high-level navigation only. It did not rewrite the page
content.

## [2026-07-05] restructure | Project docs moved into LLM wiki

The project adopted an LLM-wiki structure for its written project knowledge.

Moved project documentation from `docs/` into `wiki/pages/`:

- foundation: strategy, architecture, contracts;
- planning: roadmap and audit;
- releases: v0.1/v0.2/v0.3 acceptance;
- integration: local development, LLM resources, Chibigochi bridge;
- governance: licensing;
- research: research notes and implementation specs.

Added:

- `wiki/AGENTS.md` as the wiki schema for future agents;
- `wiki/index.md` as the content index;
- `wiki/log.md` as the chronological wiki maintenance log.

Root README, HISTORY, DEVLOG, LICENSE, SECURITY, prompt files, runtime README
files, crate README files, host README files, and config README files remain in
their operational locations.
