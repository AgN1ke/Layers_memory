# Contextual Memory Expansion and Provider Configuration - 2026-07-05

## Purpose

This document records the next design step after vector storage Phase B and the
owner question about providers, keys, and model choice.

The goal is still token economy with usable memory. The ordinary prompt should
stay small: current conversation, compressed long memory, and stable Core facts.
When the active topic clearly needs detail, the engine should add a small number
of detailed memories related to that topic.

## Current Decision

Contextual memory expansion is a core-side feature.

The host may provide a query embedding for the current turn. The engine then
decides whether there are high-confidence detailed memories worth adding to the
context package. Those details must be scarce, deduplicated against already
visible memory, and counted inside the normal context budget.

This keeps the host thin. Telegram, Godot, or another product should not each
invent their own rules for when a Core fact needs supporting memory.

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

## Provider and Key Boundary

The Rust core does not own provider keys and does not choose Google, OpenAI,
Anthropic, DeepSeek, or any other vendor.

The core only says what kind of work it needs:

- reasoning: harder semantic work, validation, reflection, repair;
- balanced: ordinary sleep passes, chat host defaults, moderate semantic work;
- fast: cheap/simple work and embedding tasks where the host supports them.

The host maps those roles to concrete providers and model names.

For example, one host may map:

- reasoning -> Anthropic Claude;
- balanced -> Gemini Flash;
- fast -> DeepSeek or a small OpenAI model.

Another host may map all three roles to Gemini. Both are valid as long as the
host returns the expected response shape to the engine.

## Where Keys Live

Keys live in the host application or local environment.

For the current Telegram development host:

- GUI/dev harness stores keys in
  `hosts/telegram_gemini_bot/runtime/state/secrets.local.json`;
- that runtime directory is ignored by git;
- environment variables can override local cache values;
- the `/models` command shows the active role-to-model mapping.

For a future packaged product, the product must provide its own settings UI or
configuration file. The library should document the required roles and response
contracts, then let the host decide how users enter keys.

## Current Limitation

The example hosts currently implement Gemini execution paths. The core is
provider-agnostic, and `config/llm.example.toml` already shows the intended
provider shape for OpenAI, Google, Anthropic, Kimi, and DeepSeek.

Supporting GPT, Claude, DeepSeek, or another provider is host work:

1. read the user's key securely;
2. map `reasoning`, `balanced`, and `fast` roles to provider model names;
3. call the provider API;
4. normalize the provider result into the existing engine response shape.

The memory logic should not change for each provider.

## Implementation Plan

### Step 1 - Documented Config Contract

Add a short host-facing provider contract:

- required roles: reasoning, balanced, fast;
- optional embedding provider, with local embedding as the recommended default;
- key sources: environment variables, ignored local config, or product UI;
- normalized output expected by the engine.

Acceptance: a new host author can tell where keys go and how to choose models
without reading the Telegram bot code.

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

Do not make every host inject its own expansion text into the prompt.

Do not start Phase 2 storage changes before Phase 1 proves the behavior with the
existing memory-unit vector index.

## Next Concrete Work

First implement Step 1 as documentation and small host cleanup if needed.

Then implement Step 2 on a feature branch with deterministic conformance before
running it in Telegram.
