use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core_store::{
    CandidateBelief, CandidateReinforcement, CandidateStatus, PromotionChecks, ReviewRecord,
};
use crate::types::{
    Id, Link, Timestamp, CANDIDATE_BELIEF_SCHEMA_VERSION, HYPOTHESIS_EVIDENCE_LINK_SCHEMA_VERSION,
    HYPOTHESIS_GRAPH_EXPORT_SCHEMA_VERSION, HYPOTHESIS_GRAPH_MIGRATION_RESULT_SCHEMA_VERSION,
    HYPOTHESIS_NODE_SCHEMA_VERSION, HYPOTHESIS_RELATION_SCHEMA_VERSION,
};
use crate::{MemoryEngineError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisStatus {
    Draft,
    ReadyForReview,
    Approved,
    Rejected,
    Promoted,
    Superseded,
}

impl From<CandidateStatus> for HypothesisStatus {
    fn from(value: CandidateStatus) -> Self {
        match value {
            CandidateStatus::Draft => Self::Draft,
            CandidateStatus::ReadyForReview => Self::ReadyForReview,
            CandidateStatus::Approved => Self::Approved,
            CandidateStatus::Rejected => Self::Rejected,
            CandidateStatus::Promoted => Self::Promoted,
            CandidateStatus::Superseded => Self::Superseded,
        }
    }
}

impl From<HypothesisStatus> for CandidateStatus {
    fn from(value: HypothesisStatus) -> Self {
        match value {
            HypothesisStatus::Draft => Self::Draft,
            HypothesisStatus::ReadyForReview => Self::ReadyForReview,
            HypothesisStatus::Approved => Self::Approved,
            HypothesisStatus::Rejected => Self::Rejected,
            HypothesisStatus::Promoted => Self::Promoted,
            HypothesisStatus::Superseded => Self::Superseded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisValidity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<Timestamp>,
}

/// A reviewable belief between Archive evidence and owner-approved Core.
///
/// Phase 2 deliberately preserves the complete CandidateBelief surface so the
/// existing reflection lifecycle can be projected through this record without
/// changing prompt behavior. Evidence identity lives in separate links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisNode {
    pub schema_version: String,
    pub hypothesis_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_candidate_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_session_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub statement: String,
    pub category: String,
    /// Unknown for migrated flat candidates. Phase 3 may assign explicit
    /// narrow-to-broad levels, but migration never guesses them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstraction_level: Option<u32>,
    pub status: HypothesisStatus,
    /// Explainable support signal inherited from v0.5 confidence for migrated
    /// candidates. It is not a probability of truth.
    pub support_score: f64,
    /// Kept separate from support. `None` means that the flat source record did
    /// not contain a defensible contradiction score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contradiction_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<HypothesisValidity>,
    pub evidence_summary: String,
    pub promotion_checks: PromotionChecks,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicting_archive_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicted_core_fact_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_core_fact_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reinforcements: Vec<CandidateReinforcement>,
}

impl HypothesisNode {
    pub fn from_candidate(candidate: &CandidateBelief) -> Self {
        Self {
            schema_version: HYPOTHESIS_NODE_SCHEMA_VERSION.to_string(),
            hypothesis_id: candidate.candidate_id.clone(),
            legacy_candidate_id: Some(candidate.candidate_id.clone()),
            source_session_id: candidate.source_session_id.clone(),
            source_session_ids: candidate.source_session_ids.clone(),
            core_scope: candidate.core_scope.clone(),
            created_at: candidate.created_at.clone(),
            updated_at: candidate.updated_at.clone(),
            statement: candidate.text.clone(),
            category: candidate.category.clone(),
            abstraction_level: None,
            status: candidate.status.into(),
            support_score: candidate.confidence,
            contradiction_score: None,
            validity: None,
            evidence_summary: candidate.evidence_summary.clone(),
            promotion_checks: candidate.promotion_checks.clone(),
            supporting_archive_ids: candidate.supporting_archive_ids.clone(),
            contradicting_archive_ids: candidate.contradicting_archive_ids.clone(),
            contradicted_core_fact_ids: candidate.contradicted_core_fact_ids.clone(),
            source_memory_unit_ids: candidate.source_memory_unit_ids.clone(),
            tags: candidate.tags.clone(),
            promoted_core_fact_id: candidate.promoted_core_fact_id.clone(),
            review: candidate.review.clone(),
            links: candidate.links.clone(),
            reinforcements: candidate.reinforcements.clone(),
        }
    }

    /// Refresh fields owned by the v0.5 candidate lifecycle while preserving
    /// graph-native meaning that the flat projection cannot represent.
    pub fn merge_candidate_projection(&mut self, candidate: &CandidateBelief) {
        self.legacy_candidate_id = Some(candidate.candidate_id.clone());
        self.source_session_id = candidate.source_session_id.clone();
        self.source_session_ids = candidate.source_session_ids.clone();
        self.core_scope = candidate.core_scope.clone();
        self.created_at = candidate.created_at.clone();
        self.updated_at = candidate.updated_at.clone();
        self.statement = candidate.text.clone();
        self.category = candidate.category.clone();
        self.status = candidate.status.into();
        self.support_score = candidate.confidence;
        self.evidence_summary = candidate.evidence_summary.clone();
        self.promotion_checks = candidate.promotion_checks.clone();
        self.supporting_archive_ids = candidate.supporting_archive_ids.clone();
        self.contradicting_archive_ids = candidate.contradicting_archive_ids.clone();
        self.contradicted_core_fact_ids = candidate.contradicted_core_fact_ids.clone();
        self.source_memory_unit_ids = candidate.source_memory_unit_ids.clone();
        self.tags = candidate.tags.clone();
        self.promoted_core_fact_id = candidate.promoted_core_fact_id.clone();
        self.review = candidate.review.clone();
        self.links = candidate.links.clone();
        self.reinforcements = candidate.reinforcements.clone();
    }

    pub fn to_candidate(&self) -> CandidateBelief {
        CandidateBelief {
            schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
            candidate_id: self
                .legacy_candidate_id
                .clone()
                .unwrap_or_else(|| self.hypothesis_id.clone()),
            source_session_id: self.source_session_id.clone(),
            source_session_ids: self.source_session_ids.clone(),
            core_scope: self.core_scope.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            text: self.statement.clone(),
            category: self.category.clone(),
            status: self.status.into(),
            confidence: self.support_score,
            supporting_archive_ids: self.supporting_archive_ids.clone(),
            contradicting_archive_ids: self.contradicting_archive_ids.clone(),
            contradicted_core_fact_ids: self.contradicted_core_fact_ids.clone(),
            evidence_summary: self.evidence_summary.clone(),
            promotion_checks: self.promotion_checks.clone(),
            source_memory_unit_ids: self.source_memory_unit_ids.clone(),
            tags: self.tags.clone(),
            promoted_core_fact_id: self.promoted_core_fact_id.clone(),
            review: self.review.clone(),
            links: self.links.clone(),
            reinforcements: self.reinforcements.clone(),
        }
    }

    pub fn validate_basic(&self) -> Result<()> {
        if self.schema_version != HYPOTHESIS_NODE_SCHEMA_VERSION {
            return Err(MemoryEngineError::Validation(format!(
                "unsupported hypothesis node schema_version: {}",
                self.schema_version
            )));
        }
        validate_hypothesis_storage_id("hypothesis_id", &self.hypothesis_id)?;
        if let Some(candidate_id) = &self.legacy_candidate_id {
            validate_hypothesis_storage_id("legacy_candidate_id", candidate_id)?;
        }
        for memory_unit_id in &self.source_memory_unit_ids {
            validate_hypothesis_storage_id("source_memory_unit_id", memory_unit_id)?;
        }
        if self.statement.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "hypothesis statement must not be empty".to_string(),
            ));
        }
        validate_score("hypothesis support_score", self.support_score)?;
        if let Some(score) = self.contradiction_score {
            validate_score("hypothesis contradiction_score", score)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisEvidencePolarity {
    Supports,
    Contradicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisEvidenceDirectness {
    Direct,
    Inferred,
    LegacyUnspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisEvidenceResolution {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisEvidenceOriginKind {
    Candidate,
    Reinforcement,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisEvidenceOrigin {
    pub kind: HypothesisEvidenceOriginKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_id: Option<Id>,
    pub created_at: Timestamp,
    pub summary: String,
    pub strength: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisEvidenceLink {
    pub schema_version: String,
    pub evidence_link_id: Id,
    pub hypothesis_id: Id,
    pub memory_unit_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<Id>,
    pub polarity: HypothesisEvidencePolarity,
    pub directness: HypothesisEvidenceDirectness,
    /// Unknown for a migrated flat candidate; Phase 3 may populate it when a
    /// wider hypothesis inherits support from a narrower one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generalization_distance: Option<u32>,
    pub resolution: HypothesisEvidenceResolution,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Several legacy records may cite one unit, but they remain one evidence
    /// edge and therefore one independent source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<HypothesisEvidenceOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHypothesisEvidenceSource {
    pub archive_id: Id,
    pub source_session_id: Id,
}

/// Convert the evidence that a flat candidate can actually prove into one
/// independent edge per MemoryUnit. Archive-only contradiction references stay
/// on the node because mapping them to a specific unit would be an invention.
pub fn evidence_links_from_candidate<F>(
    candidate: &CandidateBelief,
    mut resolve: F,
) -> Vec<HypothesisEvidenceLink>
where
    F: FnMut(&str) -> Option<ResolvedHypothesisEvidenceSource>,
{
    let mut links = HashMap::<String, HypothesisEvidenceLink>::new();
    let reinforcement_unit_ids: HashSet<&str> = candidate
        .reinforcements
        .iter()
        .flat_map(|reinforcement| {
            reinforcement
                .source_memory_unit_ids
                .iter()
                .map(String::as_str)
        })
        .collect();

    for memory_unit_id in &candidate.source_memory_unit_ids {
        if reinforcement_unit_ids.contains(memory_unit_id.as_str()) {
            continue;
        }
        let resolved = if validate_hypothesis_storage_id("memory_unit_id", memory_unit_id).is_ok() {
            resolve(memory_unit_id)
        } else {
            None
        };
        add_evidence_origin(
            candidate,
            memory_unit_id,
            resolved,
            HypothesisEvidenceOrigin {
                kind: HypothesisEvidenceOriginKind::Candidate,
                origin_id: Some(candidate.candidate_id.clone()),
                created_at: candidate.created_at.clone(),
                summary: candidate.evidence_summary.clone(),
                strength: candidate.confidence,
                tags: candidate.tags.clone(),
            },
            &mut links,
        );
    }

    for reinforcement in &candidate.reinforcements {
        for memory_unit_id in &reinforcement.source_memory_unit_ids {
            let resolved =
                if validate_hypothesis_storage_id("memory_unit_id", memory_unit_id).is_ok() {
                    resolve(memory_unit_id)
                } else {
                    None
                };
            add_evidence_origin(
                candidate,
                memory_unit_id,
                resolved,
                HypothesisEvidenceOrigin {
                    kind: HypothesisEvidenceOriginKind::Reinforcement,
                    origin_id: Some(reinforcement.reinforcement_id.clone()),
                    created_at: reinforcement.created_at.clone(),
                    summary: reinforcement.evidence_summary.clone(),
                    strength: reinforcement.confidence,
                    tags: reinforcement.tags.clone(),
                },
                &mut links,
            );
        }
    }

    let mut links: Vec<_> = links.into_values().collect();
    links.sort_by(|left, right| left.evidence_link_id.cmp(&right.evidence_link_id));
    links
}

fn add_evidence_origin(
    candidate: &CandidateBelief,
    memory_unit_id: &str,
    resolved: Option<ResolvedHypothesisEvidenceSource>,
    origin: HypothesisEvidenceOrigin,
    links: &mut HashMap<String, HypothesisEvidenceLink>,
) {
    let evidence_link_id = deterministic_evidence_link_id(
        &candidate.candidate_id,
        memory_unit_id,
        HypothesisEvidencePolarity::Supports,
    );
    let created_at = origin.created_at.clone();
    let link = links
        .entry(evidence_link_id.clone())
        .or_insert_with(|| HypothesisEvidenceLink {
            schema_version: HYPOTHESIS_EVIDENCE_LINK_SCHEMA_VERSION.to_string(),
            evidence_link_id,
            hypothesis_id: candidate.candidate_id.clone(),
            memory_unit_id: memory_unit_id.to_string(),
            archive_id: resolved.as_ref().map(|source| source.archive_id.clone()),
            source_session_id: resolved
                .as_ref()
                .map(|source| source.source_session_id.clone()),
            polarity: HypothesisEvidencePolarity::Supports,
            directness: HypothesisEvidenceDirectness::LegacyUnspecified,
            generalization_distance: None,
            resolution: if resolved.is_some() {
                HypothesisEvidenceResolution::Resolved
            } else {
                HypothesisEvidenceResolution::Unresolved
            },
            created_at,
            updated_at: candidate.updated_at.clone(),
            origins: Vec::new(),
        });
    if !link
        .origins
        .iter()
        .any(|existing| existing.kind == origin.kind && existing.origin_id == origin.origin_id)
    {
        link.origins.push(origin);
        link.origins.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.origin_id.cmp(&right.origin_id))
        });
    }
}

impl HypothesisEvidenceLink {
    pub fn validate_basic(&self) -> Result<()> {
        if self.schema_version != HYPOTHESIS_EVIDENCE_LINK_SCHEMA_VERSION {
            return Err(MemoryEngineError::Validation(format!(
                "unsupported hypothesis evidence link schema_version: {}",
                self.schema_version
            )));
        }
        validate_hypothesis_storage_id("evidence_link_id", &self.evidence_link_id)?;
        validate_hypothesis_storage_id("hypothesis_id", &self.hypothesis_id)?;
        validate_hypothesis_storage_id("memory_unit_id", &self.memory_unit_id)?;
        for origin in &self.origins {
            validate_score("hypothesis evidence strength", origin.strength)?;
        }
        Ok(())
    }

    /// Merge a refreshed flat-candidate projection without erasing graph-only
    /// directness or propagation metadata that Phase 3 may add later.
    pub fn merge_existing_graph_fields(&mut self, existing: HypothesisEvidenceLink) {
        if existing.directness != HypothesisEvidenceDirectness::LegacyUnspecified {
            self.directness = existing.directness;
        }
        if existing.generalization_distance.is_some() {
            self.generalization_distance = existing.generalization_distance;
        }
        if self.resolution == HypothesisEvidenceResolution::Unresolved
            && existing.resolution == HypothesisEvidenceResolution::Resolved
        {
            self.resolution = existing.resolution;
            self.archive_id = existing.archive_id.clone();
            self.source_session_id = existing.source_session_id.clone();
        }
        self.created_at = self.created_at.clone().min(existing.created_at);
        for origin in existing.origins {
            if !self.origins.iter().any(|candidate| {
                candidate.kind == origin.kind && candidate.origin_id == origin.origin_id
            }) {
                self.origins.push(origin);
            }
        }
        self.origins.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.origin_id.cmp(&right.origin_id))
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisRelationKind {
    Generalizes,
    Specializes,
    PossibleEquivalent,
    Contradicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisRelationStatus {
    Proposed,
    Confirmed,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisRelation {
    pub schema_version: String,
    pub relation_id: Id,
    pub source_hypothesis_id: Id,
    pub target_hypothesis_id: Id,
    pub kind: HypothesisRelationKind,
    pub status: HypothesisRelationStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub explanation: String,
    pub proposed_by: String,
}

impl HypothesisRelation {
    pub fn validate_basic(&self) -> Result<()> {
        if self.schema_version != HYPOTHESIS_RELATION_SCHEMA_VERSION {
            return Err(MemoryEngineError::Validation(format!(
                "unsupported hypothesis relation schema_version: {}",
                self.schema_version
            )));
        }
        validate_hypothesis_storage_id("relation_id", &self.relation_id)?;
        validate_hypothesis_storage_id("source_hypothesis_id", &self.source_hypothesis_id)?;
        validate_hypothesis_storage_id("target_hypothesis_id", &self.target_hypothesis_id)?;
        if self.source_hypothesis_id == self.target_hypothesis_id {
            return Err(MemoryEngineError::Validation(
                "hypothesis relation cannot point to the same node".to_string(),
            ));
        }
        if let Some(score) = self.score {
            validate_score("hypothesis relation score", score)?;
        }
        if self.explanation.trim().is_empty() || self.proposed_by.trim().is_empty() {
            return Err(MemoryEngineError::Validation(
                "hypothesis relation explanation and proposed_by must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisGraphExportRequest {
    pub schema_version: String,
    /// `None` means only the isolated unscoped namespace, never all scopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisGraphExport {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<HypothesisNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_links: Vec<HypothesisEvidenceLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<HypothesisRelation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl HypothesisGraphExport {
    pub fn new(core_scope: Option<String>) -> Self {
        Self {
            schema_version: HYPOTHESIS_GRAPH_EXPORT_SCHEMA_VERSION.to_string(),
            core_scope,
            nodes: Vec::new(),
            evidence_links: Vec::new(),
            relations: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisGraphMigrationResult {
    pub schema_version: String,
    pub migrated_at: Timestamp,
    pub source_candidate_count: usize,
    pub nodes_created: usize,
    pub nodes_updated: usize,
    pub nodes_unchanged: usize,
    pub evidence_links_created: usize,
    pub evidence_links_updated: usize,
    pub evidence_links_unchanged: usize,
    /// Phase 2 migration never invents semantic relations from flat text.
    pub relations_created: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_memory_unit_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl HypothesisGraphMigrationResult {
    pub fn empty(migrated_at: Timestamp) -> Self {
        Self {
            schema_version: HYPOTHESIS_GRAPH_MIGRATION_RESULT_SCHEMA_VERSION.to_string(),
            migrated_at,
            source_candidate_count: 0,
            nodes_created: 0,
            nodes_updated: 0,
            nodes_unchanged: 0,
            evidence_links_created: 0,
            evidence_links_updated: 0,
            evidence_links_unchanged: 0,
            relations_created: 0,
            unresolved_memory_unit_ids: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub fn deterministic_evidence_link_id(
    hypothesis_id: &str,
    memory_unit_id: &str,
    polarity: HypothesisEvidencePolarity,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(hypothesis_id.as_bytes());
    hasher.update([0]);
    hasher.update(memory_unit_id.as_bytes());
    hasher.update([0]);
    hasher.update(match polarity {
        HypothesisEvidencePolarity::Supports => b"supports".as_slice(),
        HypothesisEvidencePolarity::Contradicts => b"contradicts".as_slice(),
    });
    let digest = format!("{:x}", hasher.finalize());
    format!("hypothesis_evidence_{}", &digest[..24])
}

fn validate_score(label: &str, value: f64) -> Result<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(MemoryEngineError::Validation(format!(
            "{label} must be finite and between 0.0 and 1.0"
        )))
    }
}

pub(crate) fn validate_hypothesis_storage_id(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
        || value.contains("..")
    {
        Err(MemoryEngineError::Validation(format!(
            "invalid {label} path component: {value}"
        )))
    } else {
        Ok(())
    }
}
