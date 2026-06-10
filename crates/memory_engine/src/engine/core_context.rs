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
        let scope = normalize_optional_string(input.scope.as_deref());
        let fact_text = normalize_whitespace(&input.text);
        let mut category = self.storage.read_core_store_category(&category_name)?;

        if category.schema_version.trim().is_empty() {
            category.schema_version = CORE_STORE_SCHEMA_VERSION.to_string();
        }
        category.category = category_name.clone();
        category.updated_at = now.clone();

        let needle = normalize_match_text(&fact_text);
        let mut created = false;
        let fact = if let Some(existing) = category
            .facts
            .iter_mut()
            .find(|fact| normalize_match_text(&fact.text) == needle && fact.scope == scope)
        {
            existing.scope = scope.clone();
            existing.text = fact_text;
            existing.status = CoreFactStatus::Active;
            existing.confidence = existing.confidence.max(input.confidence).clamp(0.0, 1.0);
            existing.updated_at = now.clone();
            merge_unique(&mut existing.tags, &input.tags);
            merge_unique(&mut existing.source_archive_ids, &input.source_archive_ids);
            if existing.source_candidate_id.is_none() {
                existing.source_candidate_id = input.source_candidate_id.clone();
            }
            existing.clone()
        } else {
            created = true;
            let fact = CoreFact {
                schema_version: CORE_FACT_SCHEMA_VERSION.to_string(),
                core_fact_id: new_id("core_fact")?,
                scope,
                text: fact_text,
                status: CoreFactStatus::Active,
                confidence: input.confidence.clamp(0.0, 1.0),
                created_at: now.clone(),
                updated_at: now,
                source_archive_ids: input.source_archive_ids,
                source_candidate_id: input.source_candidate_id,
                tags: unique_strings(input.tags),
                links: Vec::new(),
                review: None,
            };
            category.facts.push(fact.clone());
            fact
        };

        self.storage.write_core_store_category(&category)?;
        Ok(CoreFactUpsertResult {
            schema_version: CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION.to_string(),
            category: category_name,
            created,
            fact,
        })
    }

    pub fn patch_core_fact(&self, input: CoreFactPatchInput) -> Result<CoreFactPatchResult> {
        validate_core_fact_patch_input(&input)?;
        self.ensure_manifest()?;

        let now = now_rfc3339()?;
        let scope = normalize_optional_string(input.scope.as_deref());
        let patch_text = input.text.as_deref().map(normalize_whitespace);
        let patch_tags = input.tags.map(unique_strings);

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
            let mut category = self.storage.read_core_store_category(&category_name)?;
            let Some(fact) = category
                .facts
                .iter_mut()
                .find(|fact| fact.core_fact_id == input.core_fact_id && fact.scope == scope)
            else {
                return Err(MemoryEngineError::Validation(format!(
                    "core fact not found for requested scope: {}",
                    input.core_fact_id
                )));
            };

            if let Some(text) = patch_text.as_ref() {
                fact.text = text.clone();
            }
            if let Some(status) = input.status {
                fact.status = status;
            }
            if let Some(confidence) = input.confidence {
                fact.confidence = confidence.clamp(0.0, 1.0);
            }
            if let Some(tags) = patch_tags.as_ref() {
                fact.tags = tags.clone();
            }
            fact.updated_at = now.clone();

            let patched_fact = fact.clone();
            category.updated_at = now;
            let category_name = category.category.clone();
            self.storage.write_core_store_category(&category)?;

            Ok(CoreFactPatchResult {
                schema_version: CORE_FACT_PATCH_RESULT_SCHEMA_VERSION.to_string(),
                category: category_name,
                fact: patched_fact,
            })
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
            filters: RecallFilters {
                source_layers: vec![RecallSourceLayer::Archive],
                ..RecallFilters::default()
            },
            limit: recall_limit,
            include_core: false,
            explain: false,
        })?;
        let archive_relevant = recall_result.items;

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

        let budget_config = request
            .token_budget
            .unwrap_or(self.options.context.token_budget);
        let budgeted = apply_context_token_budget(
            core_facts,
            session_recent,
            session_trace,
            archive_relevant,
            &request.domain_state,
            budget_config,
        );
        notes.extend(budgeted.notes);

        Ok(CoreContextPackage {
            schema_version: CORE_CONTEXT_PACKAGE_SCHEMA_VERSION.to_string(),
            created_at,
            core_facts: budgeted.core_facts,
            session_recent: budgeted.session_recent,
            session_trace: budgeted.session_trace,
            archive_relevant: budgeted.archive_relevant,
            domain_state: request.domain_state,
            budget: Some(budgeted.report),
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
}
