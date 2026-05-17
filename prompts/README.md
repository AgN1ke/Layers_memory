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
