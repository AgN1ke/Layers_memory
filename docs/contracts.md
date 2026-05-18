# Контракти даних Memory Engine v0.1

## Для чого існує цей документ

`docs/architecture.md` фіксує архітектурну форму Memory Engine: ядро системи, три шари пам'яті, PendingTask, recall, sleep, storage і адаптери.

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

- `memory/sessions/<session_id>/events.jsonl`.

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
- `core_fact_upsert_result.v1`;
- `core_context_request.v1`;
- `core_context_package.v1`;
- `candidate_belief.v1`;
- `recall_query.v1`;
- `recall_result.v1`;
- `pending_task.v1`;
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
  "links": [],
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
- `links`;
- `importance_hint`;
- `processing_mode`.

Правила:

- `payload` належить хосту і може мати довільну JSON-структуру;
- ядро системи не має жорстко знати всі можливі `type`;
- `tags` мають бути короткими машинними рядками;
- `theme` - одна основна тема, якщо хост її знає;
- `emotional_tone` - рядок, без фіксованого enum на v0.1.

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

`events.jsonl` містить саме `StoredEvent`.

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
  },
  "auto_sleep": null
}
```

`auto_sleep` або `null`/відсутній, або має форму `SleepStage1Result`:

```json
{
  "archive_entry": {},
  "pending_task": {}
}
```

Auto-sleep створюється ядром, коли кількість незаархівованих подій у сесії досягає налаштованого порога. Хост не вирішує, коли стискати сесію; хост тільки виконує повернений `PendingTask` через свій LLM-провайдер.

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
- `archived_to` - список `archive_id`, створених зі сесії;
- `notes` - службові або людські нотатки.

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
- може бути перебудований із `events.jsonl` і `session.json`.

---

## 5. ArchiveEntry

Архівний спогад - довгостроковий запис, створений зі сесії.

Файли:

- `memory/archive/<YYYY>/<MM>/<archive_id>.json`;
- `memory/archive/<YYYY>/<MM>/<archive_id>.md`.

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
- `status`;
- `llm_enhanced`;
- `prompt_id`;
- `prompt_version`;
- `embedding_model_id`;
- `embedding`.

`status`:

- `preliminary` - створений алгоритмічно на sleep-stage-1;
- `complete` - оновлений після LLM-доробки або прийнятий як повний без неї;
- `superseded` - замінений новішим спогадом;
- `needs_review` - потребує ручного огляду.

`embedding_model_id` і `embedding` на v0.1 завжди `null`.

Поля, які мають бути присутні в JSON, але дозволено значення `null`:

- `theme` (сесія могла не мати чіткої теми);
- `last_recalled_at` (поки спогад не повертався recall'ом);
- `prompt_id` (preliminary запис без LLM-доробки);
- `prompt_version` (те саме);
- `embedding_model_id` (на v0.1 завжди `null`);
- `embedding` (на v0.1 завжди `null`).

### 5.2 Markdown

Файл `<archive_id>.md` - людський виклад того самого спогаду.

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

- `memory/core/store/<category>.json`;
- `memory/core/store/<category>.md`.

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

### 6.3 CoreContextRequest

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
  "recall_limit": 5,
  "session_recent_limit": 40,
  "session_trace_event_limit": 120,
  "include_core": false
}
```

`domain_state` приходить від хоста у момент запиту і не записується в Core Store. `core_scope` фільтрує `core_facts`; якщо він заданий, ядро повертає тільки факти з таким самим `scope`.

### 6.4 CoreContextPackage

Core Context Package не обов'язково зберігається на диск. Це відповідь ядра на запит хоста.

```json
{
  "schema_version": "core_context_package.v1",
  "created_at": "2026-05-17T17:25:00.000Z",
  "core_facts": [
    {
      "category": "profile",
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
      "gist": "Розмова про МіГ-15.",
      "narrative": "Користувач питав про радянський винищувач МіГ-15.",
      "facts": [],
      "quotes": [],
      "source_session_id": "2026-05-17_005",
      "tags": ["aircraft"],
      "theme": "aviation",
      "weight": 0.9,
      "freshness": 1.0,
      "relevance_score": 0.8
    }
  ],
  "domain_state": {
    "active_topic": "travel_planning"
  },
  "notes": []
}
```

На v0.1 `core_facts` заповнюється з категорій `profile`, `preferences`, `relationship`, якщо host або користувач уже зберіг туди стабільні факти. Хости мають використовувати `CoreContextPackage` як єдину точку збору prompt-контексту, а не дублювати session/recent/archive/core логіку в кожному host-і.

---

## 7. CandidateBelief

CandidateBelief - кандидат на стабільний висновок у Core Store.

Файл: `memory/core/candidates/<candidate_id>.json`.

```json
{
  "schema_version": "candidate_belief.v1",
  "candidate_id": "candidate_01J00000000000000000000001",
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

На v0.1 candidate може бути створений вручну або через майбутній reflection-контракт, але автоматичний reflection не реалізується.

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

`source_layers`:

- `session`;
- `archive`;
- `core`.

На v0.1 основний recall працює по `archive` і `core`. Пошук у live-session може бути доданий як проста перевірка поточної сесії.

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
  "stage_used": "stage1",
  "items": [
    {
      "source_layer": "archive",
      "id": "archive_01J00000000000000000000001",
      "gist": "Користувач повідомив, що переїхав у Берлін минулого місяця.",
      "narrative": "Під час розмови користувач уточнив важливу зміну в особистому контексті.",
      "facts": [
        "Користувач переїхав у Берлін приблизно у квітні 2026 року."
      ],
      "quotes": [
        "Я переїхав у Берлін минулого місяця"
      ],
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
    "candidate_count": 12,
    "filtered_count": 3
  }
}
```

`stage_used`:

- `stage1` на v0.1;
- `stage2_embeddings` у майбутньому;
- `stage3_llm` у майбутньому.

`debug` може бути відсутнім у production-режимі.

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
  "last_error": null
}
```

`task_type` v0.1:

- `sleep_compression`;
- `score_event` опціонально.

Зарезервовані на v0.2+:

- `reflection_analyze`;
- `recall_rerank`;
- `compute_embedding`;
- `fact_check`;
- `tag_proposal`.

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

---

## 10. SleepCompressionResult

Результат, який хост повертає у `engine.resume(task_id, result)` для `sleep_compression`.

```json
{
  "schema_version": "sleep_compression_result.v1",
  "archive_id": "archive_01J00000000000000000000001",
  "gist": "Користувач повідомив, що нещодавно переїхав у Берлін.",
  "narrative": "У розмові користувач поділився важливою зміною в особистому контексті: він переїхав у Берлін минулого місяця. Це може впливати на майбутні розмови про побут, роботу, подорожі й локальний контекст.",
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
  "links": []
}
```

Правила валідації:

- `archive_id` має збігатися з preliminary archive entry, якщо task був створений для оновлення існуючого запису;
- `gist` не має бути порожнім;
- `narrative` не має бути порожнім;
- `facts` може бути порожнім, але якщо факт є, він має мати `text` і `confidence`;
- `quotes` можуть бути порожніми;
- `weight` має бути `0.0..1.0`;
- `tags` мають бути короткими машинними рядками.

Тіло промпта для отримання такого результату не створюється в цьому документі. Воно з'явиться у `prompts/sleep_compression.md` тільки разом із реальною sleep-stage-2 реалізацією.

---

## 11. Manifest

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
    "pending_task": "pending_task.v1",
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

## 12. JournalOperation

JournalOperation описує мульти-файлову операцію, яку потрібно або завершити, або безпечно розібрати після обриву.

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

`op_type` v0.1:

- `sleep`;
- `migration`;
- `core_promotion`.

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

## 13. Файли і джерела правди

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
  archive/
    <YYYY>/<MM>/
      <archive_id>.json
      <archive_id>.md
  core/
    store/
      <category>.json
      <category>.md
    candidates/
      <candidate_id>.json
  tasks/
    <task_id>.json
  journal/
    <op_id>.json
```

Джерела правди:

- `events.jsonl` - правда для подій сесії;
- `<archive_id>.json` - правда для архівного спогаду;
- `<category>.json` - правда для Core Store;
- `<candidate_id>.json` - правда для candidate belief;
- `<task_id>.json` - правда для PendingTask;
- `manifest.json` - правда для версій сховища;
- Markdown-файли - людський вигляд, не машинна правда.

---

## 14. Що цей документ не задає

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
2. Повернути `IngestResult` зі `StoredEvent` і, за потреби, `auto_sleep`.
3. Записати `StoredEvent` у `memory/sessions/<session_id>/events.jsonl`.
4. Підтримувати людський `session.md`.
5. Створити preliminary `ArchiveEntry` під час sleep-stage-1.
6. Створити `PendingTask` для `sleep_compression`.
7. Прийняти `SleepCompressionResult` через `resume()`.
8. Оновити той самий `ArchiveEntry`.
9. Повернути `RecallResult` за `RecallQuery` через stage1 recall.
10. Повернути `CoreContextPackage` за `CoreContextRequest`.
11. Тримати `manifest.json` і `journal/` для версій та crash safety.

Це достатній контракт для старту Rust-коду без розпливання архітектури.
