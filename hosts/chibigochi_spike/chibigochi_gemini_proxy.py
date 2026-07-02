"""Local Gemini-backed HTTP LLM executor for the Chibigochi Godot spike.

The Godot host stays thin: it sends chat/sleep/fidelity payloads over HTTP and
this process owns provider keys, model selection, prompt loading, and network
I/O. Secrets are read from the existing gitignored Telegram/Gemini cache unless
overridden by environment variables.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
PROMPTS_DIR = ROOT / "prompts"
HOST_DIR = Path(__file__).resolve().parent
DEFAULT_SECRETS_PATH = (
    ROOT / "hosts" / "telegram_gemini_bot" / "runtime" / "state" / "secrets.local.json"
)
DEFAULT_CHAT_SYSTEM_PROMPT = HOST_DIR / "chibigochi_chat_system.md"
RUNTIME_DIR = HOST_DIR / "runtime"
LOG_DIR = RUNTIME_DIR / "logs"
PROXY_LOG_PATH = LOG_DIR / "chibigochi_gemini_proxy.log"
TOKEN_USAGE_PATH = LOG_DIR / "chibigochi_gemini_proxy_token_usage.jsonl"
GEMINI_API = "https://generativelanguage.googleapis.com/v1beta"

DEFAULT_REASONING_MODEL = "gemini-2.5-pro"
DEFAULT_BALANCED_MODEL = "gemini-2.5-flash"
DEFAULT_FAST_MODEL = "gemini-2.5-flash-lite"
DEFAULT_CHAT_ROLE = "balanced"

INNER_THOUGHT_SYSTEM = (
    "Ти — Чібі, чібі-дівчина, що живе у квартирі-студії. Тобі дають твій поточний "
    "фізичний/емоційний стан і де ти стоїш. Сформулюй ОДНУ коротку природну думку "
    "від першої особи — чого тобі ЗАРАЗ хочеться і як ти почуваєшся, як справжня "
    "людина. НЕ згадуй шкали, цифри чи назви станів (не «ситість низька», а «дуже "
    "хочеться їсти»). Людина не думає категоріями шкал — вона одразу хоче те, що "
    "покращить стан. 1-2 короткі речення, українською."
)


@dataclass(frozen=True)
class ModelSelection:
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
        raise ValueError(f"unknown model role: {role_hint!r}")

    def chat_model(self) -> ModelSelection:
        return self.for_role(self.chat_role)


@dataclass(frozen=True)
class GeminiTextResponse:
    text: str
    usage: dict[str, int | None]
    model: str


class GeminiApiError(RuntimeError):
    def __init__(self, model: str, message: str, status_code: int | None = None) -> None:
        super().__init__(message)
        self.model = model
        self.status_code = status_code


class GeminiNoCandidatesError(RuntimeError):
    def __init__(self, model: str, result: dict[str, Any]) -> None:
        self.model = model
        self.result = result
        feedback = result.get("promptFeedback")
        self.block_reason = feedback.get("blockReason") if isinstance(feedback, dict) else None
        self.usage = gemini_usage_metadata(result)
        super().__init__(f"Gemini {model} returned no candidates")


class GeminiClient:
    def __init__(self, api_key: str, timeout_seconds: float) -> None:
        self.api_key = api_key
        self.timeout_seconds = timeout_seconds

    def validate_key(self) -> None:
        request = urllib.request.Request(
            f"{GEMINI_API}/models",
            headers={"x-goog-api-key": self.api_key},
            method="GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout_seconds) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as err:
            body = err.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"Gemini key validation failed: HTTP {err.code}: {body}") from err
        except urllib.error.URLError as err:
            raise RuntimeError(f"Gemini key validation failed: {err}") from err
        if not isinstance(payload.get("models"), list):
            raise RuntimeError("Gemini key validation returned an unexpected payload")

    def generate_text(
        self,
        model: str,
        system_instruction: str,
        prompt: str,
        response_mime_type: str | None,
        operation: str,
        model_role: str | None,
    ) -> GeminiTextResponse:
        url_model = urllib.parse.quote(model, safe="")
        url = f"{GEMINI_API}/models/{url_model}:generateContent"
        payload: dict[str, Any] = {
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
                "Content-Type": "application/json; charset=utf-8",
                "x-goog-api-key": self.api_key,
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout_seconds) as response:
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
        text = extract_gemini_text(result, model)
        log_token_usage(
            operation=operation,
            model=model,
            model_role=model_role,
            usage=usage,
            prompt=prompt,
            output=text,
            response_mime_type=response_mime_type,
        )
        return GeminiTextResponse(text=text, usage=usage, model=model)


class ChibigochiGeminiProxy:
    def __init__(
        self,
        gemini: GeminiClient,
        llm_config: HostLlmConfig,
        chat_system_prompt_path: Path,
    ) -> None:
        self.gemini = gemini
        self.llm_config = llm_config
        self.chat_system_prompt_path = chat_system_prompt_path

    def handle_payload(self, payload: dict[str, Any]) -> dict[str, Any]:
        operation = clean_string(payload.get("operation"))
        if operation == "chat_reply":
            return self._chat_reply(payload)
        if operation == "memory_request":
            return self._memory_request(payload)
        if operation == "memory_fidelity_pass":
            return self._memory_fidelity_pass(payload)
        if operation == "inner_thought":
            return self._inner_thought(payload)
        return {"error": f"unsupported operation: {operation!r}"}

    def _inner_thought(self, payload: dict[str, Any]) -> dict[str, Any]:
        state = clean_string(payload.get("input_text"))
        response = self.gemini.generate_text(
            model=self.llm_config.for_role("balanced").model,
            system_instruction=INNER_THOUGHT_SYSTEM,
            prompt=state,
            response_mime_type=None,
            operation="chibigochi_inner_thought",
            model_role="balanced",
        )
        return {"text": response.text}

    def _chat_reply(self, payload: dict[str, Any]) -> dict[str, Any]:
        text = clean_string(payload.get("input_text"))
        memory_view = clean_string(payload.get("memory_view"))
        prompt = (
            f"{memory_view}\n\n"
            "<current_player_message>\n"
            f"{text}\n"
            "</current_player_message>\n\n"
            "<assistant_response_slot>\n"
        )
        selection = self.llm_config.chat_model()
        response = self.gemini.generate_text(
            model=selection.model,
            system_instruction=self.chat_system_prompt_path.read_text(encoding="utf-8").strip(),
            prompt=prompt,
            response_mime_type=None,
            operation="chibigochi_chat_reply",
            model_role=self.llm_config.chat_role,
        )
        return {"text": response.text}

    def _memory_request(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = payload.get("request")
        if not isinstance(request, dict):
            return {"error": "memory_request payload missed request"}
        return self._execute_llm_request(request)

    def _memory_fidelity_pass(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = payload.get("request")
        if not isinstance(request, dict):
            return {"error": "memory_fidelity_pass payload missed request"}
        return self._execute_llm_request(request)

    def _execute_llm_request(self, request: dict[str, Any]) -> dict[str, Any]:
        request_id = clean_string(request.get("request_id"))
        prompt_id = clean_string(request.get("prompt_id"))
        role_hint = clean_string(request.get("role_hint")) or "balanced"
        try:
            prompt_path = PROMPTS_DIR / f"{prompt_id}.md"
            prompt_text = prompt_path.read_text(encoding="utf-8")
            selection = self.llm_config.for_role(role_hint)
            prompt_payload = json.dumps(request.get("prompt_inputs", {}), ensure_ascii=False, indent=2)
            response = self.gemini.generate_text(
                model=selection.model,
                system_instruction=prompt_text,
                prompt=prompt_payload,
                response_mime_type=response_mime_type_for_prompt(prompt_id),
                operation=prompt_id,
                model_role=role_hint,
            )
            return {"status": "ok", "request_id": request_id, "text": response.text}
        except GeminiNoCandidatesError as err:
            kind = "provider_blocked" if err.block_reason else "other"
            return {"status": "err", "request_id": request_id, "kind": kind, "detail": str(err)}
        except GeminiApiError as err:
            kind = "transport" if err.status_code in (None, 408, 409, 429, 500, 502, 503, 504) else "other"
            return {"status": "err", "request_id": request_id, "kind": kind, "detail": str(err)}
        except Exception as err:  # keep the Godot bridge alive; the engine owns fail-soft.
            return {
                "status": "err",
                "request_id": request_id,
                "kind": "other",
                "detail": f"{type(err).__name__}: {err}",
            }


class BridgeServer(ThreadingHTTPServer):
    def __init__(self, server_address: tuple[str, int], proxy: ChibigochiGeminiProxy) -> None:
        super().__init__(server_address, BridgeHandler)
        self.proxy = proxy


class BridgeHandler(BaseHTTPRequestHandler):
    server: BridgeServer

    def do_POST(self) -> None:  # noqa: N802 - stdlib API
        try:
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length).decode("utf-8"))
            if not isinstance(payload, dict):
                raise ValueError("request body must be a JSON object")
            response = self.server.proxy.handle_payload(payload)
            status = 200 if "error" not in response else 400
        except Exception as err:
            response = {"error": f"{type(err).__name__}: {err}"}
            status = 500
        raw = json.dumps(response, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002 - stdlib API
        log_line("http " + format % args)


def response_mime_type_for_prompt(prompt_id: str) -> str | None:
    if prompt_id == "sleep_consolidator":
        return None
    return "application/json"


def load_secrets(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"secrets cache not found: {path}")
    payload = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(payload, dict):
        raise ValueError(f"secrets cache is not a JSON object: {path}")
    return payload


def llm_config_from_sources(secrets: dict[str, Any]) -> HostLlmConfig:
    import os

    chat_role = os.environ.get("MEMORY_BOT_CHAT_ROLE") or clean_string(secrets.get("chat_role"))
    if chat_role not in {"reasoning", "balanced", "fast"}:
        chat_role = DEFAULT_CHAT_ROLE
    return HostLlmConfig(
        reasoning=ModelSelection(
            os.environ.get("GEMINI_REASONING_MODEL")
            or clean_string(secrets.get("reasoning_model"))
            or DEFAULT_REASONING_MODEL
        ),
        balanced=ModelSelection(
            os.environ.get("GEMINI_BALANCED_MODEL")
            or clean_string(secrets.get("balanced_model"))
            or DEFAULT_BALANCED_MODEL
        ),
        fast=ModelSelection(
            os.environ.get("GEMINI_FAST_MODEL")
            or clean_string(secrets.get("fast_model"))
            or DEFAULT_FAST_MODEL
        ),
        chat_role=chat_role,
    )


def api_key_from_sources(secrets: dict[str, Any]) -> str:
    import os

    key = os.environ.get("GEMINI_API_KEY") or clean_string(secrets.get("gemini_api_key"))
    if not key:
        raise RuntimeError("Gemini API key not found in GEMINI_API_KEY or secrets cache")
    return key


def make_proxy(args: argparse.Namespace) -> ChibigochiGeminiProxy:
    secrets_path = Path(args.secrets_path).resolve()
    secrets = load_secrets(secrets_path)
    api_key = api_key_from_sources(secrets)
    llm_config = llm_config_from_sources(secrets)
    gemini = GeminiClient(api_key=api_key, timeout_seconds=args.timeout)
    if args.validate_key:
        gemini.validate_key()
    log_line(
        "starting Chibigochi Gemini proxy "
        f"key={secret_fingerprint(api_key)} "
        f"reasoning={llm_config.reasoning.model} "
        f"balanced={llm_config.balanced.model} "
        f"fast={llm_config.fast.model} "
        f"chat_role={llm_config.chat_role}"
    )
    return ChibigochiGeminiProxy(
        gemini=gemini,
        llm_config=llm_config,
        chat_system_prompt_path=Path(args.chat_system_prompt).resolve(),
    )


def run_server(args: argparse.Namespace) -> None:
    proxy = make_proxy(args)
    server = BridgeServer((args.host, args.port), proxy)
    endpoint = f"http://{args.host}:{server.server_port}/llm"
    print(f"Chibigochi Gemini proxy listening at {endpoint}")
    print("Press Ctrl+C to stop.")
    log_line(f"listening endpoint={endpoint}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopping proxy.")
    finally:
        server.server_close()


def run_conformance(args: argparse.Namespace) -> int:
    proxy = make_proxy(args)
    server = BridgeServer((args.host, 0), proxy)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    endpoint = f"http://{args.host}:{server.server_port}/llm"
    log_line(f"running Chibigochi Gemini bridge conformance endpoint={endpoint}")
    try:
        module = load_host_conformance_module()
        result = module.run_godot_script(
            keep_runtime=args.keep_runtime,
            godot_bin=args.godot_bin,
            project_source=ROOT / "hosts" / "chibigochi_spike",
            script="res://llm_bridge_runner.gd",
            success_marker="CHIBIGOCHI LLM BRIDGE PASSED",
            script_args=["--llm-endpoint", endpoint],
        )
        print("HOST CONFORMANCE PASSED")
        print("host=chibigochi-gemini-bridge")
        print(f"archive_id={result.archive_id}")
        print(f"memory_units={result.memory_unit_count}")
        print(f"core_facts={result.core_fact_count}")
        if args.keep_runtime:
            print(f"runtime_dir={result.runtime_dir}")
        return 0
    except Exception as err:
        print(f"HOST CONFORMANCE FAILED: {type(err).__name__}: {err}", file=sys.stderr)
        return 1
    finally:
        server.shutdown()
        server.server_close()


def load_host_conformance_module() -> Any:
    path = ROOT / "tests" / "host_conformance" / "host_conformance.py"
    spec = importlib.util.spec_from_file_location("layers_memory_host_conformance", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load host conformance module: {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def clean_string(value: Any) -> str:
    return value.strip() if isinstance(value, str) else ""


def now_rfc3339() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def secret_fingerprint(value: str) -> str:
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()[:12]
    return f"len={len(value)} sha256_12={digest}"


def estimate_tokens(text: str) -> int:
    return max(1, (len(text) + 1) // 2) if text else 0


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
        "prompt_tokens": metadata.get("promptTokenCount")
        if isinstance(metadata.get("promptTokenCount"), int)
        else None,
        "output_tokens": metadata.get("candidatesTokenCount")
        if isinstance(metadata.get("candidatesTokenCount"), int)
        else None,
        "total_tokens": metadata.get("totalTokenCount")
        if isinstance(metadata.get("totalTokenCount"), int)
        else None,
        "thoughts_tokens": metadata.get("thoughtsTokenCount")
        if isinstance(metadata.get("thoughtsTokenCount"), int)
        else None,
    }


def extract_gemini_text(result: dict[str, Any], model: str) -> str:
    candidates = result.get("candidates") or []
    if not candidates:
        raise GeminiNoCandidatesError(model, result)
    parts = candidates[0].get("content", {}).get("parts") or []
    texts = [part.get("text", "") for part in parts if isinstance(part, dict) and part.get("text")]
    if not texts:
        raise GeminiNoCandidatesError(model, result)
    return "\n".join(texts).strip()


def log_line(message: str) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    line = f"{now_rfc3339()} {message}"
    with PROXY_LOG_PATH.open("a", encoding="utf-8") as file:
        file.write(line + "\n")


def log_token_usage(
    operation: str,
    model: str,
    model_role: str | None,
    usage: dict[str, int | None],
    prompt: str,
    output: str,
    response_mime_type: str | None,
) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    record = {
        "timestamp": now_rfc3339(),
        "kind": "chibigochi_gemini_token_usage",
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
    with TOKEN_USAGE_PATH.open("a", encoding="utf-8") as file:
        file.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
    log_line(
        "token_usage "
        f"operation={operation} role={model_role or '-'} model={model} "
        f"prompt={usage.get('prompt_tokens')} output={usage.get('output_tokens')} "
        f"total={usage.get('total_tokens')} est_prompt={record['estimated_prompt_tokens']}"
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run a Gemini HTTP proxy for Chibigochi.")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--secrets-path", default=str(DEFAULT_SECRETS_PATH))
    parser.add_argument("--chat-system-prompt", default=str(DEFAULT_CHAT_SYSTEM_PROMPT))
    parser.add_argument("--validate-key", action="store_true")
    parser.add_argument(
        "--run-conformance",
        action="store_true",
        help="Start a temporary proxy and run the Godot Chibigochi bridge scenario.",
    )
    parser.add_argument("--godot-bin", help="Godot executable for --run-conformance")
    parser.add_argument("--keep-runtime", action="store_true")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.run_conformance:
        return run_conformance(args)
    run_server(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
