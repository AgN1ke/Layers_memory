# Configuration

This folder is the expected place for human-editable configuration.

The Rust core must not hardcode providers, model names, API keys, paths, limits,
or test modes. Those choices belong to the application that embeds the library.

## Files

- `llm.example.toml` - example shape for choosing providers and model roles.
- `local*.toml` - local files for real keys or personal testing. These files are ignored by git.

For the full developer-facing resource contract, see
[`docs/llm-integration-resources.md`](../docs/llm-integration-resources.md).

## Current Implementation Status

The core library is provider-neutral. It emits work by role (`reasoning`,
`balanced`, `fast`) and expects the integrating application to return normalized
results.

The included Telegram and Chibigochi development applications currently ship
with Gemini executors. That means a user can run the included demos with Gemini
keys today. OpenAI, Anthropic/Claude, DeepSeek, Kimi, or another provider require
an executor in the integrating application that maps the same roles to that
provider's API and returns the same engine response shape.

This is integration work, not a memory-core change.

## Required Model Resources

An application embedding the library should expose these resources to the memory
engine:

- `reasoning`: a stronger model for validation, reflection, contradiction
  checks, and repair;
- `balanced`: a normal model for sleep passes and ordinary memory shaping;
- `fast`: a cheaper model for lightweight passes when the product wants to save
  cost;
- `embedding`: optional local embedding model, required only when vector memory
  is enabled.

The same text model can fill all three text roles. A larger product may choose a
mixed setup, for example DeepSeek for reasoning, GPT for balanced work, and a
cheaper Gemini or OpenAI model for fast work. The library only cares that each
role returns the expected normalized result.

## Where Keys Go

Keys must live outside git-tracked source files.

Supported places:

- environment variables, such as `GEMINI_API_KEY`, `OPENAI_API_KEY`,
  `ANTHROPIC_API_KEY`, or `DEEPSEEK_API_KEY`;
- ignored local config files such as `config/local.llm.toml`;
- product-specific settings UI;
- application runtime cache under ignored `runtime/` directories.

For the current Telegram dev application, `run_gui.ps1` and `run_dev_bot.ps1`
use:

```text
hosts/telegram_gemini_bot/runtime/state/secrets.local.json
```

That file is local plaintext developer convenience, ignored by git, and should
not be treated as a production secret store.

## Human Rule

One person should be able to open the local config file and see:

- which provider is active;
- which model is used for reasoning tasks;
- which model is used for balanced tasks;
- which model is used for fast cheap tasks;
- where API keys come from;
- where prompts and memory are stored.

No production code should require searching through source files to change these choices.
