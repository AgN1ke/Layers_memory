# Memory Engine

Memory Engine is a reusable Rust-based memory core that gives AI applications human-like long-term memory, built as a standalone library with thin host adapters.

Most AI projects rebuild long-term memory from scratch and hit the same wall: the context window. You either stuff everything into the prompt (burning thousands of tokens every turn and still overflowing), or the assistant forgets the moment the window fills. Memory Engine is designed to reduce that trade-off. It distills conversations into compact, layered memory and recalls only what matters for the current moment, so an assistant can build up long-running memory while each turn spends only a small, focused memory budget.

Memory mirrors how people remember, across three layers:

- **Session** - the live, working conversation.
- **Archive** - long-term, consolidated experience. Optional local vectors over
  atomic memory units add semantic re-ranking and deep recall without replacing
  the file-backed archive.
- **Core** - the slow, stable layer: who the user is and what is settled and trusted.

Experience moves upward only when it earns its place: it is validated against its source, promoted through review rather than written blindly, and forgotten gently and reversibly when it stops mattering.

The v0.5 direction adds an autonomous rhythm of reflection. The core can notice
when new validated experience reaches critical mass, or when slow accumulation
has waited long enough for a sleep/maintenance check, and then request a
bounded reasoning pass. Reflection may propose reviewable beliefs or conclude
that nothing durable changed; it still cannot write Core directly.

Reflection also remembers its own unfinished understanding. The current v0.5
path can reinforce a visible existing candidate, open an explicit reviewable
challenge, or return structured no-insight instead of turning every episode
into a trait. Apply and owner review are now scope-safe, journal-backed, and
recoverable across multi-file writes; v0.5 release acceptance remains open for
live Chibigochi calibration rather than for this integrity gap.

The v0.6 storage foundation begins that hierarchy without pretending the
reasoning behavior already exists. Flat candidates can be migrated through a
journaled, restart-safe operation into separate hypothesis nodes and atomic
MemoryUnit evidence links; graph export is exact-scope and human-inspectable,
while the existing reflection/review API keeps working through a compatibility
projection. Migration never invents broader/narrower relations. Hierarchical
proposal and maturation remain the next behavior phase. The storage/migration
gate has passed a non-degenerate Chibigochi host-path fixture; owner memory was
not seeded or migrated. This internal graph is
part of the slow Archive-to-Core transition, not a fourth prompt-visible
memory layer.

Reflection is personality-scoped rather than conversation-scoped. A host binds
several source sessions through one explicit `core_scope`; the core can then
accumulate their new validated units and add a bounded, explainable slice of
structurally related older history. Similar text never links identities, and
the non-vector path remains complete.

Optional vectors do not create another persisted memory layer. They form a
local, rebuildable sidecar index over atomic memory units. During a live turn,
the Rust core may use that index to build a small ephemeral
`associative_memory` section: older details activated by the current topic,
clearly separated from the ongoing conversation, ordinary Archive recall, and
stable Core facts. The section is rebuilt for every request, so changing the
topic does not carry the previous association forward. Its conservative,
model-calibrated similarity gate may return no association at all: the core
does not force the nearest old memory into an unrelated conversation.

The information path is now explicit end to end. A host keeps lossless domain
data in `payload` and supplies a separate `memory_text` projection for
cognition. One stable `personality_scope_id` can span multiple interaction
Sessions without becoming a global search. Sleep freezes the complete source
projection, and standalone MemoryUnits become the canonical prompt truth:
forgotten, rejected, distorted, or revision-pending units cannot leak back
through an old Archive gist. `CoreContextPackage` is the final context plan;
budgeting/deduplication precede recall feedback, while current host state is
rendered separately from remembered claims.

For a centralized Google Play deployment, the current file backend is a local
and reference implementation, not the production topology. The accepted
server track keeps the same Rust cognitive core behind an authenticated Memory
Runtime, binds one logical vault per game account, and adds transactional
PostgreSQL/pgvector storage plus durable workers. S1 now defines fail-closed
vault identity and restart-safe ordered/idempotent turns. The active S2
foundation adds an additive vault-bound work-unit contract. Event and Session
metadata commit together; Core upsert/patch uses exact category revisions and
durable replay results; transactional Stage 1 recall ranks one committed
Archive snapshot and atomically records its correctness-affecting feedback.
Transactional sleep start now reuses the same Rust Stage 1 policy and commits
one coherent reservation: Session `SleepPending`, a preliminary Archive, two
durable tasks, the initial mutable `SleepRun`, and an immutable replay result.
Provider work stays outside the transaction, the live tail follows trusted
receive order, and only future successful finalization may mark events as
covered by a Complete archive. These paths have optimistic retry, phantom
protection and idempotent replay. They are still a conformance reference, not
the production server backend: sleep submit/finalize/cancel, remaining
workflows and vector recall, indexed PostgreSQL storage, the authenticated game
API and durable workers are open. The phased plan and launch gates are
documented in the
[Server-scale Memory Runtime](wiki/pages/planning/server-scale-memory-runtime.md).

## Principles

- **Yours and auditable** - local memory lives in plain files; server memory
  must round-trip through a versioned human-readable export you can inspect,
  migrate and delete. No opaque lock-in.
- **Private by choice** - semantic vectors are opt-in, computed by the host, and
  can stay entirely local; the non-vector recall path remains fully supported.
- **Provider-independent by design** - the Rust core performs no network I/O and holds no model, vendor, or API key; the host runs language tasks through whatever model it chooses.
- **Reusable** - one memory layer that any host (a bot, a game, an assistant) adopts through a thin adapter, instead of rebuilding memory.

The strategic source of truth is the project wiki, starting from
[`wiki/index.md`](wiki/index.md). Current work is a Cargo workspace with the
Rust memory core, PyO3 Python adapter, and runnable host examples.

## Main Documents

- [`wiki/index.md`](wiki/index.md) - project knowledge index.
- [`wiki/pages/foundation/strategy.md`](wiki/pages/foundation/strategy.md) - strategic intent, product boundaries, and human-control principles.
- [`wiki/pages/foundation/architecture.md`](wiki/pages/foundation/architecture.md) - architecture v0.1: terminology, data flows, storage, PendingTask, recall, sleep, adapters, and MVP scope.
- [`wiki/pages/foundation/contracts.md`](wiki/pages/foundation/contracts.md) - data contracts v0.1: JSON/JSONL shapes for events, sessions, archive, core, recall, tasks, manifest, and journal.
- [`wiki/pages/integration/local-development.md`](wiki/pages/integration/local-development.md) - local Windows/Rust setup, installed tools, paths, and verification commands.
- [`wiki/pages/governance/licensing.md`](wiki/pages/governance/licensing.md) - Memory Engine non-commercial public license.
- [`wiki/pages/governance/versioning.md`](wiki/pages/governance/versioning.md) - normative SemVer, schema-version, and release-tag policy.
- [`wiki/pages/planning/autonomous-reflection-v0.5.md`](wiki/pages/planning/autonomous-reflection-v0.5.md) - strategic design and phased acceptance for salience/time-driven reflection.
- [`wiki/pages/planning/reflective-belief-system.md`](wiki/pages/planning/reflective-belief-system.md) - phased plan from reliable reflection to an evidence-backed hypothesis graph, contradiction handling, and native clarification.
- [`wiki/pages/planning/server-scale-memory-runtime.md`](wiki/pages/planning/server-scale-memory-runtime.md) - production track for authenticated per-player vaults, transactional storage, durable workers, migration, and whole-path scale acceptance.
- [`wiki/pages/planning/information-flow-integrity.md`](wiki/pages/planning/information-flow-integrity.md) - canonical event-to-prompt flow, completed integrity repairs, and remaining transactional phases.
- [`LICENSE.md`](LICENSE.md) - root license file for GitHub publication.

## Versioning

Memory Engine uses Semantic Versioning for library and adapter releases. After
`1.0.0`, an incompatible public API or behavior change increments `MAJOR`, a
backward-compatible capability increments `MINOR`, and a compatible defect fix
increments `PATCH`. During pre-stable `0.x`, both public features and intentional
breaking changes increment `MINOR`; breaking releases must be marked explicitly
and include migration notes. Compatible fixes increment `PATCH`.

The root Cargo workspace version is the release source of truth. Rust crates,
the Python package, Git tag `vX.Y.Z`, release acceptance, and HISTORY must agree
before a release is tagged. Persisted/wire `schema_version` values such as
`recall_result.v1` are versioned separately: compatible optional additions may
stay on `.v1`, while incompatible format changes require a new schema and a
tested migration.

Historical `v0.2.0` and `v0.3.0` tags were project milestones while package
manifests remained `0.1.0`; those tags stay immutable. After live acceptance of
vector Stage 2 and unit-focused recall, the next normalized release target is
`0.4.0`. See the full [versioning policy](wiki/pages/governance/versioning.md).

## Current Structure

- `wiki/` - project knowledge base: strategy, architecture, contracts,
  research, plans, acceptance records, and integration notes.
- `crates/memory_engine/` - Rust memory core.
- `crates/python_adapter/` - PyO3 adapter exposed to Python as `memory_engine`.
- `hosts/` - runnable host applications that use Memory Engine without putting provider/model/API-key logic into the Rust core.
- `config/` - configuration examples and local configuration rules.
- `prompts/` - prompt files when they actually exist.
- `memory/` - local runtime memory layout for sessions, archive, and core.
- `DEVLOG.md` - development diary and working notes.
- `HISTORY.md` - important product-level changes and compatibility notes.

## Local Development

Required local tools:

- Rust stable toolchain managed by `rustup`.
- On Windows: Visual Studio Build Tools 2022 with the C++ build tools workload, because the MSVC Rust target needs `link.exe`.

Detailed local paths and troubleshooting are documented in [`wiki/pages/integration/local-development.md`](wiki/pages/integration/local-development.md).

Useful checks:

```powershell
cargo fetch
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python tools\check_release_versions.py
```

Run the local memory terminal:

```powershell
cargo run -p memory_engine --bin memory_terminal -- memory
```

Run the Telegram + Gemini host bot:

```powershell
.\hosts\telegram_gemini_bot\run.ps1
```

## Working Rules

- The Rust core must not hardcode LLM providers, model names, or API keys.
- Real prompts must live in `prompts/`, not inside code.
- Prompts are added only when a real feature starts using them.
- Runtime memory must be readable and split by layer and session.
- Human-facing README files must explain what each configurable or inspectable area does.
