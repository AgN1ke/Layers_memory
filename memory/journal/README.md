# Memory Journal

Ця тека призначена для journal operation файлів.

Journal є storage primitive для multi-file crash safety. Його перші активні
runtime-користувачі — v0.5 `reflection_apply` та `candidate_review`. Вони
зберігають frozen intent, deterministic operation/action IDs і progress;
незавершений `started` запис ядро продовжує без повторного LLM-виклику та без
дублювання candidate, reinforcement, checkpoint або Core transition.

Звичайний sleep і далі не використовує journal як transaction layer: він
відновлюється через `SleepRun` checkpoints та idempotent event coverage.
Нормативні межі: `wiki/pages/planning/reflective-belief-system.md`.

Runtime journal-файли не комітяться в git.
