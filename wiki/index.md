# Memory Engine Wiki Index

Start at [Wiki Home](README.md) if you are reading this project for the first
time.

This is the content index for the Memory Engine project wiki.

The wiki contains project knowledge: strategy, architecture, contracts,
research, plans, audits, acceptance records, and integration notes. Operational
files such as root README, HISTORY, DEVLOG, prompts, runtime README files, and
crate/host README files stay outside the wiki.

## Wiki Maintenance

- [Wiki Home](README.md) - first page to read before using the index.
- [Wiki Rules](AGENTS.md) - maintenance schema for adding, moving, and linking
  wiki pages.
- [Wiki Log](log.md) - chronological log of wiki maintenance operations.

## Foundation

- [Strategy](pages/foundation/strategy.md) - strategic intent, product
  boundaries, human-control principles, and major direction updates.
- [Architecture](pages/foundation/architecture.md) - architecture, terminology,
  data flows, storage, task boundaries, adapters, version scope, and the
  unit-level semantic recall boundary.
- [Contracts](pages/foundation/contracts.md) - JSON/JSONL schemas, storage
  contracts, task contracts, prompt-facing contracts, hybrid recall scoring,
  unit-focused semantic provenance, hypothesis graph storage/migration,
  structured recall diagnostics, and compatibility rules.

## Planning

- [Information Flow Integrity](pages/planning/information-flow-integrity.md) -
  end-to-end host-event → Session → Sleep → MemoryUnit → recall → context-plan
  authority, implemented integrity repairs, and ordered server completion
  phases.
- [Roadmap](pages/planning/roadmap.md) - current implementation state, completed
  unit-focused vector Stage 2, v0.5 live gate, implemented v0.6 graph-storage
  foundation, open work, deferred items, and phase ordering.
- [Autonomous Reflection v0.5](pages/planning/autonomous-reflection-v0.5.md) -
  strategic design, trigger semantics, personality-scope evidence, bounded
  related history, typed candidate reinforcement/Core challenge/no-insight,
  integrity audit, journaled Phase C.1 closure, and acceptance boundary.
- [Reflective Belief System](pages/planning/reflective-belief-system.md) -
  strategic implementation plan from scope-safe durable reflection through a
  hierarchical evidence graph, temporal contradiction, and opt-in native
  clarification.
- [Server-scale Memory Runtime](pages/planning/server-scale-memory-runtime.md) -
  accepted production track, provisional launch envelope, implemented S1
  identity/ordered-turn reference, externally accepted S2.A/S2.B logical
  reference, implemented transactional S2.C Sleep lifecycle pending external
  acceptance, and
  planned PostgreSQL, durable worker, migration and launch acceptance phases.
- [Storage v2 Work-Unit Inventory](pages/planning/storage-v2-work-unit-inventory.md)
  - complete audit of current storage call chains, externally accepted
    ingest/Core/Stage 1 recall reference, restartable transactional Sleep with
    immutable input and terminal coverage semantics, remaining migration waves
    and S2 exit gate.
- [Audit 2026-06-10](pages/planning/audit-2026-06-10.md) - post-v0.2 audit,
  cleanup queue, risks, and resolved findings.
- [Production Integration Feedback 2026-08](pages/planning/production-feedback-2026-08.md)
  - production observations and implemented controls for Sleep cost, compact
    provider input, structured chat context, Core/reflection diagnostics and
    graceful run budgets.

## Releases

- [v0.6 Hypothesis Graph Storage Acceptance](pages/releases/v0.6-hypothesis-graph-storage-acceptance.md)
  - completed Chibigochi host-path fixture for exact-scope provenance,
    deduplicated evidence, idempotence, compatibility, graph-only fallback,
    privacy and stable empty contracts, while preserving the first live
    dataset gap and build-provenance caveat.
- [v0.1 Acceptance](pages/releases/v0.1-acceptance.md) - acceptance criteria for
  the first usable memory foundation.
- [v0.2 Acceptance](pages/releases/v0.2-acceptance.md) - acceptance criteria for
  the living-memory lifecycle.
- [v0.3 Acceptance](pages/releases/v0.3-acceptance.md) - host conformance and
  multi-host acceptance criteria.

## Integration

- [Local Development](pages/integration/local-development.md) - Windows setup,
  build commands, local tools, dev harnesses, and troubleshooting.
- [LLM Integration Resources](pages/integration/llm-integration-resources.md) -
  model resources, keys, provider roles, and integration responsibilities.
- [Chibigochi LLM Bridge](pages/integration/chibigochi-llm-bridge.md) -
  Chibigochi/Godot bridge shape and LLM boundary notes.

## Governance

- [Licensing](pages/governance/licensing.md) - non-commercial license rationale
  and license text history.
- [Versioning](pages/governance/versioning.md) - normative SemVer policy,
  pre-1.0 compatibility rules, schema-version separation, and release checklist.

## Research

- [Agentic Architectures](pages/research/agentic-architectures.md) - agent
  framework landscape and what Memory Engine adopts or rejects.
- [Generative Agents Memory Stream](pages/research/generative-agents-memory-stream-2026-07-06.md)
  - external retrieval/reflection review, expanded with Letta sleep-time,
    LangGraph/Temporal durability, Mem0 comparison-before-write, and the
    accepted hypothesis-graph conclusion.
- [Contextual Memory Expansion](pages/research/contextual-memory-expansion-2026-07-05.md)
  - plan for topic-triggered expansion of detailed memory under budget.
- [Memory Time Perception](pages/research/memory-time-perception-2026-07-02.md)
  - handling relative time, timestamps, and time-aware rendering.
- [Multi-Speaker Geometry](pages/research/multi-speaker-geometry-2026-06-12.md)
  - speaker attribution, reply links, and multi-speaker memory geometry.
- [Multimodal Media](pages/research/multimodal-media.md) - multimodal provider
  and media capability research.
- [Provider Landscape](pages/research/provider-landscape.md) - model provider
  landscape, pricing, capabilities, and provider tradeoffs.
- [Vector Recall Draft](pages/research/vector-recall.md) - original vector
  recall research draft.
- [Vector Storage Implementation Spec](pages/research/vector-storage-tz-2026-07-03.md)
  - accepted vector storage and deep recall implementation plan.
- [Server Memory Runtime Research](pages/research/server-memory-runtime-2026-08-01.md)
  - GitHub, PostgreSQL and Google Play/Cloud findings behind the centralized
  Chibigochi architecture.
- [HISTORY Discipline Reference](pages/research/history-discipline-reference.md)
  - reference note for HISTORY discipline and trust-document style.
- [LLM Wiki Operating Patterns](pages/research/llm-wiki-operating-patterns-2026-07-05.md)
  - useful operating patterns from a podcast transcript about Karpathy-style
    LLM wikis.

## Operating Files Outside The Wiki

- [Project README](../README.md) - public entry point.
- [HISTORY](../HISTORY.md) - product-level trust and compatibility record.
- [DEVLOG](../DEVLOG.md) - chronological working diary.
- [Prompts](../prompts/README.md) - prompt directory rules and prompt files.
- [Configuration](../config/README.md) - config examples and local key rules.
