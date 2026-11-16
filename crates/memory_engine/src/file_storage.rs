use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::archive::{ArchiveEntry, ArchiveFilters, MemoryUnit};
use crate::core_store::{CandidateBelief, CoreStoreCategory};
use crate::erasure::MemoryErasurePlan;
use crate::event::StoredEvent;
use crate::hypothesis::{
    evidence_links_from_candidate, validate_hypothesis_storage_id, HypothesisEvidenceLink,
    HypothesisNode, HypothesisRelation, ResolvedHypothesisEvidenceSource,
};
use crate::journal::{JournalOperation, JournalState};
use crate::llm::SleepRun;
use crate::manifest::Manifest;
use crate::reflection::ReflectionState;
use crate::session::{SessionMetadata, SessionRecord, SessionStatus};
use crate::storage::{memory_unit_publication_key, Storage, StorageCollection, StorageReadWarning};
use crate::tasks::TaskState;
use crate::types::{
    CORE_STORE_SCHEMA_VERSION, MEMORY_ERASURE_PLAN_SCHEMA_VERSION, REFLECTION_STATE_SCHEMA_VERSION,
    SESSION_SCHEMA_VERSION,
};
use crate::vector::{
    VectorAppendRecord, VectorIndexData, VectorIndexManifest, VectorTombstone,
    VECTOR_INDEX_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

mod archive_reader;
mod idempotent_ingest;
pub(crate) use idempotent_ingest::JournaledIngest;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
static PUBLICATION_RECEIPT_RETIREMENT: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct FileStorage {
    root: PathBuf,
    host_id: String,
}

impl FileStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            host_id: "unknown".to_string(),
        }
    }

    pub fn with_host_id(root: impl Into<PathBuf>, host_id: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            host_id: host_id.into(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.root.join("sessions"))?;
        fs::create_dir_all(self.root.join("archive"))?;
        fs::create_dir_all(self.root.join("archive").join("units"))?;
        fs::create_dir_all(self.root.join("archive").join("forgotten"))?;
        fs::create_dir_all(self.root.join("archive").join("vectors"))?;
        fs::create_dir_all(self.root.join("core").join("store"))?;
        fs::create_dir_all(self.root.join("core").join("candidates"))?;
        fs::create_dir_all(self.root.join("core").join("hypotheses").join("nodes"))?;
        fs::create_dir_all(self.root.join("core").join("hypotheses").join("evidence"))?;
        fs::create_dir_all(self.root.join("core").join("hypotheses").join("relations"))?;
        fs::create_dir_all(self.root.join("tasks"))?;
        fs::create_dir_all(self.root.join("tasks").join("completed"))?;
        fs::create_dir_all(self.root.join("runs").join("sleep"))?;
        fs::create_dir_all(self.root.join("runs").join("reflection"))?;
        fs::create_dir_all(self.root.join("journal"))?;
        Ok(())
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join("sessions").join(session_id)
    }

    fn session_json_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("session.json")
    }

    fn session_md_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("session.md")
    }

    fn events_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("events.jsonl")
    }

    fn archived_events_dir(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("archived")
    }

    fn next_archived_events_path(&self, session_id: &str) -> Result<PathBuf> {
        let dir = self.archived_events_dir(session_id);
        fs::create_dir_all(&dir)?;

        let mut index = 1usize;
        loop {
            let path = dir.join(format!("events-{index:06}.jsonl"));
            if !path.exists() {
                return Ok(path);
            }
            index += 1;
        }
    }

    fn archive_entry_path(&self, entry: &ArchiveEntry) -> PathBuf {
        let (year, month) = year_month_from_timestamp(&entry.created_at);
        self.root
            .join("archive")
            .join(year)
            .join(month)
            .join(format!("{}.json", entry.archive_id))
    }

    fn archive_entry_path_by_id(&self, archive_id: &str) -> Result<PathBuf> {
        let mut files = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut files)?;

        files
            .into_iter()
            .find(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(archive_id))
            .ok_or_else(|| {
                MemoryEngineError::Storage(format!("archive entry not found: {archive_id}"))
            })
    }

    fn archive_entry_path_by_id_optional(&self, archive_id: &str) -> Result<Option<PathBuf>> {
        let mut files = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut files)?;
        Ok(files
            .into_iter()
            .find(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(archive_id)))
    }

    fn core_store_path(&self, category: &str) -> PathBuf {
        self.root
            .join("core")
            .join("store")
            .join(format!("{category}.json"))
    }

    fn memory_unit_path(&self, unit_id: &str) -> PathBuf {
        self.root
            .join("archive")
            .join("units")
            .join(format!("{unit_id}.json"))
    }

    fn candidate_path(&self, candidate_id: &str) -> PathBuf {
        self.root
            .join("core")
            .join("candidates")
            .join(format!("{candidate_id}.json"))
    }

    fn hypothesis_node_path(&self, hypothesis_id: &str) -> PathBuf {
        self.root
            .join("core")
            .join("hypotheses")
            .join("nodes")
            .join(format!("{hypothesis_id}.json"))
    }

    fn hypothesis_evidence_path(&self, evidence_link_id: &str) -> PathBuf {
        self.root
            .join("core")
            .join("hypotheses")
            .join("evidence")
            .join(format!("{evidence_link_id}.json"))
    }

    fn hypothesis_relation_path(&self, relation_id: &str) -> PathBuf {
        self.root
            .join("core")
            .join("hypotheses")
            .join("relations")
            .join(format!("{relation_id}.json"))
    }

    fn task_path(&self, task_id: &str) -> PathBuf {
        self.root.join("tasks").join(format!("{task_id}.json"))
    }

    fn completed_task_path(&self, task_id: &str) -> PathBuf {
        self.root
            .join("tasks")
            .join("completed")
            .join(format!("{task_id}.json"))
    }

    fn sleep_run_path(&self, sleep_task_id: &str) -> PathBuf {
        self.root
            .join("runs")
            .join("sleep")
            .join(format!("{sleep_task_id}.json"))
    }

    fn reflection_state_path(&self, scope: &str) -> PathBuf {
        self.root
            .join("runs")
            .join("reflection")
            .join(format!("{scope}.json"))
    }

    fn journal_path(&self, op_id: &str) -> PathBuf {
        self.root.join("journal").join(format!("{op_id}.json"))
    }

    fn vector_scope_dir(&self, scope: &str) -> PathBuf {
        self.root.join("archive").join("vectors").join(scope)
    }

    fn vector_manifest_path(&self, scope: &str) -> PathBuf {
        self.vector_scope_dir(scope).join("manifest.json")
    }

    fn vector_rows_path(&self, scope: &str) -> PathBuf {
        self.vector_scope_dir(scope).join("rows.jsonl")
    }

    fn vector_tombstones_path(&self, scope: &str) -> PathBuf {
        self.vector_scope_dir(scope).join("tombstones.jsonl")
    }

    fn vector_data_path(&self, scope: &str) -> PathBuf {
        self.vector_scope_dir(scope).join("vectors.f32")
    }

    fn read_session_metadata_file(&self, session_id: &str) -> Result<SessionMetadata> {
        read_json(&self.session_json_path(session_id))
    }

    fn empty_session_metadata(&self, session_id: &str) -> SessionMetadata {
        SessionMetadata {
            schema_version: SESSION_SCHEMA_VERSION.to_string(),
            session_id: session_id.to_string(),
            personality_scope_id: None,
            host_id: self.host_id.clone(),
            status: SessionStatus::Active,
            created_at: "1970-01-01T00:00:00Z".to_string(),
            updated_at: "1970-01-01T00:00:00Z".to_string(),
            closed_at: None,
            event_count: 0,
            summary: None,
            active_theme: None,
            tags: Vec::new(),
            archived_to: Vec::new(),
            archived_event_ids: Vec::new(),
            archived_event_index_complete: true,
            active_sleep_task_id: None,
            sleep_pending_event_ids: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn write_session_metadata_file(&self, metadata: &SessionMetadata) -> Result<()> {
        atomic_write_json_relaxed(&self.session_json_path(&metadata.session_id), metadata)
    }

    fn upsert_session_metadata(&self, session_id: &str, event: &StoredEvent) -> Result<()> {
        let metadata = self.session_metadata_after_event(session_id, event)?;
        self.write_session_metadata_file(&metadata)
    }

    fn session_metadata_after_event(
        &self,
        session_id: &str,
        event: &StoredEvent,
    ) -> Result<SessionMetadata> {
        let path = self.session_json_path(session_id);
        let mut metadata = if path.exists() {
            let metadata = self.read_session_metadata_file(session_id)?;
            metadata.validate()?;
            metadata
        } else {
            SessionMetadata {
                schema_version: SESSION_SCHEMA_VERSION.to_string(),
                session_id: session_id.to_string(),
                personality_scope_id: event.personality_scope_id.clone(),
                host_id: self.host_id.clone(),
                status: SessionStatus::Active,
                created_at: event.received_at.clone(),
                updated_at: event.received_at.clone(),
                closed_at: None,
                event_count: 0,
                summary: None,
                active_theme: event.theme.clone(),
                tags: Vec::new(),
                archived_to: Vec::new(),
                archived_event_ids: Vec::new(),
                archived_event_index_complete: true,
                active_sleep_task_id: None,
                sleep_pending_event_ids: Vec::new(),
                notes: Vec::new(),
            }
        };

        if metadata.effective_personality_scope_id() != event.effective_personality_scope_id() {
            return Err(MemoryEngineError::Validation(format!(
                "session {session_id} belongs to personality scope {}, not {}",
                metadata.effective_personality_scope_id(),
                event.effective_personality_scope_id()
            )));
        }

        metadata.updated_at = event.received_at.clone();
        metadata.event_count += 1;

        if metadata.active_theme.is_none() {
            metadata.active_theme = event.theme.clone();
        }

        for tag in &event.tags {
            if !metadata.tags.contains(tag) {
                metadata.tags.push(tag.clone());
            }
        }

        metadata.validate()?;

        Ok(metadata)
    }

    fn update_session_markdown(&self, session_id: &str, event: &StoredEvent) -> Result<()> {
        let path = self.session_md_path(session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let needs_header = !path.exists() || fs::metadata(&path)?.len() == 0;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        file.write_all(event_markdown(session_id, event, needs_header).as_bytes())?;
        Ok(())
    }
}

fn event_markdown(session_id: &str, event: &StoredEvent, needs_header: bool) -> String {
    let mut text = if needs_header {
        format!("---\nschema_version: session_view.v1\nsession_id: {session_id}\nstatus: active\n---\n\n# Сесія {session_id}\n\n## Події\n\n")
    } else {
        String::new()
    };
    text.push_str(&format!(
        "- {} {}: {}\n",
        event.timestamp,
        event.event_type,
        event_summary(event)
    ));
    text
}

impl Storage for FileStorage {
    fn manifest_exists(&self) -> Result<bool> {
        Ok(self.manifest_path().exists())
    }

    fn read_manifest(&self) -> Result<Manifest> {
        read_json(&self.manifest_path())
    }

    fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.manifest_path(), manifest)
    }

    fn read_session(&self, session_id: &str) -> Result<SessionRecord> {
        self.require_complete_ingest(session_id)?;
        let metadata = if self.session_json_path(session_id).exists() {
            self.read_session_metadata_file(session_id)?
        } else if self.session_dir(session_id).exists() {
            return Err(MemoryEngineError::Storage(format!(
                "session metadata is missing for materialized session: {session_id}"
            )));
        } else {
            self.empty_session_metadata(session_id)
        };
        let events = read_jsonl(&self.events_path(session_id))?;
        self.require_complete_ingest(session_id)?;
        Ok(SessionRecord { metadata, events })
    }

    fn read_sessions(&self) -> Result<StorageCollection<SessionMetadata>> {
        self.ensure_layout()?;
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        for entry in fs::read_dir(self.root.join("sessions"))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            // Scope selection uses atomic metadata, independent of an in-flight
            // event append. Erasure locks its own target before reading event data.
            let path = entry.path().join("session.json");
            if !path.exists() {
                continue;
            }
            if let Some(metadata) = read_json_from_collection::<SessionMetadata>(
                self,
                &path,
                "session metadata",
                &mut warnings,
            )? {
                sessions.push(metadata);
            }
        }
        sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        Ok(StorageCollection {
            items: sessions,
            warnings,
        })
    }

    fn read_session_metadata(&self, session_id: &str) -> Result<SessionMetadata> {
        self.require_complete_ingest(session_id)?;
        if self.session_json_path(session_id).exists() {
            self.read_session_metadata_file(session_id)
        } else if self.session_dir(session_id).exists() {
            Err(MemoryEngineError::Storage(format!(
                "session metadata is missing for materialized session: {session_id}"
            )))
        } else {
            Ok(self.empty_session_metadata(session_id))
        }
    }

    fn write_session_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        self.require_complete_ingest(&metadata.session_id)?;
        self.ensure_layout()?;
        if let Some(parent) = self.session_json_path(&metadata.session_id).parent() {
            fs::create_dir_all(parent)?;
        }
        self.write_session_metadata_file(metadata)
    }

    fn read_session_archived_events(&self, session_id: &str) -> Result<Vec<StoredEvent>> {
        self.require_complete_ingest(session_id)?;
        let mut files = Vec::new();
        collect_jsonl_files_shallow(&self.archived_events_dir(session_id), &mut files)?;
        files.sort();

        let mut events = Vec::new();
        for path in files {
            events.extend(read_jsonl(&path)?);
        }
        Ok(events)
    }

    fn append_event(&self, session_id: &str, event: &StoredEvent) -> Result<()> {
        self.require_complete_ingest(session_id)?;
        self.ensure_layout()?;
        event.validate()?;
        if event.session_id != session_id {
            return Err(MemoryEngineError::Validation(format!(
                "event session_id {} does not match append target {session_id}",
                event.session_id
            )));
        }
        let metadata_path = self.session_json_path(session_id);
        if metadata_path.exists() {
            let metadata = self.read_session_metadata_file(session_id)?;
            metadata.validate()?;
            if metadata.effective_personality_scope_id() != event.effective_personality_scope_id() {
                return Err(MemoryEngineError::Validation(format!(
                    "session {session_id} belongs to personality scope {}, not {}",
                    metadata.effective_personality_scope_id(),
                    event.effective_personality_scope_id()
                )));
            }
        }
        fs::create_dir_all(self.session_dir(session_id))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path(session_id))?;

        let line = serde_json::to_string(event)?;
        writeln!(file, "{line}")?;
        file.sync_all()?;

        self.upsert_session_metadata(session_id, event)?;
        self.update_session_markdown(session_id, event)?;

        Ok(())
    }

    fn rotate_session_events(
        &self,
        session_id: &str,
        covered_event_ids: &[String],
    ) -> Result<usize> {
        self.require_complete_ingest(session_id)?;
        if covered_event_ids.is_empty() {
            return Ok(0);
        }

        self.ensure_layout()?;
        fs::create_dir_all(self.session_dir(session_id))?;
        let active_path = self.events_path(session_id);
        let active_events = read_jsonl::<StoredEvent>(&active_path)?;
        if active_events.is_empty() {
            return Ok(0);
        }

        let covered = covered_event_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut archived = Vec::new();
        let mut active = Vec::new();
        for event in active_events {
            if covered.contains(event.event_id.as_str()) {
                archived.push(event);
            } else {
                active.push(event);
            }
        }

        if archived.is_empty() {
            return Ok(0);
        }

        let archived_path = self.next_archived_events_path(session_id)?;
        write_jsonl_sync(&archived_path, &archived)?;
        atomic_write_jsonl(&active_path, &active, true)?;
        Ok(archived.len())
    }

    fn write_archive_entry(&self, entry: &ArchiveEntry) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.archive_entry_path(entry), entry)
    }

    fn update_archive_entry(&self, archive_id: &str, entry: &ArchiveEntry) -> Result<()> {
        self.ensure_layout()?;
        let path = self.archive_entry_path_by_id(archive_id)?;
        atomic_write_json(&path, entry)
    }

    fn read_archive_entry_by_id(&self, archive_id: &str) -> Result<ArchiveEntry> {
        self.read_projected_archive(archive_id)
    }

    fn read_archive_entry_raw(&self, archive_id: &str) -> Result<ArchiveEntry> {
        read_json(&self.archive_entry_path_by_id(archive_id)?)
    }

    fn read_archive_inventory(&self) -> Result<StorageCollection<ArchiveEntry>> {
        let mut paths = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut paths)?;
        let mut items = Vec::new();
        let mut warnings = Vec::new();
        for path in paths {
            if let Some(entry) = read_json_from_collection(self, &path, "archive", &mut warnings)? {
                items.push(entry);
            }
        }
        Ok(StorageCollection { items, warnings })
    }

    fn read_sleep_archives(&self, session_id: &str) -> Result<Vec<ArchiveEntry>> {
        let mut paths = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut paths)?;
        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        for path in paths {
            if let Some(entry) =
                read_json_from_collection::<ArchiveEntry>(self, &path, "archive", &mut warnings)?
            {
                if entry.source_session_id == session_id {
                    entries.push(entry);
                }
            }
        }
        Ok(entries)
    }

    fn read_archive(&self, filters: &ArchiveFilters) -> Result<StorageCollection<ArchiveEntry>> {
        self.read_archive_collection(filters, None, None)
    }

    fn read_archive_scoped(
        &self,
        filters: &ArchiveFilters,
        scope: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<StorageCollection<ArchiveEntry>> {
        self.read_archive_collection(filters, scope, session_id)
    }

    fn write_memory_unit(&self, unit: &MemoryUnit) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.memory_unit_path(&unit.memory_unit_id), unit)
    }

    fn read_memory_unit_by_id(&self, memory_unit_id: &str) -> Result<MemoryUnit> {
        self.ensure_layout()?;
        self.read_canonical_unit(memory_unit_id)
    }

    fn read_memory_units_by_id(&self, ids: &[&str]) -> Result<HashMap<String, MemoryUnit>> {
        self.read_canonical_units(ids)
    }

    fn read_memory_units(&self) -> Result<StorageCollection<MemoryUnit>> {
        let StorageCollection { items, warnings } = self.read_archive(&ArchiveFilters::default())?;
        let mut items = items
            .into_iter()
            .flat_map(|archive| archive.memory_units)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.memory_unit_id.cmp(&right.memory_unit_id));
        Ok(StorageCollection { items, warnings })
    }

    fn read_memory_unit_inventory(&self) -> Result<StorageCollection<MemoryUnit>> {
        let mut inventory = self.read_memory_unit_records(|_| true)?;
        inventory
            .items
            .sort_by(|left, right| left.memory_unit_id.cmp(&right.memory_unit_id));
        Ok(inventory)
    }

    fn read_memory_units_for_archive(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>> {
        self.read_projected_archive_units(archive_id)
    }

    fn read_memory_units_for_archive_raw(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>> {
        let mut units = self.read_memory_unit_records(|unit| unit.archive_id == archive_id)?;
        sort_archive_memory_units(&mut units.items);
        Ok(units)
    }

    fn read_memory_unit_publication_receipt(
        &self,
        archive_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        match read_json(&self.memory_unit_publication_receipt_path(archive_id)) {
            Ok(receipt) => Ok(Some(receipt)),
            Err(MemoryEngineError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn write_memory_unit_publication_receipt(
        &self,
        archive_id: &str,
        receipt: &serde_json::Value,
    ) -> Result<()> {
        atomic_write_json(
            &self.memory_unit_publication_receipt_path(archive_id),
            receipt,
        )
    }

    fn read_core_store_category(&self, category: &str) -> Result<CoreStoreCategory> {
        let path = self.core_store_path(category);
        if !path.exists() {
            return Ok(CoreStoreCategory {
                schema_version: CORE_STORE_SCHEMA_VERSION.to_string(),
                category: category.to_string(),
                updated_at: "unknown".to_string(),
                facts: Vec::new(),
            });
        }
        read_json(&path)
    }

    fn read_core_store_categories(&self) -> Result<StorageCollection<CoreStoreCategory>> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join("core").join("store"), &mut files)?;

        let mut categories = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(category) = read_json_from_collection::<CoreStoreCategory>(
                self,
                &path,
                "core store",
                &mut warnings,
            )? {
                categories.push(category);
            }
        }
        categories.sort_by(|left: &CoreStoreCategory, right: &CoreStoreCategory| {
            left.category.cmp(&right.category)
        });

        Ok(StorageCollection {
            items: categories,
            warnings,
        })
    }

    fn write_core_store_category(&self, category: &CoreStoreCategory) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.core_store_path(&category.category), category)
    }

    fn write_candidate_belief(&self, candidate: &CandidateBelief) -> Result<()> {
        self.ensure_layout()?;
        let node_path = self.hypothesis_node_path(&candidate.candidate_id);
        let mut node = if node_path.exists() {
            read_json::<HypothesisNode>(&node_path)?
        } else {
            HypothesisNode::from_candidate(candidate)
        };
        node.merge_candidate_projection(candidate);
        node.validate_basic()?;
        atomic_write_json(&node_path, &node)?;
        for mut link in evidence_links_from_candidate(candidate, |memory_unit_id| {
            self.read_canonical_unit(memory_unit_id).ok().map(|unit| {
                ResolvedHypothesisEvidenceSource {
                    archive_id: unit.archive_id,
                    source_session_id: unit.source_session_id,
                }
            })
        }) {
            let link_path = self.hypothesis_evidence_path(&link.evidence_link_id);
            if link_path.exists() {
                let existing = read_json::<HypothesisEvidenceLink>(&link_path)?;
                link.merge_existing_graph_fields(existing);
            }
            link.validate_basic()?;
            atomic_write_json(&link_path, &link)?;
        }
        atomic_write_json(&self.candidate_path(&candidate.candidate_id), candidate)
    }

    fn read_candidate_belief(&self, candidate_id: &str) -> Result<CandidateBelief> {
        self.ensure_layout()?;
        validate_hypothesis_storage_id("candidate_id", candidate_id)?;
        let node_path = self.hypothesis_node_path(candidate_id);
        if node_path.exists() {
            let node: HypothesisNode = read_json(&node_path)?;
            node.validate_basic()?;
            return Ok(node.to_candidate());
        }
        let path = self.candidate_path(candidate_id);
        if !path.exists() {
            return Err(MemoryEngineError::Storage(format!(
                "candidate belief not found: {candidate_id}"
            )));
        }
        read_json(&path)
    }

    fn read_candidate_belief_optional(
        &self,
        candidate_id: &str,
    ) -> Result<Option<CandidateBelief>> {
        self.ensure_layout()?;
        validate_hypothesis_storage_id("candidate_id", candidate_id)?;
        let node_path = self.hypothesis_node_path(candidate_id);
        if node_path.exists() {
            let node: HypothesisNode = read_json(&node_path)?;
            node.validate_basic()?;
            return Ok(Some(node.to_candidate()));
        }
        let path = self.candidate_path(candidate_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json(&path)?))
    }

    fn read_legacy_candidate_beliefs(&self) -> Result<StorageCollection<CandidateBelief>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files(&self.root.join("core").join("candidates"), &mut files)?;
        let mut candidates = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(candidate) = read_json_from_collection::<CandidateBelief>(
                self,
                &path,
                "candidate belief",
                &mut warnings,
            )? {
                candidates.push(candidate);
            }
        }
        candidates.sort_by(|left: &CandidateBelief, right: &CandidateBelief| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        Ok(StorageCollection {
            items: candidates,
            warnings,
        })
    }

    fn read_candidate_beliefs(&self) -> Result<StorageCollection<CandidateBelief>> {
        self.ensure_layout()?;
        let node_collection = self.read_hypothesis_nodes()?;
        let mut candidates: Vec<CandidateBelief> = node_collection
            .items
            .into_iter()
            .map(|node| node.to_candidate())
            .collect();
        let mut seen: HashSet<String> = candidates
            .iter()
            .map(|candidate| candidate.candidate_id.clone())
            .collect();
        let mut warnings = node_collection.warnings;
        let legacy_collection = self.read_legacy_candidate_beliefs()?;
        warnings.extend(legacy_collection.warnings);
        for candidate in legacy_collection.items {
            if seen.insert(candidate.candidate_id.clone()) {
                candidates.push(candidate);
            }
        }
        candidates.sort_by(|left: &CandidateBelief, right: &CandidateBelief| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        Ok(StorageCollection {
            items: candidates,
            warnings,
        })
    }

    fn write_hypothesis_node(&self, node: &HypothesisNode) -> Result<()> {
        self.ensure_layout()?;
        node.validate_basic()?;
        atomic_write_json(&self.hypothesis_node_path(&node.hypothesis_id), node)
    }

    fn read_hypothesis_node(&self, hypothesis_id: &str) -> Result<HypothesisNode> {
        self.ensure_layout()?;
        validate_hypothesis_storage_id("hypothesis_id", hypothesis_id)?;
        let path = self.hypothesis_node_path(hypothesis_id);
        if !path.exists() {
            return Err(MemoryEngineError::Storage(format!(
                "hypothesis node not found: {hypothesis_id}"
            )));
        }
        let node: HypothesisNode = read_json(&path)?;
        node.validate_basic()?;
        Ok(node)
    }

    fn read_hypothesis_node_optional(&self, hypothesis_id: &str) -> Result<Option<HypothesisNode>> {
        self.ensure_layout()?;
        validate_hypothesis_storage_id("hypothesis_id", hypothesis_id)?;
        let path = self.hypothesis_node_path(hypothesis_id);
        if !path.exists() {
            return Ok(None);
        }
        let node: HypothesisNode = read_json(&path)?;
        node.validate_basic()?;
        Ok(Some(node))
    }

    fn read_hypothesis_nodes(&self) -> Result<StorageCollection<HypothesisNode>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files_shallow(
            &self.root.join("core").join("hypotheses").join("nodes"),
            &mut files,
        )?;
        let mut nodes = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(node) = read_json_from_collection::<HypothesisNode>(
                self,
                &path,
                "hypothesis node",
                &mut warnings,
            )? {
                if let Err(err) = node.validate_basic() {
                    warnings.push(StorageReadWarning {
                        collection: "hypothesis node".to_string(),
                        path: self.relative_path(&path),
                        error: err.to_string(),
                    });
                } else {
                    nodes.push(node);
                }
            }
        }
        nodes.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.hypothesis_id.cmp(&right.hypothesis_id))
        });
        Ok(StorageCollection {
            items: nodes,
            warnings,
        })
    }

    fn write_hypothesis_evidence_link(&self, link: &HypothesisEvidenceLink) -> Result<()> {
        self.ensure_layout()?;
        link.validate_basic()?;
        self.read_hypothesis_node(&link.hypothesis_id)?;
        atomic_write_json(&self.hypothesis_evidence_path(&link.evidence_link_id), link)
    }

    fn read_hypothesis_evidence_link_optional(
        &self,
        evidence_link_id: &str,
    ) -> Result<Option<HypothesisEvidenceLink>> {
        self.ensure_layout()?;
        validate_hypothesis_storage_id("evidence_link_id", evidence_link_id)?;
        let path = self.hypothesis_evidence_path(evidence_link_id);
        if !path.exists() {
            return Ok(None);
        }
        let link: HypothesisEvidenceLink = read_json(&path)?;
        link.validate_basic()?;
        Ok(Some(link))
    }

    fn read_hypothesis_evidence_links(&self) -> Result<StorageCollection<HypothesisEvidenceLink>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files_shallow(
            &self.root.join("core").join("hypotheses").join("evidence"),
            &mut files,
        )?;
        let mut links = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(link) = read_json_from_collection::<HypothesisEvidenceLink>(
                self,
                &path,
                "hypothesis evidence link",
                &mut warnings,
            )? {
                if let Err(err) = link.validate_basic() {
                    warnings.push(StorageReadWarning {
                        collection: "hypothesis evidence link".to_string(),
                        path: self.relative_path(&path),
                        error: err.to_string(),
                    });
                } else {
                    links.push(link);
                }
            }
        }
        links.sort_by(|left, right| left.evidence_link_id.cmp(&right.evidence_link_id));
        Ok(StorageCollection {
            items: links,
            warnings,
        })
    }

    fn write_hypothesis_relation(&self, relation: &HypothesisRelation) -> Result<()> {
        self.ensure_layout()?;
        relation.validate_basic()?;
        let source = self.read_hypothesis_node(&relation.source_hypothesis_id)?;
        let target = self.read_hypothesis_node(&relation.target_hypothesis_id)?;
        if source.core_scope != target.core_scope {
            return Err(MemoryEngineError::Validation(format!(
                "hypothesis relation cannot cross personality scopes: {} -> {}",
                relation.source_hypothesis_id, relation.target_hypothesis_id
            )));
        }
        atomic_write_json(
            &self.hypothesis_relation_path(&relation.relation_id),
            relation,
        )
    }

    fn read_hypothesis_relation_optional(
        &self,
        relation_id: &str,
    ) -> Result<Option<HypothesisRelation>> {
        self.ensure_layout()?;
        validate_hypothesis_storage_id("relation_id", relation_id)?;
        let path = self.hypothesis_relation_path(relation_id);
        if !path.exists() {
            return Ok(None);
        }
        let relation: HypothesisRelation = read_json(&path)?;
        relation.validate_basic()?;
        Ok(Some(relation))
    }

    fn read_hypothesis_relations(&self) -> Result<StorageCollection<HypothesisRelation>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files_shallow(
            &self.root.join("core").join("hypotheses").join("relations"),
            &mut files,
        )?;
        let mut relations = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(relation) = read_json_from_collection::<HypothesisRelation>(
                self,
                &path,
                "hypothesis relation",
                &mut warnings,
            )? {
                if let Err(err) = relation.validate_basic() {
                    warnings.push(StorageReadWarning {
                        collection: "hypothesis relation".to_string(),
                        path: self.relative_path(&path),
                        error: err.to_string(),
                    });
                } else {
                    relations.push(relation);
                }
            }
        }
        relations.sort_by(|left, right| left.relation_id.cmp(&right.relation_id));
        Ok(StorageCollection {
            items: relations,
            warnings,
        })
    }

    fn save_task(&self, task: &crate::tasks::PendingTask) -> Result<()> {
        self.ensure_layout()?;
        let active_path = self.task_path(&task.task_id);
        let completed_path = self.completed_task_path(&task.task_id);

        if task_state_is_terminal(&task.state) {
            atomic_write_json(&completed_path, task)?;
            remove_file_if_exists(&active_path)?;
        } else {
            atomic_write_json(&active_path, task)?;
            remove_file_if_exists(&completed_path)?;
        }
        Ok(())
    }

    fn load_task(&self, task_id: &str) -> Result<crate::tasks::PendingTask> {
        let active_path = self.task_path(task_id);
        if active_path.exists() {
            return read_json(&active_path);
        }

        let completed_path = self.completed_task_path(task_id);
        if completed_path.exists() {
            return read_json(&completed_path);
        }

        Err(MemoryEngineError::TaskNotFound(task_id.to_string()))
    }

    fn load_tasks(&self) -> Result<StorageCollection<crate::tasks::PendingTask>> {
        let mut files = Vec::new();
        collect_json_files_shallow(&self.root.join("tasks"), &mut files)?;

        let mut tasks = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(task) = read_json_from_collection::<crate::tasks::PendingTask>(
                self,
                &path,
                "task",
                &mut warnings,
            )? {
                tasks.push(task);
            }
        }

        Ok(StorageCollection {
            items: tasks,
            warnings,
        })
    }

    fn load_all_tasks(&self) -> Result<StorageCollection<crate::tasks::PendingTask>> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join("tasks"), &mut files)?;

        let mut tasks = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(task) = read_json_from_collection::<crate::tasks::PendingTask>(
                self,
                &path,
                "task",
                &mut warnings,
            )? {
                tasks.push(task);
            }
        }
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        tasks.dedup_by(|left, right| left.task_id == right.task_id);
        Ok(StorageCollection {
            items: tasks,
            warnings,
        })
    }

    fn save_sleep_run(&self, run: &SleepRun) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.sleep_run_path(&run.sleep_task_id), run)
    }

    fn load_sleep_run(&self, sleep_task_id: &str) -> Result<SleepRun> {
        self.ensure_layout()?;
        let path = self.sleep_run_path(sleep_task_id);
        if !path.exists() {
            return Err(MemoryEngineError::Storage(format!(
                "sleep run not found: {sleep_task_id}"
            )));
        }
        read_json(&path)
    }

    fn load_sleep_runs(&self) -> Result<StorageCollection<SleepRun>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files_shallow(&self.root.join("runs").join("sleep"), &mut files)?;

        let mut runs = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(run) =
                read_json_from_collection::<SleepRun>(self, &path, "sleep run", &mut warnings)?
            {
                runs.push(run);
            }
        }
        runs.sort_by(|left: &SleepRun, right: &SleepRun| {
            left.sleep_task_id.cmp(&right.sleep_task_id)
        });

        Ok(StorageCollection {
            items: runs,
            warnings,
        })
    }

    fn read_reflection_state(&self, scope: &str) -> Result<Option<ReflectionState>> {
        validate_reflection_scope_component(scope)?;
        self.ensure_layout()?;
        let path = self.reflection_state_path(scope);
        if !path.exists() {
            return Ok(None);
        }
        let state: ReflectionState = read_json(&path)?;
        if state.schema_version != REFLECTION_STATE_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_STATE_SCHEMA_VERSION.to_string(),
                actual: state.schema_version,
            });
        }
        if state.reflection_scope != scope {
            return Err(MemoryEngineError::Validation(format!(
                "reflection state scope mismatch: requested {scope}, stored {}",
                state.reflection_scope
            )));
        }
        Ok(Some(state))
    }

    fn read_reflection_states(&self) -> Result<StorageCollection<ReflectionState>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files_shallow(&self.root.join("runs").join("reflection"), &mut files)?;
        let mut states = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(state) = read_json_from_collection::<ReflectionState>(
                self,
                &path,
                "reflection state",
                &mut warnings,
            )? {
                states.push(state);
            }
        }
        states.sort_by(|left, right| left.reflection_scope.cmp(&right.reflection_scope));
        Ok(StorageCollection {
            items: states,
            warnings,
        })
    }

    fn write_reflection_state(&self, state: &ReflectionState) -> Result<()> {
        validate_reflection_scope_component(&state.reflection_scope)?;
        if state.schema_version != REFLECTION_STATE_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: REFLECTION_STATE_SCHEMA_VERSION.to_string(),
                actual: state.schema_version.clone(),
            });
        }
        self.ensure_layout()?;
        atomic_write_json(&self.reflection_state_path(&state.reflection_scope), state)
    }

    fn read_vector_index(&self, scope: &str) -> Result<Option<VectorIndexData>> {
        self.ensure_layout()?;
        validate_scope_component(scope)?;
        let manifest_path = self.vector_manifest_path(scope);
        if !manifest_path.exists() {
            return Ok(None);
        }

        let mut manifest: VectorIndexManifest = read_json(&manifest_path)?;
        if manifest.schema_version != VECTOR_INDEX_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: VECTOR_INDEX_SCHEMA_VERSION.to_string(),
                actual: manifest.schema_version.clone(),
            });
        }

        let rows = read_jsonl::<crate::vector::VectorRow>(&self.vector_rows_path(scope))?;
        let tombstones = read_jsonl::<VectorTombstone>(&self.vector_tombstones_path(scope))?;
        let vector_path = self.vector_data_path(scope);
        let vector_bytes = if vector_path.exists() {
            fs::read(&vector_path)?
        } else {
            Vec::new()
        };
        if vector_bytes.len() % std::mem::size_of::<f32>() != 0 {
            manifest.state = crate::vector::VectorScopeStatus::Corrupt;
            atomic_write_json(&manifest_path, &manifest)?;
            return Ok(Some(VectorIndexData {
                manifest,
                rows,
                vectors: Vec::new(),
                tombstones,
            }));
        }

        let expected_bytes = rows
            .len()
            .saturating_mul(manifest.dim)
            .saturating_mul(std::mem::size_of::<f32>());
        if vector_bytes.len() > expected_bytes {
            let file = OpenOptions::new().write(true).open(&vector_path)?;
            file.set_len(expected_bytes as u64)?;
        } else if vector_bytes.len() < expected_bytes {
            manifest.state = crate::vector::VectorScopeStatus::Corrupt;
            atomic_write_json(&manifest_path, &manifest)?;
            return Ok(Some(VectorIndexData {
                manifest,
                rows,
                vectors: Vec::new(),
                tombstones,
            }));
        }

        manifest.rows = rows.len();
        let vector_bytes = if vector_path.exists() {
            fs::read(&vector_path)?
        } else {
            Vec::new()
        };
        let vectors = decode_f32_vectors(&vector_bytes, manifest.dim)?;
        Ok(Some(VectorIndexData {
            manifest,
            rows,
            vectors,
            tombstones,
        }))
    }

    fn write_vector_index(&self, scope: &str, index: &VectorIndexData) -> Result<()> {
        self.ensure_layout()?;
        validate_scope_component(scope)?;
        let dir = self.vector_scope_dir(scope);
        fs::create_dir_all(&dir)?;
        atomic_write_f32_vectors(&self.vector_data_path(scope), &index.vectors)?;
        atomic_write_jsonl(&self.vector_rows_path(scope), &index.rows, true)?;
        atomic_write_jsonl(&self.vector_tombstones_path(scope), &index.tombstones, true)?;
        atomic_write_json(&self.vector_manifest_path(scope), &index.manifest)
    }

    fn write_vector_manifest(&self, scope: &str, manifest: &VectorIndexManifest) -> Result<()> {
        self.ensure_layout()?;
        validate_scope_component(scope)?;
        fs::create_dir_all(self.vector_scope_dir(scope))?;
        atomic_write_json(&self.vector_manifest_path(scope), manifest)
    }

    fn append_vector_records(
        &self,
        scope: &str,
        manifest: &VectorIndexManifest,
        records: &[VectorAppendRecord],
    ) -> Result<()> {
        if records.is_empty() {
            return self.write_vector_manifest(scope, manifest);
        }
        self.ensure_layout()?;
        validate_scope_component(scope)?;
        fs::create_dir_all(self.vector_scope_dir(scope))?;

        let mut vector_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.vector_data_path(scope))?;
        for record in records {
            write_f32_vector(&mut vector_file, &record.vector)?;
        }
        vector_file.sync_all()?;

        let mut rows_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.vector_rows_path(scope))?;
        for record in records {
            writeln!(rows_file, "{}", serde_json::to_string(&record.row)?)?;
        }
        rows_file.sync_all()?;

        atomic_write_json(&self.vector_manifest_path(scope), manifest)
    }

    fn append_vector_tombstones(&self, scope: &str, tombstones: &[VectorTombstone]) -> Result<()> {
        if tombstones.is_empty() {
            return Ok(());
        }
        self.ensure_layout()?;
        validate_scope_component(scope)?;
        if !self.vector_manifest_path(scope).exists() {
            return Ok(());
        }
        fs::create_dir_all(self.vector_scope_dir(scope))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.vector_tombstones_path(scope))?;
        for tombstone in tombstones {
            writeln!(file, "{}", serde_json::to_string(tombstone)?)?;
        }
        file.sync_all()?;
        Ok(())
    }

    fn purge_vector_scope(&self, scope: &str) -> Result<()> {
        self.ensure_layout()?;
        validate_scope_component(scope)?;
        match fs::remove_dir_all(self.vector_scope_dir(scope)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn begin_journaled_operation(&self, operation: &JournalOperation) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.journal_path(&operation.op_id), operation)
    }

    fn complete_journaled_operation(&self, op_id: &str) -> Result<()> {
        self.ensure_layout()?;
        let path = self.journal_path(op_id);
        let mut operation: JournalOperation = read_json(&path)?;
        operation.state = JournalState::Completed;
        atomic_write_json(&path, &operation)
    }

    fn load_journaled_operation(&self, op_id: &str) -> Result<Option<JournalOperation>> {
        self.ensure_layout()?;
        let path = self.journal_path(op_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json(&path)?))
    }

    fn load_journaled_operations(&self) -> Result<StorageCollection<JournalOperation>> {
        self.ensure_layout()?;
        let mut files = Vec::new();
        collect_json_files_shallow(&self.root.join("journal"), &mut files)?;

        let mut operations = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            match read_json::<JournalOperation>(&path) {
                Ok(operation) => operations.push(operation),
                // A writer can finish and remove its intent after enumeration.
                Err(MemoryEngineError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => warnings.push(StorageReadWarning {
                    collection: "journal operation".into(),
                    path: self.relative_path(&path),
                    error: error.to_string(),
                }),
            }
        }
        operations.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.op_id.cmp(&right.op_id))
        });
        Ok(StorageCollection {
            items: operations,
            warnings,
        })
    }

    fn delete_journaled_operation(&self, op_id: &str) -> Result<()> {
        validate_erasure_path_component("journal operation id", op_id)?;
        remove_file_if_exists(&self.journal_path(op_id))
    }

    fn recover_journaled_ingest(&self, operation: &JournalOperation) -> Result<()> {
        self.finish_journaled_ingest(operation)
    }

    fn discard_journaled_ingest(&self, operation_id: &str, session_id: &str) -> Result<()> {
        self.discard_ingest_intent(operation_id, session_id)
    }

    fn has_erasure_intent(&self, key: &str) -> Result<bool> {
        validate_erasure_path_component("erasure journal key", key)?;
        Ok(self.journal_path(key).try_exists()?)
    }

    fn read_erasure_generation(&self, key: &str) -> Result<u64> {
        validate_erasure_path_component("erasure generation key", key)?;
        let path = self.root.join("generations").join(format!("{key}.json"));
        match read_json(&path) {
            Err(MemoryEngineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(0)
            }
            result => result,
        }
    }

    fn write_erasure_generation(&self, key: &str, generation: u64) -> Result<()> {
        validate_erasure_path_component("erasure generation key", key)?;
        atomic_write_json(
            &self.root.join("generations").join(format!("{key}.json")),
            &generation,
        )
    }

    fn was_erased_task(&self, task_id: &str) -> Result<bool> {
        Ok(erased_task_path(&self.root, task_id).try_exists()?)
    }

    fn apply_memory_erasure_plan(&self, plan: &MemoryErasurePlan) -> Result<()> {
        self.ensure_layout()?;
        validate_memory_erasure_plan(plan)?;
        self.retire_erased_ingest_receipts(plan)?;
        if plan.operation == crate::erasure::MemoryErasureKind::DeleteSession {
            for archive in self.read_sleep_archives(&plan.target_id)? {
                self.retire_archive_replay_receipt(&archive.archive_id, None)?;
            }
        }

        for id in plan.task_ids.iter().chain(&plan.sleep_task_ids) {
            let marker = erased_task_path(&self.root, id);
            if !marker.try_exists()? {
                atomic_write_json(&marker, &true)?;
            }
        }

        let mut core_ids_by_category = BTreeMap::<&str, HashSet<&str>>::new();
        for reference in &plan.core_facts {
            core_ids_by_category
                .entry(reference.category.as_str())
                .or_default()
                .insert(reference.core_fact_id.as_str());
        }
        for (category_name, core_ids) in core_ids_by_category {
            let mut category = self
                .read_core_store_category(category_name)
                .map_err(|error| {
                    MemoryEngineError::Storage(format!(
                        "cannot read Core category {category_name:?} during memory erasure: {error}"
                    ))
                })?;
            let previous_len = category.facts.len();
            category
                .facts
                .retain(|fact| !core_ids.contains(fact.core_fact_id.as_str()));
            if category.facts.len() != previous_len {
                self.write_core_store_category(&category)?;
            }
        }

        for relation_id in &plan.hypothesis_relation_ids {
            remove_file_if_exists(&self.hypothesis_relation_path(relation_id))?;
        }
        for evidence_link_id in &plan.hypothesis_evidence_link_ids {
            remove_file_if_exists(&self.hypothesis_evidence_path(evidence_link_id))?;
        }
        for candidate_id in &plan.candidate_ids {
            remove_file_if_exists(&self.candidate_path(candidate_id))?;
        }
        for hypothesis_id in &plan.hypothesis_node_ids {
            remove_file_if_exists(&self.hypothesis_node_path(hypothesis_id))?;
        }

        for task_id in &plan.task_ids {
            remove_file_if_exists(&self.task_path(task_id))?;
            remove_file_if_exists(&self.completed_task_path(task_id))?;
        }
        for sleep_task_id in &plan.sleep_task_ids {
            remove_file_if_exists(&self.sleep_run_path(sleep_task_id))?;
        }
        for scope in &plan.reflection_scopes {
            remove_file_if_exists(&self.reflection_state_path(scope))?;
        }
        for scope in &plan.vector_scopes {
            self.purge_vector_scope(scope)?;
        }

        for archive_id in &plan.archive_ids {
            if let Some(path) = self.archive_entry_path_by_id_optional(archive_id)? {
                remove_file_if_exists(&path)?;
            }
        }
        for memory_unit_id in &plan.memory_unit_ids {
            remove_file_if_exists(&self.memory_unit_path(memory_unit_id))?;
        }
        for archive_id in &plan.archive_ids {
            let _guard = lock_publication_receipt_retirement()?;
            remove_file_if_exists(&self.memory_unit_publication_receipt_path(archive_id))?;
        }
        for session_id in &plan.session_ids {
            remove_dir_all_if_exists(&self.session_dir(session_id))?;
        }
        for op_id in &plan.journal_operation_ids {
            remove_file_if_exists(&self.journal_path(op_id))?;
        }

        Ok(())
    }
}

fn erased_task_path(root: &Path, task_id: &str) -> PathBuf {
    use sha2::{Digest, Sha256};

    let key = format!("{:x}", Sha256::digest(task_id.as_bytes()));
    root.join("generations").join("retired-tasks").join(key)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path)?;
    Ok(serde_json::from_reader(file)?)
}

fn read_json_from_collection<T: DeserializeOwned>(
    storage: &FileStorage,
    path: &Path,
    collection: &str,
    warnings: &mut Vec<StorageReadWarning>,
) -> Result<Option<T>> {
    match read_json(path) {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            warnings.push(StorageReadWarning {
                collection: collection.to_string(),
                path: storage.relative_path(path),
                error: err.to_string(),
            });
            Ok(None)
        }
    }
}

impl FileStorage {
    pub(crate) fn retire_archive_replay_receipt(
        &self,
        archive_id: &str,
        task_id: Option<&str>,
    ) -> Result<()> {
        // A session can be reused in a new scope while its older Archive
        // remains in the former scope. Coordinate this rewrite with purge.
        let _guard = lock_publication_receipt_retirement()?;
        let Some(mut receipt) = self.read_memory_unit_publication_receipt(archive_id)? else {
            return Ok(());
        };
        if receipt["archive_id"].as_str() != Some(archive_id)
            || !receipt["memory_unit_ids"].is_array()
        {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        if task_id.is_some_and(|id| receipt["task_id"].as_str() != Some(id)) {
            return Ok(());
        }
        let Some(fields) = receipt.as_object_mut() else {
            return Err(MemoryEngineError::OperationResultUnavailable);
        };
        let task = fields.remove("task_id");
        let request = fields.remove("request_digest");
        if task.is_some() || request.is_some() {
            self.write_memory_unit_publication_receipt(archive_id, &receipt)?;
        }
        Ok(())
    }

    fn memory_unit_publication_receipt_path(&self, archive_id: &str) -> PathBuf {
        self.root
            .join("journal")
            .join("memory-unit-receipts")
            .join(format!("{}.json", memory_unit_publication_key(archive_id)))
    }

    fn require_archive_publication_complete(&self, archive_id: &str) -> Result<()> {
        if self
            .journal_path(&memory_unit_publication_key(archive_id))
            .try_exists()?
        {
            return Err(MemoryEngineError::OperationIncomplete);
        }
        Ok(())
    }

    fn read_memory_unit_records(
        &self,
        include: impl Fn(&MemoryUnit) -> bool,
    ) -> Result<StorageCollection<MemoryUnit>> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join("archive").join("units"), &mut files)?;
        let mut items = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            if let Some(unit) =
                read_json_from_collection(self, &path, "memory unit", &mut warnings)?
                    .filter(&include)
            {
                items.push(unit);
            }
        }
        Ok(StorageCollection { items, warnings })
    }

    fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

fn lock_publication_receipt_retirement() -> Result<std::sync::MutexGuard<'static, ()>> {
    PUBLICATION_RECEIPT_RETIREMENT.lock().map_err(|_| {
        MemoryEngineError::Storage("publication receipt retirement lock was poisoned".into())
    })
}

fn sort_archive_memory_units(units: &mut [MemoryUnit]) {
    units.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
    });
}

fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        items.push(serde_json::from_str(&line)?);
    }

    Ok(items)
}

fn write_jsonl_sync<T: Serialize>(path: &Path, values: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(path)?;
    for value in values {
        let line = serde_json::to_string(value)?;
        writeln!(file, "{line}")?;
    }
    file.sync_all()?;
    Ok(())
}

fn atomic_write_jsonl<T: Serialize>(path: &Path, values: &[T], sync_file: bool) -> Result<()> {
    let mut content = String::new();
    for value in values {
        content.push_str(&serde_json::to_string(value)?);
        content.push('\n');
    }
    atomic_write_string(path, &content, sync_file)
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    atomic_write_string(path, &format!("{content}\n"), true)
}

fn atomic_write_f32_vectors(path: &Path, vectors: &[Vec<f32>]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = unique_tmp_path(path);
    let mut file = File::create(&tmp_path)?;
    for vector in vectors {
        write_f32_vector(&mut file, vector)?;
    }
    file.sync_all()?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn write_f32_vector(file: &mut File, vector: &[f32]) -> Result<()> {
    for value in vector {
        file.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

fn decode_f32_vectors(bytes: &[u8], dim: usize) -> Result<Vec<Vec<f32>>> {
    if dim == 0 {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        return Err(MemoryEngineError::Storage(
            "vector dim must not be zero".to_string(),
        ));
    }
    let row_bytes = dim.saturating_mul(std::mem::size_of::<f32>());
    if row_bytes == 0 || !bytes.len().is_multiple_of(row_bytes) {
        return Err(MemoryEngineError::Storage(format!(
            "vector byte length {} is not divisible by row size {}",
            bytes.len(),
            row_bytes
        )));
    }
    let mut vectors = Vec::new();
    for chunk in bytes.chunks_exact(row_bytes) {
        let mut vector = Vec::with_capacity(dim);
        for raw in chunk.chunks_exact(std::mem::size_of::<f32>()) {
            vector.push(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]));
        }
        vectors.push(vector);
    }
    Ok(vectors)
}

fn atomic_write_json_relaxed<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    atomic_write_string(path, &format!("{content}\n"), false)
}

fn atomic_write_string(path: &Path, content: &str, sync_file: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = unique_tmp_path(path);
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        if sync_file {
            file.sync_all()?;
        }
    }

    fs::rename(tmp_path, path)?;
    Ok(())
}

fn unique_tmp_path(path: &Path) -> PathBuf {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tmp");
    path.with_file_name(format!(".{file_name}.{}.{counter}.tmp", std::process::id()))
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn validate_memory_erasure_plan(plan: &MemoryErasurePlan) -> Result<()> {
    if plan.schema_version != MEMORY_ERASURE_PLAN_SCHEMA_VERSION {
        return Err(MemoryEngineError::IncompatibleSchema {
            expected: MEMORY_ERASURE_PLAN_SCHEMA_VERSION.to_string(),
            actual: plan.schema_version.clone(),
        });
    }
    validate_erasure_path_component("erasure target", &plan.target_id)?;
    for value in &plan.session_ids {
        validate_erasure_path_component("session id", value)?;
    }
    for value in &plan.archive_ids {
        validate_erasure_path_component("archive id", value)?;
    }
    for value in &plan.memory_unit_ids {
        validate_erasure_path_component("memory unit id", value)?;
    }
    for reference in &plan.core_facts {
        validate_erasure_path_component("Core category", &reference.category)?;
        validate_erasure_path_component("Core fact id", &reference.core_fact_id)?;
    }
    for value in &plan.candidate_ids {
        validate_erasure_path_component("candidate id", value)?;
    }
    for value in &plan.hypothesis_node_ids {
        validate_erasure_path_component("hypothesis node id", value)?;
    }
    for value in &plan.hypothesis_evidence_link_ids {
        validate_erasure_path_component("hypothesis evidence link id", value)?;
    }
    for value in &plan.hypothesis_relation_ids {
        validate_erasure_path_component("hypothesis relation id", value)?;
    }
    for value in &plan.task_ids {
        validate_erasure_path_component("task id", value)?;
    }
    for value in &plan.sleep_task_ids {
        validate_erasure_path_component("sleep task id", value)?;
    }
    for value in &plan.reflection_scopes {
        validate_reflection_scope_component(value)?;
    }
    for value in &plan.vector_scopes {
        validate_scope_component(value)?;
    }
    for value in &plan.journal_operation_ids {
        validate_erasure_path_component("journal operation id", value)?;
    }
    Ok(())
}

fn validate_erasure_path_component(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
        || value.contains("..")
        || value.chars().any(char::is_control)
    {
        return Err(MemoryEngineError::Validation(format!(
            "invalid {label} path component: {value}"
        )));
    }
    Ok(())
}

fn task_state_is_terminal(state: &TaskState) -> bool {
    matches!(
        state,
        TaskState::Completed | TaskState::Failed | TaskState::Cancelled
    )
}

fn collect_json_files_shallow(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }

    Ok(())
}

fn collect_jsonl_files_shallow(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }

    Ok(())
}

fn collect_json_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }

    Ok(())
}

fn collect_archive_entry_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if matches!(name, "units" | "forgotten" | "vectors") {
                continue;
            }
            collect_archive_entry_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }

    Ok(())
}

fn validate_scope_component(scope: &str) -> Result<()> {
    if scope.trim().is_empty()
        || scope.contains('/')
        || scope.contains('\\')
        || scope == "."
        || scope == ".."
        || scope.contains("..")
    {
        return Err(MemoryEngineError::Validation(format!(
            "invalid vector scope path component: {scope}"
        )));
    }
    Ok(())
}

fn validate_reflection_scope_component(scope: &str) -> Result<()> {
    if scope.trim().is_empty()
        || scope.contains('/')
        || scope.contains('\\')
        || scope == "."
        || scope == ".."
        || scope.contains("..")
    {
        return Err(MemoryEngineError::Validation(format!(
            "invalid reflection scope path component: {scope}"
        )));
    }
    Ok(())
}

fn archive_matches_filters(entry: &ArchiveEntry, filters: &ArchiveFilters) -> bool {
    if let Some(theme) = &filters.theme {
        if entry.theme.as_ref() != Some(theme) {
            return false;
        }
    }

    if let Some(min_weight) = filters.min_weight {
        if entry.weight < min_weight {
            return false;
        }
    }

    if let Some(min_freshness) = filters.min_freshness {
        if entry.freshness < min_freshness {
            return false;
        }
    }

    if !filters.tags.is_empty()
        && !filters
            .tags
            .iter()
            .all(|tag| entry.tags.iter().any(|entry_tag| entry_tag == tag))
    {
        return false;
    }

    if let Some(time_range) = &filters.time_range {
        if entry.time_range.end.as_str() < time_range.start.as_str()
            || entry.time_range.start.as_str() > time_range.end.as_str()
        {
            return false;
        }
    }

    true
}

fn year_month_from_timestamp(timestamp: &str) -> (String, String) {
    let year = timestamp.get(0..4).unwrap_or("unknown").to_string();
    let month = timestamp.get(5..7).unwrap_or("unknown").to_string();
    (year, month)
}

fn event_summary(event: &StoredEvent) -> String {
    event
        .memory_text
        .as_ref()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| {
            event
                .payload
                .get("text")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| event.payload.to_string())
}

#[cfg(test)]
mod tests {
    use super::{unique_tmp_path, FileStorage};
    use crate::archive::{
        ArchiveEntry, ArchiveFilters, ArchiveStatus, FidelityStatus, MemoryUnit, MemoryUnitStatus,
    };
    use crate::storage::Storage;
    use crate::types::TimeRange;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ROOT_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn unique_tmp_path_stays_in_same_directory_and_changes_name() {
        let path = Path::new("root").join("target.json");
        let left = unique_tmp_path(&path);
        let right = unique_tmp_path(&path);

        assert_eq!(left.parent(), path.parent());
        assert_eq!(right.parent(), path.parent());
        assert_ne!(left, right);
        assert!(left
            .file_name()
            .and_then(|name| name.to_str())
            .expect("left tmp name")
            .starts_with(".target.json."));
        assert!(right
            .file_name()
            .and_then(|name| name.to_str())
            .expect("right tmp name")
            .ends_with(".tmp"));
    }

    #[test]
    fn archive_collection_hydration_matches_individual_reads() {
        let root = std::env::temp_dir().join(format!(
            "memory-context-hydration-{}-{}",
            std::process::id(),
            TEST_ROOT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let storage = FileStorage::new(&root);
        storage.ensure_layout().expect("storage layout");

        let first = archive_fixture(
            "archive_first",
            "scope_one",
            vec![unit_fixture(
                "embedded_first",
                "archive_first",
                "embedded",
                "2026-01-01T00:00:00Z",
            )],
        );
        let second = archive_fixture("archive_second", "scope_two", Vec::new());
        storage.write_archive_entry(&first).expect("first archive");
        storage
            .write_archive_entry(&second)
            .expect("second archive");
        storage
            .write_memory_unit(&unit_fixture(
                "unit_later",
                "archive_first",
                "scope_one",
                "2026-02-01T00:00:00Z",
            ))
            .expect("later canonical unit");
        storage
            .write_memory_unit(&unit_fixture(
                "unit_tie_a",
                "archive_first",
                "scope_one",
                "2026-01-01T00:00:00Z",
            ))
            .expect("first tied canonical unit");
        storage
            .write_memory_unit(&unit_fixture(
                "unit_tie_b",
                "archive_first",
                "scope_one",
                "2026-01-01T00:00:00Z",
            ))
            .expect("second tied canonical unit");
        fs::write(root.join("archive/units/corrupt.json"), b"{broken json")
            .expect("corrupt unit fixture");

        let collection = storage
            .read_archive(&ArchiveFilters::default())
            .expect("archive collection");
        assert_eq!(collection.warnings.len(), 1);
        assert!(collection.warnings[0].path.ends_with("embedded_first"));
        assert!(storage.read_memory_unit_inventory().unwrap().warnings[0]
            .path
            .ends_with("corrupt.json"));
        for entry in collection.items {
            assert_eq!(
                entry,
                storage
                    .read_archive_entry_by_id(&entry.archive_id)
                    .expect("individual archive read")
            );
        }
        let first_read = storage
            .read_archive_entry_by_id("archive_first")
            .expect("canonical precedence");
        assert_eq!(
            first_read.memory_units[0].created_at,
            first_read.memory_units[1].created_at
        );
        assert_eq!(first_read.memory_units[2].memory_unit_id, "unit_later");
        let fallback = storage
            .read_archive_entry_by_id("archive_second")
            .expect("legacy episode");
        assert_eq!(fallback, second);

        let no_matches = storage
            .read_archive(&ArchiveFilters {
                theme: Some("no-match".to_string()),
                ..ArchiveFilters::default()
            })
            .expect("empty filtered read does not load unit files");
        assert!(no_matches.items.is_empty());
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn incomplete_unit_publication_isolates_archive_until_commit() {
        use crate::journal::{
            JournalOperation, JournalOperationType, JournalState, RecoveryPolicy,
        };
        use crate::storage::memory_unit_publication_key;
        use crate::MemoryEngineError;

        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("absolute temp path");
        let root = temp.join(format!(
            "memory-unit-visibility-{}-{}",
            std::process::id(),
            TEST_ROOT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let storage = FileStorage::new(&root);
        let archive = archive_fixture("archive_publishing", "target_scope", Vec::new());
        let neighbor = archive_fixture("archive_neighbor", "other_scope", Vec::new());
        let first = unit_fixture(
            "unit_first",
            &archive.archive_id,
            "target_scope",
            &archive.created_at,
        );
        let second = unit_fixture(
            "unit_second",
            &archive.archive_id,
            "target_scope",
            &archive.created_at,
        );
        storage.write_archive_entry(&archive).unwrap();
        storage.write_archive_entry(&neighbor).unwrap();
        let key = memory_unit_publication_key(&archive.archive_id);
        storage
            .begin_journaled_operation(&JournalOperation {
                schema_version: "journal_operation.v1".into(),
                op_id: key.clone(),
                op_type: JournalOperationType::MemoryUnitPublish,
                state: JournalState::Started,
                created_at: archive.created_at.clone(),
                updated_at: archive.updated_at.clone(),
                target_files: Vec::new(),
                intent: serde_json::Value::Null,
                recovery_policy: RecoveryPolicy::Retry,
                completed_at: None,
                error: None,
            })
            .unwrap();
        storage.write_memory_unit(&first).unwrap();

        let collection = storage.read_archive(&ArchiveFilters::default()).unwrap();
        assert_eq!(collection.items, vec![neighbor]);
        assert_eq!(collection.warnings.len(), 1);
        assert!(matches!(
            storage.read_archive_entry_by_id(&archive.archive_id),
            Err(MemoryEngineError::OperationIncomplete)
        ));
        assert!(matches!(
            storage.read_memory_unit_by_id(&first.memory_unit_id),
            Err(MemoryEngineError::OperationIncomplete)
        ));
        assert_eq!(storage.read_archive_inventory().unwrap().items.len(), 2);
        assert_eq!(
            storage.read_memory_unit_inventory().unwrap().items,
            vec![first.clone()]
        );

        storage.write_memory_unit(&second).unwrap();
        let published = ArchiveEntry {
            compact_memory: Some("unit_first\nunit_second".into()),
            memory_units: vec![first.clone(), second.clone()],
            ..archive.clone()
        };
        storage
            .update_archive_entry(&archive.archive_id, &published)
            .unwrap();
        storage
            .write_memory_unit_publication_receipt(
                &archive.archive_id,
                &serde_json::json!({
                    "schema_version": "memory_unit_task_outcome.v1",
                    "archive_id": archive.archive_id,
                    "memory_unit_ids": [first.memory_unit_id, second.memory_unit_id],
                }),
            )
            .unwrap();
        storage.delete_journaled_operation(&key).unwrap();

        // A scan begun during publication can fill gaps from the committed IDs.
        let projection = storage.project_archive(archive, Some(&[first])).unwrap();
        assert!(projection.warnings.is_empty());
        assert_eq!(
            projection.entry.unwrap().memory_units,
            published.memory_units
        );
        assert_eq!(
            storage
                .read_archive_entry_by_id(&published.archive_id)
                .unwrap(),
            published
        );
        let root = root.canonicalize().unwrap();
        assert!(root.starts_with(&temp) && root != temp);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn damaged_published_units_leave_healthy_siblings_and_scoped_diagnostics() {
        use crate::recall::{RecallFilters, RecallQuery};
        use crate::types::RECALL_QUERY_SCHEMA_VERSION;
        use crate::MemoryEngine;

        let temp = std::env::temp_dir().canonicalize().unwrap();
        let root = temp.join(format!(
            "damaged-published-units-{}-{}",
            std::process::id(),
            TEST_ROOT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let storage = FileStorage::new(&root);
        let mut archive = archive_fixture("target_archive", "target_scope", Vec::new());
        let healthy = unit_fixture(
            "healthy_unit",
            &archive.archive_id,
            "target_scope",
            &archive.created_at,
        );
        let missing = unit_fixture(
            "missing_unit",
            &archive.archive_id,
            "target_scope",
            &archive.created_at,
        );
        let corrupt = unit_fixture(
            "corrupt_unit",
            &archive.archive_id,
            "target_scope",
            &archive.created_at,
        );
        archive.memory_units = vec![healthy.clone(), missing.clone(), corrupt.clone()];
        for embedded in &mut archive.memory_units {
            embedded.thesis = format!("stale copy of {}", embedded.memory_unit_id);
        }
        archive.compact_memory = Some("stale compact projection".into());
        storage.write_archive_entry(&archive).unwrap();
        storage
            .write_archive_entry(&archive_fixture(
                "neighbor_archive",
                "other_scope",
                Vec::new(),
            ))
            .unwrap();
        for unit in [&healthy, &missing, &corrupt] {
            storage.write_memory_unit(unit).unwrap();
        }
        storage
            .write_memory_unit_publication_receipt(
                &archive.archive_id,
                &serde_json::json!({
                    "schema_version": "memory_unit_task_outcome.v1",
                    "archive_id": archive.archive_id,
                    "memory_unit_ids": [healthy.memory_unit_id, missing.memory_unit_id, corrupt.memory_unit_id],
                }),
            )
            .unwrap();
        fs::remove_file(storage.memory_unit_path(&missing.memory_unit_id)).unwrap();
        fs::write(storage.memory_unit_path(&corrupt.memory_unit_id), b"{").unwrap();
        drop(storage);

        let storage = FileStorage::new(&root);
        let detail = storage
            .read_archive_entry_by_id(&archive.archive_id)
            .unwrap();
        assert_eq!(detail.memory_units, std::slice::from_ref(&healthy));
        assert_eq!(
            detail.compact_memory.as_deref(),
            Some(healthy.thesis.as_str())
        );

        let batch = storage.read_archive(&ArchiveFilters::default()).unwrap();
        assert_eq!(batch.items.len(), 2);
        assert!(batch.items.contains(&detail));
        for damaged_id in [&missing.memory_unit_id, &corrupt.memory_unit_id] {
            assert!(batch
                .warnings
                .iter()
                .any(|warning| warning.note().contains(damaged_id)));
        }
        let units = storage
            .read_memory_units_for_archive(&archive.archive_id)
            .unwrap();
        assert_eq!(units.items, std::slice::from_ref(&healthy));
        assert_eq!(units.warnings.len(), 2);

        let engine = MemoryEngine::new(storage.clone());
        let recall = |scope: &str| {
            engine
                .recall(RecallQuery {
                    schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
                    query_id: None,
                    created_at: None,
                    personality_scope_id: Some(scope.to_string()),
                    session_id: None,
                    context: serde_json::Value::Null,
                    query_text: None,
                    query_embedding: None,
                    filters: RecallFilters::default(),
                    limit: 5,
                    include_core: false,
                    explain: false,
                })
                .unwrap()
        };
        let target_recall = recall("target_scope");
        assert_eq!(target_recall.items.len(), 1);
        assert_eq!(target_recall.items[0].id, archive.archive_id);
        assert_eq!(
            target_recall.items[0].compact_memory.as_deref(),
            Some(healthy.thesis.as_str())
        );
        assert!(!target_recall.notes.is_empty());
        assert!(recall("other_scope").notes.is_empty());

        fs::remove_file(storage.memory_unit_path(&healthy.memory_unit_id)).unwrap();
        assert!(storage
            .read_archive_entry_by_id(&archive.archive_id)
            .is_err());
        let units = storage
            .read_memory_units_for_archive(&archive.archive_id)
            .unwrap();
        assert!(units.items.is_empty());
        assert_eq!(units.warnings.len(), 3);
        assert!(recall("target_scope").items.is_empty());

        let root = root.canonicalize().unwrap();
        assert!(root.starts_with(&temp) && root != temp);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn published_empty_set_ignores_stale_archive_and_unlisted_unit() {
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let root = temp.join(format!(
            "published-empty-units-{}-{}",
            std::process::id(),
            TEST_ROOT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let storage = FileStorage::new(&root);
        let mut archive = archive_fixture("empty_archive", "target_scope", Vec::new());
        let stale = unit_fixture(
            "unlisted_unit",
            &archive.archive_id,
            "target_scope",
            &archive.created_at,
        );
        archive.memory_units.push(stale.clone());
        archive.compact_memory = Some(stale.thesis.clone());
        storage.write_archive_entry(&archive).unwrap();
        storage.write_memory_unit(&stale).unwrap();
        storage
            .write_memory_unit_publication_receipt(
                &archive.archive_id,
                &serde_json::json!({
                    "schema_version": "memory_unit_task_outcome.v1",
                    "archive_id": archive.archive_id,
                    "memory_unit_ids": [],
                }),
            )
            .unwrap();

        let detail = storage
            .read_archive_entry_by_id(&archive.archive_id)
            .unwrap();
        assert!(detail.memory_units.is_empty());
        assert!(detail.compact_memory.is_none());
        assert_eq!(
            storage
                .read_archive(&ArchiveFilters::default())
                .unwrap()
                .items,
            [detail]
        );
        assert!(storage
            .read_memory_units_for_archive(&archive.archive_id)
            .unwrap()
            .items
            .is_empty());
        assert!(storage
            .read_memory_unit_by_id(&stale.memory_unit_id)
            .is_err());

        fs::write(
            storage.memory_unit_publication_receipt_path(&archive.archive_id),
            b"{",
        )
        .unwrap();
        let damaged = storage.read_archive(&ArchiveFilters::default()).unwrap();
        assert!(damaged.items.is_empty());
        assert_eq!(damaged.warnings.len(), 1);
        assert_eq!(
            damaged.warnings[0].error,
            "corrupt memory unit publication receipt"
        );

        let root = root.canonicalize().unwrap();
        assert!(root.starts_with(&temp) && root != temp);
        fs::remove_dir_all(root).unwrap();
    }

    fn archive_fixture(id: &str, scope: &str, memory_units: Vec<MemoryUnit>) -> ArchiveEntry {
        ArchiveEntry {
            schema_version: "archive_entry.v1".to_string(),
            archive_id: id.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            personality_scope_id: Some(scope.to_string()),
            source_session_id: format!("session_{scope}"),
            source_event_ids: vec![],
            time_range: TimeRange {
                start: "2026-01-01T00:00:00Z".to_string(),
                end: "2026-01-01T00:00:00Z".to_string(),
            },
            theme: None,
            tags: vec![],
            gist: id.to_string(),
            narrative: id.to_string(),
            compact_memory: None,
            memory_units,
            facts: vec![],
            quotes: vec![],
            weight: 1.0,
            freshness: 1.0,
            recall_count: 0,
            last_recalled_at: None,
            links: vec![],
            emotional_markers: vec![],
            topic_thread: vec![],
            personal_signals: vec![],
            relational_tone: None,
            status: ArchiveStatus::Complete,
            llm_enhanced: false,
            prompt_id: None,
            prompt_version: None,
            embedding_model_id: None,
            embedding: None,
        }
    }

    fn unit_fixture(id: &str, archive_id: &str, scope: &str, created_at: &str) -> MemoryUnit {
        MemoryUnit {
            schema_version: "memory_unit.v1".to_string(),
            memory_unit_id: id.to_string(),
            archive_id: archive_id.to_string(),
            personality_scope_id: Some(scope.to_string()),
            source_session_id: format!("session_{scope}"),
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
            thesis: id.to_string(),
            subject: None,
            source_event_ids: vec![],
            evidence: None,
            tags: vec![],
            weight: 1.0,
            status: MemoryUnitStatus::ActiveArchive,
            fidelity_status: FidelityStatus::Valid,
            fidelity_review: None,
            forget_review: None,
            reflection_count: 0,
            last_reflected_at: None,
            last_reflection_operation_id: None,
        }
    }
}
