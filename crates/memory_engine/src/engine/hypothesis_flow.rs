use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PreparedHypothesisEvidenceMigration {
    desired: crate::hypothesis::HypothesisEvidenceLink,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<crate::hypothesis::HypothesisEvidenceLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PreparedHypothesisCandidateMigration {
    candidate_id: String,
    desired_node: HypothesisNode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_node: Option<HypothesisNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence_links: Vec<PreparedHypothesisEvidenceMigration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct HypothesisGraphMigrationOperationIntent {
    schema_version: String,
    migrated_at: String,
    prepared_candidates: Vec<PreparedHypothesisCandidateMigration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    applied_candidate_ids: Vec<String>,
    #[serde(default)]
    manifest_updated: bool,
    result: HypothesisGraphMigrationResult,
}

impl<S: Storage> MemoryEngine<S> {
    /// Materialize every flat v0.5 candidate as an auditable graph node plus
    /// the MemoryUnit evidence edges that can be proven from persisted IDs.
    /// Re-running the migration updates projections in place and never invents
    /// semantic relations.
    pub fn migrate_hypothesis_graph(&self) -> Result<HypothesisGraphMigrationResult> {
        self.ensure_manifest()?;
        self.with_resource_lock("hypothesis_graph:migration".to_string(), || {
            let (intent, target_files) = self.prepare_hypothesis_graph_migration()?;
            let now = intent.migrated_at.clone();
            let operation = JournalOperation {
                schema_version: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
                op_id: new_id("hypothesis_graph_migration")?,
                op_type: JournalOperationType::Migration,
                state: JournalState::Started,
                created_at: now.clone(),
                updated_at: now,
                target_files,
                intent: serde_json::to_value(intent)?,
                recovery_policy: RecoveryPolicy::RetryOrManualReview,
                completed_at: None,
                error: None,
            };
            self.storage.begin_journaled_operation(&operation)?;
            self.execute_hypothesis_graph_migration_unlocked(operation)
        })
    }

    fn prepare_hypothesis_graph_migration(
        &self,
    ) -> Result<(HypothesisGraphMigrationOperationIntent, Vec<String>)> {
        let migrated_at = now_rfc3339()?;
        let candidate_collection = self.storage.read_legacy_candidate_beliefs()?;
        let mut result = HypothesisGraphMigrationResult::empty(migrated_at.clone());
        result.source_candidate_count = candidate_collection.items.len();
        result.warnings.extend(
            candidate_collection
                .warnings
                .iter()
                .map(StorageReadWarning::note),
        );
        let mut prepared_candidates = Vec::new();
        let mut target_files = vec!["manifest.json".to_string()];

        for candidate in candidate_collection.items {
            let existing_node = self
                .storage
                .read_hypothesis_node_optional(&candidate.candidate_id)?;
            let mut node = existing_node
                .clone()
                .unwrap_or_else(|| HypothesisNode::from_candidate(&candidate));
            node.merge_candidate_projection(&candidate);
            node.validate_basic()?;
            match existing_node.as_ref() {
                None => result.nodes_created += 1,
                Some(existing) if existing == &node => result.nodes_unchanged += 1,
                Some(_) => result.nodes_updated += 1,
            }
            target_files.push(format!("core/hypotheses/nodes/{}.json", node.hypothesis_id));

            if candidate.source_memory_unit_ids.is_empty()
                && !candidate.supporting_archive_ids.is_empty()
            {
                result.warnings.push(format!(
                    "candidate {} has archive provenance but no MemoryUnit IDs; migration did not invent an evidence edge",
                    candidate.candidate_id
                ));
            }
            if !candidate.contradicting_archive_ids.is_empty() {
                result.warnings.push(format!(
                    "candidate {} has archive-level contradiction provenance; migration kept it on the node because no exact MemoryUnit ID is available",
                    candidate.candidate_id
                ));
            }

            let mut prepared_links = Vec::new();
            let mut links = evidence_links_from_candidate(&candidate, |memory_unit_id| {
                self.storage
                    .read_memory_unit_by_id(memory_unit_id)
                    .ok()
                    .map(|unit| ResolvedHypothesisEvidenceSource {
                        archive_id: unit.archive_id,
                        source_session_id: unit.source_session_id,
                    })
            });
            for link in &mut links {
                if link.resolution == HypothesisEvidenceResolution::Unresolved {
                    result
                        .unresolved_memory_unit_ids
                        .push(link.memory_unit_id.clone());
                }
                let existing = self
                    .storage
                    .read_hypothesis_evidence_link_optional(&link.evidence_link_id)?;
                if let Some(existing) = existing.clone() {
                    link.merge_existing_graph_fields(existing);
                }
                link.validate_basic()?;
                match existing.as_ref() {
                    None => result.evidence_links_created += 1,
                    Some(existing) if existing == link => result.evidence_links_unchanged += 1,
                    Some(_) => result.evidence_links_updated += 1,
                }
                target_files.push(format!(
                    "core/hypotheses/evidence/{}.json",
                    link.evidence_link_id
                ));
                prepared_links.push(PreparedHypothesisEvidenceMigration {
                    desired: link.clone(),
                    previous: existing,
                });
            }
            prepared_candidates.push(PreparedHypothesisCandidateMigration {
                candidate_id: candidate.candidate_id,
                desired_node: node,
                previous_node: existing_node,
                evidence_links: prepared_links,
            });
        }

        result.unresolved_memory_unit_ids.sort();
        result.unresolved_memory_unit_ids.dedup();
        result.warnings.sort();
        result.warnings.dedup();

        target_files.sort();
        target_files.dedup();
        Ok((
            HypothesisGraphMigrationOperationIntent {
                schema_version: HYPOTHESIS_GRAPH_MIGRATION_OPERATION_SCHEMA_VERSION.to_string(),
                migrated_at,
                prepared_candidates,
                applied_candidate_ids: Vec::new(),
                manifest_updated: false,
                result,
            },
            target_files,
        ))
    }

    pub(super) fn recover_hypothesis_graph_migration_unlocked(
        &self,
        operation: JournalOperation,
    ) -> Result<HypothesisGraphMigrationResult> {
        self.execute_hypothesis_graph_migration_unlocked(operation)
    }

    fn execute_hypothesis_graph_migration_unlocked(
        &self,
        operation: JournalOperation,
    ) -> Result<HypothesisGraphMigrationResult> {
        let operation_id = operation.op_id.clone();
        let result = self.execute_hypothesis_graph_migration_inner(operation);
        if let Err(error) = &result {
            self.record_memory_operation_error_best_effort(&operation_id, error);
        }
        result
    }

    fn execute_hypothesis_graph_migration_inner(
        &self,
        mut operation: JournalOperation,
    ) -> Result<HypothesisGraphMigrationResult> {
        if operation.schema_version != JOURNAL_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: JOURNAL_OPERATION_SCHEMA_VERSION.to_string(),
                actual: operation.schema_version,
            });
        }
        if operation.op_type != JournalOperationType::Migration {
            return Err(MemoryEngineError::Validation(format!(
                "operation is not a hypothesis graph migration: {}",
                operation.op_id
            )));
        }
        let mut intent: HypothesisGraphMigrationOperationIntent =
            serde_json::from_value(operation.intent.clone())?;
        if intent.schema_version != HYPOTHESIS_GRAPH_MIGRATION_OPERATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: HYPOTHESIS_GRAPH_MIGRATION_OPERATION_SCHEMA_VERSION.to_string(),
                actual: intent.schema_version,
            });
        }
        if operation.state == JournalState::Completed {
            return Ok(intent.result);
        }

        for prepared in intent.prepared_candidates.clone() {
            self.with_resource_lock(candidate_lock_key(&prepared.candidate_id), || {
                let current = self
                    .storage
                    .read_hypothesis_node_optional(&prepared.desired_node.hypothesis_id)?;
                if current.as_ref() != Some(&prepared.desired_node) {
                    if current != prepared.previous_node {
                        return Err(MemoryEngineError::Validation(format!(
                            "hypothesis node changed while migration was pending: {}",
                            prepared.desired_node.hypothesis_id
                        )));
                    }
                    self.storage.write_hypothesis_node(&prepared.desired_node)?;
                }
                for evidence in &prepared.evidence_links {
                    let current = self.storage.read_hypothesis_evidence_link_optional(
                        &evidence.desired.evidence_link_id,
                    )?;
                    if current.as_ref() != Some(&evidence.desired) {
                        if current != evidence.previous {
                            return Err(MemoryEngineError::Validation(format!(
                                "hypothesis evidence changed while migration was pending: {}",
                                evidence.desired.evidence_link_id
                            )));
                        }
                        self.storage
                            .write_hypothesis_evidence_link(&evidence.desired)?;
                    }
                }
                Ok(())
            })?;
            if !intent
                .applied_candidate_ids
                .contains(&prepared.candidate_id)
            {
                intent.applied_candidate_ids.push(prepared.candidate_id);
                self.persist_hypothesis_graph_migration_operation(&mut operation, &intent)?;
            }
        }

        let mut manifest = self.storage.read_manifest()?;
        manifest.schema_versions.hypothesis_node = HYPOTHESIS_NODE_SCHEMA_VERSION.to_string();
        manifest.schema_versions.hypothesis_evidence_link =
            HYPOTHESIS_EVIDENCE_LINK_SCHEMA_VERSION.to_string();
        manifest.schema_versions.hypothesis_relation =
            HYPOTHESIS_RELATION_SCHEMA_VERSION.to_string();
        manifest.last_migration_at = Some(intent.migrated_at.clone());
        manifest.updated_at = intent.migrated_at.clone();
        self.storage.write_manifest(&manifest)?;
        if !intent.manifest_updated {
            intent.manifest_updated = true;
            self.persist_hypothesis_graph_migration_operation(&mut operation, &intent)?;
        }

        operation.state = JournalState::Completed;
        operation.completed_at = Some(now_rfc3339()?);
        operation.updated_at = operation
            .completed_at
            .clone()
            .unwrap_or_else(|| operation.updated_at.clone());
        operation.error = None;
        operation.intent = serde_json::to_value(&intent)?;
        self.storage.begin_journaled_operation(&operation)?;
        Ok(intent.result)
    }

    fn persist_hypothesis_graph_migration_operation(
        &self,
        operation: &mut JournalOperation,
        intent: &HypothesisGraphMigrationOperationIntent,
    ) -> Result<()> {
        operation.updated_at = now_rfc3339()?;
        operation.intent = serde_json::to_value(intent)?;
        self.storage.begin_journaled_operation(operation)
    }

    /// Export one exact personality scope for human inspection. `None` means
    /// the isolated unscoped namespace, never a wildcard across people.
    pub fn export_hypothesis_graph(
        &self,
        request: HypothesisGraphExportRequest,
    ) -> Result<HypothesisGraphExport> {
        if request.schema_version != crate::types::HYPOTHESIS_GRAPH_EXPORT_REQUEST_SCHEMA_VERSION {
            return Err(MemoryEngineError::Validation(format!(
                "unsupported hypothesis graph export request schema_version: {}",
                request.schema_version
            )));
        }
        self.ensure_manifest()?;
        let scope = normalize_graph_scope(request.core_scope);
        let lock_key = format!(
            "hypothesis_graph:{}",
            scope.as_deref().unwrap_or("__unscoped__")
        );
        self.with_resource_lock(lock_key, || self.export_hypothesis_graph_unlocked(scope))
    }

    fn export_hypothesis_graph_unlocked(
        &self,
        core_scope: Option<String>,
    ) -> Result<HypothesisGraphExport> {
        let node_collection = self.storage.read_hypothesis_nodes()?;
        let evidence_collection = self.storage.read_hypothesis_evidence_links()?;
        let relation_collection = self.storage.read_hypothesis_relations()?;

        let mut export = HypothesisGraphExport::new(core_scope.clone());
        export.nodes = node_collection
            .items
            .into_iter()
            .filter(|node| node.core_scope == core_scope)
            .collect();
        let node_ids: HashSet<&str> = export
            .nodes
            .iter()
            .map(|node| node.hypothesis_id.as_str())
            .collect();
        export.evidence_links = evidence_collection
            .items
            .into_iter()
            .filter(|link| node_ids.contains(link.hypothesis_id.as_str()))
            .collect();
        export.relations = relation_collection
            .items
            .into_iter()
            .filter(|relation| {
                node_ids.contains(relation.source_hypothesis_id.as_str())
                    && node_ids.contains(relation.target_hypothesis_id.as_str())
            })
            .collect();
        export.warnings.extend(
            node_collection
                .warnings
                .iter()
                .chain(evidence_collection.warnings.iter())
                .chain(relation_collection.warnings.iter())
                .map(StorageReadWarning::note),
        );
        export.warnings.sort();
        export.warnings.dedup();
        Ok(export)
    }
}

fn normalize_graph_scope(scope: Option<String>) -> Option<String> {
    scope.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}
