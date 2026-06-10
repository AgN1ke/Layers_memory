use super::*;

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
        self.with_resource_lock(session_lock_key(&session_id), || {
            let (initial_weight, weight_reason) =
                self.options.event_scoring.score_ingest_event(&event);
            let stored = StoredEvent::from_ingest(
                event,
                new_id("event")?,
                now_rfc3339()?,
                initial_weight,
                weight_reason,
            );

            self.storage.append_event(&stored.session_id, &stored)?;

            Ok(IngestResult {
                schema_version: INGEST_RESULT_SCHEMA_VERSION.to_string(),
                stored_event: stored,
            })
        })
    }

    pub(super) fn ensure_manifest(&self) -> Result<()> {
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

    pub fn pending_tasks(&self) -> Result<Vec<PendingTask>> {
        Ok(self
            .storage
            .load_tasks()?
            .into_items()
            .into_iter()
            .filter(|task| matches!(task.state, TaskState::Pending | TaskState::Submitted))
            .collect())
    }

    pub(super) fn archived_event_ids_for_session(
        &self,
        session_id: &str,
    ) -> Result<HashSet<String>> {
        self.with_resource_lock(session_lock_key(session_id), || {
            self.archived_event_ids_for_session_unlocked(session_id)
        })
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
            .read_archive(&ArchiveFilters::default())?
            .into_items()
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
                let mut entry = self.storage.read_archive_entry_by_id(&archive_id)?;
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
