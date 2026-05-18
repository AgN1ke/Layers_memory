# HISTORY

Memory Engine canonical record of post-launch corrections, schema changes, behavior changes, retracted claims, and any public notices that affect trust in the project. Newest first.

This file is not a changelog of features. It is a document of trust. The format follows the practice we adopted from MemPalace's `HISTORY.md` (see `docs/research/`).

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

## 2026-05-17

- Added the first public `MemoryEngine` facade with `ingest()` for converting `IngestEvent` into `StoredEvent` and writing it through the configured `Storage`.
- Added deterministic event pre-scoring configuration through `EventScoringConfig`; no LLM provider, model, key, or prompt text is involved in this step.
- Added RFC3339 UTC timestamp generation for engine-owned `received_at` values.
- Added `MemoryEngine::sleep()` stage 1: selected session events now become preliminary `ArchiveEntry` records and `sleep_compression` pending tasks.
- Added `MemoryEngine::resume_sleep_compression()` for applying `sleep_compression_result.v1` to an existing archive entry.
- Added `MemoryEngine::recall()` stage 1 for archive recall by filters and text scoring.
- Added the local `memory_terminal` runner for manual live testing of ingest, sleep, tasks, and recall.
- Added the first real prompt file, `prompts/sleep_compression.md`, because `sleep_compression` is now a real pending LLM task.
