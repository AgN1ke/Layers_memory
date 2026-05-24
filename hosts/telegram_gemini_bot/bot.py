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
from html import escape as xml_escape
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
ARCHIVE_DIR = MEMORY_DIR / "archive"
LOG_DIR = RUNTIME_DIR / "logs"
LOG_PATH = LOG_DIR / "bot.log"
TOKEN_USAGE_PATH = LOG_DIR / "token_usage.jsonl"
STATE_DIR = RUNTIME_DIR / "state"
OFFSET_PATH = STATE_DIR / "telegram_offset.json"
SLEEP_SCHEDULER_STATE_PATH = STATE_DIR / "sleep_scheduler_state.json"

DEFAULT_REASONING_MODEL = "gemini-2.5-pro"
DEFAULT_BALANCED_MODEL = "gemini-2.5-flash"
DEFAULT_FAST_MODEL = "gemini-2.5-flash-lite"
DEFAULT_CHAT_ROLE = "balanced"
DEFAULT_TOKEN_PRESSURE_RATIO = 0.80
DEFAULT_IDLE_SLEEP_HOUR = 4
DEFAULT_IDLE_SLEEP_MIN_SECONDS = 1800
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

ARCHIVE_LIST_LIMIT = 5
ARCHIVE_DETAIL_LIMIT = 10
CORE_SIGNAL_MIN_CONFIDENCE = 0.85
MAX_CORE_CATEGORY_LENGTH = 64
SLEEP_PASS_MAX_ATTEMPTS = 3
SLEEP_PASS_RETRY_DELAY_SECONDS = 1.0


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


@dataclass(frozen=True)
class GeminiTextResponse:
    text: str
    usage: dict[str, int | None]
    model: str
    operation: str


class GeminiApiError(RuntimeError):
    def __init__(self, model: str, message: str, status_code: int | None = None) -> None:
        super().__init__(message)
        self.model = model
        self.status_code = status_code


class GeminiNoCandidatesError(RuntimeError):
    def __init__(self, model: str, result: dict[str, Any]) -> None:
        self.model = model
        self.result = result
        self.block_reason = (
            result.get("promptFeedback", {}).get("blockReason")
            if isinstance(result.get("promptFeedback"), dict)
            else None
        )
        self.usage = gemini_usage_metadata(result)
        super().__init__(f"Gemini {model} returned no candidates: {result}")


class SleepRunner:
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
        reason: str = "background sleep",
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
            )
            summary = complete_sleep_result(engine, self._gemini, self._llm_config, sleep_result)
            log_line(f"{reason} completed: {summary.replace(chr(10), ' | ')}")
            if telegram is not None and chat_id is not None:
                telegram.send_message(chat_id, f"Memory updated.\n\n{summary}")
        except Exception as err:
            log_exception(f"{reason} completion failed", err)
            if telegram is not None and chat_id is not None:
                telegram.send_message(chat_id, "Memory update failed.\n\n" + friendly_error_message(err))
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
        operation: str = "generate_text",
        model_role: str | None = None,
        telemetry: dict[str, Any] | None = None,
    ) -> GeminiTextResponse:
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
            raise GeminiApiError(
                model=model,
                status_code=err.code,
                message=f"Gemini {model} failed: HTTP {err.code}: {body}",
            ) from err
        except urllib.error.URLError as err:
            raise GeminiApiError(model=model, message=f"Gemini {model} failed: {err}") from err

        usage = gemini_usage_metadata(result)
        try:
            text = extract_gemini_text(result, model)
        except GeminiNoCandidatesError:
            log_token_usage(
                operation=operation,
                model=model,
                model_role=model_role,
                usage=usage,
                prompt=prompt,
                output="",
                response_mime_type=response_mime_type,
                telemetry={**(telemetry or {}), "error": "no_candidates"},
            )
            raise
        log_token_usage(
            operation=operation,
            model=model,
            model_role=model_role,
            usage=usage,
            prompt=prompt,
            output=text,
            response_mime_type=response_mime_type,
            telemetry=telemetry,
        )
        return GeminiTextResponse(text=text, usage=usage, model=model, operation=operation)


def main() -> None:
    print("Telegram Gemini Memory Bot")
    print(f"Runtime memory: {MEMORY_DIR}")
    print(f"Runtime log: {LOG_PATH}")
    print(f"Local secrets cache: {STATE_DIR / 'secrets.local.json'} (gitignored)")
    print()
    log_line("starting Telegram Gemini Memory Bot")

    telegram_token = read_secret("Telegram bot token", "TELEGRAM_BOT_TOKEN")
    gemini_key = read_secret("Gemini API key", "GEMINI_API_KEY")
    llm_config = read_model_config()

    MEMORY_DIR.mkdir(parents=True, exist_ok=True)
    engine = memory_engine.MemoryEngine(
        str(MEMORY_DIR),
        host_id="telegram_gemini_bot",
    )
    telegram = TelegramClient(telegram_token)
    gemini = GeminiClient(gemini_key)
    sleep_runner = SleepRunner(gemini, llm_config)

    log_line(f"telegram token fingerprint: {secret_fingerprint(telegram_token)}")
    log_line(f"gemini key fingerprint: {secret_fingerprint(gemini_key)}")
    gemini.validate_key()
    log_line("gemini key validation completed")

    telegram.delete_webhook()
    log_line("deleteWebhook completed")
    print("Bot is running. Open Telegram and write to your bot.")
    print(
        "Commands: /help, /sleep, /archives, /archive_last, /recall text, "
        "/core, /core_seed, /remember text, /core_update id text, "
        "/core_forget id, /tasks, /models"
    )
    offset = read_saved_offset()
    log_line(
        "bot polling started "
        f"offset={offset} "
        f"token_pressure_ratio={read_token_pressure_ratio()} "
        f"idle_sleep_hour={read_idle_sleep_hour()} "
        f"idle_sleep_min_seconds={read_idle_sleep_min_seconds()}"
    )

    while True:
        try:
            updates = telegram.get_updates(offset)
            if updates:
                log_line(f"poll received {len(updates)} update(s), offset={offset}")
            for update in updates:
                update_id = update.get("update_id")
                try:
                    handle_update(update, telegram, gemini, engine, llm_config, sleep_runner)
                except Exception as err:
                    log_exception(f"update {update_id} failed", err)
                    notify_update_error(update, telegram, err)
                finally:
                    if update_id is not None:
                        offset = update_id + 1
                        save_offset(offset)
            maybe_queue_scheduled_idle_sleep(engine, sleep_runner)
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
    sleep_runner: SleepRunner,
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

    if text == "/core_seed":
        summary = promote_existing_archives(engine)
        telegram.send_message(chat_id, format_core_seed(summary))
        return

    if text == "/archives":
        telegram.send_message(chat_id, format_archives())
        return

    if text == "/archive_last":
        archive = last_complete_archive()
        telegram.send_message(chat_id, format_archive_detail(archive))
        return

    if text == "/archive":
        telegram.send_message(chat_id, "Usage: /archive_last or /archive archive_id")
        return

    if text.startswith("/archive "):
        archive_id = text.removeprefix("/archive").strip()
        telegram.send_message(chat_id, format_archive_detail(find_archive(archive_id)))
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
            sleep_runner=sleep_runner,
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

    if text.startswith("/"):
        telegram.send_message(chat_id, "Unknown command. Use /help for available commands.")
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
    prompt = chat_prompt(package, text)
    answer_response = gemini.generate_text(
        model=model,
        system_instruction=chat_system_instruction(),
        prompt=prompt,
        operation="chat_reply",
        model_role=llm_config.chat_role,
        telemetry=chat_prompt_telemetry(package, session_id, prompt),
    )
    answer = answer_response.text
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

    maybe_queue_token_pressure_sleep(
        engine=engine,
        sleep_runner=sleep_runner,
        session_id=session_id,
        package=package,
    )


def queue_sleep_update(
    engine: memory_engine.MemoryEngine,
    sleep_runner: SleepRunner,
    telegram: TelegramClient | None,
    chat_id: int | None,
    session_id: str,
    reason: str,
    notify: bool = True,
) -> None:
    if has_pending_sleep_task(engine, session_id):
        log_line(f"{reason} skipped: pending sleep task already exists for session={session_id}")
        return
    sleep_result = json.loads(engine.sleep(session_id))
    queued = sleep_runner.submit(
        sleep_result,
        telegram=telegram,
        chat_id=chat_id,
        reason=reason,
    )
    if queued and notify and telegram is not None and chat_id is not None:
        telegram.send_message(chat_id, "Memory update queued.")


def maybe_queue_token_pressure_sleep(
    engine: memory_engine.MemoryEngine,
    sleep_runner: SleepRunner,
    session_id: str,
    package: dict[str, Any],
) -> None:
    if has_pending_sleep_task(engine, session_id):
        return

    stats = session_unarchived_token_stats(session_id)
    if stats["event_count"] <= 0:
        return

    budget = package.get("budget") if isinstance(package.get("budget"), dict) else {}
    ratio = read_token_pressure_ratio()
    reasons = token_pressure_reasons(budget, stats, ratio)
    if not reasons:
        return

    log_line(
        "token_pressure_sleep_trigger "
        f"session={session_id} reasons={','.join(reasons)} "
        f"unarchived_events={stats['event_count']} "
        f"unarchived_est_tokens={stats['estimated_tokens']} "
        f"ratio={ratio:.2f}"
    )
    queue_sleep_update(
        engine=engine,
        sleep_runner=sleep_runner,
        telegram=None,
        chat_id=None,
        session_id=session_id,
        reason="token pressure sleep",
        notify=False,
    )


def token_pressure_reasons(
    budget: dict[str, Any],
    stats: dict[str, Any],
    ratio: float,
) -> list[str]:
    reasons: list[str] = []
    if budget.get("budget_exceeded"):
        reasons.append("budget_exceeded")

    if int_or_zero(budget.get("dropped_session_recent")) > 0:
        reasons.append("dropped_session_recent")
    if int_or_zero(budget.get("dropped_session_trace")) > 0:
        reasons.append("dropped_session_trace")

    total_budget = int_or_zero(budget.get("total_budget_tokens"))
    total_estimated = int_or_zero(budget.get("estimated_total_tokens"))
    if total_budget and total_estimated >= int(total_budget * ratio):
        reasons.append("total_budget_pressure")

    current_budget = int_or_zero(budget.get("current_memory_budget_tokens"))
    current_estimated = int_or_zero(budget.get("estimated_current_memory_tokens"))
    if current_budget and current_estimated >= int(current_budget * ratio):
        reasons.append("current_memory_pressure")
    if current_budget and int(stats["estimated_tokens"]) >= int(current_budget * ratio):
        reasons.append("unarchived_session_pressure")

    return unique_preserve_order(reasons)


def maybe_queue_scheduled_idle_sleep(
    engine: memory_engine.MemoryEngine,
    sleep_runner: SleepRunner,
) -> None:
    now_local = datetime.now().astimezone()
    if now_local.hour != read_idle_sleep_hour():
        return

    state = read_sleep_scheduler_state()
    today = now_local.date().isoformat()
    if state.get("last_idle_sleep_date") == today:
        return

    queued_any = False
    for session_id in session_ids():
        if has_pending_sleep_task(engine, session_id):
            continue
        stats = session_unarchived_token_stats(session_id)
        if stats["event_count"] <= 0:
            continue
        last_event_at = stats.get("last_event_at")
        if not isinstance(last_event_at, datetime):
            continue
        idle_seconds = (now_local - last_event_at.astimezone()).total_seconds()
        if idle_seconds < read_idle_sleep_min_seconds():
            continue

        log_line(
            "scheduled_idle_sleep_trigger "
            f"session={session_id} idle_seconds={int(idle_seconds)} "
            f"unarchived_events={stats['event_count']} "
            f"unarchived_est_tokens={stats['estimated_tokens']}"
        )
        queue_sleep_update(
            engine=engine,
            sleep_runner=sleep_runner,
            telegram=None,
            chat_id=None,
            session_id=session_id,
            reason="scheduled idle sleep",
            notify=False,
        )
        queued_any = True

    if queued_any:
        write_sleep_scheduler_state({"last_idle_sleep_date": today, "updated_at": now_rfc3339()})


def session_unarchived_token_stats(session_id: str) -> dict[str, Any]:
    events = unarchived_session_events(session_id)
    transcript = sleep_events_transcript(events)
    last_event_at = None
    for event in reversed(events):
        last_event_at = event_datetime(event)
        if last_event_at is not None:
            break
    return {
        "event_count": len(events),
        "estimated_tokens": estimate_tokens(transcript),
        "last_event_at": last_event_at,
    }


def unarchived_session_events(session_id: str) -> list[dict[str, Any]]:
    archived_ids = archived_event_ids_for_session(session_id)
    return [
        event
        for event in read_session_events(session_id)
        if clean_string(event.get("event_id")) not in archived_ids
    ]


def archived_event_ids_for_session(session_id: str) -> set[str]:
    event_ids: set[str] = set()
    for archive in complete_archives():
        if clean_string(archive.get("source_session_id")) != session_id:
            continue
        for event_id in normalize_string_list(archive.get("source_event_ids")):
            event_ids.add(event_id)
    return event_ids


def read_session_events(session_id: str) -> list[dict[str, Any]]:
    if not session_id or "/" in session_id or "\\" in session_id:
        return []
    events_path = MEMORY_DIR / "sessions" / session_id / "events.jsonl"
    if not events_path.exists():
        return []

    events: list[dict[str, Any]] = []
    try:
        with events_path.open("r", encoding="utf-8") as file:
            for line in file:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(event, dict):
                    events.append(event)
    except OSError as err:
        log_exception(f"failed to read session events {events_path}", err)
    return events


def session_ids() -> list[str]:
    sessions_dir = MEMORY_DIR / "sessions"
    if not sessions_dir.exists():
        return []
    return sorted(path.name for path in sessions_dir.iterdir() if path.is_dir())


def event_datetime(event: dict[str, Any]) -> datetime | None:
    for key in ("received_at", "timestamp"):
        parsed = parse_rfc3339_datetime(clean_string(event.get(key)))
        if parsed is not None:
            return parsed
    return None


def parse_rfc3339_datetime(value: str) -> datetime | None:
    if not value:
        return None
    try:
        normalized = value.replace("Z", "+00:00")
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=timezone.utc)
    return parsed


def has_pending_sleep_task(engine: memory_engine.MemoryEngine, session_id: str) -> bool:
    try:
        tasks = json.loads(engine.pending_tasks())
    except Exception as err:
        log_exception("failed to inspect pending sleep tasks", err)
        return True

    for task in tasks if isinstance(tasks, list) else []:
        if not isinstance(task, dict):
            continue
        if clean_string(task.get("task_type")) != "sleep_compression":
            continue
        inputs = task.get("inputs") if isinstance(task.get("inputs"), dict) else {}
        if clean_string(inputs.get("session_id")) == session_id:
            return True
    return False


def read_sleep_scheduler_state() -> dict[str, Any]:
    if not SLEEP_SCHEDULER_STATE_PATH.exists():
        return {}
    try:
        payload = json.loads(SLEEP_SCHEDULER_STATE_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as err:
        log_exception("failed to read sleep scheduler state", err)
        return {}
    return payload if isinstance(payload, dict) else {}


def write_sleep_scheduler_state(payload: dict[str, Any]) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    SLEEP_SCHEDULER_STATE_PATH.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def int_or_zero(value: Any) -> int:
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return max(0, value)
    try:
        return max(0, int(value))
    except (TypeError, ValueError):
        return 0


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
    memory_unit_task = sleep_result.get("memory_unit_task")
    compact_task = sleep_result.get("compact_memory_task")
    llm_result = execute_sleep_compression(
        task,
        gemini,
        llm_config,
        memory_unit_task=memory_unit_task,
        compact_task=compact_task,
    )
    updated = json.loads(
        engine.resume_sleep_compression(task["task_id"], json.dumps(llm_result, ensure_ascii=False))
    )
    memory_units = normalize_memory_units(llm_result.get("memory_units"))
    compact_memory = clean_string(llm_result.get("compact_memory"))
    if isinstance(memory_unit_task, dict) and memory_units:
        memory_unit_result = {
            "schema_version": memory_unit_task.get("expected_output_schema")
            or "memory_units_result.v1",
            "archive_id": archive["archive_id"],
            "memory_units": memory_units,
        }
        updated = json.loads(
            engine.resume_memory_unit_pass(
                memory_unit_task["task_id"],
                json.dumps(memory_unit_result, ensure_ascii=False),
            )
        )
    elif isinstance(compact_task, dict) and compact_memory:
        updated = json.loads(engine.resume_compact_memory_pass(compact_task["task_id"], compact_memory))
    log_sleep_compression_metrics(task, llm_result)
    core_summary = promote_archive_personal_signals(engine, updated)
    compact_tokens = estimate_tokens(clean_string(updated.get("compact_memory")))
    return (
        f"Archive: {archive['archive_id']}\n"
        f"Task: {task['task_id']}\n"
        f"Model role: {task['role_hint']}\n"
        f"Model: {llm_config.for_role(task['role_hint']).model}\n"
        f"Compact memory tokens: {compact_tokens}\n"
        f"Emotional markers: {len(updated.get('emotional_markers', []))}\n"
        f"Personal signals: {len(updated.get('personal_signals', []))}\n"
        f"Core signals: {format_core_signal_counts(core_summary)}\n"
        f"Gist: {updated['gist']}"
    )


def promote_archive_personal_signals(
    engine: memory_engine.MemoryEngine,
    archive: dict[str, Any],
) -> dict[str, int]:
    summary = {"created": 0, "updated": 0, "skipped": 0}
    if archive.get("status") != "complete":
        return summary

    archive_id = clean_string(archive.get("archive_id"))
    session_id = clean_string(archive.get("source_session_id"))
    scope = core_scope(session_id) if session_id else ""
    user_event_ids = user_event_ids_for_session(session_id)
    personal_signals = archive.get("personal_signals", [])
    for signal in personal_signals if isinstance(personal_signals, list) else []:
        if not isinstance(signal, dict):
            summary["skipped"] += 1
            continue

        text = clean_string(signal.get("text"))
        source_category = normalize_category(signal.get("category"))
        core_category = source_category
        confidence = clamp_float(signal.get("confidence"), 0.0)
        source_event_ids = normalize_string_list(signal.get("source_event_ids"))
        has_user_source = bool(user_event_ids) and any(
            event_id in user_event_ids for event_id in source_event_ids
        )
        if (
            not text
            or not core_category
            or confidence < CORE_SIGNAL_MIN_CONFIDENCE
            or not has_user_source
            or is_near_duplicate_core_fact(core_category, scope, text)
        ):
            summary["skipped"] += 1
            continue

        result = upsert_core_fact(
            engine=engine,
            category=core_category,
            scope=scope,
            text=text,
            confidence=confidence,
            tags=[
                "archive_signal",
                "telegram",
                f"signal_category:{source_category}",
            ],
            source_archive_ids=[archive_id] if archive_id else [],
        )
        if result.get("created"):
            summary["created"] += 1
        else:
            summary["updated"] += 1

    return summary


def promote_existing_archives(engine: memory_engine.MemoryEngine) -> dict[str, int]:
    summary = {"archives": 0, "created": 0, "updated": 0, "skipped": 0}
    for archive in complete_archives():
        summary["archives"] += 1
        archive_summary = promote_archive_personal_signals(engine, archive)
        for key in ("created", "updated", "skipped"):
            summary[key] += archive_summary[key]
    return summary


def format_core_signal_counts(summary: dict[str, int]) -> str:
    return (
        f"{summary.get('created', 0)} new, "
        f"{summary.get('updated', 0)} updated, "
        f"{summary.get('skipped', 0)} skipped"
    )


def is_near_duplicate_core_fact(category: str, scope: str, text: str) -> bool:
    for existing_text in active_core_texts(category, scope):
        if normalized_text(existing_text) == normalized_text(text):
            return False
        if token_overlap(existing_text, text) >= 0.55:
            return True
    return False


def active_core_texts(category: str, scope: str) -> list[str]:
    path = MEMORY_DIR / "core" / "store" / f"{category}.json"
    if not path.exists():
        return []
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return []

    texts = []
    facts = payload.get("facts", []) if isinstance(payload, dict) else []
    for fact in facts if isinstance(facts, list) else []:
        if not isinstance(fact, dict):
            continue
        if fact.get("status") != "active":
            continue
        if clean_string(fact.get("scope")) != scope:
            continue
        text = clean_string(fact.get("text"))
        if text:
            texts.append(text)
    return texts


def normalized_text(text: str) -> str:
    return re.sub(r"\s+", " ", text.strip().lower())


def token_overlap(left: str, right: str) -> float:
    left_tokens = meaningful_tokens(left)
    right_tokens = meaningful_tokens(right)
    if not left_tokens or not right_tokens:
        return 0.0
    return len(left_tokens & right_tokens) / min(len(left_tokens), len(right_tokens))


def meaningful_tokens(text: str) -> set[str]:
    return {
        token
        for token in re.findall(r"[0-9A-Za-zА-Яа-яІіЇїЄєҐґ_-]{4,}", text.lower())
        if token
        not in {
            "user",
            "users",
            "the",
            "and",
            "with",
            "that",
            "this",
            "has",
            "have",
            "interest",
            "interested",
            "strong",
            "specifically",
            "користувач",
            "користувача",
            "користувачу",
            "дуже",
            "любить",
            "цікавиться",
        }
    }


def execute_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    memory_unit_task: dict[str, Any] | None = None,
    compact_task: dict[str, Any] | None = None,
) -> dict[str, Any]:
    sleep_mode = os.environ.get("MEMORY_BOT_SLEEP_MODE", "multi_pass").strip().lower()
    if sleep_mode == "single":
        return execute_single_pass_sleep_compression(
            task,
            gemini,
            llm_config,
            memory_unit_task=memory_unit_task,
            compact_task=compact_task,
        )
    return execute_multi_pass_sleep_compression(
        task,
        gemini,
        llm_config,
        memory_unit_task=memory_unit_task,
        compact_task=compact_task,
    )


def execute_single_pass_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    memory_unit_task: dict[str, Any] | None = None,
    compact_task: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if isinstance(memory_unit_task, dict):
        memory_units_result = execute_memory_unit_pass(memory_unit_task, gemini, llm_config)
    else:
        memory_units_result = empty_memory_unit_result(task)
    memory_units = normalize_memory_units(memory_units_result.get("memory_units"))
    compact_memory = memory_units_to_compact_memory(memory_units)
    if not compact_memory and compact_task:
        compact_memory = execute_compact_memory_pass(compact_task, gemini, llm_config)
    parsed = execute_prompt_json(
        prompt_id=task["prompt_id"],
        prompt_input=task["inputs"],
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
    )
    normalize_sleep_compression_result(parsed, task)
    parsed["compact_memory"] = compact_memory
    parsed["memory_units"] = memory_units
    return parsed


def execute_multi_pass_sleep_compression(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    memory_unit_task: dict[str, Any] | None = None,
    compact_task: dict[str, Any] | None = None,
) -> dict[str, Any]:
    sleep_input = task["inputs"]
    pass_input = {"sleep_task": sleep_input}
    failed_passes: list[str] = []
    if isinstance(memory_unit_task, dict):
        memory_units_result = safe_execute_memory_unit_pass(
            memory_unit_task, gemini, llm_config, failed_passes
        )
    else:
        memory_units_result = empty_memory_unit_result(task)
    memory_units = normalize_memory_units(memory_units_result.get("memory_units"))
    compact_memory = memory_units_to_compact_memory(memory_units)
    if not compact_memory and compact_task:
        compact_memory = safe_execute_compact_memory_pass(
            compact_task, gemini, llm_config, failed_passes
        )
    emotional = safe_execute_sleep_pass_json(
        prompt_id="sleep_emotional_pass",
        prompt_input=pass_input,
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
        fallback={"emotional_markers": []},
        failed_passes=failed_passes,
    )
    topic_thread = safe_execute_sleep_pass_json(
        prompt_id="sleep_topic_thread_pass",
        prompt_input=pass_input,
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
        fallback={"topic_thread": []},
        failed_passes=failed_passes,
    )
    personal_signals = safe_execute_sleep_pass_json(
        prompt_id="sleep_personal_signal_pass",
        prompt_input=pass_input,
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
        fallback={"personal_signals": []},
        failed_passes=failed_passes,
    )
    relational = safe_execute_sleep_pass_json(
        prompt_id="sleep_relational_pass",
        prompt_input=pass_input,
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
        fallback={"relational_tone": None},
        failed_passes=failed_passes,
    )

    consolidator_input = {
        "sleep_task": sleep_input,
        "emotional_pass": emotional,
        "topic_thread_pass": topic_thread,
        "personal_signal_pass": personal_signals,
        "relational_pass": relational,
    }
    try:
        consolidated = execute_prompt_json(
            "sleep_consolidator",
            consolidator_input,
            task["role_hint"],
            gemini,
            llm_config,
            retry_on_json_error=True,
        )
        completion_mode = "consolidated"
    except Exception as err:
        log_exception("sleep_consolidator failed; using track fallback", err)
        consolidated = fallback_sleep_compression_result(
            task=task,
            emotional=emotional,
            topic_thread=topic_thread,
            personal_signals=personal_signals,
            relational=relational,
            error=err,
        )
        completion_mode = "fallback_from_tracks"

    normalize_sleep_compression_result(consolidated, task)
    consolidated["compact_memory"] = compact_memory
    consolidated["memory_units"] = memory_units
    if not consolidated["emotional_markers"]:
        consolidated["emotional_markers"] = emotional.get("emotional_markers", [])
    if not consolidated["topic_thread"]:
        consolidated["topic_thread"] = topic_thread.get("topic_thread", [])
    if not consolidated["personal_signals"]:
        consolidated["personal_signals"] = personal_signals.get("personal_signals", [])
    if consolidated.get("relational_tone") is None:
        consolidated["relational_tone"] = relational.get("relational_tone")
    tags = [*consolidated.get("tags", []), f"completion_mode:{completion_mode}"]
    tags.extend(f"pass_failed:{prompt_id}" for prompt_id in failed_passes)
    consolidated["tags"] = unique_preserve_order(tags)
    normalize_sleep_compression_result(consolidated, task)
    return consolidated


def safe_execute_memory_unit_pass(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    failed_passes: list[str],
) -> dict[str, Any]:
    prompt_id = clean_string(task.get("prompt_id")) or "memory_unit_pass"
    fallback = {
        "schema_version": task.get("expected_output_schema") or "memory_units_result.v1",
        "archive_id": task["inputs"]["preliminary_archive_id"],
        "memory_units": [],
    }
    last_error: Exception | None = None
    for attempt in range(1, SLEEP_PASS_MAX_ATTEMPTS + 1):
        try:
            return execute_memory_unit_pass(task, gemini, llm_config)
        except Exception as err:
            last_error = err
            if attempt < SLEEP_PASS_MAX_ATTEMPTS:
                log_line(
                    f"{prompt_id} attempt={attempt}/{SLEEP_PASS_MAX_ATTEMPTS} failed; "
                    f"retrying: {type(err).__name__}: {err}"
                )
                time.sleep(SLEEP_PASS_RETRY_DELAY_SECONDS)
                continue
            failed_passes.append(prompt_id)
            log_exception(
                f"{prompt_id} failed after {SLEEP_PASS_MAX_ATTEMPTS} attempts; "
                "continuing without memory units",
                err,
            )
            return fallback

    failed_passes.append(prompt_id)
    if last_error is not None:
        log_exception(
            f"{prompt_id} failed after {SLEEP_PASS_MAX_ATTEMPTS} attempts; "
            "continuing without memory units",
            last_error,
        )
    return fallback


def empty_memory_unit_result(task: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": "memory_units_result.v1",
        "archive_id": task["inputs"]["preliminary_archive_id"],
        "memory_units": [],
    }


def safe_execute_compact_memory_pass(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    failed_passes: list[str],
) -> str:
    prompt_id = clean_string(task.get("prompt_id")) or "compact_memory_pass"
    last_error: Exception | None = None
    for attempt in range(1, SLEEP_PASS_MAX_ATTEMPTS + 1):
        try:
            return execute_compact_memory_pass(task, gemini, llm_config)
        except Exception as err:
            last_error = err
            if attempt < SLEEP_PASS_MAX_ATTEMPTS:
                log_line(
                    f"{prompt_id} attempt={attempt}/{SLEEP_PASS_MAX_ATTEMPTS} failed; "
                    f"retrying: {type(err).__name__}: {err}"
                )
                time.sleep(SLEEP_PASS_RETRY_DELAY_SECONDS)
                continue
            failed_passes.append(prompt_id)
            log_exception(
                f"{prompt_id} failed after {SLEEP_PASS_MAX_ATTEMPTS} attempts; "
                "continuing without compact memory",
                err,
            )
            return ""

    failed_passes.append(prompt_id)
    if last_error is not None:
        log_exception(
            f"{prompt_id} failed after {SLEEP_PASS_MAX_ATTEMPTS} attempts; "
            "continuing without compact memory",
            last_error,
        )
    return ""


def safe_execute_sleep_pass_json(
    prompt_id: str,
    prompt_input: dict[str, Any],
    role_hint: str,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    fallback: dict[str, Any],
    failed_passes: list[str],
) -> dict[str, Any]:
    last_error: Exception | None = None
    for attempt in range(1, SLEEP_PASS_MAX_ATTEMPTS + 1):
        try:
            return execute_prompt_json(
                prompt_id,
                prompt_input,
                role_hint,
                gemini,
                llm_config,
                retry_on_json_error=True,
            )
        except Exception as err:
            last_error = err
            if attempt < SLEEP_PASS_MAX_ATTEMPTS:
                log_line(
                    f"{prompt_id} attempt={attempt}/{SLEEP_PASS_MAX_ATTEMPTS} failed; "
                    f"retrying: {type(err).__name__}: {err}"
                )
                time.sleep(SLEEP_PASS_RETRY_DELAY_SECONDS)
                continue
            failed_passes.append(prompt_id)
            log_exception(
                f"{prompt_id} failed after {SLEEP_PASS_MAX_ATTEMPTS} attempts; "
                "using empty track fallback",
                err,
            )
            return fallback.copy()

    failed_passes.append(prompt_id)
    if last_error is not None:
        log_exception(
            f"{prompt_id} failed after {SLEEP_PASS_MAX_ATTEMPTS} attempts; "
            "using empty track fallback",
            last_error,
        )
    return fallback.copy()


def execute_memory_unit_pass(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    prompt_id = clean_string(task.get("prompt_id")) or "memory_unit_pass"
    parsed = execute_prompt_json(
        prompt_id=prompt_id,
        prompt_input={"sleep_task": task["inputs"]},
        role_hint=task["role_hint"],
        gemini=gemini,
        llm_config=llm_config,
        retry_on_json_error=True,
    )
    normalize_memory_unit_pass_result(parsed, task)
    return parsed


def execute_compact_memory_pass(
    task: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> str:
    prompt_id = clean_string(task.get("prompt_id")) or "compact_memory_pass"
    prompt_path = PROMPTS_DIR / f"{prompt_id}.md"
    prompt_text = prompt_path.read_text(encoding="utf-8")
    selection = llm_config.for_role(task["role_hint"])
    prompt_payload = json.dumps({"sleep_task": task["inputs"]}, ensure_ascii=False, indent=2)
    response = gemini.generate_text(
        model=selection.model,
        system_instruction=prompt_text,
        prompt=prompt_payload,
        operation="compact_memory_pass",
        model_role=task["role_hint"],
        telemetry={"prompt_id": prompt_id},
    )
    return normalize_compact_memory_text(response.text)


def fallback_sleep_compression_result(
    task: dict[str, Any],
    emotional: dict[str, Any],
    topic_thread: dict[str, Any],
    personal_signals: dict[str, Any],
    relational: dict[str, Any],
    error: Exception,
) -> dict[str, Any]:
    emotional_markers = normalize_emotional_markers(emotional.get("emotional_markers"))
    topic_items = normalize_topic_thread(topic_thread.get("topic_thread"))
    signals = normalize_personal_signals(personal_signals.get("personal_signals"))
    relational_tone = normalize_relational_tone(relational.get("relational_tone"))

    primary_signal = signals[0]["text"] if signals else ""
    primary_topic = topic_items[0]["topic"] if topic_items else ""
    primary_emotion = ""
    if emotional_markers:
        marker = emotional_markers[0]
        primary_emotion = f"{marker['target']} ({marker['affect']})"

    gist_parts = [part for part in (primary_signal, primary_topic, primary_emotion) if part]
    gist = "; ".join(gist_parts[:2]) or "Сесія збережена через fallback без фінального consolidator."

    topic_lines = [
        f"- {item['topic']}: {item.get('summary', '')}".strip()
        for item in topic_items[:8]
    ]
    signal_lines = [f"- {signal['text']}" for signal in signals[:10]]
    emotion_lines = [
        f"- {marker['target']}: {marker['affect']} ({marker['strength']:.2f})"
        for marker in emotional_markers[:10]
    ]
    narrative_sections = [
        "Consolidator не повернув валідний JSON, тому archive зібрано з успішних спеціалізованих проходів.",
        f"Причина fallback: {type(error).__name__}: {truncate_chars(str(error), 220)}",
    ]
    if topic_lines:
        narrative_sections.append("Теми:\n" + "\n".join(topic_lines))
    if signal_lines:
        narrative_sections.append("Особисті сигнали:\n" + "\n".join(signal_lines))
    if emotion_lines:
        narrative_sections.append("Емоційні маркери:\n" + "\n".join(emotion_lines))

    tags = ["multi_pass_sleep", "consolidator_fallback"]
    for item in topic_items[:5]:
        tags.append(normalize_category(item["topic"]))

    return {
        "schema_version": task["expected_output_schema"],
        "archive_id": task["inputs"]["preliminary_archive_id"],
        "gist": gist,
        "narrative": "\n\n".join(narrative_sections),
        "facts": [],
        "quotes": [],
        "tags": unique_preserve_order([tag for tag in tags if tag]),
        "theme": primary_topic or None,
        "weight": fallback_weight(emotional_markers, signals),
        "links": [],
        "compact_memory": None,
        "emotional_markers": emotional_markers,
        "topic_thread": topic_items,
        "personal_signals": signals,
        "relational_tone": relational_tone,
    }


def fallback_weight(emotional_markers: list[dict[str, Any]], signals: list[dict[str, Any]]) -> float:
    strongest_emotion = max(
        (clamp_float(marker.get("strength"), 0.0) for marker in emotional_markers),
        default=0.0,
    )
    strongest_signal = max(
        (clamp_float(signal.get("confidence"), 0.0) for signal in signals),
        default=0.0,
    )
    return max(0.55, min(1.0, max(strongest_emotion, strongest_signal)))


def execute_prompt_json(
    prompt_id: str,
    prompt_input: dict[str, Any],
    role_hint: str,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    retry_on_json_error: bool = False,
) -> dict[str, Any]:
    prompt_path = PROMPTS_DIR / f"{prompt_id}.md"
    prompt_text = prompt_path.read_text(encoding="utf-8")
    selection = llm_config.for_role(role_hint)
    prompt_payload = json.dumps(prompt_input, ensure_ascii=False, indent=2)
    response = gemini.generate_text(
        model=selection.model,
        system_instruction=prompt_text,
        prompt=prompt_payload,
        response_mime_type="application/json",
        operation=prompt_id,
        model_role=role_hint,
        telemetry={"prompt_id": prompt_id},
    )
    try:
        return parse_json_object(response.text)
    except (json.JSONDecodeError, ValueError) as err:
        if not retry_on_json_error:
            raise
        retry_prompt = (
            f"{prompt_payload}\n\n"
            "The previous response was not valid JSON.\n"
            f"JSON parser error: {err}\n"
            "Return ONLY one valid JSON object matching the requested schema. "
            "No prose. No markdown. No comments."
        )
        retry_response = gemini.generate_text(
            model=selection.model,
            system_instruction=prompt_text,
            prompt=retry_prompt,
            response_mime_type="application/json",
            operation=f"{prompt_id}_retry",
            model_role=role_hint,
            telemetry={"prompt_id": prompt_id, "retry_reason": "json_decode_error"},
        )
        return parse_json_object(retry_response.text)


def normalize_sleep_compression_result(parsed: dict[str, Any], task: dict[str, Any]) -> None:
    if parsed.get("archive_id") != task["inputs"]["preliminary_archive_id"]:
        parsed["archive_id"] = task["inputs"]["preliminary_archive_id"]
    parsed.setdefault("schema_version", task["expected_output_schema"])
    compact_memory = normalize_compact_memory_text(parsed.get("compact_memory"))
    parsed["compact_memory"] = compact_memory or None
    parsed["facts"] = normalize_weighted_facts(parsed.get("facts"))
    parsed["quotes"] = normalize_quotes(parsed.get("quotes"))
    parsed["tags"] = normalize_string_list(parsed.get("tags"))
    parsed["links"] = normalize_links(parsed.get("links"))
    parsed["emotional_markers"] = normalize_emotional_markers(parsed.get("emotional_markers"))
    parsed["topic_thread"] = normalize_topic_thread(parsed.get("topic_thread"))
    parsed["personal_signals"] = normalize_personal_signals(parsed.get("personal_signals"))
    parsed["relational_tone"] = normalize_relational_tone(parsed.get("relational_tone"))


def normalize_memory_unit_pass_result(parsed: dict[str, Any], task: dict[str, Any]) -> None:
    if parsed.get("archive_id") != task["inputs"]["preliminary_archive_id"]:
        parsed["archive_id"] = task["inputs"]["preliminary_archive_id"]
    parsed.setdefault("schema_version", task.get("expected_output_schema") or "memory_units_result.v1")
    parsed["memory_units"] = normalize_memory_units(parsed.get("memory_units"))


def normalize_memory_units(value: Any) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        return []
    units: list[dict[str, Any]] = []
    for raw in value:
        if not isinstance(raw, dict):
            continue
        thesis = clean_string(raw.get("thesis"))
        if not thesis:
            continue
        unit = {
            "thesis": thesis,
            "source_event_ids": normalize_string_list(raw.get("source_event_ids")),
            "tags": normalize_string_list(raw.get("tags")),
            "weight": clamp_float(raw.get("weight"), 0.55),
        }
        evidence = clean_string(raw.get("evidence"))
        if evidence:
            unit["evidence"] = truncate_text(evidence, 320)
        units.append(unit)
    return units


def memory_units_to_compact_memory(units: list[dict[str, Any]]) -> str:
    theses = []
    for unit in units:
        thesis = clean_string(unit.get("thesis"))
        if thesis:
            theses.append(thesis)
    return "\n".join(theses).strip()


def normalize_compact_memory_text(value: Any) -> str:
    text = clean_string(value)
    if not text:
        return ""
    text = re.sub(r"^```(?:text|markdown)?\s*", "", text, flags=re.IGNORECASE)
    text = re.sub(r"\s*```$", "", text)
    lines = []
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        line = re.sub(r"^\s*[-*]\s+", "", line)
        line = re.sub(r"^\s*\d+[.)]\s+", "", line)
        if line:
            lines.append(line)
    return "\n".join(lines).strip()


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
        category = normalize_category(item.get("category"))
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


def safe_list(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def clean_string(value: Any) -> str:
    if not isinstance(value, str):
        return ""
    return value.strip()


def normalize_category(value: Any) -> str:
    raw = clean_string(value).lower()
    category = re.sub(r"[^a-z0-9_]+", "_", raw)
    category = re.sub(r"_+", "_", category).strip("_")
    if not category:
        return "other"
    return category[:MAX_CORE_CATEGORY_LENGTH].strip("_") or "other"


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
    source_archive_ids: list[str] | None = None,
    source_candidate_id: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema_version": "core_fact_input.v1",
        "category": category,
        "scope": scope,
        "text": text,
        "confidence": confidence,
        "tags": tags,
    }
    if source_archive_ids:
        payload["source_archive_ids"] = source_archive_ids
    if source_candidate_id:
        payload["source_candidate_id"] = source_candidate_id

    return json.loads(
        engine.upsert_core_fact(
            json.dumps(payload, ensure_ascii=False)
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
    package = json.loads(engine.core_context_package(json.dumps(request, ensure_ascii=False)))
    log_context_budget(package, session_id)
    return package


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
        telegram.send_message(chat_id, friendly_error_message(err))
    except Exception as notify_err:
        log_exception("failed to send error notification to Telegram", notify_err)


def friendly_error_message(err: Exception) -> str:
    if isinstance(err, GeminiNoCandidatesError):
        if err.block_reason == "PROHIBITED_CONTENT":
            return (
                "Модель зараз заблокувала відповідь фільтром безпеки, тому я не змогла "
                "нормально відповісти на це повідомлення. Перефразуй, будь ласка, або "
                "давай зайдемо з іншого боку. Деталі я записала в лог."
            )
        return (
            "Модель не повернула текстову відповідь. Я записала деталі в лог, "
            "а ти можеш повторити або перефразувати повідомлення."
        )

    if isinstance(err, GeminiApiError):
        message = str(err).lower()
        if "invalid" in message and "api key" in message:
            return (
                "Gemini відхилив API key. Перезапусти бота і введи актуальний ключ. "
                "Повні технічні деталі записані в лог."
            )
        if err.status_code == 429 or "rate" in message or "quota" in message:
            return (
                "Gemini зараз вперся в ліміт або квоту. Зачекай трохи й спробуй ще раз. "
                "Деталі я записала в лог."
            )
        return "Gemini повернув технічну помилку. Я записала деталі в лог, спробуй ще раз."

    if "memory" in type(err).__name__.lower() or "memory_engine" in str(err):
        return "Є технічна заминка в пам'яті. Я записала деталі в лог, спробуй ще раз."

    return "Щось пішло не так під час обробки повідомлення. Я записала деталі в лог."


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


def read_token_pressure_ratio() -> float:
    raw = os.environ.get("MEMORY_BOT_TOKEN_PRESSURE_RATIO")
    if not raw:
        return DEFAULT_TOKEN_PRESSURE_RATIO
    try:
        value = float(raw)
    except ValueError:
        print(
            f"Invalid MEMORY_BOT_TOKEN_PRESSURE_RATIO={raw!r}; "
            f"using {DEFAULT_TOKEN_PRESSURE_RATIO}."
        )
        return DEFAULT_TOKEN_PRESSURE_RATIO
    return min(1.0, max(0.10, value))


def read_idle_sleep_hour() -> int:
    raw = os.environ.get("MEMORY_BOT_IDLE_SLEEP_HOUR")
    if not raw:
        return DEFAULT_IDLE_SLEEP_HOUR
    try:
        value = int(raw)
    except ValueError:
        print(
            f"Invalid MEMORY_BOT_IDLE_SLEEP_HOUR={raw!r}; "
            f"using {DEFAULT_IDLE_SLEEP_HOUR}."
        )
        return DEFAULT_IDLE_SLEEP_HOUR
    return min(23, max(0, value))


def read_idle_sleep_min_seconds() -> int:
    raw = os.environ.get("MEMORY_BOT_IDLE_SLEEP_MIN_SECONDS")
    if not raw:
        return DEFAULT_IDLE_SLEEP_MIN_SECONDS
    try:
        value = int(raw)
    except ValueError:
        print(
            f"Invalid MEMORY_BOT_IDLE_SLEEP_MIN_SECONDS={raw!r}; "
            f"using {DEFAULT_IDLE_SLEEP_MIN_SECONDS}."
        )
        return DEFAULT_IDLE_SLEEP_MIN_SECONDS
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


def estimate_tokens(text: str) -> int:
    # Same deterministic conservative estimator as Rust v0.1 budget report.
    return (len(text) + 1) // 2


def gemini_usage_metadata(result: dict[str, Any]) -> dict[str, int | None]:
    metadata = result.get("usageMetadata")
    if not isinstance(metadata, dict):
        return {
            "prompt_tokens": None,
            "output_tokens": None,
            "total_tokens": None,
            "thoughts_tokens": None,
        }
    return {
        "prompt_tokens": int(metadata["promptTokenCount"])
        if isinstance(metadata.get("promptTokenCount"), int)
        else None,
        "output_tokens": int(metadata["candidatesTokenCount"])
        if isinstance(metadata.get("candidatesTokenCount"), int)
        else None,
        "total_tokens": int(metadata["totalTokenCount"])
        if isinstance(metadata.get("totalTokenCount"), int)
        else None,
        "thoughts_tokens": int(metadata["thoughtsTokenCount"])
        if isinstance(metadata.get("thoughtsTokenCount"), int)
        else None,
    }


def log_token_usage(
    operation: str,
    model: str,
    model_role: str | None,
    usage: dict[str, int | None],
    prompt: str,
    output: str,
    response_mime_type: str | None,
    telemetry: dict[str, Any] | None = None,
) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    record: dict[str, Any] = {
        "timestamp": now_rfc3339(),
        "kind": "gemini_token_usage",
        "operation": operation,
        "model_role": model_role,
        "model": model,
        "response_mime_type": response_mime_type,
        "prompt_tokens": usage.get("prompt_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "total_tokens": usage.get("total_tokens"),
        "thoughts_tokens": usage.get("thoughts_tokens"),
        "estimated_prompt_tokens": estimate_tokens(prompt),
        "estimated_output_tokens": estimate_tokens(output),
        "prompt_chars": len(prompt),
        "output_chars": len(output),
    }
    if telemetry:
        record.update(telemetry)

    with TOKEN_USAGE_PATH.open("a", encoding="utf-8") as file:
        file.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")

    baseline = record.get("baseline_without_compression_estimated_tokens")
    savings = record.get("estimated_savings_vs_baseline_tokens")
    extra = ""
    if isinstance(baseline, int):
        extra += f" baseline_no_compression_est={baseline}"
    if isinstance(savings, int):
        extra += f" savings_est={savings}"
    log_line(
        "token_usage "
        f"operation={operation} role={model_role or '-'} model={model} "
        f"prompt={usage.get('prompt_tokens')} output={usage.get('output_tokens')} "
        f"total={usage.get('total_tokens')} est_prompt={record['estimated_prompt_tokens']}"
        f"{extra}"
    )


def log_context_budget(package: dict[str, Any], session_id: str) -> None:
    budget = package.get("budget")
    if not isinstance(budget, dict):
        return
    log_line(
        "context_budget "
        f"session={session_id} "
        f"total={budget.get('estimated_total_tokens')}/{budget.get('total_budget_tokens')} "
        f"current={budget.get('estimated_current_memory_tokens')}/{budget.get('current_memory_budget_tokens')} "
        f"archive={budget.get('estimated_compressed_memory_tokens')}/{budget.get('compressed_memory_budget_tokens')} "
        f"core={budget.get('estimated_core_tokens')}/{budget.get('core_budget_tokens')} "
        f"domain={budget.get('estimated_domain_state_tokens')} "
        f"dropped_recent={budget.get('dropped_session_recent')} "
        f"dropped_trace={budget.get('dropped_session_trace')} "
        f"dropped_archive={budget.get('dropped_archive_relevant')} "
        f"dropped_core={budget.get('dropped_core_facts')} "
        f"exceeded={budget.get('budget_exceeded')}"
    )


def log_sleep_compression_metrics(task: dict[str, Any], result: dict[str, Any]) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    events = task.get("inputs", {}).get("events", [])
    transcript = sleep_events_transcript(events if isinstance(events, list) else [])
    raw_tokens = estimate_tokens(transcript)
    stored_archive_payload = {
        "gist": result.get("gist"),
        "narrative": result.get("narrative"),
        "compact_memory": result.get("compact_memory"),
        "facts": result.get("facts", []),
        "quotes": result.get("quotes", []),
        "emotional_markers": result.get("emotional_markers", []),
        "topic_thread": result.get("topic_thread", []),
        "personal_signals": result.get("personal_signals", []),
        "relational_tone": result.get("relational_tone"),
    }
    prompt_archive_payload = compact_archive_for_prompt(stored_archive_payload)
    stored_archive_tokens = estimate_tokens(json.dumps(stored_archive_payload, ensure_ascii=False))
    prompt_archive_tokens = estimate_tokens(json.dumps(prompt_archive_payload, ensure_ascii=False))
    compact_memory_tokens = estimate_tokens(clean_string(result.get("compact_memory")))
    stored_ratio = stored_archive_tokens / raw_tokens if raw_tokens else 0.0
    prompt_ratio = prompt_archive_tokens / raw_tokens if raw_tokens else 0.0
    compact_ratio = compact_memory_tokens / raw_tokens if raw_tokens else 0.0
    record = {
        "timestamp": now_rfc3339(),
        "kind": "sleep_compression_metric",
        "task_id": task.get("task_id"),
        "archive_id": task.get("inputs", {}).get("preliminary_archive_id"),
        "raw_event_count": len(events) if isinstance(events, list) else 0,
        "raw_chat_estimated_tokens": raw_tokens,
        "stored_archive_estimated_tokens": stored_archive_tokens,
        "stored_archive_ratio": round(stored_ratio, 4),
        "prompt_archive_estimated_tokens": prompt_archive_tokens,
        "prompt_archive_ratio": round(prompt_ratio, 4),
        "compact_memory_estimated_tokens": compact_memory_tokens,
        "compact_memory_ratio": round(compact_ratio, 4),
        "compressed_estimated_tokens": compact_memory_tokens,
        "compression_ratio": round(compact_ratio, 4),
        "memory_units": len(result.get("memory_units", [])),
        "emotional_markers": len(result.get("emotional_markers", [])),
        "personal_signals": len(result.get("personal_signals", [])),
        "topic_thread": len(result.get("topic_thread", [])),
        "tags": result.get("tags", []),
    }
    with TOKEN_USAGE_PATH.open("a", encoding="utf-8") as file:
        file.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
    log_line(
        "sleep_compression_tokens "
        f"task={record['task_id']} archive={record['archive_id']} "
        f"events={record['raw_event_count']} raw_est={raw_tokens} "
        f"stored_est={stored_archive_tokens} stored_ratio={record['stored_archive_ratio']} "
        f"prompt_est={prompt_archive_tokens} prompt_ratio={record['prompt_archive_ratio']} "
        f"compact_est={compact_memory_tokens} compact_ratio={record['compact_memory_ratio']} "
        f"memory_units={record['memory_units']}"
    )


def chat_system_instruction() -> str:
    return CHAT_SYSTEM_PROMPT_PATH.read_text(encoding="utf-8").strip()


def chat_prompt(package: dict[str, Any], user_text: str) -> str:
    return render_chat_prompt(package, user_text)


def render_chat_prompt(package: dict[str, Any], user_text: str) -> str:
    recent_events = normalized_context_events(package.get("session_recent"))
    trace_events = normalized_context_events(package.get("session_trace"))
    prior_recent = drop_current_user_message(recent_events, user_text)
    recent_ids = {event["event_id"] for event in recent_events if event.get("event_id")}
    older_trace = [
        event
        for event in trace_events
        if event.get("event_id") and event["event_id"] not in recent_ids
    ][-20:]

    lines = ["<memory_context>"]
    lines.extend(
        [
            "<state>",
            f"conversation_state: {'ongoing' if prior_recent else 'new_or_no_recent_context'}",
        ]
    )
    if prior_recent:
        lines.append(
            "instruction: Continue the dialogue from the latest turn. Do not greet unless the current user message is a greeting."
        )
    else:
        lines.append("instruction: No prior active dialogue is visible; a short greeting is allowed if natural.")
    lines.append("</state>")

    core_lines = render_core_facts_for_prompt(package.get("core_facts"))
    lines.append("")
    lines.append("<core_memory>")
    if core_lines:
        lines.extend(core_lines)
    else:
        lines.append("(empty)")
    lines.append("</core_memory>")

    archive_lines = render_archive_memories_for_prompt(package.get("archive_relevant"))
    lines.append("")
    lines.append("<long_memory>")
    if archive_lines:
        lines.extend(archive_lines)
    else:
        lines.append("(empty)")
    lines.append("</long_memory>")

    lines.append("")
    lines.append("<short_memory>")
    if older_trace:
        lines.append("<older_active_dialogue>")
        lines.extend(render_dialogue_lines(older_trace, max_text_chars=180))
        lines.append("</older_active_dialogue>")

    if prior_recent:
        lines.append("<recent_dialogue>")
        lines.extend(render_dialogue_lines(prior_recent, max_text_chars=900))
        lines.append("</recent_dialogue>")
    else:
        lines.append("(empty)")
    lines.append("</short_memory>")

    lines.extend(
        [
            "",
            "<current_user_message>",
            xml_escape(clean_string(user_text), quote=False),
            "</current_user_message>",
            "",
            "<assistant_response_slot>",
            "Write only the assistant reply for the current user message.",
            "</assistant_response_slot>",
            "</memory_context>",
        ]
    )
    return "\n".join(lines)


def normalized_context_events(value: Any) -> list[dict[str, str]]:
    if not isinstance(value, list):
        return []
    events = []
    for item in value:
        if not isinstance(item, dict):
            continue
        text = context_event_text(item)
        if not text:
            continue
        events.append(
            {
                "event_id": clean_string(item.get("event_id")),
                "role": context_event_role(item),
                "text": text,
            }
        )
    return events


def context_event_text(event: dict[str, Any]) -> str:
    payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
    return clean_string(event.get("text")) or clean_string(payload.get("text"))


def context_event_role(event: dict[str, Any]) -> str:
    event_type = clean_string(event.get("type")) or clean_string(event.get("event_type"))
    return "assistant" if event_type == "assistant_message" else "user"


def drop_current_user_message(events: list[dict[str, str]], user_text: str) -> list[dict[str, str]]:
    if not events:
        return []
    current_text = clean_string(user_text)
    last = events[-1]
    if last.get("role") == "user" and last.get("text") == current_text:
        return events[:-1]
    return events


def render_core_facts_for_prompt(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    lines = []
    for fact in value:
        if not isinstance(fact, dict):
            continue
        text = truncate_text(clean_string(fact.get("text")), 260)
        if not text:
            continue
        text = xml_escape(text, quote=False)
        category = clean_string(fact.get("category")) or "core"
        confidence = fact.get("confidence")
        if confidence is None:
            lines.append(f"- {category}: {text}")
        else:
            lines.append(f"- {category} ({round(clamp_float(confidence, 0.0), 2)}): {text}")
    return lines


def render_archive_memories_for_prompt(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    lines = []
    for archive in value[:5]:
        if not isinstance(archive, dict):
            continue
        compact = compact_archive_for_prompt(archive)
        memory = clean_string(compact.get("compact_memory")) or clean_string(compact.get("gist"))
        if not memory:
            continue
        relevance = compact.get("relevance_score")
        prefix = f"- [{relevance}] " if relevance is not None else "- "
        for index, memory_line in enumerate(memory.splitlines()):
            memory_line = memory_line.strip()
            if not memory_line:
                continue
            memory_line = xml_escape(memory_line, quote=False)
            lines.append((prefix if index == 0 else "  ") + memory_line)
    return lines


def render_dialogue_lines(events: list[dict[str, str]], max_text_chars: int) -> list[str]:
    lines = []
    for event in events:
        text = truncate_text(event.get("text", ""), max_text_chars)
        if text:
            lines.append(f"{event.get('role', 'user')}: {xml_escape(text, quote=False)}")
    return lines


def truncate_text(text: str, max_chars: int) -> str:
    cleaned = clean_string(text)
    if len(cleaned) <= max_chars:
        return cleaned
    return cleaned[: max_chars - 3].rstrip() + "..."


def chat_prompt_json_debug(package: dict[str, Any]) -> str:
    compact = compact_context_package(package)
    return (
        "Memory Engine compact context JSON:\n"
        f"{json.dumps(compact, ensure_ascii=False, indent=2)}\n\n"
        "This view is for debug only."
    )


def chat_prompt_telemetry(package: dict[str, Any], session_id: str, prompt: str) -> dict[str, Any]:
    baseline_text = raw_chat_history_baseline_text(session_id)
    system_tokens = estimate_tokens(chat_system_instruction())
    baseline_without_compression = system_tokens + estimate_tokens(baseline_text)
    prompt_estimate = system_tokens + estimate_tokens(prompt)
    debug_package_tokens = estimate_tokens(json.dumps(package, ensure_ascii=False))
    budget = package.get("budget") if isinstance(package.get("budget"), dict) else {}
    return {
        "session_id": session_id,
        "system_instruction_estimated_tokens": system_tokens,
        "compact_prompt_estimated_tokens": prompt_estimate,
        "debug_package_estimated_tokens": debug_package_tokens,
        "baseline_without_compression_estimated_tokens": baseline_without_compression,
        "estimated_savings_vs_baseline_tokens": baseline_without_compression - prompt_estimate,
        "context_budget_estimated_total_tokens": budget.get("estimated_total_tokens"),
        "context_budget_exceeded": budget.get("budget_exceeded"),
        "context_budget_dropped_session_recent": budget.get("dropped_session_recent"),
        "context_budget_dropped_session_trace": budget.get("dropped_session_trace"),
        "context_budget_dropped_archive_relevant": budget.get("dropped_archive_relevant"),
        "context_budget_dropped_core_facts": budget.get("dropped_core_facts"),
    }


def compact_context_package(package: dict[str, Any]) -> dict[str, Any]:
    return {
        "core_facts": [compact_core_fact(fact) for fact in package.get("core_facts", [])],
        "session_recent": [compact_event(event) for event in package.get("session_recent", [])],
        "session_trace": [compact_event(event) for event in package.get("session_trace", [])],
        "archive_relevant": [
            compact_archive_for_prompt(archive) for archive in package.get("archive_relevant", [])
        ],
        "domain_state": compact_domain_state(package.get("domain_state")),
    }


def compact_core_fact(fact: Any) -> dict[str, Any]:
    if not isinstance(fact, dict):
        return {}
    compact = {
        "category": clean_string(fact.get("category")) or "core",
        "text": clean_string(fact.get("text")),
    }
    if fact.get("confidence") is not None:
        compact["confidence"] = round(clamp_float(fact.get("confidence"), 0.0), 2)
    return compact


def compact_event(event: Any) -> dict[str, Any]:
    if not isinstance(event, dict):
        return {}
    payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
    text = clean_string(event.get("text")) or clean_string(payload.get("text"))
    event_type = clean_string(event.get("type")) or clean_string(event.get("event_type"))
    compact: dict[str, Any] = {"type": event_type or "event", "text": text}
    theme = clean_string(event.get("theme"))
    if theme:
        compact["theme"] = theme
    return compact


def compact_archive_for_prompt(archive: Any) -> dict[str, Any]:
    if not isinstance(archive, dict):
        return {}
    compact: dict[str, Any] = {}
    archive_id = clean_string(archive.get("id")) or clean_string(archive.get("archive_id"))
    compact_memory = clean_string(archive.get("compact_memory"))
    if archive_id:
        compact["archive_id"] = archive_id
    if compact_memory:
        compact["compact_memory"] = compact_memory
        if archive.get("relevance_score") is not None:
            compact["relevance_score"] = round(clamp_float(archive.get("relevance_score"), 0.0), 2)
        return compact

    gist = clean_string(archive.get("gist"))
    theme = clean_string(archive.get("theme"))
    if gist:
        compact["gist"] = gist
    if theme:
        compact["theme"] = theme
    if archive.get("relevance_score") is not None:
        compact["relevance_score"] = round(clamp_float(archive.get("relevance_score"), 0.0), 2)
    return {key: value for key, value in compact.items() if value not in ("", [], None)}


def compact_emotional_marker(marker: dict[str, Any]) -> dict[str, Any]:
    compact = {
        "target": truncate_text(clean_string(marker.get("target")), 80),
        "affect": truncate_text(clean_string(marker.get("affect")), 80),
        "strength": round(clamp_float(marker.get("strength"), 0.0), 2),
    }
    evidence = truncate_text(clean_string(marker.get("evidence")), 160)
    quote = truncate_text(clean_string(marker.get("quote")), 160)
    if evidence:
        compact["evidence"] = evidence
    if quote:
        compact["quote"] = quote
    return compact


def compact_topic_thread_item(item: dict[str, Any]) -> dict[str, Any]:
    compact = {"topic": truncate_text(clean_string(item.get("topic")), 100)}
    subtopics = normalize_string_list(item.get("subtopics"))
    summary = truncate_text(clean_string(item.get("summary")), 160)
    energy = clean_string(item.get("energy"))
    if subtopics:
        compact["subtopics"] = [truncate_text(item, 60) for item in subtopics[:4]]
    if summary:
        compact["summary"] = summary
    if energy:
        compact["energy"] = energy
    return compact


def compact_personal_signal(signal: dict[str, Any]) -> dict[str, Any]:
    compact = {
        "category": normalize_category(signal.get("category")),
        "text": truncate_text(clean_string(signal.get("text")), 180),
        "confidence": round(clamp_float(signal.get("confidence"), 0.0), 2),
    }
    evidence = truncate_text(clean_string(signal.get("evidence")), 160)
    if evidence:
        compact["evidence"] = evidence
    return compact


def compact_relational_tone(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        return {}
    compact = {}
    for key in ("warmth", "intellectual_engagement", "intimacy", "trust", "playfulness", "tension"):
        if value.get(key) is not None:
            compact[key] = round(clamp_float(value.get(key), 0.0), 2)
    summary = truncate_text(clean_string(value.get("summary")), 180)
    if summary:
        compact["summary"] = summary
    return compact


def compact_domain_state(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        return {}
    compact = {}
    current_text = clean_string(value.get("current_text"))
    active_topic = clean_string(value.get("active_topic"))
    if current_text:
        compact["current_text"] = current_text
    if active_topic:
        compact["active_topic"] = active_topic
    return compact


def raw_chat_history_baseline_text(session_id: str) -> str:
    events_path = MEMORY_DIR / "sessions" / session_id / "events.jsonl"
    if not events_path.exists():
        return ""
    lines = ["Raw chronological chat history baseline without Memory Engine compression:"]
    try:
        with events_path.open("r", encoding="utf-8") as file:
            for line in file:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
                text = clean_string(payload.get("text"))
                if not text:
                    continue
                event_type = clean_string(event.get("type"))
                role = "assistant" if event_type == "assistant_message" else "user"
                lines.append(f"{role}: {text}")
    except OSError as err:
        log_exception(f"failed to build raw chat history baseline {events_path}", err)
    return "\n".join(lines)


def sleep_events_transcript(events: list[Any]) -> str:
    lines = []
    for event in events:
        if not isinstance(event, dict):
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        text = clean_string(payload.get("text"))
        if not text:
            continue
        event_type = clean_string(event.get("type"))
        role = "assistant" if event_type == "assistant_message" else "user"
        lines.append(f"{role}: {text}")
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


def archive_paths() -> list[Path]:
    if not ARCHIVE_DIR.exists():
        return []
    return sorted(
        ARCHIVE_DIR.rglob("*.json"),
        key=lambda path: path.stat().st_mtime,
    )


def read_archive(path: Path) -> dict[str, Any] | None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as err:
        log_exception(f"failed to read archive {path}", err)
        return None
    return payload if isinstance(payload, dict) else None


def complete_archives() -> list[dict[str, Any]]:
    archives = []
    for path in archive_paths():
        archive = read_archive(path)
        if archive and archive.get("status") == "complete":
            archives.append(archive)
    return archives


def user_event_ids_for_session(session_id: str) -> set[str]:
    if not session_id or "/" in session_id or "\\" in session_id:
        return set()
    events_path = MEMORY_DIR / "sessions" / session_id / "events.jsonl"
    if not events_path.exists():
        return set()

    event_ids = set()
    try:
        with events_path.open("r", encoding="utf-8") as file:
            for line in file:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if event.get("type") == "user_message":
                    event_id = clean_string(event.get("event_id"))
                    if event_id:
                        event_ids.add(event_id)
    except OSError as err:
        log_exception(f"failed to read session events {events_path}", err)
    return event_ids


def last_complete_archive() -> dict[str, Any] | None:
    archives = complete_archives()
    return archives[-1] if archives else None


def find_archive(archive_id: str) -> dict[str, Any] | None:
    if not archive_id:
        return None
    for archive in complete_archives():
        if archive.get("archive_id") == archive_id:
            return archive
    return None


def format_archives() -> str:
    archives = complete_archives()
    if not archives:
        return "No completed archives found yet. Use /sleep after a meaningful chat."

    lines = [f"Recent archives ({min(len(archives), ARCHIVE_LIST_LIMIT)} of {len(archives)}):"]
    for archive in reversed(archives[-ARCHIVE_LIST_LIMIT:]):
        archive_id = archive.get("archive_id", "")
        lines.append(
            f"- {archive_id} events={len(archive.get('source_event_ids', []))} "
            f"emotional={len(archive.get('emotional_markers', []))} "
            f"personal={len(archive.get('personal_signals', []))}"
        )
        gist = clean_string(archive.get("gist"))
        if gist:
            lines.append(f"  {truncate_chars(gist, 260)}")
    lines.append("Use /archive_last or /archive archive_id to inspect details.")
    return "\n".join(lines)


def format_archive_detail(archive: dict[str, Any] | None) -> str:
    if not archive:
        return "Archive not found. Use /archives to list available archive ids."

    lines = [
        f"Archive: {archive.get('archive_id', '')}",
        f"Status: {archive.get('status', '')}",
        f"Events: {len(archive.get('source_event_ids', []))}",
    ]
    gist = clean_string(archive.get("gist"))
    narrative = clean_string(archive.get("narrative"))
    compact_memory = clean_string(archive.get("compact_memory"))
    if compact_memory:
        lines.append("Compact memory:")
        lines.extend(f"- {line}" for line in compact_memory.splitlines() if line.strip())
    if gist:
        lines.append(f"Gist: {gist}")
    if narrative:
        lines.append(f"Narrative: {narrative}")

    emotional_markers = sorted(
        [item for item in archive.get("emotional_markers", []) if isinstance(item, dict)],
        key=lambda item: clamp_float(item.get("strength"), 0.0),
        reverse=True,
    )
    if emotional_markers:
        lines.append("Emotional markers:")
        for marker in emotional_markers[:ARCHIVE_DETAIL_LIMIT]:
            target = clean_string(marker.get("target"))
            affect = clean_string(marker.get("affect"))
            strength = clamp_float(marker.get("strength"), 0.0)
            lines.append(f"- {target} | {affect} | {strength:.2f}")
            evidence = clean_string(marker.get("evidence"))
            if evidence:
                lines.append(f"  {truncate_chars(evidence, 240)}")

    personal_signals = sorted(
        [item for item in archive.get("personal_signals", []) if isinstance(item, dict)],
        key=lambda item: clamp_float(item.get("confidence"), 0.0),
        reverse=True,
    )
    if personal_signals:
        lines.append("Personal signals:")
        for signal in personal_signals[:ARCHIVE_DETAIL_LIMIT]:
            text = clean_string(signal.get("text"))
            category = clean_string(signal.get("category"))
            confidence = clamp_float(signal.get("confidence"), 0.0)
            lines.append(f"- [{category} {confidence:.2f}] {text}")

    tone = archive.get("relational_tone")
    if isinstance(tone, dict):
        tone_parts = []
        for key in ("warmth", "intellectual_engagement", "intimacy", "trust", "playfulness", "tension"):
            if tone.get(key) is not None:
                tone_parts.append(f"{key}={clamp_float(tone.get(key), 0.0):.2f}")
        tone_summary = clean_string(tone.get("summary"))
        if tone_parts or tone_summary:
            lines.append("Relational tone:")
            if tone_parts:
                lines.append("- " + ", ".join(tone_parts))
            if tone_summary:
                lines.append(f"- {tone_summary}")

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


def format_core_seed(summary: dict[str, int]) -> str:
    return (
        "Core seed finished.\n"
        f"Archives scanned: {summary.get('archives', 0)}\n"
        f"Signals: {format_core_signal_counts(summary)}"
    )


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
        "/archives - list recent completed archive memories\n"
        "/archive_last - inspect the newest archive memory\n"
        "/archive id - inspect one archive memory by id\n"
        "/recall text - search archive memory\n"
        "/core - show stable Core facts\n"
        "/core_seed - seed Core from completed archive personal signals\n"
        "/remember text - save a stable Core fact manually\n"
        "/core_update id text - update a Core fact in this chat\n"
        "/core_forget id - deprecate a Core fact in this chat\n"
        "/tasks - show pending tasks\n"
        "/models - show model role mapping\n"
        "\n"
        "Plain text is stored as an event and answered by Gemini with memory context.\n"
        "Sleep is queued by token pressure, by the nightly idle schedule, or manually with /sleep."
    )


def importance_hint(text: str) -> str:
    lowered = text.lower()
    if has_memory_request(lowered):
        return "high"
    if has_core_signal(lowered):
        return "medium"
    return "normal"


def event_tags(text: str) -> list[str]:
    lowered = text.lower()
    tags = ["telegram_message"]

    if has_memory_request(lowered):
        tags.append("explicit_memory_request")
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


def extract_gemini_text(result: dict[str, Any], model: str) -> str:
    candidates = result.get("candidates") or []
    if not candidates:
        raise GeminiNoCandidatesError(model, result)
    parts = candidates[0].get("content", {}).get("parts") or []
    texts = [part.get("text", "") for part in parts if part.get("text")]
    if not texts:
        raise GeminiNoCandidatesError(model, result)
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
