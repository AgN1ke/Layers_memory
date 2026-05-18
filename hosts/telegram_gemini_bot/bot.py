"""Simple Telegram + Gemini host for Memory Engine.

This host is deliberately outside the Rust core. It owns Telegram polling,
Gemini API calls, API keys, and model selection.
"""

from __future__ import annotations

import getpass
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    import memory_engine
except ImportError as err:
    raise SystemExit(
        "Could not import memory_engine. Run hosts\\telegram_gemini_bot\\run.ps1 "
        "so maturin can build and install the PyO3 adapter."
    ) from err


ROOT = Path(__file__).resolve().parents[2]
PROMPTS_DIR = ROOT / "prompts"
RUNTIME_DIR = Path(__file__).resolve().parent / "runtime"
MEMORY_DIR = RUNTIME_DIR / "memory"

DEFAULT_REASONING_MODEL = "gemini-2.5-pro"
DEFAULT_BALANCED_MODEL = "gemini-2.5-flash"
DEFAULT_FAST_MODEL = "gemini-2.5-flash-lite"
DEFAULT_CHAT_ROLE = "balanced"

TELEGRAM_API = "https://api.telegram.org"
GEMINI_API = "https://generativelanguage.googleapis.com/v1beta"
MEMORY_KEYWORDS = (
    "запам'ятай",
    "запамʼятай",
    "пам'ятай",
    "памʼятай",
    "важливо",
)


@dataclass(frozen=True)
class ModelSelection:
    provider: str
    model: str


@dataclass(frozen=True)
class HostLlmConfig:
    reasoning: ModelSelection
    balanced: ModelSelection
    fast: ModelSelection
    chat_role: str

    def for_role(self, role_hint: str) -> ModelSelection:
        if role_hint == "reasoning":
            return self.reasoning
        if role_hint == "balanced":
            return self.balanced
        if role_hint == "fast":
            return self.fast
        raise ValueError(f"Unknown model role: {role_hint}")

    def chat_model(self) -> ModelSelection:
        return self.for_role(self.chat_role)


class TelegramClient:
    def __init__(self, token: str) -> None:
        self.token = token

    def call(self, method: str, data: dict[str, Any] | None = None) -> dict[str, Any]:
        url = f"{TELEGRAM_API}/bot{self.token}/{method}"
        encoded = None
        headers = {}
        if data is not None:
            encoded = urllib.parse.urlencode(data).encode("utf-8")
            headers["Content-Type"] = "application/x-www-form-urlencoded"

        request = urllib.request.Request(url, data=encoded, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as err:
            body = err.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"Telegram {method} failed: HTTP {err.code}: {body}") from err
        except urllib.error.URLError as err:
            raise RuntimeError(f"Telegram {method} failed: {err}") from err

        if not payload.get("ok"):
            raise RuntimeError(f"Telegram {method} returned error: {payload}")
        return payload

    def delete_webhook(self) -> None:
        self.call("deleteWebhook", {"drop_pending_updates": "false"})

    def get_updates(self, offset: int | None) -> list[dict[str, Any]]:
        data: dict[str, Any] = {
            "timeout": 30,
            "limit": 50,
            "allowed_updates": json.dumps(["message"]),
        }
        if offset is not None:
            data["offset"] = offset
        return self.call("getUpdates", data)["result"]

    def send_message(self, chat_id: int, text: str) -> None:
        for chunk in chunk_text(text, 3900):
            self.call("sendMessage", {"chat_id": chat_id, "text": chunk})


class GeminiClient:
    def __init__(self, api_key: str) -> None:
        self.api_key = api_key

    def generate_text(self, model: str, system_instruction: str, prompt: str) -> str:
        url_model = urllib.parse.quote(model, safe="")
        url = f"{GEMINI_API}/models/{url_model}:generateContent"
        payload = {
            "system_instruction": {"parts": [{"text": system_instruction}]},
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        }
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        request = urllib.request.Request(
            url,
            data=data,
            headers={
                "Content-Type": "application/json",
                "x-goog-api-key": self.api_key,
            },
            method="POST",
        )

        try:
            with urllib.request.urlopen(request, timeout=90) as response:
                result = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as err:
            body = err.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"Gemini {model} failed: HTTP {err.code}: {body}") from err
        except urllib.error.URLError as err:
            raise RuntimeError(f"Gemini {model} failed: {err}") from err

        return extract_gemini_text(result)


def main() -> None:
    print("Telegram Gemini Memory Bot")
    print(f"Runtime memory: {MEMORY_DIR}")
    print("Keys are read from terminal and are not stored.")
    print()

    telegram_token = read_secret("Telegram bot token", "TELEGRAM_BOT_TOKEN")
    gemini_key = read_secret("Gemini API key", "GEMINI_API_KEY")
    llm_config = read_model_config()

    MEMORY_DIR.mkdir(parents=True, exist_ok=True)
    engine = memory_engine.MemoryEngine(str(MEMORY_DIR), host_id="telegram_gemini_bot")
    telegram = TelegramClient(telegram_token)
    gemini = GeminiClient(gemini_key)

    telegram.delete_webhook()
    print("Bot is running. Open Telegram and write to your bot.")
    print("Commands: /help, /sleep, /recall text, /tasks, /models")

    offset: int | None = None
    while True:
        try:
            updates = telegram.get_updates(offset)
            for update in updates:
                offset = update["update_id"] + 1
                handle_update(update, telegram, gemini, engine, llm_config)
        except KeyboardInterrupt:
            print("\nStopped.")
            return
        except Exception as err:  # Keep the bot alive during temporary network/API errors.
            print(f"[error] {err}", file=sys.stderr)
            time.sleep(3)


def handle_update(
    update: dict[str, Any],
    telegram: TelegramClient,
    gemini: GeminiClient,
    engine: memory_engine.MemoryEngine,
    llm_config: HostLlmConfig,
) -> None:
    message = update.get("message") or {}
    text = (message.get("text") or "").strip()
    chat = message.get("chat") or {}
    chat_id = chat.get("id")
    user = message.get("from") or {}

    if not text or chat_id is None:
        return

    session_id = f"telegram_{chat_id}"

    if text in {"/start", "/help"}:
        telegram.send_message(chat_id, help_text())
        return

    if text == "/models":
        telegram.send_message(chat_id, model_text(llm_config))
        return

    if text == "/tasks":
        tasks = json.loads(engine.pending_tasks())
        telegram.send_message(chat_id, format_tasks(tasks))
        return

    if text == "/sleep":
        telegram.send_message(chat_id, run_sleep(engine, gemini, llm_config, session_id))
        return

    if text.startswith("/recall"):
        query = text.removeprefix("/recall").strip() or text
        telegram.send_message(chat_id, format_recall(recall(engine, session_id, query, explain=True)))
        return

    stored = json.loads(
        engine.ingest(
            json.dumps(
                {
                    "schema_version": "event.v1",
                    "type": "user_message",
                    "source": f"telegram_user_{user.get('id', 'unknown')}",
                    "timestamp": now_rfc3339(),
                    "session_id": session_id,
                    "payload": {
                        "text": text,
                        "telegram_chat_id": chat_id,
                        "telegram_message_id": message.get("message_id"),
                    },
                    "tags": ["telegram_message"],
                    "theme": "telegram_conversation",
                    "importance_hint": importance_hint(text),
                },
                ensure_ascii=False,
            )
        )
    )

    memory_context = format_memory_context(recall(engine, session_id, text, explain=False))
    model = llm_config.chat_model().model
    answer = gemini.generate_text(
        model=model,
        system_instruction=chat_system_instruction(),
        prompt=chat_prompt(memory_context, text),
    )
    telegram.send_message(chat_id, answer)
    print(f"chat={chat_id} event={stored['event_id']} model={model}")

    if should_auto_sleep(text):
        summary = run_sleep(engine, gemini, llm_config, session_id)
        telegram.send_message(chat_id, f"Memory updated.\n\n{summary}")


def run_sleep(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    session_id: str,
) -> str:
    sleep_result = json.loads(engine.sleep(session_id))
    task = sleep_result["pending_task"]
    archive = sleep_result["archive_entry"]
    llm_result = execute_sleep_compression(task, gemini, llm_config)
    updated = json.loads(
        engine.resume_sleep_compression(task["task_id"], json.dumps(llm_result, ensure_ascii=False))
    )
    return (
        f"Archive: {archive['archive_id']}\n"
        f"Task: {task['task_id']}\n"
        f"Model role: {task['role_hint']}\n"
        f"Model: {llm_config.for_role(task['role_hint']).model}\n"
        f"Gist: {updated['gist']}"
    )


def execute_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    prompt_path = PROMPTS_DIR / f"{task['prompt_id']}.md"
    prompt_text = prompt_path.read_text(encoding="utf-8")
    selection = llm_config.for_role(task["role_hint"])
    raw = gemini.generate_text(
        model=selection.model,
        system_instruction=prompt_text,
        prompt=json.dumps(task["inputs"], ensure_ascii=False, indent=2),
    )
    parsed = parse_json_object(raw)
    if parsed.get("archive_id") != task["inputs"]["preliminary_archive_id"]:
        parsed["archive_id"] = task["inputs"]["preliminary_archive_id"]
    parsed.setdefault("schema_version", task["expected_output_schema"])
    parsed.setdefault("links", [])
    return parsed


def recall(engine: memory_engine.MemoryEngine, session_id: str, query_text: str, explain: bool) -> dict[str, Any]:
    return json.loads(
        engine.recall(
            json.dumps(
                {
                    "schema_version": "recall_query.v1",
                    "session_id": session_id,
                    "created_at": now_rfc3339(),
                    "context": {"recent_text": query_text},
                    "query_text": query_text,
                    "filters": {"source_layers": ["archive"]},
                    "limit": 5,
                    "include_core": False,
                    "explain": explain,
                },
                ensure_ascii=False,
            )
        )
    )


def read_secret(label: str, env_name: str) -> str:
    value = os.environ.get(env_name)
    if value:
        print(f"{label}: using {env_name} from environment")
        return value
    while True:
        value = getpass.getpass(f"{label}: ").strip()
        if value:
            return value
        print("Value must not be empty.")


def read_model_config() -> HostLlmConfig:
    print("Gemini model mapping. Press Enter to keep defaults.")
    reasoning = input_default("reasoning model", DEFAULT_REASONING_MODEL)
    balanced = input_default("balanced model", DEFAULT_BALANCED_MODEL)
    fast = input_default("fast model", DEFAULT_FAST_MODEL)
    chat_role = input_default("chat reply role", DEFAULT_CHAT_ROLE)
    if chat_role not in {"reasoning", "balanced", "fast"}:
        print(f"Unknown chat role {chat_role!r}; using {DEFAULT_CHAT_ROLE}.")
        chat_role = DEFAULT_CHAT_ROLE
    return HostLlmConfig(
        reasoning=ModelSelection("google", reasoning),
        balanced=ModelSelection("google", balanced),
        fast=ModelSelection("google", fast),
        chat_role=chat_role,
    )


def input_default(label: str, default: str) -> str:
    value = input(f"{label} [{default}]: ").strip()
    return value or default


def chat_system_instruction() -> str:
    return (
        "You are a concise Telegram assistant. Use the provided memory context when it is relevant. "
        "If memory context is empty or irrelevant, answer normally. Do not claim you remember things "
        "unless they are present in memory context or the current user message."
    )


def chat_prompt(memory_context: str, user_text: str) -> str:
    return f"Memory context:\n{memory_context}\n\nUser message:\n{user_text}"


def format_memory_context(recall_result: dict[str, Any]) -> str:
    items = recall_result.get("items", [])
    if not items:
        return "(no archive memory found)"
    lines = []
    for item in items:
        lines.append(f"- {item['gist']}")
        if item.get("narrative"):
            lines.append(f"  {item['narrative']}")
    return "\n".join(lines)


def format_recall(recall_result: dict[str, Any]) -> str:
    items = recall_result.get("items", [])
    if not items:
        return "No archive memory found yet. Write something important, then use /sleep."
    lines = ["Recall:"]
    for index, item in enumerate(items, start=1):
        lines.append(f"{index}. [{item['relevance_score']:.2f}] {item['gist']}")
        if item.get("narrative"):
            lines.append(f"   {item['narrative']}")
        if item.get("relevance_explanation"):
            lines.append(f"   {item['relevance_explanation']}")
    return "\n".join(lines)


def format_tasks(tasks: list[dict[str, Any]]) -> str:
    if not tasks:
        return "No pending tasks."
    lines = ["Pending tasks:"]
    for task in tasks:
        lines.append(
            f"- {task['task_id']} {task['task_type']} {task['state']} "
            f"role={task['role_hint']} prompt={task['prompt_id']}"
        )
    return "\n".join(lines)


def model_text(config: HostLlmConfig) -> str:
    return (
        "Model mapping:\n"
        f"reasoning: {config.reasoning.model}\n"
        f"balanced: {config.balanced.model}\n"
        f"fast: {config.fast.model}\n"
        f"chat replies: {config.chat_role} -> {config.chat_model().model}"
    )


def help_text() -> str:
    return (
        "Memory bot commands:\n"
        "/sleep - commit current session into archive memory\n"
        "/recall text - search archive memory\n"
        "/tasks - show pending tasks\n"
        "/models - show model role mapping\n"
        "\n"
        "Plain text is stored as an event and answered by Gemini with memory context.\n"
        "Messages containing 'запамʼятай', 'памʼятай', or 'важливо' auto-run /sleep."
    )


def importance_hint(text: str) -> str:
    lowered = text.lower()
    if should_auto_sleep(lowered):
        return "high"
    return "normal"


def should_auto_sleep(text: str) -> bool:
    lowered = text.lower()
    return any(keyword in lowered for keyword in MEMORY_KEYWORDS)


def now_rfc3339() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def extract_gemini_text(result: dict[str, Any]) -> str:
    candidates = result.get("candidates") or []
    if not candidates:
        raise RuntimeError(f"Gemini returned no candidates: {result}")
    parts = candidates[0].get("content", {}).get("parts") or []
    texts = [part.get("text", "") for part in parts if part.get("text")]
    if not texts:
        raise RuntimeError(f"Gemini returned no text parts: {result}")
    return "\n".join(texts).strip()


def parse_json_object(raw: str) -> dict[str, Any]:
    text = raw.strip()
    if text.startswith("```"):
        text = re.sub(r"^```(?:json)?\s*", "", text, flags=re.IGNORECASE)
        text = re.sub(r"\s*```$", "", text)
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        match = re.search(r"\{.*\}", text, flags=re.DOTALL)
        if not match:
            raise
        parsed = json.loads(match.group(0))
    if not isinstance(parsed, dict):
        raise ValueError("Expected JSON object from Gemini")
    return parsed


def chunk_text(text: str, limit: int) -> list[str]:
    if len(text) <= limit:
        return [text]
    chunks = []
    start = 0
    while start < len(text):
        chunks.append(text[start : start + limit])
        start += limit
    return chunks


if __name__ == "__main__":
    main()
