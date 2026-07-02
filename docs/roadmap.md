# Roadmap Memory Engine

Цей документ — **жива дорожня карта** проєкту. Він не дублює стратегію (`docs/strategy.md`) чи архітектуру (`docs/architecture.md`). Він відповідає на одне питання: **куди ми йдемо і де зараз стоїмо**.

Документ оновлюється у двох випадках: коли пункт стає зробленим (відмітка `[x]` + дата + посилання на коміт або файл), і коли з'являється нова велика мета, яку треба зафіксувати раніше реалізації. Дрібні правки і робочий процес живуть у `DEVLOG.md`, не тут.

Версія документа: 2026-06-10.

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

## Поточний стан (2026-07-02)

Після v0.3: ліцензію повернуто на non-commercial (merge у main 2026-07-02); Chibigochi має async product loop і реальну інтеграцію бібліотеки в Godot-проєкт власника; prompt view отримав derived time labels (`current_time`, вік archive items, денні маркери) — див. секцію «Час у памʼяті» нижче і HISTORY 2026-07-02. Наступні великі напрями: мультиспікерна геометрія (гілки 1–3 нижче) і продуктова Chibigochi-інтеграція.

## Стан на 2026-06-10

v0.2 закрито як повний цикл живої памʼяті: session → sleep → Archive/MemoryUnit → recall/context → fidelity → reflection candidates → review → Core/contested → forgetting → remember_back. Формальний acceptance-anchor — `crates/memory_engine/tests/living_memory_cycle.rs` і `docs/v0.2-acceptance.md`; публічний реліз позначено тегом `v0.2.0`.

Після v0.2 короткий audit-cleanup перед v0.3 закрито: A1/A2/A3/A4 step 1/A4 step 3/B3/A5/A7/B4/B2 виконані. Journal/recovery scope (A3) закрито як рішення: journal лишається storage primitive для майбутніх migration/recovery-heavy операцій, runtime sleep спирається на durable `SleepRun` checkpoints. `engine.rs` більше не є монолітом: доменні частини винесені в `crates/memory_engine/src/engine/`.

v0.3 host-conformance також закрито: direct/local, Telegram-local і Godot-headless проходять однаковий automated scenario; Godot acceptance підтверджено реальним Godot 4.6 stable console. Це доводить, що ядро використовується як reusable library із тонкими adapters, а не як Telegram-specific bot core. Наступне меню після v0.3: Chibigochi/Godot integration, Telegram-live smoke як транспортний regression, reviewer-pass, opt-in vector recall alignment, unit-level recall counters, diagnostics/backup.

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
- [x] Шар Сесія: active `events.jsonl` + archived raw event segments + `session.json` + людський `session.md`.
- [x] Шар Архів: multi-track `ArchiveEntry` (gist/narrative/facts/quotes + emotional_markers/topic_thread/personal_signals/relational_tone).
- [x] Шар Ядро: `CoreStoreCategory` + `CoreFact` + `upsert_core_fact` + `patch_core_fact`.
- [-] Подієвий sleep-trigger за кількістю повідомлень прибрано з коду, GUI, harness і поточних docs: він віджив як тестовий режим і не відповідає продуктовій моделі памʼяті. — 2026-05-22.
- [x] Product sleep trigger від token/context budget pressure: коли active session/current memory наближається до budget, Telegram host ставить sleep у background queue. — 2026-05-22, `maybe_queue_token_pressure_sleep(...)` у `hosts/telegram_gemini_bot/bot.py`.
- [x] `core_context_package` як єдиний context entry-point для хоста.
- [x] Recall stage 1 (фільтр + scoring) із explanation і debug.
- [x] PendingTask + serializable resume для lifecycle resilience.
- [x] Manifest з auto-write при першому запуску.
- [x] Partial sleep при ліміті контексту: стискати старшу частину unarchived window, лишати свіжий active tail у session. — 2026-05-24, `SleepStage1Config.active_tail_ratio` default `0.30`, `partial_sleep_min_events`, `engine_sleep_preserves_configured_active_tail`.
- [x] Token-budget allocator для `core_context_package`: максимум 11k токенів памʼяті в prompt, із розподілом 7k поточна памʼять / 3k стиснута памʼять / 1k Core. Стиснення має зберігати сенси, емоційні маркери і personal signals, а не просто обрізати текст. — 2026-05-20, `CoreContextTokenBudget`, `CoreContextBudgetReport`, `engine_core_context_package_enforces_token_budget_by_layer`.
- [x] Compact prompt representation для chat prompt: звичайний LLM-facing view не містить довгих технічних `event_id` / `archive_id` / `core_fact_id`, schema/source/debug metadata і числових хвостів без потреби. Повний storage/debug JSON лишається для аудитності; prompt view є семантично достатнім і token-економним. — 2026-05-21, спершу `compact_context_package(...)` у Telegram host; 2026-05-30 перенесено в ядро як `render_memory_view(...)`.
- [x] Role-transcript prompt geometry як core-owned prompt view: chat prompt більше не є JSON-дампом, а подає активний діалог як `user:` / `assistant:` transcript, відокремлює current user message, прибирає дублювання `session_recent`/`session_trace` у prompt і дає archive memories короткими bullets. Це виправляє регулярні привітання всередині діалогу й економить prompt tokens. — 2026-05-22, спершу `render_chat_prompt(...)`; 2026-05-30, `crates/memory_engine/src/prompt_view.rs`.
- [x] Compact memory theses для prompt-facing archive recall: окремий `compact_memory_pass` створює plain-text тези "подія -> висновок", `ArchiveEntry.compact_memory` зберігає їх, а `archive_relevant` у chat prompt використовує ці тези замість JSON-проекції full archive tracks. — 2026-05-22, `prompts/compact_memory_pass.md`, `TaskType::CompactMemoryPass`, `ArchiveEntry.compact_memory`, `RecallItem.compact_memory`.
- [x] Core context читає всі free-category файли `memory/core/store/*.json`, а не старий whitelist `profile/preferences/relationship`. `/core`, `/core_forget` і `core_context_package` тепер працюють із категоріями на кшталт `name`, `pet`, `physical_trait`, `food_preference`. — 2026-05-22, `Storage::read_core_store_categories`, `MemoryEngine::core_context_facts`, `MemoryEngine::patch_core_fact`.
- [x] Archive recall scoped by `RecallQuery.session_id`: звичайний `core_context_package` не підтягує archive memories з інших сесій; глобальний archive recall лишається тільки для explicit debug/admin-запиту без `session_id`. — 2026-05-22, `MemoryEngine::recall`, `engine_recall_with_session_id_does_not_leak_other_sessions`.
- [x] Engine method `patch_core_fact { status: Deprecated }` як публічний шлях демотації Core-факта. — 2026-05-20, `MemoryEngine::patch_core_fact`.

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
- [x] Канал 4 — reflection-based: окремий `reflection_analyze` PendingTask дивиться на validated memory units і активний Core, пропонує candidate beliefs, але не пише в Core без review. — 2026-06-01, `TaskType::ReflectionAnalyze`, `CandidateBelief`, `review_candidate`.

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
- [x] Прийняти `/core_forget <id>` як завершений шлях демотації Core-факта через `patch_core_fact` зі статусом `Deprecated`. — 2026-05-20.
- [x] Стару ідею `/core_refresh` замінено на `/core_seed`: команда seed-ить Core тільки з completed archive `personal_signals`, не з raw text і не regex-backfill. — 2026-05-20, `178bff4`.
- [x] Live-тест на сесії "ім'я → літаки → Маріана → кішка → Європа Юпітера → що ти про мене знаєш?". Маркери успіху: кішка в `emotional_markers`, кішка в `personal_signals`, кішка в Core через bridge, bot згадує її на питання "що знаєш про мене". — 2026-05-21, DEVLOG Запис 52; після тесту відкриті точкові дефекти зафіксовано окремими пунктами.

### Інфраструктура і тести

- [x] Git workspace, `.gitattributes`, `.gitignore` під runtime memory і secrets.
- [x] Orphan-гілка `github-code` для публікації без внутрішніх docs.
- [x] Ліцензія Memory Engine Non-Commercial Public License v0.2 + `LICENSE.md` у корені.
- [x] `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `pytest crates/python_adapter/tests` як обов'язкові gates.
- [x] Unit і integration тести: 17+ у `memory_engine`, 9+ у Python adapter.
- [x] Local conversation harness без Telegram: сценарії `mixed_short`, `topic_switching`, `identity_noise` проганяють живий Gemini chat через той самий prompt/context/sleep шлях, пишуть markdown-звіти і ловлять mid-dialog greeting / stale archive contamination. Це preflight, не заміна live Telegram acceptance. — 2026-05-22, `hosts/telegram_gemini_bot/local_harness.py`.
- [x] **Сценарний тест базової памʼяті**: end-to-end fixture, що проганяє `ingest` → sleep → `recall` → `core_context_package` і доводить повний цикл. — 2026-06-01, `crates/memory_engine/tests/living_memory_cycle.rs`.
- [ ] **Сценарний тест production sleep triggers**: окремо перевірити budget-pressure sleep і scheduled idle sleep.
- [ ] **Сценарний тест token budget**: зібрати `core_context_package` на довгій сесії й перевірити, що host prompt вкладається у 11k токенів: до 7k active session/current memory, до 3k archive/compressed memory, до 1k Core.
- [ ] **Сценарний тест compact prompt representation**: перевірити, що фактичний LLM prompt не містить зайвих довгих технічних ID і debug-полів, якщо користувач не викликав debug/core-edit команду.
- [x] Оновити `github-code` гілку під workspace після live-тесту. — 2026-05-23, `882a207`.
- [x] Перший push на GitHub repository (URL від власника). — 2026-05-23, `origin/main` → `882a207`.

---

## v0.2 — Жива памʼять (memory units, reflection, fidelity, forgetting)

Мета: перетворити foundation на справжню "living memory" — пам'ять, що розкладає досвід на атомарні спогади, перевіряє, що стиснення не викривило джерела, сама пропонує стабільні висновки як candidates, але не править Core напряму, і поступово виводить рутину з активного recall.

### v0.2 North Star: Adaptive Stable Core

- [x] Ядро адаптується через validated memory units, reflection candidates, recall feedback і contested/deprecated lifecycle. — 2026-06-01, `living_memory_cycle.rs`.
- [x] Ядро залишається стабільним: жоден агент не може напряму записати або переписати CoreFact без lifecycle/review. — 2026-06-01, reflection/forgetting/contested APIs.
- [x] Кожен Core-кандидат має мати source evidence, fidelity status, вагу/важливість і зрозуміле пояснення, чому це стабільне знання, а не тимчасовий стан. — 2026-06-01, `CandidateBelief.source_memory_unit_ids`, fidelity-gated reflection inputs.
- [x] Нові знання можуть з'являтись під час розмови, але активний Core оновлюється тільки через підтверджений structural path. — 2026-06-01, Archive-to-Core bridge або `review_candidate(approved)`.
- [x] Суперечності не перезаписують Core одразу: вони переводять факт у `contested`, а вже потім у `deprecated` після review або підтвердження. — 2026-06-01, `review_candidate(approved)` contested path.

### v0.2.1 Reflection foundation

- [ ] Гілка `feature/reflection`: усі зміни reflection робити не в `main`, а в окремій гілці з merge тільки після live-тесту.
- [x] Schema `MemoryUnit`: атомарний спогад із короткою тезою, `event -> conclusion`, source ids, weight, status, fidelity status і короткою локальною label-формою для prompt (`m1`, `m2`, `m3`), щоб не тягнути довгі технічні ID у LLM-контекст. — 2026-05-24, `ArchiveEntry.memory_units`, `MemoryUnit`, `Storage::write_memory_unit`.
- [x] `memory_unit_pass`: LLM-agent розбиває sleep/archive material на стільки змістових одиниць, скільки реально є в розмові. Жодних fixed quotas на кшталт "5-7 тез". — 2026-05-24, `TaskType::MemoryUnitPass`, `prompts/memory_unit_pass.md`.
- [x] `compact_memory` стає prompt-проекцією memory units, а не другим LLM-підсумком. У prompt іде `core_memory` + `long_memory` + `short_memory` + `current_user_message`, з явними межами. — 2026-05-24, `resume_memory_unit_pass`, projection із `MemoryUnit.thesis`.
- [x] Prompt geometry рендериться XML-подібними тегами або рівноцінними чіткими секціями; `telegram_chat_system.md` пояснює різницю між `long_memory`, `short_memory` і `current_user_message`. — 2026-05-24, `render_chat_prompt(...)`; 2026-05-30, канонічний render у `memory_engine::render_memory_view`.
- [x] `evidence_pack` builder: для важливого unit/candidate бере не всю розмову, а тільки source events і потрібний локальний контекст, достатній для перевірки. — 2026-05-31, `EvidencePack`, `MemoryEngine::build_evidence_pack`.
- [x] Evidence pack збирається програмно: `source_event_ids`, конфігуровані сусіди навколо source events, прямі unit evidence, budget target до 1.5k токенів, пріоритет `source > neighboring under budget`. — 2026-05-31.
- [x] `memory_fidelity_pass`: validator перевіряє memory units проти evidence pack/raw/source events і повертає `valid`, `too_broad`, `unsupported`, `distorted`, `missing_key_detail` або `needs_revision`. — 2026-05-31, `TaskType::MemoryFidelityPass`, `FidelityReview`, `/fidelity`.
- [x] `reflection_analyze` PendingTask: працює по validated `memory_units`, compact archive summaries і активному Core, повертає candidate beliefs як природні тези з evidence. — 2026-06-01, `TaskType::ReflectionAnalyze`, `prompts/reflection_analyze.md`.
- [x] Сховище `memory/core/candidates/<candidate_id>.json` для lifecycle `ready_for_review` → `rejected`/`promoted`. `confirmed` як окремий auto-confirm стан свідомо не використовується у першій ручній ітерації. — 2026-06-01.
- [x] Engine/adapter methods для `reflect(...)`, `list_candidates(...)`, `review_candidate(candidate_id, decision)`. — 2026-06-01, `begin_reflection_analysis`, `submit_reflection_response`, `list_candidates`, `review_candidate`.
- [x] Telegram-команди `/reflect`, `/candidates`, `/confirm <id>`, `/reject <id>`. — 2026-06-01.
- [x] На першій ітерації **без auto-confirm**. Агенти пропонують і перевіряють, але Core змінюється тільки через explicit review. — 2026-06-01.

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

- [x] Evidence pack builder. — 2026-05-31.
- [x] `memory_fidelity_pass` із `role_hint: reasoning`. — 2026-05-31.
- [x] Автоматична маршрутизація validation тільки для high-weight/high-risk/Core-path units, не всієї рутини. `finish_sleep_run` повертає `fidelity_requests`, які host виконує як звичайні `LlmRequest`; manual/debug шлях `/fidelity <memory_unit_id>` лишається. — 2026-05-31, `SleepOutcome.fidelity_requests`, `FidelityConfig.auto_validate_*`.
- [x] Manual live-test: evidence pack малий і bounded, reasoning validator повертає осмислені verdict-и на чесному (`valid`) і викривленому (`unsupported`) unit. — 2026-05-31, DEVLOG Запис 84.

**Phase C — Candidates + review UX.**

- [x] CandidateBelief lifecycle. — 2026-06-01, first manual lifecycle `ready_for_review` → `rejected`/`promoted`.
- [ ] Core candidate reviewer/formulation pass із `role_hint: reasoning`.
- [x] Contested logic. — 2026-06-01, approved candidates may carry `contradicted_core_fact_ids`; `review_candidate(approved)` marks those active Core facts as `contested`, keeps them in context with a contested marker, and promotes the new candidate only after manual review. Live-check passed with real Gemini: Berlin active Core fact -> Kyiv candidate -> old fact `contested`, new fact `active`.
- [x] `/reflect`, `/candidates`, `/confirm`, `/reject`. — 2026-06-01.
- [x] Live-test: кілька розмов -> `/reflect` -> кандидати -> підтвердження -> Core росте контрольовано. — 2026-06-01, Telegram: `/candidates` був порожній до `/reflect`, `/reflect` створив `candidate_1780300762881461500_3` у статусі `ready_for_review`, `/candidates` показав того самого кандидата; Core лишився незмінним без `/confirm`.

### Ваги, decay і природний відбір

Стратегія прямо каже: "вага не є одноразовою оцінкою — спогад може ставати важливішим через повторні звернення, зв'язки з новими подіями або участь у формуванні Core-висновку". Зараз recall_count і last_recalled_at оновлюються, але не впливають на вагу. Це треба замкнути.

Update 2026-06-10: Stage 1 recall already uses `recall_count` and `last_recalled_at` as a soft rank boost, and B3 changed persistence so those counters are buffered and flushed instead of rewriting archive files on every recall.

- [ ] Вага memory unit / archive memory може зростати при recall hit, участі у candidate belief або повторному підтвердженні в reflection. Конкретні коефіцієнти — конфіг, не прихований хардкод.
- [x] Stage 1 recall враховує `recall_count` і `last_recalled_at` як м'який rank boost без фізичного переписування `weight`: часто або недавно згадані archive memories трохи краще переживають decay. — 2026-05-30, `RecallStage1Config.recall_count_log_bonus`, `recent_recall_bonus`, `max_recall_boost_factor`.
- [ ] Link bonus у recall scoring: спогади, що мають звʼязок із поточною темою, Core candidate або свіжими подіями, отримують підсилення score.
- [x] Decay/freshness не має фізично видаляти спогади. Низька актуальність знижує rank і робить unit кандидатом на forgetting review. — 2026-05-30, effective freshness у Stage 1 recall з configurable half-life; повне `forget_review_pass` лишається окремим пунктом нижче.
- [x] Захист критичних спогадів: high-weight, emotionally validated або Core-linked units не мають потрапляти в forgetting без окремого reviewer warning. — 2026-06-01, `feature/forgetting` додає hard-gate: protected unit не забувається навіть якщо LLM рекомендує `forget`, protection re-check виконується на submit.
- [x] Status lifecycle для CoreFact: `active` → `contested` (накопичено суперечливі спостереження) → `deprecated` (підтверджено). Engine не видаляє факти зі статусом `deprecated` — зберігає як архівний слід. — 2026-06-01, `review_candidate(approved)` реалізує `active` → `contested`; `patch_core_fact` / `/core_forget` покриває manual demotion у `deprecated`.
- [ ] Engine method `engine.contest_core_fact(id, evidence)` — позначає факт як contested, не видаляє.
- [x] Recall враховує status: contested факти присутні в context але з позначкою, deprecated — не з'являються в context за замовчуванням. — 2026-06-01, `render_memory_view` маркує contested Core facts, `core_context_package` приховує deprecated.
- [x] `forget_review_pass`: окремий LLM-agent отримує старі low-signal `memory_units` з age/weight/archive-level recall proxy/fidelity/core-links і рекомендує, які спогади природно позначити як forgotten. Engine не видаляє full archive; v1 робить reversible status flip і rebuild `compact_memory`. — 2026-06-01, `feature/forgetting`.
- [x] Telegram-команди `/forgotten` і `/remember_back <id>` для audit і повернення забутого в активний recall. — 2026-06-01, thin host wrappers over core API.
- [x] Live-test forgetting v1: реальний `forget_review_pass` на scratch/runtime має забути рутину, не чіпати protected units, і `/remember_back` має повернути тезу в prompt-facing compact memory. — 2026-06-01, scratch live-check через cached Gemini: routine -> `Forgotten`; richer check: Core-linked unit protected at submit re-check, high-weight unit stayed active, `/remember_back` restored compact memory.
- [x] Formal v0.2 end-to-end acceptance fixture: `ingest -> sleep -> Archive/MemoryUnit -> recall/context -> fidelity -> reflection -> Core -> contested -> forgetting -> remember_back`. — 2026-06-01, `crates/memory_engine/tests/living_memory_cycle.rs`, `docs/v0.2-acceptance.md`.

### Schema versioning і міграції

Зараз усі схеми на `.v1`. Перші breaking changes неминучі (embeddings, multi-track refinement, нові поля). Стратегія вимагає чесну migration practice.

- [ ] Перший real migration test: `v1` → `v2` ArchiveEntry (наприклад, рефакторинг embeddings зберігання). Migration code в Rust, не ручне правлення JSON.
- [ ] Journal-захист під час migration (вже передбачено `JournalOperationType::Migration`).
- [ ] HISTORY-запис з reproducibility-anchor: який tag робив migration, як перевірити, що дані ідентичні до міграції.
- [ ] Engine відмовляється стартувати при schema version більшій за підтримувану (вже частково є — задокументувати чітко).

### Partial sleep, active tail і raw-event rotation

- [x] `SleepStage1Config.active_tail_ratio: f64` (default 0.30). Sleep стискає старшу частину unarchived window, свіжий tail залишає активним у session. — 2026-05-24, `engine_sleep_preserves_configured_active_tail`.
- [x] Raw-event rotation: після успішного `finish_sleep_run` raw-події, покриті Complete-архівами й старші за active tail, переносяться з active `events.jsonl` у `sessions/<session_id>/archived/events-<NNN>.jsonl`; evidence/core bridge читають active + archived segments з dedupe. — 2026-06-10, `Storage::rotate_session_events`, `storage_rotation_roundtrips_archived_session_events`.
- [x] Archived-event coverage cache: `SessionMetadata.archived_to`, `archived_event_ids` і `archived_event_index_complete` прибирають hot-path full archive scan із `core_context_package`/sleep selection; legacy metadata перебудовується з Complete archives. — 2026-06-10, `engine_context_rebuilds_legacy_archived_event_index`.
- [ ] Live-тест: довга сесія, product sleep trigger спрацьовує, bot **не** втрачає теми останніх кількох повідомлень.

### Vector storage і Recall Stage 2/3 (після готовності ядра)

Векторне сховище не входить у v0.3 close. Робочий research-документ зафіксовано в `docs/research/vector-recall.md`, але перед кодом він має пройти alignment із поточною архітектурою: індексувати validated active `MemoryUnit`, а не великі archive JSON; core приймає vectors, host рахує embeddings; recall feedback має використовувати buffered stats path.

- [ ] **Explicit-залежність:** vector storage вмикається ТІЛЬКИ після атрибуції тез (мультиспікерні гілки 1–2 у секції v0.3). Безсубʼєктні тези («користувач любив мотоцикли»), впечені в embeddings, означають migration по всьому embedded-архіву (Запис 117).
- [ ] Хост має явний режим privacy/storage: `embeddings_enabled = false` за замовчуванням або як мінімум видима галочка "з vector storage / без vector storage".
- [ ] Без embeddings Memory Engine має лишатися повністю працездатним через Stage 1 recall, compact memory і reflection по structured units.
- [ ] PendingTask тип `ComputeEmbedding` для validated active memory units, а не для великих змішаних archive JSON.
- [ ] Storage поля `embedding_model_id` і `embedding: Vec<f64>` заповнюються тільки якщо користувач/власник увімкнув feature.
- [ ] Recall Stage 2 — embedding re-ranking над топ-K кандидатами зі Stage 1. Активується через `Manifest.features.embeddings_enabled`.
- [ ] Recall Stage 3 — LLM rerank через `PendingTask::RecallRerank` із промптом `prompts/recall_rerank.md`. Активується через `Manifest.features.llm_recall_rerank_enabled`.
- [ ] Migration plan: коли embeddings вмикаються на існуючому архіві без embeddings — engine створює batch `ComputeEmbedding` tasks тільки для validated active units, не для forgotten/debug material.

### Стабільність ядра

- [x] Fine-grained in-process locking замість одного `RwLock`: `MemoryEngine` має `&self` API, PyO3 відпускає GIL на важких викликах, `LockRegistry` серіалізує ресурси `session:<id>`, `core:<category>`, `candidate:<id>` із lock-ordering без дедлоків. — 2026-05-29, `feature/concurrency`, 1000-session stress test.
- [x] `recall()` уже не пише на диск при кожному виклику — `recall_count` / `last_recalled_at` буферизуються в memory, scoring бачить pending deltas, `flush_recall_stats()` пише батчем. — 2026-06-10.

### Інше

- [ ] Ergonomic Python wrapper: dict in / dict out замість json strings. Через mixed maturin project з `python/memory_engine/__init__.py`.
- [ ] Observation masking для session-tail compression (JetBrains research, грудень 2025) — як кращий метод стиснення живого хвоста, ніж LLM summary.
- [ ] Token budget hint у PendingTask (`budget_hint: { max_input_tokens, max_output_tokens }`) — для прозорого вибору моделі хостом.

---

## v0.3 — Multi-host (Godot, третій проєкт, MCP)

Мета: довести, що ядро **справді** повторно використовуване — інший хост інтегрується через тонкий адаптер, не дублюючи логіки. Плюс інструменти для людини, що обслуговує живу пам'ять.

### Host conformance

- [x] Зафіксувати v0.3 acceptance як host-conformance, а не ручне клікання власником. Перший документ: `docs/v0.3-acceptance.md`. — 2026-06-10.
- [x] Direct/local conformance driver через Python adapter із deterministic fake LLM: `tests/host_conformance/host_conformance.py --host direct`. Це baseline для всіх наступних хостів. — 2026-06-10.
- [x] Telegram-local driver має пройти той самий сценарій без Telegram transport. — 2026-06-10, `tests/host_conformance/host_conformance.py --host telegram-local`.
- [?] Telegram-live smoke driver має перевіряти тільки транспорт і один короткий live-LLM шлях, не повний memory acceptance руками. Відкладено з v0.3 close: `telegram-local` вже покриває реальний `bot.py` memory path без Telegram API, тому live smoke є regression/operations task, не acceptance gate.
- [x] Godot-headless driver має пройти той самий conformance scenario до будь-якого polished Godot UI. — 2026-06-10: `tests/host_conformance/host_conformance.py --host godot-headless` passed with real Godot 4.6 stable console, `memory_units=3`, `core_facts=3`.
- [x] v0.3 close: кожен прийнятий host проходить однаковий scenario і доводить, що memory policy лишається в Rust core. Accepted scope: direct/local, Telegram-local, Godot-headless. — 2026-06-10, `docs/v0.3-acceptance.md`, `tests/host_conformance/host_conformance.py`.

### Адаптери

- [x] Godot-адаптер через GDExtension (`crates/godot_adapter/`). Перший хост, що **не** Telegram-бот. — 2026-06-10, thin JSON-boundary GDExtension crate compiles with `cargo check -p godot_adapter`; live Godot runtime acceptance tracked in Host conformance above.
- [~] Chibigochi-інтеграція: героїня використовує memory engine для довготривалої особистості. Перший headless product-host spike є в `hosts/chibigochi_spike/`: Godot host object проходить `user text -> context -> reply -> sleep -> restart -> persisted Core/context recall` через `tests/host_conformance/host_conformance.py --host chibigochi-spike`. Мінімальна Godot scene/UI wrapper також є (`main_scene.tscn`, `main_scene.gd`) і проходить `--host chibigochi-ui`. Production-shaped LLM bridge теж є: `chibigochi_http_llm_bridge.gd` виконує chat/sleep/fidelity через HTTP `LlmRequest -> text` proxy і проходить `--host chibigochi-llm-bridge`; реальний Gemini executor є в `chibigochi_gemini_proxy.py`. Async product-loop smoke є як `--host chibigochi-product-loop`: сцена використовує async HTTP bridge, UI busy/error states і restart recall. Відкритими лишаються polished gameplay/character UX і production packaging.
- [ ] Третій проєкт: природний кандидат — груповий Telegram-чат (див. «Мультиспікерна геометрія» нижче); він доводить universality сильніше за ще один лінійний хост.
- [ ] MCP-facade як alternative обгортка над тим самим ядром. Для зовнішніх агентів (Claude Code, OpenAI Agents SDK), що хочуть користуватись memory як tool.
- [ ] Стабілізувати JSON contracts: зафіксувати v1 для всіх schemas, прописати migration policy для v2+.

### Час у памʼяті (prompt-facing time labels)

Джерело рішення: `docs/research/memory-time-perception-2026-07-02.md`. Принцип: відносні мітки ніколи не зберігаються — обчислюються при рендері з абсолютних timestamp-ів; тому «оновлення після простою» не існує як задача.

- [x] `current_time:` рядок у `<state>` prompt view — точка відліку для моделі. — 2026-07-02, `prompt_view.rs`.
- [x] Мітка віку на archive items у `<long_memory>` (`- [yesterday | 0.88] ...`) з `RecallItem.time_range.end`; драбина: today / yesterday / N days ago / earlier this month / last month / N months ago / over a year ago (календарні дні). — 2026-07-02.
- [x] Денні маркери (`[yesterday]`) у older trace `<short_memory>`; свіжий recent tail без міток. — 2026-07-02.
- [x] `CoreContextRequest.utc_offset_minutes` + `clock_untrusted` (serde default, без міграцій); Telegram host передає локальний offset. — 2026-07-02, HISTORY-запис.
- [x] Деградація: недовірений годинник / майбутні timestamps → мітки опущені + package note. — 2026-07-02, `memory_view_omits_labels_when_clock_is_untrusted`.
- [ ] Live-перевірка на Telegram host: «коли я тобі казав про X?» після кількох днів історії.
- [ ] Хостова опортуністична корекція часу за `Date`-заголовком наявного HTTP-трафіку (без окремих time-запитів) — коли зʼявиться перший хост із недовіреним годинником.

### Мультиспікерна геометрія (кандидат на «третій проєкт»)

Джерело рішення: `docs/research/multi-speaker-geometry-2026-06-12.md` + доповнення 1–8 у DEVLOG Запис 117. Принцип: геометрія = адресація (роутинг хоста, в ядро не потрапляє) / атрибуція (єдине, що переживає стискання — як імена у змісті) / посилання (працюють при sleep-дистиляції і вмирають). Лінійні хости не змінюються взагалі; жодних «групових режимів» у ядрі.

**Гілка 1 — контракти + рендер + recall за подією:**

- [ ] Опціональний `speaker: { id, name }` в `IngestEvent`/`StoredEvent` (serde default; відсутність = поточна бінарна поведінка). `links` вже типізовані — для `reply_to` потрібна лише конвенція kind у `contracts.md` + HISTORY.
- [ ] `render_memory_view`: події зі speaker рендеряться імʼям (`Жека: ...`) замість `user:`; без speaker — байтово поточна поведінка.
- [ ] Рендер сирого матеріалу sleep-пасів: імена спікерів + компактні reply-маркери (`[Жека, у відповідь на №12]`, локальні номери вікна).
- [ ] Evidence pack теж рендерить speaker (`evidence_event_from_stored`) — інакше fidelity validator не може перевірити авторство (Запис 117, п.1).
- [ ] `recall_by_event_id(session_id, event_id)`: покрита подія → архівний запис через metadata cache; жива → ознака «в активному хвості». Перед цим закрити storage-гальмо: `archive_entry_path_by_id` сканує весь каталог — потрібен path-hint або map `event_id -> archive_id` у `SessionMetadata` (перебудовний кеш; Запис 117, п.4).
- [ ] Фаза 1 захисту Core: auto-bridge вимикається для сесій із кількома speaker — **у ядрі**, не в хості («що потрапляє в Core» — у списку заборон хоста; Запис 117, п.3).
- [ ] Host-рішення: бот у групі ingest-ить УСІ повідомлення, адресація керує лише відповіддю; Telegram privacy mode off (Запис 117, п.5).
- [ ] Детермінований мультиспікерний conformance-сценарій (direct-варіант із атрибутованими тезами від fake LLM); лінійні сценарії не чіпати (Запис 117, п.8).

**Гілка 2 — промпти пасів (окремо, бо змінює якість Archive; власний HISTORY-слід):**

- [ ] `memory_unit_pass` / `sleep_personal_signal_pass` / `sleep_consolidator`: тези атрибутуються іменами («Жека купив мотоцикл», не «користувач купив мотоцикл»); reply-ланцюжки розплутують переплетені теми; reply на стару тему формулюється як повернення до неї.
- [ ] `memory_fidelity_pass` питає про правильність авторства — validator стає guard-ом атрибуції.
- [ ] Live-тест на переплетених темах (мотоцикли Жеки + рибалка Антона + reply на старе повідомлення).

**Гілка 3 — Core subject (фаза 2):**

- [ ] `CoreFact.subject` як пара `{ id, name }` (імена змінюються, стабільний id; Запис 117, п.6); відсутній subject = «єдиний користувач скоупу».
- [ ] Субʼєктний guard у bridge: сигнал про X проходить тільки з подій авторства X; сигнали про X зі слів Y — лише через reflection + review.
- [ ] Near-duplicate gate стає subject-aware — інакше «у Жеки є мотоцикл» блокує «у Антона є мотоцикл» через token overlap (Запис 117, п.2).
- [ ] Рендер Core-фактів показує subject, коли він є.

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
