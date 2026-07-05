# Memory Engine Wiki Rules

## Related pages

- [Wiki home](README.md) - first page to read before navigating the wiki.
- [Wiki index](index.md) - main map of project knowledge pages.
- [Wiki log](log.md) - chronological record of wiki maintenance operations.
- [Project HISTORY](../HISTORY.md) - product-level behavior and compatibility log.
- [DEVLOG](../DEVLOG.md) - working development diary.
- [Strategy](pages/foundation/strategy.md) - project principles that wiki maintenance should preserve.

## Purpose

This file is the schema for the project wiki. It tells future agents how to
maintain the written knowledge of the project.

The wiki exists for project knowledge: strategy, architecture, contracts,
research, plans, audits, acceptance records, and integration notes.

Operational files stay outside the wiki:

- root `README.md`, `HISTORY.md`, `DEVLOG.md`, `LICENSE.md`, `SECURITY.md`;
- prompt files in `prompts/`;
- runtime layout README files in `memory/`;
- crate, host, config, and tool README files that explain their local folder.

## Directory Layout

- `wiki/index.md` - content index. Update it when adding, moving, renaming, or
  substantially changing a wiki page.
- `wiki/log.md` - chronological wiki maintenance log. Append one entry per
  wiki maintenance operation.
- `wiki/pages/foundation/` - stable project foundations: strategy,
  architecture, contracts.
- `wiki/pages/planning/` - roadmap, audits, planned work, cleanup plans.
- `wiki/pages/releases/` - acceptance documents and release checklists.
- `wiki/pages/integration/` - local development, adapters, provider/model
  resources, host integration boundaries.
- `wiki/pages/research/` - research notes, drafts, accepted technical
  direction documents.
- `wiki/pages/governance/` - licensing and project governance notes.

## Maintenance Rules

When adding a new page:

1. Put it in the right `wiki/pages/` subdirectory.
2. Give it a title and a short "purpose" section.
3. Link related pages explicitly.
4. Add it to `wiki/index.md`.
5. Append an entry to `wiki/log.md`.
6. If the page changes behavior, contracts, compatibility, or public claims,
   also update `HISTORY.md`.
7. If the page records working-session context or process, also update
   `DEVLOG.md`.

When editing an existing page:

1. Preserve the page's role.
2. Prefer adding dated update sections over silently rewriting old decisions.
3. Mark superseded research explicitly instead of deleting it.
4. Keep links relative and valid inside the repository.
5. Do not move provider keys, runtime memory, or generated artifacts into the
   wiki.

After wiki link edits, run:

```powershell
python tools/check_wiki_links.py --wiki-related-pages
```

## Linking Rules

Use normal Markdown links, not only Obsidian-style wikilinks, because the project
is read on GitHub and by command-line agents.

Good:

```md
[Architecture](pages/foundation/architecture.md)
```

Avoid bare references such as `see architecture` when a link can be provided.

## Research Lifecycle

Research pages may be drafts. If a draft becomes implementation authority, add a
clear note at the top saying which document supersedes which older draft.

Accepted implementation specs should point to:

- the related roadmap item;
- the HISTORY entry if behavior changed;
- the DEVLOG entry if the decision came from a working session;
- the tests or conformance checks that prove the behavior.

## Human Control

The wiki is agent-maintained but owner-directed. Agents may organize, link, and
summarize. Strategic decisions, public claims, provider choices, and release
claims stay under owner control.
