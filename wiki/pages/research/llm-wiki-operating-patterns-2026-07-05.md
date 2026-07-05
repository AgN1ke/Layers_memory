# LLM Wiki Operating Patterns From Podcast Transcript - 2026-07-05

## Related pages

- [Wiki index](../../index.md) - current project wiki map.
- [Wiki rules](../../AGENTS.md) - maintenance rules for this wiki.
- [Strategy](../foundation/strategy.md) - project principles this note must preserve.
- [Architecture](../foundation/architecture.md) - boundary between the library and embedding applications.
- [Contextual memory expansion](contextual-memory-expansion-2026-07-05.md) - active memory plan that shares the same token-economy goal.

## Purpose

This note records what is useful for Memory Engine from a Ukrainian
auto-generated transcript of a YouTube podcast about the Karpathy LLM-wiki
approach. The transcript was reviewed as an idea source, not as a dependency.

The practical takeaway is not Obsidian, VPS hosting, or any specific tool. The
useful part is the operating pattern: plain files, clear layers, automated
processing, synchronization, and regular health checks.

## Useful Ideas

1. Project knowledge works best when it is stored as ordinary text files with a
   clear directory shape.
2. A visual UI is optional. The durable system is the text base plus tools that
   can read and update it.
3. Raw material and processed knowledge should stay separate. Raw inputs can be
   transcripts, posts, meetings, or logs; processed pages should be organized by
   purpose and linked by meaning.
4. Links should represent semantic relation, not only file inclusion. A page
   should point to the pages a reader or agent is likely to need next.
5. Automation should report status. A useful knowledge base needs checks for
   broken links, stale pipelines, failed imports, and drift between source files
   and generated views.
6. Maintenance should be incremental. The podcast describes starting small and
   adding automation as the wiki becomes valuable.

## What This Means For This Repository

The current `wiki/` migration already matches the strongest parts of the
approach:

- `wiki/index.md` is the entry point;
- `wiki/AGENTS.md` is the maintenance schema;
- `wiki/log.md` records wiki maintenance;
- project knowledge lives in ordinary Markdown files;
- operational files remain in their normal locations;
- wiki pages now have first-pass `Related pages` navigation.

The missing operational piece was a repeatable health check. The link checker is
now a repository tool:

```powershell
python tools/check_wiki_links.py --wiki-related-pages
```

It checks internal Markdown links and verifies that wiki pages, except the main
index, carry a `Related pages` section.

## Optional Examples

Obsidian, graph UI views, VPS agents, and scheduled jobs are examples from a
personal note-taking workflow.

Memory Engine's written knowledge stays simple, repository-native, and readable
on GitHub and from command-line agents. Extra UIs or scheduled jobs can be added
later when a concrete workflow needs them.

## Future Ideas

These are optional follow-ups, not current requirements:

- add the wiki link checker to CI;
- add a small generated wiki health summary if the wiki grows much larger;
- add a documentation dashboard only if plain Markdown navigation stops being
  enough;
- add source-to-wiki ingestion only for project materials that repeat often
  enough to justify automation.
