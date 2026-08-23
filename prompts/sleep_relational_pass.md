# sleep_relational_pass

## Людське призначення

Цей промпт оцінює не факти про світ, а тон стосунку між користувачем і агентом у цій частині розмови.

## Коли запускається

Telegram host запускає цей prompt під час multi-pass sleep.

## Очікуваний результат

Повернути тільки JSON:

```json
{
  "schema_version": "sleep_relational_pass_result.v1",
  "relational_tone": {
    "warmth": 0.0,
    "intellectual_engagement": 0.0,
    "intimacy": 0.0,
    "trust": 0.0,
    "playfulness": 0.0,
    "tension": 0.0,
    "summary": "brief supported relational reading",
    "source_event_ids": ["event_id"]
  }
}
```

## Промпт

You are the relational tone pass of a memory system.

The source before this instruction is a compact `sleep_prompt_context.v1`.
Batch constants appear once in its header; each event line preserves the event
id, time, type, source, speaker, weight, useful markers, and memory text.

Use only the provided sleep task events. Do not invent relationship changes.

Estimate the relational tone shown by the interaction: warmth, intellectual engagement, intimacy, trust, playfulness, and tension.

Return only valid JSON matching `sleep_relational_pass_result.v1`.

Rules:

- Scores must be between `0.0` and `1.0`.
- Omit or set low any dimension not supported by the events.
- `summary` must be brief and grounded in evidence.
- `source_event_ids` must cite the strongest supporting events.
- If the conversation is purely transactional, return low values and a minimal summary.
