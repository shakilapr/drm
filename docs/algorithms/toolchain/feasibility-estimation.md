# Feasibility Estimation Algorithms

Scope: declared-feasibility reports — nominal link utilization from `expected_rate` manifests and per-edge encoded frame sizes, per-link/per-route breakdown, impossible-configuration rejection on integer bounds, and the honest boundary: estimates are engineering numbers, never runtime guarantees. Implements architecture.md §What `drm verify` Estimates. Owner: E2 (crate `drm-workspace::verifier::feasibility`). Phase 27. Depends on [verify-engine.md](verify-engine.md), [canon-codec.md](../canon-codec.md), [route-derivation.md](route-derivation.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-estimate-utilization` | nominal utilization from expected_rate × packet sizes | — |
| `REQ-TOOL-estimate-per-edge` | frame size computed per route edge (transport-specific overhead) | — |
| `REQ-TOOL-estimate-reject-impossible` | declared traffic > 100% link ⇒ build error (integer bps) | — |
| `REQ-TOOL-estimate-headroom` | headroom reported as engineering estimate | — |
| `REQ-TOOL-estimate-not-guarantee` | estimates never presented as guarantees; runtime measured by drm-diag | — |

## Data Structures

```rust
pub struct LinkEstimate {
 link: LinkId,
 nominal_bps: u128, // Σ over topic edges routed on link
 capacity_bps: u128, // declared link bandwidth
 utilization_pct: f64, // nominal / capacity × 100 (display only)
 headroom_pct: f64, // 100 − utilization (display only)
 per_topic: Vec<TopicLoad>, // breakdown: {topic, rate_hz, frame_bits, bps}
}
pub struct TopicLoad { topic: TopicPath, rate_hz: u64, frame_bits: u128, bps: u128 }
```

## Invariants

- **I-F1 ReferenceExact**: independently calculated reference networks (≥ 5 fixtures) match DRM estimates exactly (bit-level on the integer bps; pct within float-tolerance spec'd in fixture). Test: `feasibility_reference`.
- **I-F2 ImpossibleRejected**: `nominal_bps > capacity_bps` (integer comparison, `u128` checked arithmetic) ⇒ hard build error naming link and overflow. Never decided on float pct. Test: `feasibility_over_capacity`, `feasibility_integer_boundary` (99.999%).
- **I-F3 BreakdownComplete**: per-topic rows sum exactly to link total. Test: `feasibility_breakdown_sum`.
- **I-F4 NotAGuarantee**: output labels estimates `ESTIMATE`; no wording implies runtime proof; runtime reality belongs to drm-diag. Test: `feasibility_labels` + doc-lint for banned phrases.
- **I-F5 PerEdge**: encoded size is computed per route edge; a multihop route with heterogeneous transports applies each edge's own overhead. Test: `feasibility_per_edge`.

## Functions

```rust
fn estimate(model: &WorkspaceModel, plans: &RoutePlans) -> Vec<LinkEstimate> {
 let mut per_link: BTreeMap<LinkId, Vec<TopicLoad>> = BTreeMap::new();
 for (topic, plan) in plans.iter() { // route-derivation forwarding tree
 let rate = model.publisher(topic).expected_rate; // planning metadata only
 for (_compute, rule) in &plan.forwarding {
 for link in &rule.egress { // I-F5: per forwarding edge, not per topic
 let frame_bits = encoded_bits(model, topic, *link); // payload + this link's transport overhead
 let bps = (rate as u128).checked_mul(frame_bits).ok_or(Overflow)?;
 per_link[link].push(TopicLoad { topic: topic.clone(), rate_hz: rate, frame_bits, bps });
 }
 }
 }
 per_link.into_iter().map(|(link, rows)| {
 let nominal: u128 = rows.iter().map(|r| r.bps).sum(); // I-F3 (u128)
 let cap = model.link(link).bandwidth_bps as u128;
 if nominal > cap { hard_error(OverCapacity { link, nominal, cap }); } // I-F2: integer compare
 let util = (nominal as f64 / cap as f64) * 100.0; // display only
 LinkEstimate { link, nominal_bps: nominal, capacity_bps: cap,
 utilization_pct: util, headroom_pct: 100.0 - util, per_topic: rows }
 }).collect()
}

fn encoded_bits(model: &WorkspaceModel, topic: &TopicPath, link: LinkId) -> u128 {
 (canon_size(model.schema(topic)) as u128 // payload bound (worst-case declared)
 + transport_metadata_overhead(transport_kind(link)) as u128) // this edge's framing overhead
 * 8
}
```

## Protocols

*Report emission (verify-engine §Functions last step):*

1. Compute per-edge, per-link estimates over derived routes.
2. Reject impossible (`nominal_bps > capacity_bps`) as hard error on integer bps (I-F2) — before warnings/estimates print.
3. Print estimates block explicitly labeled `ESTIMATE (declared feasibility — runtime measured by drm-diag)`.

## Edge Cases & Errors

| Case | Result |
|---|---|
| expected_rate absent on publisher | topic excluded from nominal (documented); warning suggests declaring rate for estimation quality |
| rate declared but bursts exceed | estimate unaffected; runtime reality = drm-diag (burst shaping handled by lanes-shaping.md at runtime) |
| multiple routes for one topic (after overrides) | load counted on each used edge |
| multihop route, mixed transports | per-edge overhead applied (I-F5) |
| zero-capacity link declared | build error at topology parse (V-PLC family) |
| pct exactly 100.0000001 but bps equal | accepted — decision is integer bps (I-F2) |
| `rate × frame_bits` exceeds u128 | `Overflow` hard error (checked_mul) |

## Test Matrix

| Case | Expected |
|---|---|
| `feasibility_reference` (5 networks) | I-F1 exact |
| `feasibility_over_capacity` | I-F2 hard error with numbers |
| `feasibility_integer_boundary` (99.999% float, bps under) | accepted; integer decision |
| `feasibility_breakdown_sum` | I-F3 |
| `feasibility_per_edge` (multihop mixed transport) | I-F5 |
| `feasibility_labels` + doc lint | I-F4 |
| example workspaces | estimates printed, labeled |
