use crate::archive::{ArchiveEntry, ArchiveFilters, MemoryUnit};
use crate::core_store::{CandidateBelief, CoreStoreCategory};
use crate::event::StoredEvent;
use crate::journal::JournalOperation;
use crate::manifest::Manifest;
use crate::session::SessionRecord;
use crate::tasks::PendingTask;
use crate::Result;

pub trait Storage {
    fn manifest_exists(&self) -> Result<bool>;
    fn read_manifest(&self) -> Result<Manifest>;
    fn write_manifest(&self, manifest: &Manifest) -> Result<()>;

    fn read_session(&self, session_id: &str) -> Result<SessionRecord>;
    fn append_event(&self, session_id: &str, event: &StoredEvent) -> Result<()>;

    fn write_archive_entry(&self, entry: &ArchiveEntry) -> Result<()>;
    fn update_archive_entry(&self, archive_id: &str, entry: &ArchiveEntry) -> Result<()>;
    fn read_archive_entry_by_id(&self, archive_id: &str) -> Result<ArchiveEntry>;
    fn read_archive(&self, filters: &ArchiveFilters) -> Result<Vec<ArchiveEntry>>;
    fn write_memory_unit(&self, unit: &MemoryUnit) -> Result<()>;
    fn read_memory_unit_by_id(&self, memory_unit_id: &str) -> Result<MemoryUnit>;
    fn read_memory_units_for_archive(&self, archive_id: &str) -> Result<Vec<MemoryUnit>>;

    fn read_core_store_category(&self, category: &str) -> Result<CoreStoreCategory>;
    fn read_core_store_categories(&self) -> Result<Vec<CoreStoreCategory>>;
    fn write_core_store_category(&self, category: &CoreStoreCategory) -> Result<()>;
    fn write_candidate_belief(&self, candidate: &CandidateBelief) -> Result<()>;
    fn read_candidate_belief(&self, candidate_id: &str) -> Result<CandidateBelief>;
    fn read_candidate_beliefs(&self) -> Result<Vec<CandidateBelief>>;

    fn save_task(&self, task: &PendingTask) -> Result<()>;
    fn load_task(&self, task_id: &str) -> Result<PendingTask>;
    fn load_tasks(&self) -> Result<Vec<PendingTask>>;

    fn begin_journaled_operation(&self, operation: &JournalOperation) -> Result<()>;
    fn complete_journaled_operation(&self, op_id: &str) -> Result<()>;
}
