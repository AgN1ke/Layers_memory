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
- зберігає explicit Core-факти через `/remember` у scope поточного `telegram_<chat_id>`;
- після успішного sleep переносить достатньо впевнені `personal_signals` з archive у Core без regex-хардкоду;
- додає мʼякі event-теги для можливих профільних фактів, щоб майбутній reflection міг їх переглянути;
- виконує `compact_memory_pass` і `sleep_compression` pending tasks через Gemini; default flow є multi-pass: compact memory pass, emotional pass, topic thread pass, personal signal pass, relational pass і consolidator;
- повертає plain-text compact memory у `resume_compact_memory_pass` і один `sleep_compression_result.v1` у `resume_sleep_compression`.

## Запуск

З кореня репозиторію:

```powershell
.\hosts\telegram_gemini_bot\run.ps1
```

Якщо PowerShell не дає вставити token/key, запускайте GUI-варіант:

```powershell
.\hosts\telegram_gemini_bot\run_gui.ps1
```

Він відкриє маленьке вікно з полями для token/key, model mapping, порога auto-sleep і запустить bot з env-змінними.

GUI launcher може кешувати token/key і model mapping у локальному файлі:

```text
hosts/telegram_gemini_bot/runtime/state/secrets.local.json
```

Це plaintext-кеш для зручності локального тестування. Уся тека `hosts/*/runtime/` ігнорується git, тому файл не потрапляє в commit/GitHub. У GUI є кнопка `Clear saved keys`, яка видаляє цей кеш.

## Локальний Harness Без Telegram

Для швидкої перевірки memory/prompt/sleep циклу без Telegram token:

```powershell
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --list-scenarios
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario mixed_short --dry-run
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario mixed_short --turn-limit 4 --no-force-sleep-at-end
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario one_topic_compact --no-force-sleep-at-end --auto-sleep-after-events 0
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario multi_topic_compact --no-force-sleep-at-end --auto-sleep-after-events 0
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario all
```

Harness читає Gemini key із `runtime/state/secrets.local.json` або `GEMINI_API_KEY`, створює окремі `local_harness_*` сесії і пише reports у `runtime/logs/local_harness/`. Він використовує ті самі функції `bot.py`, що й Telegram host: `core_context_package`, prompt rendering, Gemini call, ingest відповіді, sleep completion і Archive → Core bridge.

Сценарії `one_topic_compact` і `multi_topic_compact` потрібні саме для перевірки compact memory: агент має сам визначити кількість змістових одиниць, а не виконувати фіксовану квоту тез.

Скрипт:

1. Увімкне UTF-8 для термінала.
2. Перевірить/створить venv у `crates/python_adapter/.venv`.
3. Збере Python adapter через `maturin develop`.
4. Запустить `local_harness.py` з переданими аргументами.
5. Візьме Gemini key і model mapping із локального кешу або env; Telegram token не потрібен.

## Моделі За Ролями

За замовчуванням:

- `reasoning` -> `gemini-2.5-pro`
- `balanced` -> `gemini-2.5-flash`
- `fast` -> `gemini-2.5-flash-lite`
- chatbot replies -> `balanced`
- dev/test auto-sleep threshold -> `50`

Під час запуску можна натиснути Enter і лишити defaults, або ввести іншу модель.

## Telegram Команди

- `/start` або `/help` - показати довідку.
- `/sleep` - стиснути поточну сесію в archive memory і виконати multi-pass LLM-доробку через Gemini.
- `/archives` - показати останні завершені archive-записи, їх gist і кількість emotional/personal треків.
- `/archive_last` - показати повний останній archive-запис: compact memory, gist, narrative, emotional markers, personal signals, relational tone.
- `/archive id` - показати конкретний archive-запис за id.
- `/recall текст` - пошукати archive memory.
- `/core` - показати стабільні Core-факти разом із `core_fact_id`.
- `/core_seed` - повторно пройти завершені archive-записи і засіяти Core з їхніх `personal_signals`.
- `/remember текст` - вручну записати стабільний Core-факт.
- `/core_update id текст` - оновити Core-факт у поточному chat scope.
- `/core_forget id` - позначити Core-факт як `deprecated`; він більше не потрапляє в prompt.
- `/tasks` - показати pending tasks.
- `/models` - показати active role -> model mapping.

Plain text без `/`:

1. Зберігається як event.
2. Якщо engine повертає `auto_sleep`, bot запамʼятовує цей sleep task для виконання після відповіді. Поточний `auto_sleep_after_events` — dev/test-прискорювач, а не продуктовий sleep trigger.
3. Просить `core_context_package` у engine.
4. Дає Gemini відповідь з готовим context package.
5. Зберігає відповідь bot-а як `assistant_message`.
6. Додає до user event мʼякі теги на кшталт `name_reference`, `age_reference`, `preference_signal`, якщо текст схожий на потенційно важливу інформацію.
7. Якщо user-message або assistant-message перетнули auto-sleep поріг, bot виконує повернені `compact_memory_pass` і `sleep_compression` tasks через Gemini flow і завершує `resume_compact_memory_pass` / `resume_sleep_compression`.

Після sleep context package не дублює archived raw events у `session_recent` / `session_trace`. Старша частина unarchived window переходить в `archive_relevant` як `compact_memory` тези "подія -> висновок", а приблизно 30% найсвіжіших events лишаються active tail для плавного продовження розмови.

Core-факти ізольовані по Telegram chat id. Якщо bot-у пишуть два різні користувачі з різних чатів, `/core` і prompt-контекст кожного чату бачать тільки свій scope.

Якщо повідомлення містить явне прохання оновити памʼять (`запам...`, `запиши в пам...`, `це важливо`, `онови пам...`), bot автоматично ставить sleep у фон після відповіді, щоб ця подія швидше стала archive memory.

Після успішного sleep host бере `personal_signals`, які виділив LLM-прохід, і переносить у Core тільки сигнали з confidence `>= 0.85`, підтримкою хоча б одного `user_message` source event і без near-duplicate у поточному Core scope. Категорія є вільним normalized `snake_case` полем (`name`, `pet`, `physical_trait`, `food_preference`, тощо), а не whitelist. Це не regex-витяг фактів із raw text: код не шукає "кішку", "імʼя" чи інші конкретні сутності. Він лише застосовує загальні gate-правила до структурованого результату sleep. `/core_seed` повторює цей крок для вже завершених archive-записів.

Для debug можна повернути старий single-pass sleep через env `MEMORY_BOT_SLEEP_MODE=single`. `compact_memory_pass` все одно виконується окремо, бо саме він створює prompt-facing стиснуту памʼять. За замовчуванням використовується multi-pass sleep.

Активний system prompt для Telegram-чату лежить у:

```text
prompts/telegram_chat_system.md
```

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
