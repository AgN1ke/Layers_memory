# Memory Engine Wiki Log

## Related pages

- [Wiki index](index.md) - main map of project knowledge pages.
- [Wiki rules](AGENTS.md) - maintenance schema this log follows.
- [DEVLOG](../DEVLOG.md) - working development diary for broader context.

This is the chronological maintenance log for the project wiki.

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
