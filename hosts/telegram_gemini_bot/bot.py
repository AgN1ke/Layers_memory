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

from local_embedder import (
    DEFAULT_EMBEDDING_DIM,
    DEFAULT_EMBEDDING_MODEL,
    LocalEmbedder,
    LocalEmbedderUnavailable,
)


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
DISTANT_RECALL_TRIGGERS = (
    "remember",
    "do you remember",
    "what did i say",
    "what have i said",
    "we talked about",
    "previously",
    "earlier",
    "last time",
    "пам'ята",
    "памʼята",
    "памята",
    "згада",
    "згадув",
    "ми говорили",
    "ми розмовляли",
    "що я казав",
    "що я казала",
    "ранiше",
    "раніше",
    "помни",
    "вспомни",
    "мы говорили",
    "что я говорил",
    "что я говорила",
)
DISTANT_RECALL_STOPWORDS = {
    "about",
    "again",
    "before",
    "did",
    "earlier",
    "have",
    "last",
    "please",
    "previously",
    "remember",
    "said",
    "talked",
    "that",
    "the",
    "this",
    "time",
    "what",
    "with",
    "you",
    "згадай",
    "згадати",
    "згадуєш",
    "казав",
    "казала",
    "мені",
    "ми",
    "памятаєш",
    "пам'ятаєш",
    "памʼятаєш",
    "про",
    "раніше",
    "розмовляли",
    "ти",
    "що",
    "говорили",
    "помнишь",
    "вспомни",
    "говорил",
    "говорила",
}


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
    def __init__(
        self,
        engine: memory_engine.MemoryEngine,
        gemini: "GeminiClient",
        llm_config: HostLlmConfig,
    ) -> None:
        self._engine = engine
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
            summary = complete_sleep_result(self._engine, self._gemini, self._llm_config, sleep_run)
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
    sleep_runner = SleepRunner(engine, gemini, llm_config)

    log_line(f"telegram token fingerprint: {secret_fingerprint(telegram_token)}")
    log_line(f"gemini key fingerprint: {secret_fingerprint(gemini_key)}")
    gemini.validate_key()
    log_line("gemini key validation completed")

    telegram.delete_webhook()
    log_line("deleteWebhook completed")
    queue_recovered_sleep_runs(engine, sleep_runner)
    print("Bot is running. Open Telegram and write to your bot.")
    print(
        "Commands: /help, /sleep, /archives, /archive_last, /recall text, "
        "/vectors, /vectors_on, /vectors_off, /vectors_purge, /recall_deep text, "
        "/core, /core_seed, /remember text, /core_update id text, "
        "/core_forget id, /evidence unit_id, /fidelity unit_id, /reflect, "
        "/candidates, /confirm id, /reject id, /forget_review, "
        "/forgotten, /remember_back unit_id, /tasks, /models"
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

    if text == "/vectors":
        state = json.loads(engine.vector_state(session_id))
        telegram.send_message(chat_id, format_vector_state(state))
        return

    if text == "/vectors_on":
        state = json.loads(engine.set_vector_scope(session_id, True, False))
        requests = json.loads(engine.pending_embedding_backfill(session_id))
        summary = run_embedding_requests(engine, requests)
        state = json.loads(engine.vector_state(session_id))
        telegram.send_message(
            chat_id,
            format_vector_state(state) + "\n" + format_embedding_summary(summary),
        )
        return

    if text == "/vectors_off":
        state = json.loads(engine.set_vector_scope(session_id, False, False))
        telegram.send_message(chat_id, format_vector_state(state))
        return

    if text == "/vectors_purge":
        state = json.loads(engine.set_vector_scope(session_id, False, True))
        telegram.send_message(chat_id, "Vector scope purged.\n" + format_vector_state(state))
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

    if text.startswith("/recall_deep"):
        query = text.removeprefix("/recall_deep").strip()
        if not query:
            telegram.send_message(chat_id, "Usage: /recall_deep text")
            return
        result = recall_distant_memory(engine, session_id, query)
        telegram.send_message(chat_id, format_deep_recall(result))
        return

    if text.startswith("/recall"):
        query = text.removeprefix("/recall").strip() or text
        telegram.send_message(chat_id, format_recall(recall(engine, session_id, query, explain=True)))
        return

    if text.startswith("/evidence"):
        unit_id = text.removeprefix("/evidence").strip()
        if not unit_id:
            telegram.send_message(chat_id, "Usage: /evidence memory_unit_id")
            return
        pack = json.loads(engine.build_evidence_pack(unit_id))
        telegram.send_message(chat_id, format_evidence_pack(pack))
        return

    if text.startswith("/fidelity"):
        unit_id = text.removeprefix("/fidelity").strip()
        if not unit_id:
            telegram.send_message(chat_id, "Usage: /fidelity memory_unit_id")
            return
        result = run_memory_fidelity(engine, gemini, llm_config, unit_id)
        telegram.send_message(chat_id, format_fidelity_result(result))
        return

    if text == "/reflect":
        result = run_reflection_analysis(
            engine,
            gemini,
            llm_config,
            session_id,
            core_scope(session_id),
        )
        telegram.send_message(chat_id, format_reflection_result(result))
        return

    if text == "/candidates":
        candidates = json.loads(engine.list_candidates())
        telegram.send_message(chat_id, format_candidates(candidates, scope=core_scope(session_id)))
        return

    if text == "/forget_review":
        result = run_forget_review(engine, gemini, llm_config, session_id)
        telegram.send_message(chat_id, format_forget_review_result(result))
        return

    if text == "/forgotten":
        result = json.loads(engine.list_forgotten_memory_units(session_id))
        telegram.send_message(chat_id, format_forgotten_units(result))
        return

    if text.startswith("/remember_back"):
        unit_id = text.removeprefix("/remember_back").strip()
        if not unit_id:
            telegram.send_message(chat_id, "Usage: /remember_back memory_unit_id")
            return
        unit = json.loads(engine.remember_back(unit_id))
        telegram.send_message(chat_id, f"Memory unit restored: {unit.get('thesis', '')}")
        return

    if text.startswith("/confirm"):
        candidate_id = text.removeprefix("/confirm").strip()
        if not candidate_id:
            telegram.send_message(chat_id, "Usage: /confirm candidate_id")
            return
        result = review_candidate(
            engine=engine,
            candidate_id=candidate_id,
            reviewed_by=f"telegram_user_{user.get('id', 'unknown')}",
            decision="approved",
            scope=core_scope(session_id),
        )
        telegram.send_message(chat_id, format_candidate_review(result))
        return

    if text.startswith("/reject"):
        candidate_id = text.removeprefix("/reject").strip()
        if not candidate_id:
            telegram.send_message(chat_id, "Usage: /reject candidate_id")
            return
        result = review_candidate(
            engine=engine,
            candidate_id=candidate_id,
            reviewed_by=f"telegram_user_{user.get('id', 'unknown')}",
            decision="rejected",
            scope=core_scope(session_id),
        )
        telegram.send_message(chat_id, format_candidate_review(result))
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
    distant_recall = maybe_add_distant_memory(engine, session_id, text, package, prompt)
    if distant_recall and distant_recall.get("used"):
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


def queue_recovered_sleep_runs(
    engine: memory_engine.MemoryEngine,
    sleep_runner: SleepRunner,
) -> None:
    try:
        runs = json.loads(engine.pending_sleep_runs())
    except Exception as err:
        log_exception("failed to inspect recoverable sleep runs", err)
        return

    queued = 0
    for run in runs if isinstance(runs, list) else []:
        if not isinstance(run, dict):
            continue
        if sleep_runner.submit(run, reason="recovered sleep"):
            queued += 1

    if queued:
        log_line(f"queued recovered sleep runs: {queued}")


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
        runs = json.loads(engine.pending_sleep_runs())
    except Exception as err:
        log_exception("failed to inspect pending sleep runs", err)
        return True

    for run in runs if isinstance(runs, list) else []:
        if not isinstance(run, dict):
            continue
        if clean_string(run.get("session_id")) == session_id:
            return True

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
    fidelity_summary = run_auto_fidelity_requests(
        engine,
        gemini,
        llm_config,
        outcome.get("fidelity_requests", []),
    )
    embedding_summary = run_embedding_requests(
        engine,
        outcome.get("embedding_requests", []),
    )
    compact_tokens = estimate_tokens(clean_string(updated.get("compact_memory")))
    log_sleep_compression_metrics(engine, sleep_run, updated)
    log_line(
        "sleep_driver_completed "
        f"archive={updated.get('archive_id')} "
        f"completion_mode={outcome.get('completion_mode')} "
        f"failed_passes={','.join(outcome.get('failed_passes', [])) or 'none'} "
        f"fidelity={format_fidelity_summary_for_log(fidelity_summary)} "
        f"embeddings={format_embedding_summary_for_log(embedding_summary)} "
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
        f"Fidelity: {format_fidelity_summary_for_user(fidelity_summary)}\n"
        f"Embeddings: {format_embedding_summary_for_user(embedding_summary)}\n"
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


def run_memory_fidelity(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    memory_unit_id: str,
) -> dict[str, Any]:
    start = json.loads(engine.begin_memory_fidelity_pass(memory_unit_id))
    response = execute_llm_request(start["request"], gemini, llm_config)
    task_id = start["pending_task"]["task_id"]
    updated = json.loads(
        engine.submit_memory_fidelity_response(
            task_id,
            json.dumps(response, ensure_ascii=False),
        )
    )
    return {
        "evidence_pack": start["evidence_pack"],
        "pending_task": start["pending_task"],
        "memory_unit": updated,
    }


def run_reflection_analysis(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    session_id: str,
    scope: str,
) -> dict[str, Any]:
    start = json.loads(engine.begin_reflection_analysis(session_id, scope))
    response = execute_llm_request(start["request"], gemini, llm_config)
    task_id = start["pending_task"]["task_id"]
    result = json.loads(
        engine.submit_reflection_response(
            task_id,
            json.dumps(response, ensure_ascii=False),
        )
    )
    return {
        "start": start,
        "result": result,
    }


def run_forget_review(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    session_id: str,
) -> dict[str, Any]:
    start = json.loads(engine.begin_forget_review(session_id))
    if int(start.get("candidate_count", 0)) == 0:
        return {"start": start, "result": None}
    response = execute_llm_request(start["request"], gemini, llm_config)
    task_id = start["pending_task"]["task_id"]
    result = json.loads(
        engine.submit_forget_review_response(
            task_id,
            json.dumps(response, ensure_ascii=False),
        )
    )
    return {
        "start": start,
        "result": result,
    }


def run_auto_fidelity_requests(
    engine: memory_engine.MemoryEngine,
    gemini: GeminiClient,
    llm_config: HostLlmConfig,
    requests: list[dict[str, Any]],
) -> dict[str, Any]:
    summary: dict[str, Any] = {
        "requested": len(requests),
        "completed": 0,
        "valid": 0,
        "rejected": 0,
        "needs_revision": 0,
        "failed": 0,
    }
    for request in requests:
        task_id = clean_string(request.get("task_id"))
        unit_id = clean_string(
            request.get("prompt_inputs", {})
            .get("evidence_pack", {})
            .get("memory_unit_id")
        )
        response = execute_llm_request(request, gemini, llm_config)
        try:
            updated = json.loads(
                engine.submit_memory_fidelity_response(
                    task_id,
                    json.dumps(response, ensure_ascii=False),
                )
            )
        except Exception as err:
            summary["failed"] += 1
            log_line(
                "auto_fidelity_failed "
                f"task={task_id or 'unknown'} "
                f"unit={unit_id or 'unknown'} "
                f"error={type(err).__name__}: {err}"
            )
            continue

        summary["completed"] += 1
        fidelity_status = clean_string(updated.get("fidelity_status"))
        unit_status = clean_string(updated.get("status"))
        if fidelity_status == "valid":
            summary["valid"] += 1
        elif unit_status == "rejected":
            summary["rejected"] += 1
        elif unit_status == "needs_revision":
            summary["needs_revision"] += 1
        log_line(
            "auto_fidelity_completed "
            f"task={task_id or 'unknown'} "
            f"unit={updated.get('memory_unit_id') or unit_id or 'unknown'} "
            f"fidelity_status={fidelity_status or 'unknown'} "
            f"unit_status={unit_status or 'unknown'}"
        )
    return summary


def run_embedding_requests(
    engine: memory_engine.MemoryEngine,
    requests: list[dict[str, Any]],
) -> dict[str, Any]:
    summary: dict[str, Any] = {
        "requested": len(requests),
        "completed": 0,
        "failed": 0,
        "items": 0,
        "appended": 0,
        "model_id": DEFAULT_EMBEDDING_MODEL,
        "dim": DEFAULT_EMBEDDING_DIM,
    }
    if not requests:
        return summary

    try:
        embedder = LocalEmbedder()
    except LocalEmbedderUnavailable as err:
        summary["failed"] = len(requests)
        summary["error"] = str(err)
        log_line(f"embedding_unavailable {err}")
        return summary

    for request in requests:
        task_id = clean_string(request.get("task_id"))
        request_id = clean_string(request.get("request_id"))
        embed_batch = request.get("prompt_inputs", {}).get("embed_batch", {})
        items = embed_batch.get("items") if isinstance(embed_batch, dict) else None
        if not isinstance(items, list) or not items:
            summary["failed"] += 1
            log_line(f"embedding_request_failed task={task_id or 'unknown'} reason=missing_items")
            continue
        texts = [clean_string(item.get("text")) for item in items if isinstance(item, dict)]
        memory_unit_ids = [
            clean_string(item.get("memory_unit_id")) for item in items if isinstance(item, dict)
        ]
        if len(texts) != len(items) or not all(texts) or not all(memory_unit_ids):
            summary["failed"] += 1
            log_line(f"embedding_request_failed task={task_id or 'unknown'} reason=invalid_items")
            continue

        try:
            vectors, telemetry = embedder.embed_passages(texts)
            result = {
                "schema_version": "embed_batch_result.v1",
                "model_id": telemetry.model_id,
                "dim": telemetry.dim,
                "results": [
                    {"memory_unit_id": memory_unit_id, "vector": vector}
                    for memory_unit_id, vector in zip(memory_unit_ids, vectors)
                ],
            }
            appended = int(engine.resume_compute_embedding(task_id, json.dumps(result, ensure_ascii=False)))
        except Exception as err:
            summary["failed"] += 1
            log_exception(f"embedding request failed task={task_id or 'unknown'}", err)
            continue

        summary["completed"] += 1
        summary["items"] += len(items)
        summary["appended"] += appended
        summary["model_id"] = telemetry.model_id
        summary["dim"] = telemetry.dim
        log_embedding_usage(
            operation="embed_batch",
            model_id=telemetry.model_id,
            dim=telemetry.dim,
            count=len(items),
            duration_ms=telemetry.duration_ms,
            telemetry={
                "request_id": request_id,
                "task_id": task_id,
                "scope": clean_string(embed_batch.get("scope")),
                "appended": appended,
            },
        )
    return summary


def recall_distant_memory(
    engine: memory_engine.MemoryEngine,
    session_id: str,
    query_text: str,
    top_k: int = 5,
    min_sim: float = 0.0,
) -> dict[str, Any]:
    try:
        embedder = LocalEmbedder()
        query_vec, telemetry = embedder.embed_query(query_text)
        log_embedding_usage(
            operation="recall_deep_query",
            model_id=telemetry.model_id,
            dim=telemetry.dim,
            count=1,
            duration_ms=telemetry.duration_ms,
            telemetry={"scope": session_id, "query_hash": text_hash(query_text)},
        )
    except LocalEmbedderUnavailable as err:
        return {"found": False, "reason": "embedder_unavailable", "error": str(err), "memories": []}

    result = json.loads(
        engine.recall_deep(
            json.dumps(
                {
                    "scope": session_id,
                    "query_vec": query_vec,
                    "model_id": DEFAULT_EMBEDDING_MODEL,
                    "top_k": top_k,
                    "min_sim": min_sim,
                    "now": now_rfc3339(),
                },
                ensure_ascii=False,
            )
        )
    )
    hits = result.get("hits") if isinstance(result.get("hits"), list) else []
    memories = [
        {
            "when": clean_string(hit.get("created_at")),
            "sim": float(hit.get("sim", 0.0) or 0.0),
            "strength": "vivid" if float(hit.get("sim", 0.0) or 0.0) >= 0.55 else "faint",
            "text": clean_string(hit.get("thesis")),
        }
        for hit in hits
        if isinstance(hit, dict)
    ]
    log_embedding_usage(
        operation="recall_deep",
        model_id=DEFAULT_EMBEDDING_MODEL,
        dim=DEFAULT_EMBEDDING_DIM,
        count=len(memories),
        duration_ms=0,
        telemetry={
            "scope": session_id,
            "query_hash": text_hash(query_text),
            "found": bool(result.get("found")),
            "reason": result.get("reason"),
            "top_sim": memories[0]["sim"] if memories else None,
        },
    )
    return {
        "found": bool(result.get("found")),
        "reason": result.get("reason"),
        "memories": memories,
        "raw": result,
    }


def should_consider_distant_recall(user_text: str) -> bool:
    lowered = user_text.lower()
    return any(trigger in lowered for trigger in DISTANT_RECALL_TRIGGERS)


def distant_recall_query_terms(user_text: str) -> list[str]:
    lowered = user_text.lower()
    terms = re.findall(r"[\w'ʼ-]{3,}", lowered, flags=re.UNICODE)
    result: list[str] = []
    for term in terms:
        cleaned = term.strip("-_'ʼ")
        if len(cleaned) < 3 or cleaned in DISTANT_RECALL_STOPWORDS:
            continue
        if any(trigger == cleaned for trigger in DISTANT_RECALL_TRIGGERS):
            continue
        result.append(cleaned)
    return unique_preserve_order(result)


def visible_memory_sections(memory_view: str) -> str:
    sections: list[str] = []
    for tag in ("core_memory", "long_memory", "short_memory"):
        match = re.search(rf"<{tag}>(.*?)</{tag}>", memory_view, flags=re.DOTALL)
        if match:
            sections.append(match.group(1))
    return "\n".join(sections).lower()


def visible_memory_already_answers(user_text: str, memory_view: str) -> bool:
    terms = distant_recall_query_terms(user_text)
    if not terms:
        return False
    visible = visible_memory_sections(memory_view)
    if not visible or visible.count("(empty)") == 3:
        return False
    matched = sum(1 for term in terms if visible_memory_contains_term(visible, term))
    required = 1 if len(terms) == 1 else 2
    return matched >= required


def visible_memory_contains_term(visible_memory: str, term: str) -> bool:
    if term in visible_memory:
        return True
    if re.search(r"[а-яіїєґё]", term, flags=re.IGNORECASE):
        return len(term) >= 3 and term[:3] in visible_memory
    return len(term) >= 5 and term[:5] in visible_memory


def distant_recall_scope_ready(engine: memory_engine.MemoryEngine, session_id: str) -> bool:
    state = json.loads(engine.vector_state(session_id))
    return state.get("status") == "ready"


def maybe_add_distant_memory(
    engine: memory_engine.MemoryEngine,
    session_id: str,
    user_text: str,
    package: dict[str, Any],
    memory_view: str,
) -> dict[str, Any] | None:
    if not should_consider_distant_recall(user_text):
        return None
    if visible_memory_already_answers(user_text, memory_view):
        log_line(f"distant recall skipped: visible memory already has query terms session={session_id}")
        return {"used": False, "reason": "visible_memory"}
    if not distant_recall_scope_ready(engine, session_id):
        log_line(f"distant recall skipped: vector scope not ready session={session_id}")
        return {"used": False, "reason": "not_ready"}

    result = recall_distant_memory(engine, session_id, user_text, top_k=3)
    if not result.get("found"):
        log_line(
            f"distant recall miss session={session_id} reason={result.get('reason')} query={text_hash(user_text)}"
        )
        return {"used": False, "reason": result.get("reason"), "result": result}

    append_distant_memories_to_package(package, result, session_id)
    log_line(
        f"distant recall added {len(result.get('memories', []))} memory item(s) "
        f"session={session_id} query={text_hash(user_text)}"
    )
    return {"used": True, "reason": "found", "result": result}


def append_distant_memories_to_package(package: dict[str, Any], result: dict[str, Any], session_id: str) -> None:
    raw = result.get("raw") if isinstance(result.get("raw"), dict) else {}
    hits = raw.get("hits") if isinstance(raw.get("hits"), list) else []
    archive_relevant = package.setdefault("archive_relevant", [])
    if not isinstance(archive_relevant, list):
        package["archive_relevant"] = archive_relevant = []

    existing = {
        clean_string(item.get("id"))
        for item in archive_relevant
        if isinstance(item, dict)
    }
    for hit in hits:
        if not isinstance(hit, dict):
            continue
        memory_unit_id = clean_string(hit.get("memory_unit_id"))
        archive_id = clean_string(hit.get("archive_id"))
        thesis = clean_string(hit.get("thesis"))
        if not memory_unit_id or not archive_id or not thesis:
            continue
        item_id = f"deep:{memory_unit_id}"
        if item_id in existing:
            continue
        sim = clamp_float(hit.get("sim"), 0.0)
        score = float(hit.get("score", sim) or sim)
        archive_relevant.insert(
            0,
            {
                "source_layer": "archive",
                "id": item_id,
                "gist": thesis,
                "compact_memory": thesis,
                "narrative": None,
                "facts": [thesis],
                "source_session_id": session_id,
                "tags": ["deep_recall", f"archive:{archive_id}"],
                "theme": "distant_memory",
                "weight": max(0.0, min(1.0, sim)),
                "freshness": 1.0,
                "relevance_score": score,
                "relevance_explanation": f"distant vector recall sim={sim:.3f}",
            },
        )
        existing.add(item_id)


def format_fidelity_summary_for_log(summary: dict[str, Any]) -> str:
    return (
        f"requested:{summary.get('requested', 0)},"
        f"completed:{summary.get('completed', 0)},"
        f"valid:{summary.get('valid', 0)},"
        f"rejected:{summary.get('rejected', 0)},"
        f"needs_revision:{summary.get('needs_revision', 0)},"
        f"failed:{summary.get('failed', 0)}"
    )


def format_fidelity_summary_for_user(summary: dict[str, Any]) -> str:
    requested = int(summary.get("requested", 0) or 0)
    if requested == 0:
        return "0 auto-routed"
    return (
        f"{requested} auto-routed, "
        f"{summary.get('completed', 0)} completed, "
        f"{summary.get('failed', 0)} failed"
    )


def format_embedding_summary_for_log(summary: dict[str, Any]) -> str:
    return (
        f"requested:{summary.get('requested', 0)},"
        f"completed:{summary.get('completed', 0)},"
        f"failed:{summary.get('failed', 0)},"
        f"items:{summary.get('items', 0)},"
        f"appended:{summary.get('appended', 0)}"
    )


def format_embedding_summary_for_user(summary: dict[str, Any]) -> str:
    requested = int(summary.get("requested", 0) or 0)
    if requested == 0:
        return "0 requested"
    base = (
        f"{requested} batch(es), "
        f"{summary.get('completed', 0)} completed, "
        f"{summary.get('appended', 0)} rows appended, "
        f"{summary.get('failed', 0)} failed"
    )
    if summary.get("error"):
        return base + f" ({summary['error']})"
    return base


def format_embedding_summary(summary: dict[str, Any]) -> str:
    return "Embeddings: " + format_embedding_summary_for_user(summary)


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


def review_candidate(
    engine: memory_engine.MemoryEngine,
    candidate_id: str,
    reviewed_by: str,
    decision: str,
    scope: str,
    note: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema_version": "candidate_review_input.v1",
        "candidate_id": candidate_id,
        "reviewed_by": reviewed_by,
        "decision": decision,
        "core_scope": scope,
    }
    if note:
        payload["note"] = note
    return json.loads(engine.review_candidate(json.dumps(payload, ensure_ascii=False)))


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
        "utc_offset_minutes": local_utc_offset_minutes(),
    }
    package = json.loads(engine.core_context_package(json.dumps(request, ensure_ascii=False)))
    log_context_budget(package, session_id)
    return package


def local_utc_offset_minutes() -> int:
    offset = datetime.now(timezone.utc).astimezone().utcoffset()
    if offset is None:
        return 0
    return int(offset.total_seconds() // 60)


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


def text_hash(value: str) -> str:
    return "sha256:" + hashlib.sha256(value.encode("utf-8")).hexdigest()[:16]


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


def log_embedding_usage(
    operation: str,
    model_id: str,
    dim: int,
    count: int,
    duration_ms: int,
    telemetry: dict[str, Any] | None = None,
) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    record: dict[str, Any] = {
        "timestamp": now_rfc3339(),
        "kind": "embedding_usage",
        "operation": operation,
        "model_id": model_id,
        "dim": dim,
        "count": count,
        "duration_ms": duration_ms,
    }
    if telemetry:
        record.update(telemetry)
    with TOKEN_USAGE_PATH.open("a", encoding="utf-8") as file:
        file.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
    log_line(
        "embedding_usage "
        f"operation={operation} model={model_id} dim={dim} count={count} duration_ms={duration_ms}"
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
    notes = recall_result.get("notes") if isinstance(recall_result.get("notes"), list) else []
    if not items:
        lines = ["No archive memory found yet. Write something important, then use /sleep."]
        if notes:
            lines.append("")
            lines.append("Notes:")
            lines.extend(f"- {note}" for note in notes)
        return "\n".join(lines)
    lines = ["Recall:"]
    for index, item in enumerate(items, start=1):
        lines.append(f"{index}. [{item['relevance_score']:.2f}] {item['gist']}")
        if item.get("narrative"):
            lines.append(f"   {item['narrative']}")
        if item.get("relevance_explanation"):
            lines.append(f"   {item['relevance_explanation']}")
    if notes:
        lines.append("")
        lines.append("Notes:")
        lines.extend(f"- {note}" for note in notes)
    return "\n".join(lines)


def format_deep_recall(result: dict[str, Any]) -> str:
    memories = result.get("memories") if isinstance(result.get("memories"), list) else []
    if not result.get("found"):
        reason = clean_string(result.get("reason")) or "not_found"
        error = clean_string(result.get("error"))
        suffix = f" ({error})" if error else ""
        return f"Deep recall: no match, reason={reason}{suffix}"
    lines = ["Deep recall:"]
    raw = result.get("raw") if isinstance(result.get("raw"), dict) else {}
    hits = raw.get("hits") if isinstance(raw.get("hits"), list) else []
    for index, memory in enumerate(memories, start=1):
        hit = hits[index - 1] if index - 1 < len(hits) and isinstance(hits[index - 1], dict) else {}
        sim = float(memory.get("sim", 0.0) or 0.0)
        score = float(hit.get("score", 0.0) or 0.0)
        lines.append(
            f"{index}. sim={sim:.3f} score={score:.3f} "
            f"{memory.get('strength', 'faint')}: {memory.get('text', '')}"
        )
    return "\n".join(lines)


def format_vector_state(state: dict[str, Any]) -> str:
    status = clean_string(state.get("status")) or "unknown"
    scope = clean_string(state.get("scope")) or "-"
    rows = int_or_zero(state.get("rows"))
    model_id = clean_string(state.get("model_id")) or "-"
    dim = state.get("dim") if isinstance(state.get("dim"), int) else "-"
    reason = clean_string(state.get("reason"))
    lines = [
        f"Vector scope: {scope}",
        f"Status: {status}",
        f"Rows: {rows}",
        f"Model: {model_id}",
        f"Dim: {dim}",
    ]
    if reason:
        lines.append(f"Reason: {reason}")
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
    memory_units = [item for item in archive.get("memory_units", []) if isinstance(item, dict)]
    if memory_units:
        lines.append("Memory units:")
        for unit in memory_units[:ARCHIVE_DETAIL_LIMIT]:
            unit_id = clean_string(unit.get("memory_unit_id"))
            status = clean_string(unit.get("status"))
            fidelity_status = clean_string(unit.get("fidelity_status"))
            thesis = clean_string(unit.get("thesis"))
            lines.append(f"- {unit_id} [{status}/{fidelity_status}] {thesis}")
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


def format_evidence_pack(pack: dict[str, Any]) -> str:
    lines = [
        f"Evidence pack: {pack.get('evidence_pack_id', '')}",
        f"Memory unit: {pack.get('memory_unit_id', '')}",
        f"Archive: {pack.get('archive_id', '')}",
        f"Estimated tokens: {pack.get('estimated_tokens', 0)}/{pack.get('max_estimated_tokens', 0)}",
        f"Truncated: {bool(pack.get('truncated'))}",
    ]
    thesis = clean_string(pack.get("target_thesis"))
    if thesis:
        lines.append(f"Thesis: {thesis}")
    evidence = clean_string(pack.get("unit_evidence"))
    if evidence:
        lines.append(f"Unit evidence: {truncate_chars(evidence, 260)}")
    events = [item for item in pack.get("events", []) if isinstance(item, dict)]
    if events:
        lines.append("Events:")
        for event in events[:ARCHIVE_DETAIL_LIMIT]:
            role = clean_string(event.get("role"))
            event_type = clean_string(event.get("type"))
            event_id = clean_string(event.get("event_id"))
            text = truncate_chars(clean_string(event.get("text")), 220)
            lines.append(f"- {role} {event_type} {event_id}: {text}")
    return "\n".join(lines)


def format_fidelity_result(result: dict[str, Any]) -> str:
    unit = result.get("memory_unit", {})
    review = unit.get("fidelity_review") if isinstance(unit, dict) else None
    if not isinstance(review, dict):
        review = {}
    lines = [
        f"Memory unit: {unit.get('memory_unit_id', '')}",
        f"Unit status: {unit.get('status', '')}",
        f"Fidelity: {unit.get('fidelity_status', '')}",
    ]
    confidence = review.get("confidence")
    explanation = clean_string(review.get("explanation"))
    if confidence is not None:
        lines.append(f"Confidence: {clamp_float(confidence, 0.0):.2f}")
    if explanation:
        lines.append(f"Explanation: {explanation}")
    revised = clean_string(review.get("revised_thesis"))
    missing = clean_string(review.get("missing_detail"))
    if revised:
        lines.append(f"Suggested thesis: {revised}")
    if missing:
        lines.append(f"Missing detail: {missing}")
    return "\n".join(lines)


def format_candidate(candidate: dict[str, Any], index: int | None = None) -> str:
    prefix = f"{index}. " if index is not None else ""
    candidate_id = clean_string(candidate.get("candidate_id"))
    category = clean_string(candidate.get("category")) or "core"
    status = clean_string(candidate.get("status")) or "candidate"
    confidence = clamp_float(candidate.get("confidence"), 0.0)
    text = clean_string(candidate.get("text"))
    suffix = ""
    contradicted_core_ids = [
        clean_string(value)
        for value in candidate.get("contradicted_core_fact_ids", [])
        if clean_string(value)
    ]
    if contradicted_core_ids:
        suffix = f" (contests: {', '.join(contradicted_core_ids[:3])})"
    return f"{prefix}{candidate_id} [{status} {category} {confidence:.2f}] {text}{suffix}"


def format_reflection_result(result: dict[str, Any]) -> str:
    start = result.get("start", {})
    reflection = result.get("result", {})
    candidates = [item for item in reflection.get("candidates", []) if isinstance(item, dict)]
    lines = [
        "Reflection finished.",
        f"Memory units scanned: {start.get('memory_unit_count', 0)}",
        f"Core facts in view: {start.get('core_fact_count', 0)}",
        f"Candidates: {len(candidates)}",
    ]
    if candidates:
        lines.append("")
        lines.append("Candidate beliefs:")
        for index, candidate in enumerate(candidates, start=1):
            lines.append(format_candidate(candidate, index))
            evidence = clean_string(candidate.get("evidence_summary"))
            if evidence:
                lines.append(f"   evidence: {truncate_chars(evidence, 180)}")
        lines.append("")
        lines.append("Use /confirm candidate_id or /reject candidate_id.")
    return "\n".join(lines)


def format_candidates(candidates: list[dict[str, Any]], scope: str) -> str:
    visible = [
        candidate
        for candidate in candidates
        if not candidate.get("core_scope") or candidate.get("core_scope") == scope
    ]
    if not visible:
        return "No candidate beliefs for this chat."
    lines = ["Candidate beliefs:"]
    for index, candidate in enumerate(visible[:20], start=1):
        lines.append(format_candidate(candidate, index))
    if len(visible) > 20:
        lines.append(f"... and {len(visible) - 20} more")
    lines.append("Use /confirm candidate_id or /reject candidate_id.")
    return "\n".join(lines)


def format_candidate_review(result: dict[str, Any]) -> str:
    candidate = result.get("candidate", {})
    promoted = result.get("promoted_fact")
    contested = [fact for fact in result.get("contested_facts", []) if isinstance(fact, dict)]
    lines = [
        f"Candidate: {candidate.get('candidate_id', '')}",
        f"Status: {candidate.get('status', '')}",
        f"Text: {candidate.get('text', '')}",
    ]
    if isinstance(promoted, dict):
        lines.append(f"Promoted Core fact: {promoted.get('core_fact_id', '')}")
    if contested:
        lines.append("Contested Core facts:")
        for fact in contested[:5]:
            lines.append(f"- {fact.get('core_fact_id', '')}: {fact.get('text', '')}")
    return "\n".join(lines)


def format_forget_review_result(result: dict[str, Any]) -> str:
    start = result.get("start", {})
    applied = result.get("result")
    candidate_count = int(start.get("candidate_count", 0) or 0)
    if not applied:
        return f"Forget review finished. Candidates: {candidate_count}. Nothing to review."
    return "\n".join(
        [
            "Forget review finished.",
            f"Candidates: {candidate_count}",
            f"Reviewed: {applied.get('reviewed', 0)}",
            f"Forgotten: {applied.get('forgotten', 0)}",
            f"Kept: {applied.get('kept', 0)}",
            f"Protected: {applied.get('protected', 0)}",
            f"Ignored: {applied.get('ignored', 0)}",
        ]
    )


def format_forgotten_units(result: dict[str, Any]) -> str:
    units = [unit for unit in result.get("units", []) if isinstance(unit, dict)]
    if not units:
        return "No forgotten memory units for this chat."
    lines = ["Forgotten memory units:"]
    for index, unit in enumerate(units[:20], start=1):
        weight = float(unit.get("weight", 0.0) or 0.0)
        lines.append(
            f"{index}. {unit.get('memory_unit_id', '')} "
            f"[{weight:.2f}] {truncate_chars(clean_string(unit.get('thesis')), 160)}"
        )
    if len(units) > 20:
        lines.append(f"... and {len(units) - 20} more")
    lines.append("Use /remember_back memory_unit_id to restore one.")
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
        status = clean_string(fact.get("status")) or "active"
        status_prefix = "" if status == "active" else f"{status} "
        lines.append(
            f"{index}. {fact_id} [{status_prefix}{category} {confidence:.2f}] {fact.get('text', '')}"
        )
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
        "/vectors - show vector index status for this chat\n"
        "/vectors_on - enable local vector indexing for this chat and run backfill\n"
        "/vectors_off - disable and remove vector index for this chat\n"
        "/vectors_purge - remove vector index for this chat\n"
        "/recall_deep text - search vector memory for this chat\n"
        "/core - show stable Core facts\n"
        "/core_seed - seed Core from completed archive personal signals\n"
        "/remember text - save a stable Core fact manually\n"
        "/core_update id text - update a Core fact in this chat\n"
        "/core_forget id - deprecate a Core fact in this chat\n"
        "/evidence memory_unit_id - inspect the source evidence pack for one memory unit\n"
        "/fidelity memory_unit_id - run the reasoning fidelity validator for one memory unit\n"
        "/reflect - analyze validated memory units and create Core candidates\n"
        "/candidates - list candidate beliefs for this chat\n"
        "/confirm candidate_id - promote a reviewed candidate into Core\n"
        "/reject candidate_id - reject a candidate belief\n"
        "/forget_review - review old low-signal memory units for reversible forgetting\n"
        "/forgotten - list forgotten memory units for this chat\n"
        "/remember_back memory_unit_id - restore a forgotten memory unit\n"
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
