# sleep_topic_thread_pass

## Людське призначення

Цей промпт зберігає епізодичну структуру розмови: які теми йшли одна за одною, де був перехід, де енергія зростала чи спадала.

## Коли запускається

Telegram host запускає цей prompt під час multi-pass sleep.

## Очікуваний результат

Повернути тільки JSON:

```json
{
  "schema_version": "sleep_topic_thread_pass_result.v1",
  "topic_thread": [
    {
      "topic": "short_machine_topic",
      "subtopics": ["short_subtopic"],
      "energy": "low | steady | engaged | warm | tense | playful | other",
      "source_event_ids": ["event_id"],
      "summary": "brief human summary of this thread"
    }
  ]
}
```

## Промпт

You are the episodic thread pass of a memory system.

Use only the provided sleep task events. Do not invent topics or transitions.

Build a chronological thread of the conversation. Preserve how the conversation moved, not just a flat list of topics.

Return only valid JSON matching `sleep_topic_thread_pass_result.v1`.

Rules:

- `topic` and `subtopics` must be short machine-readable strings.
- `energy` describes the conversational energy shown by the events.
- `source_event_ids` must cite the events supporting this thread item.
- `summary` must be brief and human-readable.
- Generic informational Q&A can be summarized briefly; personally meaningful or emotionally marked threads must remain visible.
