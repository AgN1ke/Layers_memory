use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::archive::{ArchiveEntry, ArchiveFilters, ArchiveStatus};
use crate::event::{IngestEvent, StoredEvent};
use crate::recall::{
    RecallDebug, RecallFilters, RecallItem, RecallQuery, RecallResult, RecallSourceLayer,
};
use crate::session::SessionRecord;
use crate::sleep::SleepCompressionResult;
use crate::storage::Storage;
use crate::tasks::{PendingTask, TaskState, TaskType};
use crate::types::{
    ImportanceHint, ModelRole, Quote, RecallStage, TimeRange, WeightedFact,
    ARCHIVE_ENTRY_SCHEMA_VERSION, EVENT_SCHEMA_VERSION, PENDING_TASK_SCHEMA_VERSION,
    RECALL_QUERY_SCHEMA_VERSION, RECALL_RESULT_SCHEMA_VERSION,
    SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct MemoryEngine<S> {
    storage: S,
    options: EngineOptions,
}

impl<S> MemoryEngine<S> {
    pub fn new(storage: S) -> Self {
        Self::with_options(storage, EngineOptions::default())
    }

    pub fn with_options(storage: S, options: EngineOptions) -> Self {
        Self { storage, options }
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    pub fn into_storage(self) -> S {
        self.storage
    }
}

impl<S: Storage> MemoryEngine<S> {
    pub fn ingest(&mut self, event: IngestEvent) -> Result<StoredEvent> {
        validate_ingest_event(&event)?;

        let (initial_weight, weight_reason) = self.options.event_scoring.score_ingest_event(&event);
        let stored = StoredEvent::from_ingest(
            event,
            new_id("event")?,
            now_rfc3339()?,
            initial_weight,
            weight_reason,
        );

        self.storage.append_event(&stored.session_id, &stored)?;
        Ok(stored)
    }

    pub fn pending_tasks(&self) -> Result<Vec<PendingTask>> {
        Ok(self
            .storage
            .load_tasks()?
            .into_iter()
            .filter(|task| matches!(task.state, TaskState::Pending | TaskState::Submitted))
            .collect())
    }

    pub fn sleep(&mut self, session_id: &str) -> Result<SleepStage1Result> {
        if session_id.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "sleep session_id must not be empty".to_string(),
            ));
        }

        let session = self.storage.read_session(session_id)?;
        if session.events.is_empty() {
            return Err(MemoryEngineError::Validation(format!(
                "session has no events: {session_id}"
            )));
        }

        let selected_events = select_sleep_events(&session, &self.options.sleep);
        let now = now_rfc3339()?;
        let archive_id = new_id("archive")?;
        let archive_entry =
            build_preliminary_archive(&session, &selected_events, &archive_id, &now);
        self.storage.write_archive_entry(&archive_entry)?;

        let pending_task = build_sleep_compression_task(
            &session,
            &selected_events,
            &archive_entry,
            &self.options.sleep,
            &now,
        )?;
        self.storage.save_task(&pending_task)?;

        Ok(SleepStage1Result {
            archive_entry,
            pending_task,
        })
    }

    pub fn resume_sleep_compression(
        &mut self,
        task_id: &str,
        result: SleepCompressionResult,
    ) -> Result<ArchiveEntry> {
        if result.schema_version != SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        result.validate_basic()?;

        let mut task = self
            .storage
            .load_tasks()?
            .into_iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| MemoryEngineError::TaskNotFound(task_id.to_string()))?;

        if task.task_type != TaskType::SleepCompression {
            return Err(MemoryEngineError::Validation(format!(
                "task is not sleep_compression: {task_id}"
            )));
        }

        let mut archive_entry = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_iter()
            .find(|entry| entry.archive_id == result.archive_id)
            .ok_or_else(|| {
                MemoryEngineError::Storage(format!(
                    "archive entry not found: {}",
                    result.archive_id
                ))
            })?;

        let now = now_rfc3339()?;
        archive_entry.updated_at = now.clone();
        archive_entry.theme = result.theme;
        archive_entry.tags = result.tags;
        archive_entry.gist = result.gist;
        archive_entry.narrative = result.narrative;
        archive_entry.facts = result.facts;
        archive_entry.quotes = result.quotes;
        archive_entry.weight = result.weight;
        archive_entry.links = result.links;
        archive_entry.status = ArchiveStatus::Complete;
        archive_entry.llm_enhanced = true;
        archive_entry.prompt_id = Some(task.prompt_id.clone());
        archive_entry.prompt_version = Some(task.prompt_version);

        self.storage
            .update_archive_entry(&archive_entry.archive_id, &archive_entry)?;

        task.state = TaskState::Completed;
        task.updated_at = now;
        task.last_error = None;
        self.storage.save_task(&task)?;

        Ok(archive_entry)
    }

    pub fn recall(&mut self, query: RecallQuery) -> Result<RecallResult> {
        validate_recall_query(&query)?;

        let created_at = query.created_at.clone().map_or_else(now_rfc3339, Ok)?;
        let archive_enabled = query.filters.source_layers.is_empty()
            || query
                .filters
                .source_layers
                .contains(&RecallSourceLayer::Archive);

        let mut archive_entries = if archive_enabled {
            self.storage
                .read_archive(&archive_filters_from_recall(&query.filters))?
        } else {
            Vec::new()
        };

        let candidate_count = archive_entries.len();
        let mut scored_entries = archive_entries
            .drain(..)
            .map(|entry| {
                let scored = score_archive_entry(&entry, &query, &self.options.recall);
                (entry, scored)
            })
            .collect::<Vec<_>>();

        scored_entries.sort_by(|(left_entry, left_score), (right_entry, right_score)| {
            right_score
                .score
                .total_cmp(&left_score.score)
                .then_with(|| right_entry.weight.total_cmp(&left_entry.weight))
                .then_with(|| left_entry.archive_id.cmp(&right_entry.archive_id))
        });

        let limit = if query.limit == 0 {
            self.options.recall.default_limit
        } else {
            query.limit
        };

        let selected_entries = scored_entries.into_iter().take(limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(selected_entries.len());

        for (mut entry, score) in selected_entries {
            entry.recall_count += 1;
            entry.last_recalled_at = Some(created_at.clone());
            self.storage
                .update_archive_entry(&entry.archive_id, &entry)?;
            items.push(recall_item_from_archive(entry, score, query.explain));
        }

        Ok(RecallResult {
            schema_version: RECALL_RESULT_SCHEMA_VERSION.to_string(),
            query_id: query.query_id,
            created_at,
            stage_used: RecallStage::Stage1,
            items,
            debug: query.explain.then_some(RecallDebug {
                candidate_count,
                filtered_count: candidate_count,
            }),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineOptions {
    pub event_scoring: EventScoringConfig,
    pub sleep: SleepStage1Config,
    pub recall: RecallStage1Config,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventScoringConfig {
    pub base_weight: f64,
    pub tag_bonus: f64,
    pub theme_bonus: f64,
    pub link_bonus: f64,
    pub medium_floor: f64,
    pub high_floor: f64,
    pub critical_floor: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SleepStage1Config {
    pub min_event_weight: f64,
    pub max_events: usize,
    pub prompt_id: String,
    pub prompt_version: u32,
}

impl Default for SleepStage1Config {
    fn default() -> Self {
        Self {
            min_event_weight: 0.55,
            max_events: 24,
            prompt_id: "sleep_compression".to_string(),
            prompt_version: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecallStage1Config {
    pub default_limit: usize,
    pub theme_match_factor: f64,
    pub tag_overlap_bonus: f64,
    pub text_match_bonus: f64,
    pub no_text_match_factor: f64,
}

impl Default for RecallStage1Config {
    fn default() -> Self {
        Self {
            default_limit: 5,
            theme_match_factor: 1.2,
            tag_overlap_bonus: 0.1,
            text_match_bonus: 0.5,
            no_text_match_factor: 0.7,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SleepStage1Result {
    pub archive_entry: ArchiveEntry,
    pub pending_task: PendingTask,
}

impl Default for EventScoringConfig {
    fn default() -> Self {
        Self {
            base_weight: 0.4,
            tag_bonus: 0.05,
            theme_bonus: 0.1,
            link_bonus: 0.05,
            medium_floor: 0.55,
            high_floor: 0.75,
            critical_floor: 0.95,
        }
    }
}

impl EventScoringConfig {
    pub fn score_ingest_event(&self, event: &IngestEvent) -> (f64, String) {
        let mut weight = self.base_weight;
        let mut reasons = vec![format!("base {:.2}", self.base_weight)];

        if !event.tags.is_empty() {
            let tag_bonus = self.tag_bonus * event.tags.len() as f64;
            weight += tag_bonus;
            reasons.push(format!("{} tag(s) +{tag_bonus:.2}", event.tags.len()));
        }

        if event.theme.is_some() {
            weight += self.theme_bonus;
            reasons.push(format!("theme +{:.2}", self.theme_bonus));
        }

        if !event.links.is_empty() {
            let link_bonus = self.link_bonus * event.links.len() as f64;
            weight += link_bonus;
            reasons.push(format!("{} link(s) +{link_bonus:.2}", event.links.len()));
        }

        let floor = match event.importance_hint {
            ImportanceHint::Low | ImportanceHint::Normal => None,
            ImportanceHint::Medium => Some(("medium importance floor", self.medium_floor)),
            ImportanceHint::High => Some(("high importance floor", self.high_floor)),
            ImportanceHint::Critical => Some(("critical importance floor", self.critical_floor)),
        };

        if let Some((label, floor)) = floor {
            if weight < floor {
                weight = floor;
                reasons.push(format!("{label} {floor:.2}"));
            }
        }

        (weight.clamp(0.0, 1.0), reasons.join("; "))
    }
}

fn validate_ingest_event(event: &IngestEvent) -> Result<()> {
    if event.schema_version != EVENT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: EVENT_SCHEMA_VERSION.to_string(),
            actual: event.schema_version.clone(),
        });
    }

    if event.event_type.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event type must not be empty".to_string(),
        ));
    }

    if event.source.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event source must not be empty".to_string(),
        ));
    }

    if event.timestamp.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event timestamp must not be empty".to_string(),
        ));
    }

    if event.session_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "event session_id must not be empty".to_string(),
        ));
    }

    Ok(())
}

fn validate_recall_query(query: &RecallQuery) -> Result<()> {
    if query.schema_version != RECALL_QUERY_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            actual: query.schema_version.clone(),
        });
    }

    Ok(())
}

fn select_sleep_events<'a>(
    session: &'a SessionRecord,
    config: &SleepStage1Config,
) -> Vec<&'a StoredEvent> {
    let mut selected = session
        .events
        .iter()
        .filter(|event| {
            event.initial_weight >= config.min_event_weight
                || matches!(
                    event.importance_hint,
                    ImportanceHint::High | ImportanceHint::Critical
                )
        })
        .collect::<Vec<_>>();

    if selected.is_empty() {
        selected = session.events.iter().collect();
    }

    selected.sort_by(|left, right| {
        right
            .initial_weight
            .total_cmp(&left.initial_weight)
            .then_with(|| left.timestamp.cmp(&right.timestamp))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    selected.truncate(config.max_events);
    selected.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    selected
}

fn build_preliminary_archive(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_id: &str,
    now: &str,
) -> ArchiveEntry {
    let tags = collect_tags(events);
    let theme = session
        .metadata
        .active_theme
        .clone()
        .or_else(|| events.iter().find_map(|event| event.theme.clone()));
    let quotes = events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| Quote {
                text: truncate_chars(&text, 240),
                source_event_id: Some(event.event_id.clone()),
            })
        })
        .collect::<Vec<_>>();

    ArchiveEntry {
        schema_version: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
        archive_id: archive_id.to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        source_session_id: session.metadata.session_id.clone(),
        source_event_ids: events.iter().map(|event| event.event_id.clone()).collect(),
        time_range: time_range_from_events(events),
        theme,
        tags,
        gist: preliminary_gist(events),
        narrative: preliminary_narrative(&session.metadata.session_id, events),
        facts: preliminary_facts(events),
        quotes,
        weight: archive_weight(events),
        freshness: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        links: events
            .iter()
            .flat_map(|event| event.links.iter().cloned())
            .collect(),
        status: ArchiveStatus::Preliminary,
        llm_enhanced: false,
        prompt_id: None,
        prompt_version: None,
        embedding_model_id: None,
        embedding: None,
    }
}

fn build_sleep_compression_task(
    session: &SessionRecord,
    events: &[&StoredEvent],
    archive_entry: &ArchiveEntry,
    config: &SleepStage1Config,
    now: &str,
) -> Result<PendingTask> {
    Ok(PendingTask {
        schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
        task_id: new_id("task")?,
        task_type: TaskType::SleepCompression,
        state: TaskState::Pending,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        prompt_id: config.prompt_id.clone(),
        prompt_version: config.prompt_version,
        role_hint: ModelRole::Balanced,
        expected_output_schema: SLEEP_COMPRESSION_RESULT_SCHEMA_VERSION.to_string(),
        inputs: json!({
            "session_id": &session.metadata.session_id,
            "preliminary_archive_id": &archive_entry.archive_id,
            "events": events.iter().map(|event| json!({
                "event_id": &event.event_id,
                "type": &event.event_type,
                "timestamp": &event.timestamp,
                "payload": &event.payload,
                "tags": &event.tags,
                "theme": &event.theme,
                "initial_weight": event.initial_weight,
                "weight_reason": &event.weight_reason,
            })).collect::<Vec<Value>>(),
            "hints": {
                "target_style": "compact_human_readable_memory",
                "preserve_quotes": true,
                "do_not_invent_facts": true
            }
        }),
        attempts: Vec::new(),
        last_error: None,
    })
}

fn archive_filters_from_recall(filters: &RecallFilters) -> ArchiveFilters {
    ArchiveFilters {
        time_range: filters.time_range.clone(),
        tags: filters.tags.clone(),
        theme: filters.theme.clone(),
        min_weight: filters.min_weight,
        min_freshness: filters.min_freshness,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ScoredArchiveEntry {
    score: f64,
    explanation: String,
}

fn score_archive_entry(
    entry: &ArchiveEntry,
    query: &RecallQuery,
    config: &RecallStage1Config,
) -> ScoredArchiveEntry {
    let theme_factor = if query.filters.theme.is_some() && query.filters.theme == entry.theme {
        config.theme_match_factor
    } else {
        1.0
    };

    let tag_overlap = query
        .filters
        .tags
        .iter()
        .filter(|tag| entry.tags.iter().any(|entry_tag| entry_tag == *tag))
        .count();
    let tag_factor = 1.0 + (tag_overlap as f64 * config.tag_overlap_bonus);

    let query_tokens = query_tokens(query);
    let searchable_tokens = archive_tokens(entry);
    let text_overlap = query_tokens
        .iter()
        .filter(|token| searchable_tokens.contains(*token))
        .count();
    let text_factor = if query_tokens.is_empty() {
        1.0
    } else if text_overlap == 0 {
        config.no_text_match_factor
    } else {
        1.0 + ((text_overlap as f64 / query_tokens.len() as f64) * config.text_match_bonus)
    };

    let score =
        (entry.weight * entry.freshness * theme_factor * tag_factor * text_factor).clamp(0.0, 1.0);

    ScoredArchiveEntry {
        score,
        explanation: format!(
            "weight {:.2} * freshness {:.2} * theme {:.2} * tags {:.2} * text {:.2}",
            entry.weight, entry.freshness, theme_factor, tag_factor, text_factor
        ),
    }
}

fn recall_item_from_archive(
    entry: ArchiveEntry,
    scored: ScoredArchiveEntry,
    explain: bool,
) -> RecallItem {
    RecallItem {
        source_layer: RecallSourceLayer::Archive,
        id: entry.archive_id,
        gist: entry.gist,
        narrative: Some(entry.narrative),
        facts: entry.facts.into_iter().map(|fact| fact.text).collect(),
        quotes: entry.quotes.into_iter().map(|quote| quote.text).collect(),
        source_session_id: Some(entry.source_session_id),
        time_range: Some(entry.time_range),
        tags: entry.tags,
        theme: entry.theme,
        weight: entry.weight,
        freshness: entry.freshness,
        relevance_score: scored.score,
        relevance_explanation: explain.then_some(scored.explanation),
    }
}

fn time_range_from_events(events: &[&StoredEvent]) -> TimeRange {
    let start = events
        .iter()
        .map(|event| event.timestamp.as_str())
        .min()
        .unwrap_or("unknown")
        .to_string();
    let end = events
        .iter()
        .map(|event| event.timestamp.as_str())
        .max()
        .unwrap_or("unknown")
        .to_string();

    TimeRange { start, end }
}

fn collect_tags(events: &[&StoredEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| event.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn preliminary_gist(events: &[&StoredEvent]) -> String {
    let texts = events
        .iter()
        .filter_map(|event| event_text(event))
        .take(3)
        .map(|text| truncate_chars(&text, 120))
        .collect::<Vec<_>>();

    if texts.is_empty() {
        format!("Попередній спогад із {} події(й).", events.len())
    } else {
        format!("Попередній спогад: {}.", texts.join(" / "))
    }
}

fn preliminary_narrative(session_id: &str, events: &[&StoredEvent]) -> String {
    let lines = events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| {
                format!(
                    "{} {}: {}",
                    event.timestamp,
                    event.event_type,
                    truncate_chars(&text, 180)
                )
            })
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        format!(
            "Попередній архівний спогад із сесії {session_id}, створений алгоритмічно з {} події(й).",
            events.len()
        )
    } else {
        format!(
            "Попередній архівний спогад із сесії {session_id}. Ключові події: {}",
            lines.join(" | ")
        )
    }
}

fn preliminary_facts(events: &[&StoredEvent]) -> Vec<WeightedFact> {
    events
        .iter()
        .filter_map(|event| {
            event_text(event).map(|text| WeightedFact {
                text: truncate_chars(&text, 240),
                confidence: event.initial_weight.clamp(0.0, 1.0),
                source_event_ids: vec![event.event_id.clone()],
            })
        })
        .collect()
}

fn archive_weight(events: &[&StoredEvent]) -> f64 {
    events
        .iter()
        .map(|event| event.initial_weight)
        .fold(0.0, f64::max)
        .clamp(0.0, 1.0)
}

fn event_text(event: &StoredEvent) -> Option<String> {
    event
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn query_tokens(query: &RecallQuery) -> BTreeSet<String> {
    let mut text = String::new();

    if let Some(query_text) = &query.query_text {
        text.push_str(query_text);
        text.push(' ');
    }

    if let Some(recent_text) = query
        .context
        .get("recent_text")
        .and_then(|value| value.as_str())
    {
        text.push_str(recent_text);
    }

    tokenize(&text)
}

fn archive_tokens(entry: &ArchiveEntry) -> BTreeSet<String> {
    let mut text = String::new();
    text.push_str(&entry.gist);
    text.push(' ');
    text.push_str(&entry.narrative);
    text.push(' ');

    for fact in &entry.facts {
        text.push_str(&fact.text);
        text.push(' ');
    }

    for quote in &entry.quotes {
        text.push_str(&quote.text);
        text.push(' ');
    }

    for tag in &entry.tags {
        text.push_str(tag);
        text.push(' ');
    }

    if let Some(theme) = &entry.theme {
        text.push_str(theme);
    }

    tokenize(&text)
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .map(|token| token.trim().to_lowercase())
        .filter(|token| token.chars().count() >= 2)
        .collect()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn new_id(prefix: &str) -> Result<String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| {
            MemoryEngineError::Storage(format!("system clock before unix epoch: {err}"))
        })?
        .as_nanos();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{prefix}_{nanos}_{counter}"))
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| MemoryEngineError::Storage(format!("failed to format timestamp: {err}")))
}
