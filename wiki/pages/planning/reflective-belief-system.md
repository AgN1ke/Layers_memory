# Reflective Belief System — стратегічний план

## Related pages

- [Strategy](../foundation/strategy.md) — фундаментальні межі людського
  контролю, трьох шарів пам'яті та provider-independent ядра.
- [Architecture](../foundation/architecture.md) — фактична форма Session,
  Archive, Core, PendingTask, journal і адаптерів.
- [Contracts](../foundation/contracts.md) — чинні persisted/API-контракти, від
  яких починається міграція.
- [Autonomous Reflection v0.5](autonomous-reflection-v0.5.md) — поточний цикл
  запуску reflection і його release boundary.
- [Roadmap](roadmap.md) — місце фаз у послідовності релізів.
- [Generative Agents Memory Stream review](../research/generative-agents-memory-stream-2026-07-06.md)
  — зовнішні референси та висновки, з яких виріс цей план.

## Purpose

Цей документ є стратегічним планом переходу від плоских
`CandidateBelief` до пояснюваної системи гіпотез про людину. Він поєднує три
джерела рішення:

1. фактичний аудит реалізованих Phase A–C autonomous reflection;
2. зовнішні патерни threshold-triggered reflection, sleep-time memory work і
   durable execution;
3. рішення власника дозволити кілька пов'язаних гіпотез на різних рівнях
   узагальнення замість примусового злиття їх в один текст.

Це не контракт уже реалізованих полів. Назви майбутніх сутностей у документі є
концептуальними, доки відповідна фаза не створить схему, міграцію, API і тести.

## Executive decision

Memory Engine не повинен намагатися зберігати одну «правильну» фразу про
людину. Він має поступово формувати кілька пов'язаних гіпотез різної
конкретності, зберігати походження їхньої підтримки, відрізняти пряме
свідчення від узагальнення і залишати кожний висновок переглядним.

Наприклад, одна розмова може сильно підтримати вузьку гіпотезу «користувача
цікавить колонізація Марса», слабше — «користувача цікавить колонізація інших
планет», і лише трохи — «користувача цікавить космос». Це три різні твердження,
а не три дублікати. Пізніша розмова про Венеру повинна підсилити середній і
широкий рівні, не видаючи її за новий доказ інтересу саме до Марса.

З цього випливає головна архітектурна форма: validated `MemoryUnit` лишається
атомарним досвідом в Archive; reflection формує граф reviewable гіпотез;
перевірені стабільні висновки повільно переходять у Core. Граф гіпотез є
частиною контрольованого переходу Archive → Core, а не четвертим
prompt-visible шаром пам'яті.

## Why the plan starts with reliability

Поточний happy path уже вміє створити кандидата, підсилити його новим доказом,
відкрити Core challenge або утриматися через `no_insight`. Але аудит 2026-08-01
показав, що цей шлях ще не має достатніх гарантій для складнішої когнітивної
моделі.

Застосування reflection змінює кілька файлів послідовно: candidate, memory
units, archive metadata, reflection state і PendingTask. Атомарний запис
кожного файла не робить атомарною всю операцію. Падіння між кроками може
залишити частково застосоване рішення. Окремо, відсутній `core_scope` у
поточному input builder поводиться як відсутність фільтра і може показати
reflection Core-факти інших scope. Review не фіксує дозволені переходи станів
і дозволяє caller-у підмінити scope кандидата.

Тому граф гіпотез не починається з нового промпта. Спершу ядро має гарантувати,
що один акт осмислення належить одній особистості та або відновлювано
застосований повністю, або ще не завершений.

## Target information model

### Hypothesis

Гіпотеза — це переглядне твердження, яке система сформувала про користувача,
стосунок, звичку, інтерес, стан або довготривалий патерн. Вона не є Core-фактом
і не отримує влади над поведінкою компаньйона лише через створення.

Кожна гіпотеза повинна мати:

- стабільну ідентичність та personality scope;
- формулювання і категорію;
- рівень конкретності або іншу явну ознаку семантичного охоплення;
- статус зрілості;
- окрему оцінку підтримки та окремий тиск суперечностей;
- посилання на прямі evidence records;
- зв'язки з ширшими, вужчими, можливими еквівалентними та суперечними
  гіпотезами;
- часову застосовність, коли твердження може змінитися з часом;
- повну історію review, уточнень і переходів стану.

### Evidence link

Підтримка не повинна існувати лише як агреговане число. Кожний доказ стає
окремим ребром між validated `MemoryUnit` і гіпотезою. Ребро пояснює, чи було
висловлювання прямим або виведеним, чи підтримує воно гіпотезу або суперечить
їй, з якої сесії походить і наскільки далеко гіпотеза узагальнює первинний
зміст.

Один `MemoryUnit` може бути джерелом кількох рівнів гіпотез, але лишається одним
незалежним свідченням. Поширення підтримки вгору по графу послаблюється, а його
provenance не копіюється як нове джерело. Це не дозволяє одній розмові про Марс
перетворитися на три незалежні підтвердження загального інтересу до космосу.

### Hypothesis relation

Система повинна розрізняти щонайменше чотири відношення:

- ширша гіпотеза узагальнює вужчу;
- вужча гіпотеза уточнює ширшу;
- дві гіпотези можуть означати те саме, але ще не злиті;
- одна гіпотеза суперечить іншій повністю або в певній часовій рамці.

Можлива семантична еквівалентність не дає ядру права автоматично втрачати один
із вузлів. Детермінований exact duplicate можна об'єднати безпечно. Семантичне
злиття має пройти окреме reflection/review рішення з видимим audit trail.

### Confidence is not truth probability

Внутрішній score потрібен для відбору і maturation, але число не оголошується
статистичною ймовірністю правди. Воно має бути пояснюваною функцією від:

- прямоти висловлювань;
- кількості незалежних джерел;
- різноманітності сесій і контекстів;
- user confirmation;
- відстані узагальнення;
- актуальності в часі;
- наявних заперечень.

Коефіцієнти не фіксуються в цьому документі. Спочатку ядро має зберігати
компоненти та пояснення, а початкові ваги проходять deterministic scenarios і
live calibration. Агент класифікує зміст доказу та пропонує зв'язок; ядро
обчислює score, застосовує пороги і перевіряє lifecycle.

## Responsibility boundary

LLM відповідає за мовне судження: сформулювати гіпотезу, пояснити зв'язок між
доказом і твердженням, запропонувати рівень узагальнення, помітити можливу
суперечність або визнати недостатність матеріалу.

Rust core відповідає за все, що не можна довірити статистичній моделі: scope,
допустимі ID, актуальність evidence, незалежність джерел, deterministic
identity, confidence calculation, transitions, budgets, journal, replay,
deduplication, promotion gates і аудит.

Host виконує provider call, дає ядру scheduling opportunity та рендерить
готовий prompt-facing результат. Він не створює власної memory policy. Якщо
ядро пізніше підготує можливість уточнення, host лише дає розмовному агенту
структурований шанс природно поставити питання.

## End-to-end target flow

Після sleep і fidelity новий validated unit входить у reflection trigger.
Ядро фіксує personality scope, збирає нові докази, структурно пов'язану історію
і релевантну частину наявного графа. Локальні вектори третього шару можуть
додати семантично близькі давні гіпотези, але scope встановлюється тільки
структурними ідентифікаторами, а non-vector шлях лишається повним.

LLM повертає не довільне редагування пам'яті, а типізовану пропозицію: створити
гіпотезу, додати evidence до наявної, зв'язати вузьку гіпотезу з ширшою,
позначити можливий еквівалент, відкрити суперечність або утриматися. Ядро
повторно читає всі cited records, перевіряє їхній стан і спочатку зберігає
незмінний план застосування. Лише тоді воно ідемпотентно змінює граф,
checkpoint-и та task state.

Гіпотеза не входить у звичайний chat prompt як факт. Коли її підтримка і
незалежність доказів досягають review boundary, власник бачить не лише текст,
а шлях його формування. Після explicit approval ядро проводить окрему
відновлювану promotion operation і створює або оновлює Core. До автоматичної
промоції цей план не переходить.

## Delivery phases

### Phase 1 — Reliable reflection foundation (`v0.5` closure)

Статус: реалізовано й закрито детермінованими recovery/scope/evidence tests
2026-08-01; живе приймання v0.5 ще попереду.

Мета цієї фази — зробити вже реалізовані Phase A–C чесною основою для наступних
рівнів, не додаючи нової когнітивної поведінки.

Reflection scope стає незмінною частиною кожної task/application operation.
Відсутній scope означає тільки окремий unscoped namespace, ніколи wildcard.
Candidate review бере scope із кандидата; caller може лише підтвердити його,
але не змінити. Для candidate status і Core transition вводиться явна таблиця
дозволених переходів.

Застосування typed result переходить на durable `ReflectionOperation` із
детермінованим operation ID, frozen intent, поетапним progress і завершеним
outcome. Кожний side effect стає ідемпотентним. Після restart ядро читає
незавершені operations і продовжує їх, а не повторює LLM-виклик. Owner review
і Core challenge/promotion отримують той самий recovery принцип.

Перед першим записом ядро повторно перевіряє cited MemoryUnit, candidates і
Core facts. Якщо evidence стало invalid, forgotten, moved або змінило scope,
операція завершується видимою validation failure без checkpoint нового
матеріалу. Completed task зберігає text-free outcome summary і digest, щоб
причину `no_insight`, створені/reinforced IDs та історію replay можна було
відновити після завершення host call.

Фазу завершено після fault-boundary тестів candidate → checkpoint → task →
journal, replay reinforcement без другого доказу, recovery owner review після
Core write, cross-scope/unscoped fixtures, terminal review guards і stale
evidence rejection. Це закриває фундамент цілісності, але не замінює живе
приймання v0.5 на кількох sleep/reflection cycles.

### Phase 2 — Hypothesis graph storage (`v0.6` foundation)

Статус: storage/API slice і невироджений Chibigochi host-path fixture закриті
2026-08-01. Owner storage мав нуль legacy candidates, тому його перший
live-copy run лишається історичним `DATASET GAP`; окремий disposable fixture
фактично перевірив усі mapping/provenance/isolation/idempotence/projection
гілки. Це не змінює prompt або поведінку reflection.

Мета — перейти від плоского candidate record до графа, не змінюючи одразу
поведінку reflection.

Спочатку з'являються версіоновані схеми для hypothesis node, evidence link і
hypothesis relation, storage/API methods та міграція чинних
`candidate_belief.v1`. Міграція переносить текст, status, confidence,
provenance і reinforcements, але не вигадує батьківські гіпотези або semantic
relations заднім числом. Старий кандидат стає одним вузлом із прямими evidence
links, які можна довести з наявних ID.

На цій фазі існуючий reflection може працювати через compatibility projection:
бачити graph node як старий candidate і застосовувати знайомі new/reinforce
actions. Це дозволяє окремо перевірити storage, migration, export і human
inspection до зміни промпта.

Фаза завершена, коли старе сховище відкривається без втрати provenance,
повторна міграція є ідемпотентною, а graph export дозволяє людині простежити
кожний вузол до первинних MemoryUnit.

Цей gate закрито: чотири legacy candidates у owner/other/unscoped scopes дали
сім дедуплікованих evidence links із resolved і unresolved provenance;
повторна migration не змінила graph, compatibility projection лишилася
ідентичною, а graph-only fallback відновив прибраний legacy candidate. Живу
owner memory не засівали й не мігрували.

Реалізована форма використовує окремі `hypothesis_node.v1`,
`hypothesis_evidence_link.v1` і `hypothesis_relation.v1` records у
`memory/core/hypotheses/`. `migrate_hypothesis_graph()` фіксує frozen migration
intent у journal, відновлюється після restart і не створює жодного semantic
relation. Один hypothesis + один MemoryUnit + одна polarity мають один
детермінований evidence-link ID; кілька legacy reinforcement records стають
origins того самого ребра, а не кількома незалежними доказами.

Compatibility projection двостороння: чинний reflection/review продовжує
бачити `CandidateBelief`, але кожний його запис синхронізує graph node та
evidence links; якщо legacy candidate file відсутній, node може бути
спроєктований назад у `candidate_belief.v1`. Поля, яких плоский candidate не
знає (`abstraction_level`, окремий contradiction score, validity і graph-only
directness), при такому оновленні не стираються. Relation storage відхиляє
зв'язок між різними personality scopes.

Чесна межа Phase 2: рівень узагальнення migrated node лишається `null`,
directness старого evidence — `legacy_unspecified`, а archive-only
contradiction без точного MemoryUnit лишається provenance на node. Це навмисна
відмова вигадувати зміст. Формування і maturation ієрархії починається лише у
Phase 3.

### Phase 3 — Hierarchical reflection and maturation (`v0.6` behavior)

Мета — дозволити системі формувати кілька рівнів розуміння без перетворення
перефразувань на нескінченні дублікати.

Input builder більше не бере просто останні N candidates. Він збирає bounded
підграф, пов'язаний із trigger evidence: structural selector є обов'язковим,
а optional local vector selector третього шару додає старі семантично близькі
вузли. Векторний sidecar не стає джерелом scope або truth і може бути повністю
перебудований.

Typed reflection отримує дії для створення вузької гіпотези, reinforcement,
пропозиції generalization/specialization relation, possible-equivalence і
no-insight. Ядро не приймає score моделі як остаточний. Воно додає evidence
edge, рахує незалежні джерела, послаблює підтримку під час propagation і не
дозволяє одному source unit рахуватися кілька разів у межах однієї гілки.

Широка гіпотеза дозріває лише через кілька незалежних дочірніх або прямих
гілок. Марс може сильно підтвердити Марс і трохи підтримати космос; Марс,
Венера та телескопи можуть разом сформувати сильніший загальний інтерес.

Фаза завершується сценаріями з більш ніж поточним candidate limit, різними
перефразуваннями, кількома рівнями узагальнення та non-vector fallback.
Acceptance повинен довести не відсутність усіх semantic duplicates, а
правильнішу гарантію: exact duplicates не множаться, давня релевантна гіпотеза
не губиться через recency limit, а різні рівні узагальнення зберігаються як
пов'язані, але окремі твердження.

### Phase 4 — Contradiction, time and revision (`v0.7`)

Мета — зробити пам'ять здатною визнавати не лише підтримку, а й власну помилку,
надмірне узагальнення або зміну людини з часом.

Пряме актуальне заперечення користувача створює structured dispute, а не
миттєве видалення історії. Reflection отримує гіпотезу, її support graph,
новий contradicting evidence і часовий контекст. Результатом може бути
звуження гіпотези, розділення двох значень, позначення історичної чинності,
зниження confidence, contested state, retract recommendation або потреба в
уточненні.

Пряме user statement має вищий епістемічний авторитет за виведене широке
узагальнення, але не переписує минуле. Фраза «мені нецікавий космос, мене
цікавить лише інженерія Марса» повинна послабити широку гіпотезу і зберегти
вузьку. Фраза «раніше цікавило, тепер ні» повинна закрити актуальний validity
interval, а не оголосити старі спогади неправдивими.

Core challenge переходить на той самий graph/dispute lifecycle. Старий факт
лишається видимим як contested history, а заміна не стає Core без owner review.

Фаза завершена, коли deterministic scenarios розрізняють заперечення факту,
звуження узагальнення і зміну в часі та зберігають повний audit trail.

### Phase 5 — Native clarification (`v0.7`, opt-in)

Мета — дозволити компаньйону інколи уточнити власне непевне розуміння, не
перетворюючи розмову на анкету.

Reflection не ставить питання користувачу напряму. Воно може створити
`ClarificationOpportunity`: яка гіпотеза спірна, що саме неясно, які відповіді
розв'яжуть суперечність і наскільки це важливо для майбутньої поведінки.

Ядро показує таку можливість chat agent лише коли поточна тема семантично або
структурно збігається з предметом уточнення, дотримані cooldown і question
budget, а без уточнення є реальний ризик відповісти неправильно. Старе питання
не переноситься в unrelated topic. Host не винаходить питання самостійно, а
рендерить core-owned prompt hint; розмовний агент вирішує, чи звучить воно
природно в конкретній репліці.

Відповідь користувача входить у звичайний Session → sleep → fidelity шлях.
Вона не патчить hypothesis напряму. Наступна reflection operation закриває
clarification, додає evidence і видимо змінює support/dispute state.

Ця фаза лишається opt-in до живої перевірки частоти й доречності. Acceptance
вимагає, щоб питання з'являлося під час повернення до теми, не з'являлося після
topic switch, не повторювалося після відповіді та не підміняло owner review.

## Release and implementation order

Порядок є жорсткою залежністю, а не побажанням:

```text
v0.5: reliable reflection and review
    ↓
v0.6: hypothesis graph storage
    ↓
v0.6: hierarchical reflection and evidence maturation
    ↓
v0.7: contradiction and temporal revision
    ↓
v0.7: opt-in native clarification
```

Кожна фаза має власні schema/API зміни, deterministic acceptance і live gate.
Новий prompt створюється лише разом із кодом, що його реально викликає.
Наступна фаза не використовується для виправдання незакритого acceptance
попередньої.

## What is deliberately out of scope

План не додає постійно запущеного третього агента, хмарний vector store,
автоматичну промоцію в Core, автоматичне semantic merge без audit trail або
навчання ваг моделі. Він також не перетворює confidence на заявлену
ймовірність правди.

Memory Engine не стає загальним workflow orchestrator. Він запозичує вузький
durable-execution принцип для власних multi-file memory operations. Повний
LangGraph/Temporal runtime був би зайвою залежністю для локального Rust core.

## Main risks and controls

Головний quality-ризик — модель може плутати еквівалентність і узагальнення.
Тому вона лише пропонує relation, а неоднозначне semantic merge не видаляє
вузол автоматично.

Головний integrity-ризик — recovery code сам може створити повторний side
effect. Тому operation IDs і action IDs є детермінованими, а fault injection
перевіряє кожну межу запису.

Головний product-ризик — надто активні уточнення. Тому clarification має
relevance gate, cooldown, budget і opt-in live acceptance.

Головний privacy-ризик — graph telemetry може перетворитися на лог особистого
життя. Постійна telemetry зберігає ID, типи relations, score components,
counts і machine reasons, але не тексти спогадів або запитів. Повні тексти
лишаються тільки у власному auditable memory storage.

## Definition of strategic success

Система вважається успішною не тоді, коли вона накопичила багато Core-фактів,
а коли вона може пояснити шлях від досвіду до розуміння і безпечно змінити це
розуміння після нового досвіду.

Для кожного стабільного висновку має бути видно: які конкретні спогади його
підтримали, які з них незалежні, яке узагальнення зробила система, які
заперечення вона бачила, чому висновок дозрів і хто дозволив йому перейти в
Core. Водночас випадковий епізод повинен залишатися епізодом, а непевність —
непевністю.
