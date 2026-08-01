# Memory Storage

This folder defines the human-readable local memory layout.

Runtime memory must live outside source code. It should be split by layer and by session so a person can inspect what happened without reading implementation files.

## Folders

- `sessions/` - individual working sessions. Each session gets its own folder.
- `archive/` - long-term memories produced from sessions.
- `core/` - stable facts, long-term beliefs, and durable memory foundation.
- `tasks/` - resumable LLM work requested by the Rust core.
- `runs/sleep/` - durable sleep orchestration cursors.
- `runs/reflection/` - per-scope reflection checkpoints and trigger metadata;
  this includes explicitly bound source-session membership and is processing
  state, not a fourth memory layer.
- `journal/` - frozen multi-file operation intent and recovery progress;
  currently active for reflection apply and candidate review, not a memory
  layer and not prompt context.

Candidate files under `core/candidates/` may accumulate typed reinforcement
records from later reflection passes. They remain reviewable hypotheses, not
Core facts; even a candidate that challenges Core changes nothing until owner
review.

The v0.6 storage foundation materializes the same reviewable understanding in
`core/hypotheses/`: `nodes/` keeps statements and lifecycle, `evidence/` keeps
one auditable MemoryUnit edge per hypothesis/polarity, and `relations/` is
reserved for explicit broader/narrower/equivalence/contradiction links. During
the compatibility period, `core/candidates/` remains readable and writable;
the Rust core projects it into graph records without exposing the graph as a
fourth prompt-visible memory layer.

Machine-readable files may use JSON or JSONL. Human-readable summaries should use Markdown when useful.

The exact schemas are defined in `wiki/pages/foundation/contracts.md`.
