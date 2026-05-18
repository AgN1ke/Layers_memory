"""GUI launcher for the Telegram Gemini Memory Bot.

Use this when the classic Windows PowerShell window does not accept paste.
Secrets are passed to `bot.py` through environment variables and are not
written to disk.
"""

from __future__ import annotations

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


DEFAULT_REASONING_MODEL = "gemini-2.5-pro"
DEFAULT_BALANCED_MODEL = "gemini-2.5-flash"
DEFAULT_FAST_MODEL = "gemini-2.5-flash-lite"
DEFAULT_CHAT_ROLE = "balanced"
DEFAULT_AUTO_SLEEP_AFTER_EVENTS = "50"


def main() -> None:
    app = tk.Tk()
    app.title("Memory Bot Launcher")
    app.geometry("560x390")
    app.resizable(False, False)

    token_var = tk.StringVar()
    key_var = tk.StringVar()
    reasoning_var = tk.StringVar(value=DEFAULT_REASONING_MODEL)
    balanced_var = tk.StringVar(value=DEFAULT_BALANCED_MODEL)
    fast_var = tk.StringVar(value=DEFAULT_FAST_MODEL)
    chat_role_var = tk.StringVar(value=DEFAULT_CHAT_ROLE)
    auto_sleep_var = tk.StringVar(value=DEFAULT_AUTO_SLEEP_AFTER_EVENTS)

    def add_row(row: int, label: str, variable: tk.StringVar, show: str | None = None) -> None:
        tk.Label(app, text=label, anchor="w").grid(row=row, column=0, padx=12, pady=6, sticky="w")
        entry = tk.Entry(app, textvariable=variable, show=show, width=52)
        entry.grid(row=row, column=1, padx=12, pady=6, sticky="we")

    tk.Label(
        app,
        text="Paste your Telegram token and Gemini API key here. They are not saved to files.",
        anchor="w",
        justify="left",
    ).grid(row=0, column=0, columnspan=2, padx=12, pady=(12, 8), sticky="w")

    add_row(1, "Telegram token", token_var, show="*")
    add_row(2, "Gemini API key", key_var, show="*")
    add_row(3, "reasoning model", reasoning_var)
    add_row(4, "balanced model", balanced_var)
    add_row(5, "fast model", fast_var)
    add_row(6, "chat reply role", chat_role_var)
    add_row(7, "auto-sleep events", auto_sleep_var)

    def start_bot() -> None:
        token = token_var.get().strip()
        key = key_var.get().strip()
        if not token or not key:
            messagebox.showerror("Missing values", "Telegram token and Gemini API key are required.")
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
        env["MEMORY_BOT_AUTO_SLEEP_AFTER_EVENTS"] = (
            auto_sleep_var.get().strip() or DEFAULT_AUTO_SLEEP_AFTER_EVENTS
        )
        env["MEMORY_BOT_NONINTERACTIVE"] = "1"

        creation_flags = getattr(subprocess, "CREATE_NEW_CONSOLE", 0)
        subprocess.Popen(
            [str(PYTHON_EXE), str(BOT_PY)],
            cwd=str(ROOT),
            env=env,
            creationflags=creation_flags,
        )
        messagebox.showinfo("Started", "Bot started in a separate console window.")
        app.destroy()

    tk.Button(app, text="Start bot", command=start_bot, width=18).grid(
        row=8, column=1, padx=12, pady=18, sticky="e"
    )

    app.grid_columnconfigure(1, weight=1)
    app.mainloop()


if __name__ == "__main__":
    main()
