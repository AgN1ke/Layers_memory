use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use memory_engine::archive::{FidelityStatus, MemoryUnit, MemoryUnitStatus};
use memory_engine::core_store::{
    CandidateBelief, CandidateReinforcement, CandidateStatus, PromotionChecks,
};
use memory_engine::hypothesis::{
    HypothesisEvidenceDirectness, HypothesisGraphExportRequest, HypothesisRelation,
    HypothesisRelationKind, HypothesisRelationStatus, HypothesisValidity,
};
use memory_engine::journal::{JournalOperationType, JournalState};
use memory_engine::storage::Storage;
use memory_engine::types::{
    CANDIDATE_BELIEF_SCHEMA_VERSION, HYPOTHESIS_GRAPH_EXPORT_REQUEST_SCHEMA_VERSION,
    MEMORY_UNIT_SCHEMA_VERSION,
};
use memory_engine::{FileStorage, MemoryEngine};

#[test]
fn legacy_candidates_migrate_idempotently_without_inventing_relations() {
    let root = unique_temp_dir("hypothesis_graph_migration");
    let storage = FileStorage::with_host_id(&root, "hypothesis_test");
    storage.ensure_layout().expect("create storage layout");
    storage
        .write_memory_unit(&memory_unit("mu_initial", "archive_initial", "session_a"))
        .expect("write initial unit");
    storage
        .write_memory_unit(&memory_unit(
            "mu_reinforcement",
            "archive_reinforcement",
            "session_b",
        ))
        .expect("write reinforcement unit");

    let candidate = candidate_fixture();
    fs::write(
        root.join("core")
            .join("candidates")
            .join("candidate_space.json"),
        serde_json::to_vec_pretty(&candidate).expect("serialize legacy candidate"),
    )
    .expect("seed legacy candidate without graph files");

    let engine = MemoryEngine::new(storage.clone());
    let first = engine
        .migrate_hypothesis_graph()
        .expect("migrate legacy candidate");
    assert_eq!(first.source_candidate_count, 1);
    assert_eq!(first.nodes_created, 1);
    assert_eq!(first.evidence_links_created, 3);
    assert_eq!(first.relations_created, 0);
    assert_eq!(first.unresolved_memory_unit_ids, vec!["mu_missing"]);

    let export = engine
        .export_hypothesis_graph(export_request(Some("owner_scope")))
        .expect("export owner graph");
    assert_eq!(export.nodes.len(), 1);
    assert_eq!(export.evidence_links.len(), 3);
    assert!(export.relations.is_empty());
    assert_eq!(export.nodes[0].hypothesis_id, "candidate_space");
    assert_eq!(export.nodes[0].abstraction_level, None);
    assert_eq!(export.nodes[0].contradiction_score, None);
    assert!(export
        .evidence_links
        .iter()
        .all(|link| link.directness == HypothesisEvidenceDirectness::LegacyUnspecified));

    let mut native_node = export.nodes[0].clone();
    native_node.hypothesis_id = "hypothesis_native".to_string();
    native_node.legacy_candidate_id = None;
    native_node.statement =
        "A graph-native hypothesis must not be flattened by migration.".to_string();
    native_node.abstraction_level = Some(2);
    storage
        .write_hypothesis_node(&native_node)
        .expect("write graph-native node");

    let second = engine.migrate_hypothesis_graph().expect("repeat migration");
    assert_eq!(second.source_candidate_count, 1);
    assert_eq!(second.nodes_unchanged, 1);
    assert_eq!(second.nodes_created, 0);
    assert_eq!(second.nodes_updated, 0);
    assert_eq!(second.evidence_links_unchanged, 3);
    assert_eq!(second.evidence_links_created, 0);
    assert_eq!(second.evidence_links_updated, 0);
    assert_eq!(second.relations_created, 0);
    let preserved_native = storage
        .read_hypothesis_node("hypothesis_native")
        .expect("read graph-native node after repeat migration");
    assert_eq!(preserved_native.legacy_candidate_id, None);
    assert_eq!(preserved_native.abstraction_level, Some(2));

    let unscoped = engine
        .export_hypothesis_graph(export_request(None))
        .expect("export isolated unscoped graph");
    assert!(unscoped.nodes.is_empty());
    assert!(unscoped.evidence_links.is_empty());

    let mut completed_migration = storage
        .load_journaled_operations()
        .expect("read migration journal")
        .items
        .into_iter()
        .find(|operation| operation.op_type == JournalOperationType::Migration)
        .expect("migration operation exists");
    fs::remove_dir_all(root.join("core").join("hypotheses"))
        .expect("simulate crash before graph files survive");
    completed_migration.state = JournalState::Started;
    completed_migration.completed_at = None;
    storage
        .begin_journaled_operation(&completed_migration)
        .expect("reopen migration journal");
    let restored = MemoryEngine::new(FileStorage::with_host_id(&root, "hypothesis_test"));
    assert_eq!(
        restored
            .recover_pending_memory_operations()
            .expect("recover graph migration"),
        1
    );
    let recovered = restored
        .export_hypothesis_graph(export_request(Some("owner_scope")))
        .expect("export recovered graph");
    assert_eq!(recovered.nodes.len(), 1);
    assert_eq!(recovered.evidence_links.len(), 3);

    fs::remove_dir_all(root).ok();
}

#[test]
fn graph_projection_keeps_v05_candidate_api_and_graph_only_fields() {
    let root = unique_temp_dir("hypothesis_graph_projection");
    let storage = FileStorage::with_host_id(&root, "hypothesis_test");
    storage.ensure_layout().expect("create storage layout");
    storage
        .write_memory_unit(&memory_unit("mu_initial", "archive_initial", "session_a"))
        .expect("write initial unit");

    let mut candidate = candidate_fixture();
    candidate.source_memory_unit_ids = vec!["mu_initial".to_string()];
    candidate.reinforcements.clear();
    storage
        .write_candidate_belief(&candidate)
        .expect("dual-write candidate projection");

    let mut node = storage
        .read_hypothesis_node("candidate_space")
        .expect("read graph node");
    node.abstraction_level = Some(2);
    node.contradiction_score = Some(0.2);
    node.validity = Some(HypothesisValidity {
        valid_from: Some("2026-07-01T00:00:00Z".to_string()),
        valid_to: None,
    });
    storage
        .write_hypothesis_node(&node)
        .expect("write graph-only meaning");

    let mut evidence = storage
        .read_hypothesis_evidence_links()
        .expect("read evidence")
        .items
        .remove(0);
    evidence.directness = HypothesisEvidenceDirectness::Direct;
    evidence.generalization_distance = Some(0);
    storage
        .write_hypothesis_evidence_link(&evidence)
        .expect("write graph-only evidence meaning");

    candidate.confidence = 0.91;
    candidate.updated_at = "2026-08-02T00:00:00Z".to_string();
    storage
        .write_candidate_belief(&candidate)
        .expect("refresh candidate projection");

    let refreshed = storage
        .read_hypothesis_node("candidate_space")
        .expect("read refreshed graph node");
    assert_eq!(refreshed.support_score, 0.91);
    assert_eq!(refreshed.abstraction_level, Some(2));
    assert_eq!(refreshed.contradiction_score, Some(0.2));
    assert!(refreshed.validity.is_some());
    let refreshed_evidence = storage
        .read_hypothesis_evidence_links()
        .expect("read refreshed evidence")
        .items
        .remove(0);
    assert_eq!(
        refreshed_evidence.directness,
        HypothesisEvidenceDirectness::Direct
    );
    assert_eq!(refreshed_evidence.generalization_distance, Some(0));

    fs::remove_file(
        root.join("core")
            .join("candidates")
            .join("candidate_space.json"),
    )
    .expect("remove legacy compatibility file");
    let projected = storage
        .read_candidate_belief("candidate_space")
        .expect("project graph node as v0.5 candidate");
    assert_eq!(projected.candidate_id, candidate.candidate_id);
    assert_eq!(projected.text, candidate.text);
    assert_eq!(projected.confidence, 0.91);
    assert_eq!(projected.source_memory_unit_ids, vec!["mu_initial"]);

    fs::remove_dir_all(root).ok();
}

#[test]
fn graph_storage_rejects_cross_scope_relations() {
    let root = unique_temp_dir("hypothesis_graph_scope_relations");
    let storage = FileStorage::with_host_id(&root, "hypothesis_test");
    storage.ensure_layout().expect("create storage layout");
    let first = memory_engine::hypothesis::HypothesisNode::from_candidate(&candidate_fixture());
    storage
        .write_hypothesis_node(&first)
        .expect("write first hypothesis");
    let mut second = first.clone();
    second.hypothesis_id = "candidate_other_scope".to_string();
    second.legacy_candidate_id = Some(second.hypothesis_id.clone());
    second.core_scope = Some("other_scope".to_string());
    storage
        .write_hypothesis_node(&second)
        .expect("write second hypothesis");

    let relation = HypothesisRelation {
        schema_version: "hypothesis_relation.v1".to_string(),
        relation_id: "relation_cross_scope".to_string(),
        source_hypothesis_id: first.hypothesis_id,
        target_hypothesis_id: second.hypothesis_id.clone(),
        kind: HypothesisRelationKind::Generalizes,
        status: HypothesisRelationStatus::Proposed,
        created_at: "2026-08-02T00:00:00Z".to_string(),
        updated_at: "2026-08-02T00:00:00Z".to_string(),
        score: None,
        explanation: "A test relation must not cross personalities.".to_string(),
        proposed_by: "test".to_string(),
    };
    let error = storage
        .write_hypothesis_relation(&relation)
        .expect_err("cross-scope relation must fail closed");
    assert!(error
        .to_string()
        .contains("cannot cross personality scopes"));

    second.core_scope = Some("owner_scope".to_string());
    storage
        .write_hypothesis_node(&second)
        .expect("align second hypothesis scope");
    storage
        .write_hypothesis_relation(&relation)
        .expect("write same-scope relation");
    assert_eq!(
        storage
            .read_hypothesis_relations()
            .expect("read same-scope relation")
            .items,
        vec![relation]
    );

    let mut unsafe_node = second;
    unsafe_node.hypothesis_id = "../outside".to_string();
    let unsafe_error = storage
        .write_hypothesis_node(&unsafe_node)
        .expect_err("graph IDs must not escape the storage directory");
    assert!(unsafe_error.to_string().contains("invalid hypothesis_id"));
    let unsafe_read = storage
        .read_hypothesis_node("../manifest")
        .expect_err("graph reads must not escape the storage directory");
    assert!(unsafe_read.to_string().contains("invalid hypothesis_id"));

    fs::remove_dir_all(root).ok();
}

fn export_request(core_scope: Option<&str>) -> HypothesisGraphExportRequest {
    HypothesisGraphExportRequest {
        schema_version: HYPOTHESIS_GRAPH_EXPORT_REQUEST_SCHEMA_VERSION.to_string(),
        core_scope: core_scope.map(str::to_string),
    }
}

fn memory_unit(memory_unit_id: &str, archive_id: &str, session_id: &str) -> MemoryUnit {
    MemoryUnit {
        schema_version: MEMORY_UNIT_SCHEMA_VERSION.to_string(),
        memory_unit_id: memory_unit_id.to_string(),
        archive_id: archive_id.to_string(),
        source_session_id: session_id.to_string(),
        created_at: "2026-07-01T10:00:00Z".to_string(),
        updated_at: "2026-07-01T10:00:00Z".to_string(),
        thesis: format!("Evidence from {memory_unit_id}"),
        source_event_ids: vec![format!("event_{memory_unit_id}")],
        evidence: Some("Direct user statement.".to_string()),
        tags: vec!["space".to_string()],
        weight: 0.9,
        status: MemoryUnitStatus::CoreCandidate,
        fidelity_status: FidelityStatus::Valid,
        fidelity_review: None,
        forget_review: None,
        reflection_count: 1,
        last_reflected_at: Some("2026-07-01T11:00:00Z".to_string()),
        last_reflection_operation_id: Some("reflection_apply_fixture".to_string()),
    }
}

fn candidate_fixture() -> CandidateBelief {
    CandidateBelief {
        schema_version: CANDIDATE_BELIEF_SCHEMA_VERSION.to_string(),
        candidate_id: "candidate_space".to_string(),
        source_session_id: Some("session_a".to_string()),
        source_session_ids: vec!["session_a".to_string(), "session_b".to_string()],
        core_scope: Some("owner_scope".to_string()),
        created_at: "2026-07-01T12:00:00Z".to_string(),
        updated_at: "2026-07-02T12:00:00Z".to_string(),
        text: "The user has a recurring interest in space exploration.".to_string(),
        category: "interests".to_string(),
        status: CandidateStatus::ReadyForReview,
        confidence: 0.84,
        supporting_archive_ids: vec![
            "archive_initial".to_string(),
            "archive_reinforcement".to_string(),
        ],
        contradicting_archive_ids: Vec::new(),
        contradicted_core_fact_ids: Vec::new(),
        evidence_summary: "Several validated memories support the pattern.".to_string(),
        promotion_checks: PromotionChecks {
            min_sources_met: true,
            weight_threshold_met: true,
            no_recent_contradiction: true,
            manual_review_required: true,
        },
        source_memory_unit_ids: vec![
            "mu_initial".to_string(),
            "mu_reinforcement".to_string(),
            "mu_missing".to_string(),
        ],
        tags: vec!["space".to_string()],
        promoted_core_fact_id: None,
        review: None,
        links: Vec::new(),
        reinforcements: vec![CandidateReinforcement {
            reinforcement_id: "reinforcement_space".to_string(),
            created_at: "2026-07-02T12:00:00Z".to_string(),
            source_session_ids: vec!["session_b".to_string()],
            source_memory_unit_ids: vec!["mu_reinforcement".to_string()],
            supporting_archive_ids: vec!["archive_reinforcement".to_string()],
            evidence_summary: "A later session repeated the interest.".to_string(),
            confidence: 0.84,
            tags: vec!["recurring".to_string()],
        }],
    }
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("memory_engine_{name}_{nanos}"))
}
