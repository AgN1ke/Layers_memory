# Generative Agents Memory Stream — External Pattern Review (2026-07-06)

## Related pages

- [Vector storage implementation TZ](vector-storage-tz-2026-07-03.md) - deep-recall scoring formula this note critiques.
- [Contextual memory expansion](contextual-memory-expansion-2026-07-05.md) - the plan this note's fixes should ride along with.
- [Roadmap](../planning/roadmap.md) - where the resulting action items live.
- [Architecture](../foundation/architecture.md) - Core/Session/Archive layering this pattern is compared against.
- [Reflective Belief System](../planning/reflective-belief-system.md) - accepted
  phased plan that combines this research with durable execution and the
  hierarchical-hypothesis decision.

## Purpose

External source reviewed: [agentpatterns.ai - Generative Agents Memory Stream](https://agentpatterns.ai/patterns/agent-design/generative-agents-memory-stream/),
a summary of the Park et al. "Generative Agents" memory-stream design (observation
stream + weighted retrieval + threshold-triggered reflection).

The owner asked whether this pattern should change our direction. This note
records the comparison against our actual code (not just our docs) and the
three concrete, low-risk findings worth carrying into implementation. It is a
review note, not a new architecture — our layered Core/Archive/vector design is
structurally stronger on the one axis their own writeup admits is a ceiling
(see "What we deliberately do not take").

## The External Pattern (summary)

Three layers:

1. **Observation stream** — natural-language observations with an LLM-assigned
   importance ("poignancy") score. Idle events score low; substantive events
   are scored by a dedicated LLM call per observation.
2. **Retrieval scoring** — composite score over three `[0,1]`-normalized axes:
   `recency*0.5 + relevance*3.0 + importance*2.0`, top 30 nodes enter context.
   Recency decays from *last access*, not from creation time.
3. **Reflection** — triggered when *cumulative importance crosses a threshold*
   (not on a fixed schedule); synthesizes a few higher-level insights from
   recent observations and writes them back into the stream as retrievable
   nodes.

Their own stated limitation: retrieval-based lookup does not consolidate
experience into general rules — "a ceiling on compositionally novel tasks that
more context cannot raise."

## What We Already Do (no action)

- Natural-language atomic memory units instead of raw JSON — this is exactly
  their "observations must be natural-language, raw tool output gives weak
  embedding signal" requirement. Our `MemoryUnit.thesis` already satisfies it.
- Composite retrieval scoring with the same three axes structurally: our
  `recall_deep` is `sim + w_recency*recency + w_unit*weight`
  (`crates/memory_engine/src/engine/vector_flow.rs`), their formula is the same
  three-term shape with different weights and a different recency basis (see
  Finding 1 and 2).
- Importance is assigned in a batch pass over compressed material (sleep), not
  per raw observation. This is a deliberate and, in our view, better choice
  than their per-observation LLM call (see "What we deliberately do not take").

## Findings Worth Acting On

### Finding 1 — Deep-recall importance weight is under-scaled relative to similarity

Their weights give importance 2.0 out of a 5.5 total (36%). Our
`VectorOptions` defaults (`crates/memory_engine/src/engine/options.rs`) give:

```
deep_recall_recency_weight: 0.10
deep_recall_unit_weight:    0.10
```

against a cosine `sim` term that is effectively weight 1.0. Importance is ~8%
of the combined score. In practice this means `recall_deep` barely
distinguishes a high-weight emotionally significant memory from a routine
thesis once their cosine similarities are close.

Action: raise `deep_recall_unit_weight` — not by guessing, but in the same
live-calibration pass already used for `min_sim`/`vivid_sim` (see
[Vector storage TZ, section 8](vector-storage-tz-2026-07-03.md)). Do this
together with `contextual_expansion_min_sim` calibration since both consume
the same scoring path.

### Finding 2 — Our own M4 reinforcement principle is declared but not wired into deep recall

The [vector recall draft](vector-recall.md) already states M4 ("every memory
actually returned gets `recall_count += 1`... frequently recalled memories
rank higher next time, like in humans"). The external pattern's recency term
is exactly this idea made concrete: it decays from *last access*, not from
creation.

Checked against code: `recall_deep` computes `age_days` from
`unit.created_at` only (`vector_flow.rs`, `recall_deep`); `recall_count` and
`last_recalled_at` are not read anywhere in that function. Stage 1 recall
(`RecallStage1Config.recall_count_log_bonus`,
`recent_recall_bonus`) already does this correctly for archive-level recall —
deep recall does not have the equivalent at the memory-unit level.

This is not a new idea from the external source; it is evidence that a
principle we already wrote down for ourselves is not yet implemented at the
unit level. Roadmap already tracks the general version of this
("вага memory unit / archive memory може зростати при recall hit" — see
[Roadmap](../planning/roadmap.md), v0.2.1 section). This finding narrows it to
a concrete, small fix: add a recency-from-last-recall term to
`recall_deep`'s scoring, using unit-level recall stats once those exist (see
the existing "unit-level recall counters" deferred item).

### Finding 3 — Reflection has no threshold-based trigger; sleep does, reflection does not

Sleep already has product triggers (token/context budget pressure, nightly
idle — see [Architecture](../foundation/architecture.md)). `/reflect` today is
manual-only; there is no equivalent accumulated-signal trigger.

The external pattern's reflection trigger ("cumulative importance crosses a
threshold, not a fixed schedule") maps directly onto material we already have:
sum the `weight` of newly `ActiveArchive`+`fidelity_status=valid` memory units
created since the last reflection pass, and queue `begin_reflection_analysis`
in the same background-task style as sleep triggers once that sum crosses a
configurable threshold.

This is additive and does not change the reflection contract itself (agents
still only propose `CandidateBelief`s, Core still changes only through
manual review).

**Implementation update 2026-08-01.** This finding became
[Autonomous Reflection v0.5](../planning/autonomous-reflection-v0.5.md). Phase A
implements the durable trigger/checkpoint cycle over new validated units. The
external flat memory-stream architecture is still rejected; only the
accumulated-salience trigger principle was adopted.

Phase B keeps that rejection intact while adopting the useful cross-time
insight: reflection can span several explicitly bound source sessions of one
personality. New trigger evidence remains distinct from bounded related
history, and the mandatory selector is structural/non-vector rather than one
flat recency/importance/relevance stream.

## Research update 2026-08-01 — from reflection trigger to durable belief formation

The original review answered when reflection should run, but not how a
reflection result survives failure or how several levels of understanding
should coexist. A second research pass compared four adjacent implementation
patterns.

[Letta sleep-time compute](https://www.letta.com/blog/sleep-time-compute/)
validates the separation between the conversational agent and asynchronous
memory work. Their primary agent talks to the user while a sleep-time agent
reorganizes learned context. Memory Engine keeps the same separation of
responsibility without adopting Letta's editable shared memory blocks: the
host executes a bounded reflection task, while Rust owns the memory lifecycle
and Core remains review-gated.

[LangGraph durable execution](https://github.com/langchain-ai/langgraph) and
its [interrupt/idempotency rules](https://docs.langchain.com/oss/python/langgraph/interrupts)
show the production pattern missing from our Phase C happy path: persisted
checkpoints are not sufficient if replay repeats non-idempotent side effects.
Operations that may run again need stable IDs and idempotent writes.

[Temporal's architecture](https://github.com/temporalio/temporal/blob/main/docs/architecture/README.md)
uses an append-only event history, deterministic replay and idempotent
activities to recover long-lived workflows. Memory Engine does not need a
Temporal dependency or a general workflow runtime, but its small local
reflection/review operations need the same narrow principle: write intent
before side effects and resume the same durable operation after a crash.

[Mem0's memory implementation](https://github.com/mem0ai/mem0/blob/main/mem0/memory/main.py)
demonstrates the practical pattern of presenting relevant existing memories
to an LLM before deciding whether new information should add, update or remove
memory. We adopt the comparison-before-write principle, not an unrestricted
LLM memory editor: the model proposes a typed action and Rust validates it.

The owner then identified a limitation in the flat "find the same candidate"
framing. "Interest in colonizing Mars", "interest in colonizing other
planets", and "interest in space" are not necessarily duplicates. They are
claims at different levels of generalization and may deserve different support
strengths. Later independent evidence can strengthen a broader claim without
changing the narrower one.

Accepted conclusion: after v0.5 integrity closure, reflection evolves toward a
reviewable hypothesis graph. Validated MemoryUnits attach through explicit
evidence links; broader hypotheses receive attenuated support while preserving
the original source identity; relations distinguish generalization,
specialization, possible equivalence and contradiction. Confidence remains an
explainable support score, not a claimed probability of truth.

User denial is treated as contradicting evidence and a reason to reconsider
scope, time or wording, not as permission to erase history. A future
clarification opportunity may be surfaced to the conversational agent only
when the topic naturally returns and core-owned relevance/cooldown/budget
rules allow it.

This research does **not** justify adding a permanent third agent, a fourth
memory layer, automatic Core promotion, cloud vectors, or full LangGraph /
Temporal dependencies. The accepted implementation authority is
[Reflective Belief System](../planning/reflective-belief-system.md).

### Finding 4 (structural risk, surfaced by re-reading the formula, not from the external source)

Stage 1 recall's formula is **multiplicative**
(`weight * effective_freshness * recall_boost * theme_factor * tag_factor *
text_factor`), so any near-zero factor kills the whole score. At the default
`freshness_half_life_days: 180`, a six-month-old memory has
`effective_freshness ≈ 0.12`; even at `weight = 1.0` its ceiling score is
`≈0.12`, well below a fresh routine memory at `weight ≈ 0.4`. Old-and-important
memories cannot surface through ordinary Stage 1 recall by design — this is
covered today only by the separate vector deep-recall path, which is gated
behind explicit "remember..." language or contextual expansion's higher
threshold. This is not a bug, but it is the reason Findings 1-3 matter more
than they would in an additive scoring system: the vector path is the *only*
place old importance can currently surface, so its weighting has to be right.

## What We Deliberately Do Not Take

- **Per-observation LLM importance scoring.** The source itself names this as
  the primary cost driver. We assign weight in a batched pass over compressed
  sleep material instead of per raw event; `score_event` remains declared but
  unused by design. We judge episode importance after compression, not per
  reply — cheaper and, we believe, more accurate.
- **Top-30 nodes into context.** Their setting is a cheap-observation
  simulation; our budget discipline (top-`k`≤5, ~1.5k token cap) is a
  deliberate product choice (M2 scarcity in the vector-recall draft), not a
  gap to close.
- **Retrieval as the ceiling architecture.** The source's own stated
  limitation — retrieval cannot consolidate experience into general rules —
  is exactly the gap our Core layer's `candidate -> ready_for_review ->
  confirmed/rejected -> active CoreFact` lifecycle is built to close. Their
  reflection nodes go back into the *same* retrievable stream; our reflection
  candidates go through review before becoming stable, structurally-different
  Core facts. Independently reaching the "index reflection-like output so it
  can be recalled too" shape (their reflection nodes re-entering the stream)
  is one more confirmation that [Contextual expansion Phase 2](contextual-memory-expansion-2026-07-05.md)
  (indexing Core-fact text as vector anchors) is the right next step, not a
  new idea.

## Recommended Sequencing

Do not open a dedicated branch for this. Findings 1 and 2 are parameters and a
scoring-path fix that belong inside the same live-calibration pass already
planned for [Contextual expansion Phase 2](contextual-memory-expansion-2026-07-05.md)
(that work already touches `recall_deep` scoring and requires live
calibration). Finding 3 (reflection auto-trigger) is independent of vectors
and can be picked up separately; it is now tracked as its own roadmap item.

## Reproducibility Anchor

No code changed by this note. Findings verified by direct code inspection on
2026-07-06 against:

- `crates/memory_engine/src/engine/vector_flow.rs` (`recall_deep`);
- `crates/memory_engine/src/engine/options.rs` (`VectorOptions` defaults,
  `RecallStage1Config` defaults);
- `wiki/pages/research/vector-recall.md` (M2/M4 principles);
- `wiki/pages/planning/roadmap.md` (existing weight-growth and reflection
  items).
