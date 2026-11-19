use std::collections::hash_map::Entry;

use super::scope_inspection::inspection_id_is_valid;
use super::session_ops::is_local_quote_source;
use super::*;

impl<S: Storage> MemoryEngine<S> {
    /// Resolve just the IDs carried by the selected dialogue. The caller
    /// holds the scope erasure lock and releases the dialogue session lock
    /// before entering this method.
    pub(super) fn resolve_context_quote_sources_unlocked(
        &self,
        scope: &str,
        session: &SessionRecord,
        quote_ids: &HashSet<&str>,
    ) -> HashMap<String, StoredEvent> {
        if quote_ids.is_empty() {
            return HashMap::new();
        }

        let mut sources = HashMap::new();
        let current_session_id = &session.metadata.session_id;
        // The current Session snapshot was already read under its lock and
        // cannot be erased or rotated while this scope is held.
        self.collect_context_quote_sources(scope, session, quote_ids, &mut sources);
        if sources.len() < quote_ids.len() {
            if let Ok(sessions) = self.storage.read_sessions() {
                for metadata in sessions.items.into_iter().filter(|metadata| {
                    metadata.session_id != *current_session_id
                        && metadata.effective_personality_scope_id() == scope
                        && inspection_id_is_valid(&metadata.session_id)
                }) {
                    // Recheck a sibling's identity and deletion marker once it
                    // is locked. A damaged sibling does not discard other refs.
                    let _ = self.with_resource_lock(session_lock_key(&metadata.session_id), || {
                        self.require_erasure_complete(
                            MemoryErasureKind::DeleteSession,
                            &metadata.session_id,
                        )?;
                        let sibling = self.storage.read_session(&metadata.session_id)?;
                        if sibling.metadata.session_id == metadata.session_id {
                            self.collect_context_quote_sources(
                                scope,
                                &sibling,
                                quote_ids,
                                &mut sources,
                            );
                        }
                        Ok(())
                    });
                }
            }
        }
        sources
            .into_iter()
            .filter_map(|(id, source)| source.map(|source| (id, source)))
            .collect()
    }

    fn collect_context_quote_sources(
        &self,
        scope: &str,
        session: &SessionRecord,
        quote_ids: &HashSet<&str>,
        sources: &mut HashMap<String, Option<StoredEvent>>,
    ) {
        let metadata = &session.metadata;
        if metadata.effective_personality_scope_id() != scope || metadata.validate().is_err() {
            return;
        }
        let mut collect = |events: &[StoredEvent]| {
            for event in events
                .iter()
                .filter(|event| quote_ids.contains(event.event_id.as_str()))
            {
                let visible = is_local_quote_source(event, &metadata.session_id, scope)
                    && event.quoted_event_ids.is_some()
                    && event.memory_text.is_some();
                match sources.entry(event.event_id.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(visible.then(|| event.clone()));
                    }
                    Entry::Occupied(mut entry) => {
                        if !visible
                            || entry
                                .get()
                                .as_ref()
                                .is_none_or(|existing| !same_quote_content(existing, event))
                        {
                            entry.insert(None);
                        }
                    }
                }
            }
        };
        collect(&session.events);
        if !metadata.archived_event_index_complete
            || metadata
                .archived_event_ids
                .iter()
                .any(|id| quote_ids.contains(id.as_str()))
        {
            match self
                .storage
                .read_session_archived_events(&metadata.session_id)
            {
                Ok(archived) => collect(&archived),
                Err(_) => {
                    // If an indexed event has an unreadable archived copy,
                    // avoid falling back to a possibly stale active copy.
                    for id in &metadata.archived_event_ids {
                        if quote_ids.contains(id.as_str()) {
                            sources.insert(id.clone(), None);
                        }
                    }
                }
            }
        }
    }
}

fn same_quote_content(left: &StoredEvent, right: &StoredEvent) -> bool {
    left.session_id == right.session_id
        && left.speaker == right.speaker
        && left.timestamp == right.timestamp
        && left.memory_text == right.memory_text
}
