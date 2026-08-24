# ТЗ для Smartest v2: виправлення Phase 1 telemetry - 2026-08-25

## Для чого існує цей документ

Це готове завдання для інженера або агента Smartest v2, який має доступ до
production-коду бота, конфігурації моделей, provider ledger та runtime Memory
Engine.

Перший redacted package уже приніс корисні результати. Він підтвердив compact
Sleep input, terminal fallback і правильну структуру діалогу. Водночас частина
полів не дозволяє відтворити реальні provider retries, оцінити Core у
chat-контексті, зрозуміти automatic fidelity coverage та відрізнити вимкнені
вектори від незавершеної векторної роботи.

Мета цього ТЗ: виправити вимірювання, повернути короткий qualification package
і підготувати одну стабільну конфігурацію для кількаденного production-вікна.

## Related pages

- [Phase 1 review](smartest-v2-phase1-review-2026-08-25.md) - незалежний
  розбір першого package та докази для кожної знахідки.
- [Smartest Production Validation](smartest-v2-production-validation-2026-08.md) -
  нормативні `v2` export-схеми, privacy boundary, H1-H7 і наступні фази.
- [Production Integration Feedback](production-feedback-2026-08.md) -
  початкові production-проблеми та реалізовані зміни бібліотеки.
- [LLM Integration Resources](../integration/llm-integration-resources.md) -
  межа між Memory Engine і provider executor.

## Очікуваний результат

Робота завершується новим cohort із 1-3 природними terminal Sleep runs і
узгодженими JSONL-потоками `v2`. Після перевірки maintainer повертає один із
статусів:

- `QUALIFIED` - ця сама конфігурація продовжує збір до Phase 2;
- `CHANGES_REQUIRED` - виправляється конкретне поле або join;
- `BLOCKED` - публічної межі бібліотеки недостатньо для коректної інтеграції.

Вимога приблизно 20 Sleep runs починається у Phase 2. Для цього ТЗ достатньо
1-3 runs.

## Межі відповідальності

Smartest:

- виконує `LlmRequest`, який повернуло ядро;
- передає результат назад через відповідний submit/resume API;
- задає зафіксовану конфігурацію моделей і порогів;
- пише provider/runtime telemetry та анонімізує export;
- показує аномалії без текстів приватних розмов.

Memory Engine:

- вибирає Sleep tracks і memory units для automatic fidelity;
- визначає eligibility reflection;
- вирішує Archive/Core routing, attribution, forgetting і recall;
- формує budget report і prompt plan;
- володіє runtime-файлами й task lifecycle.

Production host не вибирає окремі units для fidelity через
`begin_memory_fidelity_pass`. Цей метод залишається operator/debug API.

## Заборонені скорочення

- не редагувати runtime-файли Memory Engine вручну;
- не вважати global Core count доказом наявності фактів у конкретному scope;
- не зливати semantic engine attempt та внутрішній provider retry в одне поле;
- не записувати `0`, коли значення недоступне: використовувати `null` і
  пояснення;
- не передавати тексти чатів, memory theses, prompts, scope IDs, participant
  IDs, ключі або raw provider errors;
- не починати кількаденний Phase 2 до `QUALIFIED`.

---

## Контрольна точка A: реалізація й передперевірка

На цій точці production-вікно ще не починається. Smartest реалізує export v2,
перевіряє поточну конфігурацію та повертає коротку текстову відповідь без
приватних даних.

### A1. Довести походження завантаженої бібліотеки

Build pipeline має створювати immutable artifact manifest:

```json
{
  "engine_commit": "full-git-sha",
  "loaded_extension_sha256": "sha256:...",
  "built_at": "RFC3339",
  "build_job_id": "non-secret-id"
}
```

Runtime exporter приєднує цей manifest до
`smartest_validation_build.v2`. Наявність Git на production-сервері не
потрібна. `engine_commit: unknown` є стоп-умовою.

Також записати:

- точний host commit;
- prompt commit/version;
- provider, model і API surface для `chat`, `fast`, `balanced`, `reasoning`;
- `max_provider_calls`, `max_pass_attempts`, enabled Sleep tracks;
- numeric automatic-fidelity threshold і exact configured tags;
- global vectors switch і намір для кожного тестового scope.

### A2. Переробити provider ledger на фізичні запити

Один JSONL row відповідає одному фактичному HTTP/API request до провайдера.
Внутрішні retries записуються окремо, включно з помилками.

Мінімальна форма `smartest_provider_call.v2`:

```json
{
  "schema_version": "smartest_provider_call.v2",
  "timestamp": "RFC3339",
  "workflow": "sleep|fidelity|repair|reflection|chat|other",
  "origin_run_id": "anon_run_001-or-null",
  "engine_request_id": "anon_engine_request_001",
  "request_id": "anon_physical_request_001",
  "task_id": "anon_task_001-or-null",
  "prompt_id": "memory_unit_pass-or-null",
  "engine_attempt": 1,
  "provider_attempt": 1,
  "provider": "gemini",
  "model": "gemini-3.7-flash",
  "api_surface": "generateContent|interactions|vertex|other",
  "prompt_tokens": 0,
  "cached_input_tokens": 0,
  "billable_input_tokens": 0,
  "output_tokens": 0,
  "thoughts_tokens": 0,
  "latency_ms": 0,
  "status": "ok|error",
  "error_kind": "content_filter|rate_limit|timeout|invalid_response|transport|unknown|null"
}
```

Правила:

- `request_id` унікальний для кожного фізичного request;
- `engine_request_id` групує transport retries одного `LlmRequest`;
- `engine_attempt` відповідає semantic/pass attempt Memory Engine;
- fidelity, repair і reflection, породжені Sleep run, отримують його
  `origin_run_id`;
- raw provider response та error text лишаються всередині Smartest;
- raw provider usage metadata зберігається захищено для перевірки mapping.

Acceptance: для штучного transport retry з двома спробами exporter повертає
два rows із різними `request_id`, одним `engine_request_id` і
`provider_attempt` 1 та 2.

### A3. Виправити Sleep і automatic fidelity telemetry

Використати `smartest_sleep_run.v2` із нормативного протоколу. Додатково до
поточних полів записати:

- text-free histogram ваг усіх нових MemoryUnits;
- histogram ваг units, для яких ядро повернуло fidelity requests;
- `fidelity_requested`, outcomes та post-processing provider calls;
- `origin_run_id` на кожній fidelity/repair/reflection операції;
- terminal pass status, engine attempts і recovered failures.

Фіксовані bins на весь cohort:

```json
{
  "below_0_65": 0,
  "0_65_to_0_84": 0,
  "0_85_and_above": 0
}
```

Smartest виконує всі fidelity requests, які повернуло ядро, а потім знімає
reflection snapshot. Host не формує власний список units.

Для передперевірки порахувати на наявному cohort B:

- кількість нових units у кожному weight bin;
- кількість auto-selected units у кожному bin;
- чинний threshold;
- середню та максимальну кількість automatic fidelity requests на Sleep.

Тексти units і tags конкретних користувачів не експортувати.

### A4. Зробити reflection snapshots joinable

`smartest_reflection_snapshot.v2` має містити:

- `timestamp`;
- `origin_run_id`;
- `snapshot_phase: after_auto_fidelity|after_reflection`;
- active, valid, unchecked, excluded й eligible counts;
- accumulated/remaining salience та threshold;
- due/reason;
- task status і typed outcome counts;
- cited-unit count і source-session count, коли task завершився.

Один `reflection_analyze` provider call має приєднуватись до одного snapshot і
одного origin category. Випадок `provider call існує`, а всі snapshots мають
`task_started: false`, є стоп-умовою.

### A5. Виправити chat-context telemetry

Для кожного ordinary response записувати `smartest_chat_context_turn.v2`.
Поля `estimated_memory_tokens`, `dropped_by_budget` і `budget_exceeded`
беруться прямо з `core_context_package.budget` та мають бути заповнені.

Додати:

```json
{
  "include_core": true,
  "requested_core_scope_fingerprint": "hmac-sha256:...",
  "session_personality_scope_fingerprint": "hmac-sha256:...",
  "core_notes": ["fixed_category_only"],
  "core_fact_count": 0
}
```

Scope IDs хешуються keyed HMAC тією самою export salt. Package notes
перетворюються на фіксовані категорії без приватного тексту.

Локальна перевірка одного turn:

- `current_message_occurrences == 1`;
- role order valid;
- dialogue payload count збігається з dialogue event count;
- budget object присутній;
- `include_core` і обидва scope fingerprints присутні;
- нуль Core facts має пояснюватись scope isolation або fixed package note.

### A6. Зробити runtime health придатним для recovery-аналізу

`smartest_memory_health.v2` має додатково містити:

```json
{
  "process_instance_id": "anon_process_001",
  "snapshot_reason": "startup|post_sleep|daily",
  "measurement_scope": "global|personality_scope",
  "pending_tasks": 0,
  "pending_tasks_by_type": {
    "compute_embedding": 0,
    "memory_fidelity_pass": 0,
    "reflection_analyze": 0
  },
  "oldest_pending_task_age_seconds": null,
  "collection_warnings": 0,
  "global_embeddings_enabled": false,
  "vector_state": "disabled|building|ready|corrupt",
  "vector_rows": 0
}
```

`process_instance_id` генерується при кожному старті процесу й
анонімізується під час export. Для scope-specific snapshot додати HMAC
fingerprint scope.

### A7. Узгодити vector state до qualification run

Зараз один scope має `vector_state: building`, а pending
`compute_embedding` tasks ростуть. Спершу Smartest має письмово вибрати один
режим для кожного тестового scope:

1. vectors enabled - host виконує `ComputeEmbedding`, черга дренується, scope
   доходить до `ready`;
2. vectors disabled - public vector API вимикає scope, нові embedding tasks не
   створюються.

Якщо після публічного disable старі `compute_embedding` tasks лишаються
pending, зупинити qualification і повернути:

- анонімізовані task IDs;
- task type/status/created_at;
- global switch і scope state до/після disable;
- результат public API.

Runtime-файли не видаляти й не редагувати. Такий випадок стане окремим
engine-fix із відтворюваним доказом.

### A8. Перевірити Gemini cache mapping

Записати точний Gemini API surface та назву raw usage field, з якого формується
`cached_input_tokens`.

Після готовності physical-attempt ledger виконати малий synthetic probe через
той самий provider client і модель:

- неперсональний спільний prefix;
- provider-counted prefix понад 4,096 tokens;
- prefix стоїть першим і є побайтово однаковим;
- три запити протягом короткого інтервалу;
- різниться лише коротка фінальна інструкція;
- кожний physical request записаний у v2 ledger.

Повернути provider prompt/cached/billable tokens для трьох запитів. Це
діагностика конкретного provider path, а не benchmark Memory Engine.

### A9. Зберегти privacy boundary

Перевикористати чинний fail-closed exporter. Він має відхиляти package, якщо
знаходить:

- Telegram/API credentials або private-key markers;
- raw chat, prompt, memory thesis чи provider error;
- participant, chat, Session, scope, unit, task або fact ID без remap/HMAC;
- чотири послідовні кириличні символи в structured export.

README/anomaly prose перевіряється окремо вручну. Export salt лишається тільки
на стороні Smartest.

## Відповідь після контрольної точки A

Повернути в чат цей шаблон до запуску qualification cohort:

```text
Smartest Phase 1 v2 preflight

Engine commit + extension SHA mapping: PASS/FAIL
Host/prompt commits fixed for the cohort: PASS/FAIL
Provider physical-attempt self-test: PASS/FAIL
Post-Sleep calls carry origin_run_id: PASS/FAIL
Chat budget mapping: PASS/FAIL
Core request/scope diagnostics: PASS/FAIL
Process-instance health fields: PASS/FAIL
Privacy exporter v2: PASS/FAIL

Automatic fidelity:
- enabled:
- weight threshold:
- configured tags:
- existing new-unit weight bins:
- existing auto-selected weight bins:
- average/max requests per Sleep:

Vectors by anonymized scope:
- global enabled:
- scope state:
- pending tasks by type:
- intended mode for qualification:

Gemini:
- exact API surface:
- raw cached-usage field:
- synthetic cache probe result:

Blocked items:
Proposed qualification cohort label:
```

Memory Engine maintainer після цього підтверджує automatic-fidelity threshold,
vector mode і дозвіл на контрольну точку B.

---

## Контрольна точка B: короткий qualification cohort

Після відповіді maintainer:

1. зафіксувати один host commit, prompt version, extension hash, provider/model
   routing, Sleep profile, fidelity threshold і vector mode;
2. створити новий cohort label;
3. зібрати 1-3 природні terminal Sleep runs без scripted повідомлень від
   учасників;
4. не змінювати конфігурацію всередині cohort;
5. експортувати package через fail-closed privacy check.

## Що повернути після контрольної точки B

```text
smartest_validation_build.json
smartest_provider_calls.jsonl
smartest_sleep_runs.jsonl
smartest_reflection_snapshots.jsonl
smartest_chat_context_turns.jsonl
smartest_memory_health.jsonl
smartest_instrumentation_notes.md
smartest_privacy_check.json
```

`smartest_instrumentation_notes.md` описує джерело кожного поля, усі `null`,
відхилення від ТЗ і локальні аномалії. `smartest_privacy_check.json` містить
лише версію scanner, категорії перевірок, PASS/FAIL, timestamp і SHA-256
готового archive.

Коротка супровідна відповідь:

```text
Smartest Phase 1 v2 qualification sample

Cohort:
Engine commit:
Extension SHA-256:
Host commit:
Prompt version:
Provider/models/API surfaces:
Window:
Natural terminal Sleep runs:

Physical requests fully recorded: PASS/FAIL
Call -> run -> fidelity/repair/reflection joins: PASS/FAIL
Automatic fidelity config fixed: PASS/FAIL
Reflection snapshots after returned fidelity: PASS/FAIL
Chat budget fields populated: PASS/FAIL
Core scope diagnostics populated: PASS/FAIL
Vector state consistent with intended mode: PASS/FAIL
Pending work explained: PASS/FAIL
Restart/process joins available: PASS/FAIL
Privacy scan: PASS/FAIL

Unavailable fields:
Schema deviations:
Anomalies:
Artifact path and SHA-256:
```

## Стоп-умови

Qualification package не відправляється як PASS, якщо виконується хоча б одна
умова:

- `engine_commit` невідомий;
- provider inner retries не мають окремих rows;
- post-Sleep provider work не має origin join;
- chat budget знову `null`;
- Core scope fingerprints відсутні;
- vectors описані як disabled, а scope лишається `building` без пояснення;
- pending tasks ростуть і не розкладені за типом;
- reflection provider call суперечить snapshot;
- конфігурація змінилась усередині cohort;
- privacy scanner знайшов заборонене поле.

У цьому випадку повернути `BLOCKED` або `CHANGES_REQUIRED` із одним точним
описом причини. Кількаденний збір не починати.

## Що відбудеться після QUALIFIED

Та сама конфігурація продовжує працювати у Phase 2 до мінімум 20 terminal
Sleep runs, трьох великих чатів, двох календарних днів і 300 подій. До
агрегатів додається локальна перевірка якості MemoryUnits, Core attribution і
відповідей, що використали пам'ять. Лише цей довгий cohort відповідає на H1-H7
та визначає вхід у наступну reasoning-фазу бібліотеки.
