use crate::archive::{ArchiveEntry, ArchiveFilters, MemoryUnit};
use crate::core_store::{CandidateBelief, CoreStoreCategory};
use crate::event::StoredEvent;
use crate::journal::JournalOperation;
use crate::llm::SleepRun;
use crate::manifest::Manifest;
use crate::session::SessionRecord;
use crate::tasks::PendingTask;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageReadWarning {
    pub collection: String,
    pub path: String,
    pub error: String,
}

impl StorageReadWarning {
    pub fn note(&self) -> String {
        format!(
            "skipped unreadable {} file: {} ({})",
            self.collection, self.path, self.error
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StorageCollection<T> {
    pub items: Vec<T>,
    pub warnings: Vec<StorageReadWarning>,
}

impl<T> StorageCollection<T> {
    pub fn clean(items: Vec<T>) -> Self {
        Self {
            items,
            warnings: Vec::new(),
        }
    }

    pub fn into_items(self) -> Vec<T> {
        self.items
    }
}

pub trait Storage {
    fn manifest_exists(&self) -> Result<bool>;
    fn read_manifest(&self) -> Result<Manifest>;
    fn write_manifest(&self, manifest: &Manifest) -> Result<()>;

    fn read_session(&self, session_id: &str) -> Result<SessionRecord>;
    fn read_session_archived_events(&self, session_id: &str) -> Result<Vec<StoredEvent>>;
    fn append_event(&self, session_id: &str, event: &StoredEvent) -> Result<()>;
    fn rotate_session_events(
        &self,
        session_id: &str,
        covered_event_ids: &[String],
    ) -> Result<usize>;

    fn write_archive_entry(&self, entry: &ArchiveEntry) -> Result<()>;
    fn update_archive_entry(&self, archive_id: &str, entry: &ArchiveEntry) -> Result<()>;
    fn read_archive_entry_by_id(&self, archive_id: &str) -> Result<ArchiveEntry>;
    fn read_archive(&self, filters: &ArchiveFilters) -> Result<StorageCollection<ArchiveEntry>>;
    fn write_memory_unit(&self, unit: &MemoryUnit) -> Result<()>;
    fn read_memory_unit_by_id(&self, memory_unit_id: &str) -> Result<MemoryUnit>;
    fn read_memory_units_for_archive(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>>;

    fn read_core_store_category(&self, category: &str) -> Result<CoreStoreCategory>;
    fn read_core_store_categories(&self) -> Result<StorageCollection<CoreStoreCategory>>;
    fn write_core_store_category(&self, category: &CoreStoreCategory) -> Result<()>;
    fn write_candidate_belief(&self, candidate: &CandidateBelief) -> Result<()>;
    fn read_candidate_belief(&self, candidate_id: &str) -> Result<CandidateBelief>;
    fn read_candidate_beliefs(&self) -> Result<StorageCollection<CandidateBelief>>;

    fn save_task(&self, task: &PendingTask) -> Result<()>;
    fn load_task(&self, task_id: &str) -> Result<PendingTask>;
    fn load_tasks(&self) -> Result<StorageCollection<PendingTask>>;

    fn save_sleep_run(&self, run: &SleepRun) -> Result<()>;
    fn load_sleep_run(&self, sleep_task_id: &str) -> Result<SleepRun>;
    fn load_sleep_runs(&self) -> Result<StorageCollection<SleepRun>>;

    fn begin_journaled_operation(&self, operation: &JournalOperation) -> Result<()>;
    fn complete_journaled_operation(&self, op_id: &str) -> Result<()>;
}
