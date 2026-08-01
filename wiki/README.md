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

The active design direction has two ordered tracks. Contextual memory expansion
keeps ordinary chat context small and activates old details only for the
current topic. Autonomous reflection has a durable scope-safe apply/review
foundation and now also has the first internal graph-storage slice: journaled
candidate migration, MemoryUnit evidence edges and scope-exact inspection.
Hierarchical reasoning over that graph remains the next phase between Archive
and Core.

A separate production track is now explicit. The current `FileStorage` and
in-process locks are the local/reference deployment, not proof of centralized
multi-user scale. Google Play hosting requires an authenticated Memory Runtime,
one server-derived vault per account, transactional PostgreSQL/pgvector,
durable workers, portable export/restore and whole-turn load/isolation gates.
This architecture is accepted; its implementation phases are still open.

## How To Read The Wiki

Start here, then use [Wiki index](index.md) as the map.

For strategy and boundaries, read:

- [Strategy](pages/foundation/strategy.md)
- [Architecture](pages/foundation/architecture.md)
- [Contracts](pages/foundation/contracts.md)

For current work, read:

- [Roadmap](pages/planning/roadmap.md)
- [Reflective Belief System](pages/planning/reflective-belief-system.md)
- [Server-scale Memory Runtime](pages/planning/server-scale-memory-runtime.md)
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
