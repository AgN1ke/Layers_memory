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
import traceback
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
LOG_DIR = RUNTIME_DIR / "logs"
LOG_PATH = LOG_DIR / "bot.log"
STATE_DIR = RUNTIME_DIR / "state"
OFFSET_PATH = STATE_DIR / "telegram_offset.json"

DEFAULT_REASONING_MODEL = "gemini-2.5-pro"
DEFAULT_BALANCED_MODEL = "gemini-2.5-flash"
DEFAULT_FAST_MODEL = "gemini-2.5-flash-lite"
DEFAULT_CHAT_ROLE = "balanced"
RECENT_CONTEXT_LIMIT = 40
SESSION_TRACE_EVENT_LIMIT = 120

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
    print(f"Runtime log: {LOG_PATH}")
    print("Keys are read from terminal and are not stored.")
    print()
    log_line("starting Telegram Gemini Memory Bot")

    telegram_token = read_secret("Telegram bot token", "TELEGRAM_BOT_TOKEN")
    gemini_key = read_secret("Gemini API key", "GEMINI_API_KEY")
    llm_config = read_model_config()

    MEMORY_DIR.mkdir(parents=True, exist_ok=True)
    engine = memory_engine.MemoryEngine(str(MEMORY_DIR), host_id="telegram_gemini_bot")
    telegram = TelegramClient(telegram_token)
    gemini = GeminiClient(gemini_key)

    telegram.delete_webhook()
    log_line("deleteWebhook completed")
    print("Bot is running. Open Telegram and write to your bot.")
    print("Commands: /help, /sleep, /recall text, /tasks, /models")
    offset = read_saved_offset()
    log_line(f"bot polling started offset={offset}")

    while True:
        try:
            updates = telegram.get_updates(offset)
            if updates:
                log_line(f"poll received {len(updates)} update(s), offset={offset}")
            for update in updates:
                update_id = update.get("update_id")
                try:
                    handle_update(update, telegram, gemini, engine, llm_config)
                except Exception as err:
                    log_exception(f"update {update_id} failed", err)
                    notify_update_error(update, telegram, err)
                finally:
                    if update_id is not None:
                        offset = update_id + 1
                        save_offset(offset)
        except KeyboardInterrupt:
            log_line("bot stopped by keyboard interrupt")
            print("\nStopped.")
            return
        except Exception as err:  # Keep the bot alive during temporary network/API errors.
            log_exception("polling failed", err)
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
        log_line("ignored update without text or chat_id")
        return

    session_id = f"telegram_{chat_id}"
    log_line(f"handling chat={chat_id} message_id={message.get('message_id')} text={truncate_chars(text, 160)}")

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

    user_ingest = ingest_chat_event(
        engine=engine,
        session_id=session_id,
        event_type="user_message",
        source=f"telegram_user_{user.get('id', 'unknown')}",
        text=text,
        tags=["telegram_message"],
        importance=importance_hint(text),
        payload_extra={
            "telegram_chat_id": chat_id,
            "telegram_message_id": message.get("message_id"),
        },
    )

    package = context_package(engine, session_id, chat_id, text)
    model = llm_config.chat_model().model
    answer = gemini.generate_text(
        model=model,
        system_instruction=chat_system_instruction(),
        prompt=chat_prompt(package, text),
    )
    telegram.send_message(chat_id, answer)
    assistant_ingest = ingest_chat_event(
        engine=engine,
        session_id=session_id,
        event_type="assistant_message",
        source="telegram_bot",
        text=answer,
        tags=["telegram_reply"],
        importance="normal",
        payload_extra={
            "telegram_chat_id": chat_id,
            "model": model,
        },
    )
    stored = user_ingest["stored_event"]
    log_line(f"answered chat={chat_id} event={stored['event_id']} model={model}")
    print(f"chat={chat_id} event={stored['event_id']} model={model}")

    if should_auto_sleep(text):
        auto_sleep_results = [
            result["auto_sleep"]
            for result in (user_ingest, assistant_ingest)
            if result.get("auto_sleep")
        ]
        if auto_sleep_results:
            summary = "\n\n".join(
                complete_sleep_result(engine, gemini, llm_config, sleep_result)
                for sleep_result in auto_sleep_results
            )
        else:
            summary = run_sleep(engine, gemini, llm_config, session_id)
        telegram.send_message(chat_id, f"Memory updated.\n\n{summary}")
    else:
        run_auto_sleep_results(engine, gemini, llm_config, user_ingest, assistant_ingest)


def run_sleep(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    session_id: str,
) -> str:
    sleep_result = json.loads(engine.sleep(session_id))
    return complete_sleep_result(engine, gemini, llm_config, sleep_result)


def complete_sleep_result(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    sleep_result: dict[str, Any],
) -> str:
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


def run_auto_sleep_results(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    *ingest_results: dict[str, Any],
) -> None:
    for ingest_result in ingest_results:
        sleep_result = ingest_result.get("auto_sleep")
        if not sleep_result:
            continue
        try:
            summary = complete_sleep_result(engine, gemini, llm_config, sleep_result)
            log_line(f"auto-sleep completed: {summary.replace(chr(10), ' | ')}")
        except Exception as err:
            log_exception("auto-sleep completion failed", err)


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


def context_package(
    engine: memory_engine.MemoryEngine,
    session_id: str,
    chat_id: int,
    text: str,
) -> dict[str, Any]:
    request = {
        "schema_version": "core_context_request.v1",
        "session_id": session_id,
        "domain_state": {
            "host": "telegram_gemini_bot",
            "telegram_chat_id": chat_id,
            "current_text": text,
        },
        "query_text": text,
        "recall_limit": 5,
        "session_recent_limit": RECENT_CONTEXT_LIMIT,
        "session_trace_event_limit": SESSION_TRACE_EVENT_LIMIT,
        "include_core": False,
    }
    return json.loads(engine.core_context_package(json.dumps(request, ensure_ascii=False)))


def read_saved_offset() -> int | None:
    if not OFFSET_PATH.exists():
        return None
    try:
        payload = json.loads(OFFSET_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as err:
        log_exception("failed to read saved Telegram offset", err)
        return None

    offset = payload.get("offset")
    if isinstance(offset, int):
        return offset

    log_line(f"ignored invalid Telegram offset payload: {payload}")
    return None


def save_offset(offset: int) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    payload = {
        "offset": offset,
        "updated_at": now_rfc3339(),
    }
    OFFSET_PATH.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def notify_update_error(update: dict[str, Any], telegram: TelegramClient, err: Exception) -> None:
    message = update.get("message") or {}
    chat = message.get("chat") or {}
    chat_id = chat.get("id")
    if chat_id is None:
        return

    try:
        telegram.send_message(
            chat_id,
            "Bot error while processing this message. "
            f"Check runtime log: {LOG_PATH}\n\n{type(err).__name__}: {err}",
        )
    except Exception as notify_err:
        log_exception("failed to send error notification to Telegram", notify_err)


def ingest_chat_event(
    engine: memory_engine.MemoryEngine,
    session_id: str,
    event_type: str,
    source: str,
    text: str,
    tags: list[str],
    importance: str,
    payload_extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
    payload = {"text": text}
    if payload_extra:
        payload.update(payload_extra)

    return json.loads(
        engine.ingest(
            json.dumps(
                {
                    "schema_version": "event.v1",
                    "type": event_type,
                    "source": source,
                    "timestamp": now_rfc3339(),
                    "session_id": session_id,
                    "payload": payload,
                    "tags": tags,
                    "theme": "telegram_conversation",
                    "importance_hint": importance,
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
    if os.environ.get("MEMORY_BOT_NONINTERACTIVE") == "1":
        return HostLlmConfig(
            reasoning=ModelSelection(
                "google", os.environ.get("GEMINI_REASONING_MODEL", DEFAULT_REASONING_MODEL)
            ),
            balanced=ModelSelection(
                "google", os.environ.get("GEMINI_BALANCED_MODEL", DEFAULT_BALANCED_MODEL)
            ),
            fast=ModelSelection("google", os.environ.get("GEMINI_FAST_MODEL", DEFAULT_FAST_MODEL)),
            chat_role=os.environ.get("MEMORY_BOT_CHAT_ROLE", DEFAULT_CHAT_ROLE),
        )

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


def log_line(message: str) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    line = f"{now_rfc3339()} {message}"
    with LOG_PATH.open("a", encoding="utf-8") as file:
        file.write(line + "\n")
    print(line, flush=True)


def log_exception(message: str, err: Exception) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    line = f"{now_rfc3339()} {message}: {type(err).__name__}: {err}"
    details = "".join(traceback.format_exception(type(err), err, err.__traceback__))
    with LOG_PATH.open("a", encoding="utf-8") as file:
        file.write(line + "\n")
        file.write(details)
        if not details.endswith("\n"):
            file.write("\n")
    print(line, file=sys.stderr, flush=True)


def chat_system_instruction() -> str:
    return (
        "You are a concise Telegram assistant. Use the Memory Engine context package as the "
        "source of truth for session_recent, session_trace, archive_relevant, core_facts, and "
        "domain_state. Use session_trace for questions about what has been discussed in this "
        "chat, session_recent for short-term follow-ups and pronouns, archive_relevant for older "
        "committed memories, and core_facts for stable facts. If context is empty or irrelevant, "
        "answer normally. Do not claim you remember things unless they are present in the context "
        "package or the current user message."
    )


def chat_prompt(package: dict[str, Any], user_text: str) -> str:
    return (
        "Memory Engine context package JSON:\n"
        f"{json.dumps(package, ensure_ascii=False, indent=2)}\n\n"
        f"Current user message:\n{user_text}"
    )


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


def truncate_chars(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    return text[:limit].rstrip() + "..."


if __name__ == "__main__":
    main()
