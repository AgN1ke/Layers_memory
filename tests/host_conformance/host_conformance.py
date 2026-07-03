"""Host conformance runner for Memory Engine v0.3.

The direct driver is the deterministic baseline: it uses the public Python
adapter surface, fakes LLM responses, and asserts engine state. Future Telegram
and Godot drivers should expose the same high-level operations and pass the
same scenario without owning memory policy.
"""

from __future__ import annotations

import argparse
import http.server
import importlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
TELEGRAM_HOST_DIR = ROOT / "hosts" / "telegram_gemini_bot"
GODOT_HEADLESS_DIR = ROOT / "hosts" / "godot_headless"
CHIBIGOCHI_SPIKE_DIR = ROOT / "hosts" / "chibigochi_spike"
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import memory_engine


SESSION_ID = "host_conformance_direct"
CORE_SCOPE = SESSION_ID
VECTOR_DIM = 384
VECTOR_MODEL_ID = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"


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


def fake_vector(hot_index: int, dim: int = VECTOR_DIM) -> list[float]:
    if hot_index < 0 or hot_index >= dim:
        raise ConformanceError(f"fake vector hot index out of range: {hot_index}")
    vector = [0.0] * dim
    vector[hot_index] = 1.0
    return vector


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
        *discover_repo_godot_binaries(),
        *discover_temp_godot_binaries(),
        shutil.which("godot4"),
        shutil.which("godot"),
    ]
    for candidate in candidates:
        if candidate:
            return candidate
    raise ConformanceError(
        "Godot executable not found. Set GODOT_BIN or pass --godot-bin for --host godot-headless."
    )


def discover_repo_godot_binaries() -> list[str]:
    tools_root = ROOT / "tools" / "godot"
    if not tools_root.exists():
        return []
    binaries = [path for path in tools_root.glob("*.exe") if "godot" in path.name.lower()]
    return [str(path) for path in sorted(binaries, key=godot_binary_score)]


def discover_temp_godot_binaries() -> list[str]:
    temp_root = Path(tempfile.gettempdir()) / "godot_conformance"
    if not temp_root.exists():
        return []
    binaries = [path for path in temp_root.rglob("*.exe") if "godot" in path.name.lower()]
    return [str(path) for path in sorted(binaries, key=godot_binary_score)]


def godot_binary_score(path: Path) -> tuple[int, int, str]:
    name = path.name.lower()
    version_score = 0 if "4.6" in name else 1
    console_score = 0 if "console" in name else 1
    return (version_score, console_score, str(path))


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


def english_memory_units_for_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    units: list[dict[str, Any]] = []
    for event in events:
        text = event_text(event)
        source_id = event_id(event)
        if "My name is Mykyta" in text:
            units.append(
                {
                    "thesis": "Name -> the player is named Mykyta.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["name", "profile"],
                    "weight": 0.95,
                }
            )
        if "Irzha" in text or "cat" in text.lower():
            units.append(
                {
                    "thesis": "Cat -> the player has a cat named Irzha.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["pet", "personal_memory"],
                    "weight": 0.95,
                }
            )
        if "space" in text.lower():
            units.append(
                {
                    "thesis": "Space -> the player likes space.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["interest"],
                    "weight": 0.9,
                }
            )
        if "silver feather" in text.lower():
            units.append(
                {
                    "thesis": "Keepsake -> the player hid a silver feather under the old bridge.",
                    "source_event_ids": [source_id],
                    "evidence": text,
                    "tags": ["episode", "keepsake"],
                    "weight": 0.82,
                }
            )
    return units


def english_personal_signals_for_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    signals: list[dict[str, Any]] = []
    for event in events:
        text = event_text(event)
        source_id = event_id(event)
        if "My name is Mykyta" in text:
            signals.append(
                {
                    "text": "The player's name is Mykyta.",
                    "category": "name",
                    "confidence": 0.95,
                    "source_event_ids": [source_id],
                }
            )
        if "Irzha" in text or "cat" in text.lower():
            signals.append(
                {
                    "text": "The player has a cat named Irzha.",
                    "category": "pet",
                    "confidence": 0.95,
                    "source_event_ids": [source_id],
                }
            )
        if "space" in text.lower():
            signals.append(
                {
                    "text": "The player likes space.",
                    "category": "interest",
                    "confidence": 0.92,
                    "source_event_ids": [source_id],
                }
            )
    return signals


def english_emotional_markers_for_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    markers: list[dict[str, Any]] = []
    for event in events:
        text = event_text(event)
        if "Irzha" in text or "cat" in text.lower():
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


def sleep_events_from_request(request: dict[str, Any]) -> list[dict[str, Any]]:
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


def chibigochi_llm_proxy_response(payload: dict[str, Any]) -> dict[str, Any]:
    operation = payload.get("operation")
    if operation == "chat_reply":
        text = str(payload.get("input_text", ""))
        memory_view = str(payload.get("memory_view", ""))
        lower = text.lower()
        if "cat" in lower and "Irzha" in memory_view:
            return {"text": "I remember Irzha: your cat is part of my long-term memory."}
        if "name" in lower and "Mykyta" in memory_view:
            return {"text": "I remember your name is Mykyta."}
        return {"text": f"Chibigochi heard you: {text[:80]}"}

    if operation == "memory_request":
        request = payload.get("request")
        run = payload.get("run")
        if not isinstance(request, dict) or not isinstance(run, dict):
            raise ConformanceError("memory_request payload missed request/run")
        prompt_id = request.get("prompt_id")
        if prompt_id == "sleep_consolidator":
            return {
                "status": "ok",
                "request_id": request.get("request_id", ""),
                "text": (
                    "GIST: Chibigochi HTTP bridge learned the player's name, cat, and space interest.\n\n"
                    "The player introduced themselves as Mykyta, said they have a cat named Irzha, "
                    "and shared an interest in space through the HTTP LLM bridge path."
                ),
            }
        events = [event for event in sleep_events_from_request(request) if event_kind(event) == "user_message"]
        if not events:
            events = sleep_events_from_request(request)
        if prompt_id == "memory_unit_pass":
            response_payload = {
                "schema_version": "memory_units_result.v1",
                "archive_id": run.get("archive_id", ""),
                "memory_units": english_memory_units_for_events(events),
            }
        elif prompt_id == "sleep_emotional_pass":
            response_payload = {"emotional_markers": english_emotional_markers_for_events(events)}
        elif prompt_id == "sleep_topic_thread_pass":
            response_payload = {
                "topic_thread": [
                    {
                        "topic": "chibigochi_http_bridge",
                        "summary": "The player introduced durable facts through the HTTP LLM bridge.",
                        "source_event_ids": [event_id(event) for event in events[:3]],
                    }
                ]
            }
        elif prompt_id == "sleep_personal_signal_pass":
            response_payload = {"personal_signals": english_personal_signals_for_events(events)}
        elif prompt_id == "sleep_relational_pass":
            response_payload = {"relational_tone": None}
        else:
            raise ConformanceError(f"unexpected HTTP bridge prompt_id={prompt_id!r}")
        return {
            "status": "ok",
            "request_id": request.get("request_id", ""),
            "text": dumps(response_payload),
        }

    if operation == "memory_fidelity_pass":
        request = payload.get("request")
        if not isinstance(request, dict):
            raise ConformanceError("memory_fidelity_pass payload missed request")
        inputs = request.get("prompt_inputs") if isinstance(request.get("prompt_inputs"), dict) else {}
        evidence = inputs.get("evidence_pack") if isinstance(inputs.get("evidence_pack"), dict) else {}
        unit = inputs.get("memory_unit") if isinstance(inputs.get("memory_unit"), dict) else evidence
        return {
            "status": "ok",
            "request_id": request.get("request_id", ""),
            "text": dumps(
                {
                    "schema_version": "fidelity_review.v1",
                    "memory_unit_id": unit.get("memory_unit_id", ""),
                    "archive_id": unit.get("archive_id", ""),
                    "status": "valid",
                    "confidence": 0.95,
                    "explanation": "HTTP bridge fake validator accepts the source-backed unit.",
                    "revised_thesis": None,
                    "missing_detail": None,
                }
            ),
        }

    raise ConformanceError(f"unexpected HTTP bridge operation={operation!r}")


class ChibigochiLlmProxyHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self) -> None:  # noqa: N802 - http.server API
        try:
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length).decode("utf-8"))
            if not isinstance(payload, dict):
                raise ConformanceError("HTTP bridge payload is not an object")
            response = chibigochi_llm_proxy_response(payload)
            raw = dumps(response).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
        except Exception as err:
            raw = dumps({"error": f"{type(err).__name__}: {err}"}).encode("utf-8")
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002 - http.server API
        return


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


class EnglishDirectHostDriver(DirectHostDriver):
    def _response_for_request(self, run: dict[str, Any], request: dict[str, Any]) -> dict[str, Any]:
        prompt_id = request["prompt_id"]
        if prompt_id == "sleep_consolidator":
            return {
                "status": "ok",
                "request_id": request["request_id"],
                "text": (
                    "GIST: The player introduced stable profile facts and one old keepsake episode.\n\n"
                    "In the conformance scenario the player introduced themselves, mentioned their cat, "
                    "shared an interest in space, and described hiding a silver feather under an old bridge."
                ),
            }

        events = self._sleep_events(request)
        user_events = [event for event in events if event_kind(event) == "user_message"]
        if not user_events:
            user_events = events

        if prompt_id == "memory_unit_pass":
            payload = {
                "schema_version": "memory_units_result.v1",
                "archive_id": run["archive_id"],
                "memory_units": english_memory_units_for_events(user_events),
            }
        elif prompt_id == "sleep_emotional_pass":
            payload = {"emotional_markers": english_emotional_markers_for_events(user_events)}
        elif prompt_id == "sleep_topic_thread_pass":
            payload = {
                "topic_thread": [
                    {
                        "topic": "host_conformance",
                        "summary": "The player introduced durable facts and one old keepsake episode.",
                        "source_event_ids": [event_id(event) for event in user_events[:3]],
                    }
                ]
            }
        elif prompt_id == "sleep_personal_signal_pass":
            payload = {"personal_signals": english_personal_signals_for_events(user_events)}
        elif prompt_id == "sleep_relational_pass":
            payload = {"relational_tone": None}
        else:
            raise ConformanceError(f"unexpected LLM request prompt_id={prompt_id!r}")

        return {
            "status": "ok",
            "request_id": request["request_id"],
            "text": dumps(payload),
        }


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
        if operation == "chat_reply" and "silver feather" in prompt.lower():
            text = "I remember the silver feather under the old bridge."
        elif operation == "chat_reply":
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


def run_direct_vectors(keep_runtime: bool) -> DriverResult:
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_direct_vectors_conformance_"))
    driver = DirectHostDriver(runtime)
    try:
        state = loads(driver.engine.set_vector_scope(SESSION_ID, True, False))
        if state.get("status") not in {"building", "ready"}:
            raise ConformanceError(f"vector scope did not enable: {state!r}")

        driver.send_user_message("Мене звати Микита.")
        driver.send_user_message("У мене є кішка Іржа.")
        driver.send_user_message("Я люблю космос і хочу deep recall без Telegram.")
        outcome = driver.run_sleep()
        assert_sleep_outcome(outcome)

        requests = outcome.get("embedding_requests") or loads(
            driver.engine.pending_embedding_backfill(SESSION_ID)
        )
        if not requests:
            raise ConformanceError("direct-vectors produced no embedding requests")

        embedded_ids: list[str] = []
        for request in requests:
            embedded_ids.extend(submit_fake_embedding_request(driver.engine, request))
        if len(embedded_ids) < 2:
            raise ConformanceError(f"direct-vectors expected at least two embedded units, got {embedded_ids!r}")

        state = loads(driver.engine.vector_state(SESSION_ID))
        if state.get("status") != "ready":
            raise ConformanceError(f"vector scope not ready after fake embeddings: {state!r}")

        target_index = 1
        target_id = embedded_ids[target_index]
        result = loads(
            driver.engine.recall_deep(
                dumps(
                    {
                        "scope": SESSION_ID,
                        "query_vec": fake_vector(target_index),
                        "model_id": VECTOR_MODEL_ID,
                        "top_k": 1,
                        "min_sim": 0.9,
                        "now": "2026-06-10T11:00:00Z",
                    }
                )
            )
        )
        if not result.get("found"):
            raise ConformanceError(f"deep recall found nothing: {result!r}")
        hits = result.get("hits", [])
        if len(hits) != 1 or hits[0].get("memory_unit_id") != target_id:
            raise ConformanceError(f"deep recall did not return the vector target {target_id!r}: {result!r}")
        if hits[0].get("sim", 0.0) < 0.99:
            raise ConformanceError(f"deep recall similarity too low: {hits[0]!r}")

        disabled = loads(driver.engine.set_vector_scope(SESSION_ID, False, True))
        if disabled.get("status") != "disabled":
            raise ConformanceError(f"vector scope did not disable: {disabled!r}")
        disabled_result = loads(
            driver.engine.recall_deep(
                dumps(
                    {
                        "scope": SESSION_ID,
                        "query_vec": fake_vector(target_index),
                        "model_id": VECTOR_MODEL_ID,
                        "top_k": 1,
                        "min_sim": 0.9,
                    }
                )
            )
        )
        if disabled_result.get("found") or disabled_result.get("reason") != "disabled":
            raise ConformanceError(f"disabled deep recall did not report disabled: {disabled_result!r}")

        package = driver.context_package("Do you remember the companion animal?")
        return DriverResult(
            runtime_dir=runtime,
            archive_id=outcome["archive_entry"]["archive_id"],
            memory_unit_count=len(outcome["archive_entry"].get("memory_units", [])),
            core_fact_count=len(package.get("core_facts", [])),
        )
    finally:
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def run_direct_forced_recall(keep_runtime: bool) -> DriverResult:
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_forced_recall_conformance_"))
    driver = EnglishDirectHostDriver(runtime)
    try:
        state = loads(driver.engine.set_vector_scope(SESSION_ID, True, False))
        if state.get("status") not in {"building", "ready"}:
            raise ConformanceError(f"vector scope did not enable: {state!r}")

        driver.send_user_message("My name is Mykyta.")
        driver.send_user_message("My cat is named Irzha.")
        driver.send_user_message("I like space stories.")
        driver.send_user_message("During a rainy walk I hid a silver feather under the old bridge.")
        outcome = driver.run_sleep()
        assert_sleep_outcome(outcome)

        requests = outcome.get("embedding_requests") or loads(
            driver.engine.pending_embedding_backfill(SESSION_ID)
        )
        if not requests:
            raise ConformanceError("direct-forced-recall produced no embedding requests")

        topic_vectors: dict[str, int] = {}
        for request in requests:
            topic_vectors.update(submit_topic_fake_embedding_request(driver.engine, request))
        keepsake_index = topic_vectors.get("keepsake")
        if keepsake_index is None:
            raise ConformanceError(f"direct-forced-recall did not embed the keepsake unit: {topic_vectors!r}")

        visible = driver.context_package("Do you remember the keepsake from that rainy walk?")
        visible["core_facts"] = []
        visible["archive_relevant"] = []
        visible["session_recent"] = []
        visible["session_trace"] = []
        visible_view = driver.engine.render_memory_view(
            dumps(visible),
            "Do you remember the keepsake from that rainy walk?",
        )
        if "silver feather" in visible_view.lower():
            raise ConformanceError(f"minimal visible context already contained distant memory:\n{visible_view}")

        result = loads(
            driver.engine.recall_deep(
                dumps(
                    {
                        "scope": SESSION_ID,
                        "query_vec": fake_vector(keepsake_index),
                        "model_id": VECTOR_MODEL_ID,
                        "top_k": 1,
                        "min_sim": 0.9,
                        "now": "2026-06-10T11:00:00Z",
                    }
                )
            )
        )
        if not result.get("found"):
            raise ConformanceError(f"forced distant recall found nothing: {result!r}")
        hits = result.get("hits", [])
        if len(hits) != 1:
            raise ConformanceError(f"forced distant recall expected one scarce hit: {result!r}")
        thesis = str(hits[0].get("thesis", ""))
        if "silver feather" not in thesis.lower():
            raise ConformanceError(f"forced distant recall returned the wrong memory: {result!r}")

        reply = f"I remember this: {thesis}"
        if "silver feather" not in reply.lower():
            raise ConformanceError(f"forced distant recall reply did not use the recalled memory: {reply!r}")

        miss = loads(
            driver.engine.recall_deep(
                dumps(
                    {
                        "scope": SESSION_ID,
                        "query_vec": fake_vector(200),
                        "model_id": VECTOR_MODEL_ID,
                        "top_k": 1,
                        "min_sim": 0.9,
                    }
                )
            )
        )
        if miss.get("found") or miss.get("reason") != "below_threshold":
            raise ConformanceError(f"unrelated forced recall should miss cleanly: {miss!r}")

        return DriverResult(
            runtime_dir=runtime,
            archive_id=outcome["archive_entry"]["archive_id"],
            memory_unit_count=len(outcome["archive_entry"].get("memory_units", [])),
            core_fact_count=len(driver.context_package("What profile facts are known?").get("core_facts", [])),
        )
    finally:
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def submit_fake_embedding_request(engine: Any, request: dict[str, Any]) -> list[str]:
    embed_batch = request.get("prompt_inputs", {}).get("embed_batch", {})
    items = embed_batch.get("items", [])
    if not isinstance(items, list) or not items:
        raise ConformanceError(f"embedding request has no items: {request!r}")
    results = []
    ids = []
    for index, item in enumerate(items):
        memory_unit_id = item.get("memory_unit_id")
        if not isinstance(memory_unit_id, str) or not memory_unit_id:
            raise ConformanceError(f"embedding item has no memory_unit_id: {item!r}")
        ids.append(memory_unit_id)
        results.append({"memory_unit_id": memory_unit_id, "vector": fake_vector(index)})
    engine.resume_compute_embedding(
        request["task_id"],
        dumps(
            {
                "schema_version": "embed_batch_result.v1",
                "model_id": VECTOR_MODEL_ID,
                "dim": VECTOR_DIM,
                "results": results,
            }
        ),
    )
    return ids


def submit_topic_fake_embedding_request(engine: Any, request: dict[str, Any]) -> dict[str, int]:
    embed_batch = request.get("prompt_inputs", {}).get("embed_batch", {})
    items = embed_batch.get("items", [])
    if not isinstance(items, list) or not items:
        raise ConformanceError(f"embedding request has no items: {request!r}")

    topic_to_index = {"name": 3, "cat": 7, "space": 11, "keepsake": 17}
    seen_topics: dict[str, int] = {}
    results = []
    for item in items:
        memory_unit_id = item.get("memory_unit_id")
        text = str(item.get("text", ""))
        if not isinstance(memory_unit_id, str) or not memory_unit_id:
            raise ConformanceError(f"embedding item has no memory_unit_id: {item!r}")
        lower = text.lower()
        if "silver feather" in lower or "keepsake" in lower:
            topic = "keepsake"
        elif "irzha" in lower or "cat" in lower:
            topic = "cat"
        elif "space" in lower:
            topic = "space"
        elif "mykyta" in lower or "name" in lower:
            topic = "name"
        else:
            topic = f"other:{memory_unit_id}"
        hot_index = topic_to_index.get(topic, 31 + len(seen_topics))
        if hot_index >= VECTOR_DIM:
            raise ConformanceError(f"fake vector hot index overflow for {topic!r}")
        seen_topics[topic] = hot_index
        results.append({"memory_unit_id": memory_unit_id, "vector": fake_vector(hot_index)})

    engine.resume_compute_embedding(
        request["task_id"],
        dumps(
            {
                "schema_version": "embed_batch_result.v1",
                "model_id": VECTOR_MODEL_ID,
                "dim": VECTOR_DIM,
                "results": results,
            }
        ),
    )
    return seen_topics


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


def run_telegram_distant_gate(keep_runtime: bool) -> DriverResult:
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_telegram_distant_gate_"))
    visible_driver = TelegramLocalHostDriver(runtime / "visible")
    missing_driver = TelegramLocalHostDriver(runtime / "missing")
    disabled_driver = TelegramLocalHostDriver(runtime / "disabled")
    calls: list[str] = []
    original_recall = visible_driver.telegram_bot.recall_distant_memory
    original_scope_ready = visible_driver.telegram_bot.distant_recall_scope_ready

    def fake_recall_distant_memory(
        engine: Any,
        session_id: str,
        query_text: str,
        top_k: int = 5,
        min_sim: float = 0.0,
    ) -> dict[str, Any]:
        del engine, top_k, min_sim
        calls.append(f"{session_id}:{query_text}")
        return {
            "found": True,
            "reason": None,
            "memories": [
                {
                    "when": "2026-06-10T10:04:00Z",
                    "sim": 0.96,
                    "strength": "vivid",
                    "text": "Keepsake -> the player hid a silver feather under the old bridge.",
                }
            ],
            "raw": {
                "hits": [
                    {
                        "memory_unit_id": "mu_fake_silver_feather",
                        "archive_id": "archive_fake_keepsake",
                        "thesis": "Keepsake -> the player hid a silver feather under the old bridge.",
                        "created_at": "2026-06-10T10:04:00Z",
                        "sim": 0.96,
                        "score": 1.12,
                    }
                ]
            },
        }

    try:
        visible_driver.telegram_bot.recall_distant_memory = fake_recall_distant_memory
        visible_driver.telegram_bot.distant_recall_scope_ready = lambda engine, session_id: True
        if not visible_driver.telegram_bot.visible_memory_already_answers(
            "Пам'ятаєш про Іржу?",
            "<core_memory>\n- pet (0.95): У користувача є кішка Іржа.\n</core_memory>",
        ):
            raise ConformanceError("visible-memory gate missed a Ukrainian inflected Irzha query")
        visible_driver.send_user_message("/remember The player hid a silver feather under the old bridge.")
        visible_reply = visible_driver.send_user_message("Do you remember the silver feather?")
        if calls:
            raise ConformanceError(f"distant recall was called even though visible memory had the answer: {calls!r}")
        if "silver feather" not in visible_reply.lower():
            raise ConformanceError(f"visible-memory reply did not see the core memory: {visible_reply!r}")

        missing_reply = missing_driver.send_user_message("Do you remember the silver feather?")
        if len(calls) != 1:
            raise ConformanceError(f"distant recall was not called exactly once for missing memory: {calls!r}")
        if "silver feather" not in missing_reply.lower():
            raise ConformanceError(f"distant-memory reply did not use the recalled memory: {missing_reply!r}")

        visible_driver.telegram_bot.distant_recall_scope_ready = lambda engine, session_id: False
        disabled_reply = disabled_driver.send_user_message("Do you remember the blue lantern?")
        if len(calls) != 1:
            raise ConformanceError(f"distant recall was called while vector scope was not ready: {calls!r}")
        if "silver feather" in disabled_reply.lower():
            raise ConformanceError(f"disabled vector scope reply used distant memory: {disabled_reply!r}")

        package = missing_driver.context_package("What profile facts are known?")
        return DriverResult(
            runtime_dir=runtime,
            archive_id="telegram_distant_gate",
            memory_unit_count=0,
            core_fact_count=len(package.get("core_facts", [])),
        )
    finally:
        visible_driver.telegram_bot.recall_distant_memory = original_recall
        visible_driver.telegram_bot.distant_recall_scope_ready = original_scope_ready
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def run_godot_headless(keep_runtime: bool, godot_bin: str | None) -> DriverResult:
    return run_godot_script(
        keep_runtime=keep_runtime,
        godot_bin=godot_bin,
        project_source=GODOT_HEADLESS_DIR,
        script="res://test_runner.gd",
        success_marker="HOST CONFORMANCE PASSED",
    )


def run_chibigochi_spike(keep_runtime: bool, godot_bin: str | None) -> DriverResult:
    return run_godot_script(
        keep_runtime=keep_runtime,
        godot_bin=godot_bin,
        project_source=CHIBIGOCHI_SPIKE_DIR,
        script="res://spike_runner.gd",
        success_marker="CHIBIGOCHI SPIKE PASSED",
    )


def run_chibigochi_ui(keep_runtime: bool, godot_bin: str | None) -> DriverResult:
    return run_godot_script(
        keep_runtime=keep_runtime,
        godot_bin=godot_bin,
        project_source=CHIBIGOCHI_SPIKE_DIR,
        script="res://ui_runner.gd",
        success_marker="CHIBIGOCHI UI PASSED",
    )


def run_chibigochi_llm_bridge(keep_runtime: bool, godot_bin: str | None) -> DriverResult:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ChibigochiLlmProxyHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    endpoint = f"http://127.0.0.1:{server.server_port}/llm"
    try:
        return run_godot_script(
            keep_runtime=keep_runtime,
            godot_bin=godot_bin,
            project_source=CHIBIGOCHI_SPIKE_DIR,
            script="res://llm_bridge_runner.gd",
            success_marker="CHIBIGOCHI LLM BRIDGE PASSED",
            script_args=["--llm-endpoint", endpoint],
        )
    finally:
        server.shutdown()
        server.server_close()


def run_chibigochi_product_loop(keep_runtime: bool, godot_bin: str | None) -> DriverResult:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ChibigochiLlmProxyHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    endpoint = f"http://127.0.0.1:{server.server_port}/llm"
    try:
        return run_godot_script(
            keep_runtime=keep_runtime,
            godot_bin=godot_bin,
            project_source=CHIBIGOCHI_SPIKE_DIR,
            script="res://async_ui_runner.gd",
            success_marker="CHIBIGOCHI ASYNC UI PASSED",
            script_args=["--llm-endpoint", endpoint],
        )
    finally:
        server.shutdown()
        server.server_close()


def run_godot_script(
    keep_runtime: bool,
    godot_bin: str | None,
    project_source: Path,
    script: str,
    success_marker: str,
    script_args: list[str] | None = None,
) -> DriverResult:
    executable = find_godot_binary(godot_bin)
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_godot_headless_conformance_"))
    project_dir = runtime / "project"
    driver_runtime = runtime / "runtime"
    shutil.copytree(project_source, project_dir)
    bin_dir = project_dir / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(build_godot_extension(), bin_dir / godot_extension_filename())

    try:
        import_run = subprocess.run(
            [
                executable,
                "--headless",
                "--editor",
                "--quit",
                "--path",
                str(project_dir),
            ],
            cwd=project_dir,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=120,
        )
        import_output = import_run.stdout

        try:
            completed = subprocess.run(
                [
                    executable,
                    "--headless",
                    "--path",
                    str(project_dir),
                    "--script",
                    script,
                    "--",
                    "--runtime-dir",
                    str(driver_runtime),
                    *(script_args or []),
                ],
                cwd=project_dir,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=120,
            )
        except subprocess.TimeoutExpired as err:
            output = err.stdout or ""
            raise ConformanceError(
                f"Godot script timed out after {err.timeout}s:\n{output}\n\n"
                f"import pass exit={import_run.returncode}:\n{import_output}"
            ) from err
        if completed.returncode != 0:
            raise ConformanceError(
                "godot-headless exited with "
                f"{completed.returncode}:\n{completed.stdout}\n\n"
                f"import pass exit={import_run.returncode}:\n{import_output}"
            )
        if success_marker not in completed.stdout:
            raise ConformanceError(f"Godot script did not report success:\n{completed.stdout}")
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


MULTISPEAKER_SESSION = "host_conformance_multispeaker"


def run_direct_multispeaker(keep_runtime: bool) -> DriverResult:
    """Deterministic multi-speaker scenario over the public adapter surface.

    Proves that `speaker` survives the PyO3 JSON boundary, that the transcript
    is attributed by name, and that the Phase-1 gate keeps high-confidence
    gossip signals out of Core for multi-speaker sessions.
    """
    runtime = Path(tempfile.mkdtemp(prefix="memory_engine_multispeaker_conformance_"))
    engine = memory_engine.MemoryEngine(str(runtime), host_id="host_conformance_multispeaker")
    try:
        turns = [
            ("tg_101", "Жека", "Я нарешті купив мотоцикл!"),
            ("tg_202", "Антон", "А я на рибалці був, клювало на світанку."),
            ("tg_101", "Жека", "Завтра заберу мотоцикл із салону."),
        ]
        for index, (speaker_id, speaker_name, text) in enumerate(turns, start=1):
            engine.ingest(
                dumps(
                    {
                        "schema_version": "event.v1",
                        "type": "user_message",
                        "source": "group_chat",
                        "timestamp": f"2026-07-02T10:{index:02}:00.000Z",
                        "session_id": MULTISPEAKER_SESSION,
                        "payload": {"text": text},
                        "tags": ["group_chat"],
                        "theme": "group_chat",
                        "speaker": {"id": speaker_id, "name": speaker_name},
                        "importance_hint": "high",
                    }
                )
            )

        current_text = "Жека, коли забираєш мотоцикл?"
        live_package = loads(
            engine.core_context_package(
                dumps(
                    {
                        "schema_version": "core_context_request.v1",
                        "session_id": MULTISPEAKER_SESSION,
                        "domain_state": {"current_text": current_text},
                        "core_scope": MULTISPEAKER_SESSION,
                        "query_text": current_text,
                        "recall_limit": 5,
                        "session_recent_limit": 8,
                        "session_trace_event_limit": 20,
                        "include_core": True,
                    }
                )
            )
        )
        live_view = engine.render_memory_view(dumps(live_package), current_text)
        for marker in ["Жека: Я нарешті купив мотоцикл!", "Антон: А я на рибалці був"]:
            if marker not in live_view:
                raise ConformanceError(f"attributed memory view missing {marker!r}\n{live_view}")
        if "user: Я нарешті купив мотоцикл!" in live_view:
            raise ConformanceError("speaker event fell back to the legacy `user:` role")

        run = loads(engine.begin_sleep_run(MULTISPEAKER_SESSION))
        while True:
            step = loads(engine.next_sleep_batch(dumps(run)))
            run = step["run"]
            batch = step.get("batch")
            if not batch:
                break
            responses = [
                multispeaker_response_for_request(run, request)
                for request in batch["requests"]
            ]
            step = loads(engine.submit_sleep_batch(dumps(run), dumps(responses)))
            run = step["run"]
        outcome = loads(engine.finish_sleep_run(dumps(run)))

        archive = outcome.get("archive_entry", {})
        if archive.get("status") != "complete":
            raise ConformanceError(f"multi-speaker sleep did not complete: {archive.get('status')!r}")
        theses = "\n".join(
            str(unit.get("thesis", "")) for unit in archive.get("memory_units", [])
        )
        if "Жека" not in theses:
            raise ConformanceError(f"memory unit theses are not attributed:\n{theses}")

        core_summary = outcome.get("core_summary", {})
        if core_summary.get("created", 0) != 0:
            raise ConformanceError(
                f"multi-speaker gate failed: bridge created Core facts {core_summary!r}"
            )
        if core_summary.get("skipped", 0) < 1:
            raise ConformanceError(
                f"multi-speaker gate did not report skipped signals: {core_summary!r}"
            )
        if outcome.get("fidelity_requests"):
            raise ConformanceError(
                "no unit should be on the automatic Core path in a multi-speaker session"
            )

        package = loads(
            engine.core_context_package(
                dumps(
                    {
                        "schema_version": "core_context_request.v1",
                        "session_id": MULTISPEAKER_SESSION,
                        "domain_state": {"current_text": current_text},
                        "core_scope": MULTISPEAKER_SESSION,
                        "query_text": current_text,
                        "recall_limit": 5,
                        "session_recent_limit": 8,
                        "session_trace_event_limit": 20,
                        "include_core": True,
                    }
                )
            )
        )
        if package.get("core_facts"):
            raise ConformanceError(
                f"multi-speaker Core must stay empty without review: {package.get('core_facts')!r}"
            )
        view = engine.render_memory_view(dumps(package), current_text)
        for marker in ["<memory_context>", "<long_memory>", "Жека"]:
            if marker not in view:
                raise ConformanceError(f"post-sleep memory view missing {marker!r}\n{view}")

        return DriverResult(
            runtime_dir=runtime,
            archive_id=archive.get("archive_id", ""),
            memory_unit_count=len(archive.get("memory_units", [])),
            core_fact_count=len(package.get("core_facts", [])),
        )
    finally:
        if not keep_runtime:
            shutil.rmtree(runtime, ignore_errors=True)


def multispeaker_response_for_request(run: dict[str, Any], request: dict[str, Any]) -> dict[str, Any]:
    prompt_id = request["prompt_id"]
    if prompt_id == "sleep_consolidator":
        return {
            "status": "ok",
            "request_id": request["request_id"],
            "text": (
                "GIST: Жека купив мотоцикл, Антон розповів про рибалку.\n\n"
                "У груповому чаті переплелися дві теми: мотоцикл Жеки і рибалка Антона."
            ),
        }

    inputs = request.get("prompt_inputs", {})
    sleep_task = inputs.get("sleep_task", {}) if isinstance(inputs, dict) else {}
    events = sleep_task.get("events", []) if isinstance(sleep_task, dict) else []
    first_event_id = event_id(events[0]) if events else ""

    if prompt_id == "memory_unit_pass":
        payload: dict[str, Any] = {
            "schema_version": "memory_units_result.v1",
            "archive_id": run["archive_id"],
            "memory_units": [
                {
                    "thesis": "Мотоцикл -> Жека купив мотоцикл і забирає його завтра.",
                    "source_event_ids": [first_event_id],
                    "evidence": "Жека сам повідомив про покупку в чаті.",
                    "tags": ["group_chat"],
                    "weight": 0.6,
                },
                {
                    "thesis": "Рибалка -> Антон рибалив на світанку.",
                    "source_event_ids": [first_event_id],
                    "evidence": "Антон розповів про кльов на світанку.",
                    "tags": ["group_chat"],
                    "weight": 0.5,
                },
            ],
        }
    elif prompt_id == "sleep_emotional_pass":
        payload = {"emotional_markers": []}
    elif prompt_id == "sleep_topic_thread_pass":
        payload = {
            "topic_thread": [
                {
                    "topic": "group_chat",
                    "summary": "Мотоцикл Жеки і рибалка Антона.",
                    "source_event_ids": [first_event_id] if first_event_id else [],
                }
            ]
        }
    elif prompt_id == "sleep_personal_signal_pass":
        payload = {
            "personal_signals": [
                {
                    "text": "У Жеки є мотоцикл.",
                    "category": "vehicle",
                    "confidence": 0.95,
                    "source_event_ids": [first_event_id],
                }
            ]
        }
    elif prompt_id == "sleep_relational_pass":
        payload = {"relational_tone": None}
    else:
        raise ConformanceError(f"unexpected multi-speaker LLM request prompt_id={prompt_id!r}")

    return {"status": "ok", "request_id": request["request_id"], "text": dumps(payload)}


def main() -> int:
    parser = argparse.ArgumentParser(description="Run host conformance scenarios.")
    parser.add_argument(
        "--host",
        choices=[
            "direct",
            "direct-vectors",
            "direct-forced-recall",
            "direct-multispeaker",
            "telegram-local",
            "telegram-distant-gate",
            "godot-headless",
            "chibigochi-spike",
            "chibigochi-ui",
            "chibigochi-llm-bridge",
            "chibigochi-product-loop",
        ],
        default="direct",
    )
    parser.add_argument("--godot-bin", help="Godot executable for --host godot-headless")
    parser.add_argument("--keep-runtime", action="store_true")
    args = parser.parse_args()

    try:
        if args.host == "direct":
            result = run_direct(args.keep_runtime)
        elif args.host == "direct-vectors":
            result = run_direct_vectors(args.keep_runtime)
        elif args.host == "direct-forced-recall":
            result = run_direct_forced_recall(args.keep_runtime)
        elif args.host == "direct-multispeaker":
            result = run_direct_multispeaker(args.keep_runtime)
        elif args.host == "telegram-local":
            result = run_telegram_local(args.keep_runtime)
        elif args.host == "telegram-distant-gate":
            result = run_telegram_distant_gate(args.keep_runtime)
        elif args.host == "godot-headless":
            result = run_godot_headless(args.keep_runtime, args.godot_bin)
        elif args.host == "chibigochi-spike":
            result = run_chibigochi_spike(args.keep_runtime, args.godot_bin)
        elif args.host == "chibigochi-ui":
            result = run_chibigochi_ui(args.keep_runtime, args.godot_bin)
        elif args.host == "chibigochi-llm-bridge":
            result = run_chibigochi_llm_bridge(args.keep_runtime, args.godot_bin)
        elif args.host == "chibigochi-product-loop":
            result = run_chibigochi_product_loop(args.keep_runtime, args.godot_bin)
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
