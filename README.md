# Memory Engine

Memory Engine is planned as a separate Rust-based memory core for reusable long-term memory.

The strategic source of truth is [`docs/strategy.md`](docs/strategy.md). Current work is still in the preparation stage: repository structure, rules, configuration boundaries, prompt governance, and memory layout.

## Main Documents

- [`docs/strategy.md`](docs/strategy.md) - strategic intent, product boundaries, and human-control principles.
- [`docs/architecture.md`](docs/architecture.md) - architecture v0.1: terminology, data flows, storage, PendingTask, recall, sleep, adapters, and MVP scope.
- [`docs/contracts.md`](docs/contracts.md) - data contracts v0.1: JSON/JSONL shapes for events, sessions, archive, core, recall, tasks, manifest, and journal.
- [`docs/licensing.md`](docs/licensing.md) - Memory Engine non-commercial public license.
- [`LICENSE.md`](LICENSE.md) - root license file for GitHub publication.

## Current Structure

- `docs/` - strategy and research notes.
- `src/` - Rust crate source layout and contract types.
- `tests/` - serialization and contract-level tests.
- `config/` - configuration examples and local configuration rules.
- `prompts/` - prompt files when they actually exist.
- `memory/` - local runtime memory layout for sessions, archive, and core.
- `DEVLOG.md` - development diary and working notes.
- `HISTORY.md` - important product-level changes and compatibility notes.

## Working Rules

- The Rust core must not hardcode LLM providers, model names, or API keys.
- Real prompts must live in `prompts/`, not inside code.
- Prompts are added only when a real feature starts using them.
- Runtime memory must be readable and split by layer and session.
- Human-facing README files must explain what each configurable or inspectable area does.
