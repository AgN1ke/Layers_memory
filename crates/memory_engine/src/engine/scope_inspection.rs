use std::collections::BTreeMap;

use super::*;
use crate::inspection::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn inspect_scope(&self, scope_id: &str) -> Result<ScopedMemorySummary> {
        validate_inspection_scope(scope_id)?;
        self.with_scope_memory_write(scope_id, || {
            let mut session_ids = Vec::new();
            for session in self.storage.read_sessions()?.items {
                if session.effective_personality_scope_id() == scope_id
                    && inspection_id_is_valid(&session.session_id)
                    && self.session_is_visible(&session.session_id)?
                {
                    session_ids.push(session.session_id);
                }
            }
            session_ids.sort();
            session_ids.dedup();
            let sessions = session_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();

            let StorageCollection {
                mut items,
                warnings,
            } = self.storage.read_archive_scoped(
                &ArchiveFilters::default(),
                Some(scope_id),
                None,
            )?;
            items.sort_by(|left, right| {
                right
                    .time_range
                    .end
                    .cmp(&left.time_range.end)
                    .then_with(|| left.archive_id.cmp(&right.archive_id))
            });
            let mut archives = Vec::new();
            for entry in items {
                if inspection_id_is_valid(&entry.source_session_id)
                    && self.session_is_visible(&entry.source_session_id)?
                {
                    archives.push(entry);
                }
            }
            let operations = self.scoped_pending_operations(scope_id, &sessions, &archives)?;
            let mut pending_by_type = BTreeMap::new();
            for operation in &operations {
                *pending_by_type.entry(operation.task_type).or_insert(0) += 1;
            }
            let core_facts = self
                .storage
                .read_core_store_categories()?
                .items
                .into_iter()
                .flat_map(|category| category.facts)
                .filter(|fact| fact.scope.as_deref() == Some(scope_id))
                .count();
            let counts = ScopedMemoryCounts {
                sessions: session_ids.len(),
                complete_archives: archives
                    .iter()
                    .filter(|entry| entry.status == ArchiveStatus::Complete)
                    .count(),
                active_memory_units: archives
                    .iter()
                    .filter(|entry| entry.status == ArchiveStatus::Complete)
                    .flat_map(|entry| &entry.memory_units)
                    .filter(|unit| unit.status == MemoryUnitStatus::ActiveArchive)
                    .count(),
                core_facts,
                pending_by_type,
            };
            Ok(ScopedMemorySummary {
                schema_version: SCOPED_MEMORY_SUMMARY_SCHEMA.into(),
                scope_id: scope_id.into(),
                session_ids,
                archives: archives
                    .into_iter()
                    .map(|entry| ScopedArchiveSummary {
                        archive_id: entry.archive_id,
                        source_session_id: entry.source_session_id,
                        time_range: entry.time_range,
                        status: entry.status,
                        memory_units: entry
                            .memory_units
                            .into_iter()
                            .map(|unit| ScopedMemoryUnitSummary {
                                memory_unit_id: unit.memory_unit_id,
                                status: unit.status,
                                fidelity_status: unit.fidelity_status,
                                updated_at: unit.updated_at,
                            })
                            .collect(),
                    })
                    .collect(),
                operations,
                counts,
                warnings,
            })
        })
    }

    pub fn inspect_archive(
        &self,
        scope_id: &str,
        archive_id: &str,
    ) -> Result<Option<ScopedArchiveDetail>> {
        validate_inspection_scope(scope_id)?;
        validate_inspection_id("archive_id", archive_id)?;
        self.with_scope_memory_write(scope_id, || {
            let Some(mut projection) = self
                .storage
                .read_archive_detail_scoped(scope_id, archive_id)?
            else {
                return Ok(None);
            };
            let mut archive = projection.items.pop();
            let sources = if let Some(entry) = &mut archive {
                let session = entry.source_session_id.clone();
                validate_inspection_id("source_session_id", &session)?;
                self.with_resource_lock(session_lock_key(&session), || {
                    self.require_erasure_complete(MemoryErasureKind::DeleteSession, &session)?;
                    let sources = self.inspect_sources(
                        scope_id,
                        &session,
                        &entry.source_event_ids,
                        &mut projection.warnings,
                    );
                    let verified_ids = sources
                        .iter()
                        .map(|event| event.event_id.as_str())
                        .collect::<HashSet<_>>();
                    sanitize_archive_detail(entry, &verified_ids);
                    Ok(sources)
                })?
            } else {
                Vec::new()
            };
            Ok(Some(ScopedArchiveDetail {
                schema_version: SCOPED_ARCHIVE_DETAIL_SCHEMA.into(),
                scope_id: scope_id.into(),
                archive_id: archive_id.into(),
                archive,
                sources,
                warnings: projection.warnings,
            }))
        })
    }

    pub fn inspect_memory_unit(
        &self,
        scope_id: &str,
        memory_unit_id: &str,
    ) -> Result<Option<ScopedMemoryUnitDetail>> {
        validate_inspection_scope(scope_id)?;
        validate_inspection_id("memory_unit_id", memory_unit_id)?;
        self.with_scope_memory_write(scope_id, || {
            let Some(unit) = self.scoped_memory_unit(scope_id, memory_unit_id)? else {
                return Ok(None);
            };
            let session = unit.source_session_id.clone();
            self.with_resource_lock(session_lock_key(&session), || {
                self.require_erasure_complete(MemoryErasureKind::DeleteSession, &session)?;
                self.unit_detail(scope_id, unit)
            })
        })
    }

    pub fn remember_back_in_scope(
        &self,
        scope_id: &str,
        memory_unit_id: &str,
    ) -> Result<Option<MemoryUnit>> {
        validate_inspection_scope(scope_id)?;
        validate_inspection_id("memory_unit_id", memory_unit_id)?;
        let Some(snapshot) = self.scoped_memory_unit(scope_id, memory_unit_id)? else {
            return Ok(None);
        };
        self.ensure_manifest()?;
        self.with_memory_write(scope_id, &snapshot.source_session_id, || {
            let Some(unit) = self.scoped_memory_unit(scope_id, memory_unit_id)? else {
                return Ok(None);
            };
            if unit.archive_id != snapshot.archive_id
                || unit.source_session_id != snapshot.source_session_id
            {
                return Err(MemoryEngineError::OperationStale);
            }
            let Some(mut detail) = self.unit_detail(scope_id, unit.clone())? else {
                return Ok(None);
            };
            if detail.unit.thesis.is_empty() {
                return Err(MemoryEngineError::OperationResultUnavailable);
            }
            let restored = self.remember_back_unlocked(unit)?;
            detail.unit.status = restored.status;
            detail.unit.updated_at = restored.updated_at;
            detail.unit.tags = restored.tags;
            Ok(Some(detail.unit))
        })
    }

    fn scoped_memory_unit(&self, scope_id: &str, id: &str) -> Result<Option<MemoryUnit>> {
        Ok(self
            .storage
            .read_memory_units_by_id(&[id])?
            .remove(id)
            .filter(|unit| {
                unit.effective_personality_scope_id() == scope_id
                    && inspection_id_is_valid(&unit.source_session_id)
            }))
    }

    fn session_is_visible(&self, session: &str) -> Result<bool> {
        match self.require_erasure_complete(MemoryErasureKind::DeleteSession, session) {
            Ok(()) => Ok(true),
            Err(MemoryEngineError::OperationIncomplete) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn unit_detail(
        &self,
        scope_id: &str,
        mut unit: MemoryUnit,
    ) -> Result<Option<ScopedMemoryUnitDetail>> {
        let Some(mut projection) = self
            .storage
            .read_archive_detail_scoped(scope_id, &unit.archive_id)?
        else {
            return Ok(None);
        };
        let Some(parent) = projection.items.pop() else {
            return Ok(None);
        };
        if parent.source_session_id != unit.source_session_id {
            return Ok(None);
        }
        // The canonical parent projection can change between the first ID
        // lookup and acquiring the session lock.
        let Some(current) = parent
            .memory_units
            .into_iter()
            .find(|record| record.memory_unit_id == unit.memory_unit_id)
        else {
            return Ok(None);
        };
        unit = current;
        let expected = parent
            .source_event_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let allowed_ids = unit
            .source_event_ids
            .iter()
            .filter(|id| expected.contains(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if allowed_ids.len() != unit.source_event_ids.len() {
            projection.warnings.push(source_warning(
                "one or more citations are outside the parent archive",
            ));
        }
        let sources = self.inspect_sources(
            scope_id,
            &unit.source_session_id,
            &allowed_ids,
            &mut projection.warnings,
        );
        let verified_ids = sources
            .iter()
            .map(|event| event.event_id.as_str())
            .collect();
        sanitize_unit_detail(&mut unit, &verified_ids);
        Ok(Some(ScopedMemoryUnitDetail {
            schema_version: SCOPED_MEMORY_UNIT_DETAIL_SCHEMA.into(),
            scope_id: scope_id.into(),
            unit,
            sources,
            warnings: projection.warnings,
        }))
    }

    fn inspect_sources(
        &self,
        scope_id: &str,
        session_id: &str,
        source_ids: &[String],
        warnings: &mut Vec<StorageReadWarning>,
    ) -> Vec<ScopedSourceEvent> {
        if source_ids.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        match self.storage.read_session_archived_events(session_id) {
            Ok(archived) => events.extend(archived),
            Err(_) => warnings.push(source_warning("archived source events could not be read")),
        }
        match self.storage.read_session(session_id) {
            Ok(active) => events.extend(active.events),
            Err(_) => warnings.push(source_warning("active source events could not be read")),
        }
        let expected = source_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut resolved = BTreeMap::new();
        let mut conflicting = HashSet::new();
        for event in events {
            if !expected.contains(event.event_id.as_str())
                || event.session_id != session_id
                || event.effective_personality_scope_id() != scope_id
                || event.validate().is_err()
            {
                continue;
            }
            if let Some(previous) = resolved.get(&event.event_id) {
                if previous != &event {
                    conflicting.insert(event.event_id);
                }
            } else {
                resolved.insert(event.event_id.clone(), event);
            }
        }
        resolved.retain(|id, _| !conflicting.contains(id));
        if resolved.len() < expected.len() {
            warnings.push(source_warning("one or more cited events are unavailable"));
        }
        let mut sources = resolved
            .into_values()
            .map(|event| ScopedSourceEvent {
                event_id: event.event_id,
                event_type: event.event_type,
                source: event.source,
                timestamp: event.timestamp,
                speaker: event.speaker,
                memory_text: event.memory_text,
            })
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        sources
    }
}

fn inspection_id_is_valid(id: &str) -> bool {
    !id.trim().is_empty()
        && id.trim() == id
        && !matches!(id, "." | "..")
        && !id.ends_with('.')
        && !id.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*'])
        && !id.chars().any(char::is_control)
}

fn validate_inspection_scope(scope_id: &str) -> Result<()> {
    if scope_id.trim().is_empty() || scope_id.chars().any(char::is_control) {
        return Err(MemoryEngineError::Validation(
            "scope_id must be provided for scoped inspection".into(),
        ));
    }
    Ok(())
}

fn validate_inspection_id(label: &str, id: &str) -> Result<()> {
    if inspection_id_is_valid(id) {
        Ok(())
    } else {
        Err(MemoryEngineError::Validation(format!(
            "invalid {label} for scoped inspection"
        )))
    }
}

fn source_warning(error: &str) -> StorageReadWarning {
    StorageReadWarning {
        collection: "source event".into(),
        path: String::new(),
        error: error.into(),
    }
}

fn sanitize_archive_detail(entry: &mut ArchiveEntry, verified_ids: &HashSet<&str>) {
    let sources_incomplete = entry
        .source_event_ids
        .iter()
        .any(|id| !verified_ids.contains(id.as_str()));
    entry
        .source_event_ids
        .retain(|id| verified_ids.contains(id.as_str()));
    for unit in &mut entry.memory_units {
        sanitize_unit_detail(unit, verified_ids);
    }
    entry.links.clear();
    entry.embedding = None;
    entry.embedding_model_id = None;
    if sources_incomplete {
        entry.gist.clear();
        entry.narrative.clear();
        entry.compact_memory = None;
    } else if !entry.memory_units.is_empty() {
        entry.compact_memory = render_compact_memory_from_units(&entry.memory_units);
    }
    let linked_source_is_visible = |ids: &[String]| {
        (!sources_incomplete || !ids.is_empty())
            && ids.iter().all(|id| verified_ids.contains(id.as_str()))
    };
    entry
        .facts
        .retain(|fact| linked_source_is_visible(&fact.source_event_ids));
    entry.quotes.retain(|quote| {
        quote
            .source_event_id
            .as_deref()
            .map_or(!sources_incomplete, |id| verified_ids.contains(id))
    });
    entry
        .emotional_markers
        .retain(|marker| linked_source_is_visible(&marker.source_event_ids));
    entry
        .topic_thread
        .retain(|topic| linked_source_is_visible(&topic.source_event_ids));
    entry
        .personal_signals
        .retain(|signal| linked_source_is_visible(&signal.source_event_ids));
    if entry
        .relational_tone
        .as_ref()
        .is_some_and(|tone| !linked_source_is_visible(&tone.source_event_ids))
    {
        entry.relational_tone = None;
    }
}

fn sanitize_unit_detail(unit: &mut MemoryUnit, verified_ids: &HashSet<&str>) {
    let source_count = unit.source_event_ids.len();
    unit.source_event_ids
        .retain(|id| verified_ids.contains(id.as_str()));
    if unit.source_event_ids.len() != source_count {
        unit.thesis.clear();
        unit.evidence = None;
        unit.fidelity_review = None;
        if let Some(review) = &mut unit.forget_review {
            review.reason.clear();
        }
    }
    if let Some(review) = &mut unit.forget_review {
        review.task_id = None;
    }
    if unit.fidelity_review.as_ref().is_some_and(|review| {
        review.memory_unit_id != unit.memory_unit_id || review.archive_id != unit.archive_id
    }) {
        unit.fidelity_review = None;
    }
    unit.last_reflection_operation_id = None;
}
