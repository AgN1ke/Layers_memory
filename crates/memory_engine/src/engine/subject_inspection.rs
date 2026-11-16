use std::cmp::Reverse;

use super::scope_inspection::{inspection_id_is_valid, validate_inspection_scope};
use super::*;
use crate::hypothesis::HypothesisEvidenceLink;
use crate::subject_memory::{
    SubjectMemoryItem, SubjectMemoryKind, SubjectMemoryView, SUBJECT_MEMORY_VIEW_SCHEMA,
};

impl<S: Storage> MemoryEngine<S> {
    pub fn inspect_subject(&self, scope_id: &str, subject_id: &str) -> Result<SubjectMemoryView> {
        validate_subject_target(scope_id, subject_id)?;
        self.with_scope_memory_write(scope_id, || {
            let mut view = SubjectMemoryView {
                schema_version: SUBJECT_MEMORY_VIEW_SCHEMA.into(),
                scope_id: scope_id.into(),
                subject_id: subject_id.into(),
                items: Vec::new(),
                incomplete: false,
            };
            let sources = self.read_subject_sources(scope_id)?;
            view.incomplete = sources.incomplete;
            for (event_id, event) in &sources.events {
                if !has_known_speaker(event.speaker.as_ref())
                    || event.quoted_event_ids.as_ref().is_none_or(|quotes| {
                        quotes.iter().any(|id| {
                            !has_known_speaker(
                                sources
                                    .events
                                    .get(id)
                                    .and_then(|source| source.speaker.as_ref()),
                            )
                        })
                    })
                {
                    view.incomplete = true;
                }
                if let Some(text) = event.memory_text.as_deref() {
                    // Typed quotations are stored as references. The event's
                    // authored text is independently readable without them.
                    add_subject_item(
                        &mut view,
                        event.speaker.as_ref(),
                        event.quoted_event_ids.is_some(),
                        SubjectMemoryKind::Event,
                        event_id,
                        &event.timestamp,
                        text,
                    );
                }
            }

            let archives = self.storage.read_archive_scoped(
                &ArchiveFilters::default(),
                Some(scope_id),
                None,
            )?;
            view.incomplete |= !archives.warnings.is_empty();
            let origins =
                SubjectOrigins::from_archives(&archives.items, subject_id, &sources, &mut view);
            self.add_subject_beliefs(&origins, &mut view)?;
            view.items.sort_by_cached_key(|item| {
                (
                    Reverse(OffsetDateTime::parse(&item.timestamp, &Rfc3339).ok()),
                    item.kind,
                    item.id.clone(),
                )
            });
            Ok(view)
        })
    }

    fn read_subject_sources(&self, scope_id: &str) -> Result<SubjectSources> {
        let mut sources = SubjectSources::default();
        for session in self
            .storage
            .read_sessions()?
            .items
            .into_iter()
            .filter(|session| session.effective_personality_scope_id() == scope_id)
        {
            let session_id = session.session_id.as_str();
            if !inspection_id_is_valid(session_id) || session.validate().is_err() {
                sources.incomplete = true;
                continue;
            }
            self.with_resource_lock(session_lock_key(session_id), || {
                match self.require_erasure_complete(MemoryErasureKind::DeleteSession, session_id) {
                    Ok(()) => (),
                    Err(MemoryEngineError::OperationIncomplete) => {
                        sources.incomplete = true;
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                }
                let Ok(metadata) = self.storage.read_session_metadata(session_id) else {
                    sources.incomplete = true;
                    return Ok(());
                };
                if metadata.session_id != session_id
                    || metadata.effective_personality_scope_id() != scope_id
                {
                    sources.incomplete = true;
                    return Ok(());
                }
                sources.sessions.insert(session_id.to_owned());
                let active = self
                    .storage
                    .read_session(session_id)
                    .map(|session| session.events);
                let archived = self.storage.read_session_archived_events(session_id);
                for collection in [active, archived] {
                    match collection {
                        Ok(events) => {
                            for event in events {
                                sources.add_event(event, scope_id, session_id);
                            }
                        }
                        Err(_) => sources.incomplete = true,
                    }
                }
                Ok(())
            })?;
        }
        sources
            .events
            .retain(|id, _| !sources.conflicting.contains(id));
        Ok(sources)
    }

    fn add_subject_beliefs(
        &self,
        origins: &SubjectOrigins<'_>,
        view: &mut SubjectMemoryView,
    ) -> Result<()> {
        let facts = self.storage.read_core_store_categories()?.items;
        let facts = facts
            .iter()
            .flat_map(|category| category.facts.iter())
            .filter(|fact| fact.scope.as_deref() == Some(view.scope_id.as_str()))
            .collect::<Vec<_>>();
        let own_facts = facts
            .iter()
            .filter(|fact| {
                speaker_matches(fact.subject.as_ref(), &view.subject_id)
                    && fact
                        .source_archive_ids
                        .iter()
                        .all(|id| origins.archives.contains(id.as_str()))
            })
            .map(|fact| fact.core_fact_id.as_str())
            .collect::<HashSet<_>>();

        let nodes = self.storage.read_hypothesis_nodes()?.items;
        let aliases = nodes
            .iter()
            .filter_map(|node| node.legacy_candidate_id.as_deref())
            .collect::<HashSet<_>>();
        let evidence_links = self.storage.read_hypothesis_evidence_links()?.items;
        let mut links_by_node = HashMap::<&str, Vec<&HypothesisEvidenceLink>>::new();
        for link in &evidence_links {
            links_by_node
                .entry(&link.hypothesis_id)
                .or_default()
                .push(link);
        }
        let mut safe_candidates = HashSet::new();
        let candidates = self.storage.read_legacy_candidate_beliefs()?.items;
        for candidate in &candidates {
            if candidate.core_scope.as_deref() != Some(view.scope_id.as_str())
                || aliases.contains(candidate.candidate_id.as_str())
            {
                continue;
            }
            let safe = origins.belief_is_independent(candidate, &own_facts, &[]);
            if safe && speaker_matches(candidate.subject.as_ref(), &view.subject_id) {
                safe_candidates.insert(candidate.candidate_id.as_str());
            }
            add_subject_item(
                view,
                candidate.subject.as_ref(),
                safe,
                SubjectMemoryKind::Candidate,
                &candidate.candidate_id,
                &candidate.created_at,
                &candidate.text,
            );
        }
        for node in &nodes {
            if node.core_scope.as_deref() != Some(view.scope_id.as_str()) {
                continue;
            }
            let links = links_by_node
                .get(node.hypothesis_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default();
            let safe = origins.belief_is_independent(&node.to_candidate(), &own_facts, links);
            if safe && speaker_matches(node.subject.as_ref(), &view.subject_id) {
                safe_candidates.insert(node.hypothesis_id.as_str());
                safe_candidates.extend(node.legacy_candidate_id.as_deref());
            }
            add_subject_item(
                view,
                node.subject.as_ref(),
                safe,
                SubjectMemoryKind::Hypothesis,
                &node.hypothesis_id,
                &node.created_at,
                &node.statement,
            );
        }
        for fact in facts {
            add_subject_item(
                view,
                fact.subject.as_ref(),
                own_facts.contains(fact.core_fact_id.as_str())
                    && fact
                        .source_candidate_id
                        .as_deref()
                        .is_none_or(|id| safe_candidates.contains(id)),
                SubjectMemoryKind::CoreFact,
                &fact.core_fact_id,
                &fact.created_at,
                &fact.text,
            );
        }
        Ok(())
    }
}

pub(super) fn validate_subject_target(scope_id: &str, subject_id: &str) -> Result<()> {
    validate_inspection_scope(scope_id)?;
    if subject_id.is_empty()
        || subject_id.trim() != subject_id
        || subject_id.chars().any(char::is_control)
    {
        return Err(MemoryEngineError::Validation(
            "subject_id must be a non-empty stable identity".into(),
        ));
    }
    Ok(())
}

fn speaker_matches(speaker: Option<&Speaker>, subject_id: &str) -> bool {
    speaker.is_some_and(|speaker| speaker.id == subject_id)
}

fn has_known_speaker(speaker: Option<&Speaker>) -> bool {
    speaker.is_some_and(|speaker| {
        !speaker.id.is_empty()
            && speaker.id.trim() == speaker.id
            && !speaker.id.chars().any(char::is_control)
    })
}

fn add_subject_item(
    view: &mut SubjectMemoryView,
    subject: Option<&Speaker>,
    independent: bool,
    kind: SubjectMemoryKind,
    id: &str,
    timestamp: &str,
    text: &str,
) {
    if !has_known_speaker(subject) {
        view.incomplete = true;
        return;
    }
    if !speaker_matches(subject, &view.subject_id) {
        return;
    }
    if !independent || OffsetDateTime::parse(timestamp, &Rfc3339).is_err() {
        view.incomplete = true;
    } else if !text.trim().is_empty() {
        view.items.push(SubjectMemoryItem {
            kind,
            id: id.into(),
            timestamp: timestamp.into(),
            text: text.into(),
        });
    }
}

#[derive(Default)]
struct SubjectSources {
    sessions: HashSet<String>,
    events: HashMap<String, SubjectSourceEvent>,
    conflicting: HashSet<String>,
    incomplete: bool,
}

#[derive(PartialEq, Eq)]
struct SubjectSourceEvent {
    session_id: String,
    timestamp: String,
    speaker: Option<Speaker>,
    memory_text: Option<String>,
    quoted_event_ids: Option<Vec<String>>,
}

impl SubjectSources {
    fn add_event(&mut self, event: StoredEvent, scope_id: &str, session_id: &str) {
        if event.session_id != session_id
            || event.effective_personality_scope_id() != scope_id
            || event.validate().is_err()
        {
            self.incomplete = true;
            return;
        }
        let source = SubjectSourceEvent {
            session_id: event.session_id,
            timestamp: event.timestamp,
            speaker: event.speaker,
            memory_text: event.memory_text,
            quoted_event_ids: event.quoted_event_ids,
        };
        if let Some(previous) = self.events.get(&event.event_id) {
            if previous != &source {
                self.conflicting.insert(event.event_id);
                self.incomplete = true;
            }
        } else {
            self.events.insert(event.event_id, source);
        }
    }
}

struct SubjectOrigins<'a> {
    sessions: &'a HashSet<String>,
    archives: HashSet<&'a str>,
    units: HashMap<&'a str, &'a MemoryUnit>,
}

impl<'a> SubjectOrigins<'a> {
    fn from_archives(
        entries: &'a [ArchiveEntry],
        subject_id: &str,
        sources: &'a SubjectSources,
        view: &mut SubjectMemoryView,
    ) -> Self {
        let mut result = Self {
            sessions: &sources.sessions,
            archives: HashSet::new(),
            units: HashMap::new(),
        };
        for archive in entries
            .iter()
            .filter(|archive| archive.status == ArchiveStatus::Complete)
        {
            let source_mentions_subject = |id: &String| {
                sources.events.get(id).is_some_and(|event| {
                    event.session_id == archive.source_session_id
                        && (speaker_matches(event.speaker.as_ref(), subject_id)
                            || event.quoted_event_ids.iter().flatten().any(|quote_id| {
                                sources.events.get(quote_id).is_some_and(|quoted| {
                                    speaker_matches(quoted.speaker.as_ref(), subject_id)
                                })
                            }))
                })
            };
            let source_is_owned = |id: &String| {
                sources.events.get(id).is_some_and(|event| {
                    event.session_id == archive.source_session_id
                        && speaker_matches(event.speaker.as_ref(), subject_id)
                        && event.quoted_event_ids.as_ref().is_some_and(|quotes| {
                            quotes.iter().all(|quote_id| {
                                sources.events.get(quote_id).is_some_and(|source| {
                                    source.quoted_event_ids.is_some()
                                        && speaker_matches(source.speaker.as_ref(), subject_id)
                                })
                            })
                        })
                })
            };
            let archive_sources = archive.source_event_ids.iter().collect::<HashSet<_>>();
            if !archive_sources.is_empty() && archive_sources.iter().all(|id| source_is_owned(id)) {
                result.archives.insert(&archive.archive_id);
            } else if archive.source_event_ids.iter().any(source_mentions_subject)
                || archive.memory_units.is_empty()
            {
                // Episode prose has no explicit subject. A mixed or unbound
                // archive may contain material about the requested person.
                view.incomplete = true;
            }
            for unit in &archive.memory_units {
                let independent = unit.archive_id == archive.archive_id
                    && unit.source_session_id == archive.source_session_id
                    && unit.effective_personality_scope_id() == view.scope_id
                    && !unit.source_event_ids.is_empty()
                    && unit
                        .source_event_ids
                        .iter()
                        .all(|id| archive_sources.contains(id) && source_is_owned(id));
                if independent && speaker_matches(unit.subject.as_ref(), subject_id) {
                    result.units.insert(&unit.memory_unit_id, unit);
                } else if unit.source_event_ids.iter().any(source_mentions_subject) {
                    view.incomplete = true;
                }
                let timestamp = unit
                    .source_event_ids
                    .iter()
                    .filter_map(|id| sources.events.get(id))
                    .map(|event| event.timestamp.as_str())
                    .min_by_key(|timestamp| OffsetDateTime::parse(timestamp, &Rfc3339).ok())
                    .unwrap_or(&unit.created_at);
                add_subject_item(
                    view,
                    unit.subject.as_ref(),
                    independent,
                    SubjectMemoryKind::MemoryUnit,
                    &unit.memory_unit_id,
                    timestamp,
                    &unit.thesis,
                );
            }
        }
        result
    }

    fn belief_is_independent(
        &self,
        candidate: &CandidateBelief,
        facts: &HashSet<&str>,
        links: &[&HypothesisEvidenceLink],
    ) -> bool {
        let has_evidence = !candidate.source_memory_unit_ids.is_empty()
            || !candidate.supporting_archive_ids.is_empty()
            || !links.is_empty();
        has_evidence
            && candidate
                .source_session_id
                .iter()
                .chain(&candidate.source_session_ids)
                .all(|id| self.sessions.contains(id.as_str()))
            && candidate
                .source_memory_unit_ids
                .iter()
                .all(|id| self.units.contains_key(id.as_str()))
            && candidate
                .supporting_archive_ids
                .iter()
                .chain(&candidate.contradicting_archive_ids)
                .all(|id| self.archives.contains(id.as_str()))
            && candidate
                .contradicted_core_fact_ids
                .iter()
                .all(|id| facts.contains(id.as_str()))
            && candidate.reinforcements.iter().all(|reinforcement| {
                reinforcement
                    .source_session_ids
                    .iter()
                    .all(|id| self.sessions.contains(id.as_str()))
                    && reinforcement
                        .source_memory_unit_ids
                        .iter()
                        .all(|id| self.units.contains_key(id.as_str()))
                    && reinforcement
                        .supporting_archive_ids
                        .iter()
                        .all(|id| self.archives.contains(id.as_str()))
            })
            && links.iter().all(|link| {
                link.resolution == HypothesisEvidenceResolution::Resolved
                    && self
                        .units
                        .get(link.memory_unit_id.as_str())
                        .is_some_and(|unit| {
                            link.archive_id
                                .as_deref()
                                .is_none_or(|id| id == unit.archive_id)
                                && link
                                    .source_session_id
                                    .as_deref()
                                    .is_none_or(|id| id == unit.source_session_id)
                        })
            })
    }
}
