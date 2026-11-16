# Roadmap Memory Engine

## Related pages

- [Strategy](../foundation/strategy.md) - long-term direction that drives the
  roadmap.
- [Architecture](../foundation/architecture.md) - implemented system shape and
  boundaries.
- [Contracts](../foundation/contracts.md) - schema/API surface affected by
  roadmap items.
- [Audit 2026-06-10](audit-2026-06-10.md) - cleanup queue that unblocked v0.3.
- [Vector storage implementation spec](../research/vector-storage-tz-2026-07-03.md)
  - accepted vector storage plan referenced by this roadmap.
- [Multi-speaker geometry](../research/multi-speaker-geometry-2026-06-12.md) -
  attribution work referenced by later phases.
- [Reflective Belief System](reflective-belief-system.md) - phased transition
  from reliable autonomous reflection to an evidence-backed hypothesis graph,
  contradiction handling, and native clarification.
- [Server-scale Memory Runtime](server-scale-memory-runtime.md) - production
  track for authenticated vaults, transactional storage, durable workers,
  migration and launch acceptance.
- [Server Memory Runtime research](../research/server-memory-runtime-2026-08-01.md)
  - GitHub/PostgreSQL/Google evidence behind that production track.
- [Production Integration Feedback](production-feedback-2026-08.md) - accepted
  correction set from a live commercial group bot.
- [Integration Hardening Plan H0–H5](integration-hardening-plan-2026-11-15.md) -
  active work order since 2026-11-15; this roadmap carries only its milestones.

Цей документ — **жива дорожня карта** проєкту. Він не дублює стратегію (`wiki/pages/foundation/strategy.md`) чи архітектуру (`wiki/pages/foundation/architecture.md`). Він відповідає на одне питання: **куди ми йдемо і де зараз стоїмо**.

Документ оновлюється у двох випадках: коли пункт стає зробленим (відмітка `[x]` + дата + посилання на коміт або файл), і коли з'являється нова велика мета, яку треба зафіксувати раніше реалізації. Дрібні правки і робочий процес живуть у `DEVLOG.md`, не тут.

Версія документа: 2026-11-15.

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

## Поточний стан (2026-11-15)

Діє [план H0–H5](integration-hardening-plan-2026-11-15.md). H0
обʼєднав код бібліотеки з правками, які вже використовує Smartest2. Далі
H1 прибирає дубльовані запуски Sleep та стандартизує поставку адаптера.
H2–H3 закривають відомі проблеми запису, збереження й перегляду памʼяті.

Що змінилося відносно розділу нижче:

- Smartest2 використовує wheel `0.1.2` із `7a18569`. Обидва його коміти
  влиті через merge `bfbfd9f`; перевірений `3956ec1` доставлено в `origin/main`.
- Валідація на Smartest зупинена. Хост повідомив про вади приватності та
  походження у власному експортері, тому числа Phase 1 і `QUALIFIED` точки A
  чекають нового підтвердження; пакет точки B у репозиторії не зафіксовано.
  Gate v0.5 і вхід у v0.6 Phase 3 лишаються закритими до рішення D3 з плану.
- Клієнтський запит 2026-09-29 (C1–C4) і зведення хоста 2026-11-15 зведені в
  один порядок робіт. Серверний напрям лишається на паузі.
- Зовнішній звіт прийняв логічний еталон transactional Sleep на `c57e3cc`.
  Повний паритет S2.C, PostgreSQL і продакшен-приймання відкриті.

## Стан на 2026-08-24

Після v0.3: ліцензію повернуто на non-commercial; Chibigochi має async
product loop і реальну Godot-інтеграцію; prompt view має derived time labels.
Opt-in local vector storage, `recall_deep`, contextual expansion Phase 1 і
ordinary Recall Stage 2 уже реалізовані. Stage 2 ранжує archive-кандидатів за
max cosine конкретних `MemoryUnit`, зберігає winning unit provenance і повертає
unit-focused prompt projection замість змішаного денного summary.

Поточна operational-віха — production validation на Smartest v2 за
[окремим ТЗ](smartest-v2-production-validation-2026-08.md). Воно перевіряє
shared-prefix економіку, повний memory outcome, групову атрибуцію і живий
reflection funnel на великих чатах. Протокол розділено на qualification
логування, пасивне production-спостереження, контрольний replay та рішення.
Контрольна точка A пройшла `QUALIFIED` 2026-08-26; cohort `c` збирає 1-3
природні terminal Sleep runs на замороженій конфігурації. Довге Phase 2
починається після review цього package й остаточного `QUALIFIED` для Phase 1.
Chibigochi product work і Google Play server track поставлені власником на
паузу 2026-08-24.

Кодові й deterministic gates v0.4 зелені; unit-focused live acceptance також
проходив. Фінальний re-run calibrated associative threshold `0.47` та release
документ можна закрити на Smartest, якщо в його scope увімкнені vectors, або на
ізольованому replay. Default scoring weights до вимірювання не змінюються.

Окремо від v0.4 closure розпочато v0.5 autonomous reflection. Phase A–C і
integrity Phase C.1 реалізовані: scope ізольований, apply/review мають frozen
journal intent, evidence revalidation та restart replay без повторних side
effects. Живий gate і калібрування v0.5 тепер виконує Smartest v2. Його великі
групові чати мають показати active → valid → eligible funnel, природні
candidate/reinforcement/no-insight outcomes та відсутність повторних effects
після restart.

Наступна когнітивна віха після Smartest gate — v0.6 hypothesis graph із
рівнями узагальнення. Storage foundation Phase 2 уже реалізовано: schema,
journaled migration, scope-safe export і compatibility projection. Phase 3
reasoning відкривається після звіту Smartest, якщо в ньому є щонайменше одна
невироджена reflection action і немає незакритого attribution/data-integrity
дефекту. Chibigochi та Google Play для цього entry gate не потрібні.

Local/reference core тепер підтримує атрибутовану памʼять групового чату:
кожен self-statement перевіряється за авторами source events, Core і candidates
несуть `subject`, а неоднозначні або чужі твердження йдуть у review. Та сама
зміна відкрила embedding для підтверджено атрибутованих multi-speaker units.
Також FileStorage отримав два різні людські шляхи очищення: видалення журналу
окремої розмови зі збереженням довгоживучої памʼяті та повне незворотне
стирання scope з crash recovery. Server-side per-vault deletion лишається S6.

Продакшен-інтеграційний зріз 2026-08-24 закрив виміряні проблеми вартості й
спостережуваності Sleep: один compact source на run, стабільний provider-cache
prefix, selectable passes, whole-run call budget, контрольований early finish,
retry reports і пояснення Core bridge. Chat API отримав структурований prompt
plan з typed dialogue та host payload; нова порожня Session читається як
порожня, а fidelity/reflection status пояснює весь unit funnel. Повний
історичний профіль лишається default для сумісності.

Google Play додає незалежний release blocker: current `FileStorage` безпечно
координує тільки один writer process на runtime directory й не є production
backend-ом для тисяч клієнтів. Прийнято окремий server track S0-S7: той самий
Rust cognitive core працює в автентифікованому Memory Runtime, а один
server-derived vault per account зберігається транзакційно у
PostgreSQL/pgvector. Архітектурне S0 зафіксовано, provisional capacity envelope
має числа й відкриті owner decisions, а S1 reference contracts уже реалізують
fail-closed vault identity та ordered/idempotent turn ledger. S2.A ingest і
S2.B Core/committed-view recall пройшли зовнішнє logical-reference приймання;
S2.C має transactional Sleep lifecycle reference, згодом логічно прийнятий
на `c57e3cc`.
PostgreSQL, game API, workers і S7 ще не починались, а решта durable workflows
не перенесена. Локальні/Chibigochi
acceptance попередніх версій залишаються правдивими, але не є доказом
multi-user production scale.

## Стан на 2026-06-10

v0.2 закрито як повний цикл живої памʼяті: session → sleep → Archive/MemoryUnit → recall/context → fidelity → reflection candidates → review → Core/contested → forgetting → remember_back. Формальний acceptance-anchor — `crates/memory_engine/tests/living_memory_cycle.rs` і `wiki/pages/releases/v0.2-acceptance.md`; публічний реліз позначено тегом `v0.2.0`.

Після v0.2 короткий audit-cleanup перед v0.3 закрито: A1/A2/A3/A4 step 1/A4 step 3/B3/A5/A7/B4/B2 виконані. Journal/recovery scope (A3) закрито як рішення: journal лишається storage primitive для майбутніх migration/recovery-heavy операцій, runtime sleep спирається на durable `SleepRun` checkpoints. `engine.rs` більше не є монолітом: доменні частини винесені в `crates/memory_engine/src/engine/`.

Update v0.5: це історичне рішення для sleep не скасоване, але journal уже не
лише reserved primitive — Phase C.1 використовує його для reflection apply та
candidate review, де справді є recovery-heavy multi-file transition.

v0.3 host-conformance також закрито: direct/local, Telegram-local і Godot-headless проходять однаковий automated scenario; Godot acceptance підтверджено реальним Godot 4.6 stable console. Це доводить, що ядро використовується як reusable library із тонкими adapters, а не як Telegram-specific bot core. Наступне меню після v0.3: Chibigochi/Godot integration, Telegram-live smoke як транспортний regression, reviewer-pass, opt-in vector recall alignment, unit-level recall counters, diagnostics/backup.

---

## v0.1 — Foundation (тришарова памʼять як живий цикл)

Мета: довести, що архітектура Сесія → sleep → Архів → recall → Core працює end-to-end на реальному хості (Telegram бот) із multi-pass compression і чесним gating промоції в Core.

### Документація

- [x] Стратегія `wiki/pages/foundation/strategy.md` із принципами і MemPalace-спадкоємністю.
- [x] Архітектура `wiki/pages/foundation/architecture.md` v0.1 із чотирма рішеннями і трьома каналами Core.
- [x] Контракти `wiki/pages/foundation/contracts.md` із усіма schema-формами.
- [x] `HISTORY.md` як document of trust за MemPalace-template.
- [x] `wiki/pages/integration/local-development.md` із intsталяцією і командами.
- [x] `wiki/pages/releases/v0.1-acceptance.md` із критеріями завершення v0.1.
- [x] Цей `wiki/pages/planning/roadmap.md`. — 2026-05-19.

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
- [x] Ліцензія Memory Engine Non-Commercial Public License v0.3 + `LICENSE.md` у корені; Section 13 явно зберігає за Licensor право на інші умови і не поширює обмеження ліцензіатів на його власне законне використання.
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
- [ ] `recall_deep` (vector deep recall) не має еквівалента цього bonus-а на рівні memory unit: `recency` рахується тільки від `created_at`, `recall_count`/`last_recalled_at` не читаються взагалі. Потребує unit-level recall лічильників — досі відкладений пункт (див. "unit-level recall counters" у переліку меню після v0.3 вище). Знахідка з зовнішнього review 2026-07-06.
- [~] **Reflection auto-trigger за накопиченою вагою:** Phase A і Phase B
  реалізовано 2026-08-01. Rust core рахує тільки нові `ActiveArchive` +
  `fidelity_status=valid` units, розрізняє critical memory, accumulated salience
  і scheduled interval, тримає per-scope checkpoint, не дублює active task і
  не списує input при failure. Python/Godot мають thin API; Telegram
  after-sleep executor opt-in через `MEMORY_BOT_AUTO_REFLECTION=1`. До `[x]`
  бракує typed reinforcement замість duplicate candidates і живого Chibigochi
  acceptance. Cross-session personality evidence, bounded related history і
  existing-candidate context уже закриті Phase B. Повний план:
  [Autonomous Reflection v0.5](autonomous-reflection-v0.5.md).
- [ ] Link bonus у recall scoring: спогади, що мають звʼязок із поточною темою, Core candidate або свіжими подіями, отримують підсилення score.
- [x] Decay/freshness не має фізично видаляти спогади. Низька актуальність знижує rank і робить unit кандидатом на forgetting review. — 2026-05-30, effective freshness у Stage 1 recall з configurable half-life; повне `forget_review_pass` лишається окремим пунктом нижче.
- [x] Захист критичних спогадів: high-weight, emotionally validated або Core-linked units не мають потрапляти в forgetting без окремого reviewer warning. — 2026-06-01, `feature/forgetting` додає hard-gate: protected unit не забувається навіть якщо LLM рекомендує `forget`, protection re-check виконується на submit.
- [x] Status lifecycle для CoreFact: `active` → `contested` (накопичено суперечливі спостереження) → `deprecated` (підтверджено). Engine не видаляє факти зі статусом `deprecated` — зберігає як архівний слід. — 2026-06-01, `review_candidate(approved)` реалізує `active` → `contested`; `patch_core_fact` / `/core_forget` покриває manual demotion у `deprecated`.
- [ ] Engine method `engine.contest_core_fact(id, evidence)` — позначає факт як contested, не видаляє.
- [x] Recall враховує status: contested факти присутні в context але з позначкою, deprecated — не з'являються в context за замовчуванням. — 2026-06-01, `render_memory_view` маркує contested Core facts, `core_context_package` приховує deprecated.
- [x] `forget_review_pass`: окремий LLM-agent отримує старі low-signal `memory_units` з age/weight/archive-level recall proxy/fidelity/core-links і рекомендує, які спогади природно позначити як forgotten. Engine не видаляє full archive; v1 робить reversible status flip і rebuild `compact_memory`. — 2026-06-01, `feature/forgetting`.
- [x] Telegram-команди `/forgotten` і `/remember_back <id>` для audit і повернення забутого в активний recall. — 2026-06-01, thin host wrappers over core API.
- [x] Live-test forgetting v1: реальний `forget_review_pass` на scratch/runtime має забути рутину, не чіпати protected units, і `/remember_back` має повернути тезу в prompt-facing compact memory. — 2026-06-01, scratch live-check через cached Gemini: routine -> `Forgotten`; richer check: Core-linked unit protected at submit re-check, high-weight unit stayed active, `/remember_back` restored compact memory.
- [x] Formal v0.2 end-to-end acceptance fixture: `ingest -> sleep -> Archive/MemoryUnit -> recall/context -> fidelity -> reflection -> Core -> contested -> forgetting -> remember_back`. — 2026-06-01, `crates/memory_engine/tests/living_memory_cycle.rs`, `wiki/pages/releases/v0.2-acceptance.md`.

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

Векторне сховище не входить у v0.3 close. Робочий research-документ зафіксовано в `wiki/pages/research/vector-recall.md`, але перед кодом він має пройти alignment із поточною архітектурою: індексувати validated active `MemoryUnit`, а не великі archive JSON; core приймає vectors, host рахує embeddings; recall feedback має використовувати buffered stats path.

- [x] **Залежність від атрибуції розвʼязана посигнально:** у мультиспікерній сесії embed-яться лише units із `subject`, який ядро детерміновано підтвердило через source events; змішані або неатрибутовані units лишаються fail-closed. Однокористувацькі й speakerless scope поводяться як раніше. — 2026-08-16, `engine/vector_flow.rs`, `engine_multi_speaker.rs`.
- [x] Фаза A за ТЗ: storage `memory/archive/vectors/<scope>/` (manifest/vectors.f32/rows.jsonl/tombstones), `ComputeEmbedding`-батчі, backfill/enable/disable/purge/rebuild, R1-recovery, compaction, мультиспікерний gate. Реалізація: `feature/vector-storage-phase-a`, тести `engine_vectors.rs`.
- [x] Фаза B за ТЗ: `recall_deep` (min_sim-чесність, top_k, buffered reinforcement), локальний fastembed embedder у Telegram host, `/vectors*` і `/recall_deep` команди, conformance `--host direct-vectors`; live-калібрування на owner scope виконано для MiniLM (`min_sim=0.30`, `vivid_sim=0.55`); автоматичний distant-memory gate підключений у Telegram development host.
- [x] Contextual expansion Phase 1: `CoreContextRequest.query_embedding` дає ядру можливість додати 1-3 детальні memory-unit спогади з векторного індексу у `archive_relevant` під існуючим budget. Telegram development host передає embedding тільки для ready scope.
- [ ] **Калібрування `recall_deep` scoring (знахідка з зовнішнього review 2026-07-06):** `deep_recall_unit_weight` зараз недооцінює важливість спогаду відносно косинусної схожості (`0.10` проти фактичного `sim`-терму `~1.0`); плюс `recall_deep` не враховує `recall_count`/`last_recalled_at` (M4-принцип із `vector-recall.md` задекларований, але не підключений на рівні unit). Робити разом із live-калібруванням Contextual expansion Phase 2/Step 3 — той самий scoring-шлях. Деталі: [Generative Agents Memory Stream review](../research/generative-agents-memory-stream-2026-07-06.md).
- [x] Фаза C за ТЗ: Stage 2 re-rank через `RecallQuery.query_embedding` над top-K Stage 1; default window `40`, явна формула `stage1 * 0.5 + max_unit_cosine * 0.5`, `stage_used=stage2_embeddings`, structured candidate diagnostics. Live Chibigochi follow-up протягнув winning `MemoryUnit` до звичайного recall: `matched_memory_unit_id` пояснює provenance, а `RecallItem` проєктує конкретну тезу замість усього змішаного денного compact. Regression cases фіксують reorder і unit focus усередині window та `recall_deep` fallback поза window. — 2026-07-31.
- [ ] Хост має явний режим privacy/storage: `embeddings_enabled = false` за замовчуванням або як мінімум видима галочка "з vector storage / без vector storage".
- [x] Без embeddings Memory Engine лишається повністю працездатним через Stage 1 recall, compact memory і reflection по structured units; відсутній `query_embedding` не змінює Stage 1 output path.
- [x] PendingTask тип `ComputeEmbedding` працює для validated active memory units, а не для великих змішаних archive JSON; Telegram host виконує backfill і `SleepOutcome.embedding_requests` локальним fastembed.
- [-] Legacy archive-level поля `embedding_model_id` і `embedding` не заповнюємо: прийнята реалізація зберігає derived rows окремо під `memory/archive/vectors/<scope>/` і embed-ить `MemoryUnit.thesis`. Це прибирає змішаний archive text із vector truth і дозволяє purge/rebuild без міграції ArchiveEntry. — 2026-07-03, vector-storage ТЗ Phase A.
- [x] Recall Stage 2 — embedding re-ranking над топ-K кандидатами зі Stage 1. Активується готовим opt-in vector scope + `query_embedding`; disabled/building/corrupt scope прозоро лишається на Stage 1. — 2026-07-31.
- [ ] Recall Stage 3 — LLM rerank через `PendingTask::RecallRerank` із промптом `prompts/recall_rerank.md`. Активується через `Manifest.features.llm_recall_rerank_enabled`.
- [x] Migration plan: коли embeddings вмикаються на існуючому архіві без embeddings, engine створює batch `ComputeEmbedding` tasks тільки для validated active units, не для forgotten/debug material; нові units повертають incremental embedding requests після sleep.

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

- [x] Зафіксувати v0.3 acceptance як host-conformance, а не ручне клікання власником. Перший документ: `wiki/pages/releases/v0.3-acceptance.md`. — 2026-06-10.
- [x] Direct/local conformance driver через Python adapter із deterministic fake LLM: `tests/host_conformance/host_conformance.py --host direct`. Це baseline для всіх наступних хостів. — 2026-06-10.
- [x] Telegram-local driver має пройти той самий сценарій без Telegram transport. — 2026-06-10, `tests/host_conformance/host_conformance.py --host telegram-local`.
- [?] Telegram-live smoke driver має перевіряти тільки транспорт і один короткий live-LLM шлях, не повний memory acceptance руками. Відкладено з v0.3 close: `telegram-local` вже покриває реальний `bot.py` memory path без Telegram API, тому live smoke є regression/operations task, не acceptance gate.
- [x] Godot-headless driver має пройти той самий conformance scenario до будь-якого polished Godot UI. — 2026-06-10: `tests/host_conformance/host_conformance.py --host godot-headless` passed with real Godot 4.6 stable console, `memory_units=3`, `core_facts=3`.
- [x] v0.3 close: кожен прийнятий host проходить однаковий scenario і доводить, що memory policy лишається в Rust core. Accepted scope: direct/local, Telegram-local, Godot-headless. — 2026-06-10, `wiki/pages/releases/v0.3-acceptance.md`, `tests/host_conformance/host_conformance.py`.

### Production integration cost and observability

- [x] Provider-facing Sleep source передається як один compact shared prefix;
  batch constants не дублюються у кожній події та кожному pass request.
  Durable input лишається lossless. — 2026-08-24,
  `crates/memory_engine/tests/production_feedback.rs`.
- [x] Публічний run options API обирає tracks, whole-run provider-call budget і
  retry ceiling; budget stop зберігає отримані результати й завершує Archive.
  — 2026-08-24.
- [x] `memory_prompt_plan.v1` віддає окремі core/long/associative sections та
  typed dialogue events із payload для role-aware chat APIs. — 2026-08-24.
- [x] Sleep outcome пояснює skipped/recovered/failed work і Archive-to-Core
  рішення; reflection status розкладає active/valid/unchecked/excluded units.
  — 2026-08-24.
- [~] Заміряти реальну provider cache hit-rate і ціну на навантаженні Smartest
  v2 після переходу інтеграції на shared-prefix contract. Перший cohort
  підтвердив однаковий prefix і `4.97x` amplification на семи чистих runs.
  Передперевірка cohort `c` підтвердила правильне Gemini usage field і нуль
  implicit cache на трьох запитах зі спільним 8k-token prefix. Phase 2 оцінює
  фактичну ціну compact input без припущення про cache. Це product benchmark,
  а не library claim. — 2026-08-26,
  [Checkpoint A review](smartest-v2-checkpoint-a-review-2026-08-26.md).
- [x] Переписати
  [Smartest v2 Production Validation](smartest-v2-production-validation-2026-08.md)
  як дослідницький протокол: контекст, сім гіпотез, operator latitude,
  privacy boundary, qualification логування, пасивне вікно, replay,
  interpretation map і decision gates. — 2026-08-24.
- [~] Smartest Phase 1: перший redacted package отримав `CHANGES_REQUIRED`, а
  виправлена контрольна точка A пройшла `QUALIFIED` 2026-08-26. Provenance,
  physical attempts, origin joins, chat budget/Core diagnostics, process
  health, privacy export і vector execution підтверджені. Cohort `c` збирає
  1-3 природні terminal Sleep runs; остаточний Phase 1 status чекає package
  контрольної точки B. —
  [Checkpoint A review](smartest-v2-checkpoint-a-review-2026-08-26.md).
- [ ] Після `QUALIFIED` виконати Smartest Phase 2: мінімум 20 terminal Sleep
  runs у великих групових чатах, provider ledger, unit/Core/reply quality
  review, reflection funnel і runtime health evidence.
- [ ] Phase 3: на ізольованому replay порівняти повний і economy Sleep profiles,
  звести H1–H7 та сформувати production recommendation.
- [ ] За результатом Smartest закрити live gate v0.5 або зафіксувати точну
  причину `INSUFFICIENT_DATA`/`FAIL`; лише PASS відкриває v0.6 Phase 3.

### Адаптери

- [x] Godot-адаптер через GDExtension (`crates/godot_adapter/`). Перший хост, що **не** Telegram-бот. — 2026-06-10, thin JSON-boundary GDExtension crate compiles with `cargo check -p godot_adapter`; live Godot runtime acceptance tracked in Host conformance above.
- [?] Chibigochi-інтеграція: memory host, Godot UI, HTTP LLM bridge, real
  Gemini executor і async product loop реалізовані та лишаються regression
  fixtures. Polished gameplay/character UX і production packaging поставлені
  власником на паузу 2026-08-24.
- [~] Третій проєкт — Smartest v2, комерційний груповий Telegram-бот. Реальний
  PyO3/LLM шлях і великі чати вже дають production evidence; перший redacted
  package виявив корисні дані й instrumentation gaps. Передперевірка
  виправленого вимірювання пройшла; formal acceptance чекає короткого cohort-C
  package за
  [ТЗ Smartest](smartest-v2-production-validation-2026-08.md) та
  [review 2026-08-26](smartest-v2-checkpoint-a-review-2026-08-26.md).
- [ ] MCP-facade як alternative обгортка над тим самим ядром. Для зовнішніх агентів (Claude Code, OpenAI Agents SDK), що хочуть користуватись memory як tool.
- [ ] Стабілізувати JSON contracts: зафіксувати v1 для всіх schemas, прописати migration policy для v2+.

### Час у памʼяті (prompt-facing time labels)

Джерело рішення: `wiki/pages/research/memory-time-perception-2026-07-02.md`. Принцип: відносні мітки ніколи не зберігаються — обчислюються при рендері з абсолютних timestamp-ів; тому «оновлення після простою» не існує як задача.

- [x] `current_time:` рядок у `<state>` prompt view — точка відліку для моделі. — 2026-07-02, `prompt_view.rs`.
- [x] Мітка віку на archive items у `<long_memory>` (`- [yesterday | 0.88] ...`) з `RecallItem.time_range.end`; драбина: today / yesterday / N days ago / earlier this month / last month / N months ago / over a year ago (календарні дні). — 2026-07-02.
- [x] Денні маркери (`[yesterday]`) у older trace `<short_memory>`; свіжий recent tail без міток. — 2026-07-02.
- [x] `CoreContextRequest.utc_offset_minutes` + `clock_untrusted` (serde default, без міграцій); Telegram host передає локальний offset. — 2026-07-02, HISTORY-запис.
- [x] Деградація: недовірений годинник / майбутні timestamps → мітки опущені + package note. — 2026-07-02, `memory_view_omits_labels_when_clock_is_untrusted`.
- [ ] Natural-language time intent: core-owned structured planning boundary має
  перетворювати «минулого тижня»/«вчора»/«у травні» на
  `RecallFilters.time_range` відносно `created_at` і локального UTC offset.
  Конкретний host може виконати потрібну LLM-задачу, але не має винаходити
  власну recall/time policy.
- [ ] Live-перевірка на Telegram host: «коли я тобі казав про X?» після кількох днів історії.
- [ ] Хостова опортуністична корекція часу за `Date`-заголовком наявного HTTP-трафіку (без окремих time-запитів) — коли зʼявиться перший хост із недовіреним годинником.

### Мультиспікерна геометрія (кандидат на «третій проєкт»)

Джерело рішення: `wiki/pages/research/multi-speaker-geometry-2026-06-12.md` + доповнення 1–8 у DEVLOG Запис 117. Принцип: геометрія = адресація (роутинг хоста, в ядро не потрапляє) / атрибуція (єдине, що переживає стискання — як імена у змісті) / посилання (працюють при sleep-дистиляції і вмирають). Лінійні хости не змінюються взагалі; жодних «групових режимів» у ядрі.

**Гілка 1 — контракти + рендер + recall за подією:**

- [x] Опціональний `speaker: { id, name }` в `IngestEvent`/`StoredEvent`/`CoreContextEvent` (serde default; відсутність = поточна бінарна поведінка). `reply_to` kind-конвенція задокументована в `contracts.md`. — 2026-07-03, HISTORY-запис.
- [x] `render_memory_view`: події зі speaker рендеряться імʼям (`Жека: ...`) замість `user:`; без speaker — байтово поточна поведінка; dedup current message працює для будь-якої не-assistant ролі. — 2026-07-03, `memory_view_attributes_speakers_and_drops_current_speaker_duplicate`.
- [~] Сирий матеріал sleep-пасів несе структурований `speaker`, а промпти вимагають конкретної атрибуції. Компактні reply-маркери (`[Жека, у відповідь на №12]`) ще відкриті.
- [x] Evidence pack рендерить `speaker`, а `memory_fidelity_pass` перевіряє правильність авторства. — 2026-08-16, `fidelity.rs`, `sleep_driver.rs`, `prompts/memory_fidelity_pass.md`.
- [ ] `recall_by_event_id(session_id, event_id)`: покрита подія → архівний запис через metadata cache; жива → ознака «в активному хвості». Перед цим закрити storage-гальмо: `archive_entry_path_by_id` сканує весь каталог — потрібен path-hint або map `event_id -> archive_id` у `SessionMetadata` (перебудовний кеш; Запис 117, п.4).
- [-] Фаза 1 захисту Core вимикала auto-bridge для всієї сесії з ≥2 speaker.id. Її замінив точніший посигнальний guard: безпечні self-statements тепер проходять, чужі або неоднозначні твердження йдуть у review. — замінено 2026-08-16.
- [ ] Host-рішення: бот у групі ingest-ить УСІ повідомлення, адресація керує лише відповіддю; Telegram privacy mode off (Запис 117, п.5).
- [x] Детермінований мультиспікерний conformance-сценарій: `tests/host_conformance/host_conformance.py --host direct-multispeaker` (speaker через PyO3, атрибутований transcript, атрибутовані тези, gossip-сигнал 0.95 не потрапляє в Core); лінійні сценарії без змін. — 2026-07-03.

**Гілка 2 — промпти пасів (окремо, бо змінює якість Archive; власний HISTORY-слід):**

- [x] `memory_unit_pass` / `sleep_personal_signal_pass` / `sleep_consolidator` вимагають конкретних імен і повертають `subject_hint` / `speaker_id`; ядро трактує їх як підказку, а не істину. — 2026-08-16, prompt version 2.
- [x] `memory_fidelity_pass` перевіряє правильність авторства — validator став додатковим guard-ом атрибуції. — 2026-08-16, prompt version 2.
- [ ] Reply-ланцюжки розплутують переплетені теми, а reply на стару тему формулюється як повернення до неї.
- [ ] Live-тест на переплетених темах (мотоцикли Жеки + рибалка Антона + reply на старе повідомлення).

**Гілка 3 — Core subject (фаза 2):**

- [x] `CoreFact.subject` як пара `{ id, name }`; additive `subject` також проходить через PersonalSignal, MemoryUnit, CandidateBelief, reflection і hypothesis records. Відсутній subject зберігає семантику єдиного користувача. — 2026-08-16.
- [x] Субʼєктний guard у bridge: self-statement проходить тільки з подій того самого автора; about-other, кілька авторів або неправдива model hint створюють reviewable candidate. — 2026-08-16, `engine_multi_speaker.rs`.
- [x] Near-duplicate gate і Core upsert стали subject-aware. — 2026-08-16.
- [x] Core context і prompt view віддають subject для кожного атрибутованого факту; query-aware budget ranking враховує імʼя/id людини. — 2026-08-16.

### Diagnostic tools

Стратегія прямо передбачає "діагностичні інструменти для розробника". Без них пам'ять, що живе роками, стає чорною скринькою.

- [x] Structured recall provenance foundation: `explain_recall` показує Stage 1 складові, обидва ранги, vector status, cosine і winning `matched_memory_unit_id`; host може логувати ланцюг `query hash -> MemoryUnit -> archive -> final rank` без текстів. Повний CLI debugger лишається окремим пунктом нижче. — 2026-07-31.
- [ ] Memory inspector CLI (`crates/memory_inspector/`): окремий binary, що показує статистику архіву — кількість entries, distribution ваг, найчастіше recall'ені спогади, давно не recall'ені, candidate beliefs у черзі.
- [ ] Recall debugger: запит recall через CLI з різними фільтрами, видно повний scoring breakdown для топ-10 результатів.
- [ ] Core fact viewer: побачити всі Core facts по категорії з історією status changes (active → contested → deprecated).
- [ ] PendingTask viewer: побачити, які LLM-задачі очікують виконання, що в attempts, що в last_error.

### Backup і відновлення

Стратегія: "потрібні надійне збереження, резервне відновлення, manifest із версіями схем, перевірка цілісності і міграції". Backup-flow не може бути "копіюємо папку".

- [ ] Документований backup flow: snapshot цілої `memory/` директорії з checksum для manifest і кожного persistence layer.
- [ ] Документований restore flow: перевірка integrity, journal replay для незавершених операцій, migration на актуальну схему якщо backup старший.
- [ ] CLI команда `memory_inspector backup` і `memory_inspector restore`.

### Людське видалення у local/reference режимі

- [x] `delete_session(session_id)` ідемпотентно видаляє raw-журнал розмови,
  archived segments, session metadata та повʼязані tasks/runs, зберігаючи вже
  створену довгоживу памʼять scope. — 2026-08-16,
  `engine/erasure_flow.rs`.
- [x] `purge_scope(personality_scope_id)` ідемпотентно й незворотно видаляє всі
  FileStorage-дані scope, включно з Core, Archive/units, candidates/hypothesis,
  tasks, reflection state та derived vectors; сусідній scope зберігається. —
  2026-08-16, `engine/erasure_flow.rs`.
- [x] Обидва шляхи мають frozen journal plan, restart replay і структурований
  звіт; Python та Godot adapters відкривають той самий API. — 2026-08-16,
  `erasure.rs`, `tests/engine_erasure.rs`.
- [ ] Видалення логів застосунку, зовнішнього провайдера й резервних копій
  інтегрує конкретний продукт навколо core report.

---

## Integration hardening track H0-H5 — продакшен-хост на FileStorage

**Статус 2026-11-15:** активна черга. Роботи, порядок і критерії приймання
живуть у [плані дій H0–H5](integration-hardening-plan-2026-11-15.md); тут лише
видимі віхи.

Мета: рушій, якому продакшен-хост довіряє збереження й розвиток памʼяті без
власного форку, обхідних шляхів і читання каталогів сховища. Когнітивна дорога
v0.5 → v0.7 цим треком не змінюється.

- [x] План зафіксовано; запити хоста й клієнтський запит 2026-09-29 звірено з
  кодом. — 2026-11-15, DEVLOG Запис 176.
- [x] H0 — `c9a7742` і `7a18569` влиті через merge `bfbfd9f`; документацію
  та перевірки завершено, `3956ec1` доставлено в `origin/main`. — 2026-11-15,
  DEVLOG Запис 177.
- [~] H1 — H1.1–H1.7 реалізовано в `codex/h1-release-wheels`, включно зі
  збіркою і перевіркою Python wheels та інструкцією інтегратора. Приймання
  першого тегованого релізу очікує D4 і прогону CI. — 2026-11-15,
  DEVLOG Запис 182.
- [x] H2 — довірене застосування рішень: ідемпотентний запис, спільна межа
  запису і стирання, fidelity й forgetting без впливу запізнілих і повторених
  відповідей, повна публікація результату extraction, життєвий цикл службових
  записів (C1, C2, DR-09 хоста), міграція та спільний результат публікації
  extraction у FileStorage і Storage v2. Виконано 9 із 9 кроків.
  Пройшли 7 із 7 сценаріїв приймання та 11 із 11 локальних сценаріїв хостів.
  — 2026-11-16, DEVLOG Запис 190.
- [~] H3 — канонічне читання без повернення застарілих копій; перегляд і
  керування в межах scope; перегляд і стирання памʼяті про одну людину (C3, C4).
  Виконано 3 із 7 підпунктів, H3.0–H3.2. Пройшли рядки 9–11 матриці
  на рівні рушія. Далі перегляд і стирання за людиною.
  — 2026-11-16, DEVLOG Запис 192.
- [ ] H4 — калібрування підсилення показом; дати в памʼяті після рішення
  власника.
- [ ] H5 — живе приймання на реальній моделі й повернення до gate v0.5 та v0.6
  Phase 3.
- [ ] Рішення власника D1–D4: абсолютні дати; розгляд кандидатів у Core і
  повноваження моделі-рецензента; що відкриває ізольований replay, поки
  продакшен-приймання лишається окремим; нумерація релізів.

Трек завершує надійність та інтеграцію нинішньої памʼяті, а не задум проєкту:
ієрархія висновків, переосмислення суперечностей і оцінювання перед v1.0
лишаються у своїх розділах нижче.

---

## Production track S0-S7 — Google Play server scale

**Статус 2026-08-24:** поставлено власником на паузу. Реалізовані S1/S2
reference slices зберігаються й проходять regression gates; PostgreSQL,
runtime, workers і launch work не є активною чергою до окремого рішення
власника.

Мета: не створити другу памʼять для гри, а безпечно запустити той самий Rust
core як централізований сервіс для тисяч гравців. Нормативний порядок,
інваріанти й acceptance gates живуть у
[Server-scale Memory Runtime](server-scale-memory-runtime.md).

### S0 — Архітектура і capacity envelope

- [x] Зафіксувати різницю між local/reference `FileStorage` і production
  server deployment; один vault має один authoritative backend, а portable
  export/import зберігає людський контроль. — 2026-08-01,
  [plan](server-scale-memory-runtime.md) і
  [research](../research/server-memory-runtime-2026-08-01.md).
- [x] Прийняти PostgreSQL + pgvector без окремої vector/graph database на
  старті; exact per-vault vector search перед ANN. — 2026-08-01.
- [~] Зафіксовано planning envelope v1: 100k stored accounts, 1k peak active,
  40 sustained / 150 burst turns/s, one-EEA-region launch, service latency та
  RPO/RTO gates. Це engineering baseline, не capacity claim; retention,
  region/RPO/RTO ще потребують owner approval, а числа — заміни measured
  closed-test forecast перед infrastructure/launch decision. — 2026-08-01,
  [plan](server-scale-memory-runtime.md#planning-envelope-v1-2026-08-01).

### S1 — Vault identity та idempotent turn contract

- [x] Введено validated internal `AccountId`/`VaultId`, authenticated principal,
  active account-to-vault membership та internal `VaultContext`; відсутній або
  forged identity fail closed, shared fallback відсутній. Godot wire contract
  для цього контексту ще не існує. — 2026-08-01, `server_runtime.rs`.
- [~] Guest/Play Games identity model і manual-review conflict decision
  забороняють автоматичний merge акаунтів. Реальний Google credential exchange
  і durable identity registry належать S4.
- [x] Додано `client_request_id`, vault-scoped deterministic operation ID,
  monotonic session turn sequence, terminal result reference/digest і
  restart-safe local reference ledger для duplicate/concurrent requests. —
  2026-08-01.
- [ ] Mobile retry тримає той самий request ID до terminal acknowledgement;
  timeout/reconnect не створює другий memory event.
- [x] Negative tests закривають missing/forged identity, cross-vault context,
  conflicting replay, wrong-order completion, expected-sequence conflict,
  restart і відсутність shared fallback. — 2026-08-01,
  `server_runtime_s1.rs`.
- [~] Реалізовані S1 dedup key, operation ID і local path включають vault;
  повний аудит майбутніх cache/DB/worker keys повторюється у S2-S5.

### S2 — Transaction-capable Storage v2

- [x] Наскрізний information-flow repair вирівняв local/reference path:
  host-owned `memory_text`, stable personality scope поверх interaction
  Sessions, повний frozen Sleep input, fail-closed source provenance,
  standalone MemoryUnit як canonical prompt truth, один eligibility gate,
  present-state render і recall feedback лише після final context budget/dedup.
  Див. [Information Flow Integrity](information-flow-integrity.md). —
  2026-08-02.
- [x] Проаудитити всі Storage flows і відділити read-only, atomic mutation та
  durable workflow; не ховати multi-record races за набором SQL methods. —
  2026-08-01, [work-unit inventory](storage-v2-work-unit-inventory.md).
- [~] Додано additive transaction/work-unit boundary, point reads, optimistic
  record revisions, idempotent receipt, bounded conflict retry та in-memory
  reference. Перший domain flow (`ingest_transactional`) атомарно комітить
  event + Session metadata. S2.B Core upsert/patch також використовує exact
  category revision, durable operation result і shared Core policy, а recall
  комітить ranking result і correctness feedback в одному view. S2.A/S2.B
  externally accepted; чинний Storage/FileStorage API не зламано. S2.C
  transactional Sleep lifecycle має зовнішнє логічне приймання `c57e3cc`;
  post-finalize cognition і compatibility adapter ще відкриті.
- [x] Correctness-critical recall state класифіковано й перенесено в S2.B
  reference: `recall_transactional` читає один Archive snapshot, ранжує shared
  Stage 1 policy та одним work unit зберігає selected feedback і exact result.
  Replica-local deltas не беруть участі. Collection revision/membership
  захищені від phantom changes. Production indexed query лишається S3, vectors
  — S2.D. — 2026-08-01, `server_recall_storage_v2.rs`.
- [x] Storage v2 snapshot reads отримали domain-shaped `SessionEvents` та
  `Archives` queries з exact membership/revision phantom protection. Sleep
  більше не сканує й не валідує чужі Session events; recall query вже має
  форму, яку S3 може відобразити на bounded composite indexes. Full-collection
  scan лишився тільки compatibility/conformance API. — 2026-08-02.
- [x] S2.B Core logical implementation: versioned transactional upsert/patch,
  deterministic new fact IDs, durable replay result, exact revision retry,
  point patch із explicit category, scope/vault isolation та fault suite. Це
  library reference; зовнішнє приймання `2bdc0ae` + `7314833` підтвердило Core
  і recall suites, shared policy, concurrency/replay та legacy regression.
  Host adapter ще відкритий. — 2026-08-01, `server_core_storage_v2.rs`,
  `server_recall_storage_v2.rs`.
- [x] S2.A ingest logical-reference gate пройшов deterministic
  fault/concurrency suite: before apply,
  після кожної з двох mutations, receipt-before-publish,
  publish-before-ack із automatic reconciliation через receipt,
  replay/conflicting replay, same-session contention, multi-record rollback і
  cross-vault denial. Follow-up `af932a4` пройшов зовнішню source-level
  перевірку всіх assertions. Це logical in-memory proof, не host-adapter або
  physical crash test. — 2026-08-01.
- [x] S2.C transactional Sleep lifecycle реалізовано як один durable workflow:
  server
  receive-order визначає active-tail boundary і equal-weight tie-break, а
  shared Stage 1 policy вибирає source events, після чого
  Session claim, preliminary Archive, два PendingTasks, initial SleepRun та
  immutable input та operation result публікуються сімома атомарними mutations.
  Preliminary не означає Complete coverage; pending event IDs живуть окремо.
  Дванадцять focused tests додано, включно з fail-closed incomplete coverage,
  fail-closed rewritten-event rejection і доказом, що пошкоджена подія іншої
  Session не блокує target Sleep. Submit/finalize/cancel freeze current run,
  revalidate input/Archive/task topology, мають deterministic IDs, exact replay
  і повні logical failpoints. Лише finalize атомарно створює canonical
  MemoryUnits та Complete coverage; cancel не приховує events. Логічний
  lifecycle зовнішньо прийнято на `c57e3cc`. — 2026-08-02,
  `server_sleep_stage1_storage_v2.rs`.
- [x] Sleep payload нормалізовано у один `sleep_workflow_input.v1`; tasks/run/
  replay results тримають typed digest reference, а provider batch
  rehydrate-иться на worker boundary. Limits: 2 MiB input, 2 MiB submit batch,
  256 MemoryUnits. Transactional ingest присвоює monotonic per-Session
  `admission_sequence`, який Sleep використовує перед legacy fallback. —
  2026-08-02.
- [ ] Продовжити S2.C: перенести post-finalize Core/derived routing, fidelity і
  forgetting через той самий work-unit gate. Provider calls лишаються поза
  transactions.
- [ ] S2.D reflection/review, hypothesis graph migration, vector state і решта
  task transitions мають пройти той самий work-unit gate.

### S3 — PostgreSQLVaultStorage + pgvector

- [ ] Реалізувати pooled vault-keyed schema, migrations, composite foreign
  keys, non-owner runtime role і default-deny RLS.
- [ ] RLS vault context задається transaction-local на кожному pooled
  connection use; stale session state й absent context не можуть відкрити
  попередній vault.
- [ ] Серіалізувати короткі same-vault mutations, дозволивши різним vault
  працювати паралельно; bounded retry для transaction conflicts.
- [ ] Реалізувати indexed recall/task/graph reads і exact pgvector search за
  `(vault_id, model_id)`; HNSW лише після query-plan/latency benchmark.
- [ ] Multi-process tests: atomicity, crash recovery, cross-vault denial,
  vector/non-vector parity, pool budget і no global archive scans.

### S4 — Authenticated Memory Runtime

- [ ] Побудувати Memory Runtime як internal component game backend: public
  edge лишається game API; окремий private service дозволений пізніше без
  зміни contract, якщо його виправдають scale/failure metrics.
- [ ] Server-side validation/exchange Play Games auth code; Play Integrity як
  abuse/tamper signal, не як єдине account identity.
- [ ] Перевести Godot product path на Runtime без DB credentials і memory
  policy у клієнті.
- [ ] E2E: account create/link/reconnect, duplicate/concurrent turn,
  unauthorized vault, export/delete і restart.

### S5 — Durable background workers

- [ ] PostgreSQL jobs/outbox із lease, attempt, backoff, idempotent completion
  та poison/manual state; worker claim через `SKIP LOCKED`.
- [ ] Локальні embeddings, sleep і reflection виконуються поза DB transaction
  та submit-ять результат через version revalidation.
- [ ] Embedder load/warmup один раз на worker, batch processing, readiness за
  model/dim і versioned backfill без cold-start на першому user request.
- [ ] Окремі interactive/maintenance concurrency budgets, per-vault/global
  quotas і backpressure.
- [ ] Kill/restart acceptance: no lost/duplicate effects, bounded queue age,
  structural recall доступний при деградації workers.

### S6 — Portability, migration, backup і deletion

- [ ] Versioned deterministic vault export/import із checksums, provenance та
  round-trip через FileStorage.
- [ ] One-authority migration: snapshot -> validate -> import -> atomic routing
  cutover -> verify/rollback; жодного permanent dual-write.
- [ ] Point-in-time recovery і restore drills із визначеним RPO/RTO; schema
  rollout/rollback procedure.
- [ ] Повне per-vault delete/retention покриває records, vectors, tasks,
  outbox/exports; orphaned shared `local_user` не merge-иться наосліп.
- [ ] Google Play privacy gate: точні Data safety/privacy disclosures,
  discoverable in-app deletion і external web deletion resource; account
  deletion видаляє associated memory data, а retention явно пояснений.

### S7 — Whole-product production acceptance

- [ ] Відтворити повний workload: ingest -> embed -> context/recall -> reply
  ingest плюс реальна частка sleep/reflection, duplicates, hot vaults і
  provider stalls.
- [ ] Прогнати на 2× очікуваного launch peak; зняти p50/p95/p99, pool wait,
  vault-lock wait, queue age/depth, retries, vector latency і database growth.
- [ ] Security gate: zero cross-vault read/write, no memory text/embeddings у
  routine logs, verified deletion/export and abuse limits.
- [ ] Staged rollout internal -> opt-in cohort -> percentages -> GA з
  automatic rollback gates і перевіреним restore.

---

## v0.4 — Explainable semantic recall

Мета: випустити semantic recall як одну пояснювану можливість ядра, а не набір
експериментальних host-хаків, і вперше синхронізувати package versions, tag та
acceptance за єдиною SemVer-політикою.

- [x] Opt-in vector storage і локальний host-computed embedding boundary над
  validated active `MemoryUnit`; non-vector Stage 1 лишається повноцінним.
- [x] Незалежний `recall_deep` повертає кандидатів поза Stage 1 window, а
  ordinary Stage 2 re-rank пересортовує близькі top-K кандидати явною формулою.
- [x] Structured provenance показує Stage 1 components, vector status, cosine,
  обидва ранги, final score і winning `matched_memory_unit_id` без текстів у
  telemetry.
- [x] Unit-focused projection: ordinary Stage 2 повертає конкретну тезу winning
  MemoryUnit, а archive лишається ID/часовою/statistics boundary. — 2026-07-31,
  `d5d5f40`.
- [x] Живий Chibigochi acceptance `d5d5f40`: broad query «інші світи» повернув
  конкретний планетарний unit без sibling leakage; Титан, непрямий парафраз,
  Європа і negative control пройшли; Stage 1 fallback збережений. — 2026-08-01.
- [~] Розвести chat context і explicit semantic search: `archive_relevant` у
  `core_context_package` лишається structural recall, а current-turn vector
  details повертаються окремим ephemeral `associative_memory` і рендеряться як
  давні спогади, активовані поточною темою. Перевірити topic switch
  Марс → динозаври та outside-window association без stale carry-over. —
  `3c5aa74` і живий Chibigochi прогін підтвердили розділення, дедуплікацію,
  rebuild на кожен turn, Stage 1/non-vector fallback і product smoke. Єдиний
  провал — unrelated «динозаври» активували планетарний unit з `sim=0.455` при
  старому порозі `0.45`; default відкалібровано до `0.47`, фінальний live re-run
  ще потрібен перед `[x]`.
- [x] Automated release consistency gate звіряє root Cargo version, inheritance
  усіх Rust crates і Python package; на `v*` tag додатково вимагає exact
  three-number acceptance зі статусом accepted та HISTORY heading. —
  2026-08-01, `tools/check_release_versions.py`, CI workflow.
- [ ] Після живого acceptance створити `v0.4.0-acceptance.md`, синхронно змінити
  Cargo/Python manifests на `0.4.0`, прогнати release gates і лише тоді створити
  immutable annotated tag `v0.4.0`.

Не входить у v0.4 closure: natural-language time intent, unit-level recall
counters, зміна deep-recall weights, persisted active-topic state,
Core-anchored contextual expansion Phase 2 і Recall Stage 3. Це наступні окремо
калібровані зміни, а не умови чесного per-request associative layer.

---

## v0.5 — Autonomous reflection

Мета: памʼять сама визначає, коли накопичений перевірений досвід варто
осмислити, але не отримує права самостійно переписувати Core.

- [x] Phase A: explicit `ReflectionConfig`, structured trigger status,
  `begin_reflection_if_due`, critical-mass/time gates і bounded automatic input.
- [x] Durable per-scope reflection state та per-unit checkpoint markers;
  zero-candidate success споживає input, failed task — ні.
- [x] Per-scope active-task deduplication і відновлення через звичайний
  PendingTask lifecycle.
- [x] Thin Python/Godot adapter methods; Telegram after-sleep execution є
  явним opt-in, а не прихованою новою витратою.
- [x] Phase B: reflection evidence boundary по explicit `core_scope` через
  кілька source sessions, bounded related historical units, explainable
  non-vector structural selector, explicit no-LLM session binding/backfill та
  cross-session provenance. — 2026-08-01.
- [x] Phase C: prompt v3 і `reflection_actions_result.v1` функціонально
  розрізняють новий
  candidate, reinforcement наявного, reviewable Core challenge та structured
  no-insight. Reinforcement додає нові evidence/provenance до того самого
  candidate ID; Core challenge не змінює Core до owner review; legacy
  `reflection_result.v1` лишається сумісним для старих pending tasks. —
  2026-08-01. Functional path plus C.1 integrity gate are deterministic-green.
- [x] Phase C.1: зробити `core_scope` immutable identity boundary; missing scope
  означає лише isolated unscoped namespace; review відхиляє terminal candidate
  і scope override. — 2026-08-01.
- [x] Journal-backed idempotent reflection apply та owner review із frozen
  intent, deterministic operation/action IDs, evidence revalidation, restart
  recovery і text-free durable outcome. — 2026-08-01.
- [x] Fault-boundary replay після кожного логічного side effect, repeated
  reinforcement/review,
  cross-scope fixtures і stale-evidence scenarios. Лише цей gate закриває
  Phase C як deterministic release contract. — 2026-08-01.
- [ ] Live-calibration початкових
  `2.5 / 0.95 / 7 days / 2 units / 40 new / 20 related / related score 2 / 20 candidates`
  defaults на кількох production Sleep cycles Smartest v2 за
  [redacted telemetry spec](smartest-v2-production-validation-2026-08.md).
- [ ] Створити `v0.5.0-acceptance.md`, синхронно підняти manifests за SemVer і
  тегувати тільки після deterministic + live PASS.

Нормативний дизайн: [Autonomous Reflection v0.5](autonomous-reflection-v0.5.md).

---

## v0.6 — Hierarchical hypothesis graph

Мета: reflection формує не плоский список схожих текстів, а пояснювану мережу
вузьких і широких гіпотез із прямими evidence links. Це внутрішній простір між
Archive і Core, не четвертий prompt-visible memory layer.

**Entry gate Phase 3:** current production-feedback branch у `main`; Smartest
shared-prefix/quality report отримано; v0.5 live gate має PASS; є хоча б одна
невироджена reflection action із кількома source units або sessions; restart не
створює duplicate effect; attribution/data-integrity blockers відсутні.
Chibigochi packaging і Google Play server track не входять у цю залежність.

- [x] Версіоновані hypothesis node, evidence link і relation contracts;
  journaled restart-safe migration `candidate_belief.v1` без вигаданих
  semantic relations. — 2026-08-01,
  `crates/memory_engine/src/hypothesis.rs`,
  `engine/hypothesis_flow.rs`.
- [x] Scope-exact human-inspectable JSON graph export: кожний resolved evidence
  link простежується до MemoryUnit, session/archive provenance та legacy
  candidate/reinforcement origin; unresolved legacy IDs позначені явно. —
  2026-08-01.
- [x] Compatibility projection: чинні reflection/review API продовжують
  працювати як `CandidateBelief`, водночас graph-only abstraction,
  contradiction, validity і evidence-directness fields не стираються під час
  legacy update. Cross-scope relations fail closed. — 2026-08-01.
- [x] Перший прогін journaled migration на 565-file копії живого Chibigochi
  storage виконано: deterministic controls і product regression PASS, source
  не змінено, але owner storage мав 0 candidates/reinforcements, тому mapping,
  provenance і scope controls чесно лишилися `DATASET GAP`. Наступний
  невироджений fixture нижче закрив implementation gate, не переписуючи цей
  історичний результат. — 2026-08-01,
  [acceptance result](../releases/v0.6-hypothesis-graph-storage-acceptance.md#result-2026-08-01-deterministic-pass-live-dataset-gap).
- [x] Phase 2 закрито невиродженим disposable host-path fixture через реально
  rebuilt Chibigochi module: owner/other/unscoped candidates, resolved/missing
  units, duplicate/new reinforcement, exact-scope export, projection fallback
  і repeat idempotence пройшли; empty arrays перевірені direct indexing;
  owner storage не мігрувався. Build мав dirty-tree provenance caveat, але
  audited graph/migration/adapter paths між `e0ad0fa` і чистим descendant
  `8875e9a` ідентичні. — 2026-08-01,
  [final fixture result](../releases/v0.6-hypothesis-graph-storage-acceptance.md#final-result-2026-08-01-non-degenerate-fixture-pass).
- [ ] Trigger-related candidate subgraph замість recency-only останніх N:
  structural baseline плюс optional local-vector enrichment у глибокому шарі;
  non-vector fallback повний, vectors не визначають scope або truth.
- [ ] Typed hierarchical actions: narrow hypothesis, reinforcement,
  generalization/specialization proposal, possible equivalence і no-insight.
- [ ] Core-owned confidence components: directness, independent sources,
  diversity, user confirmation, generalization distance, recency і
  contradiction pressure. Score використовується як support, не публічна
  probability of truth.
- [ ] Evidence propagation не double-count-ить один source unit; широкі
  гіпотези дозрівають лише через незалежні гілки.
- [ ] Acceptance: Mars → planets → space hierarchy, later Venus evidence,
  понад candidate limit, paraphrase duplicate, semantic non-equivalent control,
  migration/replay і non-vector mode.

Нормативний план:
[Reflective Belief System, Phases 2–3](reflective-belief-system.md#phase-2--hypothesis-graph-storage-v06-foundation).

---

## v0.7 — Contradiction, temporal revision and clarification

Мета: памʼять не лише накопичує підтримку, а й уміє визнати надмірне
узагальнення, зміну людини з часом або потребу обережно уточнити власне
розуміння.

- [ ] Structured dispute lifecycle поверх hypothesis graph: support і
  contradiction існують одночасно до осмисленого revision/review.
- [ ] Reflection outcomes для narrow, split, time-bound, confidence decrease,
  contested/retract recommendation і clarification-needed.
- [ ] Пряме актуальне user denial має вищий авторитет за виведене
  generalization, але не стирає історичну правду та source evidence.
- [ ] Core challenge використовує той самий dispute/recovery lifecycle і не
  обходить owner review.
- [ ] Opt-in `ClarificationOpportunity`: core-owned relevance gate, cooldown і
  question budget; prompt hint зʼявляється лише під час природного повернення
  до теми й зникає після topic switch.
- [ ] Відповідь на уточнення проходить звичайний Session → sleep → fidelity →
  reflection шлях, а не патчить hypothesis напряму.
- [ ] Live acceptance на реальних суперечностях і зміні інтересу; окремо
  виміряти доречність, повтори та небажані питання.

Нормативний план:
[Reflective Belief System, Phases 4–5](reflective-belief-system.md#phase-4--contradiction-time-and-revision-v07).

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
8. **Vector storage — opt-in і derived.** Embeddings не є шаром або джерелом
   істини; користувач/власник має повний режим без vector storage, а індекс
   завжди можна перебудувати з MemoryUnit.
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
