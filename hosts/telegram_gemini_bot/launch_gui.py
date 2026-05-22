"""GUI launcher for the Telegram Gemini Memory Bot.

Use this when the classic Windows PowerShell window does not accept paste.
Secrets are passed to `bot.py` through environment variables. The launcher can
cache them locally under runtime/state, which is ignored by git.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tkinter as tk
from pathlib import Path
from tkinter import messagebox


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[1]
PYTHON_EXE = ROOT / "crates" / "python_adapter" / ".venv" / "Scripts" / "python.exe"
BOT_PY = SCRIPT_DIR / "bot.py"
STATE_DIR = SCRIPT_DIR / "runtime" / "state"
SECRETS_CACHE_PATH = STATE_DIR / "secrets.local.json"


DEFAULT_REASONING_MODEL = "gemini-2.5-pro"
DEFAULT_BALANCED_MODEL = "gemini-2.5-flash"
DEFAULT_FAST_MODEL = "gemini-2.5-flash-lite"
DEFAULT_CHAT_ROLE = "balanced"


def load_cache() -> dict[str, str]:
    if not SECRETS_CACHE_PATH.exists():
        return {}
    try:
        payload = json.loads(SECRETS_CACHE_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(payload, dict):
        return {}
    return {key: value for key, value in payload.items() if isinstance(value, str)}


def save_cache(values: dict[str, str]) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    payload = {"schema_version": "telegram_gemini_bot_secrets_cache.v1", **values}
    tmp_path = SECRETS_CACHE_PATH.with_suffix(".json.tmp")
    tmp_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
    tmp_path.replace(SECRETS_CACHE_PATH)


def clear_cache() -> None:
    try:
        SECRETS_CACHE_PATH.unlink()
    except FileNotFoundError:
        pass


def main() -> None:
    cache = load_cache()

    app = tk.Tk()
    app.title("Memory Bot Launcher")
    app.geometry("640x430")
    app.resizable(False, False)

    token_var = tk.StringVar(value=cache.get("telegram_token", ""))
    key_var = tk.StringVar(value=cache.get("gemini_api_key", ""))
    reasoning_var = tk.StringVar(value=cache.get("reasoning_model", DEFAULT_REASONING_MODEL))
    balanced_var = tk.StringVar(value=cache.get("balanced_model", DEFAULT_BALANCED_MODEL))
    fast_var = tk.StringVar(value=cache.get("fast_model", DEFAULT_FAST_MODEL))
    chat_role_var = tk.StringVar(value=cache.get("chat_role", DEFAULT_CHAT_ROLE))
    remember_var = tk.BooleanVar(value=True)

    def add_row(row: int, label: str, variable: tk.StringVar, show: str | None = None) -> None:
        tk.Label(app, text=label, anchor="w").grid(row=row, column=0, padx=12, pady=6, sticky="w")
        entry = tk.Entry(app, textvariable=variable, show=show, width=52)
        entry.grid(row=row, column=1, padx=12, pady=6, sticky="we")

    tk.Label(
        app,
        text=(
            "Paste your Telegram token and Gemini API key here. "
            "They can be cached locally in runtime/state/secrets.local.json."
        ),
        anchor="w",
        justify="left",
        wraplength=590,
    ).grid(row=0, column=0, columnspan=2, padx=12, pady=(12, 8), sticky="w")

    add_row(1, "Telegram token", token_var, show="*")
    add_row(2, "Gemini API key", key_var, show="*")
    add_row(3, "reasoning model", reasoning_var)
    add_row(4, "balanced model", balanced_var)
    add_row(5, "fast model", fast_var)
    add_row(6, "chat reply role", chat_role_var)

    tk.Checkbutton(
        app,
        text="Remember locally (ignored by git, stored as local plaintext)",
        variable=remember_var,
        anchor="w",
    ).grid(row=7, column=1, padx=12, pady=(4, 6), sticky="w")

    def clear_saved_values() -> None:
        clear_cache()
        token_var.set("")
        key_var.set("")
        messagebox.showinfo("Cache cleared", f"Deleted local cache:\n{SECRETS_CACHE_PATH}")

    def start_bot() -> None:
        token = token_var.get().strip()
        key = key_var.get().strip()
        if not token or not key:
            messagebox.showerror("Missing values", "Telegram token and Gemini API key are required.")
            return
        if token == key:
            messagebox.showerror(
                "Tokens look identical",
                "Telegram token and Gemini API key are identical. Paste the Telegram token in the first field and the Google/Gemini API key in the second field.",
            )
            return

        if not PYTHON_EXE.exists():
            messagebox.showerror(
                "Python adapter venv not found",
                f"Expected Python at:\n{PYTHON_EXE}\n\nRun run.ps1 once or rebuild the adapter venv.",
            )
            return

        env = os.environ.copy()
        env["TELEGRAM_BOT_TOKEN"] = token
        env["GEMINI_API_KEY"] = key
        env["GEMINI_REASONING_MODEL"] = reasoning_var.get().strip() or DEFAULT_REASONING_MODEL
        env["GEMINI_BALANCED_MODEL"] = balanced_var.get().strip() or DEFAULT_BALANCED_MODEL
        env["GEMINI_FAST_MODEL"] = fast_var.get().strip() or DEFAULT_FAST_MODEL
        env["MEMORY_BOT_CHAT_ROLE"] = chat_role_var.get().strip() or DEFAULT_CHAT_ROLE
        env["MEMORY_BOT_NONINTERACTIVE"] = "1"
        env["MEMORY_BOT_KEEP_CONSOLE_OPEN"] = "1"

        if remember_var.get():
            save_cache(
                {
                    "telegram_token": token,
                    "gemini_api_key": key,
                    "reasoning_model": env["GEMINI_REASONING_MODEL"],
                    "balanced_model": env["GEMINI_BALANCED_MODEL"],
                    "fast_model": env["GEMINI_FAST_MODEL"],
                    "chat_role": env["MEMORY_BOT_CHAT_ROLE"],
                }
            )

        creation_flags = getattr(subprocess, "CREATE_NEW_CONSOLE", 0)
        subprocess.Popen(
            [
                "powershell.exe",
                "-NoExit",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                f"& '{PYTHON_EXE}' '{BOT_PY}'; Write-Host ''; Write-Host 'Bot process exited. Review the error above or check runtime logs.'",
            ],
            cwd=str(ROOT),
            env=env,
            creationflags=creation_flags,
        )
        messagebox.showinfo("Started", "Bot started in a separate console window.")
        app.destroy()

    tk.Button(app, text="Start bot", command=start_bot, width=18).grid(
        row=8, column=1, padx=12, pady=18, sticky="e"
    )
    tk.Button(app, text="Clear saved keys", command=clear_saved_values, width=18).grid(
        row=8, column=0, padx=12, pady=18, sticky="w"
    )

    app.grid_columnconfigure(1, weight=1)
    app.mainloop()


if __name__ == "__main__":
    main()
