# Prompts

This folder is the only allowed place for real prompt text used by Memory Engine.

No prompt file should be created before a real feature needs it. Empty future prompts make the project harder to understand.

## Required Rule

When a prompt is added, document it here in human language:

- what it does;
- when it runs;
- what data it receives;
- what it must return;
- what can be safely edited;
- what must not be changed without updating the contract or tests.

## Current Prompts

### `sleep_compression.md`

Used after `MemoryEngine::sleep(session_id)` creates a preliminary archive entry and a `PendingTask` with `prompt_id: "sleep_compression"`.

Human purpose: turn selected session events into one durable memory item with a short gist, narrative, facts, quotes, tags, theme, and weight.

The Rust core does not execute this prompt. The host or adapter reads the prompt file, chooses the provider/model from host configuration, executes the LLM call, and returns `sleep_compression_result.v1` to the engine.

### Multi-pass sleep prompts

The Telegram host now uses these prompts by default during `/sleep` and auto-sleep. The Rust core still creates one `sleep_compression` task; the host performs multiple LLM passes around that task and returns one `sleep_compression_result.v1`.

`sleep_emotional_pass.md` finds emotionally salient moments supported by events and returns `sleep_emotional_pass_result.v1` with `emotional_markers`.

`sleep_topic_thread_pass.md` preserves the chronological topic thread and returns `sleep_topic_thread_pass_result.v1` with `topic_thread`.

`sleep_personal_signal_pass.md` finds user-specific signals useful for future reflection and returns `sleep_personal_signal_pass_result.v1` with `personal_signals`.

`sleep_relational_pass.md` estimates the supported relational tone and returns `sleep_relational_pass_result.v1` with `relational_tone`.

`sleep_consolidator.md` receives the original sleep task plus all pass outputs and returns the final `sleep_compression_result.v1` with both legacy fields and multi-track fields.

Safe to edit: wording, examples, strictness, salience priorities, and output guidance.

Do not change without code/contracts updates: prompt filenames, schema names, required top-level JSON fields, and the rule that consolidator returns `sleep_compression_result.v1`.
