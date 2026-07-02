# Memory Engine

Memory Engine is a reusable Rust-based memory core that gives AI applications human-like long-term memory, built as a standalone library with thin host adapters.

Most AI projects rebuild long-term memory from scratch and hit the same wall: the context window. You either stuff everything into the prompt (burning thousands of tokens every turn and still overflowing), or the assistant forgets the moment the window fills. Memory Engine is designed to reduce that trade-off. It distills conversations into compact, layered memory and recalls only what matters for the current moment, so an assistant can build up long-running memory while each turn spends only a small, focused memory budget.

Memory mirrors how people remember, across three layers:

- **Session** - the live, working conversation.
- **Archive** - long-term, consolidated experience. Opt-in vector storage is planned for scalable semantic recall.
- **Core** - the slow, stable layer: who the user is and what is settled and trusted.

Experience moves upward only when it earns its place: it is validated against its source, promoted through review rather than written blindly, and forgotten gently and reversibly when it stops mattering.

## Principles

- **Yours and auditable** - memory lives in plain, human-readable files you can read, fix, and delete. No black box.
- **Private by choice** - deep semantic (vector) storage is planned as opt-in, not as a default requirement.
- **Provider-independent by design** - the Rust core performs no network I/O and holds no model, vendor, or API key; the host runs language tasks through whatever model it chooses.
- **Reusable** - one memory layer that any host (a bot, a game, an assistant) adopts through a thin adapter, instead of rebuilding memory.

The strategic source of truth is [`docs/strategy.md`](docs/strategy.md). Current work is a Cargo workspace with the Rust memory core, PyO3 Python adapter, and runnable host examples.

## Main Documents

- [`docs/strategy.md`](docs/strategy.md) - strategic intent, product boundaries, and human-control principles.
- [`docs/architecture.md`](docs/architecture.md) - architecture v0.1: terminology, data flows, storage, PendingTask, recall, sleep, adapters, and MVP scope.
- [`docs/contracts.md`](docs/contracts.md) - data contracts v0.1: JSON/JSONL shapes for events, sessions, archive, core, recall, tasks, manifest, and journal.
- [`docs/local-development.md`](docs/local-development.md) - local Windows/Rust setup, installed tools, paths, and verification commands.
- [`docs/licensing.md`](docs/licensing.md) - Memory Engine non-commercial public license.
- [`LICENSE.md`](LICENSE.md) - root license file for GitHub publication.

## Current Structure

- `docs/` - strategy and research notes.
- `crates/memory_engine/` - Rust memory core.
- `crates/python_adapter/` - PyO3 adapter exposed to Python as `memory_engine`.
- `hosts/` - runnable host applications that use Memory Engine without putting provider/model/API-key logic into the Rust core.
- `config/` - configuration examples and local configuration rules.
- `prompts/` - prompt files when they actually exist.
- `memory/` - local runtime memory layout for sessions, archive, and core.
- `DEVLOG.md` - development diary and working notes.
- `HISTORY.md` - important product-level changes and compatibility notes.

## Local Development

Required local tools:

- Rust stable toolchain managed by `rustup`.
- On Windows: Visual Studio Build Tools 2022 with the C++ build tools workload, because the MSVC Rust target needs `link.exe`.

Detailed local paths and troubleshooting are documented in [`docs/local-development.md`](docs/local-development.md).

Useful checks:

```powershell
cargo fetch
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Run the local memory terminal:

```powershell
cargo run -p memory_engine --bin memory_terminal -- memory
```

Run the Telegram + Gemini host bot:

```powershell
.\hosts\telegram_gemini_bot\run.ps1
```

## Working Rules

- The Rust core must not hardcode LLM providers, model names, or API keys.
- Real prompts must live in `prompts/`, not inside code.
- Prompts are added only when a real feature starts using them.
- Runtime memory must be readable and split by layer and session.
- Human-facing README files must explain what each configurable or inspectable area does.
