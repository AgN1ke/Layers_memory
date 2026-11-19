use time::Duration;

use super::scope_inspection::{inspection_id_is_valid, validate_inspection_scope};
use super::transactional_support::serializable_digest;
use super::*;
use crate::core_store::{
    core_subject_digest, CoreFactAttributionInput, CoreFactAttributionResult,
    CoreFactAttributionRevision, CoreFactSubjectChange, CORE_FACT_ATTRIBUTION_INPUT_SCHEMA,
    CORE_FACT_ATTRIBUTION_RESULT_SCHEMA,
};

impl<S: Storage> MemoryEngine<S> {
    pub fn correct_core_fact_attribution(
        &self,
        input: CoreFactAttributionInput,
    ) -> Result<Option<CoreFactAttributionResult>> {
        let subject = validate_attribution_input(&input)?;
        let expected = Some(input.scope_id.as_str());
        let matches_fact = |fact: &CoreFact| {
            fact.core_fact_id == input.core_fact_id && fact.scope.as_deref() == expected
        };
        let request_digest = serializable_digest(&(
            CORE_FACT_ATTRIBUTION_INPUT_SCHEMA,
            &input.scope_id,
            &input.core_fact_id,
            &input.expected_updated_at,
            &subject,
        ))?;
        let operation_digest =
            serializable_digest(&(&input.scope_id, &input.core_fact_id, &input.operation_id))?;

        self.ensure_manifest()?;
        self.with_scope_memory_write(&input.scope_id, || {
            let mut categories = self
                .storage
                .read_core_store_categories()?
                .items
                .into_iter()
                .filter(|category| category.facts.iter().any(&matches_fact));
            let Some(snapshot) = categories.next() else {
                return Ok(None);
            };
            if categories.next().is_some() {
                return Err(MemoryEngineError::OperationConflict);
            }
            self.with_resource_lock(core_lock_key(&snapshot.category), || {
                let mut category = self.storage.read_core_store_category(&snapshot.category)?;
                let mut matches = category
                    .facts
                    .iter()
                    .enumerate()
                    .filter(|(_, fact)| matches_fact(fact));
                let Some((index, current)) = matches.next() else {
                    return Ok(None);
                };
                if matches.next().is_some() {
                    return Err(MemoryEngineError::OperationConflict);
                }
                if let Some(revision) = current
                    .attribution_history
                    .iter()
                    .find(|revision| revision.operation_digest == operation_digest)
                {
                    if revision.request_digest != request_digest {
                        return Err(MemoryEngineError::OperationConflict);
                    }
                    return Ok(Some(CoreFactAttributionResult {
                        schema_version: CORE_FACT_ATTRIBUTION_RESULT_SCHEMA.into(),
                        category: category.category,
                        fact: current.clone(),
                        applied_at: revision.applied_at.clone(),
                        already_applied: true,
                    }));
                }
                if current.updated_at != input.expected_updated_at {
                    return Err(MemoryEngineError::OperationStale);
                }
                if current.subject == subject {
                    return Err(MemoryEngineError::Validation(
                        "core fact already has the requested attribution".into(),
                    ));
                }
                let needle = normalize_match_text(&current.text);
                if category
                    .facts
                    .iter()
                    .enumerate()
                    .any(|(other_index, other)| {
                        other_index != index
                            && other.scope.as_deref() == expected
                            && same_subject(other.subject.as_ref(), subject.as_ref())
                            && normalize_match_text(&other.text) == needle
                    })
                {
                    return Err(MemoryEngineError::OperationConflict);
                }
                let applied_at = next_attribution_time(&current.updated_at, &category.updated_at)?;
                let fact = &mut category.facts[index];
                let scoped_digest = |subject: Option<&Speaker>| {
                    subject
                        .map(|speaker| core_subject_digest(&input.scope_id, &speaker.id))
                        .transpose()
                };
                fact.attribution_history.push(CoreFactAttributionRevision {
                    applied_at: applied_at.clone(),
                    operation_digest,
                    request_digest,
                    assertion_digest: serializable_digest(&(&input.scope_id, &needle))?,
                    previous_subject_digest: scoped_digest(fact.subject.as_ref())?,
                    assigned_subject_digest: scoped_digest(subject.as_ref())?,
                });
                fact.subject = subject;
                fact.attribution_changed_at = Some(applied_at.clone());
                fact.updated_at = applied_at.clone();
                let result = CoreFactAttributionResult {
                    schema_version: CORE_FACT_ATTRIBUTION_RESULT_SCHEMA.into(),
                    category: category.category.clone(),
                    fact: fact.clone(),
                    applied_at: applied_at.clone(),
                    already_applied: false,
                };
                category.updated_at = applied_at;
                self.storage.write_core_store_category(&category)?;
                Ok(Some(result))
            })
        })
    }
}

fn validate_attribution_input(input: &CoreFactAttributionInput) -> Result<Option<Speaker>> {
    if input.schema_version != CORE_FACT_ATTRIBUTION_INPUT_SCHEMA {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: CORE_FACT_ATTRIBUTION_INPUT_SCHEMA.into(),
            actual: input.schema_version.clone(),
        });
    }
    validate_inspection_scope(&input.scope_id)?;
    if !inspection_id_is_valid(&input.core_fact_id)
        || input.operation_id.len() > 200
        || !inspection_id_is_valid(&input.operation_id)
    {
        return Err(MemoryEngineError::Validation(
            "invalid core fact ID or attribution operation ID".into(),
        ));
    }
    if OffsetDateTime::parse(&input.expected_updated_at, &Rfc3339).is_err() {
        return Err(MemoryEngineError::Validation(
            "expected_updated_at must be RFC3339".into(),
        ));
    }
    match &input.change {
        CoreFactSubjectChange::Clear => Ok(None),
        CoreFactSubjectChange::Assign { subject } => {
            validate_speaker(subject, "core fact subject")?;
            if normalize_whitespace(&subject.id) != subject.id
                || subject.id.chars().any(char::is_control)
            {
                return Err(MemoryEngineError::Validation(
                    "core fact subject needs an exact stable ID".into(),
                ));
            }
            Ok(normalize_speaker(Some(subject)))
        }
    }
}

fn next_attribution_time(fact_time: &str, category_time: &str) -> Result<String> {
    let floor = [fact_time, category_time]
        .iter()
        .filter_map(|time| OffsetDateTime::parse(time, &Rfc3339).ok())
        .max();
    let now = OffsetDateTime::now_utc();
    let time = floor
        .filter(|prior| *prior >= now)
        .map_or(Some(now), |prior| prior.checked_add(Duration::NANOSECOND))
        .ok_or_else(|| MemoryEngineError::Storage("Core attribution time exhausted".into()))?;
    time.format(&Rfc3339)
        .map_err(|error| MemoryEngineError::Storage(error.to_string()))
}
