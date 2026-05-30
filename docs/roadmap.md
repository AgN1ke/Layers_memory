# Roadmap Memory Engine

Цей документ — **жива дорожня карта** проєкту. Він не дублює стратегію (`docs/strategy.md`) чи архітектуру (`docs/architecture.md`). Він відповідає на одне питання: **куди ми йдемо і де зараз стоїмо**.

Документ оновлюється у двох випадках: коли пункт стає зробленим (відмітка `[x]` + дата + посилання на коміт або файл), і коли з'являється нова велика мета, яку треба зафіксувати раніше реалізації. Дрібні правки і робочий процес живуть у `DEVLOG.md`, не тут.

Версія документа: 2026-05-21.

---

## Місія

Memory Engine має бути **довготривалою памʼяттю, що працює як справжня людська**: повільно компресує події у пережитий досвід, утримує стабільну особистість роками, відбирає важливе не за регексами, а через ансамбль агентів, забуває рутину, опирається змінам у Ядрі, дає підставу для будь-якого хоста (бот, гра, інший застосунок) звертатись до неї як до однієї памʼяті.

Це не "memory backend для AI-агента". Це **когнітивна модель**, реалізована як Rust-бібліотека з тонкими адаптерами.

---

## Як читати цей документ

Кожен пункт у дорожній карті має статус:

- `[x]` — зроблено. Поруч дата завершення і коротка прив'язка до коду/документа.
- `[~]` — у процесі. Поруч коротке "чому" і де воно живе.
- `[ ]` — на черзі. Без зайвої прив'язки до часу — порядок дає документ, а час — реальна робота.
- `[?]` — відкладено навмисно. Поруч пояснення, чому не зараз.

Якщо пункт виявився неактуальним — не видаляти, поставити `[-]` із причиною. Це частина MemPalace-дисципліни: ми пам'ятаємо, що передумали.

---

## Поточний стан (2026-05-22)

v0.1 Foundation зібрано приблизно на 97%. Тришарова модель активна як цикл, multi-pass compression реалізований (compact memory pass + 5 LLM-проходів + consolidator), Archive → Core bridge працює через LLM-output з gating-правилами. Telegram host уже має розширений набір debug/core/archive-команд, локальний кеш ключів, token telemetry, compact prompt representation і product sleep triggers: token/context budget pressure та scheduled idle sleep о 04:00. Code-only GitHub snapshot уже опублікований. Бракує: довести sleep triggers живим довгим тестом до формального acceptance, прийняти демотацію Core як завершений roadmap-пункт і додати формальний сценарний fixture. Після цих пунктів v0.1 закривається і починається v0.2: memory units, fidelity validation, reflection candidates і forgetting. Embeddings переносяться на пізніший opt-in етап після стабілізації ядра живої памʼяті.

---

## v0.1 — Foundation (тришарова памʼять як живий цикл)

Мета: довести, що архітектура Сесія → sleep → Архів → recall → Core працює end-to-end на реальному хості (Telegram бот) із multi-pass compression і чесним gating промоції в Core.

### Документація

- [x] Стратегія `docs/strategy.md` із принципами і MemPalace-спадкоємністю.
- [x] Архітектура `docs/architecture.md` v0.1 із чотирма рішеннями і трьома каналами Core.
- [x] Контракти `docs/contracts.md` із усіма schema-формами.
- [x] `HISTORY.md` як document of trust за MemPalace-template.
- [x] `docs/local-development.md` із intsталяцією і командами.
- [x] `docs/v0.1-acceptance.md` із критеріями завершення v0.1.
- [x] Цей `docs/roadmap.md`. — 2026-05-19.

### Rust core

- [x] Workspace із `crates/memory_engine` + `crates/python_adapter`.
- [x] Storage trait + FileStorage із atomic write і journal.
- [x] Шар Сесія: `events.jsonl` + `session.json` + людський `session.md`.
- [x] Шар Архів: multi-track `ArchiveEntry` (gist/narrative/facts/quotes + emotional_markers/topic_thread/personal_signals/relational_tone).
- [x] Шар Ядро: `CoreStoreCategory` + `CoreFact` + `upsert_core_fact` + `patch_core_fact`.
- [-] Подієвий sleep-trigger за кількістю повідомлень прибрано з коду, GUI, harness і поточних docs: він віджив як тестовий режим і не відповідає продуктовій моделі памʼяті. — 2026-05-22.
- [x] Product sleep trigger від token/context budget pressure: коли active session/current memory наближається до budget, Telegram host ставить sleep у background queue. — 2026-05-22, `maybe_queue_token_pressure_sleep(...)` у `hosts/telegram_gemini_bot/bot.py`.
- [x] `core_context_package` як єдиний context entry-point для хоста.
- [x] Recall stage 1 (фільтр + scoring) із explanation і debug.
- [x] PendingTask + serializable resume для lifecycle resilience.
- [x] Manifest з auto-write при першому запуску.
- [ ] Partial sleep при ліміті контексту: стискати старші 70%, лишати свіжі 30% у session.
- [x] Token-budget allocator для `core_context_package`: максимум 11k токенів памʼяті в prompt, із розподілом 7k поточна памʼять / 3k стиснута памʼять / 1k Core. Стиснення має зберігати сенси, емоційні маркери і personal signals, а не просто обрізати текст. — 2026-05-20, `CoreContextTokenBudget`, `CoreContextBudgetReport`, `engine_core_context_package_enforces_token_budget_by_layer`.
- [x] Compact prompt representation для chat prompt: звичайний LLM-facing view не містить довгих технічних `event_id` / `archive_id` / `core_fact_id`, schema/source/debug metadata і числових хвостів без потреби. Повний storage/debug JSON лишається для аудитності; prompt view є семантично достатнім і token-економним. — 2026-05-21, спершу `compact_context_package(...)` у Telegram host; 2026-05-30 перенесено в ядро як `render_memory_view(...)`.
- [x] Role-transcript prompt geometry як core-owned prompt view: chat prompt більше не є JSON-дампом, а подає активний діалог як `user:` / `assistant:` transcript, відокремлює current user message, прибирає дублювання `session_recent`/`session_trace` у prompt і дає archive memories короткими bullets. Це виправляє регулярні привітання всередині діалогу й економить prompt tokens. — 2026-05-22, спершу `render_chat_prompt(...)`; 2026-05-30, `crates/memory_engine/src/prompt_view.rs`.
- [x] Compact memory theses для prompt-facing archive recall: окремий `compact_memory_pass` створює plain-text тези "подія -> висновок", `ArchiveEntry.compact_memory` зберігає їх, а `archive_relevant` у chat prompt використовує ці тези замість JSON-проекції full archive tracks. — 2026-05-22, `prompts/compact_memory_pass.md`, `TaskType::CompactMemoryPass`, `ArchiveEntry.compact_memory`, `RecallItem.compact_memory`.
- [x] Core context читає всі free-category файли `memory/core/store/*.json`, а не старий whitelist `profile/preferences/relationship`. `/core`, `/core_forget` і `core_context_package` тепер працюють із категоріями на кшталт `name`, `pet`, `physical_trait`, `food_preference`. — 2026-05-22, `Storage::read_core_store_categories`, `MemoryEngine::core_context_facts`, `MemoryEngine::patch_core_fact`.
- [x] Archive recall scoped by `RecallQuery.session_id`: звичайний `core_context_package` не підтягує archive memories з інших сесій; глобальний archive recall лишається тільки для explicit debug/admin-запиту без `session_id`. — 2026-05-22, `MemoryEngine::recall`, `engine_recall_with_session_id_does_not_leak_other_sessions`.
- [ ] Engine method `forget_core_fact(id)` або `patch_core_fact { status: Deprecated }` (вже існує) як публічний шлях демотації.

### Compression (multi-pass)

- [x] П'ять промптів у `prompts/`: `sleep_emotional_pass`, `sleep_topic_thread_pass`, `sleep_personal_signal_pass`, `sleep_relational_pass`, `sleep_consolidator`.
- [x] `SleepCompressionResult` із усіма чотирма треками і validation 0..1.
- [x] Host orchestration: bot робить 4 паралельних виклики + 1 consolidator, fallback на per-pass треки якщо consolidator щось пропустив.
- [x] Robust `sleep_consolidator` path: JSON extractor, one-shot retry на невалідний JSON і fallback archive з чотирьох успішних треків, щоб sleep не лишав `pending` task через один зламаний JSON. — 2026-05-21.
- [x] Fail-soft specialized sleep passes: якщо emotional/topic/personal/relational або compact pass ловить provider block / no-candidates / parse failure, sleep не зависає в `pending`, а завершується з пустим треком для цього проходу, тегом `pass_failed:<prompt_id>` і повним логом. — 2026-05-23, `safe_execute_sleep_pass_json(...)`.
- [x] `sleep_personal_signal_pass` v2: personal signal визначається критеріями stable user-grounded self-statement / user-specific durable fact, а не whitelist-ом категорій. Категорія є вільним normalized `snake_case`. — 2026-05-21.

### Promotion в шар Ядро

- [x] Канал 1 — `upsert_core_fact` для explicit `/remember`.
- [x] Канал 2 — heuristic tagging у боті (signal-теги піднімають importance_hint без прямого запису в Core).
- [x] Канал 3 — Archive → Core bridge через `personal_signals` із gating: `confidence >= 0.85`, normalized free category, user_source guard, near-duplicate detection.
- [ ] Канал 3b — emotional path: signal із `emotional_marker.strength >= 0.85` має пройти в Core навіть якщо personal_signal pass його не виділив.
- [ ] Канал 4 — reflection-based: окремий `reflection_analyze` PendingTask, що дивиться на накопичений Архів і пропонує candidate beliefs. v0.2.

### Bot host (Telegram + Gemini)

- [x] PyO3 adapter із JSON-boundary API.
- [x] GUI launcher для введення token/key.
- [x] Локальний кеш token/key/config у `runtime/state/secrets.local.json` із кнопкою очищення. — 2026-05-20, `31bd8e1`.
- [x] Multi-pass sleep через `run_sleep()`.
- [x] Archive → Core bridge після resume_sleep_compression.
- [x] Команди `/core`, `/remember text`, `/core_seed`, `/core_update id text`, `/core_forget id`, `/archives`, `/archive_last`, `/archive id`, `/tasks`, `/sleep`, `/recall`, `/models`.
- [x] Heuristic event-теги (`personal_fact_signal`, `name_reference`, `age_reference`, `assistant_identity_reference`, `preference_signal`, `communication_style_signal`, `explicit_memory_request`).
- [x] Aurora persona через `prompts/telegram_chat_system.md`.
- [x] Host-level error UX: Gemini `PROHIBITED_CONTENT` / no-candidates / API errors не показують traceback у Telegram; повний traceback лишається тільки в `bot.log`. — 2026-05-21.
- [x] Token telemetry: `bot.log` і `runtime/logs/token_usage.jsonl` пишуть provider usageMetadata для кожного Gemini-виклику, context budget, estimated baseline без стиснення і sleep compression raw→stored archive / raw→prompt archive / raw→compact memory ratios. — 2026-05-21, оновлено 2026-05-22.
- [x] Scheduled idle sleep у Telegram host: якщо є незаархівовані події, bot запускає sleep о 04:00 локального часу або через конфігурований nightly schedule. — 2026-05-22, `maybe_queue_scheduled_idle_sleep(...)` у `hosts/telegram_gemini_bot/bot.py`.
- [ ] Прийняти `/core_forget <id>` як завершений шлях демотації Core-факта через `patch_core_fact` зі статусом `Deprecated`.
- [x] Стару ідею `/core_refresh` замінено на `/core_seed`: команда seed-ить Core тільки з completed archive `personal_signals`, не з raw text і не regex-backfill. — 2026-05-20, `178bff4`.
- [x] Live-тест на сесії "ім'я → літаки → Маріана → кішка → Європа Юпітера → що ти про мене знаєш?". Маркери успіху: кішка в `emotional_markers`, кішка в `personal_signals`, кішка в Core через bridge, bot згадує її на питання "що знаєш про мене". — 2026-05-21, DEVLOG Запис 52; після тесту відкриті точкові дефекти зафіксовано окремими пунктами.

### Інфраструктура і тести

- [x] Git workspace, `.gitattributes`, `.gitignore` під runtime memory і secrets.
- [x] Orphan-гілка `github-code` для публікації без внутрішніх docs.
- [x] Ліцензія Memory Engine Non-Commercial Public License v0.2 + `LICENSE.md` у корені.
- [x] `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `pytest crates/python_adapter/tests` як обов'язкові gates.
- [x] Unit і integration тести: 17+ у `memory_engine`, 9+ у Python adapter.
- [x] Local conversation harness без Telegram: сценарії `mixed_short`, `topic_switching`, `identity_noise` проганяють живий Gemini chat через той самий prompt/context/sleep шлях, пишуть markdown-звіти і ловлять mid-dialog greeting / stale archive contamination. Це preflight, не заміна live Telegram acceptance. — 2026-05-22, `hosts/telegram_gemini_bot/local_harness.py`.
- [ ] **Сценарний тест базової памʼяті**: end-to-end fixture, що проганяє `ingest` → sleep → `recall` → `core_context_package` і доводить повний цикл.
- [ ] **Сценарний тест production sleep triggers**: окремо перевірити budget-pressure sleep і scheduled idle sleep.
- [ ] **Сценарний тест token budget**: зібрати `core_context_package` на довгій сесії й перевірити, що host prompt вкладається у 11k токенів: до 7k active session/current memory, до 3k archive/compressed memory, до 1k Core.
- [ ] **Сценарний тест compact prompt representation**: перевірити, що фактичний LLM prompt не містить зайвих довгих технічних ID і debug-полів, якщо користувач не викликав debug/core-edit команду.
- [x] Оновити `github-code` гілку під workspace після live-тесту. — 2026-05-23, `882a207`.
- [x] Перший push на GitHub repository (URL від власника). — 2026-05-23, `origin/main` → `882a207`.

---

## v0.2 — Жива памʼять (memory units, reflection, fidelity, forgetting)

Мета: перетворити foundation на справжню "living memory" — пам'ять, що розкладає досвід на атомарні спогади, перевіряє, що стиснення не викривило джерела, сама пропонує стабільні висновки як candidates, але не править Core напряму, і поступово виводить рутину з активного recall.

### v0.2 North Star: Adaptive Stable Core

- [ ] Ядро адаптується через validated memory units, reflection candidates, recall feedback і contested/deprecated lifecycle.
- [ ] Ядро залишається стабільним: жоден агент не може напряму записати або переписати CoreFact без lifecycle/review.
- [ ] Кожен Core-кандидат має мати source evidence, fidelity status, вагу/важливість і зрозуміле пояснення, чому це стабільне знання, а не тимчасовий стан.
- [ ] Нові знання можуть з'являтись під час розмови, але активний Core оновлюється тільки через підтверджений structural path.
- [ ] Суперечності не перезаписують Core одразу: вони переводять факт у `contested`, а вже потім у `deprecated` після review або підтвердження.

### v0.2.1 Reflection foundation

- [ ] Гілка `feature/reflection`: усі зміни reflection робити не в `main`, а в окремій гілці з merge тільки після live-тесту.
- [x] Schema `MemoryUnit`: атомарний спогад із короткою тезою, `event -> conclusion`, source ids, weight, status, fidelity status і короткою локальною label-формою для prompt (`m1`, `m2`, `m3`), щоб не тягнути довгі технічні ID у LLM-контекст. — 2026-05-24, `ArchiveEntry.memory_units`, `MemoryUnit`, `Storage::write_memory_unit`.
- [x] `memory_unit_pass`: LLM-agent розбиває sleep/archive material на стільки змістових одиниць, скільки реально є в розмові. Жодних fixed quotas на кшталт "5-7 тез". — 2026-05-24, `TaskType::MemoryUnitPass`, `prompts/memory_unit_pass.md`.
- [x] `compact_memory` стає prompt-проекцією memory units, а не другим LLM-підсумком. У prompt іде `core_memory` + `long_memory` + `short_memory` + `current_user_message`, з явними межами. — 2026-05-24, `resume_memory_unit_pass`, projection із `MemoryUnit.thesis`.
- [x] Prompt geometry рендериться XML-подібними тегами або рівноцінними чіткими секціями; `telegram_chat_system.md` пояснює різницю між `long_memory`, `short_memory` і `current_user_message`. — 2026-05-24, `render_chat_prompt(...)`; 2026-05-30, канонічний render у `memory_engine::render_memory_view`.
- [ ] `evidence_pack` builder: для важливого unit/candidate бере не всю розмову, а тільки source events і потрібний локальний контекст, достатній для перевірки.
- [ ] Evidence pack збирається програмно: `source_event_ids`, конфігуровані сусіди навколо source events, прямі quotes/evidence, budget target до 1.5k токенів, пріоритет `source > direct evidence > neighboring`.
- [ ] `memory_fidelity_pass`: validator перевіряє важливі/high-risk memory units або Core candidates проти evidence pack/raw/source events і повертає `valid`, `too_broad`, `unsupported`, `distorted`, `missing_key_detail` або `needs_revision`.
- [ ] `reflection_analyze` PendingTask: працює по validated `memory_units`, `compact_memory`, archive tracks і активному Core, повертає candidate beliefs як природні тези з evidence.
- [ ] Сховище `memory/core/candidates/<candidate_id>.json` для lifecycle `candidate` → `ready_for_review` → `confirmed`/`rejected` → `promoted`.
- [ ] Engine/adapter methods для `reflect(...)`, `list_candidates(...)`, `review_candidate(candidate_id, decision)`.
- [ ] Telegram-команди `/reflect`, `/candidates`, `/confirm <id>`, `/reject <id>`.
- [ ] На першій ітерації **без auto-confirm**. Агенти пропонують і перевіряють, але Core змінюється тільки через explicit review.

### v0.2.2 Agent review pipeline

- [ ] `layer_router_pass`: агент пропонує, де має жити validated unit — active archive, core_candidate, routine/low-priority або needs_revision.
- [ ] `core_candidate_reviewer`: дорога/розумна модель на малому evidence pack перевіряє, чи кандидат відповідає структурі Core, і формулює коротку щільну Core-тезу для review.
- [ ] `memory_fidelity_pass` і `core_candidate_reviewer` мають `role_hint: reasoning`; масові sleep/memory-unit passes лишаються `role_hint: balanced`.
- [ ] `compression_reviewer`: агент перевіряє, що unit не занадто роздутий для prompt-facing памʼяті і не втратив ключового сенсу.
- [ ] Усі reviewer agents мають право тільки пропонувати status/revision, але не записувати напряму в Core.

### Фази `feature/reflection`

**Phase A — MemoryUnit foundation.**

- [x] MemoryUnit schema/storage. — 2026-05-24.
- [x] `memory_unit_pass` замість окремого `compact_memory_pass`. — 2026-05-24.
- [x] `compact_memory` як projection із units. — 2026-05-24.
- [x] Prompt geometry з тегами/секціями. — 2026-05-24.
- [ ] Live-test: бот отримує структурований prompt із atomic memory units і не плутає довгу памʼять із поточним діалогом.

**Phase B — Evidence pack + validation.**

- [ ] Evidence pack builder.
- [ ] `memory_fidelity_pass` із `role_hint: reasoning`.
- [ ] Валідація тільки high-weight/high-risk/Core-path units, не всієї рутини.
- [ ] Live-test: у логах видно evidence pack, validator status і випадки `too_broad` / `missing_key_detail` / `valid`.

**Phase C — Candidates + review UX.**

- [ ] CandidateBelief lifecycle.
- [ ] Core candidate reviewer/formulation pass із `role_hint: reasoning`.
- [ ] Contested logic.
- [ ] `/reflect`, `/candidates`, `/confirm`, `/reject`.
- [ ] Live-test: кілька розмов -> `/reflect` -> кандидати -> підтвердження -> Core росте контрольовано.

### Ваги, decay і природний відбір

Стратегія прямо каже: "вага не є одноразовою оцінкою — спогад може ставати важливішим через повторні звернення, зв'язки з новими подіями або участь у формуванні Core-висновку". Зараз recall_count і last_recalled_at оновлюються, але не впливають на вагу. Це треба замкнути.

- [ ] Вага memory unit / archive memory може зростати при recall hit, участі у candidate belief або повторному підтвердженні в reflection. Конкретні коефіцієнти — конфіг, не прихований хардкод.
- [ ] Link bonus у recall scoring: спогади, що мають звʼязок із поточною темою, Core candidate або свіжими подіями, отримують підсилення score.
- [ ] Decay/freshness не має фізично видаляти спогади. Низька актуальність знижує rank і робить unit кандидатом на forgetting review.
- [ ] Захист критичних спогадів: high-weight, emotionally validated або Core-linked units не мають потрапляти в forgetting без окремого reviewer warning.
- [ ] Status lifecycle для CoreFact: `active` → `contested` (накопичено суперечливі спостереження) → `deprecated` (підтверджено). Engine не видаляє факти зі статусом `deprecated` — зберігає як архівний слід.
- [ ] Engine method `engine.contest_core_fact(id, evidence)` — позначає факт як contested, не видаляє.
- [ ] Recall враховує status: contested факти присутні в context але з позначкою, deprecated — не з'являються в context за замовчуванням.
- [ ] `forget_review_pass`: окремий LLM-agent періодично отримує старі `compact_memory` / `memory_units` з freshness/weight/recall_count/fidelity/core-links і рекомендує, які спогади природно позначити як forgotten. Engine не видаляє full archive, а переносить/позначає зі збереженням audit trail.
- [ ] Telegram-команди `/forgotten` і `/remember_back <id>` для audit і повернення забутого в активний recall.

### Schema versioning і міграції

Зараз усі схеми на `.v1`. Перші breaking changes неминучі (embeddings, multi-track refinement, нові поля). Стратегія вимагає чесну migration practice.

- [ ] Перший real migration test: `v1` → `v2` ArchiveEntry (наприклад, рефакторинг embeddings зберігання). Migration code в Rust, не ручне правлення JSON.
- [ ] Journal-захист під час migration (вже передбачено `JournalOperationType::Migration`).
- [ ] HISTORY-запис з reproducibility-anchor: який tag робив migration, як перевірити, що дані ідентичні до міграції.
- [ ] Engine відмовляється стартувати при schema version більшій за підтримувану (вже частково є — задокументувати чітко).

### Partial sleep і session tail

- [ ] `SleepStage1Config.tail_keep_ratio: f64` (default 0.30). Sleep стискає старші 70% подій, свіжі 30% залишає у session як активний хвіст.
- [ ] Live-тест: довга сесія, product sleep trigger спрацьовує, bot **не** втрачає теми останніх кількох повідомлень.

### Vector storage і Recall Stage 2/3 (після готовності ядра)

Векторне сховище не входить у найближчий v0.2 implementation path. Воно додається тільки після того, як Core/reflection/fidelity/forgetting стали стабільними на живих даних.

- [ ] Хост має явний режим privacy/storage: `embeddings_enabled = false` за замовчуванням або як мінімум видима галочка "з vector storage / без vector storage".
- [ ] Без embeddings Memory Engine має лишатися повністю працездатним через Stage 1 recall, compact memory і reflection по structured units.
- [ ] PendingTask тип `ComputeEmbedding` для validated memory units, а не для великих змішаних archive JSON.
- [ ] Storage поля `embedding_model_id` і `embedding: Vec<f64>` заповнюються тільки якщо користувач/власник увімкнув feature.
- [ ] Recall Stage 2 — embedding re-ranking над топ-K кандидатами зі Stage 1. Активується через `Manifest.features.embeddings_enabled`.
- [ ] Recall Stage 3 — LLM rerank через `PendingTask::RecallRerank` із промптом `prompts/recall_rerank.md`. Активується через `Manifest.features.llm_recall_rerank_enabled`.
- [ ] Migration plan: коли embeddings вмикаються на існуючому архіві без embeddings — engine створює batch `ComputeEmbedding` tasks тільки для validated active units, не для forgotten/debug material.

### Стабільність ядра

- [ ] Внутрішній `RwLock<MemoryEngine>` для безпечного multi-thread доступу. Зараз Python GIL вистачає, але це борг із Запису 20 DEVLOG.
- [ ] `recall()` уже не пише на диск при кожному виклику — батчити update recall_count/last_recalled_at у memory і flush'ити периодично.

### Інше

- [ ] Ergonomic Python wrapper: dict in / dict out замість json strings. Через mixed maturin project з `python/memory_engine/__init__.py`.
- [ ] Observation masking для session-tail compression (JetBrains research, грудень 2025) — як кращий метод стиснення живого хвоста, ніж LLM summary.
- [ ] Token budget hint у PendingTask (`budget_hint: { max_input_tokens, max_output_tokens }`) — для прозорого вибору моделі хостом.

---

## v0.3 — Multi-host (Godot, третій проєкт, MCP)

Мета: довести, що ядро **справді** повторно використовуване — інший хост інтегрується через тонкий адаптер, не дублюючи логіки. Плюс інструменти для людини, що обслуговує живу пам'ять.

### Адаптери

- [ ] Godot-адаптер через GDExtension (`crates/godot_adapter/`). Перший хост, що **не** Telegram-бот.
- [ ] Chibigochi-інтеграція: героїня використовує memory engine для довготривалої особистості.
- [ ] Третій проєкт (поки безіменний). Третій хост робить foundation truly universal.
- [ ] MCP-facade як alternative обгортка над тим самим ядром. Для зовнішніх агентів (Claude Code, OpenAI Agents SDK), що хочуть користуватись memory як tool.
- [ ] Стабілізувати JSON contracts: зафіксувати v1 для всіх schemas, прописати migration policy для v2+.

### Diagnostic tools

Стратегія прямо передбачає "діагностичні інструменти для розробника". Без них пам'ять, що живе роками, стає чорною скринькою.

- [ ] Memory inspector CLI (`crates/memory_inspector/`): окремий binary, що показує статистику архіву — кількість entries, distribution ваг, найчастіше recall'ені спогади, давно не recall'ені, candidate beliefs у черзі.
- [ ] Recall debugger: запит recall через CLI з різними фільтрами, видно повний scoring breakdown для топ-10 результатів.
- [ ] Core fact viewer: побачити всі Core facts по категорії з історією status changes (active → contested → deprecated).
- [ ] PendingTask viewer: побачити, які LLM-задачі очікують виконання, що в attempts, що в last_error.

### Backup і відновлення

Стратегія: "потрібні надійне збереження, резервне відновлення, manifest із версіями схем, перевірка цілісності і міграції". Backup-flow не може бути "копіюємо папку".

- [ ] Документований backup flow: snapshot цілої `memory/` директорії з checksum для manifest і кожного persistence layer.
- [ ] Документований restore flow: перевірка integrity, journal replay для незавершених операцій, migration на актуальну схему якщо backup старший.
- [ ] CLI команда `memory_inspector backup` і `memory_inspector restore`.

---

## v1.0 — Public release і benchmarks

Мета: відкрита публікація з reproducibility-перевіркою на стандартних memory benchmarks.

- [ ] Reproducibility-перевірка на LongMemEval (R@5, R@10 у raw mode + після Stage 2 + після Stage 3).
- [ ] Опціонально LoCoMo, з чесним коментарем щодо методології.
- [ ] Порівняння з open-source memory libraries (MemGPT, Letta, Mem0, Zep, MemPalace) — чесне, з посиланнями на pubished numbers, без cherry-picking. Цей research відкладено до моменту стабілізації.
- [ ] Public README, який не претендує на більше, ніж вимірюваний результат.
- [ ] HISTORY-запис із tag, dataset, seed, шлях до result-файлів.

---

## Принципи, що тримати на всіх етапах

Ці правила діють весь час. Якщо роадмеп пропонує крок, що їх порушує — крок повертається на доопрацювання.

1. **Ядро не знає провайдерів, моделей, ключів і prompts_dir.** Усе живе в host-конфізі.
2. **Не вигадувати наперед.** Промпт/схема/файл з'являється рівно тоді, коли реальний код його використовує.
3. **Усе агентивно, ніяких regex-extractor.** Pattern decision — у LLM. Host робить gating, не extraction.
4. **MemPalace-дисципліна довіри.** Будь-яка зміна, що може вплинути на стабільні факти або recall behavior — через `HISTORY.md` із reproducibility-anchor для benchmark-claims.
5. **Шар Ядро — повільний.** Потрапити в Core дорого. Випасти з Core — ще дорожче. Demotion через явні правила і час.
6. **Агенти не правлять істину напряму.** Агенти створюють, стискають, критикують, валідують і маршрутизують. Core змінюється тільки через lifecycle і підтвердження.
7. **Стиснення має мати fidelity check.** Якщо compressed memory unit не відповідає raw/source events, він не може стати нормальним archive/Core material без revision.
8. **Vector storage — opt-in і пізніше.** Embeddings додаються після стабілізації ядра живої памʼяті; користувач/власник має мати режим без vector storage.
9. **Live перевага над теорією.** Перед тим як писати v0.2, поточний v0.1 має реально працювати на живому хості. Без live-тесту наступний рівень — фантазія.
10. **Терміни.** `ядро системи` = вся Rust-бібліотека. `шар Ядро` = стабільний шар. Не плутати.

---

## Як цей документ оновлюється

**Кодекс (і будь-яка модель), що завершує пункт:**

1. Знайти пункт у відповідному розділі.
2. Поміняти `[ ]` на `[x]`.
3. Додати в кінці рядка дату завершення і коротку прив'язку: коміт SHA, шлях до файлу або тесту.
4. Якщо пункт виявився більшим, ніж очікувалось, і породив підпункти — розбити його на checklist у тому ж місці.
5. Якщо пункт виявився непотрібним — `[-]` із одним реченням причини.
6. Якщо ця зміна — breaking change або відкликання попереднього твердження — окремий запис у `HISTORY.md`.
7. `DEVLOG.md` отримує робочу нотатку: що саме зроблено, як перевіряли, які борги залишились. Roadmap фіксує **результат**, DEVLOG фіксує **процес**.

**Власник проєкту**, коли додає нову велику мету:

1. Знайти відповідний v0.x розділ або створити новий, якщо це наступний рівень.
2. Записати ціль у форматі `[ ] <короткий пункт>`. Якщо пункт потребує обґрунтування — додати один рядок під ним.
3. Якщо нова мета впливає на стратегію або архітектуру — синхронізувати відповідні документи.

Документ не змагається з real-time tracking систем. Він — компас. Дрібні task'и живуть у `DEVLOG.md`. Тут — тільки видимі віхи.
