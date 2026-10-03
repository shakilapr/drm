//! Route derivation — forwarding-tree start slice (work-plan M02W1-E3-T1/T2):
//! graph types, producer/consumer collection, override conflict validation
//! (I-R9), deny-prune, per-consumer path classification (unique ⇒ auto;
//! ≥2 viable paths without policy ⇒ `Ambiguous`; none ⇒ `NoRoute`), hop-merge
//! under the one-parent invariant (I-R13), rooted-arborescence check.
//! Path analysis is polynomial: BFS path + bounded per-edge alternate-path
//! check ([route-derivation.md](../../docs/algorithms/toolchain/route-derivation.md)
//! I-R11); bridge/2-connectivity is a later O(V+E) fast path with identical
//! classification semantics.
//! REQ: REQ-ROUTE-004, REQ-ROUTE-010, REQ-ROUTE-003, REQ-ROUTE-005.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Graph + plan vertex: compute node or external endpoint (I-R6).
/// REQ: REQ-ROUTE-009.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VertexId {
    /// A compute unit.
    Compute(String),
    /// A third-party device endpoint.
    External(String),
}

/// Link identity (`drm.toml` links table).
/// REQ: REQ-ROUTE-010.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkId(
    /// Stable link name from the topology manifest.
    pub String,
);

/// Undirected physical link between two vertices.
/// REQ: REQ-ROUTE-010.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Link identity.
    pub id: LinkId,
    /// One endpoint.
    pub a: VertexId,
    /// Other endpoint.
    pub b: VertexId,
}

/// Dimensioned shaping limit carried into plan egress edges (I-R14).
/// REQ: REQ-ROUTE-019.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rate {
    /// Numerator.
    pub numerator: u64,
    /// Denominator (≥ 1).
    pub denominator: u64,
}

/// External endpoint: third-party device as a first-class graph vertex
/// (I-R6). REQ: REQ-ROUTE-009.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEndpoint {
    /// The link that connects the device.
    pub link: LinkId,
    /// Topics the device produces.
    pub produce: BTreeSet<String>,
    /// Topics the device consumes.
    pub consume: BTreeSet<String>,
}

/// Explicit per-topic routing overrides.
/// REQ: REQ-ROUTE-017, REQ-ROUTE-015, REQ-ROUTE-016.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Overrides {
    /// Prefer this link among viable paths.
    pub prefer: BTreeMap<String, LinkId>,
    /// Never route over these links.
    pub deny: BTreeMap<String, BTreeSet<LinkId>>,
    /// Pin the route to exactly these links (≥ 2 ⇒ conflict, I-R9).
    pub force: BTreeMap<String, BTreeSet<LinkId>>,
    /// Per-egress shaping limit, carried into plan edges (I-R14).
    pub export_limit: BTreeMap<(String, LinkId), Rate>,
}

/// Declared communication graph + physical topology.
/// REQ: REQ-ROUTE-010.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteGraph {
    /// Compute units.
    pub computes: BTreeSet<String>,
    /// Undirected links (compute↔compute and compute↔external).
    pub links: Vec<Link>,
    /// Node → compute placement.
    pub placement: BTreeMap<String, String>,
    /// Node → published topics.
    pub publishes: BTreeMap<String, BTreeSet<String>>,
    /// Node → consumed topics.
    pub consumes: BTreeMap<String, BTreeSet<String>>,
    /// External endpoints by name.
    pub external: BTreeMap<String, ExternalEndpoint>,
    /// Per-topic overrides.
    pub overrides: Overrides,
}

/// One explicit hop along a selected path (I-R13: plans are built from hops).
/// REQ: REQ-ROUTE-044.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hop {
    /// Hop source vertex.
    pub from: VertexId,
    /// Link traversed.
    pub link: LinkId,
    /// Hop destination vertex.
    pub to: VertexId,
}

/// Per-egress shaping metadata (I-R14). REQ: REQ-ROUTE-019.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ForwardingEdge {
    /// Egress link.
    pub link: LinkId,
    /// Declared export limit, if any.
    pub export_limit: Option<Rate>,
}

/// Runtime forwarding rule for one compute (computs only; REQ-ROUTE-043).
/// REQ: REQ-ROUTE-010.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForwardingRule {
    /// Link the sample arrived on (`None` = source compute).
    pub ingress: Option<LinkId>,
    /// Links to forward on (deterministic order).
    pub egress: BTreeSet<ForwardingEdge>,
    /// Deliver to local consumers on this compute.
    pub local_delivery: bool,
}

/// One topic's forwarding tree (not one route — fan-out + local delivery,
/// I-R10). REQ: REQ-ROUTE-010.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRoutePlan {
    /// Compute origin or external producer (I-R12: exactly one producer).
    pub source: VertexId,
    /// Runtime tables, computes only.
    pub forwarding: BTreeMap<String, ForwardingRule>,
}

/// Derivation error. REQ: REQ-ROUTE-005.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// Contradictory explicit overrides (I-R9).
    ConflictingRouteOverride {
        /// Topic with contradictory overrides.
        topic: String,
    },
    /// Zero or multiple producers (I-R12); no plan emitted.
    SingleWriterViolation {
        /// Topic.
        topic: String,
        /// All producer vertices found.
        producers: BTreeSet<VertexId>,
    },
    /// ≥ 2 viable paths without policy (I-R3); both named.
    Ambiguous {
        /// Topic.
        topic: String,
        /// Consumer vertex.
        consumer: VertexId,
        /// First classified path.
        primary: Vec<Hop>,
        /// Alternate path avoiding one primary edge.
        alternate: Vec<Hop>,
    },
    /// No viable path (deny-pruned or disconnected).
    NoRoute {
        /// Topic.
        topic: String,
        /// Consumer vertex.
        consumer: VertexId,
    },
    /// `force` names a link that no viable path traverses (I-R9).
    ForcedLinkNotViable {
        /// Topic.
        topic: String,
        /// Forced link.
        link: String,
        /// Consumer vertex the force could not serve.
        consumer: VertexId,
    },
    /// A compute gained a second, different parent (I-R13).
    InconsistentFanout {
        /// Topic.
        topic: String,
        /// Compute with two parents.
        compute: String,
    },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingRouteOverride { topic } => {
                write!(f, "conflicting route override for `{topic}`")
            }
            Self::SingleWriterViolation { topic, producers } => {
                write!(f, "single-writer violation on `{topic}`: {producers:?}")
            }
            Self::Ambiguous {
                topic,
                consumer,
                primary,
                alternate,
            } => {
                write!(
                    f,
                    "ambiguous route for `{topic}` to {consumer:?}: {primary:?} vs {alternate:?}"
                )
            }
            Self::NoRoute { topic, consumer } => {
                write!(f, "no route for `{topic}` to {consumer:?}")
            }
            Self::ForcedLinkNotViable {
                topic,
                link,
                consumer,
            } => {
                write!(
                    f,
                    "forced link `{link}` is not on any viable path for `{topic}` to {consumer:?}"
                )
            }
            Self::InconsistentFanout { topic, compute } => {
                write!(f, "inconsistent fan-out for `{topic}` at `{compute}`")
            }
        }
    }
}

/// Derivation output: `{plans, errors}` — never short-circuits (I-R8).
/// REQ: REQ-ROUTE-005.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteResult {
    /// Valid plans, keyed by topic (derivation continues past errors).
    pub plans: BTreeMap<String, TopicRoutePlan>,
    /// All failures, not just the first.
    pub errors: Vec<VerifyError>,
}

type Adj = BTreeMap<VertexId, Vec<(LinkId, VertexId)>>;

/// Path-traverses-link predicate.
/// REQ: REQ-ROUTE-016.
fn contains_link(hops: &[Hop], link: &LinkId) -> bool {
    hops.iter().any(|h| &h.link == link)
}

/// Endpoints of a link from the adjacency map (both orientations present for
/// undirected links; the first occurrence pair wins).
/// REQ: REQ-ROUTE-016.
fn link_endpoints(adj: &Adj, via: &LinkId) -> Option<(VertexId, VertexId)> {
    for (v, nbrs) in adj {
        for (link, w) in nbrs {
            if link == via {
                return Some((v.clone(), w.clone()));
            }
        }
    }
    None
}

/// Viable path that traverses `via`: `src →(denying via)→ u`, the `via`
/// traversal, then `v →(denying via)→ dst`, for each of the two orientations
/// of the undirected link. Polynomial (three BFS per orientation).
/// REQ: REQ-ROUTE-016.
fn path_via(
    adj: &Adj,
    src: &VertexId,
    dst: &VertexId,
    via: &LinkId,
    deny: Option<&BTreeSet<LinkId>>,
) -> Option<Vec<Hop>> {
    let (a, b) = link_endpoints(adj, via)?;
    let mut pruned: BTreeSet<LinkId> = deny.cloned().unwrap_or_default();
    pruned.insert(via.clone());
    for (u, v) in [(&a, &b), (&b, &a)] {
        let (Some(mut hops), Some(tail)) = (
            bfs(adj, src, u, None, Some(&pruned)),
            bfs(adj, v, dst, None, Some(&pruned)),
        ) else {
            continue;
        };
        hops.push(Hop {
            from: u.clone(),
            link: via.clone(),
            to: v.clone(),
        });
        hops.extend(tail);
        return Some(hops);
    }
    None
}

/// Force/prefer selection outcome for one consumer.
/// REQ: REQ-ROUTE-016, REQ-ROUTE-017.
enum Selection {
    /// Exactly one path survives the policy.
    Chosen(Vec<Hop>),
    /// Two or more distinct viable paths remain unresolved.
    Contested(Vec<Hop>, Vec<Hop>),
    /// `force` names a link no viable path traverses.
    NotViable,
}

/// Collects deduplicated viable candidates that traverse `link`.
/// REQ: REQ-ROUTE-016.
fn via_candidates(
    adj: &Adj,
    src: &VertexId,
    dst: &VertexId,
    link: &LinkId,
    deny: Option<&BTreeSet<LinkId>>,
    primary: &[Hop],
    alternate: Option<&Vec<Hop>>,
) -> Vec<Vec<Hop>> {
    let mut cands: Vec<Vec<Hop>> = Vec::new();
    if contains_link(primary, link) {
        cands.push(primary.to_vec());
    }
    if let Some(alt) = alternate
        && contains_link(alt, link)
        && !cands.contains(alt)
    {
        cands.push(alt.clone());
    }
    if let Some(p) = path_via(adj, src, dst, link, deny)
        && !cands.contains(&p)
    {
        cands.push(p);
    }
    cands
}

/// Applies the selection pipeline for one consumer: `force` pins the route
/// (a single viable candidate), `prefer` breaks a tie, `auto` demands a
/// unique path (I-R4). REQ: REQ-ROUTE-016, REQ-ROUTE-017, REQ-ROUTE-003.
#[allow(clippy::too_many_arguments)]
fn select_consumer_path(
    adj: &Adj,
    src: &VertexId,
    dst: &VertexId,
    deny: Option<&BTreeSet<LinkId>>,
    force: Option<&LinkId>,
    prefer: Option<&LinkId>,
    primary: Vec<Hop>,
    alternate: Option<Vec<Hop>>,
) -> Selection {
    if let Some(link) = force {
        let cands = via_candidates(adj, src, dst, link, deny, &primary, alternate.as_ref());
        return match cands.len() {
            0 => Selection::NotViable,
            1 => Selection::Chosen(cands.into_iter().next().unwrap_or_default()),
            _ => Selection::Contested(cands[0].clone(), cands[1].clone()),
        };
    }
    let mut cands: Vec<Vec<Hop>> = vec![primary.clone()];
    if let Some(alt) = &alternate {
        cands.push(alt.clone());
    }
    if cands.len() == 1 {
        return Selection::Chosen(primary);
    }
    if let Some(link) = prefer {
        let via: Vec<Vec<Hop>> = cands
            .iter()
            .filter(|p| contains_link(p, link))
            .cloned()
            .collect();
        if via.len() == 1 {
            return Selection::Chosen(via.into_iter().next().unwrap_or_default());
        }
    }
    Selection::Contested(primary, alternate.unwrap_or_default())
}

/// BFS from `src` to `dst` over non-denied links, optionally skipping one
/// link (alternate-path check). Deterministic: neighbors in link order.
/// REQ: REQ-ROUTE-019.
fn bfs(
    adj: &Adj,
    src: &VertexId,
    dst: &VertexId,
    skip: Option<&LinkId>,
    deny: Option<&BTreeSet<LinkId>>,
) -> Option<Vec<Hop>> {
    let mut parent: BTreeMap<VertexId, (LinkId, VertexId)> = BTreeMap::new();
    let mut seen: BTreeSet<VertexId> = BTreeSet::new();
    let mut queue: VecDeque<VertexId> = VecDeque::new();
    seen.insert(src.clone());
    queue.push_back(src.clone());
    while let Some(v) = queue.pop_front() {
        if &v == dst {
            break;
        }
        if let Some(nbrs) = adj.get(&v) {
            for (link, w) in nbrs {
                if Some(link) == skip {
                    continue;
                }
                if deny.is_some_and(|d| d.contains(link)) {
                    continue;
                }
                if seen.insert(w.clone()) {
                    parent.insert(w.clone(), (link.clone(), v.clone()));
                    queue.push_back(w.clone());
                }
            }
        }
    }
    if !seen.contains(dst) {
        return None;
    }
    let mut hops = Vec::new();
    let mut cur = dst.clone();
    while &cur != src {
        let (link, prev) = parent.get(&cur)?;
        hops.push(Hop {
            from: prev.clone(),
            link: link.clone(),
            to: cur.clone(),
        });
        cur = prev.clone();
    }
    hops.reverse();
    Some(hops)
}

/// Merge selected hops into the plan (I-R13): source computes gain egress
/// (with declared export limit, I-R14), destination computes gain at most one
/// ingress; the ingress link is never re-exported (I-R7). Returns `false`
/// when an inconsistency was reported.
/// REQ: REQ-ROUTE-044, REQ-ROUTE-014.
fn merge_hops(
    topic: &str,
    plan: &mut TopicRoutePlan,
    hops: &[Hop],
    export_limit: &BTreeMap<(String, LinkId), Rate>,
    errors: &mut Vec<VerifyError>,
) -> bool {
    for hop in hops {
        if let VertexId::Compute(c) = &hop.from {
            let rule = plan.forwarding.entry(c.clone()).or_default();
            if rule.ingress.as_ref() == Some(&hop.link) {
                errors.push(VerifyError::InconsistentFanout {
                    topic: topic.to_string(),
                    compute: c.clone(),
                });
                return false;
            }
            rule.egress.insert(ForwardingEdge {
                link: hop.link.clone(),
                export_limit: export_limit
                    .get(&(topic.to_string(), hop.link.clone()))
                    .copied(),
            });
        }
        if let VertexId::Compute(t) = &hop.to {
            let rule = plan.forwarding.entry(t.clone()).or_default();
            match &rule.ingress {
                Some(ing) if ing != &hop.link => {
                    errors.push(VerifyError::InconsistentFanout {
                        topic: topic.to_string(),
                        compute: t.clone(),
                    });
                    return false;
                }
                Some(_) => {}
                None => rule.ingress = Some(hop.link.clone()),
            }
        }
    }
    true
}

/// Build the forwarding tree for one topic: classify every consumer, merge
/// hops under the one-parent invariant, validate the arborescence. Selection
/// follows the pipeline: `deny` prunes, `force` pins, `prefer` breaks ties,
/// `auto` requires a unique path (I-R4).
/// REQ: REQ-ROUTE-004, REQ-ROUTE-044, REQ-ROUTE-015, REQ-ROUTE-016,
/// REQ-ROUTE-017.
#[allow(clippy::too_many_arguments)]
fn build_tree(
    topic: &str,
    src: &VertexId,
    consumers: &BTreeSet<VertexId>,
    deny: Option<&BTreeSet<LinkId>>,
    force: Option<&LinkId>,
    prefer: Option<&LinkId>,
    adj: &Adj,
    export_limit: &BTreeMap<(String, LinkId), Rate>,
    errors: &mut Vec<VerifyError>,
) -> Option<TopicRoutePlan> {
    let mut plan = TopicRoutePlan {
        source: src.clone(),
        forwarding: BTreeMap::new(),
    };
    if let VertexId::Compute(c) = src {
        plan.forwarding.entry(c.clone()).or_default();
    }
    let err_start = errors.len();
    for dst in consumers {
        if dst == src {
            // Same-compute consumer: local delivery only (I-R1).
            if let VertexId::Compute(c) = dst {
                plan.forwarding.entry(c.clone()).or_default().local_delivery = true;
            }
            continue;
        }
        let Some(primary) = bfs(adj, src, dst, None, deny) else {
            errors.push(VerifyError::NoRoute {
                topic: topic.to_string(),
                consumer: dst.clone(),
            });
            continue;
        };
        // Bounded alternate-path check (I-R11): a second path exists iff some
        // primary edge can be avoided; exact and polynomial.
        let alternate = primary
            .iter()
            .find_map(|h| bfs(adj, src, dst, Some(&h.link), deny));
        let selected =
            match select_consumer_path(adj, src, dst, deny, force, prefer, primary, alternate) {
                Selection::Chosen(hops) => hops,
                Selection::Contested(p, a) => {
                    errors.push(VerifyError::Ambiguous {
                        topic: topic.to_string(),
                        consumer: dst.clone(),
                        primary: p,
                        alternate: a,
                    });
                    continue;
                }
                Selection::NotViable => {
                    errors.push(VerifyError::ForcedLinkNotViable {
                        topic: topic.to_string(),
                        link: force.map_or_else(String::new, |l| l.0.clone()),
                        consumer: dst.clone(),
                    });
                    continue;
                }
            };
        if !merge_hops(topic, &mut plan, &selected, export_limit, errors) {
            return None;
        }
        if let VertexId::Compute(c) = dst {
            plan.forwarding.entry(c.clone()).or_default().local_delivery = true;
        }
    }
    // Any consumer-side failure invalidates the topic's plan; other topics
    // still derive (I-R8 collect-not-short-circuit).
    if errors.len() > err_start {
        return None;
    }
    // Rooted arborescence: source indegree 0; reached computes have a parent.
    if let VertexId::Compute(c) = src
        && plan.forwarding.get(c).is_some_and(|r| r.ingress.is_some())
    {
        errors.push(VerifyError::InconsistentFanout {
            topic: topic.to_string(),
            compute: c.clone(),
        });
        return None;
    }
    Some(plan)
}

/// Collects producers and consumers as graph vertices: nodes resolved through
/// their placement plus external endpoints (I-R6).
/// REQ: REQ-ROUTE-009, REQ-ROUTE-010.
fn collect_vertices(
    graph: &RouteGraph,
) -> (
    BTreeMap<String, BTreeSet<VertexId>>,
    BTreeMap<String, BTreeSet<VertexId>>,
) {
    let mut producers: BTreeMap<String, BTreeSet<VertexId>> = BTreeMap::new();
    let mut consumers: BTreeMap<String, BTreeSet<VertexId>> = BTreeMap::new();
    for (topics, key, out) in [
        (&graph.publishes, &graph.placement, &mut producers),
        (&graph.consumes, &graph.placement, &mut consumers),
    ] {
        for (node, set) in topics {
            let Some(compute) = key.get(node) else {
                continue;
            };
            for t in set {
                out.entry(t.clone())
                    .or_default()
                    .insert(VertexId::Compute(compute.clone()));
            }
        }
    }
    for (name, ext) in &graph.external {
        for t in &ext.produce {
            producers
                .entry(t.clone())
                .or_default()
                .insert(VertexId::External(name.clone()));
        }
        for t in &ext.consume {
            consumers
                .entry(t.clone())
                .or_default()
                .insert(VertexId::External(name.clone()));
        }
    }
    (producers, consumers)
}

/// Derive per-topic forwarding plans (route-derivation.md §Functions).
/// REQ: REQ-ROUTE-004, REQ-ROUTE-005, REQ-ROUTE-010.
#[must_use]
pub fn derive(graph: &RouteGraph) -> RouteResult {
    let mut res = RouteResult::default();

    // 1. Validate explicit overrides first (I-R9): contradictory intent is an
    // error, never resolved by precedence. A conflict requires `force`.
    for (topic, forced) in &graph.overrides.force {
        let conflict = forced.len() > 1
            || graph
                .overrides
                .deny
                .get(topic)
                .is_some_and(|d| forced.iter().any(|l| d.contains(l)));
        if conflict {
            res.errors.push(VerifyError::ConflictingRouteOverride {
                topic: topic.clone(),
            });
        }
    }

    // 2. Producers/consumers as graph vertices (nodes ∪ external, I-R6).
    let (producers, consumers) = collect_vertices(graph);

    // 3. Adjacency over all links (deny applied during BFS, I-R4).
    let mut adj: Adj = BTreeMap::new();
    for link in &graph.links {
        adj.entry(link.a.clone())
            .or_default()
            .push((link.id.clone(), link.b.clone()));
        adj.entry(link.b.clone())
            .or_default()
            .push((link.id.clone(), link.a.clone()));
    }

    // 4. Per topic (deterministic sorted order, I-R5).
    for (topic, srcs) in &producers {
        if srcs.len() != 1 {
            res.errors.push(VerifyError::SingleWriterViolation {
                topic: topic.clone(),
                producers: srcs.clone(),
            });
            continue;
        }
        let src = match srcs.iter().next() {
            Some(s) => s.clone(),
            None => continue,
        };
        let Some(consumer_set) = consumers.get(topic) else {
            // No consumers: source-only plan (empty forwarding rules).
            let mut plan = TopicRoutePlan {
                source: src.clone(),
                forwarding: BTreeMap::new(),
            };
            if let VertexId::Compute(c) = &src {
                plan.forwarding.entry(c.clone()).or_default();
            }
            res.plans.insert(topic.clone(), plan);
            continue;
        };
        // Effective single forced link: multi-link or deny-overlapping force
        // is already a ConflictingRouteOverride (I-R9); deny pruning proceeds
        // for that topic without a pin.
        let forced: Option<&LinkId> = graph
            .overrides
            .force
            .get(topic)
            .and_then(|f| f.iter().next())
            .filter(|l| {
                graph
                    .overrides
                    .deny
                    .get(topic)
                    .is_none_or(|d| !d.contains(l))
            });
        if let Some(plan) = build_tree(
            topic,
            &src,
            consumer_set,
            graph.overrides.deny.get(topic),
            forced,
            graph.overrides.prefer.get(topic),
            &adj,
            &graph.overrides.export_limit,
            &mut res.errors,
        ) {
            res.plans.insert(topic.clone(), plan);
        }
    }
    res
}
