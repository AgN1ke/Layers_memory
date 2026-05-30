use crate::core_store::{CoreContextEvent, CoreContextFact, CoreContextPackage};
use crate::recall::RecallItem;

const OLDER_TRACE_LIMIT: usize = 20;
const OLDER_TRACE_MAX_TEXT_CHARS: usize = 180;
const RECENT_MAX_TEXT_CHARS: usize = 900;
const CORE_FACT_MAX_TEXT_CHARS: usize = 260;

pub fn render_memory_view(package: &CoreContextPackage, current_user_message: &str) -> String {
    let recent_events = normalized_context_events(&package.session_recent);
    let trace_events = normalized_context_events(&package.session_trace);
    let prior_recent = drop_current_user_message(recent_events.clone(), current_user_message);
    let recent_ids = recent_events
        .iter()
        .filter(|event| !event.event_id.is_empty())
        .map(|event| event.event_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let older_trace = trace_events
        .into_iter()
        .filter(|event| !event.event_id.is_empty() && !recent_ids.contains(event.event_id.as_str()))
        .collect::<Vec<_>>();
    let older_trace = tail(older_trace, OLDER_TRACE_LIMIT);

    let mut lines = vec!["<memory_context>".to_string()];
    lines.push("<state>".to_string());
    lines.push(format!(
        "conversation_state: {}",
        if prior_recent.is_empty() {
            "new_or_no_recent_context"
        } else {
            "ongoing"
        }
    ));
    if prior_recent.is_empty() {
        lines.push(
            "instruction: No prior active dialogue is visible; a short greeting is allowed if natural."
                .to_string(),
        );
    } else {
        lines.push(
            "instruction: Continue the dialogue from the latest turn. Do not greet unless the current user message is a greeting."
                .to_string(),
        );
    }
    lines.push("</state>".to_string());

    lines.push(String::new());
    lines.push("<core_memory>".to_string());
    let core_lines = render_core_facts(&package.core_facts);
    if core_lines.is_empty() {
        lines.push("(empty)".to_string());
    } else {
        lines.extend(core_lines);
    }
    lines.push("</core_memory>".to_string());

    lines.push(String::new());
    lines.push("<long_memory>".to_string());
    let archive_lines = render_archive_memories(&package.archive_relevant);
    if archive_lines.is_empty() {
        lines.push("(empty)".to_string());
    } else {
        lines.extend(archive_lines);
    }
    lines.push("</long_memory>".to_string());

    lines.push(String::new());
    lines.push("<short_memory>".to_string());
    if !older_trace.is_empty() {
        lines.push("<older_active_dialogue>".to_string());
        lines.extend(render_dialogue_lines(
            &older_trace,
            OLDER_TRACE_MAX_TEXT_CHARS,
        ));
        lines.push("</older_active_dialogue>".to_string());
    }
    if prior_recent.is_empty() {
        lines.push("(empty)".to_string());
    } else {
        lines.push("<recent_dialogue>".to_string());
        lines.extend(render_dialogue_lines(&prior_recent, RECENT_MAX_TEXT_CHARS));
        lines.push("</recent_dialogue>".to_string());
    }
    lines.push("</short_memory>".to_string());

    lines.push(String::new());
    lines.push("<current_user_message>".to_string());
    lines.push(xml_escape(clean_string(current_user_message)));
    lines.push("</current_user_message>".to_string());
    lines.push(String::new());
    lines.push("<assistant_response_slot>".to_string());
    lines.push("Write only the assistant reply for the current user message.".to_string());
    lines.push("</assistant_response_slot>".to_string());
    lines.push("</memory_context>".to_string());

    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DialogueEvent {
    event_id: String,
    role: &'static str,
    text: String,
}

fn normalized_context_events(events: &[CoreContextEvent]) -> Vec<DialogueEvent> {
    events
        .iter()
        .filter_map(|event| {
            let text = clean_string(event.text.as_deref()?);
            (!text.is_empty()).then(|| DialogueEvent {
                event_id: clean_string(&event.event_id).to_string(),
                role: if event.event_type == "assistant_message" {
                    "assistant"
                } else {
                    "user"
                },
                text: text.to_string(),
            })
        })
        .collect()
}

fn drop_current_user_message(
    mut events: Vec<DialogueEvent>,
    current_user_message: &str,
) -> Vec<DialogueEvent> {
    let current = clean_string(current_user_message);
    if events
        .last()
        .is_some_and(|event| event.role == "user" && event.text == current)
    {
        events.pop();
    }
    events
}

fn render_core_facts(facts: &[CoreContextFact]) -> Vec<String> {
    facts
        .iter()
        .filter_map(|fact| {
            let text = truncate_text(clean_string(&fact.text), CORE_FACT_MAX_TEXT_CHARS);
            if text.is_empty() {
                return None;
            }
            let category = if clean_string(&fact.category).is_empty() {
                "core"
            } else {
                clean_string(&fact.category)
            };
            let confidence = format_score(fact.confidence);
            Some(format!(
                "- {category} ({confidence}): {}",
                xml_escape(&text)
            ))
        })
        .collect()
}

fn render_archive_memories(archives: &[RecallItem]) -> Vec<String> {
    let mut lines = Vec::new();
    for archive in archives.iter().take(5) {
        let memory = archive
            .compact_memory
            .as_deref()
            .map(clean_string)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| clean_string(&archive.gist));
        if memory.is_empty() {
            continue;
        }
        let prefix = format!("- [{}] ", format_score(archive.relevance_score));
        for (index, memory_line) in memory.lines().map(str::trim).enumerate() {
            if memory_line.is_empty() {
                continue;
            }
            let line_prefix = if index == 0 { prefix.as_str() } else { "  " };
            lines.push(format!("{line_prefix}{}", xml_escape(memory_line)));
        }
    }
    lines
}

fn render_dialogue_lines(events: &[DialogueEvent], max_text_chars: usize) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| {
            let text = truncate_text(&event.text, max_text_chars);
            (!text.is_empty()).then(|| format!("{}: {}", event.role, xml_escape(&text)))
        })
        .collect()
}

fn tail<T>(mut items: Vec<T>, limit: usize) -> Vec<T> {
    if items.len() <= limit {
        return items;
    }
    let start = items.len() - limit;
    items.drain(0..start);
    items
}

fn clean_string(value: &str) -> &str {
    value.trim()
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let cleaned = clean_string(text);
    if cleaned.chars().count() <= max_chars {
        return cleaned.to_string();
    }
    let mut truncated = cleaned
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated = truncated.trim_end().to_string();
    truncated.push_str("...");
    truncated
}

fn format_score(value: f64) -> String {
    let clamped = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let rounded = (clamped * 100.0).round() / 100.0;
    let mut formatted = format!("{rounded:.2}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.push('0');
    }
    formatted
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::render_memory_view;
    use crate::core_store::{CoreContextEvent, CoreContextFact, CoreContextPackage};
    use crate::recall::{RecallItem, RecallSourceLayer};

    #[test]
    fn memory_view_keeps_layers_separate_and_drops_current_duplicate() {
        let package = CoreContextPackage {
            schema_version: "core_context_package.v1".to_string(),
            created_at: "2026-05-30T10:00:00Z".to_string(),
            core_facts: vec![CoreContextFact {
                category: "profile".to_string(),
                core_fact_id: "core_fact_1".to_string(),
                scope: Some("chat_1".to_string()),
                text: "User name is Mykyta.".to_string(),
                confidence: 0.95,
                tags: vec![],
            }],
            session_recent: vec![
                CoreContextEvent {
                    event_id: "event_1".to_string(),
                    timestamp: "2026-05-30T10:00:00Z".to_string(),
                    event_type: "assistant_message".to_string(),
                    source: "bot".to_string(),
                    text: Some("We talked about the cat Irzha.".to_string()),
                    tags: vec![],
                    theme: None,
                },
                CoreContextEvent {
                    event_id: "event_2".to_string(),
                    timestamp: "2026-05-30T10:01:00Z".to_string(),
                    event_type: "user_message".to_string(),
                    source: "user".to_string(),
                    text: Some("Do you remember Irzha?".to_string()),
                    tags: vec![],
                    theme: None,
                },
            ],
            session_trace: vec![],
            archive_relevant: vec![RecallItem {
                source_layer: RecallSourceLayer::Archive,
                id: "archive_1".to_string(),
                gist: "The cat Irzha was an important topic.".to_string(),
                compact_memory: Some("Irzha -> warm personal memory.".to_string()),
                narrative: None,
                facts: vec![],
                quotes: vec![],
                source_session_id: Some("chat_1".to_string()),
                time_range: None,
                tags: vec![],
                theme: None,
                weight: 0.9,
                freshness: 1.0,
                relevance_score: 0.876,
                relevance_explanation: None,
            }],
            domain_state: serde_json::json!({}),
            budget: None,
            notes: vec![],
        };

        let view = render_memory_view(&package, "Do you remember Irzha?");

        assert!(view.contains("<core_memory>"));
        assert!(view.contains("- profile (0.95): User name is Mykyta."));
        assert!(view.contains("<long_memory>"));
        assert!(view.contains("- [0.88] Irzha -&gt; warm personal memory."));
        assert!(view.contains("<short_memory>"));
        assert!(view.contains("assistant: We talked about the cat Irzha."));
        assert!(!view.contains("user: Do you remember Irzha?"));
        assert!(view
            .contains("<current_user_message>\nDo you remember Irzha?\n</current_user_message>"));
    }
}
