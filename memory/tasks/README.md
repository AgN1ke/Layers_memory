# Memory Tasks

Ця тека містить runtime `PendingTask` файли.

Кожен файл `memory/tasks/<task_id>.json` описує задачу, яку Rust-ядро створило для хоста або адаптера. Наприклад, після `MemoryEngine::sleep(session_id)` тут зʼявляється `sleep_compression` task.

Це машинна правда для незавершених задач. Файли runtime не комітяться в git.
