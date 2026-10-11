//! Verify-engine integration suite (work-plan M09W2-E2-T1, M09W4-E2-T1;
//! verify-engine.md §Test Matrix): the complete deterministic catalogue —
//! `V-SCH`, `V-SW`, `V-PLC`, `V-RT`, `V-REL`, `V-MEM`, `V-CMP`, `V-ID`,
//! `V-SPAT`, `V-RPC`, `V-REC` — with one negative fixture per check, positive
//! full-workspace fixture, determinism, message citations, estimate labeling,
//! no-short-circuit collection, and independent-deadline semantics.
//! REQ: REQ-TOOL-057, REQ-TOOL-058, REQ-TOOL-059, REQ-TOOL-060,
//! REQ-TOOL-061, REQ-TOOL-062, REQ-TOOL-063, REQ-TOOL-064, REQ-SPATIAL-030,
//! REQ-SPATIAL-018, REQ-SPATIAL-037, REQ-OPS-024, REQ-OPS-008.

use std::collections::{BTreeMap, BTreeSet};

use drm_workspace::budget::{ChannelCost, ComputeCost, DeploymentCost, MemoryBudget};
use drm_workspace::model::handles::HandleTable;
use drm_workspace::model::{
    ComputeEntry, NodeEntry, PackageInput, TreeInput, WorkspaceModel, build,
};
use drm_workspace::verify::engine::{
    ALL_CHECKS, CheckId, ConsumerDecl, DeploymentConfig, FactResult, HistoryDecl, LinkCapability,
    ReadMode, RecorderDecl, RecorderFull, Reliability, RpcDecl, SpatialEdgeDecl, SpatialIndexDecl,
    TopicDecl, Transport, V_CMP, V_ID, V_MEM, V_PLC, V_REC, V_REL, V_RPC, V_RT, V_SCH, V_SPAT,
    V_SW, VerifyReport, verify,
};

/// Fixture helper that fails loudly on an unexpected build error.
fn fixture<T, E: std::fmt::Debug>(result: Result<T, E>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// A node manifest with the given publish/consume interface.
fn node(name: &str, publishes: &[&str], consumes: &[&str]) -> String {
    let list = |v: &[&str]| {
        v.iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "name = \"{name}\"\npublishes = [{}]\nconsumes = [{}]\n",
        list(publishes),
        list(consumes)
    )
}

/// Two computes `c1`/`c2`, one link `l1`, with `pub_node` on `c1` and
/// `sub_node` on `c2`. REQ: REQ-TOOL-060.
const TOPOLOGY: &str = "[computes]\nc1 = true\nc2 = true\n\n[[links]]\nid = \"l1\"\na = \"c1\"\nb = \"c2\"\n\n[placement]\npub_node = \"c1\"\nsub_node = \"c2\"\n";

/// A structurally valid workspace: `pub_node` publishes `pkg/front` and
/// consumes `pkg/reject_topic`; `sub_node` is the mirror. REQ: REQ-TOOL-047.
fn workspace() -> WorkspaceModel {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![
                ("front".to_string(), b"v: u32\n".to_vec()),
                (
                    "reject_topic".to_string(),
                    b"@overflow: reject\nv: u32\n".to_vec(),
                ),
            ],
        }],
        nodes: vec![
            (
                "pub_node".to_string(),
                node("pub_node", &["pkg/front"], &["pkg/reject_topic"]).into_bytes(),
            ),
            (
                "sub_node".to_string(),
                node("sub_node", &["pkg/reject_topic"], &["pkg/front"]).into_bytes(),
            ),
        ],
        topology: Some(TOPOLOGY.as_bytes().to_vec()),
    };
    fixture(build(&tree), "workspace")
}

/// Positive deployment config: a reliable-capable stream link and a required
/// sequential consumer for the `reject` topic. REQ: REQ-TOOL-058.
fn config() -> DeploymentConfig {
    let mut deploy = DeploymentConfig::default();
    deploy.links.insert(
        "l1".to_string(),
        LinkCapability {
            transport: Transport::Tcp,
            max_datagram_payload: None,
            capacity_bps: 1_000_000,
        },
    );
    deploy.topics.insert(
        "pkg/reject_topic".to_string(),
        TopicDecl {
            reliability: Reliability::BestEffort,
            expected_period_ns: None,
            consumers: vec![ConsumerDecl {
                node: "pub_node".to_string(),
                read_mode: ReadMode::Sequential,
                required: true,
                deadline_ns: None,
            }],
        },
    );
    deploy
}

/// True iff `report` contains a failure from `check`.
fn has_fail(report: &VerifyReport, check: CheckId) -> bool {
    report
        .facts
        .iter()
        .any(|f| matches!(f, FactResult::Fail(e) if e.check == check))
}

/// Every failure cites its check id and at least one REQ id (I-V3).
fn assert_failures_cited(report: &VerifyReport) {
    for f in report.failures() {
        assert!(
            f.message.contains(f.check.0),
            "check id missing: {}",
            f.message
        );
        assert!(
            f.message.contains("REQ-"),
            "REQ citation missing: {}",
            f.message
        );
    }
}

/// `V-SCH` negative: a stored schema hash that disagrees with the canonical
/// schema. REQ: REQ-TOOL-063.
fn neg_schema() -> VerifyReport {
    let mut m = workspace();
    if let Some(entry) = m.topics.get_mut("pkg/front") {
        entry.schema_hash = [0u8; 16];
    }
    verify(&m, &config())
}

/// `V-SW` negative: a second publisher of a consumed topic. REQ: REQ-TOOL-064.
fn neg_single_writer() -> VerifyReport {
    let mut m = workspace();
    m.nodes.insert(
        "rogue".to_string(),
        NodeEntry {
            publishes: BTreeSet::from(["pkg/front".to_string()]),
            ..NodeEntry::default()
        },
    );
    verify(&m, &config())
}

/// `V-PLC` negative: a placement naming an undeclared compute.
/// REQ: REQ-TOOL-060.
fn neg_placement() -> VerifyReport {
    let mut m = workspace();
    m.placement
        .insert("ghost".to_string(), "nowhere".to_string());
    verify(&m, &config())
}

/// `V-RT` negative: an encoded frame exceeds a datagram link's bound.
/// REQ: REQ-TOOL-062.
fn neg_route() -> VerifyReport {
    let mut deploy = config();
    deploy.links.insert(
        "l1".to_string(),
        LinkCapability {
            transport: Transport::Udp,
            max_datagram_payload: Some(0),
            capacity_bps: 1_000_000,
        },
    );
    verify(&workspace(), &deploy)
}

/// `V-REL` negative: `reliable` requirement routed over a best-effort link.
/// REQ: REQ-TOOL-061.
fn neg_reliability() -> VerifyReport {
    let mut deploy = config();
    deploy.links.insert(
        "l1".to_string(),
        LinkCapability {
            transport: Transport::Udp,
            max_datagram_payload: Some(1_000),
            capacity_bps: 1_000_000,
        },
    );
    deploy.topics.insert(
        "pkg/front".to_string(),
        TopicDecl {
            reliability: Reliability::Reliable,
            expected_period_ns: None,
            consumers: Vec::new(),
        },
    );
    verify(&workspace(), &deploy)
}

/// `V-MEM` negative: a compute whose worst case exceeds its DRM budget.
/// REQ: REQ-TOOL-057.
fn neg_memory() -> VerifyReport {
    let mut deploy = config();
    let mut computes = BTreeMap::new();
    computes.insert(
        "c1".to_string(),
        ComputeCost {
            budget: MemoryBudget {
                drm_budget_bytes: 100,
                lockable_budget_bytes: 100,
                physical_ram_bytes: 1 << 30,
                precommit: false,
                page_granularity: 4096,
            },
            channels: vec![ChannelCost {
                id: "pkg/front".to_string(),
                pool_slots: 2,
                slot_bytes: 1000,
                control_block: 0,
                precommit: false,
                history_depth: 1,
                borrowers: 0,
                nominal_bytes: 0,
            }],
            router_buffers: 0,
            spatial_index_bytes: 0,
        },
    );
    deploy.budgets = DeploymentCost { computes };
    verify(&workspace(), &deploy)
}

/// `V-CMP` negative: a `reject` topic with no required sequential consumer.
/// REQ: REQ-TOOL-058.
fn neg_compatibility() -> VerifyReport {
    let mut deploy = config();
    deploy
        .topics
        .insert("pkg/reject_topic".to_string(), TopicDecl::default());
    verify(&workspace(), &deploy)
}

/// `V-ID` negative: a node name collides with a compute name.
/// REQ: REQ-TOOL-060.
fn neg_identity() -> VerifyReport {
    let mut m = workspace();
    m.computes.insert("pub_node".to_string(), ComputeEntry);
    verify(&m, &config())
}

/// `V-SPAT` negative: `index.drm` capacity exceeds the memory budget (I-MB9).
/// REQ: REQ-SPATIAL-018.
fn neg_spatial() -> VerifyReport {
    let mut deploy = config();
    deploy.spatial.index = Some(SpatialIndexDecl {
        index_bytes: 1 << 20,
        budget_bytes: 1 << 10,
    });
    verify(&workspace(), &deploy)
}

/// `V-RPC` negative: declared concurrency exceeds the capacity model.
/// REQ: REQ-OPS-021.
fn neg_rpc() -> VerifyReport {
    let mut deploy = config();
    deploy.rpc.push(RpcDecl {
        service: "svc".to_string(),
        request_topic: "pkg/front".to_string(),
        event_topic: "pkg/front".to_string(),
        concurrency: 8,
        capacity: 2,
    });
    verify(&workspace(), &deploy)
}

/// `V-REC` negative: `on_full = backpressure` on a `reject` topic without a
/// declared required sequential recorder. REQ: REQ-OPS-008.
fn neg_recorder() -> VerifyReport {
    let mut deploy = config();
    deploy.recorders.push(RecorderDecl {
        topic: "pkg/reject_topic".to_string(),
        on_full: RecorderFull::Backpressure,
        required_sequential: false,
    });
    verify(&workspace(), &deploy)
}

/// Every check with its negative fixture (I-V1). REQ: REQ-TOOL-057.
fn all_negatives() -> Vec<(CheckId, VerifyReport)> {
    vec![
        (V_SCH, neg_schema()),
        (V_SW, neg_single_writer()),
        (V_PLC, neg_placement()),
        (V_RT, neg_route()),
        (V_REL, neg_reliability()),
        (V_MEM, neg_memory()),
        (V_CMP, neg_compatibility()),
        (V_ID, neg_identity()),
        (V_SPAT, neg_spatial()),
        (V_RPC, neg_rpc()),
        (V_REC, neg_recorder()),
    ]
}

/// REQ: REQ-TOOL-059 — a valid workspace passes every check (each check emits a
/// pass fact; zero failures).
#[test]
fn verify_positive_full_workspace() {
    let report = verify(&workspace(), &config());
    assert!(report.passes(), "failures: {:?}", report.failures());
    for check in ALL_CHECKS {
        assert!(
            report.facts.contains(&FactResult::Pass(check)),
            "missing pass for {check}"
        );
    }
}

/// REQ: REQ-TOOL-057 — I-V1: every check id has ≥ 1 positive + ≥ 1 negative
/// fixture.
#[test]
fn verify_catalogue() {
    let positive = verify(&workspace(), &config());
    for check in ALL_CHECKS {
        assert!(
            positive.facts.contains(&FactResult::Pass(check)),
            "positive fixture missing {check}"
        );
    }
    for (check, report) in all_negatives() {
        assert!(has_fail(&report, check), "negative fixture missing {check}");
    }
}

/// REQ: REQ-TOOL-063 — every negative is rejected with a message citing the
/// check id and a REQ id (I-V3).
#[test]
fn verify_msg_citations() {
    for (_, report) in all_negatives() {
        assert_failures_cited(&report);
    }
}

/// REQ: REQ-TOOL-059 — I-V2: the same workspace yields an identical report
/// across 100 runs.
#[test]
fn verify_determinism() {
    let m = workspace();
    let deploy = config();
    let first = verify(&m, &deploy);
    for _ in 0..100 {
        assert_eq!(verify(&m, &deploy), first);
    }
}

/// REQ: REQ-TOOL-059 — I-V4: estimates are labeled `ESTIMATE`; no wording
/// presents an estimate as a guarantee.
#[test]
fn verify_boundary_labels() {
    let mut deploy = config();
    deploy.topics.insert(
        "pkg/front".to_string(),
        TopicDecl {
            reliability: Reliability::BestEffort,
            expected_period_ns: Some(100_000_000),
            consumers: vec![ConsumerDecl {
                node: "sub_node".to_string(),
                read_mode: ReadMode::Latest,
                required: false,
                deadline_ns: Some(1_000_000),
            }],
        },
    );
    let report = verify(&workspace(), &deploy);
    assert!(
        !report.estimates.is_empty(),
        "tight deadline yields an estimate"
    );
    for estimate in &report.estimates {
        assert!(
            estimate.message.starts_with("ESTIMATE"),
            "estimate not labeled: {}",
            estimate.message
        );
    }
    for text in report
        .failures()
        .iter()
        .map(|e| e.message.as_str())
        .chain(report.warnings.iter().map(|w| w.message.as_str()))
    {
        assert!(
            !text.to_lowercase().contains("guarantee"),
            "guarantee wording: {text}"
        );
    }
}

/// REQ: REQ-TOOL-059 — I-V5: multiple independent failures are all reported,
/// none suppressed.
#[test]
fn verify_no_shortcircuit() {
    let mut m = workspace();
    m.nodes.insert(
        "rogue".to_string(),
        NodeEntry {
            publishes: BTreeSet::from(["pkg/front".to_string()]),
            ..NodeEntry::default()
        },
    );
    m.placement
        .insert("ghost".to_string(), "nowhere".to_string());
    let report = verify(&m, &config());
    assert!(has_fail(&report, V_SW), "single-writer collected");
    assert!(has_fail(&report, V_PLC), "placement collected");
}

/// REQ: REQ-TOOL-058 — I-V6: independent per-consumer deadlines on one topic are
/// valid (no `V-CMP` failure).
#[test]
fn verify_deadlines_independent() {
    let mut deploy = config();
    deploy.topics.insert(
        "pkg/front".to_string(),
        TopicDecl {
            reliability: Reliability::BestEffort,
            expected_period_ns: Some(100_000_000),
            consumers: vec![
                ConsumerDecl {
                    node: "a".to_string(),
                    read_mode: ReadMode::Latest,
                    required: false,
                    deadline_ns: Some(20_000_000),
                },
                ConsumerDecl {
                    node: "b".to_string(),
                    read_mode: ReadMode::Latest,
                    required: false,
                    deadline_ns: Some(500_000_000),
                },
            ],
        },
    );
    let report = verify(&workspace(), &deploy);
    assert!(
        !has_fail(&report, V_CMP),
        "deadlines are independent (I-V6)"
    );
}

/// REQ: REQ-TOOL-060 — a handle assigned to two topics is a `V-ID` conflict.
#[test]
fn verify_handle_conflict() {
    let mut deploy = config();
    let mut handles = HandleTable::default();
    handles.assignments.insert("pkg/front".to_string(), 7);
    handles
        .assignments
        .insert("pkg/reject_topic".to_string(), 7);
    deploy.handles = Some(handles);
    let report = verify(&workspace(), &deploy);
    assert!(has_fail(&report, V_ID));
    assert_failures_cited(&report);
}

/// REQ: REQ-SPATIAL-030 — sub-fixtures: two writers, a write+transform
/// conflict, and an undeclared `[spatial] read` each fail `V-SPAT`.
#[test]
fn verify_spatial_edges() {
    for edges in [
        vec![SpatialEdgeDecl {
            edge: "arm".to_string(),
            writers: 2,
            ..SpatialEdgeDecl::default()
        }],
        vec![SpatialEdgeDecl {
            edge: "arm".to_string(),
            writers: 1,
            transform: true,
            ..SpatialEdgeDecl::default()
        }],
        vec![SpatialEdgeDecl {
            edge: "arm".to_string(),
            reads: 1,
            declared_read: false,
            ..SpatialEdgeDecl::default()
        }],
    ] {
        let mut deploy = config();
        deploy.spatial.edges = edges;
        let report = verify(&workspace(), &deploy);
        assert!(has_fail(&report, V_SPAT), "V-SPAT must reject: {report:?}");
        assert_failures_cited(&report);
    }
}

/// REQ: REQ-SPATIAL-037 — an unsatisfiable history bound fails; a satisfiable
/// one passes.
#[test]
fn verify_spatial_history_bound() {
    let mut bad = config();
    bad.spatial.histories = vec![HistoryDecl {
        edge: "arm".to_string(),
        history_ns: 1_000,
        min_sample_period_ns: 10,
        max_samples: 5,
    }];
    let report = verify(&workspace(), &bad);
    assert!(has_fail(&report, V_SPAT), "{report:?}");
    assert_failures_cited(&report);

    let mut ok = config();
    ok.spatial.histories = vec![HistoryDecl {
        edge: "arm".to_string(),
        history_ns: 1_000,
        min_sample_period_ns: 10,
        max_samples: 102,
    }];
    let report = verify(&workspace(), &ok);
    assert!(!has_fail(&report, V_SPAT), "bound met: {report:?}");
}

/// REQ: REQ-OPS-024 — an RPC topic declared best-effort-only fails `V-RPC`; a
/// reliably-declared one passes.
#[test]
fn verify_rpc_reliability() {
    let mut bad = config();
    bad.topics.insert(
        "pkg/front".to_string(),
        TopicDecl {
            reliability: Reliability::BestEffort,
            ..TopicDecl::default()
        },
    );
    bad.rpc.push(RpcDecl {
        service: "svc".to_string(),
        request_topic: "pkg/front".to_string(),
        event_topic: "pkg/front".to_string(),
        concurrency: 1,
        capacity: 1,
    });
    let report = verify(&workspace(), &bad);
    assert!(has_fail(&report, V_RPC), "{report:?}");
    assert_failures_cited(&report);

    let mut ok = config();
    ok.topics.insert(
        "pkg/front".to_string(),
        TopicDecl {
            reliability: Reliability::Reliable,
            ..TopicDecl::default()
        },
    );
    ok.rpc.push(RpcDecl {
        service: "svc".to_string(),
        request_topic: "pkg/front".to_string(),
        event_topic: "pkg/front".to_string(),
        concurrency: 1,
        capacity: 1,
    });
    let report = verify(&workspace(), &ok);
    assert!(!has_fail(&report, V_RPC), "reliable: {report:?}");
}

/// REQ: REQ-OPS-008 — a required sequential recorder satisfies the
/// backpressure-on-reject rule.
#[test]
fn verify_recorder_required_consumer() {
    let mut ok = config();
    ok.recorders.push(RecorderDecl {
        topic: "pkg/reject_topic".to_string(),
        on_full: RecorderFull::Backpressure,
        required_sequential: true,
    });
    let report = verify(&workspace(), &ok);
    assert!(!has_fail(&report, V_REC), "declared: {report:?}");
}
