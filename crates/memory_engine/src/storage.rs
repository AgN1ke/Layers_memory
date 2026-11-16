use std::collections::HashMap;

use crate::archive::{ArchiveEntry, ArchiveFilters, MemoryUnit};
use crate::core_store::{CandidateBelief, CoreStoreCategory};
use crate::erasure::MemoryErasurePlan;
use crate::event::StoredEvent;
use crate::hypothesis::{HypothesisEvidenceLink, HypothesisNode, HypothesisRelation};
use crate::journal::JournalOperation;
use crate::llm::SleepRun;
use crate::manifest::Manifest;
use crate::reflection::ReflectionState;
use crate::session::{SessionMetadata, SessionRecord};
use crate::tasks::PendingTask;
use crate::vector::{VectorAppendRecord, VectorIndexData, VectorIndexManifest, VectorTombstone};
use crate::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn memory_unit_publication_key(archive_id: &str) -> String {
    format!(
        "memory_unit_publish_{:x}",
        Sha256::digest(archive_id.as_bytes())
    )
}

pub(crate) const MEMORY_UNIT_TASK_OUTCOME_SCHEMA: &str = "memory_unit_task_outcome.v1";

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
    fn read_sessions(&self) -> Result<StorageCollection<SessionMetadata>>;
    fn read_session_metadata(&self, session_id: &str) -> Result<SessionMetadata>;
    fn write_session_metadata(&self, metadata: &SessionMetadata) -> Result<()>;
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
    fn read_archive_scoped(
        &self,
        filters: &ArchiveFilters,
        scope: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<StorageCollection<ArchiveEntry>> {
        let mut collection = self.read_archive(filters)?;
        if scope.is_some() || session_id.is_some() {
            collection
                .items
                .retain(|entry| archive_in_scope(entry, scope, session_id));
            collection.warnings.clear();
        }
        Ok(collection)
    }
    /// Internal publication view of the persisted Archive before unit hydration.
    fn read_archive_entry_raw(&self, archive_id: &str) -> Result<ArchiveEntry> {
        self.read_archive_entry_by_id(archive_id)
    }
    /// Physical Archive inventory for erasure, including unfinished publications.
    fn read_archive_inventory(&self) -> Result<StorageCollection<ArchiveEntry>> {
        self.read_archive(&ArchiveFilters::default())
    }
    /// Read the persisted Archive entries needed to recover Sleep for a session.
    /// Coverage and publication status do not require hydrating memory units.
    fn read_sleep_archives(&self, session_id: &str) -> Result<Vec<ArchiveEntry>> {
        Ok(self
            .read_archive(&ArchiveFilters::default())?
            .into_items()
            .into_iter()
            .filter(|entry| entry.source_session_id == session_id)
            .collect())
    }
    fn write_memory_unit(&self, unit: &MemoryUnit) -> Result<()>;
    fn read_memory_unit_by_id(&self, memory_unit_id: &str) -> Result<MemoryUnit>;
    fn read_memory_units_by_id(&self, ids: &[&str]) -> Result<HashMap<String, MemoryUnit>> {
        Ok(ids
            .iter()
            .filter_map(|id| self.read_memory_unit_by_id(id).ok())
            .map(|unit| (unit.memory_unit_id.clone(), unit))
            .collect())
    }
    fn read_memory_units(&self) -> Result<StorageCollection<MemoryUnit>>;
    fn read_memory_units_for_archive(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>>;
    /// Physical canonical-unit inventory for erasure.
    fn read_memory_unit_inventory(&self) -> Result<StorageCollection<MemoryUnit>> {
        self.read_memory_units()
    }
    /// Internal publication view, including legacy partial units.
    fn read_memory_units_for_archive_raw(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>> {
        self.read_memory_units_for_archive(archive_id)
    }
    fn read_memory_unit_publication_receipt(&self, _archive_id: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn write_memory_unit_publication_receipt(
        &self,
        _archive_id: &str,
        _receipt: &Value,
    ) -> Result<()> {
        Ok(())
    }

    fn read_core_store_category(&self, category: &str) -> Result<CoreStoreCategory>;
    fn read_core_store_categories(&self) -> Result<StorageCollection<CoreStoreCategory>>;
    fn write_core_store_category(&self, category: &CoreStoreCategory) -> Result<()>;
    fn write_candidate_belief(&self, candidate: &CandidateBelief) -> Result<()>;
    fn read_candidate_belief(&self, candidate_id: &str) -> Result<CandidateBelief>;
    fn read_candidate_belief_optional(&self, candidate_id: &str)
        -> Result<Option<CandidateBelief>>;
    /// Read only persisted v0.5 candidate files, without graph projections.
    /// Schema migration uses this so future graph-native nodes are never
    /// flattened back into legacy records.
    fn read_legacy_candidate_beliefs(&self) -> Result<StorageCollection<CandidateBelief>>;
    fn read_candidate_beliefs(&self) -> Result<StorageCollection<CandidateBelief>>;
    fn write_hypothesis_node(&self, node: &HypothesisNode) -> Result<()>;
    fn read_hypothesis_node(&self, hypothesis_id: &str) -> Result<HypothesisNode>;
    fn read_hypothesis_node_optional(&self, hypothesis_id: &str) -> Result<Option<HypothesisNode>>;
    fn read_hypothesis_nodes(&self) -> Result<StorageCollection<HypothesisNode>>;
    fn write_hypothesis_evidence_link(&self, link: &HypothesisEvidenceLink) -> Result<()>;
    fn read_hypothesis_evidence_link_optional(
        &self,
        evidence_link_id: &str,
    ) -> Result<Option<HypothesisEvidenceLink>>;
    fn read_hypothesis_evidence_links(&self) -> Result<StorageCollection<HypothesisEvidenceLink>>;
    fn write_hypothesis_relation(&self, relation: &HypothesisRelation) -> Result<()>;
    fn read_hypothesis_relation_optional(
        &self,
        relation_id: &str,
    ) -> Result<Option<HypothesisRelation>>;
    fn read_hypothesis_relations(&self) -> Result<StorageCollection<HypothesisRelation>>;

    fn save_task(&self, task: &PendingTask) -> Result<()>;
    fn load_task(&self, task_id: &str) -> Result<PendingTask>;
    fn load_tasks(&self) -> Result<StorageCollection<PendingTask>>;
    fn load_all_tasks(&self) -> Result<StorageCollection<PendingTask>>;

    fn save_sleep_run(&self, run: &SleepRun) -> Result<()>;
    fn load_sleep_run(&self, sleep_task_id: &str) -> Result<SleepRun>;
    fn load_sleep_runs(&self) -> Result<StorageCollection<SleepRun>>;

    fn read_reflection_state(&self, scope: &str) -> Result<Option<ReflectionState>>;
    fn read_reflection_states(&self) -> Result<StorageCollection<ReflectionState>>;
    fn write_reflection_state(&self, state: &ReflectionState) -> Result<()>;

    fn read_vector_index(&self, scope: &str) -> Result<Option<VectorIndexData>>;
    fn write_vector_index(&self, scope: &str, index: &VectorIndexData) -> Result<()>;
    fn write_vector_manifest(&self, scope: &str, manifest: &VectorIndexManifest) -> Result<()>;
    fn append_vector_records(
        &self,
        scope: &str,
        manifest: &VectorIndexManifest,
        records: &[VectorAppendRecord],
    ) -> Result<()>;
    fn append_vector_tombstones(&self, scope: &str, tombstones: &[VectorTombstone]) -> Result<()>;
    fn purge_vector_scope(&self, scope: &str) -> Result<()>;

    fn begin_journaled_operation(&self, operation: &JournalOperation) -> Result<()>;
    fn complete_journaled_operation(&self, op_id: &str) -> Result<()>;
    fn load_journaled_operation(&self, op_id: &str) -> Result<Option<JournalOperation>>;
    fn load_journaled_operations(&self) -> Result<StorageCollection<JournalOperation>>;
    fn delete_journaled_operation(&self, op_id: &str) -> Result<()>;

    fn recover_journaled_ingest(&self, _operation: &JournalOperation) -> Result<()> {
        Err(crate::MemoryEngineError::Storage(
            "storage does not support journaled ingest recovery".to_string(),
        ))
    }

    fn discard_journaled_ingest(&self, operation_id: &str, _session_id: &str) -> Result<()> {
        self.delete_journaled_operation(operation_id)
    }

    fn has_erasure_intent(&self, key: &str) -> Result<bool> {
        Ok(self.load_journaled_operation(key)?.is_some())
    }

    fn read_erasure_generation(&self, _key: &str) -> Result<u64> {
        Ok(0)
    }

    fn write_erasure_generation(&self, _key: &str, _generation: u64) -> Result<()> {
        Ok(())
    }

    fn was_erased_task(&self, _task_id: &str) -> Result<bool> {
        Ok(false)
    }

    /// Apply an engine-prepared deletion plan idempotently. The plan is
    /// journaled before this method is called and may be replayed after a
    /// process crash.
    fn apply_memory_erasure_plan(&self, plan: &MemoryErasurePlan) -> Result<()>;
}

pub(crate) fn archive_in_scope(
    entry: &ArchiveEntry,
    scope: Option<&str>,
    session_id: Option<&str>,
) -> bool {
    match scope {
        Some(scope) => entry.effective_personality_scope_id() == scope,
        None => session_id.is_none_or(|id| entry.source_session_id == id),
    }
}
