# Memory Engine Wiki Index

This is the content index for the Memory Engine project wiki.

The wiki contains project knowledge: strategy, architecture, contracts,
research, plans, audits, acceptance records, and integration notes. Operational
files such as root README, HISTORY, DEVLOG, prompts, runtime README files, and
crate/host README files stay outside the wiki.

## Wiki Maintenance

- [Wiki Rules](AGENTS.md) - maintenance schema for adding, moving, and linking
  wiki pages.
- [Wiki Log](log.md) - chronological log of wiki maintenance operations.

## Foundation

- [Strategy](pages/foundation/strategy.md) - strategic intent, product
  boundaries, human-control principles, and major direction updates.
- [Architecture](pages/foundation/architecture.md) - architecture, terminology,
  data flows, storage, task boundaries, adapters, and version scope.
- [Contracts](pages/foundation/contracts.md) - JSON/JSONL schemas, storage
  contracts, task contracts, prompt-facing contracts, and compatibility rules.

## Planning

- [Roadmap](pages/planning/roadmap.md) - current implementation state, accepted
  plan, open work, deferred items, and phase ordering.
- [Audit 2026-06-10](pages/planning/audit-2026-06-10.md) - post-v0.2 audit,
  cleanup queue, risks, and resolved findings.

## Releases

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

## Research

- [Agentic Architectures](pages/research/agentic-architectures.md) - agent
  framework landscape and what Memory Engine adopts or rejects.
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
- [HISTORY Discipline Reference](pages/research/history-discipline-reference.md)
  - reference note for HISTORY discipline and trust-document style.

## Operating Files Outside The Wiki

- [Project README](../README.md) - public entry point.
- [HISTORY](../HISTORY.md) - product-level trust and compatibility record.
- [DEVLOG](../DEVLOG.md) - chronological working diary.
- [Prompts](../prompts/README.md) - prompt directory rules and prompt files.
- [Configuration](../config/README.md) - config examples and local key rules.
