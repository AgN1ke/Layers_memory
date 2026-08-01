use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn upsert_core_fact(&self, input: CoreFactInput) -> Result<CoreFactUpsertResult> {
        validate_core_fact_input(&input)?;
        self.ensure_manifest()?;

        let category_name = normalize_whitespace(&input.category);
        self.with_resource_lock(core_lock_key(&category_name), || {
            self.upsert_core_fact_unlocked(input, category_name)
        })
    }

    pub(super) fn upsert_core_fact_unlocked(
        &self,
        input: CoreFactInput,
        category_name: String,
    ) -> Result<CoreFactUpsertResult> {
        let now = now_rfc3339()?;
        let category = self.storage.read_core_store_category(&category_name)?;
        let (category, result) = core_policy::apply_core_fact_upsert(
            category,
            &input,
            category_name,
            || new_id("core_fact"),
            now,
        )?;
        self.storage.write_core_store_category(&category)?;
        Ok(result)
    }

    pub fn patch_core_fact(&self, input: CoreFactPatchInput) -> Result<CoreFactPatchResult> {
        validate_core_fact_patch_input(&input)?;
        self.ensure_manifest()?;

        let now = now_rfc3339()?;
        let scope = normalize_optional_string(input.scope.as_deref());

        let category_name = self
            .storage
            .read_core_store_categories()?
            .into_items()
            .into_iter()
            .find(|category| {
                category
                    .facts
                    .iter()
                    .any(|fact| fact.core_fact_id == input.core_fact_id && fact.scope == scope)
            })
            .map(|category| category.category)
            .ok_or_else(|| {
                MemoryEngineError::Validation(format!(
                    "core fact not found for requested scope: {}",
                    input.core_fact_id
                ))
            })?;

        self.with_resource_lock(core_lock_key(&category_name), || {
            let category = self.storage.read_core_store_category(&category_name)?;
            let (category, result) = core_policy::apply_core_fact_patch(
                category,
                &input,
                category_name.clone(),
                now.clone(),
            )?;
            self.storage.write_core_store_category(&category)?;
            Ok(result)
        })
    }

    pub fn core_context_package(&self, request: CoreContextRequest) -> Result<CoreContextPackage> {
        validate_core_context_request(&request)?;
        self.ensure_manifest()?;

        let created_at = now_rfc3339()?;
        let archived_event_ids = self.archived_event_ids_for_session(&request.session_id)?;
        let session = self.storage.read_session(&request.session_id)?;
        let recent_limit = if request.session_recent_limit == 0 {
            self.options.context.default_session_recent_limit
        } else {
            request.session_recent_limit
        };
        let trace_limit = if request.session_trace_event_limit == 0 {
            self.options.context.default_session_trace_event_limit
        } else {
            request.session_trace_event_limit
        };
        let recall_limit = if request.recall_limit == 0 {
            self.options.recall.default_limit
        } else {
            request.recall_limit
        };

        let session_recent = session_context_events(&session, recent_limit, &archived_event_ids);
        let session_trace = session_context_events(&session, trace_limit, &archived_event_ids);
        let query_text = request
            .query_text
            .clone()
            .or_else(|| {
                request
                    .domain_state
                    .get("recent_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                request
                    .domain_state
                    .get("current_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let query_text_for_core_ranking = query_text.clone();

        let recall_result = self.recall(RecallQuery {
            schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            query_id: None,
            created_at: Some(created_at.clone()),
            session_id: Some(request.session_id.clone()),
            context: json!({ "recent_text": query_text.clone().unwrap_or_default() }),
            query_text,
            // Chat context keeps ordinary Archive recall structural. The same
            // current-turn embedding is consumed below by the separate
            // associative-memory path, never by both mechanisms in one list.
            query_embedding: None,
            filters: RecallFilters {
                source_layers: vec![RecallSourceLayer::Archive],
                ..RecallFilters::default()
            },
            limit: recall_limit,
            include_core: false,
            explain: request.explain_recall,
        })?;
        let archive_relevant = recall_result.items;
        let recall_debug = recall_result.debug;

        let (core_facts, core_read_warnings) = if request.include_core {
            self.core_context_facts(request.core_scope.as_deref())?
        } else {
            (Vec::new(), Vec::new())
        };
        let core_facts =
            rank_core_facts_for_query(core_facts, query_text_for_core_ranking.as_deref());
        let mut notes = if request.include_core && core_facts.is_empty() {
            vec![
                "core_facts are empty; no stable Core Store facts have been saved yet.".to_string(),
            ]
        } else {
            Vec::new()
        };
        notes.extend(recall_result.notes);
        notes.extend(storage_warning_notes(&core_read_warnings));
        let (associative_memory, expansion_notes) = self.associative_memory_items(
            &request,
            &created_at,
            &core_facts,
            &session_recent,
            &session_trace,
            &archive_relevant,
        )?;
        notes.extend(expansion_notes);

        let budget_config = request
            .token_budget
            .unwrap_or(self.options.context.token_budget);
        let time_labels = TimeLabelContext::new(
            &created_at,
            request.utc_offset_minutes,
            request.clock_untrusted,
        );
        let budgeted = apply_context_token_budget(
            ContextMemoryLayers {
                core_facts,
                session_recent,
                session_trace,
                archive_relevant,
                associative_memory,
            },
            &request.domain_state,
            budget_config,
            &time_labels,
        );
        notes.extend(budgeted.notes);

        let future_timestamps = budgeted
            .archive_relevant
            .iter()
            .filter_map(|item| item.time_range.as_ref())
            .map(|range| range.end.as_str())
            .chain(
                budgeted
                    .associative_memory
                    .iter()
                    .map(|item| item.created_at.as_str()),
            )
            .chain(
                budgeted
                    .session_recent
                    .iter()
                    .map(|event| event.timestamp.as_str()),
            )
            .chain(
                budgeted
                    .session_trace
                    .iter()
                    .map(|event| event.timestamp.as_str()),
            )
            .filter(|timestamp| timestamp_is_future(timestamp, &created_at))
            .count();
        if future_timestamps > 0 {
            notes.push(format!(
                "{future_timestamps} memory timestamp(s) are in the future relative to now; their time labels are omitted."
            ));
        }

        Ok(CoreContextPackage {
            schema_version: CORE_CONTEXT_PACKAGE_SCHEMA_VERSION.to_string(),
            created_at,
            utc_offset_minutes: request.utc_offset_minutes,
            clock_untrusted: request.clock_untrusted,
            core_facts: budgeted.core_facts,
            session_recent: budgeted.session_recent,
            session_trace: budgeted.session_trace,
            archive_relevant: budgeted.archive_relevant,
            associative_memory: budgeted.associative_memory,
            domain_state: request.domain_state,
            budget: Some(budgeted.report),
            recall_debug,
            notes,
        })
    }

    pub(super) fn core_context_facts(
        &self,
        scope: Option<&str>,
    ) -> Result<(Vec<CoreContextFact>, Vec<StorageReadWarning>)> {
        let normalized_scope = normalize_optional_string(scope);
        let categories = self.storage.read_core_store_categories()?;
        let warnings = categories.warnings;
        let mut facts = Vec::new();
        for category in categories.items {
            let fact_category = category.category.clone();
            for fact in category.facts {
                if !core_fact_visible_in_context(fact.status) {
                    continue;
                }
                if fact.scope != normalized_scope {
                    continue;
                }
                facts.push(CoreContextFact {
                    category: fact_category.clone(),
                    core_fact_id: fact.core_fact_id,
                    scope: fact.scope,
                    text: fact.text,
                    status: fact.status,
                    confidence: fact.confidence,
                    tags: fact.tags,
                });
            }
        }

        facts.sort_by(|left, right| {
            core_context_status_rank(left.status)
                .cmp(&core_context_status_rank(right.status))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| left.category.cmp(&right.category))
                .then_with(|| left.core_fact_id.cmp(&right.core_fact_id))
        });
        Ok((facts, warnings))
    }

    fn associative_memory_items(
        &self,
        request: &CoreContextRequest,
        created_at: &str,
        core_facts: &[CoreContextFact],
        session_recent: &[CoreContextEvent],
        session_trace: &[CoreContextEvent],
        archive_relevant: &[RecallItem],
    ) -> Result<(Vec<AssociativeMemoryItem>, Vec<String>)> {
        let Some(embedding) = request.query_embedding.as_ref() else {
            return Ok((Vec::new(), Vec::new()));
        };

        let scope = request.core_scope.as_deref().unwrap_or(&request.session_id);
        let min_similarity = self.options.vectors.contextual_expansion_min_sim;
        let result = self.recall_deep(DeepRecallQuery {
            scope: scope.to_string(),
            query_vec: embedding.query_vec.clone(),
            model_id: embedding.model_id.clone(),
            top_k: self.options.vectors.contextual_expansion_top_k,
            min_sim: min_similarity,
            now: Some(created_at.to_string()),
        })?;

        if !result.found {
            return Ok((
                Vec::new(),
                result
                    .reason
                    .map(|reason| {
                        format!(
                            "associative memory skipped: {reason} (min_similarity={min_similarity:.3})."
                        )
                    })
                    .into_iter()
                    .collect(),
            ));
        }

        let mut visible =
            visible_memory_texts(core_facts, session_recent, session_trace, archive_relevant);
        let mut items = Vec::new();
        let mut seen_units = HashSet::new();
        for hit in result.hits {
            let normalized_thesis = normalize_match_text(&hit.thesis);
            if normalized_thesis.is_empty()
                || visible
                    .iter()
                    .any(|text| text.contains(normalized_thesis.as_str()))
                || !seen_units.insert(hit.memory_unit_id.clone())
            {
                continue;
            }
            visible.push(normalized_thesis);
            items.push(associative_memory_item(hit, scope, min_similarity));
        }

        let notes = if items.is_empty() {
            vec![format!(
                "associative memory found only already-visible detail at min_similarity={min_similarity:.3}."
            )]
        } else {
            vec![format!(
                "associative memory added {} current-topic detail item(s) at min_similarity={min_similarity:.3}.",
                items.len(),
            )]
        };

        Ok((items, notes))
    }
}

fn associative_memory_item(
    hit: DeepRecallHit,
    scope: &str,
    activation_threshold: f32,
) -> AssociativeMemoryItem {
    AssociativeMemoryItem {
        memory_unit_id: hit.memory_unit_id,
        archive_id: hit.archive_id,
        scope: scope.to_string(),
        thesis: hit.thesis,
        created_at: hit.created_at,
        activation: AssociativeMemoryActivation::CurrentTopicVector,
        semantic_similarity: hit.sim,
        activation_threshold,
        deep_recall_score: hit.score,
        why_recalled: Some(format!(
            "current topic vector matched an older memory unit (sim={:.3} >= min_similarity={activation_threshold:.3})",
            hit.sim,
        )),
    }
}

fn visible_memory_texts(
    core_facts: &[CoreContextFact],
    session_recent: &[CoreContextEvent],
    session_trace: &[CoreContextEvent],
    archive_relevant: &[RecallItem],
) -> Vec<String> {
    let mut texts = Vec::new();
    texts.extend(
        core_facts
            .iter()
            .map(|fact| normalize_match_text(&fact.text)),
    );
    texts.extend(
        session_recent
            .iter()
            .filter_map(|event| event.text.as_deref())
            .map(normalize_match_text),
    );
    texts.extend(
        session_trace
            .iter()
            .filter_map(|event| event.text.as_deref())
            .map(normalize_match_text),
    );
    for item in archive_relevant {
        texts.push(normalize_match_text(&item.gist));
        if let Some(compact) = item.compact_memory.as_deref() {
            texts.push(normalize_match_text(compact));
        }
        if let Some(narrative) = item.narrative.as_deref() {
            texts.push(normalize_match_text(narrative));
        }
        texts.extend(item.facts.iter().map(|fact| normalize_match_text(fact)));
    }
    texts.into_iter().filter(|text| !text.is_empty()).collect()
}

fn timestamp_is_future(timestamp: &str, reference_at: &str) -> bool {
    match (parse_rfc3339(timestamp), parse_rfc3339(reference_at)) {
        (Some(then), Some(now)) => then > now,
        _ => false,
    }
}
