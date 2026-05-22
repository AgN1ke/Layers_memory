use crate::archive::{ArchiveEntry, ArchiveFilters};
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
    fn write_manifest(&mut self, manifest: &Manifest) -> Result<()>;

    fn read_session(&self, session_id: &str) -> Result<SessionRecord>;
    fn append_event(&mut self, session_id: &str, event: &StoredEvent) -> Result<()>;

    fn write_archive_entry(&mut self, entry: &ArchiveEntry) -> Result<()>;
    fn update_archive_entry(&mut self, archive_id: &str, entry: &ArchiveEntry) -> Result<()>;
    fn read_archive_entry_by_id(&self, archive_id: &str) -> Result<ArchiveEntry>;
    fn read_archive(&self, filters: &ArchiveFilters) -> Result<Vec<ArchiveEntry>>;

    fn read_core_store_category(&self, category: &str) -> Result<CoreStoreCategory>;
    fn read_core_store_categories(&self) -> Result<Vec<CoreStoreCategory>>;
    fn write_core_store_category(&mut self, category: &CoreStoreCategory) -> Result<()>;
    fn write_candidate_belief(&mut self, candidate: &CandidateBelief) -> Result<()>;

    fn save_task(&mut self, task: &PendingTask) -> Result<()>;
    fn load_task(&self, task_id: &str) -> Result<PendingTask>;
    fn load_tasks(&self) -> Result<Vec<PendingTask>>;

    fn begin_journaled_operation(&mut self, operation: &JournalOperation) -> Result<()>;
    fn complete_journaled_operation(&mut self, op_id: &str) -> Result<()>;
}
