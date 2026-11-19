use super::*;
use crate::prompt_view::fit_quote_to_prompt;

pub(super) fn session_context_events(
    session: &SessionRecord,
    limit: usize,
    archived_event_ids: &HashSet<String>,
) -> Vec<CoreContextEvent> {
    if limit == 0 {
        return Vec::new();
    }

    let mut events = session
        .events
        .iter()
        .rev()
        .filter(|event| !archived_event_ids.contains(&event.event_id))
        .take(limit)
        .collect::<Vec<_>>();
    events.reverse();

    events
        .into_iter()
        .map(|event| CoreContextEvent {
            event_id: event.event_id.clone(),
            timestamp: event.timestamp.clone(),
            event_type: event.event_type.clone(),
            source: event.source.clone(),
            speaker: event.speaker.clone(),
            text: event_text(event),
            quoted_events: Vec::new(),
            payload: event.payload.clone(),
            tags: event.tags.clone(),
            theme: event.theme.clone(),
        })
        .collect()
}

pub(super) struct BudgetedContextPackage {
    pub(super) core_facts: Vec<CoreContextFact>,
    pub(super) session_recent: Vec<CoreContextEvent>,
    pub(super) session_trace: Vec<CoreContextEvent>,
    pub(super) archive_relevant: Vec<RecallItem>,
    pub(super) associative_memory: Vec<AssociativeMemoryItem>,
    pub(super) report: CoreContextBudgetReport,
    pub(super) notes: Vec<String>,
}

pub(super) struct ContextMemoryLayers {
    pub(super) core_facts: Vec<CoreContextFact>,
    pub(super) session_recent: Vec<CoreContextEvent>,
    pub(super) session_trace: Vec<CoreContextEvent>,
    pub(super) archive_relevant: Vec<RecallItem>,
    pub(super) associative_memory: Vec<AssociativeMemoryItem>,
}

pub(super) fn apply_context_token_budget(
    layers: ContextMemoryLayers,
    domain_state: &Value,
    budget: CoreContextTokenBudget,
    time_labels: &TimeLabelContext,
) -> BudgetedContextPackage {
    let ContextMemoryLayers {
        core_facts,
        session_recent,
        session_trace,
        archive_relevant,
        associative_memory,
    } = layers;
    let estimated_domain_state_tokens = estimate_json_tokens(domain_state);

    let (core_facts, estimated_core_tokens, dropped_core_facts) = keep_front_within_budget_by(
        core_facts,
        budget.core_tokens,
        estimate_core_fact_prompt_tokens,
    );

    let current_memory_budget = budget
        .current_memory_tokens
        .saturating_sub(estimated_domain_state_tokens);
    let (mut session_recent, estimated_session_recent_tokens, dropped_session_recent) =
        keep_recent_within_budget_by(
            session_recent,
            current_memory_budget,
            estimate_recent_event_prompt_tokens,
        );
    let remaining_current_budget =
        current_memory_budget.saturating_sub(estimated_session_recent_tokens);
    let (mut session_trace, estimated_session_trace_tokens, dropped_session_trace) =
        keep_recent_within_budget_by(
            session_trace,
            remaining_current_budget,
            estimate_trace_event_prompt_tokens,
        );

    // Associative memory is a high-threshold activation by the current turn.
    // Give it first access to the compressed-memory budget, while preserving a
    // separate result type and score domain from ordinary Archive recall.
    let (associative_memory, estimated_associative_memory_tokens, dropped_associative_memory) =
        keep_front_within_budget_by(
            associative_memory,
            budget.compressed_memory_tokens,
            |item| estimate_associative_memory_prompt_tokens(item, time_labels),
        );
    let remaining_compressed_budget = budget
        .compressed_memory_tokens
        .saturating_sub(estimated_associative_memory_tokens);

    let original_archive_count = archive_relevant.len();
    let archive_relevant = archive_relevant
        .into_iter()
        .take(ARCHIVE_MEMORY_PROMPT_LIMIT)
        .collect::<Vec<_>>();
    let dropped_by_prompt_archive_limit =
        original_archive_count.saturating_sub(archive_relevant.len());
    let (archive_relevant, estimated_compressed_memory_tokens, dropped_by_compressed_budget) =
        keep_front_within_budget_by(archive_relevant, remaining_compressed_budget, |archive| {
            estimate_archive_prompt_tokens(archive, time_labels)
        });
    let dropped_archive_relevant = dropped_by_prompt_archive_limit + dropped_by_compressed_budget;
    let estimated_compressed_memory_tokens =
        estimated_associative_memory_tokens + estimated_compressed_memory_tokens;

    let base_current_tokens = estimated_domain_state_tokens
        + estimated_session_recent_tokens
        + estimated_session_trace_tokens;
    let quote_budget = budget
        .current_memory_tokens
        .saturating_sub(base_current_tokens)
        .min(budget.total_tokens.saturating_sub(
            base_current_tokens + estimated_compressed_memory_tokens + estimated_core_tokens,
        ));
    let estimated_current_memory_tokens = base_current_tokens
        + budget_context_quotes(&mut session_recent, &mut session_trace, quote_budget);
    let estimated_total_tokens = estimated_current_memory_tokens
        + estimated_compressed_memory_tokens
        + estimated_core_tokens;

    let budget_exceeded = estimated_total_tokens > budget.total_tokens
        || estimated_current_memory_tokens > budget.current_memory_tokens
        || estimated_compressed_memory_tokens > budget.compressed_memory_tokens
        || estimated_core_tokens > budget.core_tokens;

    let mut notes = Vec::new();
    if estimated_domain_state_tokens > budget.current_memory_tokens {
        notes.push(format!(
            "domain_state alone exceeds current-memory budget: estimated {estimated_domain_state_tokens} tokens > budget {}.",
            budget.current_memory_tokens
        ));
    }
    if dropped_session_recent > 0 {
        notes.push(format!(
            "token budget dropped {dropped_session_recent} session_recent event(s); newest events were kept."
        ));
    }
    if dropped_session_trace > 0 {
        notes.push(format!(
            "token budget dropped {dropped_session_trace} session_trace event(s); newest events were kept."
        ));
    }
    if dropped_archive_relevant > 0 {
        notes.push(format!(
            "token budget dropped {dropped_archive_relevant} archive_relevant item(s); highest-ranked recall items were kept."
        ));
    }
    if dropped_associative_memory > 0 {
        notes.push(format!(
            "token budget dropped {dropped_associative_memory} associative memory item(s); highest-scored current-topic associations were kept."
        ));
    }
    if dropped_core_facts > 0 {
        notes.push(format!(
            "token budget dropped {dropped_core_facts} core fact(s); highest-confidence facts were kept."
        ));
    }
    if budget_exceeded {
        notes.push(
            "token budget is still exceeded after trimming; inspect domain_state/current turn size."
                .to_string(),
        );
    }

    BudgetedContextPackage {
        core_facts,
        session_recent,
        session_trace,
        archive_relevant,
        associative_memory,
        report: CoreContextBudgetReport {
            estimator: "unicode_chars_div_2_ceil_json_v1".to_string(),
            total_budget_tokens: budget.total_tokens,
            current_memory_budget_tokens: budget.current_memory_tokens,
            compressed_memory_budget_tokens: budget.compressed_memory_tokens,
            core_budget_tokens: budget.core_tokens,
            estimated_total_tokens,
            estimated_current_memory_tokens,
            estimated_compressed_memory_tokens,
            estimated_core_tokens,
            estimated_domain_state_tokens,
            estimated_associative_memory_tokens,
            dropped_session_recent,
            dropped_session_trace,
            dropped_archive_relevant,
            dropped_associative_memory,
            dropped_core_facts,
            budget_exceeded,
        },
        notes,
    }
}

fn budget_context_quotes(
    recent: &mut [CoreContextEvent],
    trace: &mut [CoreContextEvent],
    budget: usize,
) -> usize {
    if recent
        .iter()
        .chain(trace.iter())
        .all(|event| event.quoted_events.is_empty())
    {
        return 0;
    }

    // Preserve the authored-turn selection. Both prompt modes use one copy of
    // a turn, even when the package includes it in both recent and trace.
    let mut remaining = budget;
    for event in recent.iter_mut().rev() {
        budget_event_quotes(event, RECENT_MAX_TEXT_CHARS, &mut remaining);
    }
    let recent_quotes: HashMap<_, _> = recent
        .iter()
        .filter(|event| !event.event_id.is_empty())
        .map(|event| (event.event_id.as_str(), &event.quoted_events))
        .collect();
    for event in trace.iter_mut().rev() {
        if let Some(quotes) = recent_quotes.get(event.event_id.as_str()) {
            event.quoted_events.clone_from(quotes);
        } else {
            budget_event_quotes(event, OLDER_TRACE_MAX_TEXT_CHARS, &mut remaining);
        }
    }
    budget - remaining
}

fn budget_event_quotes(event: &mut CoreContextEvent, max_text_chars: usize, remaining: &mut usize) {
    if event
        .text
        .as_ref()
        .is_none_or(|text| text.trim().is_empty())
    {
        event.quoted_events.clear();
        return;
    }
    event.quoted_events.retain_mut(|quote| {
        let Some(used_chars) =
            fit_quote_to_prompt(quote, max_text_chars, remaining.saturating_mul(2))
        else {
            return false;
        };
        *remaining -= used_chars.div_ceil(2);
        true
    });
}

pub(super) fn keep_front_within_budget_by<T: Clone>(
    items: Vec<T>,
    budget: usize,
    estimate: impl Fn(&T) -> usize,
) -> (Vec<T>, usize, usize) {
    let original_len = items.len();
    let mut kept = Vec::new();
    let mut used = 0usize;

    for item in items {
        let item_estimate = estimate(&item);
        if used + item_estimate <= budget {
            used += item_estimate;
            kept.push(item);
        }
    }

    let dropped = original_len.saturating_sub(kept.len());
    (kept, used, dropped)
}

pub(super) fn rank_core_facts_for_query(
    mut facts: Vec<CoreContextFact>,
    query_text: Option<&str>,
) -> Vec<CoreContextFact> {
    let query_tokens = core_query_tokens(query_text.unwrap_or_default());
    if query_tokens.is_empty() {
        return facts;
    }

    facts.sort_by(|left, right| {
        core_fact_query_score(right, &query_tokens)
            .cmp(&core_fact_query_score(left, &query_tokens))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.category.cmp(&right.category))
            .then_with(|| left.core_fact_id.cmp(&right.core_fact_id))
    });
    facts
}

pub(super) fn estimate_core_fact_prompt_tokens(fact: &CoreContextFact) -> usize {
    render_core_fact_prompt_line(fact)
        .map(|line| estimate_text_tokens(&line))
        .unwrap_or(0)
}

pub(super) fn estimate_archive_prompt_tokens(
    archive: &RecallItem,
    time_labels: &TimeLabelContext,
) -> usize {
    estimate_text_tokens(&render_archive_memory_prompt_lines(archive, time_labels).join("\n"))
}

pub(super) fn estimate_associative_memory_prompt_tokens(
    item: &AssociativeMemoryItem,
    time_labels: &TimeLabelContext,
) -> usize {
    estimate_text_tokens(&render_associative_memory_prompt_lines(item, time_labels).join("\n"))
}

pub(super) fn estimate_recent_event_prompt_tokens(event: &CoreContextEvent) -> usize {
    render_context_event_prompt_line(event, RECENT_MAX_TEXT_CHARS)
        .map(|line| estimate_text_tokens(&line))
        .unwrap_or(0)
}

pub(super) fn estimate_trace_event_prompt_tokens(event: &CoreContextEvent) -> usize {
    render_context_event_prompt_line(event, OLDER_TRACE_MAX_TEXT_CHARS)
        .map(|line| estimate_text_tokens(&line))
        .unwrap_or(0)
}

pub(super) fn core_fact_query_score(fact: &CoreContextFact, query_tokens: &[String]) -> usize {
    let fact_text = normalize_match_text(&fact.text);
    let fact_category = normalize_match_text(&fact.category);
    let fact_subject = fact
        .subject
        .as_ref()
        .map(|subject| normalize_match_text(&format!("{} {}", subject.id, subject.name)))
        .unwrap_or_default();
    let fact_tags: Vec<String> = fact
        .tags
        .iter()
        .map(|tag| normalize_match_text(tag))
        .collect();
    let fact_tokens: HashSet<String> = core_query_tokens(&fact.text).into_iter().collect();

    query_tokens
        .iter()
        .map(|token| {
            let mut score = 0usize;
            if fact_tokens.contains(token) {
                score += 12;
            } else if fact_text.contains(token) {
                score += 6;
            }
            if fact_category.contains(token) {
                score += 4;
            }
            if fact_subject.contains(token) {
                score += 12;
            }
            if fact_tags.iter().any(|tag| tag.contains(token)) {
                score += 2;
            }
            score
        })
        .sum()
}

pub(super) fn core_query_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut seen = HashSet::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else {
            push_core_query_token(&mut tokens, &mut seen, &mut current);
        }
    }
    push_core_query_token(&mut tokens, &mut seen, &mut current);
    tokens
}

pub(super) fn push_core_query_token(
    tokens: &mut Vec<String>,
    seen: &mut HashSet<String>,
    current: &mut String,
) {
    if current.chars().count() >= 3 && seen.insert(current.clone()) {
        tokens.push(current.clone());
    }
    current.clear();
}

pub(super) fn keep_recent_within_budget_by<T: Clone>(
    items: Vec<T>,
    budget: usize,
    estimate: impl Fn(&T) -> usize,
) -> (Vec<T>, usize, usize) {
    let original_len = items.len();
    let mut kept_reversed = Vec::new();
    let mut used = 0usize;

    for item in items.into_iter().rev() {
        let item_estimate = estimate(&item);
        if used + item_estimate <= budget {
            used += item_estimate;
            kept_reversed.push(item);
        }
    }
    kept_reversed.reverse();

    let dropped = original_len.saturating_sub(kept_reversed.len());
    (kept_reversed, used, dropped)
}
