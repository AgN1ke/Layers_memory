# Telegram Gemini Bot Host

Простий Telegram host для ручного тесту Memory Engine з Gemini.

Це не частина Rust-ядра. Це окремий host-застосунок, який:

- питає Telegram bot token у терміналі;
- питає Gemini API key у терміналі;
- використовує `memory_engine` Python adapter;
- пише повідомлення користувача і відповіді bot-а в памʼять через `ingest`;
- просить у engine готовий `core_context_package` для prompt-а;
- відповідає через Gemini;
- створює archive memory через `/sleep`;
- виконує engine-level `auto_sleep`, якщо `ingest` повернув sleep task;
- виконує `sleep_compression` pending task через Gemini і повертає результат у `resume_sleep_compression`.

## Запуск

З кореня репозиторію:

```powershell
.\hosts\telegram_gemini_bot\run.ps1
```

Якщо PowerShell не дає вставити token/key, запускайте GUI-варіант:

```powershell
.\hosts\telegram_gemini_bot\run_gui.ps1
```

Він відкриє маленьке вікно з полями для token/key і запустить bot з env-змінними.

Скрипт:

1. Увімкне UTF-8 для термінала.
2. Перевірить/створить venv у `crates/python_adapter/.venv`.
3. Збере Python adapter через `maturin develop`.
4. Запустить `bot.py`.
5. Попросить ввести Telegram token і Gemini API key.

## Моделі За Ролями

За замовчуванням:

- `reasoning` -> `gemini-2.5-pro`
- `balanced` -> `gemini-2.5-flash`
- `fast` -> `gemini-2.5-flash-lite`
- chatbot replies -> `balanced`

Під час запуску можна натиснути Enter і лишити defaults, або ввести іншу модель.

## Telegram Команди

- `/start` або `/help` - показати довідку.
- `/sleep` - стиснути поточну сесію в archive memory і виконати LLM-доробку через Gemini.
- `/recall текст` - пошукати archive memory.
- `/tasks` - показати pending tasks.
- `/models` - показати active role -> model mapping.

Plain text без `/`:

1. Зберігається як event.
2. Якщо engine повертає `auto_sleep`, bot запамʼятовує цей sleep task для виконання після відповіді.
3. Просить `core_context_package` у engine.
4. Дає Gemini відповідь з готовим context package.
5. Зберігає відповідь bot-а як `assistant_message`.
6. Якщо user-message або assistant-message перетнули auto-sleep поріг, bot виконує повернений `sleep_compression` task через Gemini і завершує `resume_sleep_compression`.

Якщо повідомлення містить `запам'ятай`, `запамʼятай`, `пам'ятай`, `памʼятай` або `важливо`, bot автоматично робить `/sleep` після відповіді, щоб цей факт одразу став archive memory.

## Runtime Дані

Локальна памʼять bot host лежить тут:

```text
hosts/telegram_gemini_bot/runtime/memory
```

Ця тека ігнорується git.

Runtime log лежить тут:

```text
hosts/telegram_gemini_bot/runtime/logs/bot.log
```

У log пишуться polling events, оброблені Telegram message id і traceback-и помилок. API keys туди не записуються.

Telegram polling offset зберігається тут:

```text
hosts/telegram_gemini_bot/runtime/state/telegram_offset.json
```

Це потрібно, щоб після рестарту bot не відповідав повторно на вже оброблені pending updates.

## Межі

Це простий long-polling bot через Telegram Bot API `getUpdates`/`sendMessage`.

Він не зберігає API keys у файлах. Token і key вводяться в терміналі на кожному запуску.

LLM-сегрегація зроблена на host-рівні: Memory Engine повертає `PendingTask.role_hint`, а bot вибирає конкретну Gemini model за role mapping.
