# Route Derivation Algorithms

Scope: automatic derivation of a per-topic **forwarding tree** (route plan) from the declared communication graph and physical topology — local elimination, 1→N fan-out, multi-hop forwarding, explicit overrides (`prefer`/`deny`/`force`), external endpoints, acyclic re-export. Path analysis is polynomial (connectivity/bridge based), never simple-path enumeration. Implements architecture.md §Automatic Routing and Explicit Overrides, §External Endpoints. Owner: E3 (crate `drm-workspace::topology::route`). Phase 28 (implemented progressively from ).

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
| `REQ-ROUTE-external-endpoints` | external endpoints participate in the same graph | `REQ-ROUTE-009` |
| `REQ-ROUTE-acyclic-reexport` | imported topics are never re-exported back over the ingress link | `REQ-ROUTE-001` |
| `REQ-ROUTE-poly-paths` | path classification is polynomial; no simple-path enumeration | `REQ-ROUTE-019` |
| `REQ-ROUTE-determinism` | same input ⇒ same plans | `REQ-ROUTE-006` |

## Data Structures

```rust
pub struct RouteGraph {
 computes: BTreeSet<ComputeName>,
 links: Vec<Link>, // undirected; transport + bandwidth from drm.toml
 placement: BTreeMap<NodeName, ComputeName>,
 publishes: BTreeMap<NodeName, BTreeSet<TopicPath>>,
 consumes: BTreeMap<NodeName, BTreeSet<TopicPath>>,
 external: BTreeMap<ExternalName, ExternalEndpoint>, // {link, consume|produce topics}
 overrides: Overrides, // per-topic prefer/deny/force
}

// One topic may have many consumers, including local + remote on the same compute and
// multi-hop paths. A single Route cannot express this.
pub struct TopicRoutePlan {
 pub source: ComputeName,
 pub forwarding: BTreeMap<ComputeName, ForwardingRule>, // every compute the topic reaches
}
pub struct ForwardingRule {
 pub ingress: Option<LinkId>, // link the sample arrived on (None = origin compute)
 pub egress: BTreeSet<LinkId>, // links to forward on (deterministic order)
 pub local_delivery: bool, // deliver to local consumers on this compute
}

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

## Functions

```rust
fn derive(graph: &RouteGraph) -> RouteResult {
 let mut res = RouteResult::default();

 // 1. Validate explicit overrides first (I-R9) — contradictory intent is an error, not precedence.
 for (topic, ov) in &graph.overrides {
 if ov.force.len() > 1
 || ov.force.iter().any(|f| ov.deny.get(topic).map_or(false, |d| d.contains(f))) {
 res.errors.push(ConflictingRouteOverride { topic: topic.clone() });
 }
 }

 let producers = collect_producers(graph); // nodes ∪ external producers
 let consumers = collect_consumers(graph); // nodes ∪ external consumers
 for (topic, srcs) in &producers {
 for src in srcs {
 let plan = build_forwarding_tree(graph, topic, *src, consumers.get(topic), &res.errors);
 if let Some(p) = plan { res.plans.insert(topic.clone(), p); } // I-R10
 }
 }
 res
}

// Builds the whole tree in one pass: BFS over the deny-filtered, loop-safe edge set,
// recording ingress/egress/local_delivery per reached compute. Polynomial (I-R11).
fn build_forwarding_tree(graph, topic, src, consumer_eps, errors) -> Option<TopicRoutePlan> {
 let edges = graph.edges_after_deny(topic); // I-R4: deny prunes candidates
 // classify each (source, consumer-compute) path as Unique / Multiple / None without
 // enumerating simple paths: bridge/2-connectivity + bounded alternate-path check (I-R11).
 let forced = graph.overrides.force.get(topic); // pins the route (validated viable)
 let prefer = graph.overrides.prefer.get(topic);
 let selected: BTreeMap<ComputeName, Vec<LinkId>> = classify_paths(edges, src, consumer_eps,
 forced, prefer, errors)?; // Unique/Multiple/None per compute
 let mut plan = TopicRoutePlan { source: src, forwarding: BTreeMap::new() };
 for (compute, path) in selected { // deterministic (BTreeMap)
 for (i, link) in path.iter().enumerate() {
 let rule = plan.forwarding.entry(compute_at(&path, i)).or_default();
 if i == 0 { rule.ingress = None; } else { rule.ingress = Some(path[i-1]); }
 rule.egress.insert(*link); // union across consumers (fan-out)
 }
 plan.forwarding.entry(compute).or_default().local_delivery |= consumer_here(compute);
 }
 Some(plan)
}
```

## Protocols

*Derivation (per topic, deterministic order):*

1. Validate explicit overrides; emit `ConflictingRouteOverride` for contradictions (I-R9) and continue (I-R8).
2. Collect producers/consumers (internal nodes + external endpoints).
3. Deny-prune the edge set; compute loop-safe reachability from the source.
4. Classify each reached consumer compute as Unique / Multiple / None via polynomial analysis (I-R11); apply force → prefer → ambiguity error.
5. Merge per-consumer paths into one `TopicRoutePlan`, unioning egress edges (fan-out) and recording local delivery (I-R10).
6. Collect every error into `RouteResult.errors`; keep successfully derived plans.
7. Emit plans keyed by topic; verify (verify-engine.md V-RT) and router consume the forwarding rules.

## Edge Cases & Errors

| Case | Result |
|---|---|
| deny removes only path | `NoRoute` (explicit error) |
| force link not viable (disconnected) | error naming force |
| force ∧ deny same link | `ConflictingRouteOverride` (I-R9) |
| two force routes | `ConflictingRouteOverride` (I-R9) |
| one source, consumers on B, C, D (fan-out) | one plan with egress {A→B, A→D, B→C, B→D} + B local (I-R10) |
| local + remote consumers on same compute | plan records both `local_delivery` and egress (I-R10) |
| topic would re-export over ingress link | excluded (I-R7) |
| disconnected forest | consumers on another island ⇒ `NoRoute` |
| large cyclic topology | classification completes in O(V+E)-scale time (I-R11) |

## Test Matrix

| Case | Expected |
|---|---|
| `route_local_only` | I-R1 |
| `route_unique` | I-R2 |
| `route_ambiguous` | I-R3 error names both paths |
| `route_overrides` (8 fixtures) | I-R4 |
| `route_conflicting_override` (force∧deny, 2×force) | I-R9 |
| `route_fanout` (1→N, multi-hop) | I-R10 |
| `route_local_and_remote` | I-R10 |
| `route_determinism` ×100 | I-R5 |
| `route_external` (consume + produce) | I-R6 |
| `route_acyclic_reexport` (A↔B↔C loop) | I-R7 |
| `route_error_collection` (multiple failures) | I-R8 |
| `route_cyclic_scale` (property-generated cyclic graphs) | I-R11 bounded time |
| 3-fixture workspaces in CI | green |
