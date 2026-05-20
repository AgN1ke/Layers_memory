"""Simple Telegram + Gemini host for Memory Engine.

This host is deliberately outside the Rust core. It owns Telegram polling,
Gemini API calls, API keys, and model selection.
"""

from __future__ import annotations

import getpass
import hashlib
import json
import os
import re
import sys
import threading
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
DEFAULT_AUTO_SLEEP_AFTER_EVENTS = 50
RECENT_CONTEXT_LIMIT = 40
SESSION_TRACE_EVENT_LIMIT = 120

TELEGRAM_API = "https://api.telegram.org"
GEMINI_API = "https://generativelanguage.googleapis.com/v1beta"
CHAT_SYSTEM_PROMPT_PATH = PROMPTS_DIR / "telegram_chat_system.md"
MEMORY_KEYWORD_PARTS = (
    "запам",
    "пам'ят",
    "памʼят",
    "важлив",
)
MEMORY_UPDATE_KEYWORD_PARTS = (
    "запам",
    "збережи в пам",
    "запиши в пам",
    "це важлив",
    "це дуже важлив",
    "важлива інформація",
    "онови пам",
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


class AutoSleepRunner:
    def __init__(self, gemini: "GeminiClient", llm_config: HostLlmConfig) -> None:
        self._gemini = gemini
        self._llm_config = llm_config
        self._lock = threading.Lock()
        self._running_task_ids: set[str] = set()

    def submit(
        self,
        sleep_result: dict[str, Any],
        telegram: "TelegramClient | None" = None,
        chat_id: int | None = None,
        reason: str = "auto-sleep",
    ) -> bool:
        task_id = sleep_result.get("pending_task", {}).get("task_id")
        if not task_id:
            log_line(f"{reason} ignored: missing task_id")
            return False

        with self._lock:
            if task_id in self._running_task_ids:
                log_line(f"{reason} already running task={task_id}")
                return False
            self._running_task_ids.add(task_id)

        thread = threading.Thread(
            target=self._run,
            args=(task_id, sleep_result, telegram, chat_id, reason),
            name=f"sleep-{task_id}",
            daemon=True,
        )
        thread.start()
        log_line(f"{reason} queued task={task_id}")
        return True

    def _run(
        self,
        task_id: str,
        sleep_result: dict[str, Any],
        telegram: "TelegramClient | None",
        chat_id: int | None,
        reason: str,
    ) -> None:
        try:
            engine = memory_engine.MemoryEngine(
                str(MEMORY_DIR),
                host_id="telegram_gemini_bot",
                auto_sleep_after_events=0,
            )
            summary = complete_sleep_result(engine, self._gemini, self._llm_config, sleep_result)
            log_line(f"{reason} completed: {summary.replace(chr(10), ' | ')}")
            if telegram is not None and chat_id is not None:
                telegram.send_message(chat_id, f"Memory updated.\n\n{summary}")
        except Exception as err:
            log_exception(f"{reason} completion failed", err)
            if telegram is not None and chat_id is not None:
                telegram.send_message(
                    chat_id,
                    f"Memory update failed. Check runtime log: {LOG_PATH}\n\n"
                    f"{type(err).__name__}: {err}",
                )
        finally:
            with self._lock:
                self._running_task_ids.discard(task_id)


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

    def validate_key(self) -> None:
        request = urllib.request.Request(
            f"{GEMINI_API}/models",
            headers={"x-goog-api-key": self.api_key},
            method="GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as err:
            body = err.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"Gemini key validation failed: HTTP {err.code}: {body}") from err
        except urllib.error.URLError as err:
            raise RuntimeError(f"Gemini key validation failed: {err}") from err

        if not isinstance(payload.get("models"), list):
            raise RuntimeError(f"Gemini key validation returned unexpected payload: {payload}")

    def generate_text(
        self,
        model: str,
        system_instruction: str,
        prompt: str,
        response_mime_type: str | None = None,
    ) -> str:
        url_model = urllib.parse.quote(model, safe="")
        url = f"{GEMINI_API}/models/{url_model}:generateContent"
        payload = {
            "system_instruction": {"parts": [{"text": system_instruction}]},
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        }
        if response_mime_type:
            payload["generationConfig"] = {"responseMimeType": response_mime_type}
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
    auto_sleep_after_events = read_auto_sleep_after_events()

    MEMORY_DIR.mkdir(parents=True, exist_ok=True)
    engine = memory_engine.MemoryEngine(
        str(MEMORY_DIR),
        host_id="telegram_gemini_bot",
        auto_sleep_after_events=auto_sleep_after_events,
    )
    telegram = TelegramClient(telegram_token)
    gemini = GeminiClient(gemini_key)
    auto_sleep_runner = AutoSleepRunner(gemini, llm_config)

    log_line(f"telegram token fingerprint: {secret_fingerprint(telegram_token)}")
    log_line(f"gemini key fingerprint: {secret_fingerprint(gemini_key)}")
    gemini.validate_key()
    log_line("gemini key validation completed")

    telegram.delete_webhook()
    log_line("deleteWebhook completed")
    print("Bot is running. Open Telegram and write to your bot.")
    print(
        "Commands: /help, /sleep, /recall text, /core, /remember text, "
        "/core_update id text, /core_forget id, /tasks, /models"
    )
    offset = read_saved_offset()
    log_line(f"bot polling started offset={offset} auto_sleep_after_events={auto_sleep_after_events}")

    while True:
        try:
            updates = telegram.get_updates(offset)
            if updates:
                log_line(f"poll received {len(updates)} update(s), offset={offset}")
            for update in updates:
                update_id = update.get("update_id")
                try:
                    handle_update(update, telegram, gemini, engine, llm_config, auto_sleep_runner)
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
    auto_sleep_runner: AutoSleepRunner,
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

    if text == "/core":
        telegram.send_message(chat_id, format_core_facts(context_package(engine, session_id, chat_id, text)))
        return

    if text.startswith("/remember"):
        fact_text = text.removeprefix("/remember").strip()
        if not fact_text:
            telegram.send_message(chat_id, "Usage: /remember stable fact text")
            return
        result = upsert_core_fact(
            engine,
            category="profile",
            scope=core_scope(session_id),
            text=fact_text,
            confidence=0.9,
            tags=["manual", "telegram"],
        )
        telegram.send_message(chat_id, f"Core fact saved: {result['fact']['text']}")
        return

    if text.startswith("/core_forget"):
        fact_id = text.removeprefix("/core_forget").strip()
        if not fact_id:
            telegram.send_message(chat_id, "Usage: /core_forget core_fact_id")
            return
        result = patch_core_fact(
            engine=engine,
            scope=core_scope(session_id),
            core_fact_id=fact_id,
            status="deprecated",
        )
        telegram.send_message(chat_id, f"Core fact deprecated: {result['fact']['text']}")
        return

    if text.startswith("/core_update"):
        parts = text.split(maxsplit=2)
        if len(parts) < 3:
            telegram.send_message(chat_id, "Usage: /core_update core_fact_id new fact text")
            return
        result = patch_core_fact(
            engine=engine,
            scope=core_scope(session_id),
            core_fact_id=parts[1],
            text=parts[2],
            status="active",
        )
        telegram.send_message(chat_id, f"Core fact updated: {result['fact']['text']}")
        return

    if text == "/sleep":
        queue_sleep_update(
            engine=engine,
            auto_sleep_runner=auto_sleep_runner,
            telegram=telegram,
            chat_id=chat_id,
            session_id=session_id,
            reason="manual sleep",
        )
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
        tags=event_tags(text),
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
            queued = False
            for sleep_result in auto_sleep_results:
                queued = auto_sleep_runner.submit(
                    sleep_result,
                    telegram=telegram,
                    chat_id=chat_id,
                    reason="memory keyword sleep",
                ) or queued
            if queued:
                telegram.send_message(chat_id, "Memory update queued.")
        else:
            queue_sleep_update(
                engine=engine,
                auto_sleep_runner=auto_sleep_runner,
                telegram=telegram,
                chat_id=chat_id,
                session_id=session_id,
                reason="memory keyword sleep",
            )
    else:
        run_auto_sleep_results(auto_sleep_runner, user_ingest, assistant_ingest)


def queue_sleep_update(
    engine: memory_engine.MemoryEngine,
    auto_sleep_runner: AutoSleepRunner,
    telegram: TelegramClient,
    chat_id: int,
    session_id: str,
    reason: str,
) -> None:
    sleep_result = json.loads(engine.sleep(session_id))
    queued = auto_sleep_runner.submit(
        sleep_result,
        telegram=telegram,
        chat_id=chat_id,
        reason=reason,
    )
    if queued:
        telegram.send_message(chat_id, "Memory update queued.")


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
        f"Emotional markers: {len(updated.get('emotional_markers', []))}\n"
        f"Personal signals: {len(updated.get('personal_signals', []))}\n"
        f"Gist: {updated['gist']}"
    )


def run_auto_sleep_results(
    auto_sleep_runner: AutoSleepRunner,
    *ingest_results: dict[str, Any],
) -> None:
    for ingest_result in ingest_results:
        sleep_result = ingest_result.get("auto_sleep")
        if not sleep_result:
            continue
        auto_sleep_runner.submit(sleep_result)


def execute_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    sleep_mode = os.environ.get("MEMORY_BOT_SLEEP_MODE", "multi_pass").strip().lower()
    if sleep_mode == "single":
        return execute_single_pass_sleep_compression(task, gemini, llm_config)
    return execute_multi_pass_sleep_compression(task, gemini, llm_config)


def execute_single_pass_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    parsed = execute_prompt_json(
        prompt_id=task["prompt_id"],
        prompt_input=task["inputs"],
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
    )
    normalize_sleep_compression_result(parsed, task)
    return parsed


def execute_multi_pass_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    sleep_input = task["inputs"]
    pass_input = {"sleep_task": sleep_input}
    emotional = execute_prompt_json(
        "sleep_emotional_pass", pass_input, task["role_hint"], gemini, llm_config
    )
    topic_thread = execute_prompt_json(
        "sleep_topic_thread_pass", pass_input, task["role_hint"], gemini, llm_config
    )
    personal_signals = execute_prompt_json(
        "sleep_personal_signal_pass", pass_input, task["role_hint"], gemini, llm_config
    )
    relational = execute_prompt_json(
        "sleep_relational_pass", pass_input, task["role_hint"], gemini, llm_config
    )

    consolidated = execute_prompt_json(
        "sleep_consolidator",
        {
            "sleep_task": sleep_input,
            "emotional_pass": emotional,
            "topic_thread_pass": topic_thread,
            "personal_signal_pass": personal_signals,
            "relational_pass": relational,
        },
        task["role_hint"],
        gemini,
        llm_config,
    )

    normalize_sleep_compression_result(consolidated, task)
    if not consolidated["emotional_markers"]:
        consolidated["emotional_markers"] = emotional.get("emotional_markers", [])
    if not consolidated["topic_thread"]:
        consolidated["topic_thread"] = topic_thread.get("topic_thread", [])
    if not consolidated["personal_signals"]:
        consolidated["personal_signals"] = personal_signals.get("personal_signals", [])
    if consolidated.get("relational_tone") is None:
        consolidated["relational_tone"] = relational.get("relational_tone")
    normalize_sleep_compression_result(consolidated, task)
    return consolidated


def execute_prompt_json(
    prompt_id: str,
    prompt_input: dict[str, Any],
    role_hint: str,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    prompt_path = PROMPTS_DIR / f"{prompt_id}.md"
    prompt_text = prompt_path.read_text(encoding="utf-8")
    selection = llm_config.for_role(role_hint)
    raw = gemini.generate_text(
        model=selection.model,
        system_instruction=prompt_text,
        prompt=json.dumps(prompt_input, ensure_ascii=False, indent=2),
        response_mime_type="application/json",
    )
    return parse_json_object(raw)


def normalize_sleep_compression_result(parsed: dict[str, Any], task: dict[str, Any]) -> None:
    if parsed.get("archive_id") != task["inputs"]["preliminary_archive_id"]:
        parsed["archive_id"] = task["inputs"]["preliminary_archive_id"]
    parsed.setdefault("schema_version", task["expected_output_schema"])
    parsed["facts"] = normalize_weighted_facts(parsed.get("facts"))
    parsed["quotes"] = normalize_quotes(parsed.get("quotes"))
    parsed["tags"] = normalize_string_list(parsed.get("tags"))
    parsed["links"] = normalize_links(parsed.get("links"))
    parsed["emotional_markers"] = normalize_emotional_markers(parsed.get("emotional_markers"))
    parsed["topic_thread"] = normalize_topic_thread(parsed.get("topic_thread"))
    parsed["personal_signals"] = normalize_personal_signals(parsed.get("personal_signals"))
    parsed["relational_tone"] = normalize_relational_tone(parsed.get("relational_tone"))


def normalize_weighted_facts(value: Any) -> list[dict[str, Any]]:
    items = []
    for item in (value if isinstance(value, list) else []):
        if not isinstance(item, dict):
            continue
        text = clean_string(item.get("text"))
        if not text:
            continue
        items.append(
            {
                "text": text,
                "confidence": clamp_float(item.get("confidence"), 0.5),
                "source_event_ids": normalize_string_list(item.get("source_event_ids")),
            }
        )
    return items


def normalize_quotes(value: Any) -> list[dict[str, Any]]:
    items = []
    for item in (value if isinstance(value, list) else []):
        if not isinstance(item, dict):
            continue
        text = clean_string(item.get("text"))
        if not text:
            continue
        quote: dict[str, Any] = {"text": text}
        source_event_id = clean_string(item.get("source_event_id"))
        if source_event_id:
            quote["source_event_id"] = source_event_id
        items.append(quote)
    return items


def normalize_links(value: Any) -> list[dict[str, Any]]:
    items = []
    for item in (value if isinstance(value, list) else []):
        if not isinstance(item, dict):
            continue
        kind = clean_string(item.get("kind"))
        target = clean_string(item.get("target"))
        if not kind or not target:
            continue
        link: dict[str, Any] = {"kind": kind, "target": target}
        note = clean_string(item.get("note"))
        if note:
            link["note"] = note
        items.append(link)
    return items


def normalize_emotional_markers(value: Any) -> list[dict[str, Any]]:
    items = []
    for item in (value if isinstance(value, list) else []):
        if not isinstance(item, dict):
            continue
        target = clean_string(item.get("target"))
        affect = clean_string(item.get("affect"))
        if not target or not affect:
            continue
        marker: dict[str, Any] = {
            "target": target,
            "affect": affect,
            "strength": clamp_float(item.get("strength"), 0.5),
            "source_event_ids": normalize_string_list(item.get("source_event_ids")),
        }
        quote = clean_string(item.get("quote"))
        evidence = clean_string(item.get("evidence"))
        if quote:
            marker["quote"] = quote
        if evidence:
            marker["evidence"] = evidence
        items.append(marker)
    return items


def normalize_topic_thread(value: Any) -> list[dict[str, Any]]:
    items = []
    for item in (value if isinstance(value, list) else []):
        if not isinstance(item, dict):
            continue
        topic = clean_string(item.get("topic"))
        if not topic:
            continue
        thread_item: dict[str, Any] = {
            "topic": topic,
            "subtopics": normalize_string_list(item.get("subtopics")),
            "source_event_ids": normalize_string_list(item.get("source_event_ids")),
        }
        energy = clean_string(item.get("energy"))
        summary = clean_string(item.get("summary"))
        if energy:
            thread_item["energy"] = energy
        if summary:
            thread_item["summary"] = summary
        items.append(thread_item)
    return items


def normalize_personal_signals(value: Any) -> list[dict[str, Any]]:
    items = []
    for item in (value if isinstance(value, list) else []):
        if not isinstance(item, dict):
            continue
        text = clean_string(item.get("text"))
        category = clean_string(item.get("category"))
        if not text or not category:
            continue
        signal: dict[str, Any] = {
            "text": text,
            "category": category,
            "confidence": clamp_float(item.get("confidence"), 0.5),
            "source_event_ids": normalize_string_list(item.get("source_event_ids")),
        }
        evidence = clean_string(item.get("evidence"))
        if evidence:
            signal["evidence"] = evidence
        items.append(signal)
    return items


def normalize_relational_tone(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    tone: dict[str, Any] = {
        "source_event_ids": normalize_string_list(value.get("source_event_ids")),
    }
    for key in (
        "warmth",
        "intellectual_engagement",
        "intimacy",
        "trust",
        "playfulness",
        "tension",
    ):
        if value.get(key) is not None:
            tone[key] = clamp_float(value.get(key), 0.0)
    summary = clean_string(value.get("summary"))
    if summary:
        tone["summary"] = summary
    return tone


def normalize_string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [cleaned for item in value if (cleaned := clean_string(item))]


def clean_string(value: Any) -> str:
    if not isinstance(value, str):
        return ""
    return value.strip()


def clamp_float(value: Any, default: float) -> float:
    if isinstance(value, bool):
        return default
    try:
        number = float(value)
    except (TypeError, ValueError):
        return default
    return max(0.0, min(1.0, number))


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


def upsert_core_fact(
    engine: memory_engine.MemoryEngine,
    category: str,
    scope: str,
    text: str,
    confidence: float,
    tags: list[str],
) -> dict[str, Any]:
    return json.loads(
        engine.upsert_core_fact(
            json.dumps(
                {
                    "schema_version": "core_fact_input.v1",
                    "category": category,
                    "scope": scope,
                    "text": text,
                    "confidence": confidence,
                    "tags": tags,
                },
                ensure_ascii=False,
            )
        )
    )


def patch_core_fact(
    engine: memory_engine.MemoryEngine,
    scope: str,
    core_fact_id: str,
    text: str | None = None,
    status: str | None = None,
    confidence: float | None = None,
    tags: list[str] | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema_version": "core_fact_patch_input.v1",
        "core_fact_id": core_fact_id,
        "scope": scope,
    }
    if text is not None:
        payload["text"] = text
    if status is not None:
        payload["status"] = status
    if confidence is not None:
        payload["confidence"] = confidence
    if tags is not None:
        payload["tags"] = tags

    return json.loads(engine.patch_core_fact(json.dumps(payload, ensure_ascii=False)))


def core_scope(session_id: str) -> str:
    return session_id


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
        "core_scope": core_scope(session_id),
        "query_text": text,
        "recall_limit": 5,
        "session_recent_limit": RECENT_CONTEXT_LIMIT,
        "session_trace_event_limit": SESSION_TRACE_EVENT_LIMIT,
        "include_core": True,
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
    if value and value.strip():
        print(f"{label}: using {env_name} from environment")
        return value.strip()
    while True:
        value = getpass.getpass(f"{label}: ").strip()
        if value:
            return value
        print("Value must not be empty.")


def secret_fingerprint(value: str) -> str:
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()[:12]
    return f"len={len(value)} sha256_12={digest}"


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


def read_auto_sleep_after_events() -> int:
    raw = os.environ.get("MEMORY_BOT_AUTO_SLEEP_AFTER_EVENTS")
    if not raw:
        return DEFAULT_AUTO_SLEEP_AFTER_EVENTS
    try:
        value = int(raw)
    except ValueError:
        print(
            f"Invalid MEMORY_BOT_AUTO_SLEEP_AFTER_EVENTS={raw!r}; "
            f"using {DEFAULT_AUTO_SLEEP_AFTER_EVENTS}."
        )
        return DEFAULT_AUTO_SLEEP_AFTER_EVENTS
    return max(0, value)


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
    return CHAT_SYSTEM_PROMPT_PATH.read_text(encoding="utf-8").strip()


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


def format_core_facts(package: dict[str, Any]) -> str:
    facts = package.get("core_facts", [])
    if not facts:
        return "No Core facts saved yet. Use /remember text for explicit stable facts."

    lines = ["Core facts:"]
    for index, fact in enumerate(facts, start=1):
        category = fact.get("category", "core")
        confidence = float(fact.get("confidence", 0.0))
        fact_id = fact.get("core_fact_id", "")
        lines.append(f"{index}. {fact_id} [{category} {confidence:.2f}] {fact.get('text', '')}")
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
        "/core - show stable Core facts\n"
        "/remember text - save a stable Core fact manually\n"
        "/core_update id text - update a Core fact in this chat\n"
        "/core_forget id - deprecate a Core fact in this chat\n"
        "/tasks - show pending tasks\n"
        "/models - show model role mapping\n"
        "\n"
        "Plain text is stored as an event and answered by Gemini with memory context.\n"
        "Messages with explicit memory update wording like 'запам...' or 'це важливо' queue sleep."
    )


def importance_hint(text: str) -> str:
    lowered = text.lower()
    if should_auto_sleep(lowered):
        return "high"
    if has_core_signal(lowered):
        return "medium"
    return "normal"


def event_tags(text: str) -> list[str]:
    lowered = text.lower()
    tags = ["telegram_message"]

    if should_auto_sleep(lowered):
        tags.append("explicit_memory_request")
    elif has_memory_request(lowered):
        tags.append("memory_reference")
    if has_name_reference(lowered):
        tags.extend(["personal_fact_signal", "name_reference"])
    if has_age_reference(lowered):
        tags.extend(["personal_fact_signal", "age_reference"])
    if has_assistant_identity_reference(lowered):
        tags.append("assistant_identity_reference")
    if has_communication_style_reference(lowered):
        tags.extend(["preference_signal", "communication_style_signal"])

    return unique_preserve_order(tags)


def has_core_signal(lowered: str) -> bool:
    return any(
        (
            has_memory_request(lowered),
            has_name_reference(lowered),
            has_age_reference(lowered),
            has_assistant_identity_reference(lowered),
            has_communication_style_reference(lowered),
        )
    )


def should_auto_sleep(text: str) -> bool:
    lowered = text.lower()
    return any(keyword in lowered for keyword in MEMORY_UPDATE_KEYWORD_PARTS)


def has_memory_request(lowered: str) -> bool:
    return any(keyword in lowered for keyword in MEMORY_KEYWORD_PARTS)


def has_name_reference(lowered: str) -> bool:
    return "мене звати" in lowered or bool(re.search(r"^\s*я\s+[а-яіїєґa-z'ʼ-]{2,40}\s*[.!?]?\s*$", lowered))


def has_age_reference(lowered: str) -> bool:
    return bool(re.search(r"\bмені\s+\d{1,3}\s*(?:років|роки|року|рік)\b", lowered))


def has_assistant_identity_reference(lowered: str) -> bool:
    return "тебе звати" in lowered or "твоє ім'я" in lowered or "твоє імʼя" in lowered


def has_communication_style_reference(lowered: str) -> bool:
    return "давай на ти" in lowered or "звертайся на ти" in lowered


def unique_preserve_order(values: list[str]) -> list[str]:
    seen = set()
    unique = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        unique.append(value)
    return unique


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
    try:
        main()
    except Exception as err:
        log_exception("bot startup failed", err)
        print(f"[fatal] {type(err).__name__}: {err}", file=sys.stderr)
        if os.environ.get("MEMORY_BOT_KEEP_CONSOLE_OPEN") == "1":
            try:
                input("Bot stopped. Press Enter to close this window...")
            except EOFError:
                pass
        raise
