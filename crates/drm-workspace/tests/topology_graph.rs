//! Topology graph + route derivation fixtures (route-derivation.md Test
//! Matrix; work-plan M02W1-E3-T1/T2). REQ: REQ-ROUTE-004, REQ-ROUTE-010,
//! REQ-ROUTE-003, REQ-ROUTE-005.

use drm_workspace::topology::{
    ExternalEndpoint, ForwardingEdge, Link, LinkId, Overrides, Rate, RouteGraph, VerifyError,
    VertexId, derive,
};
use std::collections::BTreeSet;

fn cid(name: &str) -> VertexId {
    VertexId::Compute(name.to_string())
}

fn link(id: &str, a: VertexId, b: VertexId) -> Link {
    Link {
        id: LinkId(id.to_string()),
        a,
        b,
    }
}

fn singleton(topic: &str) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    s.insert(topic.to_string());
    s
}

fn edge(link_id: &str) -> ForwardingEdge {
    ForwardingEdge {
        link: LinkId(link_id.to_string()),
        export_limit: None,
    }
}

/// REQ: REQ-ROUTE-004 — I-R1: local-only consumers never leave the compute.
#[test]
fn route_local_only() {
    let mut g = RouteGraph::default();
    g.computes.insert("A".to_string());
    g.computes.insert("B".to_string());
    g.links.push(link("L1", cid("A"), cid("B")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "A".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));

    let res = derive(&g);
    assert!(res.errors.is_empty(), "errors: {res:?}");
    let plan = res
        .plans
        .get("t")
        .unwrap_or_else(|| unreachable!("plan exists"));
    assert_eq!(plan.source, cid("A"));
    assert_eq!(plan.forwarding.len(), 1, "only the source compute");
    let rule = &plan.forwarding["A"];
    assert!(rule.ingress.is_none(), "source has no ingress");
    assert!(rule.egress.is_empty(), "I-R1: no egress");
    assert!(rule.local_delivery, "local consumer delivered");
}

/// REQ: REQ-ROUTE-004 — I-R2: chain A→B→C derives the auto plan.
#[test]
fn route_unique() {
    let mut g = RouteGraph::default();
    g.computes
        .extend(["A".to_string(), "B".to_string(), "C".to_string()]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link("L2", cid("B"), cid("C")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "C".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));

    let res = derive(&g);
    assert!(res.errors.is_empty(), "errors: {res:?}");
    let plan = &res.plans["t"];
    assert_eq!(
        plan.forwarding["A"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("L1")]
    );
    assert!(
        plan.forwarding["B"]
            .ingress
            .as_ref()
            .is_some_and(|l| l.0 == "L1")
    );
    assert_eq!(
        plan.forwarding["B"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("L2")]
    );
    assert!(
        plan.forwarding["C"]
            .ingress
            .as_ref()
            .is_some_and(|l| l.0 == "L2")
    );
    assert!(plan.forwarding["C"].local_delivery);
    assert!(!plan.forwarding["A"].local_delivery);
}

/// REQ: REQ-ROUTE-003 — I-R3: diamond A→D and A→B→D ⇒ `Ambiguous` naming
/// both paths; no plan emitted.
#[test]
fn route_ambiguous() {
    let mut g = RouteGraph::default();
    g.computes.extend([
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
    ]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link("L2", cid("A"), cid("C")));
    g.links.push(link("L3", cid("B"), cid("D")));
    g.links.push(link("L4", cid("C"), cid("D")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "D".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));

    let res = derive(&g);
    assert!(
        !res.plans.contains_key("t"),
        "ambiguous topic must not emit a plan"
    );
    assert_eq!(res.errors.len(), 1, "errors: {res:?}");
    match &res.errors[0] {
        VerifyError::Ambiguous {
            topic,
            consumer,
            primary,
            alternate,
        } => {
            assert_eq!(topic, "t");
            assert_eq!(*consumer, cid("D"));
            assert!(!primary.is_empty());
            assert!(!alternate.is_empty());
            assert_ne!(primary, alternate, "two distinct paths named");
        }
        other => unreachable!("expected Ambiguous, got {other:?}"),
    }
}

/// REQ: REQ-ROUTE-015 — deny prunes candidates: denying one of the diamond's
/// edges leaves exactly one path ⇒ auto plan (I-R4 pipeline stage).
#[test]
fn route_deny_selects_unique() {
    let mut g = RouteGraph::default();
    g.computes.extend([
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
    ]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link("L2", cid("A"), cid("C")));
    g.links.push(link("L3", cid("B"), cid("D")));
    g.links.push(link("L4", cid("C"), cid("D")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "D".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));
    g.overrides = Overrides {
        deny: {
            let mut d = Overrides::default().deny;
            d.insert("t".to_string(), singleton_lid("L4"));
            d
        },
        ..Overrides::default()
    };

    let res = derive(&g);
    assert!(res.errors.is_empty(), "errors: {res:?}");
    let plan = &res.plans["t"];
    // Forced through B: A egress L1, B ingress L1 egress L3, D ingress L3.
    assert_eq!(
        plan.forwarding["A"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("L1")]
    );
    assert_eq!(
        plan.forwarding["B"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("L3")]
    );
    assert!(
        plan.forwarding["D"]
            .ingress
            .as_ref()
            .is_some_and(|l| l.0 == "L3")
    );
}

/// REQ: REQ-ROUTE-015 — deny removes the only path ⇒ explicit `NoRoute`.
#[test]
fn route_deny_no_route() {
    let mut g = RouteGraph::default();
    g.computes.extend(["A".to_string(), "B".to_string()]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "B".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));
    g.overrides
        .deny
        .insert("t".to_string(), singleton_lid("L1"));

    let res = derive(&g);
    assert!(!res.plans.contains_key("t"), "no plan without a path");
    assert_eq!(res.errors.len(), 1);
    assert!(
        matches!(&res.errors[0], VerifyError::NoRoute { topic, .. } if topic == "t"),
        "errors: {res:?}"
    );
}

/// REQ: REQ-ROUTE-014 — force ∧ deny on the same link ⇒ conflict error (I-R9).
#[test]
fn route_conflicting_override() {
    let mut g = RouteGraph::default();
    g.computes.extend(["A".to_string(), "B".to_string()]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "B".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));
    g.overrides
        .force
        .insert("t".to_string(), singleton_lid("L1"));
    g.overrides
        .deny
        .insert("t".to_string(), singleton_lid("L1"));

    let res = derive(&g);
    assert!(
        res.errors
            .iter()
            .any(|e| matches!(e, VerifyError::ConflictingRouteOverride { topic } if topic == "t")),
        "errors: {res:?}"
    );
}

/// REQ: REQ-ROUTE-010 — I-R10: fan-out tree A→B, B→C, B→D plus a local
/// consumer on B: every reached compute appears exactly once, local and
/// remote both represented.
#[test]
fn route_fanout_local_and_remote() {
    let mut g = RouteGraph::default();
    g.computes.extend([
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
    ]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link("L2", cid("B"), cid("C")));
    g.links.push(link("L3", cid("B"), cid("D")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "B".to_string());
    g.placement.insert("r".to_string(), "C".to_string());
    g.placement.insert("s".to_string(), "D".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));
    g.consumes.insert("r".to_string(), singleton("t"));
    g.consumes.insert("s".to_string(), singleton("t"));

    let res = derive(&g);
    assert!(res.errors.is_empty(), "errors: {res:?}");
    let plan = &res.plans["t"];
    assert_eq!(plan.forwarding.len(), 4, "every reached compute once");
    assert_eq!(
        plan.forwarding["A"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("L1")]
    );
    let b = &plan.forwarding["B"];
    assert!(b.local_delivery, "local + remote both represented");
    assert_eq!(
        b.egress.iter().collect::<Vec<_>>(),
        vec![&edge("L2"), &edge("L3")],
        "deterministic egress order"
    );
    assert!(plan.forwarding["C"].local_delivery);
    assert!(plan.forwarding["D"].local_delivery);
}

/// REQ: REQ-ROUTE-009 — I-R6: external producer is the source; external
/// consumer rides the last compute's egress.
#[test]
fn route_external() {
    let mut g = RouteGraph::default();
    g.computes.extend(["A".to_string(), "B".to_string()]);
    g.links
        .push(link("LE1", VertexId::External("cam".to_string()), cid("A")));
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link(
        "LE2",
        cid("B"),
        VertexId::External("plot".to_string()),
    ));
    g.placement.insert("q".to_string(), "A".to_string());
    g.publishes.insert("q".to_string(), singleton("other"));
    g.external.insert(
        "cam".to_string(),
        ExternalEndpoint {
            link: LinkId("LE1".to_string()),
            produce: singleton("t"),
            consume: BTreeSet::new(),
        },
    );
    g.external.insert(
        "plot".to_string(),
        ExternalEndpoint {
            link: LinkId("LE2".to_string()),
            produce: BTreeSet::new(),
            consume: singleton("t"),
        },
    );

    let res = derive(&g);
    assert!(res.errors.is_empty(), "errors: {res:?}");
    let plan = &res.plans["t"];
    assert_eq!(plan.source, VertexId::External("cam".to_string()));
    // A receives over LE1, forwards on L1; B exports LE2 (external consumer).
    assert!(
        plan.forwarding["A"]
            .ingress
            .as_ref()
            .is_some_and(|l| l.0 == "LE1")
    );
    assert_eq!(
        plan.forwarding["A"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("L1")]
    );
    assert!(
        plan.forwarding["B"]
            .ingress
            .as_ref()
            .is_some_and(|l| l.0 == "L1")
    );
    assert!(
        !plan.forwarding["B"].local_delivery,
        "no local consumer on B"
    );
    assert_eq!(
        plan.forwarding["B"].egress.iter().collect::<Vec<_>>(),
        vec![&edge("LE2")]
    );
    assert!(
        !plan.forwarding.contains_key("cam") && !plan.forwarding.contains_key("plot"),
        "external vertices carry no runtime rules (REQ-ROUTE-043)"
    );
}

/// REQ: REQ-ROUTE-019 — `export_limit` declared per (topic, link) is carried
/// into the plan egress edge (I-R14).
#[test]
fn route_export_limit_carried() {
    let mut g = RouteGraph::default();
    g.computes.extend(["A".to_string(), "B".to_string()]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "B".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));
    g.overrides.export_limit.insert(
        ("t".to_string(), LinkId("L1".to_string())),
        Rate {
            numerator: 500,
            denominator: 1,
        },
    );

    let res = derive(&g);
    assert!(res.errors.is_empty(), "errors: {res:?}");
    let rule = &res.plans["t"].forwarding["A"];
    let e = rule
        .egress
        .iter()
        .next()
        .unwrap_or_else(|| unreachable!("egress edge"));
    assert_eq!(
        e.export_limit,
        Some(Rate {
            numerator: 500,
            denominator: 1
        })
    );
}

/// REQ: REQ-ROUTE-005 — I-R8: derivation collects every failure (single-writer
/// on one topic, ambiguity on another) without aborting; valid topics still
/// derive.
#[test]
fn route_error_collection() {
    let mut g = RouteGraph::default();
    g.computes.extend([
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
    ]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link("L2", cid("A"), cid("C")));
    g.links.push(link("L3", cid("B"), cid("D")));
    g.links.push(link("L4", cid("C"), cid("D")));
    g.placement.insert("p1".to_string(), "A".to_string());
    g.placement.insert("p2".to_string(), "B".to_string());
    g.placement.insert("q".to_string(), "D".to_string());
    // Two producers on `bad`: single-writer violation (I-R12).
    g.publishes.insert("p1".to_string(), singleton("bad"));
    g.publishes.insert("p2".to_string(), singleton("bad"));
    // One producer on `t`: diamond ⇒ ambiguous (I-R3).
    let t_and_bad = {
        let mut s = BTreeSet::new();
        s.insert("bad".to_string());
        s.insert("t".to_string());
        s
    };
    g.publishes.insert("p1".to_string(), t_and_bad);
    g.consumes.insert("q".to_string(), {
        let mut s = BTreeSet::new();
        s.insert("bad".to_string());
        s.insert("t".to_string());
        s
    });

    let res = derive(&g);
    assert_eq!(res.errors.len(), 2, "both failures collected: {res:?}");
    assert!(
        res.errors.iter().any(
            |e| matches!(e, VerifyError::SingleWriterViolation { topic, .. } if topic == "bad")
        )
    );
    assert!(
        res.errors
            .iter()
            .any(|e| matches!(e, VerifyError::Ambiguous { topic, .. } if topic == "t"))
    );
    assert!(!res.plans.contains_key("bad") && !res.plans.contains_key("t"));
}

/// REQ: REQ-ROUTE-006 — I-R5: same graph ⇒ identical results, twice.
#[test]
fn route_determinism() {
    let mut g = RouteGraph::default();
    g.computes
        .extend(["A".to_string(), "B".to_string(), "C".to_string()]);
    g.links.push(link("L1", cid("A"), cid("B")));
    g.links.push(link("L2", cid("B"), cid("C")));
    g.links.push(link("L3", cid("A"), cid("C")));
    g.placement.insert("p".to_string(), "A".to_string());
    g.placement.insert("q".to_string(), "C".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));
    // Diamond ⇒ ambiguous, but classification itself must be deterministic.
    let a = derive(&g);
    let b = derive(&g);
    assert_eq!(a, b, "I-R5 byte-identical plans");
}

/// REQ: REQ-ROUTE-019 — I-R11: large cyclic graph classifies in bounded time
/// (polynomial, no simple-path enumeration).
#[test]
fn route_cyclic_scale() {
    let n = 200;
    let mut g = RouteGraph::default();
    for i in 0..n {
        g.computes.insert(format!("c{i}"));
    }
    // Ring: ci—c(i+1 mod n); one chord c0—c100 creates a second path.
    for i in 0..n {
        let a = format!("c{i}");
        let b = format!("c{}", (i + 1) % n);
        g.links.push(link(&format!("L{i}"), cid(&a), cid(&b)));
    }
    g.links.push(link("chord", cid("c0"), cid("c100")));
    g.placement.insert("p".to_string(), "c0".to_string());
    g.placement.insert("q".to_string(), "c150".to_string());
    g.publishes.insert("p".to_string(), singleton("t"));
    g.consumes.insert("q".to_string(), singleton("t"));

    let res = derive(&g);
    // Ring + chord ⇒ two viable paths ⇒ ambiguous, classified instantly.
    assert!(
        res.errors
            .iter()
            .any(|e| matches!(e, VerifyError::Ambiguous { topic, .. } if topic == "t")),
        "errors: {res:?}"
    );
}

/// Helper: one-`LinkId` set for override maps. REQ: REQ-ROUTE-015.
fn singleton_lid(name: &str) -> BTreeSet<LinkId> {
    let mut s = BTreeSet::new();
    s.insert(LinkId(name.to_string()));
    s
}
