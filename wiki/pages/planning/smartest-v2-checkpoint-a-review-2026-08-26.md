# Smartest v2: review контрольної точки A - 2026-08-26

## Для чого існує цей документ

Це рішення maintainer після перевірки передзапускового звіту Smartest для
qualification cohort `c`. Воно визначає, чи можна збирати коротку природну
вибірку, і виправляє дві неточні інтерпретації протоколу до початку
багатоденного production-вікна.

## Related pages

- [Phase 1 correction TZ](smartest-v2-phase1-correction-tz-2026-08-25.md) -
  завдання, за яким Smartest підготував передперевірку.
- [Phase 1 review](smartest-v2-phase1-review-2026-08-25.md) - висновок щодо
  першого telemetry package.
- [Smartest Production Validation](smartest-v2-production-validation-2026-08.md) -
  нормативний протокол і гіпотези H1-H7.
- [Roadmap](roadmap.md) - активний порядок робіт після цього рішення.

## Рішення

**Контрольна точка A: `QUALIFIED`.**

Smartest може продовжувати qualification cohort `c` з конфігурацією, яка
зафіксована 2026-08-25 05:53 UTC. Треба зібрати 1-3 природні terminal Sleep
runs і повернути package контрольної точки B.

**Phase 1 ще не завершена.** Остаточний статус `QUALIFIED` для Phase 1 буде
видано після перевірки package контрольної точки B. Багатоденний Phase 2 до
цього не починається.

## Що прийнято

- Завантажений Python extension однозначно пов'язаний із Memory Engine commit
  `6dd6b6879f853bc740a331f5d2fdeb6a4bc377f1` через byte-identical rebuild і
  SHA-256. Застарілий checkout на production-сервері більше не є джерелом
  provenance.
- Host і prompt revisions зафіксовані на весь cohort.
- Один рядок provider ledger відповідає одному фізичному запиту; semantic
  engine attempt і transport attempt розділені.
- Post-Sleep fidelity, repair і reflection мають `origin_run_id`.
- Chat budget, Core request/scope, process instance і typed pending-work поля
  заповнюються з фактичних контрактів.
- Privacy exporter v2 працює fail-closed і пройшов передперевірку.
- Automatic fidelity threshold явно зафіксований як `0.85`. Відбір нижче
  порога пояснюється Core-path або risk routing, тому не суперечить threshold.
- Для `anon_chat_002` vector scope готовий, має 23 rows і порожню embedding
  queue; інші scope лишаються вимкненими згідно з планом cohort.
- На фактичному Gemini `generateContent` path поле
  `cachedContentTokenCount` доступне, але три запити зі спільним префіксом
  понад provider minimum повернули нуль cached tokens. Для цього шляху
  implicit prefix cache у поточній конфігурації вважається недоступним.

## Обов'язкове виправлення трактування fidelity

`anomalies.md` помилково називає 49 `unchecked` units host backlog і пропонує
очищати його через `begin_memory_fidelity_pass`.

У production flow Smartest виконує тільки automatic fidelity requests, які
повернуло ядро. Саме ядро вибирає units за threshold, Core-path і risk routing.
`begin_memory_fidelity_pass` лишається ручним operator/debug API. Його масовий
виклик змінив би досліджувану політику, збільшив витрати й зробив cohort `c`
непорівнюваним.

Правильний висновок для A-001:

- automatic routing працює відповідно до конфігурації `0.85`;
- велика частка легких `unchecked` units є властивістю поточної політики, а не
  незавершеною роботою хоста;
- qualification cohort має показати, чи дає ця політика достатній живий
  reflection funnel;
- якщо funnel лишиться занадто вузьким, threshold або інша core-owned routing
  policy змінюється після завершення cohort і вимірюється в новій когорті.

Smartest має виправити A-001 у супровідній документації. Збір природних runs
через цю текстову правку не зупиняється.

## Уточнення provenance boundary

Попередня версія протоколу вимагала extension від commit `1aa427b` або його
нащадка. Це змішало runtime provenance з версією документа дослідження.

`6dd6b68` є останнім runtime implementation commit перед контрольними
документами. Diff від `6dd6b68` до `1aa427b` і `1e3101f` не містить змін у
`crates/`, `hosts/`, `prompts/` або `tests/`. Тому byte-identical extension від
`6dd6b68` прийнятий. Надалі protocol revision і extension source commit
записуються окремо; docs-only commit не вимагає беззмістовної перезбірки
бінарного модуля.

## Що Smartest робить зараз

1. Не змінює provider/model routing, prompts, Sleep profile, fidelity threshold
   `0.85`, vector mode або export schemas усередині cohort `c`.
2. Не запускає масову ручну fidelity-перевірку старих units.
3. Чекає 1-3 природні terminal Sleep runs без scripted повідомлень.
4. Повертає вісім артефактів із контрольної точки B, privacy report і
   виправлений anomaly log.
5. Окремо перелічує будь-які schema deviations, unavailable fields і локальні
   аномалії, не змінюючи runtime-файли Memory Engine вручну.

Після цього maintainer перевіряє joins і консистентність та видає фінальний
статус Phase 1. Лише `QUALIFIED` відкриває довгий Phase 2.
