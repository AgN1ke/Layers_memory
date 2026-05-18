# Local Development Environment

Цей документ потрібен для моделей або розробників, які відкривають проєкт без історії чату. Тут зафіксовано, що вже встановлено на цій машині, де воно лежить і якими командами перевіряти локальний цикл.

Документ описує поточну Windows-машину користувача для `C:\Python_projects\Layers_memory`.

## Що Встановлено

### Rust Toolchain

Встановлено через `winget`:

```powershell
winget install --id Rustlang.Rustup --source winget
```

Поточний стан:

- `rustup 1.29.0`
- `rustc 1.95.0`
- `cargo 1.95.0`
- active toolchain: `stable-x86_64-pc-windows-msvc`
- installed target: `x86_64-pc-windows-msvc`

Основні шляхи:

- Cargo binaries: `C:\Users\AgNike\.cargo\bin`
- Rustup home: `C:\Users\AgNike\.rustup`
- Cargo cache/registry: `C:\Users\AgNike\.cargo`

У `C:\Users\AgNike\.cargo\bin` лежать:

- `cargo.exe`
- `rustc.exe`
- `rustup.exe`
- `rustfmt.exe`
- `cargo-fmt.exe`
- `cargo-clippy.exe`
- `clippy-driver.exe`
- `rust-analyzer.exe`

### Visual Studio Build Tools

Встановлено через `winget`:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --source winget --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --norestart"
```

Поточний стан:

- package: `Microsoft.VisualStudio.2022.BuildTools`
- version: `17.14.32`
- installed path: `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`
- discovery tool: `C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe`

Навіщо це потрібно:

- Rust target `x86_64-pc-windows-msvc` потребує MSVC linker `link.exe`.
- Без Visual Studio Build Tools `cargo test` падає з помилкою `linker link.exe not found`.
- У звичайній PowerShell команді `where.exe link` може нічого не показати, але `cargo` після встановлення Build Tools уже вміє знайти MSVC linker.

## Як Перевірити Середовище

Спочатку перейти в корінь проєкту:

```powershell
cd C:\Python_projects\Layers_memory
```

Перевірити Rust:

```powershell
rustup --version
rustup show
rustc --version
cargo --version
```

Перевірити встановлені Windows-пакети:

```powershell
winget list --id Rustlang.Rustup --source winget
winget list --id Microsoft.VisualStudio.2022.BuildTools --source winget
```

Якщо поточна PowerShell-сесія не бачить `cargo`, тимчасово додати Cargo binaries у PATH:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
```

Це потрібно тільки для старої вже відкритої сесії. User PATH уже містить `C:\Users\AgNike\.cargo\bin`, тому в новому терміналі `cargo` має бути доступний без ручного додавання.

## Як Запускати Проєкт Локально

Підтягнути залежності:

```powershell
cargo fetch
```

Перевірити форматування:

```powershell
cargo fmt --check
```

Запустити тести:

```powershell
cargo test --workspace
```

Запустити clippy як строгий quality gate:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Повний локальний цикл для моделі:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo fetch
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Як Запустити Живий Термінал Памʼяті

Інтерактивний runner:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo run -p memory_engine --bin memory_terminal -- memory
```

Якщо термінал некоректно показує кирилицю, перед запуском увімкнути UTF-8:

```powershell
chcp 65001
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new()
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo run -p memory_engine --bin memory_terminal -- memory
```

Команди всередині runner:

```text
/help              показати команди
/where             показати активну теку памʼяті і сесію
/session <id>      перемкнути активну сесію
/sleep             стиснути поточну сесію в preliminary archive memory
/recall <text>     пошукати archive memory
/tasks             показати pending LLM tasks
/exit              вийти
```

Plain text без `/` записується як `IngestEvent` у поточну сесію.

## Як Запустити Telegram Gemini Bot

Host-застосунок лежить окремо:

```text
hosts/telegram_gemini_bot/
```

Запуск з кореня репозиторію:

```powershell
.\hosts\telegram_gemini_bot\run.ps1
```

Якщо PowerShell не дає вставити token/key, запускати GUI-варіант:

```powershell
.\hosts\telegram_gemini_bot\run_gui.ps1
```

Він відкриває маленьке вікно з полями для Telegram token, Gemini API key і model mapping. Секрети передаються в bot через env-змінні й не записуються у файли.

Скрипт:

- вмикає UTF-8 у Windows console;
- перевіряє venv `crates/python_adapter/.venv`;
- ставить/оновлює `maturin` і `pytest`;
- виконує `maturin develop` для PyO3 adapter;
- запускає `hosts/telegram_gemini_bot/bot.py`.

Під час запуску bot питає:

- Telegram bot token;
- Gemini API key;
- model mapping для ролей `reasoning`, `balanced`, `fast`.

Defaults:

- `reasoning` -> `gemini-2.5-pro`;
- `balanced` -> `gemini-2.5-flash`;
- `fast` -> `gemini-2.5-flash-lite`;
- chatbot replies -> `balanced`.

Runtime memory host-бота:

```text
hosts/telegram_gemini_bot/runtime/memory
```

Ця runtime тека ігнорується git.

Runtime log host-бота:

```text
hosts/telegram_gemini_bot/runtime/logs/bot.log
```

У log пишуться старт bot-а, polling batches, message id, короткий текст обробленого update і traceback-и помилок. API keys не логуються.

Telegram polling offset host-бота:

```text
hosts/telegram_gemini_bot/runtime/state/telegram_offset.json
```

Offset зберігається після кожного обробленого update, щоб після рестарту bot не відповідав повторно на старі Telegram pending updates.

Поточна логіка діалогу:

- plain text користувача зберігається як `user_message`;
- `engine.ingest()` повертає `IngestResult` з `stored_event` і можливим `auto_sleep`;
- bot просить `engine.core_context_package(...)`, а не збирає recent/trace/archive сам;
- Gemini отримує готовий context package: `session_recent`, `session_trace`, `archive_relevant`, `core_facts`, `domain_state`;
- відповідь bot-а зберігається як `assistant_message`;
- archive memory створюється через `/sleep`, auto-sleep keywords або engine-level auto-sleep після порога незаархівованих подій.

## Поточний Очікуваний Результат

На момент запису цього документа локальний цикл проходить:

- `cargo fmt --check` проходить;
- `cargo test --workspace` проходить;
- `cargo test --workspace` запускає 15 тестів у `memory_engine` (6 + 3 + 6 у трьох test-файлах);
- `cargo clippy --workspace --all-targets -- -D warnings` проходить.

## Python-адаптер

Python-адаптер живе у `crates/python_adapter/` як sub-crate workspace. Він зібраний на PyO3 (Rust extension) і будується через `maturin`.

### Що Встановлено

- Python 3.13 доступний через `py -3.13`. Інші версії: 3.9, 3.12.
- Локальний `venv` для адаптера: `crates\python_adapter\.venv` (Python 3.13).
- У venv встановлено `maturin 1.13.3` і `pytest 9.0.3`.

### Як Зібрати і Запустити Pytest

```powershell
$venv = "C:\Python_projects\Layers_memory\crates\python_adapter\.venv"
$env:VIRTUAL_ENV = $venv
$env:Path = "$venv\Scripts;C:\Users\AgNike\.cargo\bin;" + $env:Path
Set-Location C:\Python_projects\Layers_memory\crates\python_adapter
maturin develop
pytest tests/ -v
```

`maturin develop` компілює Rust extension і ставить його в активний venv як editable wheel під ім'ям `memory_engine`. Імпорт у Python - `import memory_engine`.

### Свіже Розгортання

Якщо `.venv` не існує:

```powershell
py -3.13 -m venv crates\python_adapter\.venv
$venv = "C:\Python_projects\Layers_memory\crates\python_adapter\.venv"
& "$venv\Scripts\python.exe" -m pip install --upgrade pip
& "$venv\Scripts\python.exe" -m pip install maturin pytest
```

Потім - команда збірки і pytest з попереднього блока.

### Поточний Очікуваний Результат Pytest

7 тестів у `tests/test_basic.py`, усі проходять:

- `test_ingest_creates_stored_event`;
- `test_read_session_returns_stored_events`;
- `test_ingest_returns_auto_sleep_after_default_threshold`;
- `test_full_cycle_ingest_sleep_resume_recall`;
- `test_core_context_package_combines_session_and_archive`;
- `test_ingest_rejects_wrong_schema`;
- `test_recall_zero_limit_uses_engine_default`.

## Що Не Треба Шукати Вручну

Не треба вручну шукати Rust по системі. Основна точка входу:

```text
C:\Users\AgNike\.cargo\bin
```

Не треба вручну шукати Visual Studio Build Tools. Основна точка входу:

```text
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools
```

Якщо треба знайти встановлення Visual Studio програмно:

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
```

## Правило Для Наступних Моделей

Перед роботою з кодом перевірити:

```powershell
git status --short --branch
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo test --workspace
```

Після змін у Rust-коді запускати:

```powershell
cargo fmt
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Якщо змінюються залежності в `Cargo.toml`, треба оновити і комітити `Cargo.lock`.

`target/` не комітити: це локальний build output і він ігнорується через `.gitignore`.
