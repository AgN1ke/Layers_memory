use std::collections::{btree_map::Entry, BTreeMap};

use super::erasure_flow::sleep_run_task_ids;
use super::scope_inspection::inspection_id_is_valid;
use super::*;
use crate::erasure::SubjectErasurePlan;

impl<S: Storage> MemoryEngine<S> {
    pub(super) fn build_subject_erasure_plan(
        &self,
        scope: &str,
        subject: &str,
    ) -> Result<SubjectErasurePlan> {
        let mut incomplete = false;
        let session_metadata = records(self.storage.read_sessions()?, &mut incomplete);
        let archives = records(self.storage.read_archive_inventory()?, &mut incomplete)
            .into_iter()
            .filter(|archive| archive.effective_personality_scope_id() == scope)
            .collect::<Vec<_>>();
        let mut events = BTreeMap::new();
        let sessions = session_metadata
            .iter()
            .filter(|session| session.effective_personality_scope_id() == scope)
            .map(|session| session.session_id.as_str())
            .chain(
                archives
                    .iter()
                    .map(|archive| archive.source_session_id.as_str()),
            )
            .collect::<BTreeSet<_>>();
        for session_id in sessions {
            if !inspection_id_is_valid(session_id) {
                incomplete = true;
                continue;
            }
            let active = self.storage.read_session(session_id);
            let archived = self.storage.read_session_archived_events(session_id);
            if active.is_err() || archived.is_err() {
                incomplete = true;
            }
            for event in active
                .into_iter()
                .flat_map(|session| session.events)
                .chain(archived.into_iter().flatten())
                .filter(|event| {
                    event.session_id == session_id
                        && event.effective_personality_scope_id() == scope
                })
            {
                match events.entry(event.event_id.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(event);
                    }
                    Entry::Occupied(entry) if entry.get() != &event => {
                        // Physical IDs are globally unique. Conflicting copies
                        // cannot identify a record to erase safely.
                        return Err(MemoryEngineError::OperationResultUnavailable);
                    }
                    Entry::Occupied(_) => {}
                }
            }
        }

        let mut event_ids_by_session = BTreeMap::new();
        let mut suppressed_event_ids_by_session = BTreeMap::new();
        let mut detached_quote_event_ids_by_session = BTreeMap::new();
        let erased_events = events
            .values()
            .filter(|event| speaker_is(event.speaker.as_ref(), subject))
            .map(|event| {
                add_event_id(&mut event_ids_by_session, event);
                event.event_id.clone()
            })
            .collect::<BTreeSet<_>>();
        let mut affected_events = erased_events.clone();
        let mut opaque_replies = HashMap::<&str, Vec<&StoredEvent>>::new();
        for event in events.values() {
            if erased_events.contains(&event.event_id) {
                continue;
            }
            if (event.speaker.is_none() || event.quoted_event_ids.is_none())
                && event_has_content(event)
            {
                incomplete = true;
            }
            if let Some(quotes) = &event.quoted_event_ids {
                incomplete |= quotes.iter().any(|id| !events.contains_key(id));
                if overlaps(quotes, &erased_events)
                    || event
                        .links
                        .iter()
                        .any(|link| erased_events.contains(&link.target))
                {
                    add_event_id(&mut detached_quote_event_ids_by_session, event);
                }
            } else {
                for link in &event.links {
                    if matches!(link.kind.as_str(), "reply" | "reply_to" | "quote") {
                        opaque_replies
                            .entry(link.target.as_str())
                            .or_default()
                            .push(event);
                    }
                }
            }
        }
        // An opaque reply can copy earlier opaque quotes as well as the
        // author of its direct parent. Walk only explicit reply/quote edges.
        let mut copied_sources = erased_events.iter().map(String::as_str).collect::<Vec<_>>();
        while let Some(source) = copied_sources.pop() {
            if let Some(replies) = opaque_replies.remove(source) {
                for reply in replies {
                    if affected_events.insert(reply.event_id.clone()) {
                        add_event_id(&mut suppressed_event_ids_by_session, reply);
                        copied_sources.push(&reply.event_id);
                        incomplete = true;
                    }
                }
            }
        }

        let archives_by_id = archives
            .iter()
            .map(|archive| (archive.archive_id.as_str(), archive))
            .collect::<HashMap<_, _>>();
        let mut units = records(self.storage.read_memory_unit_inventory()?, &mut incomplete)
            .into_iter()
            .filter(|unit| unit.effective_personality_scope_id() == scope)
            .map(|unit| (unit.memory_unit_id.clone(), unit))
            .collect::<BTreeMap<_, _>>();
        // Embedded-only copies are not read authority, but still have to be
        // scrubbed. A present standalone record remains authoritative.
        for archive in &archives {
            for unit in &archive.memory_units {
                if unit_belongs_to_archive(unit, archive, scope) {
                    units
                        .entry(unit.memory_unit_id.clone())
                        .or_insert_with(|| unit.clone());
                }
            }
        }

        let mut affected_archives = archives
            .iter()
            .filter(|archive| {
                archive_references_events(archive, &affected_events)
                    || archive
                        .personal_signals
                        .iter()
                        .any(|signal| speaker_is(signal.subject.as_ref(), subject))
                    || archive.memory_units.iter().any(|unit| {
                        unit_belongs_to_archive(unit, archive, scope)
                            && (speaker_is(unit.subject.as_ref(), subject)
                                || overlaps(&unit.source_event_ids, &affected_events))
                    })
            })
            .map(|archive| archive.archive_id.clone())
            .collect::<BTreeSet<_>>();
        for unit in units.values() {
            if (speaker_is(unit.subject.as_ref(), subject)
                || overlaps(&unit.source_event_ids, &affected_events))
                && archives_by_id.contains_key(unit.archive_id.as_str())
            {
                affected_archives.insert(unit.archive_id.clone());
            }
        }
        let mut erased_units = BTreeSet::new();
        for unit in units.values() {
            let direct = speaker_is(unit.subject.as_ref(), subject);
            let derived = overlaps(&unit.source_event_ids, &affected_events);
            let uncertain =
                unit.source_event_ids.is_empty() && affected_archives.contains(&unit.archive_id);
            if direct || derived || uncertain {
                erased_units.insert(unit.memory_unit_id.clone());
                if !direct
                    && (uncertain
                        || unit
                            .source_event_ids
                            .iter()
                            .any(|id| !erased_events.contains(id)))
                {
                    incomplete = true;
                }
            } else if unit.subject.is_none()
                && (unit.source_event_ids.is_empty()
                    || unit
                        .source_event_ids
                        .iter()
                        .any(|id| events.get(id).is_none_or(|event| event.speaker.is_none())))
            {
                incomplete = true;
            }
        }

        let mut erased_archives = BTreeSet::new();
        let mut suppressed_archive_ids = Vec::new();
        let surviving_unit_archives = units
            .values()
            .filter(|unit| {
                !erased_units.contains(&unit.memory_unit_id)
                    && archives_by_id
                        .get(unit.archive_id.as_str())
                        .is_some_and(|archive| unit_belongs_to_archive(unit, archive, scope))
            })
            .map(|unit| unit.archive_id.as_str())
            .collect::<HashSet<_>>();
        for archive in &archives {
            if !affected_archives.contains(&archive.archive_id) {
                continue;
            }
            let has_surviving_event = archive
                .source_event_ids
                .iter()
                .any(|id| !erased_events.contains(id));
            if has_surviving_event || surviving_unit_archives.contains(archive.archive_id.as_str())
            {
                suppressed_archive_ids.push(archive.archive_id.clone());
                incomplete = true;
            } else {
                erased_archives.insert(archive.archive_id.clone());
            }
        }

        let graph = self.subject_graph_records(
            scope,
            subject,
            &affected_archives,
            &erased_units,
            &units,
            &mut incomplete,
        )?;
        let mut references = affected_events.clone();
        references.extend(
            detached_quote_event_ids_by_session
                .values()
                .flatten()
                .cloned(),
        );
        references.extend(affected_archives.iter().cloned());
        references.extend(erased_units.iter().cloned());
        references.extend(graph.candidate_ids.iter().cloned());
        references.extend(graph.node_ids.iter().cloned());
        references.extend(
            graph
                .core_facts
                .iter()
                .map(|fact| fact.core_fact_id.clone()),
        );

        let mut sleep_task_ids = BTreeSet::new();
        let mut task_ids = BTreeSet::new();
        for run in records(self.storage.load_sleep_runs()?, &mut incomplete) {
            if archives_by_id
                .get(run.archive_id.as_str())
                .is_some_and(|archive| {
                    archive.source_session_id == run.session_id
                        && (affected_archives.contains(&run.archive_id)
                            || overlaps(&archive.source_event_ids, &references))
                })
            {
                sleep_task_ids.insert(run.sleep_task_id.clone());
                task_ids.extend(sleep_run_task_ids(&run));
            }
        }
        for task in records(self.storage.load_all_tasks()?, &mut incomplete) {
            let touched = references_record(&task.inputs, &references)
                || task
                    .outcome
                    .as_ref()
                    .is_some_and(|outcome| references_record(outcome, &references));
            if task_ids.contains(&task.task_id) || touched {
                if explicitly_foreign(&task.inputs, scope) {
                    incomplete = true;
                } else {
                    task_ids.insert(task.task_id);
                }
            }
        }
        references.extend(task_ids.iter().cloned());

        let reflection_scopes = records(self.storage.read_reflection_states()?, &mut incomplete)
            .into_iter()
            .filter(|state| {
                state
                    .last_task_id
                    .as_ref()
                    .is_some_and(|id| task_ids.contains(id))
            })
            .map(|state| state.reflection_scope)
            .collect::<BTreeSet<_>>();
        let mut journal_operation_ids = BTreeSet::new();
        for operation in records(self.storage.load_journaled_operations()?, &mut incomplete) {
            if operation.op_type == JournalOperationType::IngestEvent {
                let event = &operation.intent["event"];
                let origin = event["personality_scope_id"]
                    .as_str()
                    .or(event["session_id"].as_str());
                if origin == Some(scope)
                    && (event["speaker"]["id"].as_str() == Some(subject)
                        || references_record(event, &affected_events))
                {
                    journal_operation_ids.insert(operation.op_id);
                }
            } else if matches!(
                operation.op_type,
                JournalOperationType::Sleep
                    | JournalOperationType::Migration
                    | JournalOperationType::MemoryUnitPublish
                    | JournalOperationType::CorePromotion
                    | JournalOperationType::ReflectionApply
                    | JournalOperationType::CandidateReview
            ) && references_record(&operation.intent, &references)
            {
                if explicitly_foreign(&operation.intent, scope) {
                    incomplete = true;
                } else {
                    journal_operation_ids.insert(operation.op_id);
                }
            }
        }

        suppressed_archive_ids.sort_unstable();
        suppressed_archive_ids.dedup();
        Ok(SubjectErasurePlan {
            scope_id: scope.to_string(),
            event_ids_by_session,
            suppressed_event_ids_by_session,
            detached_quote_event_ids_by_session,
            suppressed_archive_ids,
            records: MemoryErasurePlan {
                schema_version: MEMORY_ERASURE_PLAN_SCHEMA_VERSION.to_string(),
                operation: MemoryErasureKind::PurgeScope,
                target_id: scope.to_string(),
                session_ids: Vec::new(),
                archive_ids: erased_archives.into_iter().collect(),
                memory_unit_ids: erased_units.into_iter().collect(),
                core_facts: graph.core_facts,
                candidate_ids: graph.candidate_ids.into_iter().collect(),
                hypothesis_node_ids: graph.node_ids.into_iter().collect(),
                hypothesis_evidence_link_ids: graph.evidence_ids.into_iter().collect(),
                hypothesis_relation_ids: graph.relation_ids.into_iter().collect(),
                task_ids: task_ids.into_iter().collect(),
                sleep_task_ids: sleep_task_ids.into_iter().collect(),
                reflection_scopes: reflection_scopes.into_iter().collect(),
                vector_scopes: Vec::new(),
                journal_operation_ids: journal_operation_ids.into_iter().collect(),
            },
            incomplete,
        })
    }

    fn subject_graph_records(
        &self,
        scope: &str,
        subject: &str,
        affected_archives: &BTreeSet<String>,
        affected_units: &BTreeSet<String>,
        units: &BTreeMap<String, MemoryUnit>,
        incomplete: &mut bool,
    ) -> Result<SubjectGraphRecords> {
        let categories = records(self.storage.read_core_store_categories()?, incomplete);
        let candidates = records(self.storage.read_legacy_candidate_beliefs()?, incomplete);
        let nodes = records(self.storage.read_hypothesis_nodes()?, incomplete);
        let projected = nodes
            .iter()
            .map(HypothesisNode::to_candidate)
            .collect::<Vec<_>>();
        let evidence = records(self.storage.read_hypothesis_evidence_links()?, incomplete);
        let relations = records(self.storage.read_hypothesis_relations()?, incomplete);
        let owned_nodes = nodes
            .iter()
            .filter(|node| node.core_scope.as_deref() == Some(scope))
            .map(|node| node.hypothesis_id.as_str())
            .collect::<HashSet<_>>();
        let known_graph = candidates
            .iter()
            .chain(&projected)
            .filter(|candidate| candidate.core_scope.as_deref() == Some(scope))
            .map(|candidate| candidate.candidate_id.as_str())
            .chain(
                nodes
                    .iter()
                    .filter(|node| owned_nodes.contains(node.hypothesis_id.as_str()))
                    .filter_map(|node| node.legacy_candidate_id.as_deref()),
            )
            .collect::<HashSet<_>>();

        let mut core_ids = BTreeSet::new();
        let mut graph_ids = BTreeSet::new();
        for link in &evidence {
            if affected_units.contains(&link.memory_unit_id)
                || link.archive_id.as_ref().is_some_and(|id| {
                    affected_archives.contains(id)
                        && units
                            .get(&link.memory_unit_id)
                            .is_none_or(|unit| unit.archive_id != *id)
                })
            {
                if owned_nodes.contains(link.hypothesis_id.as_str()) {
                    graph_ids.insert(link.hypothesis_id.clone());
                }
                *incomplete = true;
            }
        }
        loop {
            let prior = (core_ids.len(), graph_ids.len());
            for candidate in candidates.iter().chain(&projected) {
                let direct = speaker_is(candidate.subject.as_ref(), subject);
                let derived = candidate_references(
                    candidate,
                    affected_archives,
                    affected_units,
                    units,
                    &core_ids,
                );
                if candidate.core_scope.as_deref() == Some(scope) {
                    if direct || derived {
                        graph_ids.insert(candidate.candidate_id.clone());
                        *incomplete |= !direct;
                    } else if candidate.subject.is_none()
                        && candidate.source_memory_unit_ids.is_empty()
                    {
                        *incomplete = true;
                    }
                } else if candidate.core_scope.is_none() && (direct || derived) {
                    *incomplete = true;
                }
            }
            // The graph projection and its legacy source can have separate
            // physical IDs, but are the same assertion.
            for node in &nodes {
                if owned_nodes.contains(node.hypothesis_id.as_str()) {
                    if let Some(alias) = &node.legacy_candidate_id {
                        if graph_ids.contains(&node.hypothesis_id) || graph_ids.contains(alias) {
                            graph_ids.insert(node.hypothesis_id.clone());
                            graph_ids.insert(alias.clone());
                        }
                    }
                }
            }
            for fact in categories.iter().flat_map(|category| &category.facts) {
                let direct = speaker_is(fact.subject.as_ref(), subject);
                let derived = fact
                    .source_candidate_id
                    .as_ref()
                    .is_some_and(|id| graph_ids.contains(id))
                    || (fact
                        .source_candidate_id
                        .as_deref()
                        .is_none_or(|id| !known_graph.contains(id))
                        && overlaps(&fact.source_archive_ids, affected_archives));
                if fact.scope.as_deref() == Some(scope) {
                    if direct || derived {
                        core_ids.insert(fact.core_fact_id.clone());
                        *incomplete |= !direct;
                    } else if fact.subject.is_none() && fact.source_archive_ids.is_empty() {
                        *incomplete = true;
                    }
                } else if fact.scope.is_none() && (direct || derived) {
                    *incomplete = true;
                }
            }
            if prior == (core_ids.len(), graph_ids.len()) {
                break;
            }
        }

        let core_facts = categories
            .iter()
            .flat_map(|category| {
                category
                    .facts
                    .iter()
                    .filter(|fact| {
                        fact.scope.as_deref() == Some(scope)
                            && core_ids.contains(&fact.core_fact_id)
                    })
                    .map(|fact| ErasureCoreFactRef {
                        category: category.category.clone(),
                        core_fact_id: fact.core_fact_id.clone(),
                    })
            })
            .collect();
        let candidate_ids = candidates
            .iter()
            .filter(|candidate| {
                candidate.core_scope.as_deref() == Some(scope)
                    && graph_ids.contains(&candidate.candidate_id)
            })
            .map(|candidate| candidate.candidate_id.clone())
            .collect();
        let node_ids = nodes
            .iter()
            .filter(|node| {
                owned_nodes.contains(node.hypothesis_id.as_str())
                    && graph_ids.contains(&node.hypothesis_id)
            })
            .map(|node| node.hypothesis_id.clone())
            .collect::<BTreeSet<_>>();
        let evidence_ids = evidence
            .into_iter()
            .filter(|link| {
                owned_nodes.contains(link.hypothesis_id.as_str())
                    && (node_ids.contains(&link.hypothesis_id)
                        || affected_units.contains(&link.memory_unit_id))
            })
            .map(|link| link.evidence_link_id)
            .collect();
        let relation_ids = relations
            .into_iter()
            .filter(|relation| {
                node_ids.contains(&relation.source_hypothesis_id)
                    || node_ids.contains(&relation.target_hypothesis_id)
            })
            .map(|relation| relation.relation_id)
            .collect();
        Ok(SubjectGraphRecords {
            core_facts,
            candidate_ids,
            node_ids,
            evidence_ids,
            relation_ids,
        })
    }
}

struct SubjectGraphRecords {
    core_facts: Vec<ErasureCoreFactRef>,
    candidate_ids: BTreeSet<String>,
    node_ids: BTreeSet<String>,
    evidence_ids: BTreeSet<String>,
    relation_ids: BTreeSet<String>,
}

fn candidate_references(
    candidate: &CandidateBelief,
    affected_archives: &BTreeSet<String>,
    affected_units: &BTreeSet<String>,
    units: &BTreeMap<String, MemoryUnit>,
    core_ids: &BTreeSet<String>,
) -> bool {
    overlaps(&candidate.source_memory_unit_ids, affected_units)
        || archive_only_evidence(
            &candidate.supporting_archive_ids,
            &candidate.source_memory_unit_ids,
            affected_archives,
            units,
        )
        || overlaps(&candidate.contradicting_archive_ids, affected_archives)
        || overlaps(&candidate.contradicted_core_fact_ids, core_ids)
        || candidate
            .promoted_core_fact_id
            .as_ref()
            .is_some_and(|id| core_ids.contains(id))
        || candidate.reinforcements.iter().any(|reinforcement| {
            overlaps(&reinforcement.source_memory_unit_ids, affected_units)
                || archive_only_evidence(
                    &reinforcement.supporting_archive_ids,
                    &reinforcement.source_memory_unit_ids,
                    affected_archives,
                    units,
                )
        })
}

fn archive_only_evidence(
    archive_ids: &[String],
    unit_ids: &[String],
    affected_archives: &BTreeSet<String>,
    units: &BTreeMap<String, MemoryUnit>,
) -> bool {
    archive_ids.iter().any(|archive_id| {
        affected_archives.contains(archive_id)
            && !unit_ids.iter().any(|id| {
                units
                    .get(id)
                    .is_some_and(|unit| unit.archive_id == *archive_id)
            })
    })
}

fn records<T>(collection: StorageCollection<T>, incomplete: &mut bool) -> Vec<T> {
    *incomplete |= !collection.warnings.is_empty();
    collection.items
}

fn speaker_is(speaker: Option<&Speaker>, subject: &str) -> bool {
    speaker.is_some_and(|speaker| speaker.id == subject)
}

fn event_has_content(event: &StoredEvent) -> bool {
    event
        .memory_text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        || match &event.payload {
            Value::Null => false,
            Value::String(text) => !text.trim().is_empty(),
            Value::Array(values) => !values.is_empty(),
            Value::Object(fields) => !fields.is_empty(),
            _ => true,
        }
}

fn add_event_id(groups: &mut BTreeMap<String, Vec<String>>, event: &StoredEvent) {
    groups
        .entry(event.session_id.clone())
        .or_default()
        .push(event.event_id.clone());
}

fn unit_belongs_to_archive(unit: &MemoryUnit, archive: &ArchiveEntry, scope: &str) -> bool {
    unit.archive_id == archive.archive_id
        && unit.source_session_id == archive.source_session_id
        && unit.effective_personality_scope_id() == scope
}

fn overlaps(ids: &[String], affected: &BTreeSet<String>) -> bool {
    ids.iter().any(|id| affected.contains(id))
}

fn archive_references_events(archive: &ArchiveEntry, affected: &BTreeSet<String>) -> bool {
    overlaps(&archive.source_event_ids, affected)
        || archive
            .facts
            .iter()
            .any(|fact| overlaps(&fact.source_event_ids, affected))
        || archive
            .quotes
            .iter()
            .filter_map(|quote| quote.source_event_id.as_ref())
            .any(|id| affected.contains(id))
        || archive
            .emotional_markers
            .iter()
            .any(|marker| overlaps(&marker.source_event_ids, affected))
        || archive
            .topic_thread
            .iter()
            .any(|topic| overlaps(&topic.source_event_ids, affected))
        || archive
            .personal_signals
            .iter()
            .any(|signal| overlaps(&signal.source_event_ids, affected))
        || archive
            .relational_tone
            .as_ref()
            .is_some_and(|tone| overlaps(&tone.source_event_ids, affected))
        || archive
            .links
            .iter()
            .any(|link| affected.contains(&link.target))
}

// References in durable task/journal shapes. Free text, provider output, and
// host payloads are never interpreted as identity or provenance.
fn references_record(value: &Value, affected: &BTreeSet<String>) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| references_record(value, affected)),
        Value::Object(fields) => fields.iter().any(|(key, value)| match key.as_str() {
            "payload" => false,
            "target" => value.as_str().is_some_and(|id| affected.contains(id)),
            key if reference_field(key) => {
                value.as_str().is_some_and(|id| affected.contains(id))
                    || value.as_array().is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_str().is_some_and(|id| affected.contains(id)))
                    })
            }
            _ => references_record(value, affected),
        }),
        _ => false,
    }
}

fn reference_field(key: &str) -> bool {
    [
        "event_id",
        "event_ids",
        "archive_id",
        "archive_ids",
        "memory_unit_id",
        "memory_unit_ids",
        "candidate_id",
        "candidate_ids",
        "hypothesis_id",
        "hypothesis_ids",
        "core_fact_id",
        "core_fact_ids",
        "task_id",
        "task_ids",
    ]
    .iter()
    .any(|field| {
        key == *field
            || key
                .strip_suffix(field)
                .is_some_and(|prefix| prefix.ends_with('_'))
    })
}

fn explicitly_foreign(value: &Value, scope: &str) -> bool {
    ["personality_scope_id", "core_scope", "scope"]
        .into_iter()
        .any(|key| value[key].as_str().is_some_and(|value| value != scope))
}
