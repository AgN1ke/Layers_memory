use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn recall(&self, query: RecallQuery) -> Result<RecallResult> {
        validate_recall_query(&query)?;
        self.ensure_manifest()?;

        let result = if let Some(session_id) = query.session_id.clone() {
            self.with_resource_lock(session_lock_key(&session_id), || {
                self.recall_unlocked(query)
            })
        } else {
            self.with_resource_lock("archive:all".to_string(), || self.recall_unlocked(query))
        }?;

        if self.recall_flush_due() {
            self.flush_recall_stats()?;
        }
        Ok(result)
    }

    pub(super) fn recall_unlocked(&self, query: RecallQuery) -> Result<RecallResult> {
        let created_at = query.created_at.clone().map_or_else(now_rfc3339, Ok)?;
        let pending_recall_stats = self.recall_stats_snapshot()?;
        let archive_enabled = query.filters.source_layers.is_empty()
            || query
                .filters
                .source_layers
                .contains(&RecallSourceLayer::Archive);

        let archive_read = if archive_enabled {
            Some(
                self.storage
                    .read_archive(&archive_filters_from_recall(&query.filters))?,
            )
        } else {
            None
        };
        let read_warnings = archive_read
            .as_ref()
            .map(|collection| collection.warnings.clone())
            .unwrap_or_default();
        let mut archive_entries = if let Some(collection) = archive_read {
            collection
                .items
                .into_iter()
                .filter(|entry| entry.status == ArchiveStatus::Complete)
                .filter(|entry| {
                    query
                        .session_id
                        .as_ref()
                        .is_none_or(|session_id| &entry.source_session_id == session_id)
                })
                .collect()
        } else {
            Vec::new()
        };

        let candidate_count = archive_entries.len();
        let mut scored_entries = archive_entries
            .drain(..)
            .filter_map(|entry| {
                let delta = pending_recall_stats.get(&entry.archive_id);
                let effective_entry = archive_with_pending_recall_stats(entry, delta);
                let scored = score_archive_entry(
                    &effective_entry,
                    &query,
                    &created_at,
                    &self.options.recall,
                );
                if query
                    .filters
                    .min_freshness
                    .is_some_and(|min_freshness| scored.effective_freshness < min_freshness)
                {
                    None
                } else {
                    Some((effective_entry, scored))
                }
            })
            .collect::<Vec<_>>();
        let filtered_count = scored_entries.len();

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
        let selected_archive_ids = selected_entries
            .iter()
            .map(|(entry, _)| entry.archive_id.clone())
            .collect::<Vec<_>>();
        self.record_recall_stats(&selected_archive_ids, &created_at)?;

        for (entry, score) in selected_entries {
            items.push(recall_item_from_archive(entry, score, query.explain));
        }

        let notes = storage_warning_notes(&read_warnings);

        Ok(RecallResult {
            schema_version: RECALL_RESULT_SCHEMA_VERSION.to_string(),
            query_id: query.query_id,
            created_at,
            stage_used: RecallStage::Stage1,
            items,
            notes: notes.clone(),
            debug: query.explain.then_some(RecallDebug {
                candidate_count,
                filtered_count,
                notes,
            }),
        })
    }
}

pub(super) fn estimate_json_tokens<T: Serialize>(value: &T) -> usize {
    serde_json::to_string(value)
        .map(|text| estimate_text_tokens(&text))
        .unwrap_or(0)
}

pub(super) fn estimate_text_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(2)
    }
}

pub(super) fn normalize_category_name(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_underscore = false;
    for ch in normalize_whitespace(value).to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch);
            previous_underscore = false;
        } else if !previous_underscore {
            normalized.push('_');
            previous_underscore = true;
        }
    }
    normalized.trim_matches('_').chars().take(64).collect()
}

pub(super) fn meaningful_tokens(text: &str) -> BTreeSet<String> {
    let stop_words = [
        "the",
        "and",
        "this",
        "that",
        "Р С”Р С•РЎР‚Р С‘РЎРѓРЎвЂљРЎС“Р Р†Р В°РЎвЂЎ",
        "Р С”Р С•РЎР‚Р С‘РЎРѓРЎвЂљРЎС“Р Р†Р В°РЎвЂЎР В°",
        "Р С”Р С•РЎР‚Р С‘РЎРѓРЎвЂљРЎС“Р Р†Р В°РЎвЂЎРЎС“",
        "Р Т‘РЎС“Р В¶Р Вµ",
        "Р В»РЎР‹Р В±Р С‘РЎвЂљРЎРЉ",
        "РЎвЂ РЎвЂ“Р С”Р В°Р Р†Р С‘РЎвЂљРЎРЉРЎРѓРЎРЏ",
    ];
    let stop_words = stop_words.into_iter().collect::<BTreeSet<_>>();
    tokenize(text)
        .into_iter()
        .filter(|token| !stop_words.contains(token.as_str()))
        .collect()
}

pub(super) fn token_overlap_sets(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let overlap = left.intersection(right).count();
    overlap as f64 / left.len().min(right.len()) as f64
}

pub(super) fn event_text(event: &StoredEvent) -> Option<String> {
    event
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(super) fn query_tokens(query: &RecallQuery) -> BTreeSet<String> {
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

pub(super) fn archive_tokens(entry: &ArchiveEntry) -> BTreeSet<String> {
    let mut text = String::new();
    if let Some(compact_memory) = &entry.compact_memory {
        text.push_str(compact_memory);
        text.push(' ');
    }
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

pub(super) fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .map(|token| token.trim().to_lowercase())
        .filter(|token| token.chars().count() >= 2)
        .collect()
}

pub(super) fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn normalize_match_text(text: &str) -> String {
    normalize_whitespace(text).to_lowercase()
}

pub(super) fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_whitespace)
        .filter(|value| !value.is_empty())
}

pub(super) fn unique_strings(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|item| normalize_whitespace(&item))
        .filter(|item| !item.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn merge_unique(target: &mut Vec<String>, source: &[String]) {
    let mut seen = target.iter().cloned().collect::<BTreeSet<_>>();
    for item in source {
        let normalized = normalize_whitespace(item);
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        target.push(normalized);
    }
}

pub(super) fn push_link_once(target: &mut Vec<Link>, link: Link) {
    if target
        .iter()
        .any(|existing| existing.kind == link.kind && existing.target == link.target)
    {
        return;
    }
    target.push(link);
}

pub(super) fn core_fact_visible_in_context(status: CoreFactStatus) -> bool {
    matches!(status, CoreFactStatus::Active | CoreFactStatus::Contested)
}

pub(super) fn core_context_status_rank(status: CoreFactStatus) -> u8 {
    match status {
        CoreFactStatus::Active => 0,
        CoreFactStatus::Contested => 1,
        CoreFactStatus::NeedsReview => 2,
        CoreFactStatus::Contradicted => 3,
        CoreFactStatus::Deprecated => 4,
    }
}

pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) fn session_lock_key(session_id: &str) -> String {
    format!("session:{session_id}")
}

pub(super) fn core_lock_key(category: &str) -> String {
    format!("core:{category}")
}

pub(super) fn archive_lock_key(archive_id: &str) -> String {
    format!("archive:{archive_id}")
}

pub(super) fn candidate_lock_key(candidate_id: &str) -> String {
    format!("candidate:{candidate_id}")
}

pub(super) fn lock_resource<'a>(
    resource: &'a Arc<Mutex<()>>,
    key: &str,
) -> Result<MutexGuard<'a, ()>> {
    resource
        .lock()
        .map_err(|_| MemoryEngineError::Storage(format!("resource lock was poisoned: {key}")))
}

pub(super) fn new_id(prefix: &str) -> Result<String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| {
            MemoryEngineError::Storage(format!("system clock before unix epoch: {err}"))
        })?
        .as_nanos();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{prefix}_{nanos}_{counter}"))
}

pub(super) fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| MemoryEngineError::Storage(format!("failed to format timestamp: {err}")))
}

pub(super) fn default_manifest(now: &str) -> Manifest {
    Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        storage_id: "default".to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        schema_versions: SchemaVersions {
            event: EVENT_SCHEMA_VERSION.to_string(),
            session: SESSION_SCHEMA_VERSION.to_string(),
            archive_entry: ARCHIVE_ENTRY_SCHEMA_VERSION.to_string(),
            core_store: CORE_STORE_SCHEMA_VERSION.to_string(),
            core_fact: CORE_FACT_SCHEMA_VERSION.to_string(),
            candidate_belief: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            pending_task: PENDING_TASK_SCHEMA_VERSION.to_string(),
            journal_operation: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
        },
        active_embedding_model_id: None,
        last_migration_at: None,
        features: FeatureFlags {
            recall_stage: RecallStage::Stage1,
            embeddings_enabled: false,
            llm_recall_rerank_enabled: false,
            reflection_enabled: false,
        },
    }
}
