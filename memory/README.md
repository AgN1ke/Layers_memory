# Memory Storage

This folder defines the human-readable local memory layout.

Runtime memory must live outside source code. It should be split by layer and by session so a person can inspect what happened without reading implementation files.

## Folders

- `sessions/` - individual working sessions. Each session gets its own folder.
- `archive/` - long-term memories produced from sessions.
- `core/` - stable facts, long-term beliefs, and durable memory foundation.

Machine-readable files may use JSON or JSONL. Human-readable summaries should use Markdown when useful.

The exact schemas will be defined later, when implementation starts.
