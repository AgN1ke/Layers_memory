# Contextual Memory Expansion and Provider Configuration - 2026-07-05

## Related pages

- [Vector storage implementation TZ](vector-storage-tz-2026-07-03.md) - vector layer this plan builds on.
- [LLM integration resources](../integration/llm-integration-resources.md) - developer-facing resource contract for models, keys, and embeddings.
- [Provider landscape](provider-landscape.md) - provider research behind model choices.
- [Contracts](../foundation/contracts.md) - context package and request shapes affected by expansion.
- [Roadmap](../planning/roadmap.md) - where expansion fits after v0.3 and vector Phase B.

## Purpose

This document records the next design step after vector storage Phase B and the
owner question about providers, keys, and model choice.

The concrete developer-facing resource contract lives in
[`wiki/pages/integration/llm-integration-resources.md`](../integration/llm-integration-resources.md).

The goal is still token economy with usable memory. The ordinary prompt should
stay small: current conversation, compressed long memory, and stable Core facts.
When the active topic clearly needs detail, the engine should add a small number
of detailed memories related to that topic.

## Current Decision

Contextual memory expansion is a core-side feature.

The application that embeds the library may provide a query embedding for the
current turn. The engine then decides whether there are high-confidence detailed
memories worth adding to the context package. Those details must be scarce,
deduplicated against already visible memory, and counted inside the normal
context budget.

This keeps memory policy inside the library. A chat app, game, web product, or
other interface is only the surface; the actual integrating program supplies
model resources and calls the library, while the library decides how memory is
shaped.

## User-Facing Meaning

From a product user's point of view, memory should feel like this:

- The assistant always sees the short stable facts that matter.
- If the conversation turns to a known topic, the assistant can recall a few
  concrete old details about that topic.
- The assistant should not dump a database of old memories into every reply.
- If the relevant detail is already visible in the normal memory package, the
  expansion step should add nothing.

Example: Core may say "the user has a cat named Irzha". When the current topic
is Irzha, the engine may add a few detailed memory units about Irzha's coloring,
the naming story, or earlier discussion around her.

## LLM Resource Contract

The Rust core does not own provider keys and does not choose Google, OpenAI,
Anthropic, DeepSeek, or any other vendor.

The library must make its needs clear to the program that embeds it. The
integrating program provides model resources for these roles:

- reasoning: stronger model for validation, reflection, contradiction checks,
  and repair;
- balanced: normal semantic work such as sleep passes and ordinary memory
  shaping;
- fast: cheap/simple model for lightweight passes when the product wants to
  save cost;
- embedding: optional local embedding resource, required only when vector
  features are enabled.

These are roles, not vendors. The same product may map them to any provider mix.
One model may fill several roles if the product owner wants a simpler setup.

For example, one integrating program may map:

- reasoning -> Anthropic Claude;
- balanced -> Gemini Flash;
- fast -> DeepSeek or a small OpenAI model.

Another program may map reasoning to DeepSeek, balanced to GPT, and fast to
Gemini. Another may map all three text roles to one Gemini model. All are valid
as long as the program returns the expected response shape to the engine.

The minimum text setup is one capable model mapped to all text roles. The
recommended production setup is at least one stronger model for reasoning and
one cheaper model for routine work. The embedding resource is separate and is
not needed when vector memory is disabled.

## Where Keys Live

Keys live in the integrating application or local environment.

For the current Telegram development application:

- GUI/dev harness stores keys in
  `hosts/telegram_gemini_bot/runtime/state/secrets.local.json`;
- that runtime directory is ignored by git;
- environment variables can override local cache values;
- the `/models` command shows the active role-to-model mapping.

For a future packaged product, the product must provide its own settings UI or
configuration file. The library documents the required roles and response
contracts; the integrating program decides how users enter keys and which
providers fill each role.

## Current Limitation

The included demo applications currently implement Gemini execution paths. The
core is provider-agnostic, and `config/llm.example.toml` already shows the
intended provider shape for OpenAI, Google, Anthropic, Kimi, and DeepSeek.

Supporting GPT, Claude, DeepSeek, or another provider means adding an executor
in the integrating program:

1. read the user's key securely;
2. map `reasoning`, `balanced`, and `fast` roles to provider model names;
3. call the provider API;
4. normalize the provider result into the existing engine response shape.

The memory logic should not change for each provider.

## Implementation Plan

### Step 1 - Documented Config Contract

Add a short integrator-facing provider contract:

- required roles: reasoning, balanced, fast;
- optional embedding provider, with local embedding as the recommended default;
- key sources: environment variables, ignored local config, or product UI;
- normalized output expected by the engine.

Acceptance: a new application author can tell what model resources the library
needs, where keys go, and how to choose providers without reading the Telegram
demo code.

### Step 2 - Contextual Expansion Phase 1

Add `query_embedding` to the context request path and keep behavior unchanged
when it is absent.

When present and the vector scope is ready, the engine searches memory units
with a higher threshold than explicit distant recall, selects at most a few
details, removes anything already covered by visible memory, and places the
details into the context package under the existing budget.

Acceptance:

- no query embedding -> byte-identical context package;
- query about a topic with known detailed memory -> one to three relevant
  details appear;
- query where Core/long memory already covers the detail -> nothing extra is
  added;
- unrelated query -> nothing extra is added.

### Step 3 - Core-Anchored Expansion Phase 2

Add vector rows for Core facts as anchors, then connect Core facts to supporting
memory units through existing provenance links.

This gives the product behavior the owner described: short Core facts stay
visible, while detailed memories can unfold around the active Core topic.

Acceptance:

- Core fact match can pull supporting archive details;
- disputed or deprecated Core facts do not silently pull stale detail;
- token budget remains authoritative.

## What Not To Do Now

Do not move provider keys into the Rust core.

Do not make every integrating program inject its own expansion text into the
prompt.

Do not start Phase 2 storage changes before Phase 1 proves the behavior with the
existing memory-unit vector index.

## Next Concrete Work

First implement Step 1 as documentation and small integration cleanup if needed.

Then implement Step 2 on a feature branch with deterministic conformance before
running it through the included Telegram demo application.
