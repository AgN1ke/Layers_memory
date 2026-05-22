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

## 2026-05-22 — Local harness and session-scoped archive recall

The first local non-Telegram harness run exposed a real memory isolation defect: a brand-new local session could receive `archive_relevant` from older sessions. That made the bot answer with stale personal context such as the user's name before the new session had supplied it.

**What changed:**
- Added `hosts/telegram_gemini_bot/local_harness.py` for local conversation tests through the same Memory Engine, Gemini client, prompt builder, sleep flow, and Archive → Core bridge used by the Telegram host.
- Added `hosts/telegram_gemini_bot/run_local_harness.ps1`.
- `MemoryEngine::recall()` now treats `RecallQuery.session_id` as a session boundary for archive recall: when present, archive entries must have matching `source_session_id`.
- Global archive recall remains available only when `RecallQuery.session_id` is omitted or `null`.
- Added `engine_recall_with_session_id_does_not_leak_other_sessions` to prove a fresh session does not see another session's archive, while explicit global recall still can.

**What is retracted (if applicable):**
- The implicit assumption that archive recall in `core_context_package` could safely rank across all sessions. It could not: default chat context must be scoped to the current session to avoid cross-session contamination.

**What is still true:**
- Core facts remain isolated by `core_scope`.
- Archive entries still keep `source_session_id` and can be inspected globally for admin/debug flows.
- Host prompts still use compact prompt-facing projections rather than raw storage JSON.

**What we are doing:**
- Keep the local harness as a preflight tool, not a replacement for the real Telegram acceptance test.
- Add explicit acceptance coverage that a fresh session/chat must not inherit archive memories from another session/chat unless the caller deliberately requests global recall.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo fmt --check`
- `maturin develop`
- `python hosts/telegram_gemini_bot/local_harness.py --scenario mixed_short --turn-limit 4 --no-force-sleep-at-end`

**Thanks:**
- Mykyta Zagamula for insisting that tests must behave like real varied conversation and for challenging stale-memory behavior instead of accepting plausible replies.

## 2026-05-22 — Dialogue prompt geometry and free-category Core read path

The live Telegram logs showed repeated mid-dialog greetings and a Core context gap. The greeting issue was not stale memory: it happened before archive/Core were present in the prompt. The host was giving Gemini a compact JSON context package instead of a role-shaped dialogue, so the model treated turns too much like detached requests. Separately, Core facts were now written into free categories such as `name`, `pet`, and `age`, but Rust context assembly and patching still searched only the legacy `profile/preferences/relationship` list.

**What changed:**
- `Storage::read_core_store_categories()` now returns all Core Store category files.
- `FileStorage` reads every `memory/core/store/*.json` category file.
- `MemoryEngine::core_context_facts()` and `MemoryEngine::patch_core_fact()` now scan all stored categories, not `ContextPackageConfig.core_categories`.
- `ContextPackageConfig.core_categories` is retained only as a legacy seed list, not a whitelist.
- Telegram `chat_prompt()` now renders a role transcript (`user:` / `assistant:`), separates the current user message from prior context, and deduplicates `session_recent` from `session_trace` at the prompt boundary.
- Telegram archive prompt view is shorter: `gist`, a few personal signals, a few emotional markers, and compact topic/tone data instead of full narrative/quotes/debug payload.
- `prompts/telegram_chat_system.md` now explicitly forbids greetings inside an ongoing transcript unless the current user message is itself a greeting, and forbids confusing the assistant name with the user name.

**What is retracted (if applicable):**
- The claim that the previous compact JSON prompt was sufficient as the ordinary chat prompt. It was smaller than raw debug JSON, but it did not preserve dialogue geometry well enough.
- The implicit assumption that the legacy Core category list did not matter after free categories were introduced. It did matter: Core reads and patches still used it.

**What is still true:**
- `core_context_package.v1` remains the API/debug shape for hosts.
- Storage/debug files remain complete and auditable.
- Free Core categories remain allowed; trust still comes from user source, confidence, status, scope, and duplicate gates.

**What we are doing:**
- Retest a short live Telegram conversation for mid-dialog greeting regression.
- Verify `/core`, `/core_forget`, and normal chat answers now see facts stored in non-legacy categories.
- Keep the planned local non-Telegram conversation harness as the next step, after these fixes.

**Reproducibility anchor:**
- `python -m py_compile hosts/telegram_gemini_bot/bot.py`
- `cargo fmt`
- `cargo test --workspace`

**Thanks:**
- Mykyta Zagamula for noticing that the bot was losing conversation geometry and for pushing the token-economy requirement from architecture into actual prompt shape.

## 2026-05-21 — Live-test hardening and token telemetry

The live Telegram test proved the memory cycle works, but exposed concrete v0.1 defects: one invalid `sleep_consolidator` JSON left sleep unfinished, one Gemini safety block leaked a traceback to Telegram, personal-signal extraction still relied too much on narrow categories, and the host did not log enough token economics to prove compression savings.

**What changed:**
- `hosts/telegram_gemini_bot/bot.py` now logs Gemini provider `usageMetadata` for each model call into `runtime/logs/token_usage.jsonl` and short `token_usage` lines into `bot.log`.
- Chat turns now log estimated baseline tokens for raw-history-without-compression and estimated savings versus the compact Memory Engine prompt.
- Sleep now logs raw transcript estimated tokens versus compressed archive estimated tokens as `sleep_compression_metric` / `sleep_compression_tokens`.
- Chat prompts now use a compact prompt-facing projection without long storage/debug IDs.
- `sleep_consolidator` gets one JSON retry, then falls back to a complete archive assembled from the four successful specialized passes.
- Telegram error UX now hides tracebacks from users and categorizes Gemini no-candidates, safety, invalid key, rate/quota, and generic memory errors.
- `sleep_personal_signal_pass.md` was rewritten around criteria for stable user-grounded self-statements and free normalized categories, not a hardcoded category whitelist.
- `docs/architecture.md`, `docs/contracts.md`, `docs/local-development.md`, and `docs/roadmap.md` now describe the compact prompt, free categories, and token telemetry.

**What is retracted (if applicable):**
- The earlier category whitelist for Archive → Core bridge. Category mapping is now normalized free text; trust comes from source/confidence/duplicate gates, not from a closed topic list.
- The assumption that a successful provider call is guaranteed to return parseable JSON or user-visible text. The host must treat LLM output as unstable.

**What is still true:**
- Rust core still has no provider-specific tokenizer or network dependency.
- Storage/debug files stay full and auditable; compact projection only affects ordinary LLM prompt construction.
- Exact Gemini usage comes from provider `usageMetadata`; baseline-without-compression and raw→compressed sleep metrics are deterministic estimates unless we later add provider `countTokens`.

**What we are doing:**
- Retest the failure paths: invalid consolidator JSON fallback, Gemini safety/no-candidates UX, direct stable self-statements, and token telemetry in `token_usage.jsonl`.

**Reproducibility anchor:**
- `python -m py_compile hosts/telegram_gemini_bot/bot.py`
- `git diff --check`
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest crates\python_adapter\tests`

**Thanks:**
- Mykyta Zagamula for insisting that token economy must be measurable, not assumed, and for rejecting hardcoded category lists in prompts.

## 2026-05-21 — Compact prompt representation requirement

The owner flagged that long technical identifiers and verbose storage-shaped JSON can waste the same token budget the engine is supposed to preserve. The architecture now explicitly separates canonical storage/debug data from the compact representation that should be sent to an LLM prompt.

**What changed:**
- `docs/architecture.md` now states that storage/debug views may keep full IDs and metadata, but prompt-facing memory must be semantically sufficient and compact.
- `docs/contracts.md` now notes that `core_context_package.v1` is an API/debug shape, not necessarily the literal LLM prompt payload.
- `docs/roadmap.md` adds an open v0.1 item for compact prompt representation.
- `docs/v0.1-acceptance.md` adds acceptance checks that long IDs and debug metadata must not consume prompt budget unnecessarily.

**What is retracted (if applicable):**
- The implicit assumption that a full debug/API JSON package is acceptable as the literal prompt payload. It is useful for inspection, but too verbose for efficient live memory use.

**What is still true:**
- Canonical storage remains file-based, complete, human-inspectable, and audit-friendly.
- Technical IDs remain necessary for storage, linking, deduplication, commands such as `/core_update <id>`, and debugging.
- The 11k/7k/3k/1k budget still applies to the memory context that actually reaches the model.

**What we are doing:**
- Add a compact prompt projection or `prompt_view` mode before closing v0.1.
- Keep full IDs available in debug/admin paths, but remove them from ordinary chat prompt context unless required.

**Thanks:**
- Mykyta Zagamula for pointing out that verbose IDs directly conflict with the token-economy goal.

## 2026-05-20 — Core context package token budget

The owner defined the v0.1 memory prompt benchmark as a maximum of 11k tokens: 7k current memory, 3k compressed archive memory, and 1k Core. The engine now enforces that contract at the `core_context_package` boundary with a deterministic estimator and an explicit report.

**What changed:**
- `core_context_request.v1` accepts optional `token_budget` with `total_tokens`, `current_memory_tokens`, `compressed_memory_tokens`, and `core_tokens`.
- `core_context_package.v1` returns `budget`, including the estimator id, estimated layer usage, dropped item counts, and `budget_exceeded`.
- `ContextPackageConfig` now has a default `CoreContextTokenBudget` of 11k/7k/3k/1k.
- `core_context_package` trims active session context from the oldest side, archive memories by recall rank, and Core facts by confidence.

**What is retracted (if applicable):**
- Nothing is retracted, but the token count is not a provider-specific exact tokenizer result. It is a conservative deterministic estimate (`unicode_chars_div_2_ceil_json_v1`) until a host-level tokenizer is added.

**What is still true:**
- Rust core still has no provider, model, key, or network dependency.
- Hosts may still override event count limits and may pass their own token budget in the request.
- Stored session/archive/core files remain unchanged by this package-level trimming.

**What we are doing:**
- Keep the 11k/7k/3k/1k benchmark in acceptance tests.
- Add provider-specific exact token accounting later at host level if needed.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- Scenario test: `engine_core_context_package_enforces_token_budget_by_layer`

**Thanks:**
- Mykyta Zagamula for specifying the concrete 11k/7k/3k/1k memory benchmark and rejecting simple truncation as insufficient.

## 2026-05-17

- Added the first public `MemoryEngine` facade with `ingest()` for converting `IngestEvent` into `StoredEvent` and writing it through the configured `Storage`.
- Added deterministic event pre-scoring configuration through `EventScoringConfig`; no LLM provider, model, key, or prompt text is involved in this step.
- Added RFC3339 UTC timestamp generation for engine-owned `received_at` values.
- Added `MemoryEngine::sleep()` stage 1: selected session events now become preliminary `ArchiveEntry` records and `sleep_compression` pending tasks.
- Added `MemoryEngine::resume_sleep_compression()` for applying `sleep_compression_result.v1` to an existing archive entry.
- Added `MemoryEngine::recall()` stage 1 for archive recall by filters and text scoring.
- Added the local `memory_terminal` runner for manual live testing of ingest, sleep, tasks, and recall.
- Added the first real prompt file, `prompts/sleep_compression.md`, because `sleep_compression` is now a real pending LLM task.
