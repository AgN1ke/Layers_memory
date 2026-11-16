use std::collections::BTreeSet;

use super::*;
use crate::archive::render_compact_memory_from_units;
use crate::storage::{archive_in_scope, MEMORY_UNIT_TASK_OUTCOME_SCHEMA};
use crate::types::{ARCHIVE_ENTRY_SCHEMA_VERSION, MEMORY_UNIT_SCHEMA_VERSION};

pub(super) struct ArchiveProjection {
    pub(super) entry: Option<ArchiveEntry>,
    pub(super) warnings: Vec<StorageReadWarning>,
    publication_incomplete: bool,
}

impl FileStorage {
    pub(super) fn read_archive_collection(
        &self,
        filters: &ArchiveFilters,
        scope: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<StorageCollection<ArchiveEntry>> {
        let mut files = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut files)?;
        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        let mut units_by_archive = None;
        for path in files {
            let entry = match read_archive_file(&path) {
                Ok(entry) => entry,
                Err(error) => {
                    if scope.is_none() && session_id.is_none() {
                        warnings.push(StorageReadWarning {
                            collection: "archive".into(),
                            path: self.relative_path(&path),
                            error: error.to_string(),
                        });
                    }
                    continue;
                }
            };
            if !archive_in_scope(&entry, scope, session_id)
                || !archive_matches_filters(&entry, filters)
            {
                continue;
            }
            if units_by_archive.is_none() {
                units_by_archive = Some(self.canonical_unit_index()?);
            }
            let units = units_by_archive
                .as_ref()
                .and_then(|index| index.get(&entry.archive_id));
            let projection =
                self.project_archive(entry, Some(units.map(Vec::as_slice).unwrap_or_default()))?;
            entries.extend(projection.entry);
            warnings.extend(projection.warnings);
        }
        Ok(StorageCollection {
            items: entries,
            warnings,
        })
    }

    pub(super) fn read_projected_archive(&self, archive_id: &str) -> Result<ArchiveEntry> {
        let projection = self.project_archive(
            read_archive_file(&self.archive_entry_path_by_id(archive_id)?)?,
            None,
        )?;
        match projection.entry {
            Some(entry) => Ok(entry),
            None if projection.publication_incomplete => {
                Err(MemoryEngineError::OperationIncomplete)
            }
            None => Err(MemoryEngineError::OperationResultUnavailable),
        }
    }

    pub(super) fn read_projected_archive_units(
        &self,
        archive_id: &str,
    ) -> Result<StorageCollection<MemoryUnit>> {
        let projection = self.project_archive(
            read_archive_file(&self.archive_entry_path_by_id(archive_id)?)?,
            None,
        )?;
        if projection.publication_incomplete {
            return Err(MemoryEngineError::OperationIncomplete);
        }
        Ok(StorageCollection {
            items: projection
                .entry
                .map(|entry| entry.memory_units)
                .unwrap_or_default(),
            warnings: projection.warnings,
        })
    }

    pub(super) fn read_canonical_unit(&self, memory_unit_id: &str) -> Result<MemoryUnit> {
        if !valid_record_id(memory_unit_id) {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        let unit = read_unit_file(&self.memory_unit_path(memory_unit_id))?;
        let archive = read_archive_file(&self.archive_entry_path_by_id(&unit.archive_id)?)?;
        if !unit_belongs_to_archive(&unit, &archive) {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        if self
            .visible_publication_ids(&archive.archive_id)?
            .is_some_and(|ids| !ids.contains(memory_unit_id))
        {
            return Err(MemoryEngineError::OperationIncomplete);
        }
        Ok(unit)
    }

    pub(super) fn read_canonical_units(&self, ids: &[&str]) -> Result<HashMap<String, MemoryUnit>> {
        let ids = ids
            .iter()
            .copied()
            .filter(|id| valid_record_id(id))
            .collect::<HashSet<_>>();
        let mut units_by_archive = HashMap::<String, Vec<MemoryUnit>>::new();
        for unit in ids
            .iter()
            .filter_map(|id| read_unit_file(&self.memory_unit_path(id)).ok())
        {
            units_by_archive
                .entry(unit.archive_id.clone())
                .or_default()
                .push(unit);
        }
        let mut result = HashMap::new();
        if units_by_archive.is_empty() {
            return Ok(result);
        }
        let mut paths = Vec::new();
        collect_archive_entry_files(&self.root.join("archive"), &mut paths)?;
        for path in paths {
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Some(units) = units_by_archive.remove(id) else {
                continue;
            };
            let Ok(archive) = read_archive_file(&path) else {
                continue;
            };
            let Ok(published) = self.visible_publication_ids(&archive.archive_id) else {
                continue;
            };
            result.extend(
                units
                    .into_iter()
                    .filter(|unit| {
                        unit_belongs_to_archive(unit, &archive)
                            && published
                                .as_ref()
                                .is_none_or(|ids| ids.contains(&unit.memory_unit_id))
                    })
                    .map(|unit| (unit.memory_unit_id.clone(), unit)),
            );
            if units_by_archive.is_empty() {
                break;
            }
        }
        Ok(result)
    }

    fn canonical_unit_index(&self) -> Result<HashMap<String, Vec<MemoryUnit>>> {
        let mut files = Vec::new();
        collect_json_files_shallow(&self.root.join("archive").join("units"), &mut files)?;
        let mut grouped: HashMap<String, Vec<MemoryUnit>> = HashMap::new();
        for unit in files.iter().filter_map(|path| read_unit_file(path).ok()) {
            grouped
                .entry(unit.archive_id.clone())
                .or_default()
                .push(unit);
        }
        Ok(grouped)
    }

    pub(super) fn project_archive(
        &self,
        mut archive: ArchiveEntry,
        scanned: Option<&[MemoryUnit]>,
    ) -> Result<ArchiveProjection> {
        // After a batch scan, check the journal and then the receipt. If a
        // publication committed during that scan, re-read missing members by ID.
        let published = match self.visible_publication_ids(&archive.archive_id) {
            Ok(ids) => ids,
            Err(MemoryEngineError::OperationIncomplete) => {
                return Ok(ArchiveProjection {
                    warnings: vec![archive_warning(
                        &archive,
                        "incomplete memory unit publication",
                    )],
                    entry: None,
                    publication_incomplete: true,
                })
            }
            Err(
                MemoryEngineError::OperationResultUnavailable
                | MemoryEngineError::Json(_)
                | MemoryEngineError::Io(_),
            ) => {
                return Ok(ArchiveProjection {
                    warnings: vec![archive_warning(
                        &archive,
                        "corrupt memory unit publication receipt",
                    )],
                    entry: None,
                    publication_incomplete: false,
                })
            }
            Err(error) => return Err(error),
        };
        let mut units = match (scanned, &published) {
            (Some(units), _) => units.to_vec(),
            (None, None) => self
                .canonical_unit_index()?
                .remove(&archive.archive_id)
                .unwrap_or_default(),
            (None, Some(_)) => Vec::new(),
        };
        units.retain(|unit| {
            unit_belongs_to_archive(unit, &archive)
                && published
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&unit.memory_unit_id))
        });
        let mut warnings = Vec::new();
        let expected: BTreeSet<_> = match published.as_ref() {
            Some(ids) => ids.iter().map(String::as_str).collect(),
            // Before receipts, the embedded IDs describe expected standalone
            // files. The cached thesis and status are never a recovery source.
            None => archive
                .memory_units
                .iter()
                .map(|unit| unit.memory_unit_id.as_str())
                .collect(),
        };
        let available: HashSet<_> = units
            .iter()
            .map(|unit| unit.memory_unit_id.as_str())
            .collect();
        let mut recovered = Vec::new();
        for id in expected.iter().filter(|id| !available.contains(*id)) {
            let record = if valid_record_id(id) {
                read_unit_file(&self.memory_unit_path(id))
            } else {
                Err(MemoryEngineError::OperationResultUnavailable)
            };
            let error = match record {
                Ok(unit) if unit_belongs_to_archive(&unit, &archive) => {
                    recovered.push(unit);
                    continue;
                }
                Err(MemoryEngineError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    "missing canonical memory unit"
                }
                _ => "corrupt canonical memory unit",
            };
            warnings.push(StorageReadWarning {
                collection: "memory unit".into(),
                path: if valid_record_id(id) {
                    format!("{}/{}", archive.archive_id, id)
                } else {
                    archive.archive_id.clone()
                },
                error: error.into(),
            });
        }
        units.extend(recovered);
        if units.is_empty() && !expected.is_empty() {
            return Ok(ArchiveProjection {
                entry: None,
                warnings,
                publication_incomplete: false,
            });
        }
        if published.is_some() || !units.is_empty() {
            sort_archive_memory_units(&mut units);
            archive.compact_memory = render_compact_memory_from_units(&units);
            archive.memory_units = units;
        }
        Ok(ArchiveProjection {
            entry: Some(archive),
            warnings,
            publication_incomplete: false,
        })
    }

    pub(super) fn visible_publication_ids(
        &self,
        archive_id: &str,
    ) -> Result<Option<HashSet<String>>> {
        self.require_archive_publication_complete(archive_id)?;
        let Some(receipt) = self
            .read_memory_unit_publication_receipt(archive_id)
            .map_err(|_| MemoryEngineError::OperationResultUnavailable)?
        else {
            return Ok(None);
        };
        #[derive(serde::Deserialize)]
        struct PublishedUnits {
            schema_version: String,
            archive_id: String,
            memory_unit_ids: Vec<String>,
        }
        let receipt: PublishedUnits = serde_json::from_value(receipt)?;
        let count = receipt.memory_unit_ids.len();
        let ids: HashSet<_> = receipt.memory_unit_ids.into_iter().collect();
        if receipt.schema_version != MEMORY_UNIT_TASK_OUTCOME_SCHEMA
            || receipt.archive_id != archive_id
            || ids.len() != count
        {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        Ok(Some(ids))
    }
}

fn read_unit_file(path: &Path) -> Result<MemoryUnit> {
    let unit: MemoryUnit = read_json(path)?;
    if path.file_stem().and_then(|stem| stem.to_str()) != Some(unit.memory_unit_id.as_str())
        || unit.schema_version != MEMORY_UNIT_SCHEMA_VERSION
        || unit.thesis.trim().is_empty()
    {
        return Err(MemoryEngineError::OperationResultUnavailable);
    }
    Ok(unit)
}

fn read_archive_file(path: &Path) -> Result<ArchiveEntry> {
    let archive: ArchiveEntry = read_json(path)?;
    if path.file_stem().and_then(|stem| stem.to_str()) != Some(archive.archive_id.as_str())
        || archive.schema_version != ARCHIVE_ENTRY_SCHEMA_VERSION
    {
        return Err(MemoryEngineError::OperationResultUnavailable);
    }
    Ok(archive)
}

fn valid_record_id(id: &str) -> bool {
    !id.is_empty() && !matches!(id, "." | "..") && !id.contains(['/', '\\', ':'])
}

fn unit_belongs_to_archive(unit: &MemoryUnit, archive: &ArchiveEntry) -> bool {
    unit.archive_id == archive.archive_id
        && unit.source_session_id == archive.source_session_id
        && unit.effective_personality_scope_id() == archive.effective_personality_scope_id()
}

fn archive_warning(archive: &ArchiveEntry, error: &str) -> StorageReadWarning {
    StorageReadWarning {
        collection: "archive".into(),
        path: archive.archive_id.clone(),
        error: error.into(),
    }
}
