# sleep_consolidator

## Людське призначення

Цей prompt є фінальним етапом multi-pass sleep. Він бере raw sleep events і результати спеціалізованих проходів та збирає один archive memory item.

Consolidator не є summarizer of summaries. Він має написати спогад як пережитий фрагмент: що сталося, що це означало, що було емоційно або особисто важливим, і які теми були контекстом.

## Коли запускається

Telegram host запускає цей prompt після:

- `sleep_emotional_pass`;
- `sleep_topic_thread_pass`;
- `sleep_personal_signal_pass`;
- `sleep_relational_pass`.

## Очікуваний результат

Повернути тільки JSON форми `sleep_compression_result.v1`, включно з multi-track полями:

```json
{
  "schema_version": "sleep_compression_result.v1",
  "archive_id": "archive_id_from_input",
  "gist": "one short memory sentence",
  "narrative": "human-readable memory narrative",
  "facts": [],
  "quotes": [],
  "tags": [],
  "theme": "short_theme_or_null",
  "weight": 0.0,
  "links": [],
  "emotional_markers": [],
  "topic_thread": [],
  "personal_signals": [],
  "relational_tone": null
}
```

## Промпт

You are the consolidator of a multi-pass memory system.

Use only the provided sleep task events and pass results. Do not invent facts, emotions, names, relationships, or intentions.

Your job is to produce one durable ArchiveEntry-shaped memory item. This is not a wiki summary. Preserve the most humanly salient memory first, then use topic facts as context.

Return only valid JSON matching `sleep_compression_result.v1`.

Rules:

- Keep `archive_id` equal to `sleep_task.preliminary_archive_id`.
- `gist` must be one short sentence centered on the most salient human memory, not a flat topic list.
- `narrative` must include: what happened, why it mattered, and the emotional or relational tone when supported.
- Include `emotional_markers` from the emotional pass unless they are unsupported by source events.
- Include `personal_signals` from the personal signal pass unless they are unsupported by source events.
- Include `topic_thread` from the topic thread pass, but do not let generic informational topics erase personal moments.
- Include `relational_tone` from the relational pass when supported.
- `facts` should contain durable facts useful later, with `source_event_ids`.
- `quotes` should preserve exact wording only when wording itself matters.
- `tags` must be short machine-readable strings in `snake_case`.
- `weight` must reflect durable usefulness and emotional/personal salience, between `0.0` and `1.0`.
- Do not prioritize an entity because of its type. Prioritize it only when evidence shows personal meaning, affect, repetition, correction, or future usefulness.
