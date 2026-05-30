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
DEV_SLEEP_NOTICES_ENV = "MEMORY_BOT_DEV_SLEEP_NOTICES"
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
MAX_CORE_CATEGORY_LENGTH = 64


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
        sleep_run: dict[str, Any],
        telegram: "TelegramClient | None" = None,
        chat_id: int | None = None,
        reason: str = "background sleep",
    ) -> bool:
        task_id = clean_string(sleep_run.get("sleep_task_id"))
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
            args=(task_id, sleep_run, telegram, chat_id, reason),
            name=f"sleep-{task_id}",
            daemon=True,
        )
        thread.start()
        log_line(f"{reason} queued task={task_id}")
        return True

    def _run(
        self,
        task_id: str,
        sleep_run: dict[str, Any],
        telegram: "TelegramClient | None",
        chat_id: int | None,
        reason: str,
    ) -> None:
        try:
            send_sleep_notice(
                telegram,
                chat_id,
                f"[dev sleep] started: {reason}, task={task_id}",
            )
            engine = memory_engine.MemoryEngine(
                str(MEMORY_DIR),
                host_id="telegram_gemini_bot",
            )
            summary = complete_sleep_result(engine, self._gemini, self._llm_config, sleep_run)
            log_line(f"{reason} completed: {summary.replace(chr(10), ' | ')}")
            if telegram is not None and chat_id is not None:
                if sleep_notices_enabled():
                    telegram.send_message(chat_id, f"[dev sleep] completed\n\n{summary}")
                else:
                    telegram.send_message(chat_id, f"Memory updated.\n\n{summary}")
        except Exception as err:
            log_exception(f"{reason} completion failed", err)
            if telegram is not None and chat_id is not None:
                if sleep_notices_enabled():
                    telegram.send_message(
                        chat_id,
                        "[dev sleep] failed\n\n" + friendly_error_message(err),
                    )
                else:
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
        f"idle_sleep_min_seconds={read_idle_sleep_min_seconds()} "
        f"dev_sleep_notices={sleep_notices_enabled()}"
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
            maybe_queue_scheduled_idle_sleep(engine, sleep_runner, telegram)
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
    prompt = chat_prompt(engine, package, text)
    prompt_telemetry = chat_prompt_telemetry(package, session_id, prompt)
    answer_response = gemini.generate_text(
        model=model,
        system_instruction=chat_system_instruction(),
        prompt=prompt,
        operation="chat_reply",
        model_role=llm_config.chat_role,
        telemetry=prompt_telemetry,
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
        telegram=telegram,
        chat_id=chat_id,
        session_id=session_id,
        package=package,
        prompt_telemetry=prompt_telemetry,
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
    sleep_run = json.loads(engine.begin_sleep_run(session_id))
    queued = sleep_runner.submit(
        sleep_run,
        telegram=telegram,
        chat_id=chat_id,
        reason=reason,
    )
    if queued and (notify or sleep_notices_enabled()) and telegram is not None and chat_id is not None:
        if sleep_notices_enabled():
            telegram.send_message(
                chat_id,
                "[dev sleep] queued: "
                f"{reason}, archive={sleep_run.get('archive_id')}, "
                f"task={sleep_run.get('sleep_task_id')}, "
                f"requests={len(sleep_run.get('requests', []))}",
            )
        else:
            telegram.send_message(chat_id, "Memory update queued.")


def maybe_queue_token_pressure_sleep(
    engine: memory_engine.MemoryEngine,
    sleep_runner: SleepRunner,
    telegram: TelegramClient | None,
    chat_id: int | None,
    session_id: str,
    package: dict[str, Any],
    prompt_telemetry: dict[str, Any],
) -> None:
    if has_pending_sleep_task(engine, session_id):
        return

    stats = session_unarchived_token_stats(session_id)
    if stats["event_count"] <= 0:
        return

    budget = package.get("budget") if isinstance(package.get("budget"), dict) else {}
    ratio = read_token_pressure_ratio()
    reasons = token_pressure_reasons(budget, stats, prompt_telemetry, ratio)
    if not reasons:
        return

    log_line(
        "token_pressure_sleep_trigger "
        f"session={session_id} reasons={','.join(reasons)} "
        f"unarchived_events={stats['event_count']} "
        f"unarchived_est_tokens={stats['estimated_tokens']} "
        f"prompt_est_tokens={int_or_zero(prompt_telemetry.get('compact_prompt_estimated_tokens'))} "
        f"ratio={ratio:.2f}"
    )
    queue_sleep_update(
        engine=engine,
        sleep_runner=sleep_runner,
        telegram=telegram if sleep_notices_enabled() else None,
        chat_id=chat_id if sleep_notices_enabled() else None,
        session_id=session_id,
        reason="token pressure sleep",
        notify=False,
    )


def token_pressure_reasons(
    budget: dict[str, Any],
    stats: dict[str, Any],
    prompt_telemetry: dict[str, Any],
    ratio: float,
) -> list[str]:
    reasons: list[str] = []
    if budget.get("budget_exceeded"):
        reasons.append("budget_exceeded")

    if int_or_zero(budget.get("dropped_session_recent")) > 0:
        reasons.append("dropped_session_recent")

    total_budget = int_or_zero(budget.get("total_budget_tokens"))
    prompt_estimated = int_or_zero(prompt_telemetry.get("compact_prompt_estimated_tokens"))
    if total_budget and prompt_estimated >= int(total_budget * ratio):
        reasons.append("prompt_budget_pressure")

    current_budget = int_or_zero(budget.get("current_memory_budget_tokens"))
    if current_budget and int(stats["estimated_tokens"]) >= int(current_budget * ratio):
        reasons.append("unarchived_session_pressure")

    return unique_preserve_order(reasons)


def maybe_queue_scheduled_idle_sleep(
    engine: memory_engine.MemoryEngine,
    sleep_runner: SleepRunner,
    telegram: TelegramClient | None = None,
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
            telegram=telegram if sleep_notices_enabled() else None,
            chat_id=chat_id_from_session_id(session_id) if sleep_notices_enabled() else None,
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
    sleep_run = json.loads(engine.begin_sleep_run(session_id))
    return complete_sleep_result(engine, gemini, llm_config, sleep_run)


def complete_sleep_result(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    sleep_run: dict[str, Any],
) -> str:
    run = sleep_run
    step = json.loads(engine.next_sleep_batch(json.dumps(run, ensure_ascii=False)))
    while True:
        run = step["run"]
        batch = step.get("batch")
        if not batch:
            break
        responses = [execute_llm_request(request, gemini, llm_config) for request in batch["requests"]]
        step = json.loads(
            engine.submit_sleep_batch(
                json.dumps(run, ensure_ascii=False),
                json.dumps(responses, ensure_ascii=False),
            )
        )
        run = step["run"]

    outcome = json.loads(engine.finish_sleep_run(json.dumps(run, ensure_ascii=False)))
    updated = outcome["archive_entry"]
    core_summary = outcome.get("core_summary", {})
    compact_tokens = estimate_tokens(clean_string(updated.get("compact_memory")))
    log_sleep_compression_metrics(engine, sleep_run, updated)
    log_line(
        "sleep_driver_completed "
        f"archive={updated.get('archive_id')} "
        f"completion_mode={outcome.get('completion_mode')} "
        f"failed_passes={','.join(outcome.get('failed_passes', [])) or 'none'} "
        f"compact_tokens={compact_tokens}"
    )
    return (
        f"Archive: {updated.get('archive_id')}\n"
        f"Task: {sleep_run.get('sleep_task_id')}\n"
        f"Completion: {outcome.get('completion_mode')}\n"
        f"Failed passes: {', '.join(outcome.get('failed_passes', [])) or 'none'}\n"
        f"Compact memory tokens: {compact_tokens}\n"
        f"Emotional markers: {len(updated.get('emotional_markers', []))}\n"
        f"Personal signals: {len(updated.get('personal_signals', []))}\n"
        f"Core signals: {format_core_signal_counts(core_summary)}\n"
        f"Gist: {updated['gist']}"
    )


def execute_llm_request(
    request: dict[str, Any],
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
) -> dict[str, Any]:
    request_id = clean_string(request.get("request_id"))
    prompt_id = clean_string(request.get("prompt_id"))
    role_hint = clean_string(request.get("role_hint")) or "balanced"
    try:
        prompt_path = PROMPTS_DIR / f"{prompt_id}.md"
        prompt_text = prompt_path.read_text(encoding="utf-8")
        selection = llm_config.for_role(role_hint)
        prompt_payload = json.dumps(request.get("prompt_inputs", {}), ensure_ascii=False, indent=2)
        response = gemini.generate_text(
            model=selection.model,
            system_instruction=prompt_text,
            prompt=prompt_payload,
            response_mime_type="application/json",
            operation=prompt_id,
            model_role=role_hint,
            telemetry={
                "prompt_id": prompt_id,
                "request_id": request_id,
                "task_id": request.get("task_id"),
            },
        )
        return {"status": "ok", "request_id": request_id, "text": response.text}
    except GeminiNoCandidatesError as err:
        kind = "provider_blocked" if err.block_reason else "other"
        return {
            "status": "err",
            "request_id": request_id,
            "kind": kind,
            "detail": str(err),
        }
    except GeminiApiError as err:
        kind = "transport" if err.status_code in (None, 408, 409, 429, 500, 502, 503, 504) else "other"
        return {
            "status": "err",
            "request_id": request_id,
            "kind": kind,
            "detail": str(err),
        }
    except Exception as err:
        return {
            "status": "err",
            "request_id": request_id,
            "kind": "other",
            "detail": f"{type(err).__name__}: {err}",
        }


def promote_existing_archives(engine: memory_engine.MemoryEngine) -> dict[str, int]:
    return json.loads(engine.seed_core_from_archives())


def format_core_signal_counts(summary: dict[str, int]) -> str:
    return (
        f"{summary.get('created', 0)} new, "
        f"{summary.get('updated', 0)} updated, "
        f"{summary.get('skipped', 0)} skipped"
    )


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


def sleep_notices_enabled() -> bool:
    return os.environ.get(DEV_SLEEP_NOTICES_ENV, "").strip().lower() in {"1", "true", "yes", "on"}


def send_sleep_notice(
    telegram: TelegramClient | None,
    chat_id: int | None,
    text: str,
) -> None:
    if not sleep_notices_enabled() or telegram is None or chat_id is None:
        return
    try:
        telegram.send_message(chat_id, text)
    except Exception as err:
        log_exception("failed to send dev sleep notice", err)


def chat_id_from_session_id(session_id: str) -> int | None:
    prefix = "telegram_"
    if not session_id.startswith(prefix):
        return None
    try:
        return int(session_id.removeprefix(prefix))
    except ValueError:
        return None



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


def log_sleep_compression_metrics(
    engine: memory_engine.MemoryEngine,
    sleep_run: dict[str, Any],
    result: dict[str, Any],
) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    sleep_task = sleep_task_inputs_from_run(sleep_run)
    events = sleep_task.get("events", [])
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
    prompt_memory_view = archive_memory_view_for_metrics(engine, result)
    stored_archive_tokens = estimate_tokens(json.dumps(stored_archive_payload, ensure_ascii=False))
    prompt_memory_view_tokens = estimate_tokens(prompt_memory_view)
    compact_memory_tokens = estimate_tokens(clean_string(result.get("compact_memory")))
    stored_ratio = stored_archive_tokens / raw_tokens if raw_tokens else 0.0
    prompt_ratio = prompt_memory_view_tokens / raw_tokens if raw_tokens else 0.0
    compact_ratio = compact_memory_tokens / raw_tokens if raw_tokens else 0.0
    record = {
        "timestamp": now_rfc3339(),
        "kind": "sleep_compression_metric",
        "task_id": sleep_run.get("sleep_task_id"),
        "archive_id": result.get("archive_id") or sleep_task.get("preliminary_archive_id"),
        "raw_event_count": len(events) if isinstance(events, list) else 0,
        "raw_chat_estimated_tokens": raw_tokens,
        "stored_archive_estimated_tokens": stored_archive_tokens,
        "stored_archive_ratio": round(stored_ratio, 4),
        "prompt_archive_estimated_tokens": prompt_memory_view_tokens,
        "prompt_archive_ratio": round(prompt_ratio, 4),
        "prompt_memory_view_estimated_tokens": prompt_memory_view_tokens,
        "prompt_memory_view_ratio": round(prompt_ratio, 4),
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
        f"prompt_view_est={prompt_memory_view_tokens} prompt_view_ratio={record['prompt_memory_view_ratio']} "
        f"compact_est={compact_memory_tokens} compact_ratio={record['compact_memory_ratio']} "
        f"memory_units={record['memory_units']}"
    )


def sleep_task_inputs_from_run(sleep_run: dict[str, Any]) -> dict[str, Any]:
    for state in sleep_run.get("requests", []):
        if not isinstance(state, dict):
            continue
        request = state.get("request") if isinstance(state.get("request"), dict) else {}
        prompt_inputs = request.get("prompt_inputs") if isinstance(request.get("prompt_inputs"), dict) else {}
        sleep_task = prompt_inputs.get("sleep_task")
        if isinstance(sleep_task, dict):
            return sleep_task
    return {}


def archive_memory_view_for_metrics(engine: memory_engine.MemoryEngine, archive: dict[str, Any]) -> str:
    archive_id = clean_string(archive.get("archive_id")) or "archive_metric"
    recall_item = {
        "source_layer": "archive",
        "id": archive_id,
        "gist": clean_string(archive.get("gist")) or "Archived memory.",
        "compact_memory": clean_string(archive.get("compact_memory")) or None,
        "narrative": None,
        "facts": [],
        "quotes": [],
        "source_session_id": clean_string(archive.get("source_session_id")) or None,
        "time_range": archive.get("time_range") if isinstance(archive.get("time_range"), dict) else None,
        "tags": normalize_string_list(archive.get("tags")),
        "theme": clean_string(archive.get("theme")) or None,
        "weight": archive.get("weight") if isinstance(archive.get("weight"), (int, float)) else 0.0,
        "freshness": archive.get("freshness") if isinstance(archive.get("freshness"), (int, float)) else 1.0,
        "relevance_score": 1.0,
        "relevance_explanation": None,
    }
    package = {
        "schema_version": "core_context_package.v1",
        "created_at": now_rfc3339(),
        "core_facts": [],
        "session_recent": [],
        "session_trace": [],
        "archive_relevant": [recall_item],
        "domain_state": {},
        "notes": ["metric-only prompt memory view for archived sleep output"],
    }
    return engine.render_memory_view(json.dumps(package, ensure_ascii=False), "")


def chat_system_instruction() -> str:
    return CHAT_SYSTEM_PROMPT_PATH.read_text(encoding="utf-8").strip()


def chat_prompt(engine: memory_engine.MemoryEngine, package: dict[str, Any], user_text: str) -> str:
    return engine.render_memory_view(json.dumps(package, ensure_ascii=False), user_text)


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
        "prompt_memory_view_estimated_tokens": prompt_estimate - system_tokens,
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
