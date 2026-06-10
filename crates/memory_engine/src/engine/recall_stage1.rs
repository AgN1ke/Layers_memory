use super::*;

pub(super) fn archive_filters_from_recall(filters: &RecallFilters) -> ArchiveFilters {
    ArchiveFilters {
        time_range: filters.time_range.clone(),
        tags: filters.tags.clone(),
        theme: filters.theme.clone(),
        min_weight: filters.min_weight,
        min_freshness: filters.min_freshness,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScoredArchiveEntry {
    pub(super) score: f64,
    pub(super) effective_freshness: f64,
    pub(super) explanation: String,
}

pub(super) fn score_archive_entry(
    entry: &ArchiveEntry,
    query: &RecallQuery,
    reference_at: &str,
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

    let freshness_age_days = archive_age_days(entry, reference_at).unwrap_or(0.0);
    let freshness_decay =
        half_life_decay_factor(freshness_age_days, config.freshness_half_life_days);
    let effective_freshness = (entry.freshness.clamp(0.0, 1.0) * freshness_decay).clamp(0.0, 1.0);
    let recall_boost = recall_boost_factor(entry, reference_at, config);

    let score = (entry.weight
        * effective_freshness
        * recall_boost
        * theme_factor
        * tag_factor
        * text_factor)
        .clamp(0.0, 1.0);

    ScoredArchiveEntry {
        score,
        effective_freshness,
        explanation: format!(
            "weight {:.2} * freshness {:.2} * decay {:.2} ({:.1}d) = effective {:.2} * recall {:.2} * theme {:.2} * tags {:.2} * text {:.2}",
            entry.weight,
            entry.freshness,
            freshness_decay,
            freshness_age_days,
            effective_freshness,
            recall_boost,
            theme_factor,
            tag_factor,
            text_factor
        ),
    }
}

pub(super) fn archive_with_pending_recall_stats(
    mut entry: ArchiveEntry,
    delta: Option<&RecallStatDelta>,
) -> ArchiveEntry {
    let Some(delta) = delta else {
        return entry;
    };

    entry.recall_count = entry.recall_count.saturating_add(delta.added_count);
    entry.last_recalled_at = newest_timestamp(
        entry.last_recalled_at.as_deref(),
        delta.last_recalled_at.as_deref(),
    );
    entry
}

pub(super) fn storage_warning_notes(warnings: &[StorageReadWarning]) -> Vec<String> {
    warnings.iter().map(StorageReadWarning::note).collect()
}

pub(super) fn archive_age_days(entry: &ArchiveEntry, reference_at: &str) -> Option<f64> {
    timestamp_age_days(&entry.time_range.end, reference_at)
        .or_else(|| timestamp_age_days(&entry.updated_at, reference_at))
        .or_else(|| timestamp_age_days(&entry.created_at, reference_at))
}

pub(super) fn timestamp_age_days(older_timestamp: &str, newer_timestamp: &str) -> Option<f64> {
    let older = parse_rfc3339(older_timestamp)?;
    let newer = parse_rfc3339(newer_timestamp)?;
    if newer <= older {
        return Some(0.0);
    }
    Some((newer - older).whole_seconds() as f64 / 86_400.0)
}

pub(super) fn parse_rfc3339(timestamp: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(timestamp, &Rfc3339).ok()
}

pub(super) fn newest_timestamp(left: Option<&str>, right: Option<&str>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_time = parse_rfc3339(left);
            let right_time = parse_rfc3339(right);
            match (left_time, right_time) {
                (Some(left_time), Some(right_time)) if right_time > left_time => {
                    Some(right.to_string())
                }
                (Some(_), Some(_)) => Some(left.to_string()),
                (None, Some(_)) => Some(right.to_string()),
                (Some(_), None) => Some(left.to_string()),
                (None, None) => {
                    if right > left {
                        Some(right.to_string())
                    } else {
                        Some(left.to_string())
                    }
                }
            }
        }
        (Some(left), None) => Some(left.to_string()),
        (None, Some(right)) => Some(right.to_string()),
        (None, None) => None,
    }
}

pub(super) fn half_life_decay_factor(age_days: f64, half_life_days: f64) -> f64 {
    if !age_days.is_finite() || age_days <= 0.0 {
        return 1.0;
    }
    if !half_life_days.is_finite() || half_life_days <= 0.0 {
        return 1.0;
    }
    0.5_f64.powf(age_days / half_life_days).clamp(0.0, 1.0)
}

pub(super) fn recall_boost_factor(
    entry: &ArchiveEntry,
    reference_at: &str,
    config: &RecallStage1Config,
) -> f64 {
    let count_boost =
        (entry.recall_count as f64).ln_1p().max(0.0) * config.recall_count_log_bonus.max(0.0);
    let recent_boost = entry
        .last_recalled_at
        .as_deref()
        .and_then(|last_recalled_at| timestamp_age_days(last_recalled_at, reference_at))
        .map(|age_days| {
            config.recent_recall_bonus.max(0.0)
                * half_life_decay_factor(age_days, config.recent_recall_half_life_days)
        })
        .unwrap_or(0.0);
    (1.0 + count_boost + recent_boost).clamp(1.0, config.max_recall_boost_factor.max(1.0))
}

pub(super) fn recall_item_from_archive(
    entry: ArchiveEntry,
    scored: ScoredArchiveEntry,
    explain: bool,
) -> RecallItem {
    let compact_memory = normalize_optional_string(entry.compact_memory.as_deref());
    let prompt_gist = compact_memory.clone().unwrap_or_else(|| entry.gist.clone());
    let narrative = if compact_memory.is_some() {
        None
    } else {
        Some(entry.narrative)
    };
    let facts = if compact_memory.is_some() {
        Vec::new()
    } else {
        entry.facts.into_iter().map(|fact| fact.text).collect()
    };
    let quotes = if compact_memory.is_some() {
        Vec::new()
    } else {
        entry.quotes.into_iter().map(|quote| quote.text).collect()
    };

    RecallItem {
        source_layer: RecallSourceLayer::Archive,
        id: entry.archive_id,
        gist: prompt_gist,
        compact_memory,
        narrative,
        facts,
        quotes,
        source_session_id: Some(entry.source_session_id),
        time_range: Some(entry.time_range),
        tags: entry.tags,
        theme: entry.theme,
        weight: entry.weight,
        freshness: scored.effective_freshness,
        relevance_score: scored.score,
        relevance_explanation: explain.then_some(scored.explanation),
    }
}
