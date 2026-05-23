"""Example HostLlmConfig for a Memory Engine host.

The Rust memory_engine core never knows about LLM providers, models, or API
keys. When the engine needs an LLM-driven step (sleep compression, future
reflection, future recall rerank, etc.) it returns a `PendingTask` whose
`role_hint` is one of `reasoning`, `balanced`, `fast`.

This example shows the shape a host (Telegram bot, Godot adapter, etc.) is
expected to maintain on its own side: a mapping from `role_hint` to a
concrete `provider + model + api_key`, plus an executor that takes a
`PendingTask`, runs it, and submits the result back through
`engine.resume_sleep_compression(...)` (or the corresponding entry point).

This file is illustrative, not part of the shipped package. Copy it and
adapt to your host project.
"""

from __future__ import annotations

import json
import os
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class ModelSelection:
    provider: str
    model: str
    api_key_env: str

    def api_key(self) -> str:
        key = os.environ.get(self.api_key_env)
        if not key:
            raise RuntimeError(f"Missing environment variable: {self.api_key_env}")
        return key


@dataclass
class HostLlmConfig:
    """Per-host mapping from Memory Engine role hints to real providers."""

    reasoning: ModelSelection
    balanced: ModelSelection
    fast: ModelSelection

    @classmethod
    def from_toml(cls, path: str | Path) -> "HostLlmConfig":
        with open(path, "rb") as handle:
            data = tomllib.load(handle)
        roles = data["roles"]
        return cls(
            reasoning=ModelSelection(**roles["reasoning"]),
            balanced=ModelSelection(**roles["balanced"]),
            fast=ModelSelection(**roles["fast"]),
        )

    def for_role(self, role_hint: str) -> ModelSelection:
        try:
            return getattr(self, role_hint)
        except AttributeError as err:
            raise ValueError(f"Unknown role hint: {role_hint}") from err


# Example local.llm.toml shape (kept in `config/`, never committed):
#
# [roles.reasoning]
# provider = "anthropic"
# model = "claude-sonnet-4-6"
# api_key_env = "ANTHROPIC_API_KEY"
#
# [roles.balanced]
# provider = "google"
# model = "gemini-2.5-flash"
# api_key_env = "GOOGLE_API_KEY"
#
# [roles.fast]
# provider = "openai"
# model = "gpt-4.1-nano"
# api_key_env = "OPENAI_API_KEY"


def execute_pending_task(
    task: dict[str, Any],
    config: HostLlmConfig,
    prompts_dir: Path,
    call_provider,
) -> dict[str, Any]:
    """Skeleton executor.

    `task` is the dict the engine returned (parsed from `pending_task` in the
    sleep result, or from `pending_tasks()`).

    `call_provider(provider, model, api_key, prompt_text, inputs) -> dict`
    is whatever the host already has - an Anthropic client, OpenAI client,
    Gemini client, etc. It must return a dict matching the task's
    `expected_output_schema`.
    """
    selection = config.for_role(task["role_hint"])
    api_key = selection.api_key()

    prompt_path = prompts_dir / f"{task['prompt_id']}.md"
    prompt_text = prompt_path.read_text(encoding="utf-8")

    return call_provider(
        provider=selection.provider,
        model=selection.model,
        api_key=api_key,
        prompt_text=prompt_text,
        inputs=task["inputs"],
    )


# Typical host loop:
#
# import memory_engine
#
# engine = memory_engine.MemoryEngine("memory", host_id="telegram_bot")
# config = HostLlmConfig.from_toml("config/local.llm.toml")
# prompts_dir = Path("prompts")
#
# # When a session ends:
# sleep_result = json.loads(engine.sleep(session_id))
# task = sleep_result["pending_task"]
# llm_result = execute_pending_task(task, config, prompts_dir, my_call_provider)
# engine.resume_sleep_compression(task["task_id"], json.dumps(llm_result))
