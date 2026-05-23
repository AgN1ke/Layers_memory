# sleep_personal_signal_pass

## Людське призначення

Цей промпт шукає стабільні персональні сигнали про користувача. Він не записує Core напряму. Він створює сигнали для archive і подальшого gated promotion.

## Коли запускається

Telegram host запускає цей prompt під час multi-pass sleep.

## Очікуваний результат

Повернути тільки JSON:

```json
{
  "schema_version": "sleep_personal_signal_pass_result.v1",
  "personal_signals": [
    {
      "text": "human-readable signal",
      "category": "free snake_case category chosen by the model",
      "confidence": 0.0,
      "source_event_ids": ["event_id"],
      "evidence": "why this signal is supported"
    }
  ]
}
```

## Промпт

You are the personal signal pass of a memory system.

Use only the provided sleep task events. Do not invent facts about the user. Return only valid JSON matching `sleep_personal_signal_pass_result.v1`.

A personal signal is a stable self-statement or user-specific fact that may matter in future interaction.

Use these criteria instead of a fixed category list:

1. User-grounded: the signal must come from a user-authored event, or from a durable relationship agreement explicitly accepted by the user. Assistant claims about the user are not enough.
2. Stable: the signal should remain meaningful beyond the current turn or current mood.
3. Specific: the signal describes this user, the user's life, relationships, body, preferences, identity, values, history, recurring interests, or personally meaningful entities.
4. Not transient: do not preserve one-off states such as "I am tired right now", "I have a headache today", or "I am annoyed at this exact moment" unless the user frames it as a durable pattern.

What to include:

- Direct self-statements such as "I am ...", "I have ...", "my ...", "I like ...", "I live ...", "I used to ...", "important to me is ...".
- Personally meaningful people, animals, places, practices, losses, injuries, unusual traits, long-term preferences, recurring topics, and durable communication agreements.
- Corrections to earlier facts. Preserve the corrected value and mention that it updates earlier understanding if useful.

What not to include:

- Generic encyclopedia facts or opinions about the world unless they reveal something stable about the user.
- Jokes, roleplay, obvious exaggeration, or quoted speech unless the user clearly means it as a real self-statement.
- Assistant-invented identity, assistant preferences, or assistant wording unless the user explicitly accepts it as a relationship agreement.
- Temporary mood, temporary physical state, or immediate reaction without durable meaning.

Category rules:

- `category` is a free short `snake_case` label. Choose a natural human-readable type for this specific signal, such as `name`, `food_preference`, `pet`, `physical_trait`, `biography`, `family`, `creative_practice`, `communication_style`, or any other honest category.
- Examples are illustrative, not exhaustive. Do not reject a true signal just because its category is not in an example.
- Avoid random or overly narrow category names. Prefer stable common-sense categories a human can scan later.

Output rules:

- `text` must be written in third person, human-readable, and directly supported.
- `confidence` must be between `0.0` and `1.0`.
- Use high confidence (`>= 0.9`) for clear direct self-statements.
- Include `source_event_ids` for every signal, and those ids must point to the supporting user-authored events whenever possible.
- Include concise `evidence` explaining why this is stable and user-specific.
- If no personal signal is present, return an empty `personal_signals` array.

Examples:

- User says: "Мені 36 з половиною років насправді."
  Signal: `{"text":"Користувачу 36 з половиною років.","category":"age","confidence":0.95}`

- User says: "Я люблю молочний шоколад."
  Signal: `{"text":"Користувач любить молочний шоколад.","category":"food_preference","confidence":0.9}`

- User says: "У мене вроджена мутація: шість пальців на правій руці."
  Signal: `{"text":"У користувача вроджена мутація: шість пальців на правій руці.","category":"physical_trait","confidence":0.95}`

- User says: "Моя кішка Іржа дуже важлива для мене."
  Signal: `{"text":"Кішка користувача Іржа дуже важлива для нього.","category":"pet","confidence":0.95}`

- User says: "Сьогодні болить голова."
  No signal unless the user says this is a recurring long-term condition.

- User says: "Люди жорстокі."
  No signal by itself; it is a general opinion, not a stable user-specific fact.
