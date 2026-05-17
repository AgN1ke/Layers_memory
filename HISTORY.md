# HISTORY

Memory Engine has no release history yet.

This file is reserved for product-level changes that affect trust, compatibility, data, integrations, or public claims.

Record changes here when they involve:

- breaking changes in contracts;
- schema or migration changes;
- recall, sleep, reflection, scoring, decay, or storage behavior changes;
- fixes that could change memory results or prevent data damage;
- prompt changes that affect stored memory or model behavior;
- compatibility notes for adapters;
- security or data integrity issues.

For day-to-day working notes, use `DEVLOG.md`.

## 2026-05-17

- Added the first public `MemoryEngine` facade with `ingest()` for converting `IngestEvent` into `StoredEvent` and writing it through the configured `Storage`.
- Added deterministic event pre-scoring configuration through `EventScoringConfig`; no LLM provider, model, key, or prompt text is involved in this step.
- Added RFC3339 UTC timestamp generation for engine-owned `received_at` values.
