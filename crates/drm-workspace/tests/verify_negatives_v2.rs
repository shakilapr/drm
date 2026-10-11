//! Verify negatives v2 suite (work-plan M09W3-E2-T2/T3; verify-engine.md
//! §Catalogue): more than one negative fixture for the `V-RT` (framing overflow,
//! ambiguous route, no route), `V-REL` (reliable over a best-effort datagram and
//! over serial), `V-MEM` (budget, lockable, and pool-invariant violations), and
//! `V-CMP` (a `reject` topic without a required sequential consumer) checks.
//! Every negative is rejected with its check ID and a REQ-cited message; the
//! valid equivalents pass. `V-RT` consumes the single route-derivation output
//! (`topology::derive`), so derivation errors (ambiguity, no route) surface as
//! `V-RT` facts (route integration, E2-T3). REQ: REQ-TOOL-057, REQ-TOOL-058,
//! REQ-TOOL-061, REQ-TOOL-062.

use std::collections::BTreeMap;
use std::fmt::Debug;

use drm_workspace::budget::{ChannelCost, ComputeCost, DeploymentCost, MemoryBudget};
use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build};
use drm_workspace::verify::engine::{
    CheckId, ConsumerDecl, DeploymentConfig, FactResult, LinkCapability, ReadMode, Reliability,
    TopicDecl, Transport, V_CMP, V_MEM, V_REL, V_RT, VerifyReport, verify,
};

/// Fixture helper that fails loudly on an unexpected build error.
fn fixture<T, E: Debug>(result: Result<T, E>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// A node manifest with the given publish/consume interface. REQ: REQ-TOOL-047.
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
/// consumes `pkg/reject_topic`; `sub_node` is the mirror. `pkg/reject_topic`
/// carries `@overflow: reject`. REQ: REQ-TOOL-047.
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

/// A valid deployment baseline: a reliable-capable stream `l1` and a required
/// sequential consumer for the `reject` topic. Individual negatives mutate a
/// clone. REQ: REQ-TOOL-058.
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

/// A UDP link with `max` datagram payload bytes. REQ: REQ-TOOL-062.
fn udp(max: u64) -> LinkCapability {
    LinkCapability {
        transport: Transport::Udp,
        max_datagram_payload: Some(max),
        capacity_bps: 1_000_000,
    }
}

/// A minimal compute cost carrying one channel. REQ: REQ-TOOL-057.
fn compute(budget: MemoryBudget, channel: ChannelCost) -> BTreeMap<String, ComputeCost> {
    let mut computes = BTreeMap::new();
    computes.insert(
        "c1".to_string(),
        ComputeCost {
            budget,
            channels: vec![channel],
            router_buffers: 0,
            spatial_index_bytes: 0,
        },
    );
    computes
}

/// A permissive memory budget (nothing exceeds it). REQ: REQ-TOOL-057.
fn generous_budget() -> MemoryBudget {
    MemoryBudget {
        drm_budget_bytes: 1 << 40,
        lockable_budget_bytes: 1 << 40,
        physical_ram_bytes: 1 << 40,
        precommit: false,
        page_granularity: 4096,
    }
}

/// One channel over `pkg/front`. REQ: REQ-TOOL-057.
fn channel(pool_slots: u64, history_depth: u64, precommit: bool) -> ChannelCost {
    ChannelCost {
        id: "pkg/front".to_string(),
        pool_slots,
        slot_bytes: 1000,
        control_block: 0,
        precommit,
        history_depth,
        borrowers: 0,
        nominal_bytes: 0,
    }
}

/// `V-RT` framing negative: an encoded frame exceeds the datagram bound.
/// REQ: REQ-TOOL-062.
fn neg_rt_framing() -> VerifyReport {
    let mut deploy = config();
    deploy.links.insert("l1".to_string(), udp(0));
    verify(&workspace(), &deploy)
}

/// `V-RT` route negative: a diamond topology leaves two viable paths with no
/// policy ⇒ derivation emits `Ambiguous`, surfaced as a `V-RT` fact (route
/// integration, E2-T3). REQ: REQ-TOOL-062, REQ-ROUTE-003.
fn neg_rt_ambiguous() -> VerifyReport {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![("front".to_string(), b"v: u32\n".to_vec())],
        }],
        nodes: vec![
            (
                "pub_node".to_string(),
                node("pub_node", &["pkg/front"], &[]).into_bytes(),
            ),
            (
                "sub_node".to_string(),
                node("sub_node", &[], &["pkg/front"]).into_bytes(),
            ),
        ],
        topology: Some(
            b"[computes]\na = true\nb = true\nc = true\nd = true\n\n[[links]]\nid = \"L1\"\na = \"a\"\nb = \"b\"\n\n[[links]]\nid = \"L2\"\na = \"a\"\nb = \"c\"\n\n[[links]]\nid = \"L3\"\na = \"b\"\nb = \"d\"\n\n[[links]]\nid = \"L4\"\na = \"c\"\nb = \"d\"\n\n[placement]\npub_node = \"a\"\nsub_node = \"d\"\n"
                .to_vec(),
        ),
    };
    let model = fixture(build(&tree), "diamond");
    let mut deploy = DeploymentConfig::default();
    for link in ["L1", "L2", "L3", "L4"] {
        deploy.links.insert(
            link.to_string(),
            LinkCapability {
                transport: Transport::Tcp,
                max_datagram_payload: None,
                capacity_bps: 1_000_000,
            },
        );
    }
    verify(&model, &deploy)
}

/// `V-RT` route negative: two disconnected compute islands ⇒ derivation emits
/// `NoRoute`, surfaced as a `V-RT` fact. REQ: REQ-TOOL-062, REQ-ROUTE-005.
fn neg_rt_no_route() -> VerifyReport {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![("front".to_string(), b"v: u32\n".to_vec())],
        }],
        nodes: vec![
            (
                "pub_node".to_string(),
                node("pub_node", &["pkg/front"], &[]).into_bytes(),
            ),
            (
                "sub_node".to_string(),
                node("sub_node", &[], &["pkg/front"]).into_bytes(),
            ),
        ],
        topology: Some(
            b"[computes]\na = true\nb = true\nc = true\nd = true\n\n[[links]]\nid = \"L1\"\na = \"a\"\nb = \"b\"\n\n[[links]]\nid = \"L2\"\na = \"c\"\nb = \"d\"\n\n[placement]\npub_node = \"a\"\nsub_node = \"c\"\n"
                .to_vec(),
        ),
    };
    let model = fixture(build(&tree), "islands");
    let mut deploy = DeploymentConfig::default();
    for link in ["L1", "L2"] {
        deploy.links.insert(
            link.to_string(),
            LinkCapability {
                transport: Transport::Tcp,
                max_datagram_payload: None,
                capacity_bps: 1_000_000,
            },
        );
    }
    verify(&model, &deploy)
}

/// `V-REL` negative: `reliable` routed over a best-effort UDP link.
/// REQ: REQ-TOOL-061.
fn neg_rel_udp() -> VerifyReport {
    let mut deploy = config();
    deploy.links.insert("l1".to_string(), udp(1_000));
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

/// `V-REL` negative: `reliable` routed over a best-effort serial link.
/// REQ: REQ-TOOL-061.
fn neg_rel_serial() -> VerifyReport {
    let mut deploy = config();
    deploy.links.insert(
        "l1".to_string(),
        LinkCapability {
            transport: Transport::Serial,
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

/// `V-MEM` negative: worst case exceeds the DRM budget (I-MB2).
/// REQ: REQ-TOOL-057.
fn neg_mem_budget() -> VerifyReport {
    let mut deploy = config();
    deploy.budgets = DeploymentCost {
        computes: compute(
            MemoryBudget {
                drm_budget_bytes: 100,
                ..generous_budget()
            },
            channel(2, 1, false),
        ),
    };
    verify(&workspace(), &deploy)
}

/// `V-MEM` negative: the page-rounded locked span exceeds the lockable budget
/// (I-MB2). REQ: REQ-TOOL-057.
fn neg_mem_lockable() -> VerifyReport {
    let mut deploy = config();
    deploy.budgets = DeploymentCost {
        computes: compute(
            MemoryBudget {
                lockable_budget_bytes: 100,
                ..generous_budget()
            },
            channel(2, 1, true),
        ),
    };
    verify(&workspace(), &deploy)
}

/// `V-MEM` negative: `pool_slots < R + 1` violates the payload-pool floor
/// (I-MB4). REQ: REQ-TOOL-057.
fn neg_mem_pool() -> VerifyReport {
    let mut deploy = config();
    deploy.budgets = DeploymentCost {
        computes: compute(generous_budget(), channel(2, 4, false)),
    };
    verify(&workspace(), &deploy)
}

/// `V-CMP` negative: a `reject` topic with no required sequential consumer.
/// REQ: REQ-TOOL-058.
fn neg_cmp_reject() -> VerifyReport {
    let mut deploy = config();
    deploy
        .topics
        .insert("pkg/reject_topic".to_string(), TopicDecl::default());
    verify(&workspace(), &deploy)
}

/// Every v2 negative with its check ID. REQ: REQ-TOOL-057, REQ-TOOL-058,
/// REQ-TOOL-061, REQ-TOOL-062.
fn all_negatives() -> Vec<(CheckId, VerifyReport)> {
    vec![
        (V_RT, neg_rt_framing()),
        (V_RT, neg_rt_ambiguous()),
        (V_RT, neg_rt_no_route()),
        (V_REL, neg_rel_udp()),
        (V_REL, neg_rel_serial()),
        (V_MEM, neg_mem_budget()),
        (V_MEM, neg_mem_lockable()),
        (V_MEM, neg_mem_pool()),
        (V_CMP, neg_cmp_reject()),
    ]
}

/// REQ: REQ-TOOL-062 — `V-RT` rejects every framing/route negative with its
/// check ID and a REQ-cited message.
#[test]
fn verify_negatives_v2_route() {
    for report in [neg_rt_framing(), neg_rt_ambiguous(), neg_rt_no_route()] {
        assert!(has_fail(&report, V_RT), "V-RT must reject: {report:?}");
        assert_failures_cited(&report);
    }
}

/// REQ: REQ-TOOL-062, REQ-ROUTE-018 — the `V-RT` datagram-fit rejection
/// (E2-T1 route-fit e2e) is actionable: it names the oversized topic and points
/// the author at a stream transport, the build-time reject REQ-ROUTE-018
/// requires for oversize datagram payloads.
#[test]
fn verify_negatives_v2_route_fit_actionable() {
    let report = neg_rt_framing();
    let failures = report.failures();
    let failure = failures
        .iter()
        .find(|e| e.check == V_RT)
        .unwrap_or_else(|| unreachable!("V-RT rejects oversize: {report:?}"));
    assert!(
        failure.message.contains("pkg/front")
            && failure.message.contains("stream transport")
            && failure.message.contains("REQ-ROUTE-018"),
        "actionable stream-transport hint, topic named, REQ cited: {}",
        failure.message
    );
}

/// REQ: REQ-TOOL-061 — `V-REL` rejects `reliable` on both best-effort
/// transports (UDP and serial).
#[test]
fn verify_negatives_v2_reliability() {
    for report in [neg_rel_udp(), neg_rel_serial()] {
        assert!(has_fail(&report, V_REL), "V-REL must reject: {report:?}");
        assert_failures_cited(&report);
    }
}

/// REQ: REQ-TOOL-057 — `V-MEM` rejects the budget, lockable, and pool-invariant
/// violations independently.
#[test]
fn verify_negatives_v2_memory() {
    for report in [neg_mem_budget(), neg_mem_lockable(), neg_mem_pool()] {
        assert!(has_fail(&report, V_MEM), "V-MEM must reject: {report:?}");
        assert_failures_cited(&report);
    }
}

/// REQ: REQ-TOOL-058 — `V-CMP` rejects a `reject` topic with no required
/// sequential consumer.
#[test]
fn verify_negatives_v2_compatibility() {
    let report = neg_cmp_reject();
    assert!(has_fail(&report, V_CMP), "V-CMP must reject: {report:?}");
    assert_failures_cited(&report);
}

/// REQ: REQ-TOOL-057 — every v2 negative cites its check ID and a REQ ID
/// (I-V3); the valid equivalents pass.
#[test]
fn verify_negatives_v2_cited_and_valid_pass() {
    for (check, report) in all_negatives() {
        assert!(has_fail(&report, check), "negative missing {check}");
        assert_failures_cited(&report);
    }
    let valid = verify(&workspace(), &config());
    assert!(valid.passes(), "valid equivalent: {:?}", valid.failures());
}

/// REQ: REQ-TOOL-062 — route integration (E2-T3): `V-RT` consumes the single
/// `topology::derive` output, so an ambiguous route is reported as a `V-RT`
/// fact naming the topic, and the rest of the catalogue still runs.
#[test]
fn verify_route_integration_single_source() {
    let report = neg_rt_ambiguous();
    assert!(has_fail(&report, V_RT));
    let failures = report.failures();
    let ambiguous = failures
        .iter()
        .find(|e| e.check == V_RT && e.message.contains("ambiguous route"))
        .unwrap_or_else(|| unreachable!("ambiguous route surfaces as V-RT: {report:?}"));
    assert!(
        ambiguous.message.contains("pkg/front") && ambiguous.message.contains("REQ-TOOL-062"),
        "names topic + cites REQ: {}",
        ambiguous.message
    );
    // I-V5: the remaining checks still contribute (a pass for V-CMP here).
    assert!(
        report
            .facts
            .iter()
            .any(|f| matches!(f, FactResult::Pass(V_CMP)))
    );
}

/// REQ: REQ-TOOL-057 — I-V5: independent `V-RT`/`V-REL`/`V-MEM`/`V-CMP`
/// failures are all collected in one run.
#[test]
fn verify_negatives_v2_no_shortcircuit() {
    let mut deploy = config();
    deploy.links.insert("l1".to_string(), udp(0));
    deploy.topics.insert(
        "pkg/front".to_string(),
        TopicDecl {
            reliability: Reliability::Reliable,
            expected_period_ns: None,
            consumers: Vec::new(),
        },
    );
    deploy
        .topics
        .insert("pkg/reject_topic".to_string(), TopicDecl::default());
    deploy.budgets = DeploymentCost {
        computes: compute(
            MemoryBudget {
                drm_budget_bytes: 100,
                ..generous_budget()
            },
            channel(2, 1, false),
        ),
    };
    let report = verify(&workspace(), &deploy);
    for check in [V_RT, V_REL, V_MEM, V_CMP] {
        assert!(has_fail(&report, check), "collected {check}: {report:?}");
    }
}
