use super::*;

pub(super) fn validate_ingest_event(event: &IngestEvent) -> Result<()> {
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

pub(super) fn validate_recall_query(query: &RecallQuery) -> Result<()> {
    if query.schema_version != RECALL_QUERY_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: RECALL_QUERY_SCHEMA_VERSION.to_string(),
            actual: query.schema_version.clone(),
        });
    }

    Ok(())
}

pub(super) fn validate_core_context_request(request: &CoreContextRequest) -> Result<()> {
    if request.schema_version != CORE_CONTEXT_REQUEST_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_CONTEXT_REQUEST_SCHEMA_VERSION.to_string(),
            actual: request.schema_version.clone(),
        });
    }

    if request.session_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core context request session_id must not be empty".to_string(),
        ));
    }

    if request.utc_offset_minutes.abs() > 18 * 60 {
        return Err(MemoryEngineError::Validation(
            "core context request utc_offset_minutes must be within +/-18 hours".to_string(),
        ));
    }

    Ok(())
}

pub(super) fn validate_core_fact_input(input: &CoreFactInput) -> Result<()> {
    if input.schema_version != CORE_FACT_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_FACT_INPUT_SCHEMA_VERSION.to_string(),
            actual: input.schema_version.clone(),
        });
    }

    if input.category.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core fact category must not be empty".to_string(),
        ));
    }

    if input.text.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core fact text must not be empty".to_string(),
        ));
    }

    if !input.confidence.is_finite() {
        return Err(MemoryEngineError::Validation(
            "core fact confidence must be finite".to_string(),
        ));
    }

    Ok(())
}

pub(super) fn validate_core_fact_patch_input(input: &CoreFactPatchInput) -> Result<()> {
    if input.schema_version != CORE_FACT_PATCH_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_FACT_PATCH_INPUT_SCHEMA_VERSION.to_string(),
            actual: input.schema_version.clone(),
        });
    }

    if input.core_fact_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "core fact patch core_fact_id must not be empty".to_string(),
        ));
    }

    if input.text.is_none()
        && input.status.is_none()
        && input.confidence.is_none()
        && input.tags.is_none()
    {
        return Err(MemoryEngineError::Validation(
            "core fact patch must change at least one field".to_string(),
        ));
    }

    if input
        .text
        .as_deref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return Err(MemoryEngineError::Validation(
            "core fact patch text must not be empty".to_string(),
        ));
    }

    if input
        .confidence
        .is_some_and(|confidence| !confidence.is_finite())
    {
        return Err(MemoryEngineError::Validation(
            "core fact patch confidence must be finite".to_string(),
        ));
    }

    Ok(())
}

pub(super) fn validate_candidate_review_input(input: &CandidateReviewInput) -> Result<()> {
    if input.schema_version != CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CANDIDATE_REVIEW_INPUT_SCHEMA_VERSION.to_string(),
            actual: input.schema_version.clone(),
        });
    }

    if input.candidate_id.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "candidate review candidate_id must not be empty".to_string(),
        ));
    }
    if input.reviewed_by.trim().is_empty() {
        return Err(MemoryEngineError::Validation(
            "candidate review reviewed_by must not be empty".to_string(),
        ));
    }
    Ok(())
}
