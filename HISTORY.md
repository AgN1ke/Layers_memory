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

## 2026-06-10 - Recall stats are buffered instead of written on every recall

The post-v0.2 audit found that `recall()` rewrote archive entry files on every returned memory just to increment `recall_count` and `last_recalled_at`. That made recall a write-heavy operation and created avoidable write-back races around archive files.

**What changed:**
- `MemoryEngine` now buffers recall stat deltas in memory.
- Recall scoring includes pending buffered deltas, so repeated recalls can still protect a memory before the next disk flush.
- `recall()` no longer calls `update_archive_entry` for every selected archive item.
- Added `flush_recall_stats()` to write buffered `recall_count` / `last_recalled_at` updates in batches.
- `finish_sleep_run` calls `flush_recall_stats()` as a natural consolidation point.
- `RecallStage1Config` now includes `stats_flush_interval` for periodic automatic flushing; `0` disables interval flushing.
- PyO3 exposes `flush_recall_stats()` for host shutdown or maintenance.

**What is retracted (if applicable):**
- Nothing is retracted, but the precision of recall counters is explicitly advisory. Unflushed deltas can be lost if the process crashes before a flush.

**What is still true:**
- Stored archive entries still contain `recall_count` and `last_recalled_at`.
- Recall ranking still uses the same count/recent-recall boost formula; it now uses persisted values plus pending deltas.
- Core memory, archive content, and memory units are not affected by losing an unflushed advisory counter delta.

**What we are doing:**
- Continue the audit queue with engine module split and tolerant collection reads before v0.3 adapter work.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_recall_buffers_stats_until_flush_and_scores_with_pending_counts`

## 2026-06-10 - Completed sleep rotates archived raw session events

The post-v0.2 audit found that long-lived host sessions kept all raw events in one active `sessions/<session_id>/events.jsonl` file. Even after those events were covered by Complete archives, every `read_session` call still parsed the full raw chat history.

**What changed:**
- `Storage` now exposes `read_session_archived_events(session_id)` and `rotate_session_events(session_id, covered_event_ids)`.
- `FileStorage` writes rotated raw events to `sessions/<session_id>/archived/events-<NNN>.jsonl`.
- `finish_sleep_run` rotates events covered by Complete archives after archive completion, memory-unit creation, Archive-to-Core bridge, and auto-fidelity routing.
- `read_session` remains active-file only; old source events remain available through archived segments.
- Core bridge and evidence-pack construction now read active + archived session events with `event_id` deduplication, so old units remain verifiable after rotation.

**What is retracted (if applicable):**
- Nothing is retracted. This is the first step of audit A4, not the full A4 closure: recall-counter isolation and the cached `archived_event_ids` session metadata path remain open.

**What is still true:**
- `session.md` remains append-only and human-readable; it is not rotated by this change.
- Archive entries and memory units remain the canonical compressed memory.
- If a process crashes after writing an archived segment but before rewriting the active file, duplicate raw events may temporarily exist across active and archived files; engine evidence reads deduplicate by `event_id`.

**What we are doing:**
- Continue the audit queue with recall-counter isolation (B3), then engine module split and tolerant collection reads.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall`

## 2026-06-10 - SleepRun recovery is persisted in the core

The post-v0.2 audit found that multi-pass sleep orchestration kept `SleepRun` only in process memory. If the host crashed mid-sleep, durable `PendingTask` files could remain pending while no public API could reconstruct the run cursor, blocking future sleep for that session.

**What changed:**
- `Storage` now persists `SleepRun` checkpoints under `memory/runs/sleep/<sleep_task_id>.json`.
- `begin_sleep_run`, `next_sleep_batch`, `submit_sleep_batch`, and `finish_sleep_run` checkpoint the run as it advances.
- The core exposes `pending_sleep_runs()` and `cancel_sleep_run(sleep_task_id)` through Rust and PyO3.
- The Telegram host queues recovered sleep runs at startup before polling.
- `resume_sleep_compression` and `resume_memory_unit_pass` are idempotent for already-completed archive/unit artifacts, so recovery can safely re-enter `finish_sleep_run`.

**What is retracted (if applicable):**
- The prior architecture wording implied that the journal was already active for sleep lifecycle recovery. It was not. Runtime sleep recovery now relies on durable `SleepRun` checkpoints plus idempotent event coverage; the journal remains a deferred primitive for future multi-file transactions.

**What is still true:**
- The Rust core still performs no network I/O and knows no provider/model/key.
- `PendingTask` remains the durable LLM work item; `SleepRun` is the durable orchestration cursor over those work items.
- `cancel_sleep_run` does not mark session events archived. A cancelled run leaves its preliminary archive non-complete, so the same source events remain eligible for the next sleep.

**What we are doing:**
- Continue the audit queue with raw-session scaling work (A4) and recall counter isolation (B3) before v0.3 adapter work.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_sleep_run_persists_and_recovers_after_restart`
- `cargo test -p memory_engine --test engine_sleep_recall engine_cancel_sleep_run_unblocks_unarchived_events`

## 2026-06-10 - Telegram background sleep uses the shared engine

The post-v0.2 audit found that the Telegram host created a second `MemoryEngine` inside `SleepRunner._run()` for background sleep. Because `LockRegistry` is per engine instance, that bypassed the concurrency guarantees added in v0.2 and could allow lost updates between normal chat commands and background sleep completion.

**What changed:**
- `hosts/telegram_gemini_bot/bot.py` now passes the main `MemoryEngine` into `SleepRunner`.
- Background sleep completion calls `complete_sleep_result()` with that shared engine instead of constructing a new one over the same `runtime/memory` directory.
- The Telegram host now constructs `memory_engine.MemoryEngine(...)` only once, in `main()`.

**What is retracted (if applicable):**
- The implied post-concurrency assumption that all Telegram host memory writes already shared one lock registry. They did not for background sleep until this fix.

**What is still true:**
- The Rust core lock model and lock ordering are unchanged.
- The core still supports many threads through one engine instance; it still does not provide multi-process file locking over the same memory directory.
- Manual `/sleep` already used the main engine and is unchanged.

**What we are doing:**
- Continue the audit queue with persistent `SleepRun` recovery and the explicit journal decision before starting v0.3 adapter work.

**Reproducibility anchor:**
- `Select-String -Path hosts\telegram_gemini_bot\bot.py -Pattern 'MemoryEngine\('` returns one constructor call.
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`

## 2026-06-01 — v0.2 living memory cycle acceptance

v0.2 is ready to close as an end-to-end living-memory cycle in the reusable Rust core. This entry records the acceptance anchor; it does not claim external benchmark quality.

**What changed:**
- Added `docs/v0.2-acceptance.md`.
- Added deterministic integration test `crates/memory_engine/tests/living_memory_cycle.rs`.
- The test composes the full core lifecycle: `ingest -> sleep driver -> Archive + MemoryUnit -> recall/context -> fidelity -> reflection candidate -> manual Core promotion -> contested Core -> forget_review -> remember_back`.
- The acceptance path uses injected `LlmResponse` values, so the cycle is reproducible without a live provider.

**What is retracted (if applicable):**
- No quality or benchmark claim is made for v0.2. The release claim is only that the living-memory lifecycle is implemented and regression-tested end to end.

**What is still true:**
- The core still does no network I/O and knows no provider/model/key.
- Hosts still execute a single primitive: `LlmRequest -> text`, then submit the result back to the core.
- Agents still cannot write Core truth directly; Core mutation happens through explicit bridge/gating or manual review paths.

**Known limitations / deferred work:**
- Vector storage remains opt-in future work.
- Core candidate reviewer/formulation pass remains deferred.
- Unit-level recall counters are not implemented; forgetting v1 uses archive-level recall proxy.
- Per-scope Core store layout remains an optimization if shared category locks become a bottleneck.
- Partial sleep/session-tail strategy remains deferred.
- Public quality claims require a separate benchmark harness and reproducibility anchor.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test living_memory_cycle`
- Full release gate is documented in `docs/v0.2-acceptance.md`.

## 2026-06-01 — Reversible forgetting review for low-signal memory units

Memory units can now be marked as forgotten through a conservative review path. This changes prompt-facing recall behavior, because forgotten units are excluded from compact memory projections, while the full archive and audit trail remain on disk.

**What changed:**
- Added `TaskType::ForgetReview`, `forget_review_pass`, and core APIs for `begin_forget_review`, `submit_forget_review_response`, `list_forgotten_memory_units`, and `remember_back`.
- Added `MemoryUnit.forget_review` audit metadata and reversible `MemoryUnitStatus::Forgotten` application.
- Forgetting uses a triple gate: structurally eligible old/low-signal unit, LLM recommendation, and a hard engine protection re-check at submit time.
- Protected units are not forgotten even if the model says `forget`: Core-linked, high-weight, emotionally strong, or recently recalled archive-linked units stay active.
- Parent `ArchiveEntry.compact_memory` is rebuilt after both forget and remember-back, so prompt-facing memory matches unit status.

**What is retracted (if applicable):**
- Nothing is retracted. The earlier `forgotten/` directory remains future scaffolding; v1 intentionally does not physically move archive files.

**What is still true:**
- Core facts are not touched by forgetting.
- Full archive entries remain stored and auditable.
- Unit-level recall counters do not exist yet; v1 uses archive-level `recall_count` / `last_recalled_at` as a conservative proxy, which may protect routine sibling units when the archive was recalled for a more important unit.

**What we are doing:**
- Review the feature branch before merge. Scratch live-checks passed with cached Gemini: one old routine unit was recommended as `forget` and became `Forgotten`; richer live-check confirmed routine forgotten, Core-linked unit protected at submit re-check, high-weight unit stayed active, and `remember_back` restored the forgotten thesis into `compact_memory`.
- Add unit-level recall counters later only if archive-level proxy proves too conservative.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest crates\python_adapter\tests -q`
- Scenario tests: `engine_forgetting.rs`
- Scratch live-check 1: `forget_review_pass`, role `balanced`, model `gemini-2.5-flash`, prompt 580 / output 110 / total 810, result `forgotten=1`.
- Scratch live-check 2: `forget_review_pass`, role `balanced`, model `gemini-2.5-flash`, prompt 758 / output 176 / total 1339, result `forgotten=1`, `protected=1`, routine compact removed after forget and restored after `remember_back`.

## 2026-06-01 — Approved reflection candidates can contest existing Core facts

The Core can now adapt without silently overwriting old truth. When a reviewed reflection candidate explicitly contradicts an existing Core fact, approving that candidate marks the older fact as `contested` and preserves it with provenance instead of deleting or replacing it.

**What changed:**
- `ReflectionCandidateDraft` and `CandidateBelief` now support `contradicted_core_fact_ids`.
- `CandidateReviewResult` now returns `contested_facts`.
- `review_candidate(approved)` marks active same-scope Core facts listed in `contradicted_core_fact_ids` as `CoreFactStatus::Contested`, tags them, and links them to the candidate through `contested_by_candidate`.
- The promoted candidate still becomes an active Core fact only after explicit review and keeps `source_candidate_id`.
- `CoreContextFact` now includes `status`; `core_context_package` includes active and contested Core facts, and `render_memory_view` marks contested facts in prompt-facing Core memory.
- `reflection_analyze.md` now instructs the model to use `contradicted_core_fact_ids` only for real contradictions, not harmless refinements.

**What is retracted (if applicable):**
- The prior HISTORY note that contested logic remained future work is now superseded for the manual candidate-review path. A standalone `engine.contest_core_fact`, automatic contested detection outside candidate review, and richer contested-resolution UX remain future work.

**What is still true:**
- Reviewer/reflection agents still cannot write Core directly. They can only propose candidates and contradiction references.
- Core mutation still requires explicit review (`review_candidate(approved)`) or an existing owner/admin patch path.
- Contested facts are preserved as audit trail and remain visible to prompt assembly with a contested marker.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_reflection`
- `cargo test -p memory_engine --test engine_sleep_recall`
- `cargo fmt --check`
- `cargo clippy -p memory_engine --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- Live Gemini check on 2026-06-01: scratch memory seeded `The user lives in Berlin.`, a validated memory unit said the user moved back to Kyiv and Berlin was outdated, `reflection_analyze` on `gemini-2.5-pro` returned a candidate with the old `core_fact_id` in `contradicted_core_fact_ids`, and `review_candidate(approved)` produced one `contested` old fact plus one active promoted Kyiv fact.

## 2026-06-01 — Reflection candidates require manual review before Core promotion

Reflection Phase C starts the controlled path from validated memory units to Core. The core can now ask a host to run `reflection_analyze`, store candidate beliefs, list them, and promote or reject them by explicit owner review. Agents still cannot write Core directly.

**What changed:**
- Added `reflection_result.v1`, `candidate_review_input.v1`, and `candidate_review_result.v1`.
- Added `TaskType::ReflectionAnalyze` execution path through `begin_reflection_analysis` and `submit_reflection_response`.
- Added candidate storage read/list methods for `memory/core/candidates/<candidate_id>.json`.
- `CandidateBelief` now records `source_session_id`, `core_scope`, `source_memory_unit_ids`, `tags`, and optional `promoted_core_fact_id`.
- Added manual candidate review via `review_candidate`: approved candidates are promoted to Core with `source_candidate_id`; rejected candidates stay rejected.
- Telegram host exposes `/reflect`, `/candidates`, `/confirm <id>`, and `/reject <id>`.

**What is retracted (if applicable):**
- The prior HISTORY notes that candidate beliefs and `/reflect` remained future work are now superseded for the first manual-review iteration. Core candidate reviewer/formulation pass, contested logic, auto-confirm, and forgetting remain future work.

**What is still true:**
- The core still has no provider, model, key, prompt directory, or network dependency.
- Reflection reads only validated `MemoryUnit` material for candidate generation.
- Core changes still require explicit review in this iteration; no LLM response directly mutates Core.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_reflection`
- `cargo test -p memory_engine --test engine_sleep_recall`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- Telegram live-check on 2026-06-01: `/candidates` returned no candidates before `/reflect`; `/reflect` scanned 7 memory units and 37 Core facts, then created `candidate_1780300762881461500_3` as `ready_for_review`; `/candidates` listed the same candidate, and Core remained unchanged without `/confirm`.

## 2026-05-31 — Fidelity validation is auto-routed after sleep

The first manual fidelity validator proved useful, but it still required a human/debug command. This change makes the core route only selected memory units to validation after sleep: high-weight units, configured high-risk tag classes, and units whose source events also feed Archive-to-Core personal signals.

**What changed:**
- `SleepOutcome` now includes `fidelity_requests: Vec<LlmRequest>`.
- `finish_sleep_run` creates `MemoryFidelityPass` tasks for selected memory units after memory-unit creation and Archive-to-Core bridge.
- `FidelityConfig` now has `auto_validate_after_sleep`, `auto_validate_weight_threshold`, and `auto_validate_tags`.
- Telegram host executes returned `fidelity_requests` through the same prompt -> text primitive and submits responses back to the core. It does not decide which units need validation.
- Tests cover both sides of the routing policy: low-weight routine units are not validated, while low-weight Core-path units are validated.

**What is retracted (if applicable):**
- The 2026-05-31 note that automatic high-risk/high-weight routing remained a later step is now superseded. Candidate beliefs and Core promotion are still not implemented.

**What is still true:**
- Reviewer agents still do not write truth directly into Core.
- The core still has no provider, model, key, prompt directory, or network dependency.
- Fidelity validation updates `MemoryUnit` status only; candidate beliefs, `/reflect`, `/confirm`, `/reject`, and forgetting remain future v0.2 work.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-31 — Memory fidelity validation uses evidence packs

Reflection Phase B begins with an explicit validation boundary: compressed memory units can now be checked against a small evidence pack before they are trusted for critical paths.

**What changed:**
- Added `evidence_pack.v1`, built by the core from a memory unit's `source_event_ids`, configured neighbor events, unit thesis/evidence, and a token budget.
- Added `fidelity_review.v1`, stored on `MemoryUnit` as `fidelity_review`.
- Added `TaskType::MemoryFidelityPass` with `role_hint: reasoning` and prompt `memory_fidelity_pass`.
- Added core methods to build evidence packs, begin fidelity validation, submit validator responses, and persist review status.
- Added Telegram debug commands `/evidence <memory_unit_id>` and `/fidelity <memory_unit_id>`.
- Fidelity results can mark memory units as active, rejected, or needing revision without writing directly to Core.

**What is retracted (if applicable):**
- Nothing. This implements the planned validator boundary; it does not claim full reflection or automatic Core promotion is complete.

**What is still true:**
- Reviewer agents do not write truth directly into Core.
- Core still has no provider, model, key, prompt directory, or network dependency.
- Automatic routing of only high-risk/high-weight units to validation remains a later step; the current Telegram path is manual/debug.
- Candidate beliefs, `/reflect`, `/confirm`, `/reject`, and full forgetting remain future v0.2 work.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Recall uses time decay and recall feedback

Archive recall now computes an effective freshness at query time instead of treating stored `freshness` as permanently current. Old archive memories sink in rank unless text/theme/tag relevance or recall feedback keeps them useful.

**What changed:**
- `RecallStage1Config` now has configurable freshness decay and recall feedback knobs: `freshness_half_life_days`, `recall_count_log_bonus`, `recent_recall_bonus`, `recent_recall_half_life_days`, and `max_recall_boost_factor`.
- Stage 1 archive scoring uses `effective_freshness = stored_freshness * time_decay(age)` and a bounded recall boost from `recall_count` and `last_recalled_at`.
- Recall scoring is now explicitly time-dependent: `query.created_at` is the deterministic reference time for decay and recent-recall feedback; when hosts omit it, the engine falls back to current UTC time.
- `RecallResult.items[].freshness` now reports effective prompt-time freshness for the recalled item.
- Added regression tests for old-memory decay and recall-feedback boosting.

**What is retracted (if applicable):**
- The previous claim that `freshness` affected recall was incomplete: it was only a stored scalar and did not decay with time.
- `recall_count` and `last_recalled_at` were recorded but did not influence ranking.

**What is still true:**
- This is not full forgetting. No archive entry is deleted or moved to `forgotten/`.
- Full agentic forgetting (`forget_review_pass`) remains a later v0.2 step with audit trail and restore path.
- The same recall query, including the same `created_at`, against the same archive data remains deterministic.
- The Rust core still uses deterministic Stage 1 scoring here; no provider, model, prompt, or network dependency is added.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_recall_`

## 2026-05-30 — Context budget uses prompt-shaped memory estimates

Context budgeting now estimates Core, Archive, and Session items by the compact prompt-facing lines produced by the core prompt-view renderer, not by their full storage/debug JSON structs. This removes storage-only overhead such as `core_fact_id`, `scope`, `tags`, JSON braces, and audit fields from prompt budget decisions.

**What changed:**
- `apply_context_token_budget` now uses prompt-shaped estimators for Core facts, archive memories, recent dialogue, and trace dialogue.
- Archive budget is aligned with the prompt-view archive display limit.
- Added a regression test proving short Core facts with very large storage-only tags are kept because those tags are not part of the rendered prompt memory.

**What is retracted (if applicable):**
- The previous budget estimate over-counted prompt memory by measuring storage JSON. That made Core appear much more expensive than the model-facing prompt actually was.

**What is still true:**
- The estimator remains deliberately conservative (`unicode_chars_div_2_ceil_json_v1` / rendered text chars divided by two).
- The 11k/7k/3k/1k budget split remains unchanged.
- Query-aware ranking from the previous entry still decides which facts should be kept first when the compact Core budget is still full.

**Reproducibility anchor:**
- Runtime check for query `А кішка?`: Core facts kept increased from 7 to 23, dropped Core facts decreased from 22 to 6, and the `pet` fact for Іржа remained first.
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Query-aware Core fact budgeting

Core Store facts are now ranked against the current query before the 1k Core memory budget is applied. Previously, active Core facts were sorted mostly by confidence and category, so when many facts had the same confidence, a directly relevant fact could be dropped only because its category sorted later.

**What changed:**
- `core_context_package` now preserves query-relevant Core facts ahead of unrelated same-confidence facts before token-budget trimming.
- Added a regression test proving that a relevant `pet` Core fact survives a tight Core budget for the query `А кішка?`.

**What is retracted (if applicable):**
- The implicit assumption that confidence-only Core ranking was enough for prompt budgeting. It is not enough once Core contains many equally confident facts.

**What is still true:**
- Core admission rules are unchanged.
- Core token budget remains 1k by default.
- This is not a hardcoded pet/cat rule; it is a general query-token relevance ranking for Core facts.

**Reproducibility anchor:**
- `cargo test -p memory_engine --test engine_sleep_recall engine_core_context_package_keeps_query_relevant_core_fact_under_budget`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Core-owned prompt memory view

The ordinary chat prompt no longer relies on Telegram-specific Python code to decide how memory layers should be rendered. The Rust core now exposes `render_memory_view(package, current_user_message)` and returns the compact LLM-facing memory block with explicit layer geometry: `core_memory`, `long_memory`, `short_memory`, `current_user_message`, and `assistant_response_slot`.

**What changed:**
- Added `crates/memory_engine/src/prompt_view.rs` with canonical compact prompt rendering.
- Exposed `memory_engine::render_memory_view` through the PyO3 adapter.
- Changed the Telegram bot so `chat_prompt(...)` delegates to `engine.render_memory_view(...)` instead of owning memory projection helpers.
- Changed token telemetry so sleep metrics estimate the core-rendered prompt memory view rather than the old Python compact projection.
- Removed the old Telegram host compact projection helpers from the ordinary prompt/debug path.
- Added Rust and Python adapter tests for the prompt-view boundary.

**What is retracted (if applicable):**
- The previous open debt that prompt-view policy lived in Telegram host code. That policy now belongs to the core. The host still owns provider/model/key, prompt files, network execution, and Telegram UX.

**What is still true:**
- `core_context_package` remains the full API/debug shape.
- Debug/admin commands may still show IDs and audit metadata.
- Ordinary chat prompt memory should stay compact and should not carry long technical IDs unless a user command needs them.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --check`
- `python -m py_compile hosts\telegram_gemini_bot\bot.py`
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

## 2026-05-30 — Engine supports shared-reference calls and resource-scoped write locks

After the pull-based sleep driver moved memory orchestration into the core, the next strategic risk was concurrency. The engine still exposed many public calls as `&mut self`, the Python class was marked `unsendable`, and the file-backed Core store had a real lost-update risk because `core/store/<category>.json` is shared by all scopes.

**What changed:**
- `Storage`, `FileStorage`, and public `MemoryEngine` methods now use shared references for normal operations.
- `manifest_initialized` is now atomic and guarded by a manifest resource lock.
- Added an internal resource lock registry for `session:<id>`, `core:<category>`, and manifest operations.
- `ingest()` serializes writes only within the same session; different sessions can run independently.
- Core read-modify-write operations are serialized per category, including Archive-to-Core bridge writes.
- The Python adapter removed `unsendable` and wraps engine calls in `py.allow_threads(...)`.
- Added `concurrency_stress.rs`, including a quick parallel gate and an explicit 1000-session release stress test for Core lost-update detection.

**What is retracted (if applicable):**
- The earlier implicit assumption that a single mutable engine instance was acceptable for future adapters is no longer true. The reusable-library goal requires shared-reference calls and resource-scoped synchronization.

**What is still true:**
- The core still performs no network I/O and knows no provider, model, key, or prompt directory.
- The storage format remains file-based and human-inspectable.
- Core facts are still stored in per-category files for now; this change protects that layout with locks rather than migrating to per-scope directories.

**What we are doing:**
- Keep the explicit 1000-session stress test as the release gate for concurrency work.
- Revisit per-scope Core layout later if shared hot categories become a performance bottleneck.

**Reproducibility anchor:**
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --check`
- `cargo test -p memory_engine --release --test concurrency_stress -- --ignored` (1000-session stress)
- `crates\python_adapter\.venv\Scripts\maturin.exe develop`
- `crates\python_adapter\.venv\Scripts\python.exe -m pytest tests -q`

**Thanks:**
- Mykyta Zagamula for pushing the project back toward a reusable, concurrent memory library before Phase B reflection added more complexity.

## 2026-05-30 — Consolidator now returns prose, not archive JSON

Live Telegram testing of the pull-based sleep driver showed that the core driver, fail-soft path, and Archive-to-Core bridge worked, but `sleep_consolidator` repeatedly failed schema validation. The root cause was the contract: the LLM was asked to return a full nested `sleep_compression_result.v1` JSON object even though the core already had validated track outputs.

**What changed:**
- `sleep_consolidator` now returns plain text using `GIST: ...` plus a narrative paragraph.
- Added `consolidator_text.v1` as the expected output schema for the consolidator request.
- `SleepRun` stores `consolidator_gist` and `consolidator_narrative` instead of a full `consolidated_result` JSON value.
- `finish_sleep_run` always assembles `SleepCompressionResult` deterministically from validated tracks, then overlays the LLM-provided gist/narrative when present.
- The fallback path now writes a neutral assembled narrative from available tracks instead of an error-like placeholder sentence.
- Added driver tests for both successful prose consolidation and fallback after three empty consolidator responses.

**What is retracted (if applicable):**
- The previous consolidator contract was too broad. Asking a prose model to reconstruct a full archive JSON object duplicated core responsibility and made rich sessions fragile.

**What is still true:**
- The core still owns archive structure, track validation, retry, fail-soft, task completion, and Archive-to-Core bridge.
- The host still only loads prompt text, calls a provider, and returns text or an error.
- If the consolidator fails, sleep still completes with full validated tracks and `consolidator_fallback` audit tags.

**What we are doing:**
- Re-run adapter tests and a short Telegram sleep check, then continue to the concurrency branch.

**Thanks:**
- Mykyta Zagamula for catching that "valid JSON from LLM" was the wrong abstraction at the final memory boundary.

## 2026-05-29 — Sleep orchestration moved into the core as a pull-based LLM driver

The owner challenged the project boundary: Memory Engine must be a reusable memory library, not a Telegram bot with memory logic embedded in the host. The Telegram host had accumulated sleep pass ordering, semantic retry, fail-soft fallback, JSON extraction, Archive-to-Core bridge, and task completion policy. That would force future Godot or third-party adapters to reimplement memory behavior.

**What changed:**
- Added core LLM boundary types in `crates/memory_engine/src/llm.rs`: `LlmRequest`, `LlmResponse`, `LlmBatch`, `SleepRun`, `SleepRunStep`, and `SleepOutcome`.
- Added pull-based driver methods: `begin_sleep_run`, `next_sleep_batch`, `submit_sleep_batch`, and `finish_sleep_run`.
- The core now owns sleep pass graph progression, JSON extraction from LLM text, schema validation, semantic retry state, fail-soft empty tracks, consolidator fallback, memory-unit completion, and Archive-to-Core seeding.
- The Python adapter exposes the new driver through JSON-string methods while keeping provider/model/API-key selection outside Rust.
- Telegram sleep execution now runs the generic driver loop and implements only the host primitive: load prompt text, call Gemini, return `{request_id, text}` or `{request_id, error}`.
- `seed_core_from_archives` moved Archive-to-Core bridge backfill into the core.
- `bot.py` dropped the old sleep pass orchestration helpers and shrank from roughly 2650 lines to roughly 1940 lines.

**What is retracted (if applicable):**
- The earlier practical shape of the host was too thick. The claim "hosts are thin adapters" was not structurally true while multi-pass sleep orchestration and Core seeding lived in `bot.py`.

**What is still true:**
- The core still does no network I/O and knows no provider, model, API key, or `prompts_dir`.
- Prompt files still live outside the Rust core; hosts load/render prompt text.
- Provider/network retry remains a host concern. The core handles semantic retry and memory-level fallback after the host returns text or an error.
- Existing direct `sleep()` and `resume_*` APIs remain available for compatibility while the driver becomes the preferred host integration path.

**What we are doing:**
- Next: concurrency hardening. Convert read paths toward `&self`, remove PyO3 `unsendable`, add per-session storage locks, and add a 1000-session stress test before Phase B reflection validation.

**Thanks:**
- Mykyta Zagamula for forcing the reusable-library boundary before Phase B added more host-owned memory intelligence.

## 2026-05-29 — File storage safety before core orchestration

Before moving orchestration, we hardened the file storage layer so the next refactor would not sit on known write and scan hazards.

**What changed:**
- Atomic writes now use unique temp paths in the target directory instead of a shared `.tmp` path.
- `session.md` is append-only and no longer rereads/replaces the full file on every event.
- Completed tasks move to `tasks/completed/`; pending task scans ignore completed tasks while `load_task(id)` can still find them for audit.
- High-frequency derived files avoid unnecessary fsync while durable event and archive/task/core writes remain durable.

**What is retracted (if applicable):**
- Nothing about the storage contract is retracted; this is a safety and scalability correction.

**What is still true:**
- `events.jsonl` remains the session source of truth.
- Completed tasks remain available for audit.

**What we are doing:**
- Use this as the storage baseline for the pull-based core orchestration branch.

**Thanks:**
- Mykyta Zagamula and Claude for identifying fixed temp paths, O(n²) session markdown writes, and unbounded hot task scans as early collapse risks.

## 2026-05-23 — Sleep pass failures no longer leave pending tasks stuck

A live Telegram test showed that a single specialized sleep pass can be blocked by Gemini safety/no-candidates while the other passes are fine. The failed `sleep_personal_signal_pass` left both `sleep_compression` and `compact_memory_pass` tasks pending, which blocked all future sleep for the session.

**What changed:**
- Telegram host now wraps `compact_memory_pass` and the four specialized sleep passes in fail-soft handlers.
- If one pass fails, the host logs the full error, records `pass_failed:<prompt_id>` in archive tags, and continues with an empty track for that pass.
- The sleep task can still complete through consolidator or fallback from remaining tracks.
- The stuck live pending task from `telegram_311422683` was repaired with the new code; all runtime tasks are completed again.

**What is retracted (if applicable):**
- The assumption that robust `sleep_consolidator` handling is enough. Individual upstream passes also need fail-soft handling because provider safety blocks can happen before consolidator runs.

**What is still true:**
- Core promotion still requires `personal_signals`; if `sleep_personal_signal_pass` fails, that archive will not seed Core from personal signals.
- Full traceback stays in `bot.log`; Telegram users should not see it directly.
- This is a host-side LLM reliability fix. Rust core still receives only valid completed results.

**What we are doing:**
- Continue long live testing and watch for `pass_failed:*` tags. If a pass fails often, adjust the prompt or provider/model policy for that pass.

**Thanks:**
- Mykyta Zagamula for running the long test that exposed the stuck pending task path.

## 2026-05-22 — Removed message-count sleep trigger

The owner rejected the message-count sleep path as a test-era shortcut that kept leaking into product planning. The product model has two sleep triggers: token/context budget pressure and scheduled idle sleep during a quiet time window.

**What changed:**
- Removed the message-count trigger configuration from `EngineOptions` and the Python adapter constructor.
- `engine.ingest()` now only stores the event and returns `IngestResult { schema_version, stored_event }`.
- Removed the GUI field and local harness argument for the old trigger.
- Telegram host now queues sleep from token pressure (`MEMORY_BOT_TOKEN_PRESSURE_RATIO`, default 0.80) or scheduled idle sleep (`MEMORY_BOT_IDLE_SLEEP_HOUR`, default 04:00; `MEMORY_BOT_IDLE_SLEEP_MIN_SECONDS`, default 1800).
- Current docs and README describe only `/sleep`, token-pressure sleep, and scheduled idle sleep.

**What is retracted (if applicable):**
- The prior claim that the message-count trigger should remain as a dev/test accelerator or emergency guard. It is removed from current behavior.

**What is still true:**
- `engine.sleep(session_id)` remains the only path that creates sleep tasks.
- Full archive storage, compact memory, token budget, and Archive -> Core bridge behavior are unchanged.
- Historical DEVLOG entries are not rewritten; they remain records of earlier decisions and are superseded by this entry.

**What we are doing:**
- Retest the Telegram host with wiped runtime memory and long human conversations so sleep is driven by token pressure or the idle schedule.

**Thanks:**
- Mykyta Zagamula for forcing the product sleep policy back to context limits and quiet-time consolidation instead of a convenient test counter.

## 2026-05-22 — Removed fixed compact-memory thesis quota

The first `compact_memory_pass` prompt incorrectly told the model to return 5-7 theses. That was a narrow quota, not a memory principle. It could force over-splitting a one-topic conversation or under-represent a session with many distinct memory units.

**What changed:**
- `prompts/compact_memory_pass.md` now tells the model to segment events into coherent memory units and return as many theses as the conversation actually supports.
- A one-topic conversation may produce one thesis.
- Multiple distinct episodes may produce multiple theses.
- Routine repetition can be omitted.
- Timestamps are explicitly available for ordering and time boundaries, but raw ISO timestamps should not be emitted unless useful.
- `docs/architecture.md` and `docs/contracts.md` no longer describe compact memory as a fixed 5-7 item output.

**What is retracted (if applicable):**
- The fixed "5-7 short theses" instruction and the related claim in the previous HISTORY entry.

**What is still true:**
- `compact_memory_pass` remains the prompt-facing compression layer.
- Full multi-track archive entries remain storage/debug records.
- Token budget and telemetry remain in force.

**What we are doing:**
- Treat numeric limits differently depending on their purpose: transport/UI/budget limits can be configured and measured; LLM memory reasoning should not be constrained by arbitrary thematic quotas.

**Thanks:**
- Mykyta Zagamula for catching the quota before it became another hidden architecture constraint.

## 2026-05-22 — Compact memory theses for archive recall

The owner flagged that the full multi-track archive is an audit/enrichment record, not the thing that should be carried back into every chat prompt. A real memory prompt needs short human theses: event -> conclusion. The archive now stores both forms separately.

**What changed:**
- Added `TaskType::CompactMemoryPass` and a persisted `compact_memory_pass` task alongside `sleep_compression` during sleep stage 1.
- Added `ArchiveEntry.compact_memory`, `SleepCompressionResult.compact_memory`, and `RecallItem.compact_memory`.
- Added `MemoryEngine::resume_compact_memory_pass(task_id, text)` and the Python adapter method with the same boundary.
- Added `prompts/compact_memory_pass.md`: plain-text compact theses, no JSON, no debug IDs.
- Telegram sleep now runs compact memory separately from the full multi-pass/consolidator archive enrichment.
- `archive_relevant` now prefers `compact_memory`: when present, recall returns compact theses instead of narrative/facts/quotes in the prompt-facing item.
- Token telemetry now distinguishes raw transcript, stored full archive, prompt archive payload, and compact memory ratios.

**What is retracted (if applicable):**
- The claim that a shortened JSON projection of `gist` plus selected `personal_signals` / `emotional_markers` is the right compressed memory for ordinary chat. It was smaller than full archive JSON, but it still mixed audit structure with prompt memory.

**What is still true:**
- Full multi-track archive entries remain canonical storage/debug records for audit, Core bridge, future reflection, and embeddings.
- The 11k/7k/3k/1k memory prompt budget still applies.
- Rust core still does not call any provider or choose a model.

**What we are doing:**
- Keep `compact_memory` as the ordinary prompt-facing archive layer.
- Keep full multi-track archive data out of normal chat prompts unless the user asks for debug/archive details.
- Defer `forget_review_pass` and vector recall to v0.2.

**Reproducibility anchor:**
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `maturin develop`
- `python -m pytest crates/python_adapter/tests`

**Thanks:**
- Mykyta Zagamula for separating real compression from audit enrichment and for insisting that token economy be measured, not assumed.

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
