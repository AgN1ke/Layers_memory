# Telegram Gemini Bot Host

Простий Telegram host для ручного тесту Memory Engine з Gemini.

Це не частина Rust-ядра. Це окремий host-застосунок, який:

- питає Telegram bot token у терміналі;
- питає Gemini API key у терміналі;
- використовує `memory_engine` Python adapter;
- пише повідомлення користувача і відповіді bot-а в памʼять через `ingest`;
- просить у engine готовий `core_context_package` для prompt-а;
- відповідає через Gemini;
- створює archive memory через `/sleep`, token-pressure sleep або scheduled idle sleep;
- зберігає explicit Core-факти через `/remember` у scope поточного `telegram_<chat_id>`;
- після успішного sleep переносить достатньо впевнені `personal_signals` з archive у Core без regex-хардкоду;
- додає мʼякі event-теги для можливих профільних фактів, щоб майбутній reflection міг їх переглянути;
- виконує `compact_memory_pass` і `sleep_compression` pending tasks через Gemini; default flow є multi-pass: compact memory pass, emotional pass, topic thread pass, personal signal pass, relational pass і consolidator;
- може після sleep/fidelity виконати core-owned autonomous reflection, якщо
  власник явно увімкнув `MEMORY_BOT_AUTO_REFLECTION=1`;
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

Він відкриє маленьке вікно з полями для token/key і model mapping та запустить bot з env-змінними.

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
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario one_topic_compact --no-force-sleep-at-end
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario multi_topic_compact --no-force-sleep-at-end
.\hosts\telegram_gemini_bot\run_local_harness.ps1 --scenario all
```

Harness читає Gemini key із `runtime/state/secrets.local.json` або `GEMINI_API_KEY`, створює окремі `local_harness_*` сесії і пише reports у `runtime/logs/local_harness/`. Він використовує ті самі функції `bot.py`, що й Telegram host: `core_context_package`, prompt rendering, Gemini call, ingest відповіді, sleep completion і Archive → Core bridge.

Сценарії `one_topic_compact` і `multi_topic_compact` потрібні саме для перевірки compact memory: агент має сам визначити кількість змістових одиниць, а не виконувати фіксовану квоту тез.

Для рядка 16 матриці використовуйте `--scenario memory_quality`. З `--dry-run`
він друкує синтетичні репліки без доступу до ключа. Живий прогін робить два
Sleep й один запит відповіді, відкриває сховище повторно й пише `report.json`
і токени у власний каталог `runtime/local_harness/memory_quality_*`.
Структурні перевірки виконуються автоматично, зміст відповіді звіряється
з репліками. Для доказового прогону запускайте `local_harness.py` з явно
встановленого wheel, щоб звіт ідентифікував перевірену збірку. Сценарій
запускається окремо від `--scenario all`.

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

Під час запуску можна натиснути Enter і лишити defaults, або ввести іншу модель.

## Telegram Команди

- `/start` або `/help` - показати довідку.
- `/sleep` - стиснути поточну сесію в archive memory і виконати multi-pass LLM-доробку через Gemini.
- `/archives` - показати ID і кількість висновків останніх завершених архівів поточного чату.
- `/archive_last` - показати повний останній archive-запис: compact memory, gist, narrative, emotional markers, personal signals, relational tone.
- `/archive id` - показати конкретний archive-запис за id.
- `/memory_stats` - показати лічильники памʼяті поточного чату.
- `/my_memory` - показати підтверджені записи про відправника. Із групи бот надсилає їх приватно, якщо користувач уже відкрив приватний чат із ботом.
- `/forget_me` - стерти памʼять про відправника в поточному чаті й отримати звіт. Якщо частину старих записів неможливо атрибутувати, звіт показує, що стирання неповне.
- `/recall текст` - пошукати archive memory.
- `/core` - показати стабільні Core-факти разом із `core_fact_id`.
- `/core_seed` - повторно пройти завершені archive-записи і засіяти Core з їхніх `personal_signals`.
- `/remember текст` - вручну записати стабільний Core-факт.
- `/core_update id текст` - оновити Core-факт у поточному chat scope.
- `/core_forget id` - позначити Core-факт як `deprecated`; він більше не потрапляє в prompt.
- `/evidence id` і `/fidelity id` - переглянути підтверджені джерела або запустити перевірку висновку в поточному чаті.
- `/forgotten` і `/remember_back id` - переглянути або явно відновити забуті висновки поточного чату.
- `/tasks` - показати незавершені операції поточного чату.
- `/models` - показати active role -> model mapping.

Plain text без `/`:

1. Зберігається як event.
2. Просить `core_context_package` у engine.
3. Дає Gemini відповідь з готовим context package.
4. Зберігає відповідь bot-а як `assistant_message`.
5. Додає до user event мʼякі теги на кшталт `name_reference`, `age_reference`, `preference_signal`, якщо текст схожий на потенційно важливу інформацію.
6. Якщо context budget наближається до межі, bot ставить `engine.sleep(session_id)` у background queue і виконує `compact_memory_pass` / `sleep_compression` через Gemini.
7. Окремо bot має scheduled idle sleep: у нічне вікно, якщо сесія має незаархівовані події й не активна, він запускає той самий sleep flow без ручної команди.

Після sleep context package не дублює archived raw events у `session_recent` / `session_trace`. Старша частина розмови повертається в `archive_relevant` як `compact_memory` тези "подія -> висновок".

Core-факти ізольовані по Telegram chat id. Якщо bot-у пишуть два різні користувачі з різних чатів, `/core` і prompt-контекст кожного чату бачать тільки свій scope.

Перегляд і стирання за людиною використовують стабільний ID відправника.
Анонімні відправники групи не можуть виконати ці дії. Стирання охоплює
памʼять рушія. Повідомлення Telegram та історичні журнали хоста
зберігаються окремо.

Відомі цитати зберігаються як посилання на події рушія. Локальна таблиця
`runtime/state/telegram_message_events.sqlite3` зіставляє ID повідомлень
з ID подій без копіювання їхнього тексту. Для старих повідомлень без
відповідності бот зберігає лише авторський текст нової відповіді.

Якщо повідомлення містить явне прохання оновити памʼять (`запам...`, `запиши в пам...`, `це важливо`, `онови пам...`), bot позначає event тегами й піднімає `importance_hint`, але не запускає sleep тільки через ключове слово. Перехід у archive відбувається через token-pressure, scheduled idle або ручний `/sleep`.

Після успішного sleep host бере `personal_signals`, які виділив LLM-прохід, і переносить у Core тільки сигнали з confidence `>= 0.85`, підтримкою хоча б одного `user_message` source event і без near-duplicate у поточному Core scope. Категорія є вільним normalized `snake_case` полем (`name`, `pet`, `physical_trait`, `food_preference`, тощо), а не whitelist. Це не regex-витяг фактів із raw text: код не шукає "кішку", "імʼя" чи інші конкретні сутності. Він лише застосовує загальні gate-правила до структурованого результату sleep. `/core_seed` повторює цей крок для вже завершених archive-записів.

Для debug можна повернути старий single-pass sleep через env `MEMORY_BOT_SLEEP_MODE=single`. `compact_memory_pass` все одно виконується окремо, бо саме він створює prompt-facing стиснуту памʼять. За замовчуванням використовується multi-pass sleep.

Autonomous reflection вимкнений за замовчуванням, бо може спричинити додатковий
reasoning-model call. При `MEMORY_BOT_AUTO_REFLECTION=1` host після завершення
sleep і auto-fidelity викликає `begin_reflection_if_due(..., "after_sleep")`.
Пороги, eligibility, bounded input і deduplication належать Rust core; bot лише
виконує повернений `PendingTask`. У постійний sleep log пишуться reason, counts і
candidate count без текстів памʼяті. Phase B telemetry також показує кількість
привʼязаних sessions, нових units, related-history units і вже наявних
candidates. У Telegram `core_scope` стабільно дорівнює поточному chat scope;
схожість текстів не звʼязує різні чати.

Reflection prompt v3 повертає typed outcomes. Bot не вирішує сам, чи створити
новий candidate, підсилити наявний або відкрити Core challenge: він виконує
`PendingTask` і передає відповідь назад у ядро. Text-free telemetry окремо
показує кількість нових і reinforced candidates та machine-readable
`no_insight` reason.

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

Структурована діагностика recall пишеться окремо в
`runtime/logs/recall_trace.jsonl`. На кожен context package host просить
`explain_recall`, але не додає debug payload у LLM prompt. Trace містить hash
запиту, ordinary Archive rank і окрему associative-memory telemetry:
`memory_unit_id`, parent `archive_id`, activation, cosine та deep score. Текст
запиту й тексти спогадів у trace не пишуться. У chat context current-turn
embedding живить тільки ephemeral `associative_memory`; explicit `/recall` може
окремо запускати unit-focused Stage 2.

Python-конструктор `MemoryEngine` приймає необовʼязкові
`stage2_candidate_limit` (default `40`) і `stage2_vector_weight` (default
`0.5`). Це лише host-facing налаштування Rust-політики; embeddings і далі
рахуються локально через `fastembed` та передаються в ядро готовими векторами.
Context embedding рахується тільки з поточного user text, не з уже зібраного
memory prompt, тому підтягнутий старий спогад не може сам підтримувати тему на
наступному turn.

Telegram polling offset зберігається тут:

```text
hosts/telegram_gemini_bot/runtime/state/telegram_offset.json
```

Це потрібно, щоб після рестарту bot не відповідав повторно на вже оброблені pending updates.

## Межі

Це простий long-polling bot через Telegram Bot API `getUpdates`/`sendMessage`.

Він може кешувати API keys тільки у локальному gitignored `runtime/state/secrets.local.json`, якщо це явно увімкнено у GUI.

LLM-сегрегація зроблена на host-рівні: Memory Engine повертає `PendingTask.role_hint`, а bot вибирає конкретну Gemini model за role mapping.
