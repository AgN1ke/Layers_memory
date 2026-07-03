use super::*;

impl<S: Storage> MemoryEngine<S> {
    pub fn vector_state(&self, scope: &str) -> Result<VectorScopeState> {
        self.ensure_manifest()?;
        validate_vector_scope(scope)?;
        self.with_resource_lock(vectors_lock_key(scope), || {
            self.vector_state_unlocked(scope)
        })
    }

    pub fn set_vector_scope(
        &self,
        scope: &str,
        enabled: bool,
        _purge: bool,
    ) -> Result<VectorScopeState> {
        self.ensure_manifest()?;
        validate_vector_scope(scope)?;
        self.with_resource_lock(vectors_lock_key(scope), || {
            if !enabled {
                // Vectors are derived data. In v1, disabling a scope removes
                // the catalog so disabled truly means no vector reads.
                self.storage.purge_vector_scope(scope)?;
                return Ok(disabled_vector_state(scope, "scope disabled"));
            }

            let now = now_rfc3339()?;
            let mut root_manifest = self.storage.read_manifest()?;
            root_manifest.features.embeddings_enabled = true;
            root_manifest.active_embedding_model_id = Some(self.options.vectors.model_id.clone());
            root_manifest.updated_at = now.clone();
            self.storage.write_manifest(&root_manifest)?;

            if self.storage.read_vector_index(scope)?.is_none() {
                let manifest = default_vector_manifest(
                    &self.options.vectors.model_id,
                    self.options.vectors.dim,
                    &now,
                );
                self.storage.write_vector_manifest(scope, &manifest)?;
            }
            self.vector_state_unlocked(scope)
        })
    }

    pub fn rebuild_vectors(&self, scope: &str) -> Result<VectorScopeState> {
        self.ensure_manifest()?;
        validate_vector_scope(scope)?;
        self.with_resource_lock(vectors_lock_key(scope), || {
            self.storage.purge_vector_scope(scope)?;
            let now = now_rfc3339()?;
            let mut root_manifest = self.storage.read_manifest()?;
            root_manifest.features.embeddings_enabled = true;
            root_manifest.active_embedding_model_id = Some(self.options.vectors.model_id.clone());
            root_manifest.updated_at = now.clone();
            self.storage.write_manifest(&root_manifest)?;
            let manifest = default_vector_manifest(
                &self.options.vectors.model_id,
                self.options.vectors.dim,
                &now,
            );
            self.storage.write_vector_manifest(scope, &manifest)?;
            self.vector_state_unlocked(scope)
        })
    }

    pub fn pending_embedding_backfill(&self, scope: &str) -> Result<Vec<LlmRequest>> {
        self.ensure_manifest()?;
        validate_vector_scope(scope)?;
        self.with_resource_lock(vectors_lock_key(scope), || {
            self.pending_embedding_backfill_unlocked(scope)
        })
    }

    pub fn resume_compute_embedding(
        &self,
        task_id: &str,
        result: EmbedBatchResult,
    ) -> Result<usize> {
        if result.schema_version != EMBED_BATCH_RESULT_SCHEMA_VERSION {
            return Err(MemoryEngineError::IncompatibleSchema {
                expected: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
                actual: result.schema_version.clone(),
            });
        }
        self.ensure_manifest()?;

        let task = self.storage.load_task(task_id)?;
        if task.task_type != TaskType::ComputeEmbedding {
            return Err(MemoryEngineError::Validation(format!(
                "task is not compute_embedding: {task_id}"
            )));
        }
        let inputs: EmbedBatchInputs = serde_json::from_value(task.inputs.clone())?;
        if inputs.kind != "embed_batch" {
            return Err(MemoryEngineError::Validation(format!(
                "compute_embedding task has unsupported kind: {}",
                inputs.kind
            )));
        }
        if result.model_id != inputs.model_id || result.dim != inputs.dim {
            return Err(MemoryEngineError::Validation(format!(
                "embedding result model/dim mismatch: task={} {}d result={} {}d",
                inputs.model_id, inputs.dim, result.model_id, result.dim
            )));
        }

        self.with_resource_lock(vectors_lock_key(&inputs.scope), || {
            let now = now_rfc3339()?;
            let Some(mut index) = self.storage.read_vector_index(&inputs.scope)? else {
                return Err(MemoryEngineError::Validation(format!(
                    "vector scope is disabled: {}",
                    inputs.scope
                )));
            };
            ensure_vector_manifest_matches(&index.manifest, &inputs.model_id, inputs.dim)?;
            if index.manifest.state == VectorScopeStatus::Corrupt {
                return Err(MemoryEngineError::Validation(format!(
                    "vector scope is corrupt: {}",
                    inputs.scope
                )));
            }

            let result_by_id = result
                .results
                .iter()
                .map(|item| (item.memory_unit_id.as_str(), item))
                .collect::<HashMap<_, _>>();
            let tombstoned = index
                .tombstones
                .iter()
                .map(|item| item.memory_unit_id.as_str())
                .collect::<HashSet<_>>();
            let live_existing = index
                .rows
                .iter()
                .filter(|row| !tombstoned.contains(row.memory_unit_id.as_str()))
                .map(|row| (row.memory_unit_id.as_str(), row.thesis_hash.as_str()))
                .collect::<HashMap<_, _>>();

            let mut records = Vec::new();
            for item in &inputs.items {
                let Some(vector_result) = result_by_id.get(item.memory_unit_id.as_str()) else {
                    return Err(MemoryEngineError::Validation(format!(
                        "embedding result missing memory_unit_id: {}",
                        item.memory_unit_id
                    )));
                };
                let unit = self.storage.read_memory_unit_by_id(&item.memory_unit_id)?;
                if !memory_unit_is_vector_eligible(&unit) {
                    continue;
                }
                let hash = thesis_hash(&unit.thesis);
                if hash != thesis_hash(&item.text) {
                    continue;
                }
                if live_existing
                    .get(item.memory_unit_id.as_str())
                    .is_some_and(|existing_hash| *existing_hash == hash)
                {
                    continue;
                }
                let vector = normalize_vector(vector_result.vector.clone(), inputs.dim)?;
                let row_index = index.rows.len() + records.len();
                records.push(VectorAppendRecord {
                    row: VectorRow {
                        row: row_index,
                        memory_unit_id: unit.memory_unit_id.clone(),
                        archive_id: unit.archive_id.clone(),
                        created_at: unit.created_at.clone(),
                        thesis_hash: hash,
                    },
                    vector,
                });
            }

            index.manifest.rows = index.rows.len() + records.len();
            index.manifest.updated_at = now.clone();
            index.manifest.backfill_cursor =
                records.last().map(|record| record.row.created_at.clone());
            index.manifest.state = VectorScopeStatus::Building;
            self.storage
                .append_vector_records(&inputs.scope, &index.manifest, &records)?;

            let mut task = self.storage.load_task(task_id)?;
            task.state = TaskState::Completed;
            task.updated_at = now.clone();
            task.last_error = None;
            self.storage.save_task(&task)?;

            let mut index = self
                .storage
                .read_vector_index(&inputs.scope)?
                .ok_or_else(|| {
                    MemoryEngineError::Validation(format!(
                        "vector scope disappeared: {}",
                        inputs.scope
                    ))
                })?;
            if !self.has_missing_vector_units_unlocked(&inputs.scope, &index)? {
                index.manifest.state = VectorScopeStatus::Ready;
                index.manifest.updated_at = now;
                self.storage
                    .write_vector_manifest(&inputs.scope, &index.manifest)?;
            }

            Ok(records.len())
        })
    }

    pub fn recall_deep(&self, query: DeepRecallQuery) -> Result<DeepRecallResult> {
        self.ensure_manifest()?;
        validate_vector_scope(&query.scope)?;

        self.with_resource_lock(vectors_lock_key(&query.scope), || {
            let root_manifest = self.storage.read_manifest()?;
            if !root_manifest.features.embeddings_enabled {
                return Ok(deep_recall_empty("disabled"));
            }

            let Some(index) = self.storage.read_vector_index(&query.scope)? else {
                return Ok(deep_recall_empty("disabled"));
            };
            ensure_vector_manifest_matches(
                &index.manifest,
                &query.model_id,
                query.query_vec.len(),
            )?;
            if index.manifest.state == VectorScopeStatus::Corrupt {
                return Ok(deep_recall_empty("corrupt"));
            }
            if index.manifest.state != VectorScopeStatus::Ready {
                return Ok(deep_recall_empty("building"));
            }

            let query_vec = normalize_vector(query.query_vec.clone(), index.manifest.dim)?;
            let min_sim = if query.min_sim > 0.0 {
                query.min_sim
            } else {
                self.options.vectors.deep_recall_min_sim
            };
            let top_k = if query.top_k == 0 {
                self.options.vectors.deep_recall_default_top_k
            } else {
                query.top_k
            }
            .max(1);
            let now = query.now.clone().map_or_else(now_rfc3339, Ok)?;
            let tombstoned = index
                .tombstones
                .iter()
                .map(|item| item.memory_unit_id.as_str())
                .collect::<HashSet<_>>();

            let mut scored = Vec::new();
            for (row, vector) in index.rows.iter().zip(index.vectors.iter()) {
                if tombstoned.contains(row.memory_unit_id.as_str()) {
                    continue;
                }
                let sim = dot_product(&query_vec, vector);
                if sim < min_sim {
                    continue;
                }
                let Ok(unit) = self.storage.read_memory_unit_by_id(&row.memory_unit_id) else {
                    continue;
                };
                if unit.source_session_id != query.scope || !memory_unit_is_vector_eligible(&unit) {
                    continue;
                }
                let age_days = timestamp_age_days(&unit.created_at, &now).unwrap_or(0.0);
                let recency =
                    half_life_decay_factor(age_days, self.options.recall.freshness_half_life_days)
                        as f32;
                let score = sim
                    + (self.options.vectors.deep_recall_recency_weight * recency)
                    + (self.options.vectors.deep_recall_unit_weight
                        * unit.weight.clamp(0.0, 1.0) as f32);
                scored.push(DeepRecallHit {
                    memory_unit_id: unit.memory_unit_id,
                    archive_id: unit.archive_id,
                    thesis: unit.thesis,
                    created_at: unit.created_at,
                    sim,
                    score,
                });
            }

            scored.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| right.sim.total_cmp(&left.sim))
                    .then_with(|| left.created_at.cmp(&right.created_at))
                    .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
            });
            scored.truncate(top_k);

            if scored.is_empty() {
                return Ok(deep_recall_empty("below_threshold"));
            }

            let archive_ids = scored
                .iter()
                .map(|hit| hit.archive_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            self.record_recall_stats(&archive_ids, &now)?;

            Ok(DeepRecallResult {
                schema_version: DEEP_RECALL_RESULT_SCHEMA_VERSION.to_string(),
                found: true,
                reason: None,
                hits: scored,
            })
        })
    }

    pub(super) fn pending_embedding_backfill_unlocked(
        &self,
        scope: &str,
    ) -> Result<Vec<LlmRequest>> {
        let root_manifest = self.storage.read_manifest()?;
        if !root_manifest.features.embeddings_enabled {
            return Ok(Vec::new());
        }
        let Some(mut index) = self.storage.read_vector_index(scope)? else {
            return Ok(Vec::new());
        };
        ensure_vector_manifest_matches(
            &index.manifest,
            &self.options.vectors.model_id,
            self.options.vectors.dim,
        )?;
        if index.manifest.state == VectorScopeStatus::Corrupt {
            return Err(MemoryEngineError::Validation(format!(
                "vector scope is corrupt: {scope}"
            )));
        }

        self.compact_vector_scope_if_needed_unlocked(scope, &mut index)?;

        let eligible = self.eligible_vector_units_for_scope_unlocked(scope)?;
        let tombstoned = index
            .tombstones
            .iter()
            .map(|item| item.memory_unit_id.clone())
            .collect::<HashSet<_>>();
        let existing = index
            .rows
            .iter()
            .filter(|row| !tombstoned.contains(&row.memory_unit_id))
            .map(|row| (row.memory_unit_id.clone(), row.thesis_hash.clone()))
            .collect::<HashMap<_, _>>();
        let pending = self.pending_embedding_unit_ids_for_scope(scope)?;

        let mut missing = Vec::new();
        let mut new_tombstones = Vec::new();
        for unit in eligible {
            let hash = thesis_hash(&unit.thesis);
            if pending.contains(&unit.memory_unit_id) {
                continue;
            }
            match existing.get(&unit.memory_unit_id) {
                Some(existing_hash) if existing_hash == &hash => continue,
                Some(_) => {
                    new_tombstones.push(VectorTombstone {
                        memory_unit_id: unit.memory_unit_id.clone(),
                    });
                    missing.push(unit);
                }
                None => missing.push(unit),
            }
        }
        self.storage
            .append_vector_tombstones(scope, &new_tombstones)?;

        let now = now_rfc3339()?;
        if missing.is_empty() {
            index.manifest.state = if pending.is_empty() {
                VectorScopeStatus::Ready
            } else {
                VectorScopeStatus::Building
            };
            index.manifest.updated_at = now;
            self.storage.write_vector_manifest(scope, &index.manifest)?;
            return Ok(Vec::new());
        }

        let batch_size = self.options.vectors.embed_batch_size.max(1);
        let mut requests = Vec::new();
        for chunk in missing.chunks(batch_size) {
            let inputs = EmbedBatchInputs {
                kind: "embed_batch".to_string(),
                scope: scope.to_string(),
                model_id: self.options.vectors.model_id.clone(),
                dim: self.options.vectors.dim,
                items: chunk
                    .iter()
                    .map(|unit| EmbedBatchItem {
                        memory_unit_id: unit.memory_unit_id.clone(),
                        text: unit.thesis.clone(),
                    })
                    .collect(),
            };
            let task = PendingTask {
                schema_version: PENDING_TASK_SCHEMA_VERSION.to_string(),
                task_id: new_id("task")?,
                task_type: TaskType::ComputeEmbedding,
                state: TaskState::Pending,
                created_at: now.clone(),
                updated_at: now.clone(),
                prompt_id: "embed_batch".to_string(),
                prompt_version: 1,
                role_hint: ModelRole::Fast,
                expected_output_schema: EMBED_BATCH_RESULT_SCHEMA_VERSION.to_string(),
                inputs: serde_json::to_value(&inputs)?,
                attempts: Vec::new(),
                last_error: None,
            };
            self.storage.save_task(&task)?;
            requests.push(llm_request_from_task(
                &task,
                "embed_batch",
                json!({ "embed_batch": inputs }),
            )?);
        }

        index.manifest.state = VectorScopeStatus::Building;
        index.manifest.updated_at = now;
        index.manifest.backfill_cursor = missing.last().map(|unit| unit.created_at.clone());
        self.storage.write_vector_manifest(scope, &index.manifest)?;
        Ok(requests)
    }

    pub(super) fn tombstone_vector_unit_if_indexed_unlocked(
        &self,
        unit: &MemoryUnit,
    ) -> Result<()> {
        let scope = &unit.source_session_id;
        if !self.storage.read_manifest()?.features.embeddings_enabled {
            return Ok(());
        }
        self.with_resource_lock(vectors_lock_key(scope), || {
            if self.storage.read_vector_index(scope)?.is_some() {
                self.storage.append_vector_tombstones(
                    scope,
                    &[VectorTombstone {
                        memory_unit_id: unit.memory_unit_id.clone(),
                    }],
                )?;
            }
            Ok(())
        })
    }

    fn vector_state_unlocked(&self, scope: &str) -> Result<VectorScopeState> {
        let root_manifest = self.storage.read_manifest()?;
        if !root_manifest.features.embeddings_enabled {
            return Ok(disabled_vector_state(scope, "global embeddings disabled"));
        }
        let Some(index) = self.storage.read_vector_index(scope)? else {
            return Ok(disabled_vector_state(scope, "scope catalog absent"));
        };
        Ok(VectorScopeState {
            schema_version: "vector_scope_state.v1".to_string(),
            scope: scope.to_string(),
            status: index.manifest.state,
            rows: index.manifest.rows,
            model_id: Some(index.manifest.model_id),
            dim: Some(index.manifest.dim),
            updated_at: Some(index.manifest.updated_at),
            reason: None,
        })
    }

    fn compact_vector_scope_if_needed_unlocked(
        &self,
        scope: &str,
        index: &mut VectorIndexData,
    ) -> Result<()> {
        if index.tombstones.is_empty() {
            return Ok(());
        }
        let tombstoned = index
            .tombstones
            .iter()
            .map(|item| item.memory_unit_id.as_str())
            .collect::<HashSet<_>>();
        let mut rows = Vec::new();
        let mut vectors = Vec::new();
        for (row, vector) in index.rows.iter().zip(index.vectors.iter()) {
            if tombstoned.contains(row.memory_unit_id.as_str()) {
                continue;
            }
            let mut row = row.clone();
            row.row = rows.len();
            rows.push(row);
            vectors.push(vector.clone());
        }
        index.rows = rows;
        index.vectors = vectors;
        index.tombstones.clear();
        index.manifest.rows = index.rows.len();
        index.manifest.updated_at = now_rfc3339()?;
        self.storage.write_vector_index(scope, index)
    }

    fn eligible_vector_units_for_scope_unlocked(&self, scope: &str) -> Result<Vec<MemoryUnit>> {
        let events = self.read_session_events_with_archived(scope)?;
        if session_is_multi_speaker(&events) {
            return Ok(Vec::new());
        }
        let mut units = self
            .storage
            .read_archive(&ArchiveFilters::default())?
            .into_items()
            .into_iter()
            .filter(|archive| {
                archive.status == ArchiveStatus::Complete && archive.source_session_id == scope
            })
            .flat_map(|archive| archive.memory_units.into_iter())
            .filter(memory_unit_is_vector_eligible)
            .collect::<Vec<_>>();
        units.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.memory_unit_id.cmp(&right.memory_unit_id))
        });
        Ok(units)
    }

    fn has_missing_vector_units_unlocked(
        &self,
        scope: &str,
        index: &VectorIndexData,
    ) -> Result<bool> {
        let tombstoned = index
            .tombstones
            .iter()
            .map(|item| item.memory_unit_id.clone())
            .collect::<HashSet<_>>();
        let existing = index
            .rows
            .iter()
            .filter(|row| !tombstoned.contains(&row.memory_unit_id))
            .map(|row| (row.memory_unit_id.clone(), row.thesis_hash.clone()))
            .collect::<HashMap<_, _>>();
        for unit in self.eligible_vector_units_for_scope_unlocked(scope)? {
            if existing.get(&unit.memory_unit_id) != Some(&thesis_hash(&unit.thesis)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn pending_embedding_unit_ids_for_scope(&self, scope: &str) -> Result<HashSet<String>> {
        let mut ids = HashSet::new();
        for task in self.storage.load_tasks()?.into_items() {
            if task.task_type != TaskType::ComputeEmbedding
                || !matches!(task.state, TaskState::Pending | TaskState::Submitted)
            {
                continue;
            }
            let Ok(inputs) = serde_json::from_value::<EmbedBatchInputs>(task.inputs.clone()) else {
                continue;
            };
            if inputs.scope != scope {
                continue;
            }
            ids.extend(inputs.items.into_iter().map(|item| item.memory_unit_id));
        }
        Ok(ids)
    }
}

fn disabled_vector_state(scope: &str, reason: &str) -> VectorScopeState {
    VectorScopeState {
        schema_version: "vector_scope_state.v1".to_string(),
        scope: scope.to_string(),
        status: VectorScopeStatus::Disabled,
        rows: 0,
        model_id: None,
        dim: None,
        updated_at: None,
        reason: Some(reason.to_string()),
    }
}

fn deep_recall_empty(reason: &str) -> DeepRecallResult {
    DeepRecallResult {
        schema_version: DEEP_RECALL_RESULT_SCHEMA_VERSION.to_string(),
        found: false,
        reason: Some(reason.to_string()),
        hits: Vec::new(),
    }
}

fn dot_product(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>()
}

fn ensure_vector_manifest_matches(
    manifest: &crate::vector::VectorIndexManifest,
    model_id: &str,
    dim: usize,
) -> Result<()> {
    if manifest.model_id != model_id || manifest.dim != dim {
        return Err(MemoryEngineError::Validation(format!(
            "vector model/dim mismatch: index={} {}d requested={} {}d",
            manifest.model_id, manifest.dim, model_id, dim
        )));
    }
    Ok(())
}

fn validate_vector_scope(scope: &str) -> Result<()> {
    if scope.trim().is_empty()
        || scope.contains('/')
        || scope.contains('\\')
        || scope == "."
        || scope == ".."
        || scope.contains("..")
    {
        return Err(MemoryEngineError::Validation(format!(
            "invalid vector scope: {scope}"
        )));
    }
    Ok(())
}
