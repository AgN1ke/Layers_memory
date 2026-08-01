# Memory Journal

Ця тека призначена для journal operation файлів.

Journal є storage primitive для multi-file crash safety. Його перші активні
runtime-користувачі — v0.5 `reflection_apply` / `candidate_review` і v0.6
`hypothesis_graph_migration_operation.v1`. Вони зберігають frozen intent,
deterministic desired records/operation progress;
незавершений `started` запис ядро продовжує без повторного LLM-виклику та без
дублювання candidate, reinforcement, checkpoint, Core transition або graph
evidence.

Звичайний sleep і далі не використовує journal як transaction layer: він
відновлюється через `SleepRun` checkpoints та idempotent event coverage.
Нормативні межі: `wiki/pages/planning/reflective-belief-system.md`.

Runtime journal-файли не комітяться в git.
