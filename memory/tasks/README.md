# Memory Tasks

Ця тека містить runtime `PendingTask` файли.

Кожен файл `memory/tasks/<task_id>.json` описує задачу, яку Rust-ядро створило для хоста або адаптера. Наприклад, після `MemoryEngine::sleep(session_id)` тут зʼявляється `sleep_compression` task, а коли validated memory досягає core-owned reflection trigger — `reflection_analyze` task.

Phase B reflection task зберігає explicit personality `reflection_scope`,
привʼязані source sessions, окремі `trigger`/`related_history` units та bounded
existing candidates. Це auditable reasoning input, а не новий memory layer.

Phase C tasks request `reflection_actions_result.v1`. The result must contain
either typed actions (`new_candidate`, `reinforce_candidate`,
`challenge_core_fact`) or structured `no_insight`. Old persisted tasks that
expect `reflection_result.v1` remain resumable.

Після durable apply completed reflection task має additive `outcome` у формі
`reflection_outcome.v1`: operation ID, SHA-256 result digest, IDs створених або
reinforced candidates та optional структурована причина no-insight. Тексти
гіпотез і пояснення у цей terminal summary не копіюються.

Це машинна правда для незавершених задач. Файли runtime не комітяться в git.
