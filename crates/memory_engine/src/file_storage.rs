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
use crate::manifest::Manifest;
use crate::session::{SessionMetadata, SessionRecord, SessionStatus};
use crate::storage::Storage;
use crate::tasks::TaskState;
use crate::types::{CORE_STORE_SCHEMA_VERSION, SESSION_SCHEMA_VERSION};
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
        fs::create_dir_all(self.root.join("core").join("store"))?;
        fs::create_dir_all(self.root.join("core").join("candidates"))?;
        fs::create_dir_all(self.root.join("tasks"))?;
        fs::create_dir_all(self.root.join("tasks").join("completed"))?;
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

    fn journal_path(&self, op_id: &str) -> PathBuf {
        self.root.join("journal").join(format!("{op_id}.json"))
    }

    fn read_session_metadata(&self, session_id: &str) -> Result<SessionMetadata> {
        read_json(&self.session_json_path(session_id))
    }

    fn write_session_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        atomic_write_json_relaxed(&self.session_json_path(&metadata.session_id), metadata)
    }

    fn upsert_session_metadata(&self, session_id: &str, event: &StoredEvent) -> Result<()> {
        let path = self.session_json_path(session_id);
        let mut metadata = if path.exists() {
            self.read_session_metadata(session_id)?
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

        self.write_session_metadata(&metadata)
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

    fn write_manifest(&mut self, manifest: &Manifest) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.manifest_path(), manifest)
    }

    fn read_session(&self, session_id: &str) -> Result<SessionRecord> {
        let metadata = self.read_session_metadata(session_id)?;
        let events = read_jsonl(&self.events_path(session_id))?;
        Ok(SessionRecord { metadata, events })
    }

    fn append_event(&mut self, session_id: &str, event: &StoredEvent) -> Result<()> {
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

    fn write_archive_entry(&mut self, entry: &ArchiveEntry) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.archive_entry_path(entry), entry)
    }

    fn update_archive_entry(&mut self, archive_id: &str, entry: &ArchiveEntry) -> Result<()> {
        self.ensure_layout()?;
        let path = self.archive_entry_path_by_id(archive_id)?;
        atomic_write_json(&path, entry)
    }

    fn read_archive_entry_by_id(&self, archive_id: &str) -> Result<ArchiveEntry> {
        let path = self.archive_entry_path_by_id(archive_id)?;
        read_json(&path)
    }

    fn read_archive(&self, filters: &ArchiveFilters) -> Result<Vec<ArchiveEntry>> {
        let mut files = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut files)?;

        let mut entries = Vec::new();
        for path in files {
            let entry: ArchiveEntry = read_json(&path)?;
            if archive_matches_filters(&entry, filters) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    fn write_memory_unit(&mut self, unit: &MemoryUnit) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.memory_unit_path(&unit.memory_unit_id), unit)
    }

    fn read_memory_units_for_archive(&self, archive_id: &str) -> Result<Vec<MemoryUnit>> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join("archive").join("units"), &mut files)?;

        let mut units = Vec::new();
        for path in files {
            let unit: MemoryUnit = read_json(&path)?;
            if unit.archive_id == archive_id {
                units.push(unit);
            }
        }
        units.sort_by(|left, right| left.created_at.cmp(&right.created_at));

        Ok(units)
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

    fn read_core_store_categories(&self) -> Result<Vec<CoreStoreCategory>> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join("core").join("store"), &mut files)?;

        let mut categories = Vec::new();
        for path in files {
            categories.push(read_json(&path)?);
        }
        categories.sort_by(|left: &CoreStoreCategory, right: &CoreStoreCategory| {
            left.category.cmp(&right.category)
        });

        Ok(categories)
    }

    fn write_core_store_category(&mut self, category: &CoreStoreCategory) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.core_store_path(&category.category), category)
    }

    fn write_candidate_belief(&mut self, candidate: &CandidateBelief) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.candidate_path(&candidate.candidate_id), candidate)
    }

    fn save_task(&mut self, task: &crate::tasks::PendingTask) -> Result<()> {
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

    fn load_tasks(&self) -> Result<Vec<crate::tasks::PendingTask>> {
        let mut files = Vec::new();
        collect_json_files_shallow(&self.root.join("tasks"), &mut files)?;

        let mut tasks = Vec::new();
        for path in files {
            tasks.push(read_json(&path)?);
        }

        Ok(tasks)
    }

    fn begin_journaled_operation(&mut self, operation: &JournalOperation) -> Result<()> {
        self.ensure_layout()?;
        atomic_write_json(&self.journal_path(&operation.op_id), operation)
    }

    fn complete_journaled_operation(&mut self, op_id: &str) -> Result<()> {
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

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    atomic_write_string(path, &format!("{content}\n"), true)
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
            if matches!(name, "units" | "forgotten") {
                continue;
            }
            collect_archive_entry_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
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
