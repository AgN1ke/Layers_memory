"""Host conformance runner for Memory Engine v0.3.

The direct driver is the deterministic baseline: it uses the public Python
adapter surface, fakes LLM responses, and asserts engine state. Future Telegram
and Godot drivers should expose the same high-level operations and pass the
same scenario without owning memory policy.
"""

from __future__ import annotations

import argparse
import importlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
TELEGRAM_HOST_DIR = ROOT / "hosts" / "telegram_gemini_bot"
GODOT_HEADLESS_DIR = ROOT / "hosts" / "godot_headless"
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import memory_engine


SESSION_ID = "host_conformance_direct"
CORE_SCOPE = SESSION_ID


class ConformanceError(AssertionError):
    pass


@dataclass
class DriverResult:
    runtime_dir: Path
    archive_id: str
    memory_unit_count: int
    core_fact_count: int


def dumps(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False)


def loads(raw: str) -> Any:
    return json.loads(raw)


def event_text(event: dict[str, Any]) -> str:
    text = event.get("text")
    if isinstance(text, str):
        return text
    payload = event.get("payload")
    if isinstance(payload, dict) and isinstance(payload.get("text"), str):
        return payload["text"]
    return ""


def event_kind(event: dict[str, Any]) -> str:
    value = event.get("event_type") or event.get("type")
    return value if isinstance(value, str) else ""


def event_id(event: dict[str, Any]) -> str:
    value = event.get("event_id")
    if not isinstance(value, str) or not value:
        raise ConformanceError(f"sleep prompt event has no event_id: {event!r}")
    return value


def load_telegram_bot() -> Any:
    if str(TELEGRAM_HOST_DIR) not in sys.path:
        sys.path.insert(0, str(TELEGRAM_HOST_DIR))
    return importlib.import_module("bot")


def find_godot_binary(explicit: str | None = None) -> str:
    candidates = [
        explicit,
        os.environ.get("GODOT_BIN"),
        shutil.which("godot4"),
        shutil.which("godot"),
    ]
    for candidate in candidates:
        if candidate:
            return candidate
    raise ConformanceError(
        "Godot executable not found. Set GODOT_BIN or pass --godot-bin for --host godot-headless."
    )


def godot_extension_filename() -> str:
    if sys.platform.startswith("win"):
        return "memory_engine_godot.dll"
    if sys.platform == "darwin":
        return "libmemory_engine_godot.dylib"
    return "libmemory_engine_godot.so"


def build_godot_extension() -> Path:
    subprocess.run(
        ["cargo", "build", "-p", "godot_adapter"],
        cwd=ROOT,
        check=True,
    )
    library = ROOT / "target" / "debug" / godot_extension_filename()
    if not library.exists():
        raise ConformanceError(f"Godot adapter library was not built: {library}")
    return library


def memory_units_for_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    units: list[dict[str, Any]] = []
    for event in events:
        text = event_text(event)
        source_id = event_id(event)
        if "Мене звати Микита" in text:
            units.append(
                {
                    "thesis": "Ім'я -> користувача звати Микита.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["name", "profile"],
                    "weight": 0.95,
                }
            )
        if "Іржа" in text or "кішк" in text.lower():
            units.append(
                {
                    "thesis": "Кішка Іржа -> у користувача є кішка Іржа.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["pet", "personal_memory"],
                    "weight": 0.95,
                }
            )
        if "космос" in text.lower():
            units.append(
                {
                    "thesis": "Космос -> користувач любить тему космосу.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["interest"],
                    "weight": 0.9,
                }
            )
    if not units and events:
        units.append(
            {
                "thesis": "Conformance dialogue -> коротка перевірка host memory path.",
                "source_event_ids": [event_id(events[0])],
                "evidence": event_text(events[0]),
                "tags": ["host_conformance"],
                "weight": 0.7,
            }
        )
    return units


def personal_signals_for_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    signals: list[dict[str, Any]] = []
    for event in events:
        text = event_text(event)
        source_id = event_id(event)
        if "Мене звати Микита" in text:
            signals.append(
                {
                    "text": "Користувача звати Микита.",
                    "category": "name",
                    "confidence": 0.95,
                    "source_event_ids": [source_id],
                }
            )
        if "Іржа" in text or "кішк" in text.lower():
            signals.append(
                {
                    "text": "У користувача є кішка Іржа.",
                    "category": "pet",
                    "confidence": 0.95,
                    "source_event_ids": [source_id],
                }
            )
        if "люблю космос" in text.lower():
            signals.append(
                {
                    "text": "Користувач любить космос.",
                    "category": "interest",
                    "confidence": 0.92,
                    "source_event_ids": [source_id],
                }
            )
    return signals


def emotional_markers_for_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    markers: list[dict[str, Any]] = []
    for event in events:
        text = event_text(event)
        if "Іржа" in text or "кішк" in text.lower():
            markers.append(
                {
                    "target": "cat_irzha",
                    "affect": "warmth",
                    "strength": 0.9,
                    "source_event_ids": [event_id(event)],
                    "quote": text,
                }
            )
    return markers


class DirectHostDriver:
    def __init__(self, runtime_dir: Path) -> None:
        self.runtime_dir = runtime_dir
        self.engine = memory_engine.MemoryEngine(str(runtime_dir), host_id="host_conformance_direct")
        self.turn_index = 0

    def send_user_message(self, text: str) -> str:
        self.turn_index += 1
        self._ingest("user_message", "conformance_user", text, ["host_conformance"])
        package = self.context_package(text)
        view = self.engine.render_memory_view(dumps(package), text)
        if "<current_user_message>" not in view:
            raise ConformanceError("rendered memory view missed current_user_message")
        reply = f"ACK {self.turn_index}: {text[:48]}"
        self._ingest("assistant_message", "conformance_assistant", reply, ["host_conformance_reply"])
        return reply

    def run_sleep(self) -> dict[str, Any]:
        run = loads(self.engine.begin_sleep_run(SESSION_ID))
        while True:
            step = loads(self.engine.next_sleep_batch(dumps(run)))
            run = step["run"]
            batch = step.get("batch")
            if not batch:
                break
            responses = [self._response_for_request(run, request) for request in batch["requests"]]
            step = loads(self.engine.submit_sleep_batch(dumps(run), dumps(responses)))
            run = step["run"]
        outcome = loads(self.engine.finish_sleep_run(dumps(run)))
        for request in outcome.get("fidelity_requests", []):
            self._submit_valid_fidelity(request)
        return outcome

    def context_package(self, current_text: str) -> dict[str, Any]:
        return loads(
            self.engine.core_context_package(
                dumps(
                    {
                        "schema_version": "core_context_request.v1",
                        "session_id": SESSION_ID,
                        "domain_state": {"current_text": current_text},
                        "core_scope": CORE_SCOPE,
                        "query_text": current_text,
                        "recall_limit": 5,
                        "session_recent_limit": 8,
                        "session_trace_event_limit": 20,
                        "include_core": True,
                    }
                )
            )
        )

    def render_memory_view(self, current_text: str) -> str:
        package = self.context_package(current_text)
        return self.engine.render_memory_view(dumps(package), current_text)

    def _ingest(self, event_type: str, source: str, text: str, tags: list[str]) -> dict[str, Any]:
        timestamp = f"2026-06-10T10:{self.turn_index:02}:00.000Z"
        return loads(
            self.engine.ingest(
                dumps(
                    {
                        "schema_version": "event.v1",
                        "type": event_type,
                        "source": source,
                        "timestamp": timestamp,
                        "session_id": SESSION_ID,
                        "payload": {"text": text},
                        "tags": tags,
                        "theme": "host_conformance",
                        "importance_hint": "high" if event_type == "user_message" else "normal",
                    }
                )
            )
        )

    def _response_for_request(self, run: dict[str, Any], request: dict[str, Any]) -> dict[str, Any]:
        prompt_id = request["prompt_id"]
        if prompt_id == "sleep_consolidator":
            return {
                "status": "ok",
                "request_id": request["request_id"],
                "text": (
                    "GIST: Користувач назвався Микитою, розповів про кішку Іржу "
                    "і згадав інтерес до космосу.\n\n"
                    "У conformance-сценарії користувач дав стабільні особисті факти "
                    "і коротку тематичну розмову, яку треба зберегти як пам'ять."
                ),
            }

        events = self._sleep_events(request)
        user_events = [event for event in events if event_kind(event) == "user_message"]
        if not user_events:
            user_events = events

        if prompt_id == "memory_unit_pass":
            units = memory_units_for_events(user_events)
            payload = {
                "schema_version": "memory_units_result.v1",
                "archive_id": run["archive_id"],
                "memory_units": units,
            }
        elif prompt_id == "sleep_emotional_pass":
            payload = {"emotional_markers": emotional_markers_for_events(user_events)}
        elif prompt_id == "sleep_topic_thread_pass":
            source_ids = [event_id(event) for event in user_events[:3]]
            payload = {
                "topic_thread": [
                    {
                        "topic": "host_conformance",
                        "summary": "Користувач перевіряє пам'ять через ім'я, кішку Іржу і тему космосу.",
                        "source_event_ids": source_ids,
                    }
                ]
            }
        elif prompt_id == "sleep_personal_signal_pass":
            payload = {"personal_signals": personal_signals_for_events(user_events)}
        elif prompt_id == "sleep_relational_pass":
            payload = {"relational_tone": None}
        else:
            raise ConformanceError(f"unexpected LLM request prompt_id={prompt_id!r}")

        return {
            "status": "ok",
            "request_id": request["request_id"],
            "text": dumps(payload),
        }

    def _sleep_events(self, request: dict[str, Any]) -> list[dict[str, Any]]:
        inputs = request.get("prompt_inputs")
        if not isinstance(inputs, dict):
            raise ConformanceError(f"request has no prompt_inputs: {request!r}")
        sleep_task = inputs.get("sleep_task")
        if isinstance(sleep_task, dict) and isinstance(sleep_task.get("events"), list):
            return [event for event in sleep_task["events"] if isinstance(event, dict)]
        events = inputs.get("events")
        if isinstance(events, list):
            return [event for event in events if isinstance(event, dict)]
        raise ConformanceError(f"request has no sleep events: {request!r}")

    def _submit_valid_fidelity(self, request: dict[str, Any]) -> None:
        unit = request.get("prompt_inputs", {}).get("memory_unit", {})
        memory_unit_id = unit.get("memory_unit_id")
        archive_id = unit.get("archive_id")
        if not isinstance(memory_unit_id, str) or not isinstance(archive_id, str):
            return
        response = {
            "status": "ok",
            "request_id": request["request_id"],
            "text": dumps(
                {
                    "schema_version": "fidelity_review.v1",
                    "memory_unit_id": memory_unit_id,
                    "archive_id": archive_id,
                    "status": "valid",
                    "confidence": 0.95,
                    "explanation": "Direct conformance fake validator accepts the source-backed unit.",
                    "revised_thesis": None,
                    "missing_detail": None,
                }
            ),
        }
        self.engine.submit_memory_fidelity_response(request["task_id"], dumps(response))


class FakeTelegram:
    def __init__(self) -> None:
        self.messages: list[tuple[int, str]] = []

    def send_message(self, chat_id: int, text: str) -> None:
        self.messages.append((chat_id, text))


class FakeGemini:
    def __init__(self, telegram_bot: Any) -> None:
        self.telegram_bot = telegram_bot

    def generate_text(
        self,
        model: str,
        system_instruction: str,
        prompt: str,
        response_mime_type: str | None = None,
        operation: str = "generate_text",
        model_role: str | None = None,
        telemetry: dict[str, Any] | None = None,
    ) -> Any:
        del system_instruction, response_mime_type, model_role, telemetry
        if operation == "chat_reply":
            text = "ACK telegram-local: відповідь з fake Gemini без Telegram transport."
        elif operation == "sleep_consolidator":
            text = (
                "GIST: Telegram-local host зберіг ім'я Микити, кішку Іржу "
                "і космічний інтерес.\n\n"
                "Локальний Telegram host пройшов sleep без реального Telegram API, "
                "використовуючи той самий bot.py шлях і fake LLM."
            )
        elif operation == "memory_fidelity_pass":
            text = dumps(self._fidelity_payload(prompt))
        else:
            inputs = self._prompt_inputs(prompt)
            text = dumps(self._sleep_payload(operation, inputs))
        return self.telegram_bot.GeminiTextResponse(text=text, usage={}, model=model, operation=operation)

    def _prompt_inputs(self, prompt: str) -> dict[str, Any]:
        try:
            payload = json.loads(prompt)
        except json.JSONDecodeError as err:
            raise ConformanceError(f"fake Gemini received non-JSON prompt for operation: {err}") from err
        if not isinstance(payload, dict):
            raise ConformanceError("fake Gemini prompt payload is not an object")
        return payload

    def _sleep_events(self, inputs: dict[str, Any]) -> list[dict[str, Any]]:
        sleep_task = inputs.get("sleep_task")
        if isinstance(sleep_task, dict) and isinstance(sleep_task.get("events"), list):
            return [event for event in sleep_task["events"] if isinstance(event, dict)]
        return []

    def _sleep_payload(self, operation: str, inputs: dict[str, Any]) -> dict[str, Any]:
        events = [event for event in self._sleep_events(inputs) if event_kind(event) == "user_message"]
        if not events:
            events = self._sleep_events(inputs)

        if operation == "memory_unit_pass":
            return {
                "schema_version": "memory_units_result.v1",
                "archive_id": inputs.get("sleep_task", {}).get("archive_id", ""),
                "memory_units": memory_units_for_events(events),
            }
        if operation == "sleep_emotional_pass":
            return {"emotional_markers": emotional_markers_for_events(events)}
        if operation == "sleep_topic_thread_pass":
            return {
                "topic_thread": [
                    {
                        "topic": "telegram_local_conformance",
                        "summary": "Локальний Telegram host перевіряє ім'я, Іржу і космос.",
                        "source_event_ids": [event_id(event) for event in events[:3]],
                    }
                ]
            }
        if operation == "sleep_personal_signal_pass":
            return {"personal_signals": personal_signals_for_events(events)}
        if operation == "sleep_relational_pass":
            return {"relational_tone": None}
        raise ConformanceError(f"fake Gemini got unexpected operation={operation!r}")

    def _fidelity_payload(self, prompt: str) -> dict[str, Any]:
        inputs = self._prompt_inputs(prompt)
        evidence = inputs.get("evidence_pack") if isinstance(inputs.get("evidence_pack"), dict) else inputs
        return {
            "schema_version": "fidelity_review.v1",
            "memory_unit_id": evidence.get("memory_unit_id", ""),
            "archive_id": evidence.get("archive_id", ""),
            "status": "valid",
            "confidence": 0.95,
            "explanation": "Telegram-local fake validator accepts the source-backed unit.",
            "revised_thesis": None,
            "missing_detail": None,
        }


class TelegramLocalHostDriver:
    def __init__(self, runtime_dir: Path) -> None:
        self.telegram_bot = load_telegram_bot()
        self.runtime_dir = runtime_dir
        self.chat_id = 93001001
        self.session_id = f"telegram_{self.chat_id}"
        self.engine = memory_engine.MemoryEngine(str(runtime_dir / "memory"), host_id="telegram_local_conformance")
        self.telegram = FakeTelegram()
        self.gemini = FakeGemini(self.telegram_bot)
        self.llm_config = self.telegram_bot.HostLlmConfig(
            reasoning=self.telegram_bot.ModelSelection("fake", "fake-reasoning"),
            balanced=self.telegram_bot.ModelSelection("fake", "fake-balanced"),
            fast=self.telegram_bot.ModelSelection("fake", "fake-fast"),
            chat_role="balanced",
        )
        self.sleep_runner = self.telegram_bot.SleepRunner(self.engine, self.gemini, self.llm_config)
        self.turn_index = 0
        self._patch_runtime_paths()

    def _patch_runtime_paths(self) -> None:
        self.telegram_bot.MEMORY_DIR = self.runtime_dir / "memory"
        self.telegram_bot.ARCHIVE_DIR = self.telegram_bot.MEMORY_DIR / "archive"
        self.telegram_bot.LOG_DIR = self.runtime_dir / "logs"
        self.telegram_bot.LOG_PATH = self.telegram_bot.LOG_DIR / "bot.log"
        self.telegram_bot.TOKEN_USAGE_PATH = self.telegram_bot.LOG_DIR / "token_usage.jsonl"
        self.telegram_bot.STATE_DIR = self.runtime_dir / "state"
        self.telegram_bot.OFFSET_PATH = self.telegram_bot.STATE_DIR / "telegram_offset.json"
        self.telegram_bot.SLEEP_SCHEDULER_STATE_PATH = self.telegram_bot.STATE_DIR / "sleep_scheduler_state.json"
        self.telegram_bot.LOG_DIR.mkdir(parents=True, exist_ok=True)
        self.telegram_bot.STATE_DIR.mkdir(parents=True, exist_ok=True)

    def send_user_message(self, text: str) -> str:
        self.turn_index += 1
        before = len(self.telegram.messages)
        update = {
            "update_id": self.turn_index,
            "message": {
                "message_id": self.turn_index,
                "date": 1781080000 + self.turn_index,
                "chat": {"id": self.chat_id, "type": "private"},
                "from": {"id": 311422683, "first_name": "Mykyta"},
                "text": text,
            },
        }
        self.telegram_bot.handle_update(
            update,
            telegram=self.telegram,
            gemini=self.gemini,
            engine=self.engine,
            llm_config=self.llm_config,
            sleep_runner=self.sleep_runner,
        )
        if len(self.telegram.messages) <= before:
            raise ConformanceError("telegram-local host did not send a reply")
        return self.telegram.messages[-1][1]

    def run_sleep(self) -> dict[str, Any]:
        summary = self.telegram_bot.run_sleep(self.engine, self.gemini, self.llm_config, self.session_id)
        if "Archive:" not in summary:
            raise ConformanceError(f"telegram-local sleep returned unexpected summary:\n{summary}")
        if re.search(r"\b[1-9]\d* failed\b", summary):
            raise ConformanceError(f"telegram-local sleep had failed fidelity tasks:\n{summary}")
        archives = self._completed_archives()
        if not archives:
            raise ConformanceError("telegram-local sleep produced no completed archive")
        return {"archive_entry": archives[-1], "summary": summary}

    def context_package(self, current_text: str) -> dict[str, Any]:
        return self.telegram_bot.context_package(self.engine, self.session_id, self.chat_id, current_text)

    def render_memory_view(self, current_text: str) -> str:
        package = self.context_package(current_text)
        return self.telegram_bot.chat_prompt(self.engine, package, current_text)

    def _completed_archives(self) -> list[dict[str, Any]]:
        archives = self.telegram_bot.complete_archives()
        return [archive for archive in archives if archive.get("source_session_id") == self.session_id]


def run_direct(keep_runtime: bool) -> DriverResult:
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_host_conformance_"))
    driver = DirectHostDriver(runtime)
    try:
        driver.send_user_message("Мене звати Микита.")
        driver.send_user_message("У мене є кішка Іржа.")
        driver.send_user_message("Я люблю космос і хочу, щоб це пам'яталось без Telegram.")
        outcome = driver.run_sleep()
        assert_sleep_outcome(outcome)
        view = driver.render_memory_view("Що ти пам'ятаєш про Іржу?")
        assert_memory_view(view)
        package = driver.context_package("Що ти пам'ятаєш про Іржу?")
        assert_core(package)
        return DriverResult(
            runtime_dir=runtime,
            archive_id=outcome["archive_entry"]["archive_id"],
            memory_unit_count=len(outcome["archive_entry"].get("memory_units", [])),
            core_fact_count=len(package.get("core_facts", [])),
        )
    finally:
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def run_telegram_local(keep_runtime: bool) -> DriverResult:
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_telegram_local_conformance_"))
    driver = TelegramLocalHostDriver(runtime)
    try:
        driver.send_user_message("Мене звати Микита.")
        driver.send_user_message("У мене є кішка Іржа.")
        driver.send_user_message("Я люблю космос і хочу, щоб Telegram host не мав власної memory logic.")
        outcome = driver.run_sleep()
        archive = outcome["archive_entry"]
        assert_sleep_archive(archive)
        view = driver.render_memory_view("Що ти пам'ятаєш про Іржу?")
        assert_memory_view(view)
        package = driver.context_package("Що ти пам'ятаєш про Іржу?")
        assert_core(package)
        return DriverResult(
            runtime_dir=runtime,
            archive_id=archive["archive_id"],
            memory_unit_count=len(archive.get("memory_units", [])),
            core_fact_count=len(package.get("core_facts", [])),
        )
    finally:
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def run_godot_headless(keep_runtime: bool, godot_bin: str | None) -> DriverResult:
    executable = find_godot_binary(godot_bin)
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_godot_headless_conformance_"))
    project_dir = runtime / "project"
    driver_runtime = runtime / "runtime"
    shutil.copytree(GODOT_HEADLESS_DIR, project_dir)
    bin_dir = project_dir / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(build_godot_extension(), bin_dir / godot_extension_filename())

    try:
        completed = subprocess.run(
            [
                executable,
                "--headless",
                "--path",
                str(project_dir),
                "--script",
                "res://test_runner.gd",
                "--",
                "--runtime-dir",
                str(driver_runtime),
            ],
            cwd=project_dir,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=120,
        )
        if completed.returncode != 0:
            raise ConformanceError(
                f"godot-headless exited with {completed.returncode}:\n{completed.stdout}"
            )
        if "HOST CONFORMANCE PASSED" not in completed.stdout:
            raise ConformanceError(f"godot-headless did not report success:\n{completed.stdout}")
        fields = parse_conformance_output(completed.stdout)
        return DriverResult(
            runtime_dir=runtime,
            archive_id=fields.get("archive_id", ""),
            memory_unit_count=int(fields.get("memory_units", "0")),
            core_fact_count=int(fields.get("core_facts", "0")),
        )
    finally:
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def parse_conformance_output(raw: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in raw.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        fields[key.strip()] = value.strip()
    return fields


def assert_sleep_outcome(outcome: dict[str, Any]) -> None:
    archive = outcome.get("archive_entry", {})
    assert_sleep_archive(archive)
    core_summary = outcome.get("core_summary", {})
    if core_summary.get("created", 0) < 2:
        raise ConformanceError(f"expected core bridge to create facts, got {core_summary!r}")


def assert_sleep_archive(archive: dict[str, Any]) -> None:
    if archive.get("status") != "complete":
        raise ConformanceError(f"sleep did not complete: {archive.get('status')!r}")
    units = archive.get("memory_units", [])
    if len(units) < 2:
        raise ConformanceError(f"expected at least two memory units, got {len(units)}")


def assert_memory_view(view: str) -> None:
    required = [
        "<memory_context>",
        "<core_memory>",
        "<long_memory>",
        "<short_memory>",
        "<current_user_message>",
        "Що ти пам'ятаєш про Іржу?",
    ]
    for marker in required:
        if marker not in view:
            raise ConformanceError(f"memory view missing {marker!r}\n{view}")
    if "user: Що ти пам'ятаєш про Іржу?" in view:
        raise ConformanceError("current user message leaked into short_memory duplicate")


def assert_core(package: dict[str, Any]) -> None:
    texts = "\n".join(str(fact.get("text", "")) for fact in package.get("core_facts", []))
    if "Микита" not in texts:
        raise ConformanceError(f"core facts do not include user name:\n{texts}")
    if "Іржа" not in texts:
        raise ConformanceError(f"core facts do not include Irzha:\n{texts}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Run host conformance scenarios.")
    parser.add_argument("--host", choices=["direct", "telegram-local", "godot-headless"], default="direct")
    parser.add_argument("--godot-bin", help="Godot executable for --host godot-headless")
    parser.add_argument("--keep-runtime", action="store_true")
    args = parser.parse_args()

    try:
        if args.host == "direct":
            result = run_direct(args.keep_runtime)
        elif args.host == "telegram-local":
            result = run_telegram_local(args.keep_runtime)
        elif args.host == "godot-headless":
            result = run_godot_headless(args.keep_runtime, args.godot_bin)
        else:
            raise ConformanceError(f"unsupported host: {args.host}")
    except Exception as err:
        print(f"HOST CONFORMANCE FAILED: {type(err).__name__}: {err}", file=sys.stderr)
        return 1

    print("HOST CONFORMANCE PASSED")
    print(f"host={args.host}")
    print(f"archive_id={result.archive_id}")
    print(f"memory_units={result.memory_unit_count}")
    print(f"core_facts={result.core_fact_count}")
    if args.keep_runtime:
        print(f"runtime_dir={result.runtime_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
