# Configuration

This folder is the expected place for human-editable configuration.

The Rust core must not hardcode providers, model names, API keys, paths, limits, or test modes. Those choices belong in configuration owned by the host project, adapter, or local development environment.

## Files

- `llm.example.toml` - example shape for choosing providers and model roles.
- `local*.toml` - local files for real keys or personal testing. These files are ignored by git.

## Human Rule

One person should be able to open the local config file and see:

- which provider is active;
- which model is used for reasoning tasks;
- which model is used for balanced tasks;
- which model is used for fast cheap tasks;
- where API keys come from;
- where prompts and memory are stored.

No production code should require searching through source files to change these choices.
