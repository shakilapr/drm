//! Route e2e: derived forwarding plan ↔ router filtering (work-plan
//! M10W2-E2-T1; route-derivation.md I-R1/I-R10, router-core.md I-RT1, verify
//! `V-RT`). A workspace is built, its route plan derived, and the plan's egress
//! edges are handed to the router data path: the router's filter set equals the
//! derived egress set, a local-only topic never egresses, and the `V-RT`
//! datagram-fit bound agrees with the router's I-RT3 send-time refusal.
//! REQ: REQ-ROUTE-022, REQ-ROUTE-032, REQ-ROUTE-018, REQ-TOOL-062.

use std::collections::BTreeMap;

use drm_router::data_path::{EgressRoute, Router};
use drm_router::transport::{Reliability, TransportAdapter};
use drm_router::wire::WireHeader;
use drm_types::Status;
use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build};
use drm_workspace::topology::{TopicRoutePlan, derive};
use drm_workspace::verify::engine::{
    DeploymentConfig, FactResult, LinkCapability, Transport, V_RT, VerifyReport, route_graph,
    verify,
};

/// Two computes on one link; `pub_node` publishes both topics, `sub_node`
/// consumes the remote topic, `local_node` consumes the local topic on c1.
/// REQ: REQ-TOOL-062.
const TOPOLOGY: &str = "[computes]\nc1 = true\nc2 = true\n\n[[links]]\nid = \"l1\"\na = \"c1\"\nb = \"c2\"\n\n[placement]\npub_node = \"c1\"\nsub_node = \"c2\"\nlocal_node = \"c1\"\n";

/// A node manifest with the given publish/consume interface.
/// REQ: REQ-TOOL-062.
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

/// A workspace with one remote topic (`pkg/front`) and one same-compute topic
/// (`pkg/local`). REQ: REQ-TOOL-062.
fn workspace() -> WorkspaceModel {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![
                ("front".to_string(), b"v: u32\n".to_vec()),
                ("local".to_string(), b"v: u32\n".to_vec()),
            ],
        }],
        nodes: vec![
            (
                "pub_node".to_string(),
                node("pub_node", &["pkg/front", "pkg/local"], &[]).into_bytes(),
            ),
            (
                "sub_node".to_string(),
                node("sub_node", &[], &["pkg/front"]).into_bytes(),
            ),
            (
                "local_node".to_string(),
                node("local_node", &[], &["pkg/local"]).into_bytes(),
            ),
        ],
        topology: Some(TOPOLOGY.as_bytes().to_vec()),
    };
    match build(&tree) {
        Ok(model) => model,
        Err(e) => unreachable!("workspace fixture: {e:?}"),
    }
}

/// A deployment config with link `l1` as a UDP datagram link bounded by
/// `max_datagram_payload`. REQ: REQ-TOOL-062.
fn udp_config(max_datagram_payload: u64) -> DeploymentConfig {
    let mut deploy = DeploymentConfig::default();
    deploy.links.insert(
        "l1".to_string(),
        LinkCapability {
            transport: Transport::Udp,
            max_datagram_payload: Some(max_datagram_payload),
            capacity_bps: 1_000_000,
        },
    );
    deploy
}

/// Whether `report` contains a `V-RT` failure.
/// REQ: REQ-TOOL-062.
fn has_vrt_failure(report: &VerifyReport) -> bool {
    report
        .facts
        .iter()
        .any(|f| matches!(f, FactResult::Fail(e) if e.check == V_RT))
}

/// A recording adapter: accepted frames are captured, no real socket.
/// REQ: REQ-ROUTE-022.
struct Recording {
    frames: Vec<(WireHeader, Vec<u8>)>,
}

impl TransportAdapter for Recording {
    fn send_frame(&mut self, header: &WireHeader, payload: &[u8]) -> Result<(), Status> {
        self.frames.push((*header, payload.to_vec()));
        Ok(())
    }

    fn recv_frame<'a>(
        &mut self,
        _buf: &'a mut [u8],
    ) -> Result<Option<(WireHeader, &'a [u8])>, Status> {
        Ok(None)
    }

    fn capabilities(&self) -> Reliability {
        Reliability::BestEffort
    }
}

/// Stable `LinkId` → router link-index mapping (deterministic order).
/// REQ: REQ-ROUTE-022.
fn link_indices(model: &WorkspaceModel) -> BTreeMap<String, u32> {
    model
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id.clone(), u32::try_from(i + 1).unwrap_or(1)))
        .collect()
}

/// Converts one derived plan's egress edges into router route bindings.
/// REQ: REQ-ROUTE-022.
fn routes_from_plan(
    plan: &TopicRoutePlan,
    topic: u32,
    links: &BTreeMap<String, u32>,
) -> Vec<EgressRoute> {
    let mut out = Vec::new();
    for rule in plan.forwarding.values() {
        for edge in &rule.egress {
            let link = *links
                .get(&edge.link.0)
                .unwrap_or_else(|| unreachable!("link {}", edge.link.0));
            out.push(EgressRoute {
                topic,
                link,
                wire_channel_id: topic * 100 + link,
                remote_consumers: true,
                max_datagram_payload: 1400,
                max_record_bytes: 4096,
            });
        }
    }
    out
}

/// REQ: REQ-ROUTE-022, REQ-TOOL-062 — the derived plan's egress edges are
/// exactly the router's remote route bindings (I-RT1); a local-only topic has no
/// egress edge and never egresses; `V-RT` passes for a fitting datagram bound.
#[test]
fn route_e2e_plan_matches_router_filter() {
    let model = workspace();
    let graph = route_graph(&model);
    let derived = derive(&graph);
    assert!(
        derived.errors.is_empty(),
        "derivation errors: {:?}",
        derived.errors
    );

    let remote = derived
        .plans
        .get("pkg/front")
        .unwrap_or_else(|| unreachable!("remote plan"));
    assert!(
        remote
            .forwarding
            .values()
            .any(|r| r.egress.iter().any(|e| e.link.0 == "l1")),
        "remote topic egresses l1"
    );

    let local = derived
        .plans
        .get("pkg/local")
        .unwrap_or_else(|| unreachable!("local plan"));
    assert!(
        local.forwarding.values().all(|r| r.egress.is_empty()),
        "local-only topic has no egress edge (I-R1)"
    );

    // Hand the derived egress edges to the router.
    let links = link_indices(&model);
    let mut bindings = routes_from_plan(remote, 1, &links);
    // The local-only topic contributes no binding (I-RT1).
    bindings.extend(routes_from_plan(local, 2, &links));
    assert!(
        bindings.iter().all(|b| b.topic == 1),
        "only the remote topic has route bindings"
    );
    let mut router = Router::new(bindings);
    let mut wire = Recording { frames: Vec::new() };

    // The router egresses the remote topic and filters the local-only one.
    assert_eq!(
        router.egress_udp(1, 1, 1, 0, &[0xAB; 4], &mut wire),
        Ok(true)
    );
    assert_eq!(
        router.egress_udp(1, 2, 1, 0, &[0xCD; 4], &mut wire),
        Ok(false),
        "local-only topic never egresses (I-RT1)"
    );
    assert_eq!(wire.frames.len(), 1);

    // V-RT agrees: a generous datagram bound admits the route.
    let report = verify(&model, &udp_config(1400));
    assert!(
        !has_vrt_failure(&report),
        "V-RT passes for a fitting datagram bound"
    );
}

/// REQ: REQ-ROUTE-018, REQ-TOOL-062 — `V-RT` rejects a topic whose canon payload
/// exceeds the datagram bound, and the router refuses the same oversize frame at
/// send time (I-RT3): verification and runtime agree on the bound.
#[test]
fn route_e2e_oversize_agrees() {
    let model = workspace();

    // Build-time: a 1-byte datagram bound cannot fit any canon payload.
    let report = verify(&model, &udp_config(1));
    assert!(
        has_vrt_failure(&report),
        "V-RT rejects the oversize datagram route"
    );

    // Send-time: the same bound refuses the encoded frame before any syscall.
    let mut router = Router::new(vec![EgressRoute {
        topic: 1,
        link: 1,
        wire_channel_id: 11,
        remote_consumers: true,
        max_datagram_payload: 1,
        max_record_bytes: 4096,
    }]);
    let mut wire = Recording { frames: Vec::new() };
    assert_eq!(
        router.egress_udp(1, 1, 1, 0, &[0xAB; 4], &mut wire),
        Err(Status::InvalidArg),
        "I-RT3 refuses the oversize datagram"
    );
    assert!(wire.frames.is_empty(), "no frame reaches the wire");
}
