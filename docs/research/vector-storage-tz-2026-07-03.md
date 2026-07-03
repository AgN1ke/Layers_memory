# ТЗ: Векторне сховище і глибокий recall (opt-in, вимикається) — 2026-07-03

## Для чого існує цей документ

Це імплементаційне ТЗ для Кодекса. Власник ухвалив рішення: робимо векторне сховище зараз, з можливістю повністю вимкнути його, і тестуємо на живих даних.

Документ базується на драфті `docs/research/vector-recall.md` (прийнятий як напрям) і **розв'язує всі його `TODO(align)`** проти фактичного коду v0.2/v0.3: MemoryUnits, fidelity, forgetting, buffered recall stats, multi-speaker gate, реальна розкладка `memory/` (без per-scope каталогів верхнього рівня). Де це ТЗ суперечить драфту — діє це ТЗ; розбіжності перелічені в розділі 2 з причинами.

Порядок роботи: фази A → B → C, кожна — окрема гілка з review, HISTORY-записом (зміни контрактів/поведінки), DEVLOG-записом і оновленням roadmap. Фаза C вмикається тільки після калібрування на живих даних фази B.

## 1. Інваріанти (кожен — тест або структурна гарантія)

1. **Ядро не рахує embeddings і не робить мережевих викликів.** Вектори приходять від хоста через існуючий `PendingTask`-механізм (`TaskType::ComputeEmbedding` вже оголошений у `tasks.rs`) і через `query_vec` у recall-запитах.
2. **Вимкнено = справді вимкнено.** Дефолт — disabled. Для вимкненого scope: жодного embed-таска, жодного читання векторів, звичайний recall байтово ідентичний сьогоднішньому, текст чату ніколи не потрапляє в embedder.
3. **Вектори — похідні дані, не правда.** Джерело правди — тези `MemoryUnit`. Векторний каталог можна видалити (purge) і перебудувати (rebuild) без втрати інформації. Жоден код-шлях не читає з векторів нічого, крім векторів.
4. **Приватність за замовчуванням.** Embedder — **локальний** (fastembed/ONNX на CPU, див. §6). Текст пам'яті не йде в жодний зовнішній embedding-API. Це host-policy, зафіксована в README хоста; ядро зберігає `model_id` інформаційно.
5. **Мультиспікерний gate.** Units із сесій, де `session_is_multi_speaker()` == true (хелпер уже є в `engine/sleep_flow.rs`), **не embed-яться**, доки не закриті гілки 1b–2 атрибуції. Це хірургічне розв'язання записаної залежності «vector storage тільки після атрибуції»: однокористувацькі scope (приватний Telegram, Chibigochi) embed-яться зараз безпечно, бо «користувач» там однозначний; групові чекають атрибутованих тез. Після гілки 2 gate знімається одним рядком + backfill.
6. **Model/dim mismatch → відмова, не тихе прийняття.** На submit і на recall_deep.
7. **Ізоляція scope.** Пошук ніколи не перетинає scope; файли одного scope живуть тільки в його підкаталозі.
8. **Один процес** (існуюча модель конкурентності); нові локи через існуючий `LockRegistry`, ключ `vectors:<scope>`, порядок `session → vectors`, зворотного не існує.

## 2. Що прийнято з драфту і що змінено (résolution всіх TODO(align))

**Прийнято без змін:** локальний embedder через fastembed/ONNX на CPU; flat brute-force cosine за трейтом `VectorIndex` (HNSW — свідомо ні); бінарний формат `vectors.f32` + `rows.jsonl` + recovery-правило R1; compaction; `min_sim`-чесність («не пам'ятаю» замість слабких збігів); on-demand tool-based deep recall (M1: НЕ every-turn RAG); scarcity top_k=5 + token budget; калібрування `min_sim` перед шипінгом (§8); performance envelope.

**Змінено після live-перевірки 2026-07-03:** початковий вибір `intfloat/multilingual-e5-small` ретрактовано, бо `fastembed==0.8.0` не має цієї моделі в `TextEmbedding.list_supported_models()`. Дефолт v1 — `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (384 dim, multilingual, fastembed-supported, prefixes not required but harmless).

**Змінено (з причин, зафіксованих у maintainer note 2026-06-10 і Записі 117):**

1. **Embed-иться теза `MemoryUnit`, не «canonical archive text».** Одиниця індексу — unit. Критерії: `status == ActiveArchive` && `fidelity_status != Rejected` && сесія не мультиспікерна. `Forgotten` → tombstone; `remember_back` → re-embed (backfill підхопить за відсутністю рядка).
2. **Розкладка**: у проєкті немає `memory/<scope>/` — scope це поле. Вектори живуть у `memory/archive/vectors/<scope>/` (scope = `source_session_id` unit-а; для Telegram це `telegram_<chat_id>`, що дає **per-chat** opt-in із §8.4 драфту).
3. **`events.jsonl` з драфту скасований.** Reinforcement повернутих спогадів іде через **існуючий** buffered recall stats шлях (`record_recall_stats` по батьківських archive entries, flush за існуючими правилами). Лічильники в rows.jsonl не зберігаються. Для tombstones — окремий append-only `tombstones.jsonl` (тільки id, merge при compaction).
4. **Без `register_embedder` у PyO3-адаптері.** Нормативний core API приймає готовий `query_vec`; текст→вектор — це помічник у `bot.py` (окремий модуль хоста), не в адаптері. Адаптер лишається тонким JSON-шаром.
5. **Task kind:** реюз існуючого `TaskType::ComputeEmbedding` (не новий enum). `prompt_id: "embed_batch"` — операційна мітка, БЕЗ файлу в `prompts/` (це не промпт; зафіксувати в contracts.md, щоб не порушувати правило prompts/README). `role_hint: Fast` — документовано ігнорується хостом для embedding-тасків.
6. **Legacy-поля `ArchiveEntry.embedding_model_id` / `embedding`** (entry-рівень) — НЕ використовуються; позначити в contracts.md як deprecated-зарезервовані, без міграції.
7. **Scoring recall_deep:** `score = sim + w_recency * recency + w_weight * unit.weight` (recency за існуючою half-life формулою `half_life_decay_factor`; `strength` з recall_count відкладений — unit-рівневих лічильників ще немає, це записаний борг).
8. **Стан scope** живе у `memory/archive/vectors/<scope>/manifest.json` (`building|ready|corrupt`); «disabled» = відсутність каталогу. Глобальний master-switch — існуючий `Manifest.features.embeddings_enabled` (обидва мають бути true, щоб щось embed-илось).

## 3. Storage (Фаза A)

```
memory/archive/vectors/<scope>/
  manifest.json     # {schema_version:"vector_index.v1", model_id, dim, metric:"cosine",
                    #  normalized:true, rows, state:"building|ready|corrupt",
                    #  built_at, updated_at, backfill_cursor: Option<unit_created_at>}
  vectors.f32       # LE f32, row-major, L2-нормалізовані (ядро нормалізує захисно)
  rows.jsonl        # {"row":0,"memory_unit_id":"mu_...","archive_id":"archive_...",
                    #  "created_at":"...","thesis_hash":"sha256:..."}
  tombstones.jsonl  # {"memory_unit_id":"mu_..."} — forgotten/rejected units
```

- **Append-протокол** (під `vectors:<scope>`): дописати вектори → дописати рядки → переписати manifest атомарно (існуючий tmp+rename патерн).
- **R1 на відкритті:** `len(vectors.f32) > L*dim*4` → truncate (падіння між append-ами); `<` → `state=corrupt`, вихід тільки через rebuild.
- **Compaction:** під час sleep, якщо є tombstones: переписати пару файлів без tombstoned-рядків, tmp+atomic rename, manifest останнім, tombstones.jsonl обнулити.
- **Дедуп:** unit уже в rows.jsonl зі збіжним `thesis_hash` → не emit-иться повторно. Хеш не збігся (теза відредагована/ревізована) → tombstone старого рядка + новий embed.

### Core API (Фаза A)

```rust
pub fn vector_state(&self, scope: &str) -> Result<VectorScopeState>;      // Disabled|Building|Ready|Corrupt + rows + model_id
pub fn set_vector_scope(&self, scope: &str, enabled: bool, purge: bool) -> Result<VectorScopeState>;
pub fn rebuild_vectors(&self, scope: &str) -> Result<VectorScopeState>;   // purge + enable
pub fn pending_embedding_backfill(&self, scope: &str) -> Result<Vec<LlmRequest>>; // батчі ComputeEmbedding
pub fn resume_compute_embedding(&self, task_id: &str, result: EmbedBatchResult) -> Result<usize>; // валідує model/dim, нормалізує, append
```

- `pending_embedding_backfill`: сканує eligible units scope (див. інваріант 5), пропускає наявні з валідним хешем, ріже на батчі ≤ `embed_batch_size` (конфіг, default 64), просуває `backfill_cursor`; коли хвіст порожній → `state=ready`.
- Виклик-точки: після `finish_sleep_run` (нові units цього sleep → engine повертає embed-запити в `SleepOutcome.embedding_requests`, симетрично до `fidelity_requests`); після enable; після rebuild; після `remember_back`.
- Payload-и (contracts.md):

```json
// inputs таска / LlmRequest.prompt_inputs
{ "kind": "embed_batch", "scope": "telegram_311422683",
  "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2", "dim": 384,
  "items": [ { "memory_unit_id": "mu_...", "text": "<теза unit-а>" } ] }
// результат хоста (embed_batch_result.v1), повертається як LlmResponse::Ok { text: <цей JSON> }
{ "schema_version": "embed_batch_result.v1",
  "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2", "dim": 384,
  "results": [ { "memory_unit_id": "mu_...", "vector": [0.01, -0.02] } ] }
```

## 4. `recall_deep` (Фаза B)

```rust
pub struct DeepRecallQuery { pub scope: String, pub query_vec: Vec<f32>, pub model_id: String,
    pub top_k: usize /*0=default 5*/, pub min_sim: f32 /*0.0=конфіг*/, pub now: Option<Timestamp> /*інжект для тестів*/ }
pub struct DeepRecallHit { pub memory_unit_id: Id, pub archive_id: Id, pub thesis: String,
    pub created_at: Timestamp, pub sim: f32, pub score: f32 }
pub struct DeepRecallResult { pub found: bool, pub reason: Option<String> /*disabled|building|below_threshold|corrupt*/,
    pub hits: Vec<DeepRecallHit> }
pub fn recall_deep(&self, query: DeepRecallQuery) -> Result<DeepRecallResult>;
```

- Алгоритм: нормалізувати query → dot по всіх не-tombstoned рядках (кеш матриці в RAM per scope, lazy, інвалідація на append/compaction, LRU за `cache_max_scopes`) → gate по `min_sim` на сирому `sim` → ранж за score → top_k → token budget (existing estimator, тези й так короткі).
- Side effect: `record_recall_stats` по batьківських archive_id повернутих hits (існуючий буфер, нуль синхронних записів на диск).
- PyO3: `recall_deep(query_json) -> json` — тонкий, як усе інше.
- `memory_terminal`: `vectors status|enable|disable [--purge]|rebuild <scope>`, `recall-deep <scope> "запит" [--debug]` (debug: топ-20 сирих sim без порога — інструмент калібрування §8). Термінал НЕ рахує вектори; для debug-запитів він приймає `--query-vec-file` або хост-скрипт (термінал без embedder-а лишається без query-функції — це ок, калібрування йде через bot-скрипт).

## 5. Stage 2 re-rank у звичайному recall (Фаза C — після калібрування)

- `RecallQuery.query_embedding: Option<Vec<f32>>` + `CoreContextRequest.query_embedding` (serde default; протягнути в внутрішній recall).
- Якщо вектор присутній && scope ready: над top-K (конфіг, default 40) кандидатів Stage 1 — `final = stage1_score * (1 - w_vec) + max_unit_cosine * w_vec` (`w_vec` конфіг, старт 0.5; max по активних units entry). `RecallResult.stage_used = Stage2`, `relevance_explanation` показує обидві складові.
- Без вектора / вимкнено — Stage 1 байтово як зараз (це тест).
- Host: локальний `query_embed` на кожен хід дешевий (CPU, ~мс), але Фаза C вмикається окремим рішенням після того, як B-калібрування покаже реальні sim-розподіли.

## 6. Host-частина (Telegram, Фаза B)

1. Новий модуль `hosts/telegram_gemini_bot/local_embedder.py`: fastembed `TextEmbedding("sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2")`, `embed_passages` / `embed_query`, L2-нормалізація, lazy-ініціалізація (перше завантаження ONNX — повідомити в лог). `fastembed` — нова dev-залежність venv, зафіксувати в `docs/local-development.md`.
2. Виконання `ComputeEmbedding`-тасків: після sleep (з `SleepOutcome.embedding_requests`) і по `pending_embedding_backfill` при enable — батчами через local_embedder, submit через `resume_compute_embedding`.
3. Gemini function-calling tool `recall_distant_memory` (декларація і system-prompt доповнення — з драфту §8.2, перекласти в `chibigochi/telegram` персону відповідно): виклик → typing-екшн → `embed_query` → `engine.recall_deep` → tool result `{found, reason, memories:[{when: <мітка з існуючого time-labels bucketing — реюз>, sim, strength: vivid|faint, text: thesis}]}`. Немає збігів → модель чесно каже, що не пам'ятає.
4. Команди: `/vectors` (status), `/vectors_on`, `/vectors_off`, `/vectors_purge` (з підтвердженням другим повідомленням), scope = поточний чат.
5. Telemetry: `token_usage.jsonl`-стиль лог embed-батчів (кількість units, dims, тривалість) і recall_deep викликів (query hash, found, top sim) — без текстів.

## 7. Тести (детермінований fake-embedder: seeded hash тези → псевдовектор; жодних завантажень моделей у CI)

Фаза A: roundtrip f32; R1-обидва напрями; дедуп за хешем; tombstone на forget + re-embed на remember_back; compaction; purge видаляє каталог і НЕ чіпає units/archives; rebuild детермінований; disabled scope → нуль тасків (інваріант 2); мультиспікерна сесія → нуль тасків (інваріант 5); model/dim mismatch → відмова.
Фаза B: min_sim-gate (кандидат нижче порога не проходить навіть із max recency); score-ранжування з інжектованим now; scarcity top_k; scope-ізоляція; reason-семантика disabled/building/below_threshold; reinforcement потрапляє в buffered stats.
Фаза C: увімкнений Stage 2 змінює порядок там, де парафраз без текстового збігу (два entries з fake-векторами); без query_embedding — байтова ідентичність Stage 1; `stage_used` коректний.
Conformance: новий `--host direct-vectors` — детермінований сценарій через PyO3 з fake-векторами: enable → sleep → backfill → recall_deep знаходить парафраз, який Stage 1 текстово не матчить; off → found:false/reason=disabled. Лінійні хости без змін.

## 8. Калібрування (обов'язково до Фази C і до заявлених висновків)

На живому Telegram-архіві власника: увімкнути вектори, ~10 реальних «пам'ятаєш...» запитів через `--debug`-шлях, записати sim найслабшого true positive і найсильнішого false positive, поставити `min_sim` між ними, `vivid_sim` біля кластера сильних. Значення + model_id → у DEVLOG. Live-калібрування 2026-07-03 для `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` поставило стартові `min_sim=0.30`, `vivid_sim=0.55`.

## 9. Чого свідомо не робити (v1)

HNSW/квантизація; near-dup merge на індексації; реконсолідація; drill-down до raw-діалогу; міжпроцесні локи; шифрування векторів; векторний recall по шару Ядро; unit-рівневі recall-лічильники (окремий записаний борг); зміна `compact_memory`/prompt view (deep recall — це tool-шлях, не зміна звичайного prompt-у).

## 10. Дисципліна

- Кожна фаза: HISTORY-запис (нові контракти: vector_index.v1, embed_batch_result.v1, deep_recall.v1, query_embedding поля), contracts.md, roadmap-галочки у секції «Vector storage і Recall Stage 2/3», DEVLOG.
- Roadmap: рядок explicit-залежності від атрибуції ЗАМІНЮЄТЬСЯ на surgical gate (інваріант 5) — рішення власника 2026-07-03; після гілки 2 мультиспікерності gate знімається + backfill групових scope.
- Повний release gate + всі conformance-хости на кожній фазі.
