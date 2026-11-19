use std::collections::BTreeSet;

use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::*;
use crate::archive::{render_compact_memory_from_units, ArchiveStatus};
use crate::erasure::MemoryErasureKind;
use crate::storage::MEMORY_UNIT_TASK_OUTCOME_SCHEMA;
use crate::types::MEMORY_UNIT_SCHEMA_VERSION;
use crate::vector::{VectorRow, VectorScopeStatus};

impl FileStorage {
    pub(super) fn apply_subject_erasure_records(&self, plan: &SubjectErasurePlan) -> Result<()> {
        validate_subject_plan(plan)?;
        self.ensure_layout()?;
        let erased_events = plan
            .event_ids_by_session
            .values()
            .flatten()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let suppressed_events = plan
            .suppressed_event_ids_by_session
            .values()
            .flatten()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let retired_events = erased_events.union(&suppressed_events).copied().collect();
        let erased_units = plan
            .records
            .memory_unit_ids
            .iter()
            .map(String::as_str)
            .collect();
        let superseded_archives =
            self.rewrite_subject_archives(plan, &retired_events, &erased_units)?;
        self.rewrite_subject_vectors(&plan.scope_id, &erased_units)?;
        self.retire_subject_ingest_receipts(plan)?;
        self.apply_erasure_records(&plan.records)?;
        self.scrub_subject_attribution_histories(plan)?;

        let sessions = plan
            .event_ids_by_session
            .keys()
            .chain(plan.suppressed_event_ids_by_session.keys())
            .chain(plan.detached_quote_event_ids_by_session.keys())
            .collect::<BTreeSet<_>>();
        for session in sessions {
            self.rewrite_subject_session(
                plan,
                session,
                &erased_events,
                &suppressed_events,
                &retired_events,
                &superseded_archives,
            )?;
        }
        Ok(())
    }

    fn scrub_subject_attribution_histories(&self, plan: &SubjectErasurePlan) -> Result<()> {
        let Some(scrub) = &plan.attribution_scrub else {
            return Ok(());
        };
        let mut by_category = BTreeMap::<&str, HashSet<&str>>::new();
        for reference in &scrub.facts {
            by_category
                .entry(&reference.category)
                .or_default()
                .insert(&reference.core_fact_id);
        }
        for (name, ids) in by_category {
            let mut category = self.read_core_store_category(name)?;
            let mut changed = false;
            for fact in &mut category.facts {
                if fact.scope.as_deref() == Some(&plan.scope_id)
                    && ids.contains(fact.core_fact_id.as_str())
                {
                    let before = fact.attribution_history.len();
                    fact.attribution_history
                        .retain(|revision| !revision.refers_to_subject(&scrub.subject_digest));
                    changed |= before != fact.attribution_history.len();
                }
            }
            if changed {
                self.write_core_store_category(&category)?;
            }
        }
        Ok(())
    }

    fn rewrite_subject_session(
        &self,
        plan: &SubjectErasurePlan,
        session: &str,
        erased_events: &HashSet<&str>,
        suppressed_events: &HashSet<&str>,
        retired_events: &HashSet<&str>,
        superseded_archives: &HashSet<String>,
    ) -> Result<()> {
        let active_path = self.events_path(session);
        let mut paths = Vec::new();
        collect_jsonl_files_shallow(&self.archived_events_dir(session), &mut paths)?;
        paths.sort();
        paths.push(active_path.clone());
        let mut events = BTreeMap::new();
        let mut archived_ids = BTreeSet::new();
        let mut active_ids = HashSet::new();
        for path in paths {
            let mut rows = match read_jsonl::<StoredEvent>(&path) {
                Ok(rows) => rows,
                // A planning pass already reports unreadable files as incomplete.
                Err(MemoryEngineError::Json(_)) if plan.incomplete => continue,
                Err(error) => return Err(error),
            };
            let original_len = rows.len();
            rows.retain(|event| {
                !valid_scope_event(event, &plan.scope_id, session)
                    || !erased_events.contains(event.event_id.as_str())
            });
            let mut changed = rows.len() != original_len;
            for event in &mut rows {
                if valid_scope_event(event, &plan.scope_id, session) {
                    changed |=
                        remove_subject_event_content(event, suppressed_events, retired_events);
                    if path == active_path {
                        active_ids.insert(event.event_id.clone());
                    } else {
                        archived_ids.insert(event.event_id.clone());
                    }
                }
                events.insert(event.event_id.clone(), event.clone());
            }
            if changed {
                atomic_write_jsonl(&path, &rows, true)?;
            }
        }

        if !self.session_dir(session).try_exists()? {
            return Ok(());
        }
        let mut events = events.into_values().collect::<Vec<_>>();
        events.sort_by(|left, right| {
            OffsetDateTime::parse(&left.timestamp, &Rfc3339)
                .ok()
                .cmp(&OffsetDateTime::parse(&right.timestamp, &Rfc3339).ok())
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        if events.is_empty() {
            remove_file_if_exists(&self.session_md_path(session))?;
        } else {
            let markdown = events
                .iter()
                .enumerate()
                .map(|(index, event)| event_markdown(session, event, index == 0))
                .collect::<String>();
            atomic_write_string(&self.session_md_path(session), &markdown, true)?;
        }
        if !self.session_json_path(session).try_exists()? {
            return Ok(());
        }
        let mut metadata = self.read_session_metadata_file(session)?;
        if metadata.effective_personality_scope_id() != plan.scope_id {
            return Ok(());
        }
        let own_events = events
            .iter()
            .filter(|event| valid_scope_event(event, &plan.scope_id, session))
            .collect::<Vec<_>>();
        metadata.event_count = own_events.len() as u64;
        metadata.summary = None;
        metadata.notes.clear();
        metadata.active_theme = own_events.iter().find_map(|event| event.theme.clone());
        metadata.tags = own_events
            .iter()
            .flat_map(|event| event.tags.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        metadata.archived_to.retain(|id| {
            !plan.records.archive_ids.contains(id) && !superseded_archives.contains(id)
        });
        metadata
            .archived_event_ids
            .retain(|id| !erased_events.contains(id.as_str()));
        archived_ids.extend(metadata.archived_event_ids);
        metadata.archived_event_ids = archived_ids.into_iter().collect();
        if metadata
            .active_sleep_task_id
            .as_ref()
            .is_some_and(|id| plan.records.task_ids.contains(id))
            || metadata
                .sleep_pending_event_ids
                .iter()
                .any(|id| retired_events.contains(id.as_str()))
        {
            metadata.active_sleep_task_id = None;
            metadata.sleep_pending_event_ids.clear();
        }
        if metadata.active_sleep_task_id.is_none() {
            metadata.status = if active_ids
                .iter()
                .all(|id| metadata.archived_event_ids.contains(id))
                && !metadata.archived_to.is_empty()
            {
                SessionStatus::Archived
            } else {
                metadata.closed_at = None;
                SessionStatus::Active
            };
        }
        metadata.validate()?;
        self.write_session_metadata_file(&metadata)
    }

    fn rewrite_subject_archives(
        &self,
        plan: &SubjectErasurePlan,
        retired_events: &HashSet<&str>,
        erased_units: &HashSet<&str>,
    ) -> Result<HashSet<String>> {
        let mut superseded = HashSet::new();
        if plan.suppressed_archive_ids.is_empty() {
            return Ok(superseded);
        }
        let selected_archives = plan
            .suppressed_archive_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut units_by_archive = HashMap::<String, Vec<MemoryUnit>>::new();
        for unit in self
            .read_memory_unit_records(|unit| {
                selected_archives.contains(unit.archive_id.as_str())
                    && unit.effective_personality_scope_id() == plan.scope_id
                    && unit.schema_version == MEMORY_UNIT_SCHEMA_VERSION
                    && !erased_units.contains(unit.memory_unit_id.as_str())
            })?
            .items
        {
            units_by_archive
                .entry(unit.archive_id.clone())
                .or_default()
                .push(unit);
        }
        for id in &plan.suppressed_archive_ids {
            let Some(path) = self.archive_entry_path_by_id_optional(id)? else {
                continue;
            };
            let mut entry: ArchiveEntry = read_json(&path)?;
            if entry.archive_id != *id || entry.effective_personality_scope_id() != plan.scope_id {
                return Err(MemoryEngineError::OperationResultUnavailable);
            }
            let _guard = lock_publication_receipt_retirement()?;
            let expected = match self.read_memory_unit_publication_receipt(id) {
                Ok(Some(value)) => Some(publication_members(&value, id).unwrap_or_default()),
                Ok(None) => None,
                Err(MemoryEngineError::Json(_)) => Some(BTreeSet::new()),
                Err(error) => return Err(error),
            };
            entry
                .source_event_ids
                .retain(|id| !retired_events.contains(id.as_str()));
            let present = entry
                .source_event_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let mut units = units_by_archive.remove(id).unwrap_or_default();
            units.retain(|unit| {
                unit.source_session_id == entry.source_session_id
                    && expected
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&unit.memory_unit_id))
                    && !unit.source_event_ids.is_empty()
                    && unit
                        .source_event_ids
                        .iter()
                        .all(|source| present.contains(source.as_str()))
            });
            sort_archive_memory_units(&mut units);
            let compact = render_compact_memory_from_units(&units);
            entry.gist = compact.clone().unwrap_or_default();
            entry.narrative.clear();
            entry.compact_memory = compact;
            entry.theme = None;
            entry.tags.clear();
            entry.facts.clear();
            entry.quotes.clear();
            entry.links.clear();
            entry.emotional_markers.clear();
            entry.topic_thread.clear();
            entry.personal_signals.clear();
            entry.relational_tone = None;
            entry.embedding = None;
            entry.embedding_model_id = None;
            entry.recall_count = 0;
            entry.last_recalled_at = None;
            entry.memory_units = units;
            if entry.status != ArchiveStatus::Complete || entry.memory_units.is_empty() {
                entry.status = ArchiveStatus::Superseded;
                superseded.insert(id.clone());
            }
            atomic_write_json(&path, &entry)?;

            // Membership is durable for availability. Replay credentials
            // describe the previous, partially erased publication.
            self.write_memory_unit_publication_receipt(
                id,
                &serde_json::json!({
                    "schema_version": MEMORY_UNIT_TASK_OUTCOME_SCHEMA,
                    "archive_id": id,
                    "memory_unit_ids": entry.memory_units.iter()
                        .map(|unit| unit.memory_unit_id.as_str()).collect::<Vec<_>>(),
                }),
            )?;
        }
        Ok(superseded)
    }

    fn rewrite_subject_vectors(&self, scope: &str, erased_units: &HashSet<&str>) -> Result<()> {
        if erased_units.is_empty() {
            return Ok(());
        }
        validate_scope_component(scope)?;
        let directory = self.vector_scope_dir(scope);
        if !directory.is_dir() {
            return Ok(());
        }
        let path = directory.join(".subject-erasure.json");
        let staged: StagedVectorIndex = match read_json(&path) {
            Ok(staged) => staged,
            Err(MemoryEngineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(index) = self.read_vector_index(scope)? else {
                    return self.clear_damaged_subject_vector_index(scope);
                };
                if index.manifest.state == VectorScopeStatus::Corrupt
                    || index.rows.len() != index.vectors.len()
                    || index
                        .rows
                        .iter()
                        .enumerate()
                        .any(|(position, row)| row.row != position)
                {
                    // The sidecar is derived. Damaged rows cannot be mapped
                    // safely to the vectors that must be removed.
                    return self.clear_damaged_subject_vector_index(scope);
                }
                if !index
                    .rows
                    .iter()
                    .any(|row| erased_units.contains(row.memory_unit_id.as_str()))
                    && !index
                        .tombstones
                        .iter()
                        .any(|row| erased_units.contains(row.memory_unit_id.as_str()))
                {
                    return Ok(());
                }
                let mut manifest = index.manifest;
                let mut rows = Vec::new();
                let mut vectors = Vec::new();
                for (mut row, vector) in index.rows.into_iter().zip(index.vectors) {
                    if !erased_units.contains(row.memory_unit_id.as_str()) {
                        row.row = rows.len();
                        rows.push(row);
                        vectors.push(vector);
                    }
                }
                let tombstones = index
                    .tombstones
                    .into_iter()
                    .filter(|row| !erased_units.contains(row.memory_unit_id.as_str()))
                    .collect();
                manifest.rows = rows.len();
                let staged = StagedVectorIndex {
                    manifest,
                    rows,
                    vectors,
                    tombstones,
                };
                atomic_write_json(&path, &staged)?;
                staged
            }
            Err(error) => return Err(error),
        };
        self.write_vector_index(
            scope,
            &VectorIndexData {
                manifest: staged.manifest,
                rows: staged.rows,
                vectors: staged.vectors,
                tombstones: staged.tombstones,
            },
        )?;
        remove_file_if_exists(&path)
    }

    fn clear_damaged_subject_vector_index(&self, scope: &str) -> Result<()> {
        for path in [
            self.vector_data_path(scope),
            self.vector_rows_path(scope),
            self.vector_tombstones_path(scope),
            self.vector_manifest_path(scope),
        ] {
            remove_file_if_exists(&path)?;
        }
        Ok(())
    }
}

fn valid_scope_event(event: &StoredEvent, scope: &str, session: &str) -> bool {
    event.session_id == session && event.effective_personality_scope_id() == scope
}

fn remove_subject_event_content(
    event: &mut StoredEvent,
    suppressed_events: &HashSet<&str>,
    retired_events: &HashSet<&str>,
) -> bool {
    if suppressed_events.contains(event.event_id.as_str()) {
        event.memory_text = None;
        event.payload = serde_json::Value::Null;
        event.tags.clear();
        event.theme = None;
        event.emotional_tone = None;
        event.weight_reason.clear();
        event.links.clear();
        return true;
    }
    let link_count = event.links.len();
    event
        .links
        .retain(|link| !retired_events.contains(link.target.as_str()));
    let mut changed = event.links.len() != link_count;
    if let Some(quotes) = &mut event.quoted_event_ids {
        let count = quotes.len();
        quotes.retain(|id| !retired_events.contains(id.as_str()));
        changed |= quotes.len() != count;
    }
    changed
}

fn publication_members(value: &serde_json::Value, archive_id: &str) -> Result<BTreeSet<String>> {
    if value["archive_id"].as_str() != Some(archive_id)
        || value["schema_version"].as_str() != Some(MEMORY_UNIT_TASK_OUTCOME_SCHEMA)
    {
        return Err(MemoryEngineError::OperationResultUnavailable);
    }
    let expected = value["memory_unit_ids"]
        .as_array()
        .ok_or(MemoryEngineError::OperationResultUnavailable)?;
    let ids = expected
        .iter()
        .map(|id| id.as_str().map(str::to_owned))
        .collect::<Option<BTreeSet<_>>>()
        .ok_or(MemoryEngineError::OperationResultUnavailable)?;
    if ids.len() != expected.len() {
        return Err(MemoryEngineError::OperationResultUnavailable);
    }
    Ok(ids)
}

fn validate_subject_plan(plan: &SubjectErasurePlan) -> Result<()> {
    validate_memory_erasure_plan(&plan.records)?;
    if plan.records.operation != MemoryErasureKind::PurgeScope
        || plan.records.target_id != plan.scope_id
        || !plan.records.session_ids.is_empty()
        || !plan.records.vector_scopes.is_empty()
    {
        return Err(MemoryEngineError::OperationResultUnavailable);
    }
    for (session, ids) in plan
        .event_ids_by_session
        .iter()
        .chain(&plan.suppressed_event_ids_by_session)
        .chain(&plan.detached_quote_event_ids_by_session)
    {
        validate_erasure_path_component("session id", session)?;
        for id in ids {
            validate_erasure_path_component("event id", id)?;
        }
    }
    for id in &plan.suppressed_archive_ids {
        validate_erasure_path_component("archive id", id)?;
    }
    if let Some(scrub) = &plan.attribution_scrub {
        for reference in &scrub.facts {
            validate_erasure_path_component("Core category", &reference.category)?;
            validate_erasure_path_component("Core fact id", &reference.core_fact_id)?;
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct StagedVectorIndex {
    manifest: VectorIndexManifest,
    rows: Vec<VectorRow>,
    vectors: Vec<Vec<f32>>,
    tombstones: Vec<VectorTombstone>,
}
