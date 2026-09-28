# Feasibility Estimation Algorithms

Scope: declared-feasibility reports — nominal link utilization from typed `expected_rate` metadata (normalized by workspace-model.md), per-edge encoded frame sizes owned by each transport adapter, route `export_limit` shaping propagated as effective rate, per-link/per-route breakdown, impossible-configuration rejection collected into the verify report, and the honest boundary: estimates are engineering numbers, never runtime guarantees. Implements architecture.md §What `drm verify` Estimates, §Traffic Shaping and Bandwidth Budgeting. Owner: E2 (crate `drm-workspace::verifier::feasibility`). Phase 27. Depends on [verify-engine.md](verify-engine.md), [canon-codec.md](../canon-codec.md), [route-derivation.md](route-derivation.md), [workspace-model.md](workspace-model.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-estimate-utilization` | nominal utilization from typed expected_rate × per-edge wire sizes | `REQ-TOOL-022` |
| `REQ-TOOL-estimate-per-edge` | frame size computed per route edge (transport adapter sizing) | `REQ-TOOL-020` |
| `REQ-TOOL-estimate-reject-impossible` | declared traffic > 100% link ⇒ verify error (integer bps), collected not short-circuiting | `REQ-TOOL-021` |
| `REQ-TOOL-estimate-headroom` | headroom reported as engineering estimate | `REQ-TOOL-018` |
| `REQ-TOOL-estimate-not-guarantee` | estimates never presented as guarantees; runtime measured by drm-diag | `REQ-TOOL-019` |
| `REQ-TOOL-estimate-shaping` | declared `export_limit` caps effective rate along the route branch | `REQ-TOOL-077` |
| `REQ-TOOL-estimate-sizing` | each transport adapter owns its wire-size function (COBS payload-dependent) | `REQ-TOOL-078` |
| `REQ-TOOL-estimate-report` | feasibility returns `{estimates, warnings, errors}`; per-link independent | `REQ-TOOL-079` |

## Data Structures

```rust
pub struct LinkEstimate {
 link: LinkId,
 nominal_bps: u128, // Σ over topic edges routed on link (shaping-aware)
 capacity_bps: u128, // declared link bandwidth = physical line capacity (architecture.md)
 utilization_pct: f64, // nominal / capacity × 100 (display only)
 headroom_pct: f64, // 100 − utilization (display only)
 per_topic: Vec<TopicLoad>, // breakdown: {topic, effective_hz, wire_bits, bps}
}
pub struct TopicLoad { topic: TopicPath, effective_hz: Rational, wire_bits: u128, bps: u128 }

pub struct FeasibilityReport {
 estimates: Vec<LinkEstimate>,
 warnings: Vec<VerifyWarning>, // e.g. missing expected_rate; deadline tighter than nominal cadence
 errors: Vec<VerifyError>, // e.g. OverCapacity, RateOverflow, InvalidCapacity — one per link, never short-circuit
}

// Each transport adapter owns exactly one sizing implementation (REQ-TOOL-078).
pub trait TransportSizing {
 fn max_payload(&self) -> Option<usize>; // datagram bound; None = stream/unbounded
 fn max_wire_size(&self, drm_frame_bytes: usize) -> usize; // full wire bytes incl. header, CRC, framing, COBS expansion — this per-edge size is exactly the runtime shaper's charge_bytes (lanes-shaping.md I-LS9)
 fn capacity_unit(&self) -> CapacityUnit; // Bits/s vs Baud (line encoding factor)
}
// Serial sizing: DRM header + payload + CRC32C (+ optional e2e CRC) + COBS worst-case expansion
// (1 extra byte per ≤254 payload bytes — payload-DEPENDENT) + 0x00 delimiter, baud → bits/s conversion.
// UDP sizing: DRM frame + UDP + IP headers per declared link semantics.
```

## Invariants

- **I-F1 ReferenceExact**: independently calculated reference networks (≥ 5 fixtures) match DRM estimates exactly (bit-level on the integer bps; pct within float-tolerance spec'd in fixture). Test: `feasibility_reference`.
- **I-F2 ImpossibleRejected**: `nominal_bps > capacity_bps` (integer comparison, `u128` checked arithmetic) ⇒ verify **error naming link and overflow**, collected into the report (I-F8) — never decided on float pct. Test: `feasibility_over_capacity`, `feasibility_integer_boundary` (99.999%).
- **I-F3 BreakdownComplete**: per-topic rows sum exactly to link total. Test: `feasibility_breakdown_sum`.
- **I-F4 NotAGuarantee**: output labels estimates `ESTIMATE`; no wording implies runtime proof; runtime reality belongs to drm-diag. Test: `feasibility_labels` + doc-lint for banned phrases.
- **I-F5 PerEdge**: wire size is computed per route edge by that edge's transport adapter (I-F7); a multihop route with heterogeneous transports applies each edge's own sizing. Test: `feasibility_per_edge`.
- **I-F6 ShapingPropagated**: effective rate along a branch is `min(ingress_effective, edge.export_limit)`; a downstream router cannot recreate frames dropped upstream, so the cap propagates. This is the sustained nominal rate (token bucket: bursts are a runtime property, lanes-shaping.md). Test: `feasibility_export_limit` (60 Hz publisher, 10 Hz limit ⇒ 10 Hz × wire size).
- **I-F7 AdapterOwnedSizing**: overhead is a function of (transport adapter, integrity profile, payload size) — never a constant per transport. COBS expansion is payload-dependent. Test: `feasibility_cobs_scales`.
- **I-F8 CollectAll**: per-link evaluation is independent; every over-capacity, overflow, and invalid-capacity error is returned in `FeasibilityReport.errors`; estimates and warnings for healthy links still present. No `?` propagation, no early abort. Test: `feasibility_all_links_reported`.

## Functions

```rust
fn estimate(model: &WorkspaceModel, routes: &RouteResult) -> FeasibilityReport {
 let mut rep = FeasibilityReport::default();
 let mut per_link: BTreeMap<LinkId, Vec<TopicLoad>> = BTreeMap::new();
 for (topic, plan) in &routes.plans { // route-derivation forwarding tree
 let publisher = model.publisher(topic);
 let Some(rate) = publisher.expected_rate else { // typed Rate (workspace-model I-D10)
  rep.warnings.push(MissingRate { topic }); // excluded from nominal; documented
  continue;
 };
 let src_eff = rate;
 for hop in plan.hops_bfs_from_source() { // deterministic tree order (BTreeMap)
  let limit = hop.edge.export_limit; // I-R14: declared per-(topic, link) shaping
  let eff = min_checked(src_eff, limit, &mut rep.errors); // I-F6: cap propagates downstream
  let sizing = adapter_for(hop.link).sizing(); // I-F7: adapter-owned wire size
  let wire_bytes = sizing.max_wire_bytes(drm_frame_bytes(model, topic, hop.link)); // incl. COBS
  let wire_bits = wire_bytes as u128 * 8;
  let Some(bps) = eff.checked_mul_bits(wire_bits) else {
   rep.errors.push(RateOverflow { topic, link: hop.link }); continue; // I-F8: collected
  };
  per_link[hop.link].push(TopicLoad { topic: topic.clone(), effective_hz: eff, wire_bits, bps });
  if let VertexId::Compute(next) = hop.to { src_eff_at(next) = eff; } // branch-effective rate
 }
 }
 for (link, rows) in per_link {
 let nominal = rows.iter().fold(0u128, |s, r| s.saturating_add(r.bps)); // overflow ⇒ collected error above
 let cap = model.link(link).bandwidth_bps as u128;
 if cap == 0 { rep.errors.push(InvalidCapacity { link }); continue; } // I-F8
 if nominal > cap { rep.errors.push(OverCapacity { link, nominal, cap }); } // I-F2: integer compare, collected
 let util = (nominal as f64 / cap as f64) * 100.0; // display only
 rep.estimates.push(LinkEstimate { link, nominal_bps: nominal, capacity_bps: cap,
  utilization_pct: util, headroom_pct: 100.0 - util, per_topic: rows });
 }
 rep // verify-engine merges: estimates → r.estimates; warnings → r.warnings; errors → facts (I-V5)
}
```

## Protocols

*Report emission (verify-engine §Functions last step):*

1. Compute per-edge, per-link estimates over derived routes; adapter-owned wire sizing per edge (I-F5/I-F7); shaping-aware effective rates (I-F6).
2. Collect impossible configurations (`nominal_bps > capacity_bps`, overflow, invalid capacity) as report errors (I-F2/I-F8) — verify prints them with the facts; healthy links still report estimates.
3. Print estimates block explicitly labeled `ESTIMATE (declared feasibility — runtime measured by drm-diag)`.

## Edge Cases & Errors

| Case | Result |
|---|---|
| expected_rate absent on publisher | topic excluded from nominal (documented); warning suggests declaring rate for estimation quality |
| publisher 60 Hz, route `export_limit = 10 Hz` | effective rate 10 Hz × wire size on that branch (I-F6) — not a false over-capacity |
| rate declared but bursts exceed | estimate unaffected; sustained-rate model; runtime reality = drm-diag (token-bucket shaping at runtime: lanes-shaping.md) |
| multiple routes for one topic (after overrides) | load counted on each used edge |
| multihop route, mixed transports | per-edge adapter sizing applied (I-F5/I-F7) |
| serial link bandwidth declared in baud | `capacity_unit()` converts line encoding to bits/s for the comparison (I-F7) |
| serial link declared | report `blocking_max = 10 × max_wire_size / baud` per link as ESTIMATE against declared deadlines (serial-mcu.md I-SR7) — warning, never a hard failure |
| `link.bandwidth` | physical line capacity (architecture.md); adapter counts its own full wire bytes against it |
| zero-capacity link declared | `InvalidCapacity` collected; verify surfaces with topology facts |
| pct exactly 100.0000001 but bps equal | accepted — decision is integer bps (I-F2) |
| `effective_hz × wire_bits` exceeds u128 | `RateOverflow` collected (checked rational arithmetic, I-F8) |
| two over-capacity links | both reported (I-F8) |

## Test Matrix

| Case | Expected |
|---|---|
| `feasibility_reference` (5 networks) | I-F1 exact |
| `feasibility_over_capacity` | I-F2 collected error with numbers |
| `feasibility_integer_boundary` (99.999% float, bps under) | accepted; integer decision |
| `feasibility_breakdown_sum` | I-F3 |
| `feasibility_per_edge` (multihop mixed transport) | I-F5 |
| `feasibility_export_limit` (60 Hz → 10 Hz branch) | I-F6 effective rate |
| `feasibility_cobs_scales` (payload 254 vs 255 bytes) | I-F7 overhead grows with payload |
| `feasibility_all_links_reported` (2 bad links + healthy links) | I-F8 all errors + all estimates |
| `feasibility_labels` + doc lint | I-F4 |
| example workspaces | estimates printed, labeled |
