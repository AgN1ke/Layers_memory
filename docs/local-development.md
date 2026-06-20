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

## Local Secret Scan

Public repository safety relies on two layers:

- GitHub Actions runs `gitleaks` on pushes and pull requests.
- Developers should run the same scan locally before committing.

Install local tools:

```powershell
pip install pre-commit
winget install --id Gitleaks.Gitleaks --source winget
```

Enable the hook:

```powershell
pre-commit install
pre-commit run --all-files
```

## Local Godot Tool Binary

Godot conformance can run without installing Godot globally. Put the Windows
Godot 4.6 console build here:

```text
tools/godot/Godot_v4.6.3-stable_win64_console.exe
```

Keep the matching non-console executable beside it if the distribution ships both
files:

```text
tools/godot/Godot_v4.6.3-stable_win64.exe
```

`tools/godot/` is gitignored. The conformance runner checks this directory before
`%TEMP%`, `GODOT_BIN`, and PATH fallbacks, so this command should work on this
machine without extra environment variables:

```powershell
crates\python_adapter\.venv\Scripts\python.exe tests\host_conformance\host_conformance.py --host godot-headless
```

The first product-host Godot spike uses the same local binary and runs a minimal
Chibigochi-style memory loop: user text -> context -> reply -> sleep -> restart
-> persisted Core/context recall.

```powershell
crates\python_adapter\.venv\Scripts\python.exe tests\host_conformance\host_conformance.py --host chibigochi-spike
```

The minimal scene/UI wrapper can be smoke-tested headlessly as well:

```powershell
crates\python_adapter\.venv\Scripts\python.exe tests\host_conformance\host_conformance.py --host chibigochi-ui
```

The Chibigochi LLM bridge path starts a local HTTP LLM proxy and verifies that
Godot can execute chat/sleep/fidelity requests through that bridge instead of
using in-process fake responses:

```powershell
crates\python_adapter\.venv\Scripts\python.exe tests\host_conformance\host_conformance.py --host chibigochi-llm-bridge
```

The async Chibigochi product-loop smoke uses the Godot scene/UI wrapper with an
async HTTP bridge, loading/error states, sleep, and restart recall:

```powershell
crates\python_adapter\.venv\Scripts\python.exe tests\host_conformance\host_conformance.py --host chibigochi-product-loop
```

To run the same Chibigochi HTTP bridge against the cached Gemini key instead of
the deterministic fake proxy:

```powershell
crates\python_adapter\.venv\Scripts\python.exe hosts\chibigochi_spike\chibigochi_gemini_proxy.py --run-conformance --validate-key
```

For manual Godot experiments, start the proxy as a local endpoint:

```powershell
crates\python_adapter\.venv\Scripts\python.exe hosts\chibigochi_spike\chibigochi_gemini_proxy.py --host 127.0.0.1 --port 8765 --validate-key
```

Secrets are read from the existing gitignored cache:

```text
hosts/telegram_gemini_bot/runtime/state/secrets.local.json
```

Proxy logs and token telemetry are written under gitignored runtime state:

```text
hosts/chibigochi_spike/runtime/logs/
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

**Важливо про одночасний доступ.** Конкурентність Memory Engine у v0.2 є внутрішньопроцесною: один процес може безпечно обробляти багато потоків/сесій через `LockRegistry`, але два окремі процеси над одним `memory/` каталогом не синхронізовані. Не запускай `memory_terminal` проти `hosts/telegram_gemini_bot/runtime/memory`, поки живий Telegram bot пише у цю саму runtime-теку. Для debug використовуй окрему scratch-теку або спершу зупини bot через `run_dev_bot.ps1`.

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

Він відкриває маленьке вікно з полями для Telegram token, Gemini API key і model mapping. Для локальної зручності GUI може кешувати ці значення у `hosts/telegram_gemini_bot/runtime/state/secrets.local.json`; файл gitignored і має кнопку очищення.

Для щоденного dev-циклу після одного GUI-запуску з кешованими ключами краще використовувати:

```powershell
.\hosts\telegram_gemini_bot\run_dev_bot.ps1
```

`run_dev_bot.ps1` зупиняє старі `bot.py` процеси, збирає PyO3 adapter через `maturin develop`, читає кешовані ключі з `runtime/state/secrets.local.json`, запускає bot non-interactive з UTF-8 і вмикає dev sleep notices. Корисні режими: `-ClearMemory` стерти runtime memory перед запуском, `-NoBuild` пропустити збірку, `-Visible` запустити видиме вікно, `-TailLog` одразу читати `bot.log`, `-NoDevSleepNotices` вимкнути тимчасові Telegram-повідомлення про sleep.

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

Sleep у bot host запускається вручну через `/sleep`, при token/context budget pressure або від scheduled idle sleep, наприклад нічного запуску о 04:00.

Runtime memory host-бота:

```text
hosts/telegram_gemini_bot/runtime/memory
```

Ця runtime тека ігнорується git.

Не відкривати цю теку другим writer-процесом. Живий bot, local harness і `memory_terminal` мають або працювати по черзі, або використовувати різні memory directories.

Runtime log host-бота:

```text
hosts/telegram_gemini_bot/runtime/logs/bot.log
```

У log пишуться старт bot-а, polling batches, message id, короткий текст обробленого update, traceback-и помилок, `context_budget` для кожного chat turn, `token_usage` для кожного Gemini-виклику і `sleep_compression_tokens` для sleep. API keys не логуються.

Детальна token telemetry:

```text
hosts/telegram_gemini_bot/runtime/logs/token_usage.jsonl
```

Там кожен рядок — JSON record. Для Gemini-викликів пишуться `operation`, `model_role`, `model`, provider `usageMetadata` (`prompt_tokens`, `output_tokens`, `total_tokens`, `thoughts_tokens`), estimated prompt/output tokens і, для chat replies, estimated baseline "raw history without compression" та savings estimate. Для sleep пишеться `sleep_compression_metric`: raw transcript, stored full archive, prompt-facing archive payload і `compact_memory` tokens/ratios. `compressed_estimated_tokens` лишається legacy alias для `compact_memory_estimated_tokens`.

## Локальний Conversation Harness Без Telegram

Для перевірки памʼяті без Telegram polling є harness:

```powershell
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --list-scenarios
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario mixed_short --dry-run
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario mixed_short --turn-limit 4 --no-force-sleep-at-end
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario one_topic_compact --no-force-sleep-at-end
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario multi_topic_compact --no-force-sleep-at-end
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario all
```

Він використовує той самий `memory_engine`, Gemini client, `core_context_package`, prompt builder, sleep flow і Archive → Core bridge, що й Telegram host. Telegram token не потрібен. Gemini key і model mapping читаються з `hosts/telegram_gemini_bot/runtime/state/secrets.local.json` або з env-змінних (`GEMINI_API_KEY`, `MEMORY_BOT_MODEL_*`).

Сценарії навмисно не є одним жорстким golden path: `mixed_short`, `topic_switching`, `identity_noise` перевіряють різні переходи тем, особисті твердження, шум і контроль mid-dialog greeting. `one_topic_compact` і `multi_topic_compact` перевіряють, що compact memory не має штучної квоти тез: однотемна розмова має стискатись в одну змістову тезу, а багатотемна — у стільки тез, скільки реально підтримують епізоди розмови. Reports пишуться сюди:

```text
hosts/telegram_gemini_bot/runtime/logs/local_harness/
```

Harness — це preflight для швидкої діагностики. Він не замінює acceptance на реальному Telegram host, бо Telegram лишається першим production-like інтерфейсом.

Telegram polling offset host-бота:

```text
hosts/telegram_gemini_bot/runtime/state/telegram_offset.json
```

Offset зберігається після кожного обробленого update, щоб після рестарту bot не відповідав повторно на старі Telegram pending updates.

Поточна логіка діалогу:

- plain text користувача зберігається як `user_message`;
- `engine.ingest()` повертає `IngestResult` з `stored_event`;
- bot просить `engine.core_context_package(...)`, а не збирає recent/trace/archive сам;
- Gemini отримує compact prompt view із `session_recent`, `session_trace`, `archive_relevant` як `compact_memory` тези, `core_facts`, `domain_state`; повний debug/API package лишається в engine response і log/report, але не тягнеться в ordinary chat prompt із довгими IDs;
- `session_recent` і `session_trace` містять тільки unarchived active tail; події, які вже пройшли sleep, мають приходити через `archive_relevant`;
- відповідь bot-а зберігається як `assistant_message`;
- plain text не записується напряму в Core Store через regex extraction;
- Telegram host додає мʼякі event-теги (`personal_fact_signal`, `name_reference`, `age_reference`, `preference_signal`) і піднімає `importance_hint`, щоб sleep/reflection потім уважніше переглянули ці події;
- Telegram host записує і читає Core-факти зі scope `telegram_<chat_id>`, щоб факти різних чатів не змішувались;
- Core можна перевірити командою `/core`, явно додати факт командою `/remember text`, оновити через `/core_update id text`, або прибрати з активного контексту через `/core_forget id`;
- archive memory створюється через `/sleep`, token-pressure sleep або scheduled idle sleep;
- Telegram host за замовчуванням виконує multi-pass sleep: compact memory pass, emotional pass, topic thread pass, personal signal pass, relational pass і consolidator. `compact_memory_pass` створює короткі тези для prompt, а решта проходів створює повний audit/archive. Для debug старий single-pass режим можна увімкнути env `MEMORY_BOT_SLEEP_MODE=single`.
- активний чат-промпт Telegram host-а лежить у `prompts/telegram_chat_system.md`, а не захардкоджений у Python.

## Поточний Очікуваний Результат

На момент запису цього документа локальний цикл проходить:

- `cargo fmt --check` проходить;
- `cargo test --workspace` проходить;
- `cargo test --workspace` запускає 17 тестів у `memory_engine` (6 + 3 + 8 у трьох test-файлах);
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

9 тестів у `tests/test_basic.py`, усі проходять:

- `test_ingest_creates_stored_event`;
- `test_read_session_returns_stored_events`;
- `test_explicit_sleep_preserves_active_tail`;
- `test_full_cycle_ingest_sleep_resume_recall`;
- `test_core_context_package_combines_session_and_archive`;
- `test_upsert_core_fact_is_returned_in_context_package`;
- `test_core_context_package_does_not_leak_facts_between_scopes`;
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

Перед тим як називати v0.1 завершеним, пройти live-checklist:

```text
docs/v0.1-acceptance.md
```
