# Нотатки: як логічно завершити 3-шарову пам’ять

Джерела: `wiki/pages/planning/autonomous-reflection-v0.5.md` та
`wiki/pages/planning/reflective-belief-system.md` у `C:/Python_projects/Layers_memory`.
Це коротка дизайн-вирізка: вона розділяє вже реалізований фундамент і заплановану
поведінку, не оголошуючи roadmap прийнятим рішенням.

## Принципи

1. **Три шари — це шлях довіри, а не три незалежні сховища істини.**
   Session → validated Archive `MemoryUnit` → reviewable hypothesis → лише після
   explicit approval Core **для висновків reflection**; гіпотеза не стає prompt-visible фактом через саме
   створення ([Reflective Belief System — “Executive decision”, lines 38–53;
   “End-to-end target flow”, 155–174]). Вже реалізований v0.5 sleep/fidelity/reflection
   шлях має механізми переходу Archive → Candidate/Core review, але не всю
   ієрархію і не замінює ще відкритого live gate. Це не правило для будь-якого
   запису Core: чинний personal bridge окремо допускає прямі self-statements із
   перевіркою автора, джерела та scope (architecture.md, §5.4).

2. **Кожне узагальнення має залишатися пояснюваним і незалежним за доказами.**
   Один `MemoryUnit` може підтримати кілька рівнів, але рахується одним незалежним
   свідченням; propagation вгору послаблюється і не копіює provenance
   ([Reflective Belief System — “Evidence link”, 95–106]). У Phase C це вже
   частково забезпечено provenance/reinforcement deduplication; незалежні джерела,
   weakening propagation і maturation — наступна Phase 3 поведінка (266–309).

3. **Confidence — пояснюваний робочий score, не ймовірність правди.**
   Його компоненти мають охоплювати прямоту, незалежні джерела, різні сесії,
   підтвердження користувача, відстань узагальнення, час і заперечення
   ([Reflective Belief System — “Confidence is not truth probability”, 121–137]).
   Ядро рахує score, lifecycle та пороги; LLM лише класифікує і пропонує зв’язок
   ([“Responsibility boundary”, 139–153]).

4. **Суперечність змінює актуальність, а не стирає історію.**
   Пряме заперечення має створити structured dispute; минулий факт лишається
   contested history, а часовий контекст може закрити validity interval
   ([Reflective Belief System — “Phase 4 — Contradiction, time and revision”,
   311–333]). У поточному v0.5 є лише `challenge_core_fact`, який залишає Core
   активним до owner approval ([autonomous-reflection-v0.5 — “Phase C - Meaningful
   result actions”, 202–231]); temporal revision ще не реалізована.

5. **`no_insight` — нормальний результат, що захищає від надмірного навчання.**
   Тимчасовий стан має залишатися епізодом; успішний zero-candidate/no-insight
   рухає checkpoint і не створює belief ([autonomous-reflection-v0.5 — “Trigger
   Semantics”, 79–107; “Phase C - Meaningful result actions”, 207–228]). Це вже
   реалізовано й має бути обов’язковим guard для майбутньої ієрархії.

6. **Уточнення — рідкісна, core-owned, opt-in можливість, а не фонове опитування.**
   `ClarificationOpportunity` показується лише при поверненні до релевантної теми,
   після cooldown/budget і коли помилка без питання справді ризикована; відповідь
   проходить звичайний Session → sleep → fidelity шлях ([Reflective Belief System
   — “Phase 5 — Native clarification”, 335–357]). Це майбутня v0.7 функція, не
   дозвіл host-у патчити hypothesis напряму.

## Логічні наступні кроки

- **Спершу закрити живий v0.5 gate.** Перевірити кілька sleep cycles: стабільний
  інтерес → одна candidate з незалежним reinforcement; тимчасовий стан → no-insight;
  конфлікт → challenge без тихої зміни Core. Це саме нормативна live-процедура
  ([autonomous-reflection-v0.5 — “Live Phase C procedure”, 315–351]).
- **Потім увімкнути Phase 3 поверх уже наявного graph storage.** Ввести bounded
  trigger-linked subgraph із обов’язковим structural selector, typed actions для
  generalization/specialization/equivalence/no-insight і acceptance для exact
  duplicate, давньої релевантності та кількох рівнів узагальнення
  ([Reflective Belief System — “Phase 3 — Hierarchical reflection and maturation”,
  266–309]).
- **Після цього додати temporal/dispute lifecycle.** Зберігати polarity,
  validity interval, support/contradiction pressure та audit trail; розрізняти
  заперечення, звуження узагальнення й зміну з часом ([“Hypothesis”, 76–93;
  “Phase 4 — Contradiction, time and revision”, 311–333]).
- **Лише після вимірювання доречності — opt-in clarification.** Спочатку довести
  topic return, cooldown, question budget, no-repeat-after-answer і відсутність
  переносу питання після topic switch ([“Phase 5 — Native clarification”, 335–357]).

Підсумкова межа: реалізований фундамент уже дає scope safety, durable/replayable
apply-review, evidence revalidation, typed outcomes і `no_insight`; запланована
когнітивна надбудова має додати незалежність джерел у maturation, рівні гіпотез,
часову ревізію та обережне уточнення, не створюючи четвертого prompt-visible шару.
