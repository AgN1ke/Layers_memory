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

The Telegram host uses these prompts by default during `/sleep` and auto-sleep.
The Rust core creates one durable lossless Sleep input and one compact
`sleep_prompt_context.v1` projection shared by the selected passes. A host must
place the shared context first, the pass instruction second, and the small
request-specific inputs last. This stable prefix allows provider-side prefix
caching without copying the full event JSON into every request.

The full default profile contains six tracks: memory units, emotional markers,
topic thread, personal signals, relational tone, and the prose consolidator.
`begin_sleep_run_with_options` may select a smaller profile and set a whole-run
provider-call limit. Omitted options preserve the full profile.

`sleep_emotional_pass.md` finds emotionally salient moments supported by events and returns `sleep_emotional_pass_result.v1` with `emotional_markers`.

`sleep_topic_thread_pass.md` preserves the chronological topic thread and returns `sleep_topic_thread_pass_result.v1` with `topic_thread`.

`sleep_personal_signal_pass.md` finds user-specific signals useful for future reflection and returns `sleep_personal_signal_pass_result.v1` with `personal_signals`.

`sleep_relational_pass.md` estimates the supported relational tone and returns `sleep_relational_pass_result.v1` with `relational_tone`.

`sleep_consolidator.md` receives the same shared source plus the available pass
outputs. It returns the plain-text `GIST:` and narrative overlay; the Rust core
assembles the final `sleep_compression_result.v1` deterministically.

Prompt version 3 of the multi-pass Sleep prompts uses the compact shared input.
Participant attribution introduced in version 2 remains part of
`sleep_personal_signal_pass.md` and `memory_unit_pass.md`. Outputs may include
`subject_hint` (`self_statement` or `about_other`) and `speaker_id`; these are
untrusted hints that the Rust core checks against cited source events.
`memory_fidelity_pass.md` version 2 also checks whether the compressed thesis
kept the correct author. `sleep_consolidator.md` version 2 preserves concrete
participant names in prose.

Safe to edit: wording, examples, strictness, salience priorities, and output guidance.

Do not change without code/contracts updates: prompt filenames, schema names,
required top-level JSON fields, and the rule that the consolidator returns the
plain-text `consolidator_text.v1` form that starts with `GIST:`.

### `reflection_analyze.md`

Used by manual reflection and by v0.5 autonomous reflection after the Rust core
decides that validated memory has reached critical mass or the interval safety
net. The host does not choose the evidence or decide whether reflection is due.

Prompt version 3 receives validated `MemoryUnit` theses from every source
session explicitly bound to the personality `core_scope`. New `trigger` units
are separate from bounded `related_history`; parent archive summaries include
the structural metadata that explains selection. Visible Core facts, existing
draft/ready candidates, source identifiers, and operational trigger metadata
are also present. It returns `reflection_actions_result.v1`: a new candidate,
reinforcement of an existing candidate with new evidence, a reviewable Core
challenge, or structured no-insight. It never writes Core.

Safe to edit: wording about stability, temporary state, evidence quality,
contradictions, and the preference for zero candidates over invented meaning.

Do not change without code/contracts/tests updates:
`reflection_actions_result.v1`, its exact-one-of actions/no-insight rule,
required source unit provenance, candidate field names, or the rule that Core
changes only through review. `reflection_result.v1` remains a legacy readable
schema for already-saved tasks and older hosts. Successful typed application
returns `reflection_apply_result.v1`.
