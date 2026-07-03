use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::archive::{ArchiveEntry, ArchiveFilters, MemoryUnit};
use crate::core_store::{CandidateBelief, CoreStoreCategory};
use crate::event::StoredEvent;
use crate::journal::{JournalOperation, JournalState};
use crate::llm::SleepRun;
use crate::manifest::Manifest;
use crate::session::{SessionMetadata, SessionRecord, SessionStatus};
use crate::storage::{Storage, StorageCollection, StorageReadWarning};
use crate::tasks::TaskState;
use crate::types::{CORE_STORE_SCHEMA_VERSION, SESSION_SCHEMA_VERSION};
use crate::vector::{
    VectorAppendRecord, VectorIndexData, VectorIndexManifest, VectorTombstone,
    VECTOR_INDEX_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

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
        fs::create_dir_all(self.root.join("tasks"))?;
        fs::create_dir_all(self.root.join("tasks").join("completed"))?;
        fs::create_dir_all(self.root.join("runs").join("sleep"))?;
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

    fn write_session_metadata_file(&self, metadata: &SessionMetadata) -> Result<()> {
        atomic_write_json_relaxed(&self.session_json_path(&metadata.session_id), metadata)
    }

    fn upsert_session_metadata(&self, session_id: &str, event: &StoredEvent) -> Result<()> {
        let path = self.session_json_path(session_id);
        let mut metadata = if path.exists() {
            self.read_session_metadata_file(session_id)?
        } else {
            SessionMetadata {
                schema_version: SESSION_SCHEMA_VERSION.to_string(),
                session_id: session_id.to_string(),
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
                notes: Vec::new(),
            }
        };

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

        self.write_session_metadata_file(&metadata)
    }

    fn update_session_markdown(&self, session_id: &str, event: &StoredEvent) -> Result<()> {
        let path = self.session_md_path(session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let needs_header = !path.exists() || fs::metadata(&path)?.len() == 0;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        if needs_header {
            write!(
                file,
                "---\nschema_version: session_view.v1\nsession_id: {session_id}\nstatus: active\n---\n\n# Сесія {session_id}\n\n## Події\n\n"
            )?;
        }

        let line = format!(
            "- {} {}: {}\n",
            event.timestamp,
            event.event_type,
            event_summary(event)
        );
        file.write_all(line.as_bytes())?;
        Ok(())
    }
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
        let metadata = self.read_session_metadata_file(session_id)?;
        let events = read_jsonl(&self.events_path(session_id))?;
        Ok(SessionRecord { metadata, events })
    }

    fn read_session_metadata(&self, session_id: &str) -> Result<SessionMetadata> {
        self.read_session_metadata_file(session_id)
    }

    fn write_session_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        self.ensure_layout()?;
        if let Some(parent) = self.session_json_path(&metadata.session_id).parent() {
            fs::create_dir_all(parent)?;
        }
        self.write_session_metadata_file(metadata)
    }

    fn read_session_archived_events(&self, session_id: &str) -> Result<Vec<StoredEvent>> {
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
        self.ensure_layout()?;
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
        let path = self.archive_entry_path_by_id(archive_id)?;
        read_json(&path)
    }

    fn read_archive(&self, filters: &ArchiveFilters) -> Result<StorageCollection<ArchiveEntry>> {
        let mut files = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut files)?;

        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            let Some(entry) =
                read_json_from_collection::<ArchiveEntry>(self, &path, "archive", &mut warnings)?
            else {
                continue;
            };
            if archive_matches_filters(&entry, filters) {
                entries.push(entry);
            }
        }

        Ok(StorageCollection {
            items: entries,
            warnings,
        })
    }

    fn write_memory_unit(&self, unit: &MemoryUnit) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.memory_unit_path(&unit.memory_unit_id), unit)
    }

    fn read_memory_unit_by_id(&self, memory_unit_id: &str) -> Result<MemoryUnit> {
        self.ensure_layout()?;
        let path = self.memory_unit_path(memory_unit_id);
        if !path.exists() {
            return Err(MemoryEngineError::Storage(format!(
                "memory unit not found: {memory_unit_id}"
            )));
        }
        read_json(&path)
    }

    fn read_memory_units_for_archive(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join("archive").join("units"), &mut files)?;

        let mut units = Vec::new();
        let mut warnings = Vec::new();
        for path in files {
            let Some(unit) =
                read_json_from_collection::<MemoryUnit>(self, &path, "memory unit", &mut warnings)?
            else {
                continue;
            };
            if unit.archive_id == archive_id {
                units.push(unit);
            }
        }
        units.sort_by(|left, right| left.created_at.cmp(&right.created_at));

        Ok(StorageCollection {
            items: units,
            warnings,
        })
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
        atomic_write_json(&self.candidate_path(&candidate.candidate_id), candidate)
    }

    fn read_candidate_belief(&self, candidate_id: &str) -> Result<CandidateBelief> {
        self.ensure_layout()?;
        let path = self.candidate_path(candidate_id);
        if !path.exists() {
            return Err(MemoryEngineError::Storage(format!(
                "candidate belief not found: {candidate_id}"
            )));
        }
        read_json(&path)
    }

    fn read_candidate_beliefs(&self) -> Result<StorageCollection<CandidateBelief>> {
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
    fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
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
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| event.payload.to_string())
}

#[cfg(test)]
mod tests {
    use super::unique_tmp_path;
    use std::path::Path;

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
}
