# Route Derivation Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: automatic derivation of a per-topic **forwarding tree** (route plan) from the declared communication graph and physical topology — local elimination, 1→N fan-out, multi-hop forwarding, explicit overrides (`prefer`/`deny`/`force`/`export_limit`), external endpoints as first-class vertices, explicit-hop tree construction under a one-parent invariant, acyclic re-export. Path analysis is polynomial (connectivity/bridge based), never simple-path enumeration. Implements architecture.md §Automatic Routing and Explicit Overrides, §External Endpoints. Owner: E3 (crate `drm-workspace::topology::route`). Phase 28.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-derive-auto` | single viable path ⇒ automatic; same-compute ⇒ local-only | `REQ-ROUTE-004` |
| `REQ-ROUTE-forwarding-tree` | one topic ⇒ a forwarding tree, not one route (fan-out + local delivery) | `REQ-ROUTE-010` |
| `REQ-ROUTE-derive-ambiguity` | ≥ 2 paths without policy ⇒ verify error, never silent choice | `REQ-ROUTE-003` |
| `REQ-ROUTE-derive-errors` | derivation returns `{plans, errors}`; never short-circuits verification | `REQ-ROUTE-005` |
| `REQ-ROUTE-override-conflict` | contradictory explicit overrides ⇒ `ConflictingRouteOverride` | `REQ-ROUTE-014` |
| `REQ-ROUTE-override-prefer` | prefer selects among viable paths | `REQ-ROUTE-017` |
| `REQ-ROUTE-override-deny` | deny removes candidate links before selection | `REQ-ROUTE-015` |
| `REQ-ROUTE-override-force` | force pins the route; conflicts with deny ⇒ error | `REQ-ROUTE-016` |
| `REQ-ROUTE-external-endpoints` | external endpoints participate in the same graph as first-class vertices | `REQ-ROUTE-009` |
| `REQ-ROUTE-acyclic-reexport` | imported topics are never re-exported back over the ingress link | `REQ-ROUTE-001` |
| `REQ-ROUTE-poly-paths` | path classification is polynomial; no simple-path enumeration | `REQ-ROUTE-019` |
| `REQ-ROUTE-determinism` | same input ⇒ same plans | `REQ-ROUTE-006` |
| `REQ-ROUTE-plan-vertices` | plans address external vertices via `VertexId`; runtime tables compute-only | `REQ-ROUTE-043` |
| `REQ-ROUTE-hop-plan` | plans built from explicit hops; one-parent merge invariant | `REQ-ROUTE-044` |

## Data Structures

```rust
pub enum VertexId { Compute(ComputeName), External(ExternalName) } // graph + plan vertices (I-R6)

pub struct RouteGraph {
 computes: BTreeSet<ComputeName>,
 links: Vec<Link>, // undirected; transport + bandwidth from drm.toml
 placement: BTreeMap<NodeName, ComputeName>,
 publishes: BTreeMap<NodeName, BTreeSet<TopicPath>>,
 consumes: BTreeMap<NodeName, BTreeSet<TopicPath>>,
 external: BTreeMap<ExternalName, ExternalEndpoint>, // {link, consume|produce topics}
 overrides: Overrides, // per-topic prefer/deny/force/export_limit
}

// One topic may have many consumers, including local + remote on the same compute and
// multi-hop paths. A single Route cannot express this.
pub struct TopicRoutePlan {
 pub source: VertexId, // compute origin or external producer (I-R12: exactly one producer)
 pub forwarding: BTreeMap<ComputeName, ForwardingRule>, // runtime tables: computes only (REQ-ROUTE-043)
}
pub struct ForwardingRule {
 pub ingress: Option<LinkId>, // link the sample arrived on (None = source compute)
 pub egress: BTreeSet<ForwardingEdge>, // links to forward on (deterministic order)
 pub local_delivery: bool, // deliver to local consumers on this compute
}
pub struct Overrides { // explicit per-topic intent; every map keyed by canonical topic path
 pub prefer: BTreeMap<TopicPath, LinkId>,
 pub deny: BTreeMap<TopicPath, BTreeSet<LinkId>>,
 pub force: BTreeMap<TopicPath, BTreeSet<LinkId>>, // ≥ 2 links ⇒ ConflictingRouteOverride (I-R9)
 pub export_limit: BTreeMap<(TopicPath, LinkId), Rate>,
}
pub struct ForwardingEdge { pub link: LinkId, pub export_limit: Option<Rate> } // declared shaping carried into plan (I-R14)
pub struct Hop { pub from: VertexId, pub link: LinkId, pub to: VertexId } // explicit hop — plans are built from hops, never reconstructed from link indices

pub struct RouteResult {
 pub plans: BTreeMap<TopicPath, TopicRoutePlan>, // valid plans (derivation continues past errors)
 pub errors: Vec<VerifyError>, // all failures, not just the first
}
```

## Invariants

- **I-R1 LocalNeverLeaves**: a topic with only same-compute consumers has no egress edges; it never crosses a link. Test: `route_local_only`.
- **I-R2 UniqueAuto**: exactly one viable path per (source, consumer-compute) ⇒ auto plan, no config. Test: `route_unique`.
- **I-R3 AmbiguityError**: ≥ 2 viable paths ∧ no prefer/force ⇒ `Ambiguous` error naming paths. Test: `route_ambiguous`.
- **I-R4 OverridesPipeline**: selection pipeline is config-validity → `deny` (prune) → `force` (pin) → `prefer` (select among viable) → auto. Test: `route_overrides` (8-fixture matrix).
- **I-R5 Deterministic**: same graph ⇒ byte-identical plans (BTree iteration; no tie-breaking randomness). Test: `route_determinism`.
- **I-R6 ExternalSameGraph**: external endpoints resolved as graph vertices; no parallel export list. Test: `route_external`.
- **I-R7 AcyclicReexport**: a topic imported over a link is never exported back over that same ingress link; a plan has no router loop. Test: `route_acyclic_reexport`.
- **I-R8 CollectNotShortCircuit**: derivation returns all errors (e.g. multiple ambiguous topics) without aborting. Test: `route_error_collection`.
- **I-R9 ConflictingOverride**: for a topic, `force` and `deny` targeting the same link, or ≥ 2 `force` routes, is a verify error `ConflictingRouteOverride` — never resolved by precedence. Test: `route_conflicting_override`.
- **I-R10 FanOutComplete**: every compute that has a consumer of the topic reachable from the source appears exactly once in `forwarding`; local and remote consumers on the same compute are both represented (plans are never overwritten per consumer). Test: `route_fanout`, `route_local_and_remote`.
- **I-R11 PolyPaths**: path existence/uniqueness is decided by connectivity + bridge/2-connectivity analysis (O(V+E)), with a bounded alternate-path check for multi-path classification; `all_paths` simple-path enumeration is prohibited. Test: `route_cyclic_scale` (large cyclic graph completes in bounded time).
- **I-R12 OneRootedPlan**: a topic with other than one producer yields no plan; every valid plan is a rooted arborescence with exactly one ingress per non-source compute. Test: `route_source_cardinality`, `route_tree_consistency`.
- **I-R13 HopMergeOneParent**: plans are constructed from explicit `Hop`s. For each hop: source compute gains egress; destination compute gains at most one ingress — a second, different parent for the same compute is an error (`InconsistentFanout`), never silently unioned. Test: `route_two_parent_rejected`.
- **I-R14 ExportLimitCarried**: declared per-`(topic, link)` `export_limit` is resolved into each plan egress edge; feasibility reads it (feasibility-estimation.md), router enforces it (lanes-shaping.md). Test: `route_export_limit_carried`.

## Functions

```rust
fn derive(graph: &RouteGraph) -> RouteResult {
 let mut res = RouteResult::default();

 // 1. Validate explicit overrides first (I-R9) — contradictory intent is an error, not precedence.
 // A conflict requires `force` (≥2, or force∩deny), so force keys cover every case; deny-only is harmless.
 for topic in graph.overrides.force.keys() {
  let forced = graph.overrides.force.get(topic);
  let denied = graph.overrides.deny.get(topic);
  if forced.map_or(0, |f| f.len()) > 1
   || forced.map_or(false, |f| f.iter().any(|l| denied.map_or(false, |d| d.contains(l)))) {
   res.errors.push(ConflictingRouteOverride { topic: topic.clone() });
  }
 }

 let producers: BTreeMap<TopicPath, BTreeSet<VertexId>> = collect_producers(graph); // nodes ∪ external producers
 let consumers: BTreeMap<TopicPath, BTreeSet<VertexId>> = collect_consumers(graph); // nodes ∪ external consumers
 for (topic, srcs) in &producers {
  if srcs.len() != 1 {
   res.errors.push(SingleWriterViolation { topic: topic.clone(), producers: srcs.clone() });
   continue; // never overwrite one source's plan with another source's plan
  }
  let src = *srcs.iter().next().unwrap(); // VertexId: Compute(_) or External(_)
  let plan = build_forwarding_tree(graph, topic, src, consumers.get(topic), &mut res.errors);
  if let Some(p) = plan { res.plans.insert(topic.clone(), p); } // I-R10/I-R12
 }
 res
}

// Builds the whole tree in one pass. Paths are explicit hop sequences (I-R13);
// classification is polynomial — connectivity + bridge/2-connectivity + bounded
// alternate-path check (I-R11), never simple-path enumeration.
fn build_forwarding_tree(graph, topic, src: VertexId, consumer_eps, errors) -> Option<TopicRoutePlan> {
 let edges = graph.edges_after_deny(topic); // I-R4: deny prunes candidates
 let forced = graph.overrides.force.get(topic); // pins the route (validated viable)
 let prefer = graph.overrides.prefer.get(topic);
 let selected: BTreeMap<VertexId, Vec<Hop>> = classify_paths(edges, src, consumer_eps, forced, prefer, errors)?;
 // Unique/Multiple/None per consumer vertex; force → prefer → Ambiguous error (I-R3).

 let mut plan = TopicRoutePlan { source: src, forwarding: BTreeMap::new() };
 for (_consumer, hops) in selected { // deterministic (BTreeMap)
 for hop in &hops {
  if let VertexId::Compute(from) = hop.from {
   plan.forwarding.entry(from).or_default()
    .egress.insert(ForwardingEdge { link: hop.link, export_limit: limit_for(graph, topic, hop.link) }); // I-R14
  }
  if let VertexId::Compute(to) = hop.to {
   let rule = plan.forwarding.entry(to).or_default();
   if rule.ingress.is_some() && rule.ingress != Some(hop.link) {
    errors.push(InconsistentFanout { topic, compute: to }); return None; // I-R13: second parent
   }
   rule.ingress = Some(hop.link);
  } // External(_) destination: delivery over hop.link from the source compute's egress; no runtime rule
 }
 if let VertexId::Compute(c) = _consumer {
  plan.forwarding.entry(c).or_default().local_delivery |= consumer_here(_consumer);
 }
 }
 if !is_rooted_acyclic_arborescence(&plan) { // source indegree 0; every reached compute indegree 1; acyclic; reachable
  errors.push(InconsistentFanout { topic }); return None;
 }
 Some(plan)
}
```

## Protocols

*Derivation (per topic, deterministic order):*

1. Validate explicit overrides; emit `ConflictingRouteOverride` for contradictions (I-R9) and continue (I-R8).
2. Collect producers/consumers as `VertexId`s (internal nodes + external endpoints).
3. Deny-prune the edge set; compute loop-safe reachability from the source vertex.
4. Classify each reached consumer vertex as Unique / Multiple / None via polynomial analysis (I-R11); apply force → prefer → ambiguity error.
5. Build the plan from explicit hops (I-R13): hop source computes gain egress (with declared `export_limit`, I-R14); hop destination computes gain their single ingress; merge under the one-parent invariant.
6. Validate the rooted arborescence (source indegree 0; one ingress per reached compute; acyclic; reachable).
7. Collect every error into `RouteResult.errors`; keep successfully derived plans.
8. Emit plans keyed by topic; verify (verify-engine.md V-RT) and router consume the forwarding rules.

## Edge Cases & Errors

| Case | Result |
|---|---|
| deny removes only path | `NoRoute` (explicit error) |
| force link not viable (disconnected) | error naming force |
| force ∧ deny same link | `ConflictingRouteOverride` (I-R9) |
| two force routes | `ConflictingRouteOverride` (I-R9) |
| one source, consumers on B, C, D (fan-out, tree A→B, B→C, B→D) | one valid arborescence plan (I-R10/I-R12) |
| one source, two viable paths to D (A→D and A→B→D), no policy | `Ambiguous` error naming both paths (I-R3) — never emit a two-parent D |
| selected paths give one compute two ingresses | `InconsistentFanout` (I-R13); require explicit consistent overrides |
| external producer is source | plan `source = External(_)`; first compute on the link records `ingress = that link` (I-R6/REQ-ROUTE-043) |
| external consumer | last compute's egress covers its link; no runtime rule for the device itself |
| local + remote consumers on same compute | plan records both `local_delivery` and egress (I-R10) |
| topic would re-export over ingress link | excluded (I-R7) |
| disconnected forest | consumers on another island ⇒ `NoRoute` |
| large cyclic topology | classification completes in O(V+E)-scale time (I-R11) |
| zero or multiple producers | single-writer error; no route plan emitted (I-R12) |

## Test Matrix

| Case | Expected |
|---|---|
| `route_local_only` | I-R1 |
| `route_unique` | I-R2 |
| `route_ambiguous` | I-R3 error names both paths |
| `route_overrides` (8 fixtures) | I-R4 |
| `route_conflicting_override` (force∧deny, 2×force) | I-R9 |
| `route_fanout` (1→N tree, multi-hop) | I-R10 + arborescence holds |
| `route_local_and_remote` | I-R10 |
| `route_two_parent_rejected` (A→D ∧ B→D merge attempt) | I-R13 `InconsistentFanout` |
| `route_determinism` ×100 | I-R5 |
| `route_external` (consume + produce as source) | I-R6 + REQ-ROUTE-043 |
| `route_acyclic_reexport` (A↔B↔C loop) | I-R7 |
| `route_error_collection` (multiple failures) | I-R8 |
| `route_cyclic_scale` (property-generated cyclic graphs) | I-R11 bounded time |
| `route_export_limit_carried` | I-R14 edge metadata present |
| `route_source_cardinality` / `route_tree_consistency` | I-R12 |
| 3-fixture workspaces in CI | green |
