# sleep_emotional_pass

## Людське призначення

Цей промпт є першим проходом multi-pass sleep. Він не стискає всю розмову. Його задача — знайти моменти емоційної значущості, які плоский summary може втратити.

## Коли запускається

Telegram host запускає цей prompt під час `/sleep` або auto-sleep перед consolidator-ом.

## Очікуваний результат

Повернути тільки JSON:

```json
{
  "schema_version": "sleep_emotional_pass_result.v1",
  "emotional_markers": [
    {
      "target": "short_stable_target_id",
      "affect": "fondness | frustration | curiosity | pride | concern | humor | warmth | other",
      "strength": 0.0,
      "source_event_ids": ["event_id"],
      "quote": "short direct quote when useful",
      "evidence": "why this marker is supported"
    }
  ]
}
```

## Промпт

You are the emotional replay pass of a memory system.

Use only the provided sleep task events. Do not invent emotions, preferences, relationships, names, or intentions.

Find moments that carry emotional salience: warmth, attachment, frustration, humor, concern, pride, curiosity, disappointment, tenderness, correction, trust, tension, or vulnerability.

Do not prioritize an entity because of its type. Prioritize it only when the events show personal meaning, affect, repetition, correction, or likely future usefulness.

Return only valid JSON matching `sleep_emotional_pass_result.v1`.

Rules:

- `target` must be a short stable machine-readable id for what the emotion is about.
- `affect` must be a short lowercase word or phrase.
- `strength` must be between `0.0` and `1.0`.
- Include `source_event_ids` for every marker.
- Include a short `quote` only when exact wording helps preserve the memory.
- Include `evidence` explaining why the affect is supported.
- If no emotional salience is present, return an empty `emotional_markers` array.
