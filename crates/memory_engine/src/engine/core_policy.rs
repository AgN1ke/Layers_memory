use super::*;

pub(super) fn apply_core_fact_upsert<F>(
    mut category: CoreStoreCategory,
    input: &CoreFactInput,
    category_name: String,
    new_fact_id: F,
    applied_at: String,
) -> Result<(CoreStoreCategory, CoreFactUpsertResult)>
where
    F: FnOnce() -> Result<String>,
{
    let scope = normalize_optional_string(input.scope.as_deref());
    let fact_text = normalize_whitespace(&input.text);

    if category.schema_version.trim().is_empty() {
        category.schema_version = CORE_STORE_SCHEMA_VERSION.to_string();
    }
    category.category = category_name.clone();
    category.updated_at = applied_at.clone();

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
        existing.updated_at = applied_at.clone();
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
            core_fact_id: new_fact_id()?,
            scope,
            text: fact_text,
            status: CoreFactStatus::Active,
            confidence: input.confidence.clamp(0.0, 1.0),
            created_at: applied_at.clone(),
            updated_at: applied_at,
            source_archive_ids: input.source_archive_ids.clone(),
            source_candidate_id: input.source_candidate_id.clone(),
            tags: unique_strings(input.tags.clone()),
            links: Vec::new(),
            review: None,
        };
        category.facts.push(fact.clone());
        fact
    };

    let result = CoreFactUpsertResult {
        schema_version: CORE_FACT_UPSERT_RESULT_SCHEMA_VERSION.to_string(),
        category: category_name,
        created,
        fact,
    };
    Ok((category, result))
}

pub(super) fn apply_core_fact_patch(
    mut category: CoreStoreCategory,
    input: &CoreFactPatchInput,
    category_name: String,
    applied_at: String,
) -> Result<(CoreStoreCategory, CoreFactPatchResult)> {
    let scope = normalize_optional_string(input.scope.as_deref());
    let patch_text = input.text.as_deref().map(normalize_whitespace);
    let patch_tags = input.tags.clone().map(unique_strings);
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

    if let Some(text) = patch_text {
        fact.text = text;
    }
    if let Some(status) = input.status {
        fact.status = status;
    }
    if let Some(confidence) = input.confidence {
        fact.confidence = confidence.clamp(0.0, 1.0);
    }
    if let Some(tags) = patch_tags {
        fact.tags = tags;
    }
    fact.updated_at = applied_at.clone();

    let patched_fact = fact.clone();
    category.category = category_name.clone();
    category.updated_at = applied_at;
    let result = CoreFactPatchResult {
        schema_version: CORE_FACT_PATCH_RESULT_SCHEMA_VERSION.to_string(),
        category: category_name,
        fact: patched_fact,
    };
    Ok((category, result))
}
