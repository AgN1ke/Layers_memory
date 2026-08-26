# Memory Engine Wiki Home

Read this page first.

This wiki is the project knowledge base for Memory Engine. It is written for
humans and coding agents who need to understand the project without reading the
whole chat history.

## Related pages

- [Wiki index](index.md) - full map of all wiki pages.
- [Strategy](pages/foundation/strategy.md) - why the project exists and what it must preserve.
- [Architecture](pages/foundation/architecture.md) - how the library is structured.
- [Contracts](pages/foundation/contracts.md) - data and task shapes that integrations must respect.
- [Roadmap](pages/planning/roadmap.md) - current state and next planned work.
- [Autonomous reflection v0.5](pages/planning/autonomous-reflection-v0.5.md) -
  how validated experience reaches reflection by salience, sleep, and time,
  then matures one reviewable understanding through typed outcomes.
- [Reflective Belief System](pages/planning/reflective-belief-system.md) - how
  v0.5 integrity closure leads into hierarchical hypotheses, explainable
  support, contradiction, temporal revision, and bounded clarification.
- [Server-scale Memory Runtime](pages/planning/server-scale-memory-runtime.md) -
  how the same core becomes an isolated, transactional service for a Google
  Play game with many players.
- [Information Flow Integrity](pages/planning/information-flow-integrity.md) -
  the canonical event-to-prompt path, personality/session boundary, one
  MemoryUnit truth, optional association, completed repairs and remaining
  transactional phases.
- [Production Integration Feedback](pages/planning/production-feedback-2026-08.md) -
  the implemented response to real Sleep cost, context-shape, Core,
  reflection and provider-observability findings.
- [Smartest v2 Production Validation](pages/planning/smartest-v2-production-validation-2026-08.md) -
  what the production log owner must measure and return before v0.6 reasoning.
- [Smartest v2 Phase 1 Review](pages/planning/smartest-v2-phase1-review-2026-08-25.md) -
  what the first returned package proved and which instrumentation corrections
  precede the formal observation window.
- [Smartest v2 Phase 1 Correction TZ](pages/planning/smartest-v2-phase1-correction-tz-2026-08-25.md) -
  the standalone logging, preflight and qualification task used by Smartest.
- [Smartest v2 Checkpoint A Review](pages/planning/smartest-v2-checkpoint-a-review-2026-08-26.md) -
  the maintainer decision that qualifies cohort `c` for its short natural-run
  sample and corrects the fidelity/provenance interpretation.
- [LLM integration resources](pages/integration/llm-integration-resources.md) - what model/key resources an application must provide.

## What This Project Is

Memory Engine is a reusable memory library for AI applications.

Its goal is to keep useful long-term memory while spending as few prompt tokens
as possible. The library stores events, compresses conversations, builds stable
facts, verifies memory, recalls old details, and prepares a small context package
for the application to give to a language model.

The library is reusable across product surfaces. Telegram and Godot/Chibigochi
are current applications that use it. Memory policy lives in the Rust core.

## Current State

The project has already proved the main foundation:

- the memory core is reusable across product surfaces;
- sleep, archive, Core, recall, fidelity, reflection, contested Core, and
  forgetting work as one lifecycle;
- multiple host paths pass conformance checks;
- vector memory and deep recall are implemented as opt-in derived search over
  memory units;
- project documentation now lives in this wiki with checked internal links.

The active sequence is production validation on Smartest v2, closure of the
v0.5 live reflection gate, then v0.6 hierarchical reasoning. The first
Smartest package passed privacy, shared-prefix and dialogue-geometry checks and
was classified `CHANGES_REQUIRED`. The corrected instrumentation preflight is
now `QUALIFIED`; cohort `c` is collecting 1-3 natural terminal Sleep runs on a
frozen configuration. The formal multi-day window follows review of that
sample without asking chat participants to perform scripted tests.
Autonomous reflection already has durable scope-safe
apply/review and the first internal graph-storage slice: journaled candidate
migration, MemoryUnit evidence edges and scope-exact inspection. Hierarchical
reasoning starts after the redacted Smartest report confirms a non-degenerate
reflection action and no data-integrity blocker.

A separate server-scale track remains documented. The current `FileStorage` and
in-process locks are the local/reference deployment, not proof of centralized
multi-user scale. Google Play hosting requires an authenticated Memory Runtime,
one server-derived vault per account, transactional PostgreSQL/pgvector,
durable workers, portable export/restore and whole-turn load/isolation gates.
This architecture is accepted. S1 reference contracts now cover fail-closed
vault identity, idempotent ordered turns and a local restart/isolation harness.
Storage v2 has an externally accepted logical reference for ingest, Core and
committed-view Stage 1 recall. Its S2.C Sleep reference now covers start,
provider-result submit, restart recovery, finalize and cancel as one durable
workflow. One bounded immutable input owns private source events; only finalize
publishes canonical MemoryUnits and Complete coverage, while cancel releases
the sources unchanged. External acceptance is pending. Post-finalize
Core/derived routing, fidelity, forgetting, reflection/vector workflows,
PostgreSQL, the authenticated game API and production-scale acceptance remain
open. Chibigochi product UX/packaging and the Google Play server track are
paused by the owner as of 2026-08-24; their implemented paths remain regression
fixtures.

## How To Read The Wiki

Start here, then use [Wiki index](index.md) as the map.

For strategy and boundaries, read:

- [Strategy](pages/foundation/strategy.md)
- [Architecture](pages/foundation/architecture.md)
- [Contracts](pages/foundation/contracts.md)

For current work, read:

- [Roadmap](pages/planning/roadmap.md)
- [Information Flow Integrity](pages/planning/information-flow-integrity.md)
- [Production Integration Feedback](pages/planning/production-feedback-2026-08.md)
- [Smartest v2 Production Validation](pages/planning/smartest-v2-production-validation-2026-08.md)
- [Reflective Belief System](pages/planning/reflective-belief-system.md)
- [Server-scale Memory Runtime](pages/planning/server-scale-memory-runtime.md)
- [Storage v2 Work-Unit Inventory](pages/planning/storage-v2-work-unit-inventory.md)
- [Server Memory Runtime research](pages/research/server-memory-runtime-2026-08-01.md)
- [Contextual memory expansion](pages/research/contextual-memory-expansion-2026-07-05.md)
- [Vector storage implementation spec](pages/research/vector-storage-tz-2026-07-03.md)

For integration work, read:

- [LLM integration resources](pages/integration/llm-integration-resources.md)
- [Local development](pages/integration/local-development.md)
- [Chibigochi LLM Bridge](pages/integration/chibigochi-llm-bridge.md)

## Maintenance Rule

When editing wiki pages, keep links valid:

```powershell
python tools/check_wiki_links.py --wiki-related-pages
```

Behavior changes still belong in root [HISTORY](../HISTORY.md). Working-session
context still belongs in root [DEVLOG](../DEVLOG.md). The wiki is for stable
project knowledge and research notes.
