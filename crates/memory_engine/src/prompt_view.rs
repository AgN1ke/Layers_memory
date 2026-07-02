use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime, UtcOffset};

use crate::core_store::{CoreContextEvent, CoreContextFact, CoreContextPackage, CoreFactStatus};
use crate::recall::RecallItem;

pub const ARCHIVE_MEMORY_PROMPT_LIMIT: usize = 5;
pub const OLDER_TRACE_LIMIT: usize = 20;
pub const OLDER_TRACE_MAX_TEXT_CHARS: usize = 180;
pub const RECENT_MAX_TEXT_CHARS: usize = 900;
pub const CORE_FACT_MAX_TEXT_CHARS: usize = 260;

/// Prompt-facing time context. Relative labels ("yesterday", "3 days ago")
/// are never stored; they are derived at render time from stored absolute
/// timestamps and the package `created_at`, so they can never go stale.
#[derive(Debug, Clone, Copy)]
pub struct TimeLabelContext {
    now: Option<OffsetDateTime>,
    offset: UtcOffset,
}

impl TimeLabelContext {
    pub fn new(now_rfc3339: &str, utc_offset_minutes: i32, clock_untrusted: bool) -> Self {
        let offset = UtcOffset::from_whole_seconds(utc_offset_minutes.saturating_mul(60))
            .unwrap_or(UtcOffset::UTC);
        let now = if clock_untrusted {
            None
        } else {
            OffsetDateTime::parse(now_rfc3339, &Rfc3339).ok()
        };
        Self { now, offset }
    }

    pub fn from_package(package: &CoreContextPackage) -> Self {
        Self::new(
            &package.created_at,
            package.utc_offset_minutes,
            package.clock_untrusted,
        )
    }

    pub fn disabled() -> Self {
        Self {
            now: None,
            offset: UtcOffset::UTC,
        }
    }

    fn local_now(&self) -> Option<OffsetDateTime> {
        Some(self.now?.to_offset(self.offset))
    }

    /// One line for `<state>`: the model's reference point for every
    /// relative label below it.
    fn current_time_line(&self) -> Option<String> {
        let local = self.local_now()?;
        Some(format!(
            "current_time: {:04}-{:02}-{:02} {:02}:{:02} {} ({})",
            local.year(),
            u8::from(local.month()),
            local.day(),
            local.hour(),
            local.minute(),
            local.weekday(),
            offset_label(self.offset),
        ))
    }

    /// Relative age label for a stored timestamp, or None when time is
    /// untrusted, unparseable, or in the future relative to `now`.
    fn age_label(&self, timestamp: &str) -> Option<String> {
        let local_now = self.local_now()?;
        let then = OffsetDateTime::parse(timestamp, &Rfc3339)
            .ok()?
            .to_offset(self.offset);
        if then > local_now {
            return None;
        }
        Some(bucket_label(then.date(), local_now.date()))
    }
}

fn bucket_label(then: Date, now: Date) -> String {
    let days = (now - then).whole_days();
    match days {
        days if days <= 0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=6 => format!("{days} days ago"),
        _ => {
            let months = i64::from(now.year()) * 12 + i64::from(u8::from(now.month()))
                - (i64::from(then.year()) * 12 + i64::from(u8::from(then.month())));
            match months {
                months if months <= 0 => "earlier this month".to_string(),
                1 => "last month".to_string(),
                2..=11 => format!("{months} months ago"),
                _ => "over a year ago".to_string(),
            }
        }
    }
}

fn offset_label(offset: UtcOffset) -> String {
    let total_minutes = offset.whole_seconds() / 60;
    if total_minutes == 0 {
        return "UTC".to_string();
    }
    let sign = if total_minutes > 0 { '+' } else { '-' };
    let hours = total_minutes.abs() / 60;
    let minutes = total_minutes.abs() % 60;
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}

pub fn render_memory_view(package: &CoreContextPackage, current_user_message: &str) -> String {
    let time = TimeLabelContext::from_package(package);
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
    if let Some(line) = time.current_time_line() {
        lines.push(line);
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
    let archive_lines = render_archive_memories(&package.archive_relevant, &time);
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
        lines.extend(render_dialogue_lines_with_day_markers(
            &older_trace,
            OLDER_TRACE_MAX_TEXT_CHARS,
            &time,
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
    timestamp: String,
    role: String,
    text: String,
}

fn normalized_context_events(events: &[CoreContextEvent]) -> Vec<DialogueEvent> {
    events
        .iter()
        .filter_map(|event| {
            let text = clean_string(event.text.as_deref()?);
            (!text.is_empty()).then(|| DialogueEvent {
                event_id: clean_string(&event.event_id).to_string(),
                timestamp: clean_string(&event.timestamp).to_string(),
                role: dialogue_role(event),
                text: text.to_string(),
            })
        })
        .collect()
}

/// Attributed transcript role: assistant events stay `assistant`; a user-side
/// event with a `speaker` renders under the speaker's name (multi-speaker
/// chats), otherwise under the legacy `user` role.
fn dialogue_role(event: &CoreContextEvent) -> String {
    if event.event_type == "assistant_message" {
        return "assistant".to_string();
    }
    event
        .speaker
        .as_ref()
        .map(|speaker| clean_string(&speaker.name))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "user".to_string())
}

fn drop_current_user_message(
    mut events: Vec<DialogueEvent>,
    current_user_message: &str,
) -> Vec<DialogueEvent> {
    let current = clean_string(current_user_message);
    if events
        .last()
        .is_some_and(|event| event.role != "assistant" && event.text == current)
    {
        events.pop();
    }
    events
}

fn render_core_facts(facts: &[CoreContextFact]) -> Vec<String> {
    facts
        .iter()
        .filter_map(render_core_fact_prompt_line)
        .collect()
}

fn render_archive_memories(archives: &[RecallItem], time: &TimeLabelContext) -> Vec<String> {
    archives
        .iter()
        .take(ARCHIVE_MEMORY_PROMPT_LIMIT)
        .flat_map(|archive| render_archive_memory_prompt_lines(archive, time))
        .collect()
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

/// Older-trace rendering with day markers: one `[yesterday]`-style line per
/// calendar-day group instead of a label on every line. Without a trusted
/// clock this renders exactly like `render_dialogue_lines`.
fn render_dialogue_lines_with_day_markers(
    events: &[DialogueEvent],
    max_text_chars: usize,
    time: &TimeLabelContext,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_marker: Option<String> = None;
    for event in events {
        let text = truncate_text(&event.text, max_text_chars);
        if text.is_empty() {
            continue;
        }
        if let Some(label) = time.age_label(&event.timestamp) {
            if current_marker.as_deref() != Some(label.as_str()) {
                lines.push(format!("[{label}]"));
                current_marker = Some(label);
            }
        }
        lines.push(format!("{}: {}", event.role, xml_escape(&text)));
    }
    lines
}

pub fn render_core_fact_prompt_line(fact: &CoreContextFact) -> Option<String> {
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
    let status_marker = match fact.status {
        CoreFactStatus::Active => String::new(),
        CoreFactStatus::Contested => " [contested]".to_string(),
        CoreFactStatus::Deprecated => " [deprecated]".to_string(),
        CoreFactStatus::Contradicted => " [contradicted]".to_string(),
        CoreFactStatus::NeedsReview => " [needs_review]".to_string(),
    };
    Some(format!(
        "- {category}{status_marker} ({confidence}): {}",
        xml_escape(&text)
    ))
}

pub fn render_archive_memory_prompt_lines(
    archive: &RecallItem,
    time: &TimeLabelContext,
) -> Vec<String> {
    let memory = archive
        .compact_memory
        .as_deref()
        .map(clean_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| clean_string(&archive.gist));
    if memory.is_empty() {
        return Vec::new();
    }

    let age = archive
        .time_range
        .as_ref()
        .and_then(|range| time.age_label(&range.end));
    let prefix = match age {
        Some(age) => format!("- [{age} | {}] ", format_score(archive.relevance_score)),
        None => format!("- [{}] ", format_score(archive.relevance_score)),
    };
    let mut lines = Vec::new();
    for (index, memory_line) in memory.lines().map(str::trim).enumerate() {
        if memory_line.is_empty() {
            continue;
        }
        let line_prefix = if index == 0 { prefix.as_str() } else { "  " };
        lines.push(format!("{line_prefix}{}", xml_escape(memory_line)));
    }
    lines
}

pub fn render_context_event_prompt_line(
    event: &CoreContextEvent,
    max_text_chars: usize,
) -> Option<String> {
    let text = clean_string(event.text.as_deref()?);
    if text.is_empty() {
        return None;
    }
    let role = dialogue_role(event);
    Some(format!(
        "{role}: {}",
        xml_escape(&truncate_text(text, max_text_chars))
    ))
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
    use super::{render_memory_view, TimeLabelContext};
    use crate::core_store::{
        CoreContextEvent, CoreContextFact, CoreContextPackage, CoreFactStatus,
    };
    use crate::recall::{RecallItem, RecallSourceLayer};
    use crate::types::TimeRange;

    #[test]
    fn memory_view_keeps_layers_separate_and_drops_current_duplicate() {
        let package = CoreContextPackage {
            schema_version: "core_context_package.v1".to_string(),
            created_at: "2026-05-30T10:00:00Z".to_string(),
            utc_offset_minutes: 0,
            clock_untrusted: false,
            core_facts: vec![CoreContextFact {
                category: "profile".to_string(),
                core_fact_id: "core_fact_1".to_string(),
                scope: Some("chat_1".to_string()),
                text: "User name is Mykyta.".to_string(),
                status: CoreFactStatus::Active,
                confidence: 0.95,
                tags: vec![],
            }],
            session_recent: vec![
                CoreContextEvent {
                    event_id: "event_1".to_string(),
                    timestamp: "2026-05-30T10:00:00Z".to_string(),
                    event_type: "assistant_message".to_string(),
                    source: "bot".to_string(),
                    speaker: None,
                    text: Some("We talked about the cat Irzha.".to_string()),
                    tags: vec![],
                    theme: None,
                },
                CoreContextEvent {
                    event_id: "event_2".to_string(),
                    timestamp: "2026-05-30T10:01:00Z".to_string(),
                    event_type: "user_message".to_string(),
                    source: "user".to_string(),
                    speaker: None,
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
        assert!(view.contains("current_time: 2026-05-30 10:00 Saturday (UTC)"));
    }

    fn labeled_package(created_at: &str, clock_untrusted: bool) -> CoreContextPackage {
        CoreContextPackage {
            schema_version: "core_context_package.v1".to_string(),
            created_at: created_at.to_string(),
            utc_offset_minutes: 0,
            clock_untrusted,
            core_facts: vec![],
            session_recent: vec![CoreContextEvent {
                event_id: "event_recent".to_string(),
                timestamp: created_at.to_string(),
                event_type: "user_message".to_string(),
                source: "user".to_string(),
                speaker: None,
                text: Some("Fresh line.".to_string()),
                tags: vec![],
                theme: None,
            }],
            session_trace: vec![
                CoreContextEvent {
                    event_id: "event_old".to_string(),
                    timestamp: "2026-07-01T09:00:00Z".to_string(),
                    event_type: "user_message".to_string(),
                    source: "user".to_string(),
                    speaker: None,
                    text: Some("Older line.".to_string()),
                    tags: vec![],
                    theme: None,
                },
                CoreContextEvent {
                    event_id: "event_recent".to_string(),
                    timestamp: created_at.to_string(),
                    event_type: "user_message".to_string(),
                    source: "user".to_string(),
                    speaker: None,
                    text: Some("Fresh line.".to_string()),
                    tags: vec![],
                    theme: None,
                },
            ],
            archive_relevant: vec![RecallItem {
                source_layer: RecallSourceLayer::Archive,
                id: "archive_1".to_string(),
                gist: "Motorcycle purchase discussion.".to_string(),
                compact_memory: Some("Zheka -> bought a motorcycle.".to_string()),
                narrative: None,
                facts: vec![],
                quotes: vec![],
                source_session_id: Some("chat_1".to_string()),
                time_range: Some(TimeRange {
                    start: "2026-07-01T08:00:00Z".to_string(),
                    end: "2026-07-01T09:30:00Z".to_string(),
                }),
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
        }
    }

    #[test]
    fn memory_view_labels_archive_age_and_marks_older_dialogue_days() {
        let view = render_memory_view(&labeled_package("2026-07-02T10:00:00Z", false), "Next?");

        assert!(view.contains("current_time: 2026-07-02 10:00 Thursday (UTC)"));
        assert!(view.contains("- [yesterday | 0.88] Zheka -&gt; bought a motorcycle."));
        assert!(view.contains("[yesterday]\nuser: Older line."));
    }

    #[test]
    fn memory_view_relabels_same_package_when_rendered_later() {
        let view = render_memory_view(&labeled_package("2026-07-09T10:00:00Z", false), "Next?");

        assert!(view.contains("- [earlier this month | 0.88] Zheka -&gt; bought a motorcycle."));
    }

    #[test]
    fn memory_view_omits_labels_when_clock_is_untrusted() {
        let view = render_memory_view(&labeled_package("2026-07-02T10:00:00Z", true), "Next?");

        assert!(!view.contains("current_time:"));
        assert!(view.contains("- [0.88] Zheka -&gt; bought a motorcycle."));
        assert!(!view.contains("[yesterday]"));
    }

    #[test]
    fn age_labels_follow_calendar_buckets_and_local_offset() {
        let utc = TimeLabelContext::new("2026-07-02T10:00:00Z", 0, false);
        assert_eq!(
            utc.age_label("2026-07-02T00:30:00Z").as_deref(),
            Some("today")
        );
        assert_eq!(
            utc.age_label("2026-07-01T23:59:00Z").as_deref(),
            Some("yesterday")
        );
        assert_eq!(
            utc.age_label("2026-06-30T10:00:00Z").as_deref(),
            Some("2 days ago")
        );
        assert_eq!(
            utc.age_label("2026-06-26T10:00:00Z").as_deref(),
            Some("6 days ago")
        );
        assert_eq!(
            utc.age_label("2026-06-25T10:00:00Z").as_deref(),
            Some("last month")
        );
        assert_eq!(utc.age_label("2026-07-03T10:00:00Z"), None);

        let mid_month = TimeLabelContext::new("2026-07-20T10:00:00Z", 0, false);
        assert_eq!(
            mid_month.age_label("2026-07-10T10:00:00Z").as_deref(),
            Some("earlier this month")
        );
        assert_eq!(
            mid_month.age_label("2026-03-10T10:00:00Z").as_deref(),
            Some("4 months ago")
        );
        assert_eq!(
            mid_month.age_label("2025-05-10T10:00:00Z").as_deref(),
            Some("over a year ago")
        );

        let kyiv = TimeLabelContext::new("2026-07-01T22:30:00Z", 180, false);
        assert_eq!(
            kyiv.age_label("2026-07-01T20:00:00Z").as_deref(),
            Some("yesterday"),
            "local midnight already passed in UTC+3"
        );
        let utc_same_pair = TimeLabelContext::new("2026-07-01T22:30:00Z", 0, false);
        assert_eq!(
            utc_same_pair.age_label("2026-07-01T20:00:00Z").as_deref(),
            Some("today")
        );
    }

    #[test]
    fn memory_view_attributes_speakers_and_drops_current_speaker_duplicate() {
        use crate::types::Speaker;

        let mut package = labeled_package("2026-07-02T10:00:00Z", false);
        package.archive_relevant = vec![];
        package.session_trace = vec![];
        package.session_recent = vec![
            CoreContextEvent {
                event_id: "event_zheka".to_string(),
                timestamp: "2026-07-02T09:58:00Z".to_string(),
                event_type: "user_message".to_string(),
                source: "group_chat".to_string(),
                speaker: Some(Speaker {
                    id: "tg_101".to_string(),
                    name: "Жека".to_string(),
                }),
                text: Some("Купив мотоцикл!".to_string()),
                tags: vec![],
                theme: None,
            },
            CoreContextEvent {
                event_id: "event_bot".to_string(),
                timestamp: "2026-07-02T09:59:00Z".to_string(),
                event_type: "assistant_message".to_string(),
                source: "bot".to_string(),
                speaker: None,
                text: Some("Вітаю з покупкою!".to_string()),
                tags: vec![],
                theme: None,
            },
            CoreContextEvent {
                event_id: "event_anton".to_string(),
                timestamp: "2026-07-02T10:00:00Z".to_string(),
                event_type: "user_message".to_string(),
                source: "group_chat".to_string(),
                speaker: Some(Speaker {
                    id: "tg_202".to_string(),
                    name: "Антон".to_string(),
                }),
                text: Some("А клюло сьогодні на світанку?".to_string()),
                tags: vec![],
                theme: None,
            },
        ];

        let view = render_memory_view(&package, "А клюло сьогодні на світанку?");

        assert!(view.contains("Жека: Купив мотоцикл!"));
        assert!(view.contains("assistant: Вітаю з покупкою!"));
        assert!(
            !view.contains("Антон: А клюло сьогодні на світанку?"),
            "current speaker message must not be duplicated into short memory"
        );
        assert!(view.contains(
            "<current_user_message>\nА клюло сьогодні на світанку?\n</current_user_message>"
        ));
    }
}
