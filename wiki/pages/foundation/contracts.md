# Контракти даних Memory Engine v0.1

## Related pages

- [Architecture](architecture.md) - conceptual boundaries behind these data
  contracts.
- [Strategy](strategy.md) - product-level principles these contracts protect.
- [Vector storage implementation spec](../research/vector-storage-tz-2026-07-03.md)
  - vector-specific contracts and payloads accepted after v0.3.
- [LLM integration resources](../integration/llm-integration-resources.md) -
  provider/model resources that execute LLM tasks outside the core.
- [Roadmap](../planning/roadmap.md) - current and planned schema-impacting work.
- [Reflective Belief System](../planning/reflective-belief-system.md) - phased
  source for future reflection-operation and hypothesis-graph contracts.
- [Server-scale Memory Runtime](../planning/server-scale-memory-runtime.md) -
  accepted vault, transaction and service contracts; S1 identity and additive
  S2 reference flows are implemented, while PostgreSQL remains pending.

## Для чого існує цей документ

`wiki/pages/foundation/architecture.md` фіксує архітектурну форму Memory Engine: ядро системи, три шари пам'яті, PendingTask, recall, sleep, storage і адаптери.

Цей документ фіксує наступний рівень: **точні форми даних**, які потрібні для першого MVP. Його задача - зробити так, щоб Rust-код, Python-адаптер, файлове сховище і майбутні тести говорили однією мовою.

Це ще не JSON Schema-файл і не Rust-код. Це контрактний документ для людини і ШІ-розробника. На його основі далі можна створювати:

- Rust `struct` і `enum`;
- JSON Schema fixtures;
- тести серіалізації;
- приклади файлів у `memory/`;
- Python typing для адаптера.

Документ описує v0.1. Він навмисно не намагається повністю описати reflection, embeddings, LLM recall re-rank або Godot-адаптер. Для v0.1 вони або відсутні, або мають зарезервовані поля.

---

## 1. Загальні правила форматів

### 1.1 JSON

Усі машинні файли зберігаються як UTF-8 JSON або JSONL.

Правила:

- назви полів у `snake_case`;
- час у форматі RFC 3339 / ISO 8601 UTC, наприклад `2026-05-17T16:18:05.000Z`;
- ідентифікатори як рядки;
- числові ваги і score у діапазоні `0.0..1.0`, якщо окремо не вказано інше;
- unknown fields у вхідних payload хоста дозволені і зберігаються;
- unknown fields у власних структурах ядра не мають мовчки змінювати поведінку.

### 1.2 JSONL

JSONL-файл містить один JSON-об'єкт на рядок.

На v0.1 JSONL використовується для:

- `memory/sessions/<session_id>/events.jsonl`;
- `memory/sessions/<session_id>/archived/events-<NNN>.jsonl` for raw events rotated after they are covered by Complete archives.

Кожен рядок має бути валідним `StoredEvent`.

### 1.3 Markdown

Markdown-файли існують для людини. Вони не замінюють машинні JSON/JSONL-файли.

Правила:

- Markdown має бути читабельним без знання Rust-коду;
- Markdown може містити YAML-frontmatter, якщо це корисно;
- якщо Markdown і JSON розходяться, джерелом правди є JSON/JSONL;
- ручне редагування Markdown не повинно змінювати машинний стан, поки немає спеціальної команди імпорту.

### 1.4 Версії схем

Кожна персистентна структура має `schema_version`.

У v0.1 використовуємо рядковий формат:

```json
"schema_version": "event.v1"
```

Початкові версії:

- `event.v1`;
- `ingest_result.v1`;
- `session.v1`;
- `archive_entry.v1`;
- `core_store.v1`;
- `core_fact.v1`;
- `core_fact_input.v1`;
- `core_fact_patch_input.v1`;
- `core_fact_patch_result.v1`;
- `core_fact_upsert_result.v1`;
- `core_context_request.v1`;
- `core_context_package.v1`;
- `candidate_belief.v1`;
- `recall_query.v1`;
- `recall_result.v1`;
- `pending_task.v1`;
- `evidence_pack.v1`;
- `fidelity_review.v1`;
- `sleep_compression_result.v1`;
- `manifest.v1`;
- `journal_operation.v1`.

Зміна формату будь-якої з цих структур потребує оновлення відповідної версії і міграції, якщо старі дані вже могли бути записані.

---

## 2. Спільні типи

### 2.1 Id

`Id` - рядок, стабільний у межах свого типу.

Рекомендовані префікси:

- `event_...`;
- `session_...` або людський формат `2026-05-17_001`;
- `archive_...`;
- `core_fact_...`;
- `candidate_...`;
- `task_...`;
- `journal_...`.

Ядро системи може приймати `session_id` від хоста, але всі інші внутрішні id має генерувати саме.

### 2.2 Timestamp

`Timestamp` - рядок UTC:

```json
"2026-05-17T16:18:05.000Z"
```

Хост може передати `timestamp` події. Ядро системи додатково пише `received_at`, щоб розрізняти час події і час прийому.

### 2.3 Link

Зв'язок між подіями, спогадами, Core-фактами або кандидатами.

```json
{
  "kind": "follow_up",
  "target": "event:event_01H...",
  "note": "Гравець вилікував героїню після цього рейду."
}
```

Поля:

- `kind` - рядок. Базові значення v0.1: `follow_up`, `supports`, `contradicts`, `related`, `source`, `promoted_from`.
- `target` - рядок у форматі `<type>:<id>`, наприклад `archive:archive_01H...`.
- `note` - необов'язкове людське пояснення.

### 2.4 ImportanceHint

Підказка від хоста, не остаточне рішення.

Допустимі значення:

- `low`;
- `normal`;
- `medium`;
- `high`;
- `critical`.

Якщо поле відсутнє, ядро трактує його як `normal`.

### 2.5 ProcessingMode

Як хост хоче обробити подію.

Допустимі значення:

- `immediate`;
- `defer_to_sleep`.

Якщо поле відсутнє, типове значення - `defer_to_sleep`.

### 2.6 ModelRole

Роль моделі, яку ядро системи може попросити через PendingTask.

Допустимі значення:

- `reasoning`;
- `balanced`;
- `fast`.

Ядро системи не знає конкретних провайдерів і моделей. Хост мапить `ModelRole` на реальний `provider + model` у конфігу.

---

## 3. Event

Подія - єдиний канал входу інформації у ядро системи.

Є дві форми:

- `IngestEvent` - те, що хост передає в `engine.ingest()`;
- `StoredEvent` - те, що ядро записує у `events.jsonl`.

### 3.1 IngestEvent

```json
{
  "schema_version": "event.v1",
  "type": "user_message",
  "source": "telegram_user_42",
  "timestamp": "2026-05-17T16:32:11.420Z",
  "session_id": "2026-05-17_005",
  "payload": {
    "text": "Я переїхав у Берлін минулого місяця",
    "chat_id": 42
  },
  "tags": ["personal_fact", "location"],
  "theme": "personal_background",
  "emotional_tone": "neutral",
  "speaker": { "id": "tg_311422683", "name": "Микита" },
  "links": [{ "kind": "reply_to", "target": "event_01J00000000000000000000009" }],
  "importance_hint": "high",
  "processing_mode": "defer_to_sleep"
}
```

Обов'язкові поля:

- `schema_version`;
- `type`;
- `source`;
- `timestamp`;
- `session_id`;
- `payload`.

Опціональні поля:

- `tags`;
- `theme`;
- `emotional_tone`;
- `speaker`;
- `links`;
- `importance_hint`;
- `processing_mode`.

Правила:

- `payload` належить хосту і може мати довільну JSON-структуру;
- ядро системи не має жорстко знати всі можливі `type`;
- `tags` мають бути короткими машинними рядками;
- `theme` - одна основна тема, якщо хост її знає;
- `emotional_tone` - рядок, без фіксованого enum на v0.1;
- `speaker` - `{ "id", "name" }` автора події в мультиспікерній сесії (груповий чат). `id` — стабільна ідентичність хоста (імена змінюються і колізять), `name` — prompt-facing форма. Відсутній `speaker` зберігає стару однокористувацьку семантику; лінійні хости поле не заповнюють. Сесія вважається мультиспікерною, коли серед `user_message` подій є ≥2 різних `speaker.id` — тоді автоматичний Archive → Core bridge вимикається (Фаза 1 захисту від «пліток у Core»), а Core росте через `/remember` і reflection + review;
- `links` типізовані: `{ "kind", "target" }`. Конвенція kind `reply_to`: `target` — `event_id` повідомлення, на яке відповідає ця подія. Reply-структура живе в сирому шарі сесії, використовується під час sleep-дистиляції і не зберігається в Archive/MemoryUnit (зміст замість структури); не плутати з `ArchiveEntry.links` — то звʼязки спогадів на іншому шарі.

### 3.2 StoredEvent

`StoredEvent` - це `IngestEvent` плюс поля ядра.

```json
{
  "schema_version": "event.v1",
  "event_id": "event_01J00000000000000000000001",
  "received_at": "2026-05-17T16:32:12.003Z",
  "type": "user_message",
  "source": "telegram_user_42",
  "timestamp": "2026-05-17T16:32:11.420Z",
  "session_id": "2026-05-17_005",
  "payload": {
    "text": "Я переїхав у Берлін минулого місяця",
    "chat_id": 42
  },
  "tags": ["personal_fact", "location"],
  "theme": "personal_background",
  "emotional_tone": "neutral",
  "links": [],
  "importance_hint": "high",
  "processing_mode": "defer_to_sleep",
  "initial_weight": 0.75,
  "weight_reason": "High importance hint and personal_fact tag."
}
```

Додаткові поля ядра:

- `event_id` - генерує ядро;
- `received_at` - час прийому ядром;
- `initial_weight` - попередня вага `0.0..1.0`;
- `weight_reason` - коротке пояснення для debug і людського аудиту.

`events.jsonl` і `archived/events-<NNN>.jsonl` містять саме `StoredEvent`.

### 3.3 IngestResult

`IngestResult` - відповідь `engine.ingest(event)`.

```json
{
  "schema_version": "ingest_result.v1",
  "stored_event": {
    "schema_version": "event.v1",
    "event_id": "event_01J00000000000000000000001",
    "received_at": "2026-05-17T16:32:12.003Z",
    "type": "user_message",
    "source": "telegram_user_42",
    "timestamp": "2026-05-17T16:32:11.420Z",
    "session_id": "2026-05-17_005",
    "payload": {
      "text": "Я переїхав у Берлін минулого місяця",
      "chat_id": 42
    },
    "tags": ["personal_fact", "location"],
    "theme": "personal_background",
    "importance_hint": "high",
    "processing_mode": "defer_to_sleep",
    "initial_weight": 0.75,
    "weight_reason": "High importance hint and personal_fact tag."
  }
}
```

Sleep запускається окремим викликом `engine.sleep(session_id)`: вручну, при token-budget pressure або за scheduled idle trigger у хості. Хост не має стискати raw chat сам; він тільки запускає sleep і виконує повернені `PendingTask` через свій LLM-провайдер.

---

## 4. Session

Сесія - робоча пам'ять поточної взаємодії.

### 4.1 SessionMetadata

Файл: `memory/sessions/<session_id>/session.json`.

На v0.1 цей файл корисний, але може з'явитися разом із першим кодом. `events.jsonl` є мінімально обов'язковим.

```json
{
  "schema_version": "session.v1",
  "session_id": "2026-05-17_005",
  "host_id": "telegram_bot",
  "status": "active",
  "created_at": "2026-05-17T16:30:00.000Z",
  "updated_at": "2026-05-17T16:45:00.000Z",
  "closed_at": null,
  "event_count": 12,
  "summary": "Розмова про переїзд користувача і поточну роботу.",
  "active_theme": "personal_background",
  "tags": ["personal_fact", "location", "work"],
  "archived_to": [],
  "archived_event_ids": [],
  "archived_event_index_complete": true,
  "active_sleep_task_id": null,
  "sleep_pending_event_ids": [],
  "notes": []
}
```

Поля:

- `session_id` - збігається з папкою;
- `host_id` - короткий id хоста;
- `status` - `active`, `sleep_pending`, `archived`, `failed`;
- `created_at`, `updated_at`, `closed_at`;
- `event_count`;
- `summary` - короткий машинно/людський підсумок;
- `active_theme`;
- `tags`;
- `archived_to` - список complete `archive_id`, створених зі сесії;
- `archived_event_ids` - перебудовний cache raw `event_id`, вже покритих Complete-архівами цієї сесії;
- `archived_event_index_complete` - `true`, якщо cache можна читати без повного archive scan; старі metadata без цього поля перебудовуються з Complete archive entries;
- `active_sleep_task_id` - additive Storage v2 reservation: ID єдиного активного
  sleep workflow цієї Session. `null` означає, що активної резервації немає;
- `sleep_pending_event_ids` - raw events, відібрані активним sleep workflow, але
  ще не покриті Complete-архівом. Це processing state, не archive coverage;
- `notes` - службові або людські нотатки.

`active_sleep_task_id` і `sleep_pending_event_ids` мають serde-default, тому
старі `session.v1` records без них читаються як `null` та `[]`. Узгоджений
pending-стан має `status=sleep_pending`, непорожній task ID і непорожній список
подій. `archived_to` та `archived_event_ids` змінюються тільки після переходу
Archive у `complete`: preliminary Archive і pending reservation не мають права
видавати незавершену консолідацію за канонічне покриття.

JSON contract є backward-compatible, але додавання public полів змінює Rust
struct literals. За pre-1.0 release governance такий source-level change
виходить у наступній minor версії з migration note, не як patch release.

### 4.2 session.md

Файл: `memory/sessions/<session_id>/session.md`.

Це людський вигляд сесії. Мінімальна форма:

```markdown
---
schema_version: session_view.v1
session_id: 2026-05-17_005
status: active
updated_at: 2026-05-17T16:45:00.000Z
---

# Сесія 2026-05-17_005

## Коротко

Розмова про переїзд користувача і поточну роботу.

## Події

- 16:32:11 user_message: користувач повідомив, що переїхав у Берлін.
```

Правила:

- файл створюється для читання людиною;
- не є джерелом правди для коду;
- може бути перебудований із `events.jsonl`, `archived/events-<NNN>.jsonl` і `session.json`.

---

## 5. ArchiveEntry

Архівний спогад - довгостроковий запис, створений зі сесії.

Файли:

- `memory/archive/<YYYY>/<MM>/<archive_id>.json`.

`<archive_id>.json` є pretty-printed JSON і поточною human-inspectable формою ArchiveEntry. Окремий `<archive_id>.md` dump не є частиною v0.2 storage contract; якщо він зʼявиться, це має бути похідний export, не джерело правди.

### 5.1 JSON

```json
{
  "schema_version": "archive_entry.v1",
  "archive_id": "archive_01J00000000000000000000001",
  "created_at": "2026-05-17T17:10:00.000Z",
  "updated_at": "2026-05-17T17:12:00.000Z",
  "source_session_id": "2026-05-17_005",
  "source_event_ids": [
    "event_01J00000000000000000000001",
    "event_01J00000000000000000000002"
  ],
  "time_range": {
    "start": "2026-05-17T16:30:00.000Z",
    "end": "2026-05-17T17:00:00.000Z"
  },
  "theme": "personal_background",
  "tags": ["personal_fact", "location"],
  "gist": "Користувач повідомив, що переїхав у Берлін минулого місяця.",
  "narrative": "Під час розмови користувач уточнив важливу зміну в особистому контексті: він нещодавно переїхав у Берлін.",
  "compact_memory": "Переїзд у Берлін -> користувач повідомив стабільний особистий контекст, який варто пам'ятати.",
  "facts": [
    {
      "text": "Користувач переїхав у Берлін приблизно у квітні 2026 року.",
      "confidence": 0.8,
      "source_event_ids": ["event_01J00000000000000000000001"]
    }
  ],
  "quotes": [
    {
      "text": "Я переїхав у Берлін минулого місяця",
      "source_event_id": "event_01J00000000000000000000001"
    }
  ],
  "weight": 0.82,
  "freshness": 1.0,
  "recall_count": 0,
  "last_recalled_at": null,
  "links": [],
  "emotional_markers": [
    {
      "target": "cat_named_irzha",
      "affect": "fondness",
      "strength": 0.95,
      "source_event_ids": ["event_01J00000000000000000000002"],
      "quote": "У мене є кішечка Іржа",
      "evidence": "Користувач тепло розповів про важливу для нього тварину."
    }
  ],
  "topic_thread": [
    {
      "topic": "personal_pet",
      "subtopics": ["cat_name", "appearance"],
      "energy": "warm",
      "source_event_ids": ["event_01J00000000000000000000002"],
      "summary": "Користувач розповів про кішечку і походження її імені."
    }
  ],
  "personal_signals": [
    {
      "text": "Користувач має кішечку на ім'я Іржа.",
      "category": "relationships_with_pets",
      "confidence": 0.95,
      "source_event_ids": ["event_01J00000000000000000000002"],
      "evidence": "Пряма заява користувача."
    }
  ],
  "relational_tone": {
    "warmth": 0.8,
    "intellectual_engagement": 0.3,
    "intimacy": 0.5,
    "trust": 0.4,
    "playfulness": 0.3,
    "tension": 0.0,
    "summary": "Розмова стала теплішою через особисту згадку.",
    "source_event_ids": ["event_01J00000000000000000000002"]
  },
  "status": "preliminary",
  "llm_enhanced": false,
  "prompt_id": null,
  "prompt_version": null,
  "embedding_model_id": null,
  "embedding": null
}
```

Обов'язкові поля:

- `schema_version`;
- `archive_id`;
- `created_at`;
- `updated_at`;
- `source_session_id`;
- `source_event_ids`;
- `time_range`;
- `theme`;
- `tags`;
- `gist`;
- `narrative`;
- `facts`;
- `quotes`;
- `weight`;
- `freshness`;
- `recall_count`;
- `last_recalled_at`;
- `links`;
- `emotional_markers`;
- `topic_thread`;
- `personal_signals`;
- `relational_tone`;
- `status`;
- `llm_enhanced`;
- `prompt_id`;
- `prompt_version`;
- `embedding_model_id`;
- `embedding`.

`status`:

- `preliminary` - створений алгоритмічно на sleep-stage-1; це durable checkpoint
  активного workflow, але не recall-visible довгострокова памʼять і не
  канонічне покриття raw events;
- `complete` - оновлений після LLM-доробки або прийнятий як повний без неї;
- `superseded` - замінений новішим спогадом;
- `needs_review` - потребує ручного огляду.

`embedding_model_id` і `embedding` на v0.1 завжди `null`.

`compact_memory` — prompt-facing стиснута памʼять: plain-text тези "подія ->
висновок". Основний sleep driver отримує атомарні тези через
`memory_unit_pass`, валідує `MemoryUnit` і детерміновано рендерить із них
`compact_memory`; legacy API окремого `compact_memory_pass` лишається
compatibility-шляхом. Це не debug JSON і не довільна програмна проекція з full
archive. Ordinary chat recall читає лише Complete archive entries; якщо
`compact_memory` відсутній у Complete legacy entry, projection може
fallback-нути на `gist`.

Multi-track поля `emotional_markers`, `topic_thread`, `personal_signals`, `relational_tone` заповнюються multi-pass sleep у host-і для storage/debug audit, Core bridge і майбутнього reflection. Старі archive entries можуть не мати цих полів; reader має трактувати відсутні масиви як порожні, а відсутній `relational_tone` як `null`.

Поля, які дозволено лишати `null` або пропускати, якщо вони ще не створені:

- `theme` (сесія могла не мати чіткої теми);
- `compact_memory` може бути відсутнім у preliminary або legacy archive entry;
- `last_recalled_at` (поки спогад не повертався recall'ом);
- `prompt_id` (preliminary запис без LLM-доробки);
- `prompt_version` (те саме);
- `embedding_model_id` (на v0.1 завжди `null`);
- `embedding` (на v0.1 завжди `null`).

### 5.2 Deferred Markdown export

Окремий Markdown-виклад архівного спогаду не пишеться у v0.2. Нижче наведено можливу форму майбутнього derived export, якщо зʼявиться реальний workflow ручного читання архіву людиною.

Мінімальна форма:

```markdown
---
schema_version: archive_view.v1
archive_id: archive_01J00000000000000000000001
source_session_id: 2026-05-17_005
status: preliminary
weight: 0.82
freshness: 1.0
---

# Спогад: переїзд користувача

## Коротко

Користувач повідомив, що переїхав у Берлін минулого місяця.

## Наратив

Під час розмови користувач уточнив важливу зміну в особистому контексті.

## Факти

- Користувач переїхав у Берлін приблизно у квітні 2026 року.

## Цитати

- "Я переїхав у Берлін минулого місяця"
```

---

## 6. Core Store

Core Store - стабільна основа пам'яті.

Файли:

- `memory/core/store/<category>.json`.

`<category>.json` є pretty-printed JSON і поточною human-inspectable формою Core Store. Окремий `<category>.md` dump не є частиною v0.2 storage contract; якщо він зʼявиться, це має бути похідний export, не джерело правди.

### 6.1 CoreStoreCategory

```json
{
  "schema_version": "core_store.v1",
  "category": "personal_facts",
  "updated_at": "2026-05-17T17:20:00.000Z",
  "facts": [
    {
      "schema_version": "core_fact.v1",
      "core_fact_id": "core_fact_01J00000000000000000000001",
      "scope": "telegram_311422683",
      "text": "Користувач живе в Берліні.",
      "status": "active",
      "confidence": 0.82,
      "created_at": "2026-05-17T17:20:00.000Z",
      "updated_at": "2026-05-17T17:20:00.000Z",
      "source_archive_ids": ["archive_01J00000000000000000000001"],
      "source_candidate_id": "candidate_01J00000000000000000000001",
      "tags": ["personal_fact", "location"],
      "links": [],
      "review": {
        "reviewed_by": "owner",
        "reviewed_at": "2026-05-17T17:19:00.000Z",
        "decision": "approved",
        "note": "Підтверджено вручну."
      }
    }
  ]
}
```

`status`:

- `active`;
- `deprecated`;
- `contradicted`;
- `needs_review`.

На v0.1 Core Store змінюється через явний `engine.upsert_core_fact(...)` або спеціальну команду host-а, наприклад `/remember`. Host-рівневі heuristic rules можуть додавати теги до подій, але не мають напряму записувати plain text у Core. Повна автоматична промоція з CandidateBelief без огляду не входить у v0.1.

### 6.2 CoreFactInput

Вхід у `engine.upsert_core_fact(input)`.

```json
{
  "schema_version": "core_fact_input.v1",
  "category": "profile",
  "scope": "telegram_311422683",
  "text": "Користувача звати Микита.",
  "confidence": 0.95,
  "tags": ["telegram", "profile", "name"],
  "source_archive_ids": [],
  "source_candidate_id": null
}
```

Відповідь:

```json
{
  "schema_version": "core_fact_upsert_result.v1",
  "category": "profile",
  "created": true,
  "fact": {
    "schema_version": "core_fact.v1",
    "core_fact_id": "core_fact_01J00000000000000000000001",
    "scope": "telegram_311422683",
    "text": "Користувача звати Микита.",
    "status": "active",
    "confidence": 0.95,
    "created_at": "2026-05-17T17:20:00.000Z",
    "updated_at": "2026-05-17T17:20:00.000Z",
    "tags": ["name", "profile", "telegram"]
  }
}
```

`scope` визначає межу видимості факту. Для Telegram host використовується `session_id` виду `telegram_<chat_id>`, щоб факти одного чату не потрапляли в контекст іншого. Upsert дедуплікує факти за нормалізованим текстом і `scope` у межах категорії.

### 6.3 CoreFactPatchInput

Вхід у `engine.patch_core_fact(input)`.

```json
{
  "schema_version": "core_fact_patch_input.v1",
  "core_fact_id": "core_fact_01J00000000000000000000001",
  "scope": "telegram_311422683",
  "text": "Користувача звати Микита Загамула.",
  "status": "active"
}
```

Для forget/deprecate:

```json
{
  "schema_version": "core_fact_patch_input.v1",
  "core_fact_id": "core_fact_01J00000000000000000000001",
  "scope": "telegram_311422683",
  "status": "deprecated"
}
```

Відповідь:

```json
{
  "schema_version": "core_fact_patch_result.v1",
  "category": "profile",
  "fact": {
    "schema_version": "core_fact.v1",
    "core_fact_id": "core_fact_01J00000000000000000000001",
    "scope": "telegram_311422683",
    "text": "Користувача звати Микита Загамула.",
    "status": "active",
    "confidence": 0.95,
    "created_at": "2026-05-17T17:20:00.000Z",
    "updated_at": "2026-05-17T17:30:00.000Z",
    "tags": ["name", "profile", "telegram"]
  }
}
```

Patch шукає факт за `core_fact_id` і `scope`. Це захищає від редагування факту іншого Telegram-чату через випадково скопійований id. Факти зі статусом `deprecated`, `contradicted` або `needs_review` не повертаються в `CoreContextPackage.core_facts`.

### 6.4 CoreContextRequest

Вхід у `engine.core_context_package(request)`.

```json
{
  "schema_version": "core_context_request.v1",
  "session_id": "2026-05-17_005",
  "domain_state": {
    "active_topic": "travel_planning",
    "current_text": "Що ми говорили про літаки?"
  },
  "core_scope": "telegram_311422683",
  "query_text": "літаки",
  "query_embedding": {
    "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    "query_vec": [0.01, -0.02, 0.03]
  },
  "explain_recall": true,
  "recall_limit": 5,
  "session_recent_limit": 40,
  "session_trace_event_limit": 120,
  "include_core": false,
  "token_budget": {
    "total_tokens": 11000,
    "current_memory_tokens": 7000,
    "compressed_memory_tokens": 3000,
    "core_tokens": 1000
  },
  "utc_offset_minutes": 180,
  "clock_untrusted": false
}
```

`domain_state` приходить від хоста у момент запиту і не записується в Core Store. `core_scope` фільтрує `core_facts`; якщо він заданий, ядро повертає тільки факти з таким самим `scope`.

`query_embedding` необовʼязковий. У `core_context_package` він представляє
актуальний user turn і активує окремий ephemeral `associative_memory` над
готовим vector scope. Інтегруюча програма не повинна embed-ити вже зібраний
prompt або раніше підтягнуті асоціативні спогади: інакше старий retrieval сам
підтримуватиме власну тему. `archive_relevant` у цьому API лишається звичайним
structural recall; opt-in Stage 2 доступний через explicit `engine.recall(...)`.
Якщо embedding відсутній, associative section порожня, а non-vector package path
не змінюється.

`explain_recall` (default `false`) вмикає структурований `recall_debug` у відповіді. Це debug/telemetry payload, а не prompt content: він показує Stage 1 множники, початковий і фінальний ранги, vector status та Stage 2 складові, але не повинен передаватися LLM у звичайному chat prompt.

`utc_offset_minutes` (default `0`) — локальний зсув хоста для календарних часових міток у prompt view («вчора» — локальне поняття, сховище лишається в UTC). `clock_untrusted` (default `false`) — хост може позначити свій годинник ненадійним; тоді prompt view опускає часові мітки замість рендерити хибні. Відносні мітки ніколи не зберігаються у файлах — вони обчислюються при рендері з абсолютних timestamp-ів і `created_at` пакета, тому не «гниють» після простою.

`session_recent` і `session_trace` у відповіді містять тільки active tail: події поточної сесії, які ще не покриті жодним Complete archive entry через `source_event_ids`. Preliminary entry не прибирає їх із живого контексту. Події, які вже завершили sleep, повертаються через `archive_relevant`, а не дублюються як raw session events. Default rolling sleep лишає приблизно 30% найсвіжіших unarchived events active, якщо window досяг `partial_sleep_min_events`.

`token_budget` необовʼязковий. Якщо host його не передає, ядро використовує default 11k/7k/3k/1k. Це budget для memory context package, а не повний budget усього LLM-запиту. На v0.1 token count — детермінована оцінка за conservative estimator, не provider-specific tokenizer.

На v0.1 `core_facts` читаються з усіх файлів `memory/core/store/*.json`, а не з фіксованого списку категорій. Категорія є вільним normalized `snake_case` рядком, який приходить із LLM personal-signal pass або explicit host command. Старі категорії `profile/preferences/relationship` не є whitelist.

`core_context_package.v1` є API/debug контрактом. Звичайний LLM prompt може і повинен мати компактну host projection: рольовий transcript замість JSON-дампу, короткі archive bullets замість повного narrative/debug payload, без довгих технічних ID, якщо вони не потрібні для конкретної команди.

### 6.5 CoreContextPackage

Core Context Package не обов'язково зберігається на диск. Це відповідь ядра на запит хоста.

```json
{
  "schema_version": "core_context_package.v1",
  "created_at": "2026-05-17T17:25:00.000Z",
  "utc_offset_minutes": 180,
  "clock_untrusted": false,
  "core_facts": [
    {
      "category": "location",
      "core_fact_id": "core_fact_01J00000000000000000000001",
      "scope": "telegram_311422683",
      "text": "Користувач живе в Берліні.",
      "confidence": 0.82,
      "tags": ["personal_fact", "location"]
    }
  ],
  "session_recent": [
    {
      "event_id": "event_01J00000000000000000000002",
      "timestamp": "2026-05-17T17:20:00.000Z",
      "type": "user_message",
      "source": "telegram_user_42",
      "text": "А що треба для початку риболовлі?",
      "tags": ["telegram_message"],
      "theme": "telegram_conversation"
    }
  ],
  "session_trace": [
    {
      "event_id": "event_01J00000000000000000000001",
      "timestamp": "2026-05-17T17:10:00.000Z",
      "type": "user_message",
      "source": "telegram_user_42",
      "text": "Розкажи про МіГ-15.",
      "tags": ["telegram_message"],
      "theme": "telegram_conversation"
    }
  ],
  "archive_relevant": [
    {
      "source_layer": "archive",
      "id": "archive_01J00000000000000000000001",
      "gist": "Обговорили МіГ-15 і F-86 -> користувач цікавиться військовою авіацією Корейської війни.",
      "compact_memory": "Обговорили МіГ-15 і F-86 -> користувач цікавиться військовою авіацією Корейської війни.",
      "source_session_id": "2026-05-17_005",
      "tags": ["aircraft"],
      "theme": "aviation",
      "weight": 0.9,
      "freshness": 1.0,
      "relevance_score": 0.8
    }
  ],
  "associative_memory": [
    {
      "memory_unit_id": "mu_01J00000000000000000000009",
      "archive_id": "archive_01J00000000000000000000007",
      "scope": "2026-05-17_005",
      "thesis": "Раніше обговорювали Марс і умови на планетах земного типу.",
      "created_at": "2026-02-10T12:00:00.000Z",
      "activation": "current_topic_vector",
      "semantic_similarity": 0.71,
      "activation_threshold": 0.47,
      "deep_recall_score": 0.76,
      "why_recalled": "current topic vector matched an older memory unit"
    }
  ],
  "budget": {
    "estimator": "unicode_chars_div_2_ceil_json_v1",
    "total_budget_tokens": 11000,
    "current_memory_budget_tokens": 7000,
    "compressed_memory_budget_tokens": 3000,
    "core_budget_tokens": 1000,
    "estimated_total_tokens": 2480,
    "estimated_current_memory_tokens": 1710,
    "estimated_compressed_memory_tokens": 520,
    "estimated_core_tokens": 250,
    "estimated_domain_state_tokens": 40,
    "estimated_associative_memory_tokens": 45,
    "dropped_session_recent": 0,
    "dropped_session_trace": 0,
    "dropped_archive_relevant": 0,
    "dropped_associative_memory": 0,
    "dropped_core_facts": 0,
    "budget_exceeded": false
  },
  "recall_debug": {
    "stage_used": "stage1",
    "candidate_count": 12,
    "filtered_count": 12,
    "vector_stage": {
      "status": "not_requested",
      "reason": "query_embedding was not provided",
      "candidate_limit": 40,
      "candidate_count": 12,
      "matched_archive_count": 0,
      "vector_weight": 0.5,
      "stage1_weight": 0.5
    },
    "candidates": [
      {
        "archive_id": "archive_01J00000000000000000000001",
        "stage1_rank": 1,
        "final_rank": 1,
        "selected": true,
        "stage1_score": 0.22,
        "final_score": 0.22,
        "components": {
          "weight": 0.9,
          "stored_freshness": 1.0,
          "freshness_decay": 0.7,
          "age_days": 92.0,
          "effective_freshness": 0.7,
          "recall_boost": 1.0,
          "theme_factor": 1.0,
          "tag_factor": 1.0,
          "text_factor": 0.7,
          "query_token_count": 5,
          "text_overlap_count": 0
        }
      }
    ]
  },
  "domain_state": {
    "active_topic": "travel_planning"
  },
  "notes": []
}
```

На v0.1 `core_facts` заповнюється стабільними фактами поточного `core_scope`; категорії є вільними normalized `snake_case`, а не закритим списком. Хости мають використовувати `CoreContextPackage` як єдину точку збору prompt-контексту, а не дублювати session/recent/archive/core логіку в кожному host-і.

`budget` завжди показує, скільки memory context приблизно займає після trimming.
Active session events зберігаються від найсвіжіших назад,
`archive_relevant` — за власним recall ranking, `core_facts` — за confidence.
Високопороговий `associative_memory` отримує перший доступ до
compressed-memory budget, бо це явна активація поточною темою; звичайні Archive
items використовують залишок і зберігають внутрішній rank. Звіт рахує dropped
items окремо, тому не називає associative detail переможцем іншої формули. Якщо
`budget_exceeded: true`, host має логувати це як warning і перевірити
`domain_state` або власний prompt overhead.

`archive_relevant` пріоритезує `compact_memory`, якщо archive entry має це поле. У такому випадку `gist` у recall item дорівнює compact thesis, а `narrative`, `facts` і `quotes` не повертаються в prompt-facing форму. Full archive із narrative, tracks і цитатами лишається у storage/debug entry.

`associative_memory` не є четвертим persisted memory layer. Це per-request
prompt projection із rebuildable vector index. Кожний item зберігає atomic
`memory_unit_id`, parent `archive_id`, час, similarity, застосований
`activation_threshold`, deep-recall score і пояснення активації. Для bundled
`sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` default
`VectorConfig.contextual_expansion_min_sim` дорівнює `0.47`: якщо жоден unit не
досягає порога, секція порожня, а `notes` містить `below_threshold` і сам поріг.
Canonical prompt view рендерить прийняті hits окремим
`<associative_memory>` блоком із явною позначкою, що це давні спогади,
активовані поточною темою.

Важливо: цей JSON-контракт є API/debug формою, а не обовʼязковим дослівним prompt payload. Для LLM має використовуватись компактне prompt-facing представлення: без `schema_version`, довгих `event_id` / `archive_id` / `core_fact_id`, службових `source`, зайвих timestamp/source полів і дубльованих технічних ключів, якщо вони не потрібні для конкретної відповіді або debug-команди. Канонічні storage-файли лишаються повними; prompt view має нести сенс, а не аудитну обвʼязку. На v0.1 Telegram host реалізує compact projection перед chat prompt через role transcript і compact memory theses; спільний engine-level `prompt_view` контракт лишається задачею v0.2.

---

## 7. CandidateBelief

CandidateBelief - кандидат на стабільний висновок у Core Store.

`candidate_belief.v1` є чинною плоскою compatibility-формою v0.5. Phase 2
графа не додає до неї полів: node/evidence/relation мають окремі versioned
schemas, описані в секції 7.3. Чинний reflection/review може далі читати й
писати CandidateBelief через graph projection; ширше/вужче reasoning і relation
proposals починаються лише у Phase 3 за планом
[Reflective Belief System](../planning/reflective-belief-system.md).

Файл: `memory/core/candidates/<candidate_id>.json`.

```json
{
  "schema_version": "candidate_belief.v1",
  "candidate_id": "candidate_01J00000000000000000000001",
  "source_session_id": "session_2026_08_01",
  "source_session_ids": ["session_2026_07_20", "session_2026_08_01"],
  "core_scope": "owner_scope",
  "created_at": "2026-05-17T17:18:00.000Z",
  "updated_at": "2026-05-17T17:18:00.000Z",
  "text": "Користувач живе в Берліні.",
  "category": "personal_facts",
  "status": "ready_for_review",
  "confidence": 0.82,
  "supporting_archive_ids": ["archive_01J00000000000000000000001"],
  "contradicting_archive_ids": [],
  "evidence_summary": "Підтримано прямим повідомленням користувача.",
  "promotion_checks": {
    "min_sources_met": false,
    "weight_threshold_met": true,
    "no_recent_contradiction": true,
    "manual_review_required": true
  },
  "review": null,
  "reinforcements": [
    {
      "reinforcement_id": "reinforcement_01J...",
      "created_at": "2026-06-01T12:00:00.000Z",
      "source_session_ids": ["session_2026_06_01"],
      "source_memory_unit_ids": ["mu_01J..."],
      "supporting_archive_ids": ["archive_01J..."],
      "evidence_summary": "Пізніший валідований досвід повторив той самий патерн.",
      "confidence": 0.88,
      "tags": ["recurring"]
    }
  ],
  "links": [
    {
      "kind": "source",
      "target": "archive:archive_01J00000000000000000000001"
    }
  ]
}
```

`status`:

- `draft`;
- `ready_for_review`;
- `approved`;
- `rejected`;
- `promoted`;
- `superseded`.

Ручний reflection реалізований у v0.2; автоматичний trigger/checkpoint lifecycle
починається у v0.5. Обидва шляхи створюють тільки reviewable candidates.

### 7.1 Autonomous reflection processing state (v0.5)

Autonomous reflection не додає persisted memory layer. Він додає durable
processing metadata у `memory/runs/reflection/<scope>.json`:

```json
{
  "schema_version": "reflection_state.v1",
  "reflection_scope": "owner_scope",
  "source_session_ids": ["session_2026_07_20", "session_2026_08_01"],
  "source_session_id": "session_...",
  "created_at": "2026-08-01T10:00:00Z",
  "updated_at": "2026-08-01T10:05:00Z",
  "last_completed_at": "2026-08-01T10:05:00Z",
  "last_task_id": "task_...",
  "last_trigger_reason": "accumulated_salience",
  "last_candidate_count": 1
}
```

`MemoryUnit` отримує три additive backward-compatible поля у
`memory_unit.v1`:

- `reflection_count` — скільки успішних reflection passes використали unit як
  новий trigger material;
- `last_reflected_at` — час останнього такого успішного проходу;
- `last_reflection_operation_id` — deterministic journal operation, яка
  застосувала останній checkpoint і захищає replay від другого increment.

Відсутні поля старого запису читаються як `0`, `null` і `null`. Failed task не
змінює їх. Успішний zero-candidate result змінює: осмислення відбулося, навіть
якщо стійкого висновку не знайдено.

`reflection_status(session_id, core_scope, opportunity)` повертає
`reflection_trigger_status.v1`: scope, opportunity, eligible unit count,
повний `source_session_ids` membership, накопичену вагу, thresholds, oldest
eligible time, last completion, optional active task id, `due` і
machine-readable `reason`.

`reason` має значення `no_eligible_memory`, `not_due`, `already_pending`,
`critical_memory`, `accumulated_salience` або `scheduled_interval`.

`bind_reflection_session(session_id, core_scope)` додає session до durable
membership без trigger evaluation, PendingTask або LLM-виклику. Це шлях для
host session lifecycle і контрольованого backfill історичних сесій.

`begin_reflection_if_due(...)` повертає той самий status і optional
`ReflectionPassStart`. Відсутнє або `null` поле `reflection` є нормальним
abstention result, а не помилкою. Перевірка і створення task виконуються під
одним per-scope lock. Метод також idempotently bind-ить поточну session, тому
простий host не зобовʼязаний робити окремий bind.

Автоматичний `reflection_analyze` task додає в `inputs`:

- `reflection_scope`;
- `source_session_ids` — тільки sessions, представлені фактично відібраними
  units; повний host-bound membership лишається у reflection state/status;
- `personality_session_count` — розмір повного membership без роздування
  reasoning input довгим списком;
- `trigger_reason`;
- `new_memory_unit_ids`;
- `related_memory_unit_ids`;
- bounded `memory_units`, де `evidence_role` дорівнює `trigger` або
  `related_history`;
- summaries тільки parent archives фактично включених units;
- bounded `existing_candidates` зі статусом `draft`/`ready_for_review` того ж
  `core_scope`.

Legacy candidate без `core_scope` може входити лише тоді, коли його singular
або plural source-session provenance належить explicit bound membership.
Candidate з іншим непорожнім scope не підмішується навіть за спільної session.

`trigger` означає новий validated unit, що входить у checkpoint після
успішного submit. `related_history` означає раніше осмислений unit, який ядро
додало як контекст через exact structural overlap. Він не checkpoint-иться
повторно. Selector не потребує vectors і використовує явні ваги: спільний
link `4`, archive theme `3`, topic/subtopic `2`, unit/archive tag `1`.
`relation_score` і `relation_reasons` пояснюють відбір, але не є доказом
стабільного висновку. Default `min_related_score=2`, тому один слабкий
спільний tag із вагою `1` сам по собі не підмішує давній спогад.

`ReflectionPassStart` окремо показує `new_memory_unit_count`,
`related_memory_unit_count` та `existing_candidate_count`. Початкові limits:
`ReflectionConfig.max_related_units=20`, `min_related_score=2` і
`max_existing_candidates=20`.

`CandidateBelief.source_session_ids` є additive provenance усіх sessions, з
яких походять прийняті `source_memory_unit_ids`. Legacy singular
`source_session_id` залишається triggering session.

### 7.2 Typed reflection outcome (v0.5 Phase C)

Нові `reflection_analyze` tasks мають prompt version `3` і
`expected_output_schema=reflection_actions_result.v1`. Результат містить
ідентичність source/scope та рівно один із двох варіантів: непорожній `actions`
або `no_insight`.

```json
{
  "schema_version": "reflection_actions_result.v1",
  "source_session_id": "session_2026_08_01",
  "core_scope": "owner_scope",
  "actions": [
    {
      "action": "reinforce_candidate",
      "candidate_id": "candidate_01J...",
      "confidence": 0.88,
      "evidence_summary": "Новий валідований unit повторює патерн.",
      "source_memory_unit_ids": ["mu_01J..."],
      "supporting_archive_ids": ["archive_01J..."],
      "tags": ["recurring"]
    }
  ]
}
```

Допустимі actions:

- `new_candidate { candidate }` — нова reviewable гіпотеза, якої немає у Core
  або `existing_candidates`;
- `reinforce_candidate` — додає нові task-visible evidence до того самого
  draft/ready candidate, записує `CandidateReinforcement`, обʼєднує
  unit/archive/session provenance і може лише підвищити confidence;
- `challenge_core_fact { candidate }` — вимагає task-visible
  `contradicted_core_fact_ids`, додає tag `core_challenge`, але не змінює Core;
  contested transition відбувається тільки через звичайний owner approval;
- `no_insight` — окремий обʼєкт з `reason` і непорожнім `explanation`.

`no_insight.reason` є одним із `insufficient_repetition`,
`already_represented`, `temporary_state`, `conflicting_evidence` або
`no_stable_pattern`. Він є успішним осмисленням і пересуває checkpoint.

Typed apply є fail-closed: candidate має бути видимим у task, reinforcement
мусить цитувати хоча б один новий для кандидата task unit, challenge мусить
цитувати видимий Core fact, а new/challenge не можуть дублювати видимий
candidate. Семантична помилка не checkpoint-ить trigger units. Saved tasks і
старі hosts можуть і далі повернути `reflection_result.v1`; цей legacy contract
читається сумісно, але нові tasks його не запитують.

**Integrity closure 2026-08-01.** Typed apply і owner review тепер є
journal-backed ідемпотентними операціями з frozen intent. До першого side
effect apply повторно читає cited MemoryUnit/archive/candidate/Core evidence;
змінений, forgotten або cross-scope матеріал зупиняє операцію без checkpoint.
Відсутній `core_scope` означає тільки unscoped namespace. Review бере scope з
candidate, відхиляє інший caller scope і приймає лише `draft` або
`ready_for_review`. Restart recovery не викликає LLM повторно й не дублює
candidate, reinforcement, unit checkpoint або Core transition.

Після typed apply ядро повертає `reflection_apply_result.v1`:
`ReflectionCandidatesResult.candidates` містить нові та challenge candidates,
`reinforced_candidates` — оновлені наявні records, а optional `no_insight` —
структуроване утримання від зміни довготривалого розуміння. Additive
`operation_id` та `result_digest` звʼязують відповідь із durable apply. Legacy
apply зберігає старий `reflection_result.v1` output для сумісності.

Тексти не входять у постійну trigger telemetry. Повний LLM input залишається у
звичайному auditable PendingTask, як і для ручного reflection.

### 7.3 Hypothesis graph storage (v0.6 Phase 2)

Phase 2 додає три окремі persisted contracts, не змінюючи
`candidate_belief.v1` і не додаючи graph до chat prompt:

- `memory/core/hypotheses/nodes/<hypothesis_id>.json` —
  `hypothesis_node.v1`;
- `memory/core/hypotheses/evidence/<evidence_link_id>.json` —
  `hypothesis_evidence_link.v1`;
- `memory/core/hypotheses/relations/<relation_id>.json` —
  `hypothesis_relation.v1`.

Скорочений node:

```json
{
  "schema_version": "hypothesis_node.v1",
  "hypothesis_id": "candidate_01J...",
  "legacy_candidate_id": "candidate_01J...",
  "source_session_ids": ["session_a", "session_b"],
  "core_scope": "owner_scope",
  "created_at": "2026-08-01T10:00:00Z",
  "updated_at": "2026-08-02T10:00:00Z",
  "statement": "Користувача стабільно цікавить колонізація Марса.",
  "category": "interests",
  "abstraction_level": null,
  "status": "ready_for_review",
  "support_score": 0.84,
  "contradiction_score": null,
  "validity": null,
  "evidence_summary": "Дві валідовані розмови підтримують вузький інтерес.",
  "promotion_checks": {
    "min_sources_met": true,
    "weight_threshold_met": true,
    "no_recent_contradiction": true,
    "manual_review_required": true
  },
  "supporting_archive_ids": ["archive_a", "archive_b"],
  "source_memory_unit_ids": ["mu_a", "mu_b"],
  "tags": ["mars", "colonization"],
  "reinforcements": []
}
```

`support_score` є пояснюваною support-оцінкою, а не probability of truth.
`contradiction_score` зберігається окремо. Для migrated flat candidate
`abstraction_level`, `contradiction_score` і `validity` дорівнюють `null`, бо
старий record не містить достатньої інформації й migration не має права її
вигадати. Node зберігає candidate lifecycle/review/provenance fields, щоб
compatibility projection у `CandidateBelief` була lossless.

Evidence link:

```json
{
  "schema_version": "hypothesis_evidence_link.v1",
  "evidence_link_id": "hypothesis_evidence_4a1f...",
  "hypothesis_id": "candidate_01J...",
  "memory_unit_id": "mu_a",
  "archive_id": "archive_a",
  "source_session_id": "session_a",
  "polarity": "supports",
  "directness": "legacy_unspecified",
  "generalization_distance": null,
  "resolution": "resolved",
  "created_at": "2026-08-01T10:00:00Z",
  "updated_at": "2026-08-02T10:00:00Z",
  "origins": [
    {
      "kind": "candidate",
      "origin_id": "candidate_01J...",
      "created_at": "2026-08-01T10:00:00Z",
      "summary": "Прямий validated memory unit.",
      "strength": 0.84,
      "tags": ["mars"]
    }
  ]
}
```

ID evidence link детерміновано залежить від `hypothesis_id + memory_unit_id +
polarity`. Тому кілька legacy reinforcement records, що цитують той самий
unit, стають кількома `origins` одного ребра, а не кількома незалежними
доказами. Якщо unit ID збережено в candidate, але сам unit неможливо прочитати,
link має `resolution: unresolved` і не отримує вигаданих archive/session IDs.
Archive-only contradiction без exact MemoryUnit лишається у
`HypothesisNode.contradicting_archive_ids`; migration не створює для нього
ребро навмання.

Relation має `source_hypothesis_id`, `target_hypothesis_id`, `kind`
(`generalizes`, `specializes`, `possible_equivalent`, `contradicts`), `status`
(`proposed`, `confirmed`, `rejected`, `superseded`), optional `score`,
`explanation` і `proposed_by`. Self-relations та cross-scope relations
відхиляються. Phase 2 migration завжди повертає `relations_created: 0`; лише
наступна typed reflection phase може запропонувати semantic relation.

Адміністративний `migrate_hypothesis_graph()` повертає
`hypothesis_graph_migration_result.v1` із created/updated/unchanged counts,
`unresolved_memory_unit_ids`, warnings і нульовим relation count. Frozen
`hypothesis_graph_migration_operation.v1` зберігається як
`JournalOperationType::Migration`; restart повторює desired node/evidence
effects і відмовляється перезаписувати graph record, що змінився після
підготовки operation.
Migration сканує тільки persisted `core/candidates/*.json`, а не graph nodes,
спроєктовані як candidates для compatibility; тому повторний запуск не
перетворить майбутню graph-native hypothesis назад на legacy record.

`unresolved_memory_unit_ids` і `warnings` є обовʼязковими JSON arrays навіть
для порожньої міграції. Публічний контракт не змінює shape залежно від
кількості кандидатів: порожній результат повертає `[]`, а не відсутній key.

`export_hypothesis_graph(request)` приймає:

```json
{
  "schema_version": "hypothesis_graph_export_request.v1",
  "core_scope": "owner_scope"
}
```

і повертає `hypothesis_graph_export.v1` з `nodes`, `evidence_links`,
`relations` та text-safe storage warnings. Scope порівнюється точно;
`core_scope: null` означає лише isolated unscoped namespace, ніколи wildcard.
Python і Godot adapters лише передають ці JSON contracts у Rust core.
Усі чотири collection keys (`nodes`, `evidence_links`, `relations`, `warnings`)
присутні завжди; exact-scope export без записів повертає порожні arrays.

Під час compatibility period кожний `write_candidate_belief` ідемпотентно
оновлює node/evidence projection, а candidate read віддає graph projection,
якщо node існує. Оновлення flat candidate не стирає graph-only
`abstraction_level`, `contradiction_score`, `validity`, evidence directness або
generalization distance. Це storage compatibility, не hierarchical reasoning.

---

## 8. Recall

### 8.1 RecallQuery

Вхід у `engine.recall(query)`.

```json
{
  "schema_version": "recall_query.v1",
  "query_id": "recall_query_01J00000000000000000000001",
  "created_at": "2026-05-17T17:30:00.000Z",
  "session_id": "2026-05-17_005",
  "context": {
    "active_theme": "personal_background",
    "recent_text": "Користувач питає, що ми пам'ятаємо про його переїзд."
  },
  "query_text": "Що користувач казав про місце проживання?",
  "query_embedding": {
    "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    "query_vec": [0.01, -0.02, 0.03]
  },
  "filters": {
    "time_range": null,
    "tags": ["personal_fact", "location"],
    "theme": null,
    "min_weight": 0.0,
    "min_freshness": 0.0,
    "source_layers": ["archive", "core"]
  },
  "limit": 5,
  "include_core": true,
  "explain": true
}
```

Обов'язкові поля:

- `schema_version`;
- `context`;
- `filters`;
- `limit`;
- `include_core`;
- `explain`.

Опціональні:

- `query_id`;
- `created_at`;
- `session_id`;
- `query_text`.
- `query_embedding` (`model_id` + `query_vec`).

`source_layers`:

- `session`;
- `archive`;
- `core`.

На v0.1 основний recall працює по `archive` і `core`. Пошук у live-session може бути доданий як проста перевірка поточної сесії.

`session_id` у `RecallQuery` є isolation boundary для archive recall. Якщо поле задане, ядро повертає тільки archive entries, де `source_session_id` дорівнює цьому `session_id`. Це default для `core_context_package` і звичайного chat prompt. Якщо хосту потрібен явний глобальний archive search для debug/admin-команди, він має передати `session_id: null` або не передавати поле.

Якщо `query_embedding` передано і vector scope для `session_id` має стан `ready`, ядро застосовує Stage 2 до перших `VectorConfig.stage2_candidate_limit` кандидатів Stage 1 (default `40`): `final = stage1_score * (1 - w_vec) + max_memory_unit_cosine * w_vec`, де `w_vec = VectorConfig.stage2_vector_weight` (default `0.5`). Max cosine береться тільки по активних vector-eligible `MemoryUnit` батьківського archive entry; якщо такого unit-вектора немає, semantic term дорівнює `0`, а debug лишає `semantic_similarity` порожнім. На рівному cosine winning unit обирається детерміновано за найменшим `memory_unit_id`. Для matched archive Stage 2 зберігає winning unit як semantic provenance: `RecallItem.matched_memory_unit_id` і debug-кандидат містять його ID, а `RecallItem.gist`/`compact_memory` проєктують саме тезу цього unit замість усього змішаного денного `compact_memory`. Archive ID, archive-level Stage 1 score, час, теги й recall statistics лишаються батьківською структурною межею. Final score обмежується діапазоном `0..1`. Без query-вектора, для disabled/building/corrupt scope або поза candidate window порядок і projection лишаються Stage 1. Незалежний `recall_deep` продовжує шукати по всьому готовому unit-індексу й страхує випадок, коли правильний archive не ввійшов у Stage 2 window.

`limit`:

- ціле число `>= 0`;
- значення `0` явно означає "використати default ядра системи" (на v0.1 - `5`, налаштовується через `RecallStage1Config.default_limit`);
- будь-яке значення `> 0` обмежує `RecallResult.items` саме цим числом.

### 8.2 RecallResult

Вихід з `engine.recall(query)`.

```json
{
  "schema_version": "recall_result.v1",
  "query_id": "recall_query_01J00000000000000000000001",
  "created_at": "2026-05-17T17:30:00.050Z",
  "stage_used": "stage2_embeddings",
  "items": [
    {
      "source_layer": "archive",
      "id": "archive_01J00000000000000000000001",
      "matched_memory_unit_id": "mu_01J00000000000000000000001",
      "gist": "Берлін -> користувач переїхав туди приблизно у квітні 2026 року.",
      "compact_memory": "Берлін -> користувач переїхав туди приблизно у квітні 2026 року.",
      "source_session_id": "2026-05-17_005",
      "time_range": {
        "start": "2026-05-17T16:30:00.000Z",
        "end": "2026-05-17T17:00:00.000Z"
      },
      "tags": ["personal_fact", "location"],
      "theme": "personal_background",
      "weight": 0.82,
      "freshness": 1.0,
      "relevance_score": 0.79,
      "relevance_explanation": "Збіг тегів personal_fact/location, висока вага, свіжа актуальність."
    }
  ],
  "debug": {
    "stage_used": "stage2_embeddings",
    "candidate_count": 12,
    "filtered_count": 3,
    "vector_stage": {
      "status": "applied",
      "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
      "candidate_limit": 40,
      "candidate_count": 3,
      "matched_archive_count": 3,
      "vector_weight": 0.5,
      "stage1_weight": 0.5
    },
    "candidates": []
  }
}
```

`stage_used`:

- `stage1`, коли query embedding відсутній або vector scope не готовий;
- `stage2_embeddings`, коли готовий unit-level vector scope пересортував Stage 1 candidate window;
- `stage3_llm` у майбутньому.

`relevance_score` є обовʼязковим у кожному `RecallItem`; адаптери не мають його викидати. `matched_memory_unit_id` є optional semantic provenance: присутній для unit-focused Stage 2 або contextual deep-recall item і відсутній у старому Stage 1 output. Це additive backward-compatible поле `recall_result.v1`; старі payload-и без нього валідні. `debug` присутній лише при `explain: true`. Його `candidates` містять структуровані компоненти Stage 1 та, якщо Stage 2 запускався, winning `matched_memory_unit_id`, semantic similarity і обидва ранги. `vector_stage.status` розрізняє `not_requested`, `disabled`, `building`, `corrupt` та `applied` без вгадування за порожнім `notes`.

---

## 9. PendingTask

PendingTask - спосіб, яким ядро системи просить хост виконати LLM-задачу.

Файл: `memory/tasks/<task_id>.json`.

### 9.1 PendingTask JSON

```json
{
  "schema_version": "pending_task.v1",
  "task_id": "task_01J00000000000000000000001",
  "task_type": "sleep_compression",
  "state": "pending",
  "created_at": "2026-05-17T17:05:00.000Z",
  "updated_at": "2026-05-17T17:05:00.000Z",
  "prompt_id": "sleep_compression",
  "prompt_version": 1,
  "role_hint": "balanced",
  "expected_output_schema": "sleep_compression_result.v1",
  "inputs": {
    "session_id": "2026-05-17_005",
    "preliminary_archive_id": "archive_01J00000000000000000000001",
    "events": [
      {
        "event_id": "event_01J00000000000000000000001",
        "type": "user_message",
        "timestamp": "2026-05-17T16:32:11.420Z",
        "payload": {
          "text": "Я переїхав у Берлін минулого місяця"
        },
        "tags": ["personal_fact", "location"],
        "initial_weight": 0.75
      }
    ],
    "hints": {
      "target_style": "compact_human_readable_memory"
    }
  },
  "attempts": [],
  "last_error": null,
  "outcome": null
}
```

`outcome` є optional terminal core-owned result. Старі task-файли без нього
лишаються валідними. Для completed `reflection_analyze` це
`reflection_outcome.v1`; він навмисно text-free:

```json
{
  "schema_version": "reflection_outcome.v1",
  "operation_id": "reflection_apply_task_01J...",
  "result_digest": "64-hex-sha256",
  "completed_at": "2026-08-01T12:00:00.000Z",
  "new_candidate_ids": ["candidate_01J..."],
  "reinforced_candidate_ids": [],
  "no_insight_reason": null
}
```

`task_type` v0.1:

- `sleep_compression`;
- `compact_memory_pass`;
- `memory_unit_pass`;
- `memory_fidelity_pass`;
- `score_event` опціонально.

Зарезервовані на v0.2+:

- `reflection_analyze`;
- `recall_rerank`;
- `compute_embedding`;
- `fact_check`;
- `tag_proposal`.

`compute_embedding` is active starting with vector storage Phase A. It is not a
prompt rendered from `prompts/`: `prompt_id = "embed_batch"` is an operational
label, `role_hint = "fast"` is informational for hosts, and the host must execute
the task with a local embedding model. The core never computes embeddings and
never calls an embedding provider.

`compute_embedding.inputs` shape:

```json
{
  "kind": "embed_batch",
  "scope": "telegram_311422683",
  "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
  "dim": 384,
  "items": [
    { "memory_unit_id": "mu_...", "text": "MemoryUnit thesis text" }
  ]
}
```

Host result submitted through `resume_compute_embedding(task_id, result)`:

```json
{
  "schema_version": "embed_batch_result.v1",
  "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
  "dim": 384,
  "results": [
    { "memory_unit_id": "mu_...", "vector": [0.01, -0.02] }
  ]
}
```

The core rejects model/dim mismatches and normalizes accepted vectors before
storing them.

`state`:

- `pending`;
- `submitted`;
- `completed`;
- `failed`;
- `cancelled`.

### 9.2 TaskAttempt

Елемент масиву `attempts`.

```json
{
  "attempt_id": "attempt_01J00000000000000000000001",
  "started_at": "2026-05-17T17:06:00.000Z",
  "finished_at": "2026-05-17T17:06:30.000Z",
  "provider": "google",
  "model": "put-balanced-model-name-here",
  "status": "completed",
  "error": null
}
```

Це metadata для аудиту. Ядро не вибирає provider/model, але може зберегти те, що хост повідомив після виконання.

### 9.3 EvidencePack і FidelityReview

`EvidencePack` — малий пакет доказів для validator-а. Його збирає ядро програмно: source events із `MemoryUnit.source_event_ids`, конфігуровані сусідні події для локального контексту, thesis/evidence самого unit і token budget. Хост не збирає ці дані сам; він тільки рендерить `prompt_id = memory_fidelity_pass`, виконує LLM-запит і повертає відповідь.

Після `finish_sleep_run` ядро може повернути `SleepOutcome.fidelity_requests`: готові `LlmRequest` для memory units, які варто перевірити автоматично. Правила маршрутизації залишаються в ядрі: high-weight units, конфігуровані high-risk tags і Core-path units через personal signals. Хост не вирішує, які units перевіряти; він тільки виконує повернуті запити і submit-ить відповіді через `submit_memory_fidelity_response`.

```json
{
  "schema_version": "evidence_pack.v1",
  "evidence_pack_id": "evidence_pack_mu_01J00000000000000000000001",
  "created_at": "2026-05-31T10:00:00.000Z",
  "memory_unit_id": "mu_01J00000000000000000000001",
  "archive_id": "archive_01J00000000000000000000001",
  "source_session_id": "telegram_311422683",
  "target_thesis": "Користувач має кішку Іржу.",
  "unit_evidence": "Користувач прямо розповів про кішку та її ім'я.",
  "events": [
    {
      "event_id": "event_01J00000000000000000000001",
      "timestamp": "2026-05-31T09:58:00.000Z",
      "type": "user_message",
      "source": "telegram_user_311422683",
      "role": "source",
      "text": "В мене ще є кішечка. Її звати Іржа.",
      "tags": ["personal_fact_signal"]
    }
  ],
  "max_estimated_tokens": 1500,
  "estimated_tokens": 96,
  "truncated": false
}
```

`FidelityReview` — verdict validator-а щодо одного `MemoryUnit`. Він не пише в Core і не створює нову істину; він тільки позначає, чи стиснений спогад відповідає evidence pack. Ядро зберігає review в memory unit і оновлює status unit-а: `valid` лишає unit активним, `unsupported`/`distorted` відхиляють, `too_broad`/`missing_key_detail`/`needs_revision` переводять у revision path.

```json
{
  "schema_version": "fidelity_review.v1",
  "memory_unit_id": "mu_01J00000000000000000000001",
  "archive_id": "archive_01J00000000000000000000001",
  "status": "valid",
  "confidence": 0.92,
  "explanation": "Теза прямо підтримана source event і не додає нових деталей.",
  "revised_thesis": null,
  "missing_detail": null
}
```

Допустимі `status` для v0.2 Phase B:

- `valid`;
- `too_broad`;
- `unsupported`;
- `distorted`;
- `missing_key_detail`;
- `needs_revision`.

---

## 10. CompactMemoryPass Result (compatibility)

Результат старого прямого шляху, який хост повертає у `engine.resume_compact_memory_pass(task_id, text)` для `compact_memory_pass`. Основний sleep-run використовує `memory_unit_pass` і рендерить `compact_memory` у ядрі; цей контракт збережений для сумісності.

Це **plain text**, не JSON:

```text
Обговорили МіГ-15 і F-86 -> користувач цікавиться військовою авіацією Корейської війни.
Розмова про кішку Іржу -> у користувача є кішка, і ця згадка має теплий особистий окрас.
Повернулись до Маріанської западини й Європи Юпітера -> користувачу цікаві крайні середовища, де може існувати життя.
```

Правила:

- кількість тез не фіксована: одна coherent тема може повернути одну тезу, кілька різних епізодів — кілька тез;
- агент не має заповнювати штучну квоту і не має дробити одну тему заради кількості;
- одна теза — одне коротке речення або дуже короткий абзац;
- форма змісту: "подія -> висновок";
- без markdown table, JSON, службових ID і пояснень формату;
- timestamps з подій використовуються для порядку й часових меж; raw ISO timestamps не треба виводити без потреби;
- це prompt-facing стиснута памʼять, а не audit/debug archive.

---

## 11. SleepCompressionResult

Результат, який хост повертає у `engine.resume(task_id, result)` для `sleep_compression`.

```json
{
  "schema_version": "sleep_compression_result.v1",
  "archive_id": "archive_01J00000000000000000000001",
  "gist": "Користувач повідомив, що нещодавно переїхав у Берлін.",
  "narrative": "У розмові користувач поділився важливою зміною в особистому контексті: він переїхав у Берлін минулого місяця. Це може впливати на майбутні розмови про побут, роботу, подорожі й локальний контекст.",
  "compact_memory": "Переїзд у Берлін -> користувач повідомив стабільний особистий контекст, який варто пам'ятати.",
  "facts": [
    {
      "text": "Користувач живе в Берліні з приблизно квітня 2026 року.",
      "confidence": 0.8,
      "source_event_ids": ["event_01J00000000000000000000001"]
    }
  ],
  "quotes": [
    {
      "text": "Я переїхав у Берлін минулого місяця",
      "source_event_id": "event_01J00000000000000000000001"
    }
  ],
  "tags": ["personal_fact", "location"],
  "theme": "personal_background",
  "weight": 0.82,
  "links": [],
  "emotional_markers": [],
  "topic_thread": [],
  "personal_signals": [],
  "relational_tone": null
}
```

Правила валідації:

- `archive_id` має збігатися з preliminary archive entry, якщо task був створений для оновлення існуючого запису;
- `gist` не має бути порожнім;
- `gist` не має бути механічним списком тем; він має коротко передавати найважливіший людський епізод або епізоди;
- `narrative` не має бути порожнім і має передавати не лише "про що говорили", а й "що сталося / що це означало / який емоційний тон був прямо підтриманий подіями";
- `facts` може бути порожнім, але якщо факт є, він має мати `text` і `confidence`;
- `quotes` можуть бути порожніми;
- `compact_memory` може бути відсутнім у preliminary/incomplete або Complete
  legacy entry. Основний sleep-run будує його з validated `memory_unit_pass`;
  окремий `compact_memory_pass` лишається compatibility API. Якщо поле
  присутнє, воно не може бути порожнім;
- `weight` має бути `0.0..1.0`;
- `tags` мають бути короткими машинними рядками.
- `emotional_markers`, `topic_thread`, `personal_signals` можуть бути порожніми масивами;
- `relational_tone` може бути `null`.

Тіло промпта для отримання такого результату не створюється в цьому документі. Воно з'явиться у `prompts/sleep_compression.md` тільки разом із реальною sleep-stage-2 реалізацією.

---

## 12. Vector index

Vector storage is opt-in and derived from `MemoryUnit` theses. The source of
truth remains archives and units; `memory/archive/vectors/<scope>/` may be
deleted and rebuilt without losing memory.

Scope directory:

```text
memory/archive/vectors/<scope>/
  manifest.json
  vectors.f32
  rows.jsonl
  tombstones.jsonl
```

`manifest.json`:

```json
{
  "schema_version": "vector_index.v1",
  "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
  "dim": 384,
  "metric": "cosine",
  "normalized": true,
  "rows": 1,
  "state": "ready",
  "built_at": "2026-07-03T12:00:00Z",
  "updated_at": "2026-07-03T12:00:00Z",
  "backfill_cursor": null
}
```

`rows.jsonl`:

```json
{"row":0,"memory_unit_id":"mu_...","archive_id":"archive_...","created_at":"2026-07-03T12:00:00Z","thesis_hash":"sha256:..."}
```

`vectors.f32` is little-endian row-major `f32`, one L2-normalized vector per
row. `tombstones.jsonl` contains `{"memory_unit_id":"mu_..."}` records for
forgotten/rejected units until compaction.

Eligibility: `MemoryUnit.status == active_archive`, fidelity status is not
`distorted` / `unsupported` / `needs_revision`, scope vector catalog is enabled,
global `Manifest.features.embeddings_enabled == true`, and the session is not
multi-speaker. Multi-speaker units are skipped until attributed memory units are
implemented.

### 12.1 Deep recall

`recall_deep` is active starting with vector storage Phase B. The core accepts a
ready query vector from the host; it does not tokenize text, compute embeddings,
load an embedding model, or call an embedding provider.

Request:

```json
{
  "scope": "telegram_311422683",
  "query_vec": [0.01, -0.02],
  "model_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
  "top_k": 5,
  "min_sim": 0.30,
  "now": "2026-07-03T12:00:00Z"
}
```

Response:

```json
{
  "schema_version": "deep_recall.v1",
  "found": true,
  "reason": null,
  "hits": [
    {
      "memory_unit_id": "mu_...",
      "archive_id": "archive_...",
      "thesis": "User has a cat named Irzha.",
      "created_at": "2026-07-03T12:00:00Z",
      "sim": 0.84,
      "score": 0.99
    }
  ]
}
```

`reason` when `found == false`:

- `disabled` - global embeddings are disabled or the scope catalog is absent;
- `building` - the scope exists but backfill is not ready;
- `corrupt` - the derived vector catalog needs rebuild;
- `below_threshold` - no row passed raw cosine `min_sim`.

Model and dimension mismatches are hard errors. Search is scope-isolated and
never crosses `memory/archive/vectors/<scope>/`. Returned hits update existing
buffered recall stats for parent archives; they do not write synchronously to
archive files.

Legacy `ArchiveEntry.embedding_model_id` and `ArchiveEntry.embedding` are
deprecated reserved fields. Vector storage does not use archive-level
embeddings.

## 13. Manifest

Файл: `memory/manifest.json`.

```json
{
  "schema_version": "manifest.v1",
  "engine_version": "0.1.0",
  "storage_id": "default",
  "created_at": "2026-05-17T17:00:00.000Z",
  "updated_at": "2026-05-17T17:30:00.000Z",
  "schema_versions": {
    "event": "event.v1",
    "session": "session.v1",
    "archive_entry": "archive_entry.v1",
    "core_store": "core_store.v1",
    "core_fact": "core_fact.v1",
    "candidate_belief": "candidate_belief.v1",
    "hypothesis_node": "hypothesis_node.v1",
    "hypothesis_evidence_link": "hypothesis_evidence_link.v1",
    "hypothesis_relation": "hypothesis_relation.v1",
    "pending_task": "pending_task.v1",
    "reflection_state": "reflection_state.v1",
    "evidence_pack": "evidence_pack.v1",
    "fidelity_review": "fidelity_review.v1",
    "vector_index": "vector_index.v1",
    "embed_batch_result": "embed_batch_result.v1",
    "deep_recall": "deep_recall.v1",
    "journal_operation": "journal_operation.v1"
  },
  "active_embedding_model_id": null,
  "last_migration_at": null,
  "features": {
    "recall_stage": "stage1",
    "embeddings_enabled": false,
    "llm_recall_rerank_enabled": false,
    "reflection_enabled": false
  }
}
```

Правила:

- manifest читається при старті ядра системи;
- якщо версія схеми у файлах нижча за підтримувану - запускається міграція;
- якщо версія схеми вища за підтримувану - ядро відмовляється стартувати;
- `active_embedding_model_id` на v0.1 має бути `null`.

---

## 14. JournalOperation

JournalOperation описує мульти-файлову операцію, яку потрібно завершити або
видимо передати на ручний розбір після обриву. Runtime sleep і далі
відновлюється через `SleepRun`, але v0.5 reflection apply та candidate review є
першими активними journal-backed runtime flows.

Файл: `memory/journal/<op_id>.json`.

```json
{
  "schema_version": "journal_operation.v1",
  "op_id": "journal_01J00000000000000000000001",
  "op_type": "sleep",
  "state": "started",
  "created_at": "2026-05-17T17:05:00.000Z",
  "updated_at": "2026-05-17T17:05:00.000Z",
  "target_files": [
    "memory/archive/2026/05/archive_01J00000000000000000000001.json",
    "memory/sessions/2026-05-17_005/session.json"
  ],
  "intent": {
    "session_id": "2026-05-17_005",
    "archive_id": "archive_01J00000000000000000000001"
  },
  "recovery_policy": "retry_or_manual_review",
  "completed_at": null,
  "error": null
}
```

`op_type`:

- `sleep`;
- `migration`;
- `core_promotion`;
- `reflection_apply`;
- `candidate_review`.

`reflection_apply` має deterministic ID `reflection_apply_<task_id>`. Його
`intent` версії `reflection_operation.v1` містить digest початкового typed
result, frozen prepared actions, text-bearing domain outcome та progress
markers. Journal є приватним auditable storage, тому intent може містити повну
гіпотезу; постійна telemetry і `PendingTask.outcome` лишаються text-free.

`candidate_review` freezing-ує exact desired candidate і Core-category
effects, а також previous `updated_at` для optimistic conflict detection.
Операція не перечитує вільний review input після restart.

Phase 2 `migration` використовує intent
`hypothesis_graph_migration_operation.v1`: frozen desired nodes/evidence,
previous graph records для optimistic conflict detection, per-candidate
progress, frozen result і manifest marker. Recovery повторює кожний desired
effect навіть якщо progress уже був записаний, але не перезаписує graph record,
який після підготовки змінився на третє значення.

Public `recover_pending_memory_operations()` продовжує всі підтримувані
`started` memory operations. Окремо перший звичайний API-виклик на новому
engine instance виконує цей recovery один раз до читання або зміни memory;
тому host не мусить знати, який саме метод має бути першим після restart.

`state`:

- `started`;
- `completed`;
- `failed`;
- `needs_manual_review`.

`recovery_policy`:

- `retry`;
- `rollback`;
- `manual_review`;
- `retry_or_manual_review`.

---

## 15. Файли і джерела правди

Цей розділ є нормативним для реалізованого `FileStorage`. Реалізований additive
Storage v2 server contract має окрему operational truth boundary у §15.1, але
ще не є production PostgreSQL backend і зобовʼязаний
round-trip-ити ті самі versioned domain records через portable export/import.

Мінімальна файлова структура v0.1:

```text
memory/
  manifest.json
  sessions/
    <session_id>/
      events.jsonl
      session.json
      session.md
      archived/
        events-<NNN>.jsonl
  archive/
    <YYYY>/<MM>/
      <archive_id>.json
  core/
    store/
      <category>.json
    candidates/
      <candidate_id>.json
  tasks/
    <task_id>.json
  journal/
    <op_id>.json
```

Джерела правди:

- `events.jsonl` + `archived/events-<NNN>.jsonl` - правда для raw-подій сесії;
- `session.json` `archived_event_ids` - cache покриття Complete-архівами, перебудовується з Archive entries і не є окремим джерелом правди;
- `<archive_id>.json` - правда для архівного спогаду;
- `<category>.json` - правда для Core Store;
- `<candidate_id>.json` - правда для candidate belief;
- `<task_id>.json` - правда для PendingTask;
- `<op_id>.json` у `journal/` - правда для frozen intent і progress
  незавершеної multi-file operation; це processing truth, не memory layer;
- `manifest.json` - правда для версій сховища;
- Markdown-файли - людський вигляд, не машинна правда.

### 15.1 Server identity and ordered-turn envelope

Централізований runtime не може просто додати `user_id` до кількох запитів.
`VaultId` є opaque server-owned identity boundary. Він виводиться з
автентифікованого internal account membership і не приймається від Godot як
довірене значення. Session/core/reflection scopes існують усередині vault і не
замінюють його. Відсутній identity у server mode є помилкою, не `local_user`.

S1 reference contracts реалізовані в Rust. `authenticated_principal.v1` несе
вже перевірений internal account, тип principal і authentication ID;
`account_vault_membership.v1` окремо несе active/suspended/revoked membership
та revision. Тільки їхня точна active-пара утворює internal
`vault_context.v1`:

```json
{
  "schema_version": "vault_context.v1",
  "account_id": "account_01J...",
  "vault_id": "vault_01J...",
  "membership_id": "membership_01J...",
  "membership_revision": 1,
  "authentication_id": "auth_01J...",
  "principal_kind": "guest",
  "authorized_at": "2026-08-01T12:00:00Z"
}
```

Це request-scoped internal capability, а не public request schema або
довгоживучий security token. Десеріалізація JSON сама по собі не є
авторизацією: game backend створює контекст тільки після перевірки актуальних
credential і membership. Verified Play Games identity, яка вже належить іншому
account, повертає `manual_review_required`; автоматичного merge памʼяті немає.

S1 також реалізує `turn_request.v1`, `turn_operation.v1` і
`vault_turn_ledger.v1`. Перший request отримує server-assigned sequence; точний
повтор повертає наявну operation незалежно від її стану. Повтор того самого
`client_request_id` з іншим session або digest є idempotency conflict. Optional
`expected_turn_sequence` перевіряється тільки під час першого admission.

```json
{
  "schema_version": "turn_operation.v1",
  "operation_id": "turn_<sha256(vault_id, client_request_id)>",
  "vault_id": "vault_01J...",
  "account_id": "account_01J...",
  "membership_id": "membership_01J...",
  "membership_revision": 1,
  "client_request_id": "request_01J...",
  "session_id": "session_01J...",
  "request_digest": "sha256:<64 hex>",
  "turn_sequence": 1,
  "status": "accepted | ready_to_commit | committed",
  "terminal_result": {
    "outcome": "succeeded",
    "result_reference": "response:01J...",
    "result_digest": "sha256:<64 hex>"
  },
  "created_at": "2026-08-01T12:00:00Z",
  "updated_at": "2026-08-01T12:00:02Z"
}
```

Failed terminal result має `outcome=failed` і `error_code` замість success
reference/digest. `ready_to_commit` може чекати попередню sequence, але не може
стати `committed` раніше за неї. Terminal duplicate з тим самим результатом є
ідемпотентним; інший результат для тієї самої operation відхиляється.

`LocalVaultHandle` є лише file-backed reference: він створює
`vaults/<vault_id>/memory` і окремий atomic turn ledger, не має default vault і
дозволяє перевірити restart/isolation до PostgreSQL. Він успадковує правило
одного writer process від `FileStorage` і не є production multi-replica
гарантією.

### 15.2 Transactional server-storage envelope (S2 foundation)

Кожен майбутній server-owned record отримує однаковий envelope:

```text
vault_id
record_id
schema_version
revision
created_at
updated_at
```

Реалізований additive Storage v2 contract уточнює envelope:

```text
VaultRecord = {
  schema_version: "vault_record.v1",
  vault_id,
  key: { collection, record_id },
  payload_schema_version,
  revision >= 1,
  created_at,
  updated_at,
  payload
}
```

`WorkUnitIntent` (`storage_work_unit_intent.v1`) фіксує `operation_id`,
`operation_kind`, `request_digest` і `started_at`. Кожен put/delete вимагає
`ExpectedRevision::Absent` або `Exact(n)`, а commit повторно звіряє всі point
reads. Успіх створює `WorkUnitReceipt`
(`storage_work_unit_receipt.v1`) з bound vault/account/membership provenance,
result reference/digest, mutation count, commit sequence і часом. Повтор того
самого operation/input повертає receipt; повтор ID з іншим kind/digest
відхиляється.

Операція комітиться повністю або не комітиться; retryable `Conflict` повторює
всю детерміновану функцію від transaction begin. Durable workflow лишається
двофазним: task intent commit -> зовнішній LLM/embed call ->
submit/revalidate commit. Provider work під transaction заборонене. Повний
flow inventory і migration order зафіксовані у
[Storage v2 Work-Unit Inventory](../planning/storage-v2-work-unit-inventory.md).

Перші versioned domain requests поверх envelope:

- `transactional_ingest_request.v1` / `transactional_ingest_result.v1` — event
  і Session metadata;
- `transactional_core_upsert_request.v1` /
  `transactional_core_upsert_result.v1` — одна Core category та durable result;
- `transactional_core_patch_request.v1` /
  `transactional_core_patch_result.v1` — point patch, де category є явною
  частиною frozen input;
- `transactional_sleep_start_request.v1` /
  `transactional_sleep_start_result.v1` — атомарний старт одного sleep workflow
  для Session. Його immutable replay target має внутрішню persisted schema
  `sleep_start_operation_result.v1`.

Core operation result зберігається атомарно окремим versioned record і
перевіряється через receipt digest. Це дає exact replay навіть якщо category
після першого commit уже змінили інші operations. Transactional Core не є
другим каналом істини: він застосовує ту саму upsert/patch policy, що й
сумісний FileStorage path. Новий Core fact ID детермінований усередині vault;
scope лишається exact isolation boundary.

`recall_count` і `last_recalled_at` у server form є correctness state, а не
lossy telemetry: вони впливають на Stage 1 rank і forgetting protection.
Server recall не враховує незлиті replica-local deltas.
`transactional_recall_request.v1` freeze-ить `RecallQuery` разом із
`created_at`; `transactional_recall_result.v1` повертає exact committed
`recall_result.v1` та receipt. Stage 1 ranking, increment вибраних archives і
result record комітяться разом. Collection snapshot відстежує membership і
revisions, тому phantom insert/delete/update спричиняє retry. Replay повертає
той самий result і не збільшує counters вдруге.

Поточний Storage v2 recall contract є Stage 1 reference. Explicit
`query_embedding` відхиляється до перенесення vector records у S2.D. In-memory
`scan_collection` доводить committed-view semantics; production S3 adapter має
реалізувати indexed vault query, а не переносити full scan у hot path.

У server domain collection `session_events` є append-only. Transactional ingest
створює event тільки через `ExpectedRevision::Absent`; валідний StoredEvent
залишається на revision `1` і не оновлюється або видаляється звичайним domain
flow. Session metadata змінюється окремо. Retention/migration, якщо зʼявиться,
має бути явною audited операцією, а не прихованим update події. Sleep scan
fail-closed на event record, який порушує цю інваріанту.

`transactional_sleep_start_request.v1` freeze-ить `session_id`, `created_at` і
повний `SleepStage1Config` у request digest. Рекомендований
`MemoryEngine::new_transactional_sleep_start_request` копіює policy з
`EngineOptions`. Низькорівневий request type публічний для серіалізації й
trusted Runtime integration, тому game API не має напряму експонувати його
policy fields клієнту. Core валідує форму й digest, але навмисно не порівнює
frozen policy з поточними options: replay після конфігураційної зміни повинен
лишитися історично точним. FileStorage і Storage v2 викликають одну shared
Stage 1 policy для compactable window, selection, preliminary Archive та task
projection — server path не має другої когнітивної реалізації.

Один успішний `sleep_start` комітить рівно шість mutations в одному vault-bound
work unit:

1. Session переходить у `sleep_pending` і отримує `active_sleep_task_id` та
   `sleep_pending_event_ids`;
2. створюється preliminary Archive;
3. створюється `sleep_compression` PendingTask;
4. створюється `memory_unit_pass` PendingTask;
5. створюється початковий durable `SleepRun`;
6. створюється immutable `sleep_start_operation_result.v1` snapshot.

Archive/task/run/request IDs детерміновані з bound `vault_id`, operation ID,
frozen request digest і внутрішньої label. Receipt посилається на immutable
operation result, а не на mutable `SleepRun`, тому replay повертає початковий
run навіть після того, як worker просуне живий run далі. Повтор того самого
operation/input не створює нових records; той самий operation ID з іншим input
відхиляється.

Reference implementation читає exact Session і committed snapshot
`session_events`, фільтрує тільки цю Session та відновлює server-controlled
порядок надходження за `received_at`, потім `event_id`. Shared policy застосовує
цей порядок і до active-tail partition, і до equal-weight truncation/final task
order; client event `timestamp` лишається domain provenance, а не керує
selection. Однаковий `received_at` поки має детермінований ID tie-break, але до
host cutover server contract має додати monotonic per-Session admission
sequence. Full-vault collection scan існує лише як in-memory conformance proof;
production S3 adapter має дати еквівалентний vault/session-bound indexed query
з тією самою snapshot/phantom семантикою.

Жодного LLM/provider call усередині start transaction немає: атомарний commit
лише публікує durable intent і workflow state. Transactional submit,
finalize/Complete coverage та cancel/clear-reservation ще не реалізовані у
Storage v2. До їх появи S2.C є перевіреною основою старту, а не завершеним
server Sleep flow і не підставою перемикати Python/Godot adapters.

PostgreSQL row-level security, composite `(vault_id, id)` references і
non-owner runtime role є defense in depth поверх bound storage handle. Вони не
скасовують server-side account-to-vault authorization. RLS context для pooled
connection задається заново всередині кожної транзакції; session-global tenant
state, який може перейти наступному borrower, заборонений.

Для server vault PostgreSQL є operational source of truth. Portable export має
містити всі versioned domain records, manifest/schema state, provenance і
checksums у детермінованому людсько-читабельному bundle. Один vault не може
одночасно мати FileStorage і PostgreSQL як дві writable truths.

---

## 16. Що цей документ не задає

Цей документ не задає:

- тіло жодного промпта;
- конкретні LLM-провайдери, моделі або API-ключі;
- точні коефіцієнти scoring/decay/relevance;
- Rust-модулі і назви файлів коду;
- UI для ручного review;
- повну JSON Schema в окремих `.schema.json` файлах.

Це свідоме обмеження. Документ задає контракт даних, а не весь майбутній продукт.

---

## Підсумок

Для MVP Memory Engine має вміти:

1. Прийняти `IngestEvent`.
2. Повернути `IngestResult` зі `StoredEvent`.
3. Записати `StoredEvent` у `memory/sessions/<session_id>/events.jsonl`.
4. Підтримувати людський `session.md`.
5. Створити preliminary `ArchiveEntry` під час sleep-stage-1.
6. Створити `PendingTask` для `sleep_compression`.
7. Прийняти `SleepCompressionResult` через `resume()`.
8. Оновити той самий `ArchiveEntry`.
9. Повернути `RecallResult` за `RecallQuery` через stage1 recall.
10. Повернути `CoreContextPackage` за `CoreContextRequest`.
11. Тримати `manifest.json` для версій; `journal/` відновлює
    reflection apply/review, тоді як runtime sleep окремо покладається на
    durable `SleepRun` checkpoints та ідемпотентні coverage rules.

Це достатній контракт для старту Rust-коду без розпливання архітектури.
