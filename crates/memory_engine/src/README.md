# Rust Source Layout

Ця папка містить Rust-скелет ядра системи Memory Engine.

Поточний етап - типи контрактів і межі модулів. Реальна бізнес-логіка буде додаватися поступово поверх цих типів.

## Модулі

- `lib.rs` - публічний вхід у crate.
- `engine.rs` - публічний facade `MemoryEngine`: перший людський API поверх storage, починаючи з `ingest()`.
- `bin/memory_terminal.rs` - локальний інтерактивний terminal runner для ручної перевірки ingest, sleep і recall.
- `types.rs` - спільні типи: id, timestamp, links, model roles, schema constants.
- `event.rs` - `IngestEvent` і `StoredEvent`.
- `file_storage.rs` - перша файлова імплементація `Storage`.
- `session.rs` - metadata сесії і `SessionRecord`.
- `archive.rs` - `ArchiveEntry` і фільтри архіву.
- `core_store.rs` - Core Store, Core Context Package і Candidate Belief.
- `recall.rs` - `RecallQuery`, `RecallResult` і stage markers.
- `tasks.rs` - `PendingTask` і task attempts.
- `sleep.rs` - `SleepCompressionResult` і базова валідація результату.
- `reflection.rs` - manual/autonomous reflection contracts, trigger status і
  durable per-scope processing state.
- `engine/reflection_flow.rs` - core-owned eligibility, task orchestration,
  personality-scope membership, bounded structural history selection,
  typed result actions, reinforcement, checkpointing і CandidateBelief
  lifecycle.
- `manifest.rs` - `memory/manifest.json`.
- `journal.rs` - journal operation для crash safety.
- `storage.rs` - `Storage` trait як межа між ядром системи і фізичним сховищем.
- `config.rs` - конфігурація ядра системи: шлях до `memory_dir` і ліміти. Провайдери, моделі, API-ключі і `prompts_dir` сюди не входять - це справа хоста і адаптера.
- `error.rs` - спільний тип помилок.

## Правило

Модулі мають залишатися малими і відповідати контрактам у `wiki/pages/foundation/contracts.md`.

Якщо нова логіка потребує нового типу даних, спочатку оновлюємо контракт або явно фіксуємо, чому це внутрішній тип, який не є частиною зовнішнього формату.

## Personality-scope reflection

Autonomous reflection не прирівнює сесію до особистості. Host явно передає
стабільний `core_scope`; reflection state памʼятає привʼязані до нього source
sessions, а кожний unit зберігає власне походження. Давня history входить у
task лише через bounded structural match і маркується окремо від нового
trigger evidence.

Phase C actions do not add another truth store. A reinforcement extends the
same candidate record with new evidence, while a Core challenge remains a
candidate until explicit review. New tasks request
`reflection_actions_result.v1`; legacy `reflection_result.v1` responses remain
readable for compatibility. Scope isolation, terminal review guards, live
evidence revalidation, journal-backed idempotent apply/review and text-free
task outcomes are implemented; live v0.5 acceptance remains pending. The
post-v0.5 hypothesis graph is planned, not part of the current source contract.
