# sleep_compression

## Людське призначення

Цей промпт використовується тоді, коли Memory Engine уже створив попередній архівний спогад алгоритмічно, але хост хоче доробити його LLM-моделлю: зробити коротший gist, чистіший narrative, виділити факти, цитати, теги і тему.

Промпт не запускається самим Rust-ядром. Ядро створює `PendingTask` з `prompt_id: "sleep_compression"`, а хост або адаптер читає цей файл і виконує задачу через обрану модель.

## Коли запускається

Після `MemoryEngine::sleep(session_id)`, коли створено:

- preliminary `ArchiveEntry`;
- `PendingTask` типу `sleep_compression`;
- structured `inputs.events` зі списком подій сесії.

## Вхідні дані

Хост передає моделі structured input з `PendingTask.inputs`:

- `session_id`;
- `preliminary_archive_id`;
- `events`;
- `hints`.

Кожна подія містить:

- `event_id`;
- `type`;
- `timestamp`;
- `payload`;
- `tags`;
- `theme`;
- `initial_weight`;
- `weight_reason`.

## Очікуваний результат

Модель має повернути JSON форми `sleep_compression_result.v1`:

```json
{
  "schema_version": "sleep_compression_result.v1",
  "archive_id": "archive_id_from_input",
  "gist": "Короткий людський зміст.",
  "narrative": "Стислий, але зрозумілий опис спогаду.",
  "facts": [
    {
      "text": "Факт, який прямо випливає з подій.",
      "confidence": 0.8,
      "source_event_ids": ["event_id"]
    }
  ],
  "quotes": [
    {
      "text": "Коротка дослівна цитата, якщо вона важлива.",
      "source_event_id": "event_id"
    }
  ],
  "tags": ["short_machine_tag"],
  "theme": "short_theme",
  "weight": 0.8,
  "links": []
}
```

## Промпт

You are compressing a session into one durable memory item for Memory Engine.

Use only the provided events. Do not invent facts, dates, preferences, emotions, names, locations, or intentions that are not supported by the events.

Return only valid JSON matching `sleep_compression_result.v1`. Do not wrap the JSON in Markdown.

Rules:

- Keep `archive_id` equal to `preliminary_archive_id`.
- `gist` must be one short sentence.
- `gist` is not a topic list. It must preserve the most humanly salient episode or episodes.
- `narrative` must be human-readable and concise, but it must preserve the memory shape: what happened, what it meant, and the emotional tone when the events support one.
- `facts` must contain only stable facts that may be useful later.
- Each fact must include `source_event_ids`.
- Use direct `quotes` only when the wording itself is useful.
- `tags` must be short machine-readable strings in `snake_case`.
- `theme` must be a short machine-readable string or `null`.
- `weight` must be between `0.0` and `1.0`.
- Prefer preserving important user-stated facts over stylistic summaries.
- Prefer personally meaningful user disclosures, explanations, preferences, corrections, relationship details, named recurring entities, and emotionally colored moments over generic encyclopedia-style Q&A.
- For generic informational Q&A, keep only the user-facing takeaway unless the exact factual detail is likely to be useful later.
- If the user shares why something matters, preserve both the event and the reason.
- If the user shows affect through wording, correction, enthusiasm, frustration, affection, disappointment, or humor, include that affect in `narrative` or as a supported fact. Do not invent feelings that are not evidenced by the events.
- Do not prioritize an entity because of its type. Prioritize it only when the events show personal meaning, emotional salience, repetition, correction, or likely future usefulness.
- If the events do not contain durable memory, return a low `weight` and a minimal summary.

## Що Можна Безпечно Редагувати

Можна змінювати стиль інструкцій, суворість стислості, правила тегів і підказки щодо якості фактів.

## Що Не Можна Міняти Без Оновлення Контрактів Або Тестів

Не можна змінювати:

- `prompt_id`;
- очікувану схему `sleep_compression_result.v1`;
- назви JSON-полів;
- вимогу повертати тільки JSON;
- правило, що `archive_id` дорівнює `preliminary_archive_id`.
