use super::*;

pub(super) fn is_local_quote_source(event: &StoredEvent, session_id: &str, scope: &str) -> bool {
    event.session_id == session_id
        && event.effective_personality_scope_id() == scope
        && event
            .speaker
            .as_ref()
            .is_some_and(|speaker| !speaker.id.trim().is_empty())
        && event.validate().is_ok()
}

impl<S: Storage> MemoryEngine<S> {
    pub(super) fn with_resource_lock<T, F>(&self, key: String, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T>,
    {
        let resource = self.locks.resource(&key)?;
        let _guard = lock_resource(&resource, &key)?;
        f()
    }

    pub fn ingest(&self, event: IngestEvent) -> Result<IngestResult> {
        validate_ingest_event(&event)?;
        self.ensure_manifest()?;

        let session_id = event.session_id.clone();
        let scope = event
            .personality_scope_id
            .as_deref()
            .unwrap_or(&session_id)
            .to_owned();
        self.with_memory_write(&scope, &session_id, || {
            let stored = self.prepare_ingest_event(event)?;

            self.storage.append_event(&stored.session_id, &stored)?;

            Ok(IngestResult {
                schema_version: INGEST_RESULT_SCHEMA_VERSION.to_string(),
                stored_event: stored,
            })
        })
    }

    pub(super) fn prepare_ingest_event(&self, event: IngestEvent) -> Result<StoredEvent> {
        self.require_local_quote_sources(&event)?;
        let (initial_weight, weight_reason) = self.options.event_scoring.score_ingest_event(&event);
        Ok(StoredEvent::from_ingest(
            event,
            new_id("event")?,
            now_rfc3339()?,
            initial_weight,
            weight_reason,
        ))
    }

    fn require_local_quote_sources(&self, event: &IngestEvent) -> Result<()> {
        let Some(ids) = event
            .quoted_event_ids
            .as_ref()
            .filter(|ids| !ids.is_empty())
        else {
            return Ok(());
        };
        let scope = event
            .personality_scope_id
            .as_deref()
            .unwrap_or(&event.session_id);
        let mut missing = ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut sessions = self
            .storage
            .read_sessions()?
            .items
            .into_iter()
            .filter(|metadata| {
                metadata.effective_personality_scope_id() == scope
                    && metadata.session_id != event.session_id
                    && super::scope_inspection::inspection_id_is_valid(&metadata.session_id)
            })
            .map(|metadata| metadata.session_id)
            .collect::<Vec<_>>();
        sessions.push(event.session_id.clone());
        for session in sessions.into_iter().rev() {
            for source in self.read_session_events_with_archived(&session)? {
                if is_local_quote_source(&source, &session, scope) {
                    missing.remove(source.event_id.as_str());
                }
            }
            if missing.is_empty() {
                return Ok(());
            }
        }
        Err(MemoryEngineError::Validation(
            "quoted event is unavailable in the requested scope".into(),
        ))
    }

    pub(super) fn ensure_manifest(&self) -> Result<()> {
        self.ensure_manifest_initialized()?;
        self.ensure_pending_memory_operations_recovered()
    }

    pub(super) fn ensure_manifest_initialized(&self) -> Result<()> {
        if self.manifest_initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        self.with_resource_lock("manifest".to_string(), || {
            if self.manifest_initialized.load(Ordering::Acquire) {
                return Ok(());
            }
            if !self.storage.manifest_exists()? {
                let now = now_rfc3339()?;
                let manifest = default_manifest(&now);
                self.storage.write_manifest(&manifest)?;
            }
            self.manifest_initialized.store(true, Ordering::Release);
            Ok(())
        })
    }

    fn ensure_pending_memory_operations_recovered(&self) -> Result<()> {
        if self.memory_operations_recovered.load(Ordering::Acquire) {
            return Ok(());
        }
        self.with_resource_lock("memory_operations_recovery".to_string(), || {
            if self.memory_operations_recovered.load(Ordering::Acquire) {
                return Ok(());
            }
            self.recover_pending_memory_operations_unlocked()?;
            self.memory_operations_recovered
                .store(true, Ordering::Release);
            Ok(())
        })
    }

    pub fn pending_tasks(&self) -> Result<Vec<PendingTask>> {
        self.ensure_manifest()?;
        Ok(self
            .storage
            .load_tasks()?
            .into_items()
            .into_iter()
            .filter(|task| matches!(task.state, TaskState::Pending | TaskState::Submitted))
            .filter(|task| task.schema_version == PENDING_TASK_SCHEMA_VERSION)
            .filter(|task| match task.task_type {
                TaskType::MemoryFidelityPass => super::fidelity_flow::fidelity_task_has_basis(task),
                TaskType::ForgetReview => super::forgetting_flow::forget_task_has_basis(task),
                TaskType::SleepCompression | TaskType::MemoryUnitPass => task.inputs["events"]
                    .as_array()
                    .is_some_and(|events| !events.is_empty()),
                _ => true,
            })
            .collect())
    }

    /// Read a coherent session snapshot while local writes or Sleep rotate it.
    pub fn read_session(&self, session_id: &str) -> Result<SessionRecord> {
        self.with_resource_lock(session_lock_key(session_id), || {
            self.read_local_session_unlocked(session_id)
        })
    }

    /// Enumerate local sessions for a trusted, store-wide maintenance worker.
    pub fn list_local_sessions(&self) -> Result<Vec<String>> {
        let mut sessions = Vec::new();
        for metadata in self.storage.read_sessions()?.items {
            if metadata.validate().is_err()
                || !super::scope_inspection::inspection_id_is_valid(&metadata.session_id)
            {
                continue;
            }
            let erasure = self
                .require_erasure_complete(
                    MemoryErasureKind::PurgeScope,
                    metadata.effective_personality_scope_id(),
                )
                .and_then(|_| {
                    self.require_erasure_complete(
                        MemoryErasureKind::DeleteSession,
                        &metadata.session_id,
                    )
                });
            match erasure {
                Ok(()) => sessions.push(metadata.session_id),
                Err(MemoryEngineError::OperationIncomplete) => continue,
                Err(error) => return Err(error),
            }
        }
        sessions.sort();
        sessions.dedup();
        Ok(sessions)
    }

    /// Read active events still eligible for Sleep in a trusted local session.
    pub fn read_unarchived_session(&self, session_id: &str) -> Result<SessionRecord> {
        if !super::scope_inspection::inspection_id_is_valid(session_id) {
            return Err(MemoryEngineError::Validation("invalid session ID".into()));
        }
        self.with_session_memory_write(session_id, || {
            let mut session = self.read_local_session_unlocked(session_id)?;
            let covered = self.archived_event_ids_for_session_unlocked(session_id)?;
            session
                .events
                .retain(|event| !covered.contains(&event.event_id));
            Ok(session)
        })
    }

    fn read_local_session_unlocked(&self, session_id: &str) -> Result<SessionRecord> {
        self.require_erasure_complete(MemoryErasureKind::DeleteSession, session_id)?;
        let session = self.storage.read_session(session_id)?;
        self.require_erasure_complete(
            MemoryErasureKind::PurgeScope,
            session.metadata.effective_personality_scope_id(),
        )?;
        Ok(session)
    }

    pub(super) fn archived_event_ids_for_session_unlocked(
        &self,
        session_id: &str,
    ) -> Result<HashSet<String>> {
        let metadata = self.storage.read_session_metadata(session_id)?;
        if metadata.archived_event_index_complete {
            return Ok(metadata.archived_event_ids.into_iter().collect());
        }
        self.rebuild_archived_event_index_for_session_unlocked(session_id)
    }

    pub(super) fn rebuild_archived_event_index_for_session_unlocked(
        &self,
        session_id: &str,
    ) -> Result<HashSet<String>> {
        let mut archive_ids = BTreeSet::new();
        let mut event_ids = BTreeSet::new();

        for entry in self
            .storage
            .read_sleep_archives(session_id)?
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .filter(|entry| entry.status == ArchiveStatus::Complete)
        {
            archive_ids.insert(entry.archive_id);
            event_ids.extend(entry.source_event_ids);
        }

        let mut metadata = self.storage.read_session_metadata(session_id)?;
        metadata.archived_to = archive_ids.into_iter().collect();
        metadata.archived_event_ids = event_ids.iter().cloned().collect();
        metadata.archived_event_index_complete = true;
        self.storage.write_session_metadata(&metadata)?;

        Ok(event_ids.into_iter().collect())
    }

    pub(super) fn record_completed_archive_in_session_metadata_unlocked(
        &self,
        session_id: &str,
        archive_entry: &ArchiveEntry,
    ) -> Result<HashSet<String>> {
        if archive_entry.source_session_id != session_id
            || archive_entry.status != ArchiveStatus::Complete
        {
            return self.archived_event_ids_for_session_unlocked(session_id);
        }

        let mut metadata = self.storage.read_session_metadata(session_id)?;
        if !metadata.archived_event_index_complete {
            return self.rebuild_archived_event_index_for_session_unlocked(session_id);
        }

        let mut archive_ids = metadata.archived_to.into_iter().collect::<BTreeSet<_>>();
        archive_ids.insert(archive_entry.archive_id.clone());

        let mut event_ids = metadata
            .archived_event_ids
            .into_iter()
            .collect::<BTreeSet<_>>();
        event_ids.extend(archive_entry.source_event_ids.iter().cloned());

        metadata.archived_to = archive_ids.into_iter().collect();
        metadata.archived_event_ids = event_ids.iter().cloned().collect();
        metadata.archived_event_index_complete = true;
        self.storage.write_session_metadata(&metadata)?;

        Ok(event_ids.into_iter().collect())
    }

    pub(super) fn read_session_events_with_archived(
        &self,
        session_id: &str,
    ) -> Result<Vec<StoredEvent>> {
        let mut events = self.storage.read_session_archived_events(session_id)?;
        events.extend(self.storage.read_session(session_id)?.events);

        let mut seen = HashSet::new();
        events.retain(|event| seen.insert(event.event_id.clone()));
        events.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        Ok(events)
    }

    pub(super) fn recall_stats_snapshot(&self) -> Result<HashMap<String, RecallStatDelta>> {
        let stats = self.recall_stats.lock().map_err(|_| {
            MemoryEngineError::Storage("recall stats mutex was poisoned".to_string())
        })?;
        Ok(stats.clone())
    }

    pub(super) fn record_recall_stats(
        &self,
        archive_ids: &[String],
        recalled_at: &str,
    ) -> Result<()> {
        if archive_ids.is_empty() {
            return Ok(());
        }

        let mut stats = self.recall_stats.lock().map_err(|_| {
            MemoryEngineError::Storage("recall stats mutex was poisoned".to_string())
        })?;
        for archive_id in archive_ids {
            let delta = stats.entry(archive_id.clone()).or_default();
            delta.added_count = delta.added_count.saturating_add(1);
            delta.last_recalled_at =
                newest_timestamp(delta.last_recalled_at.as_deref(), Some(recalled_at));
        }
        self.recall_calls_since_flush
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub(super) fn recall_flush_due(&self) -> bool {
        let interval = self.options.recall.stats_flush_interval;
        interval > 0 && self.recall_calls_since_flush.load(Ordering::Relaxed) >= interval
    }

    pub(super) fn restore_recall_stats(
        &self,
        pending: HashMap<String, RecallStatDelta>,
    ) -> Result<()> {
        if pending.is_empty() {
            return Ok(());
        }

        let mut stats = self.recall_stats.lock().map_err(|_| {
            MemoryEngineError::Storage("recall stats mutex was poisoned".to_string())
        })?;
        for (archive_id, delta) in pending {
            let current = stats.entry(archive_id).or_default();
            current.added_count = current.added_count.saturating_add(delta.added_count);
            current.last_recalled_at = newest_timestamp(
                current.last_recalled_at.as_deref(),
                delta.last_recalled_at.as_deref(),
            );
        }
        Ok(())
    }

    pub fn flush_recall_stats(&self) -> Result<usize> {
        let _flush_guard = self.recall_stats_flush_lock.lock().map_err(|_| {
            MemoryEngineError::Storage("recall stats flush mutex was poisoned".to_string())
        })?;

        let (mut pending, taken_call_count) = {
            let mut stats = self.recall_stats.lock().map_err(|_| {
                MemoryEngineError::Storage("recall stats mutex was poisoned".to_string())
            })?;
            if stats.is_empty() {
                self.recall_calls_since_flush.store(0, Ordering::Relaxed);
                return Ok(0);
            }
            let taken_call_count = self.recall_calls_since_flush.swap(0, Ordering::Relaxed);
            (mem::take(&mut *stats), taken_call_count)
        };

        let mut flushed = 0usize;
        for archive_id in pending.keys().cloned().collect::<Vec<_>>() {
            let Some(delta) = pending.remove(&archive_id) else {
                continue;
            };
            let result = self.with_resource_lock(archive_lock_key(&archive_id), || {
                let mut entry = self.storage.read_archive_entry_raw(&archive_id)?;
                entry.recall_count = entry.recall_count.saturating_add(delta.added_count);
                entry.last_recalled_at = newest_timestamp(
                    entry.last_recalled_at.as_deref(),
                    delta.last_recalled_at.as_deref(),
                );
                self.storage.update_archive_entry(&archive_id, &entry)
            });
            if let Err(err) = result {
                pending.insert(archive_id, delta);
                self.restore_recall_stats(pending)?;
                self.recall_calls_since_flush
                    .fetch_add(taken_call_count, Ordering::Relaxed);
                return Err(err);
            }
            flushed += 1;
        }
        Ok(flushed)
    }
}
