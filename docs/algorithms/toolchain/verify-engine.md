# Static Verification Engine Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: `drm verify` — the complete catalogue of deterministic configuration checks (facts), their negative fixtures, and the strict boundary to feasibility estimates (Phase 27, [feasibility-estimation.md](feasibility-estimation.md)). Implements architecture.md §Verification and Runtime Observation: Facts, Feasibility, and Reality. Owner: E2 (crate `drm-workspace::verify::engine`). Phase 26. Depends on [workspace-model.md](workspace-model.md), [memory-budget.md](../memory-budget.md), [route-derivation.md](route-derivation.md), [handle-system.md](handle-system.md), [payload-pool.md](../runtime/payload-pool.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-verify-schemas` | schemas valid and consistent across packages | `REQ-TOOL-063` |
| `REQ-TOOL-verify-single-writer` | every consumed topic has exactly one publisher | `REQ-TOOL-064` |
| `REQ-TOOL-verify-placement` | placements/computes/links valid, no conflicting identifiers | `REQ-TOOL-060` |
| `REQ-TOOL-verify-route-fit` | routes exist; encoded frames fit `max_datagram_payload` | `REQ-TOOL-062` |
| `REQ-TOOL-verify-reliability` | `reliable` only on reliable-capability transports | `REQ-TOOL-061` |
| `REQ-TOOL-verify-budgets` | channels fit deployment memory budget; precommit fits lockable budget | `REQ-TOOL-057` |
| `REQ-TOOL-verify-compatibility` | consumer requirements on shared topics mutually compatible | `REQ-TOOL-058` |
| `REQ-TOOL-verify-estimates-boundary` | estimates never reported as guarantees (never prove timing) | `REQ-TOOL-059` |

## Data Structures

```rust
pub struct VerifyReport {
 facts: Vec<FactResult>, // pass/fail — deterministic
 warnings: Vec<Warning>, // non-fatal (e.g. over-RAM within budget)
 estimates: Vec<Estimate>, // labeled ESTIMATE, never "proven"
}
pub struct CheckId(&'static str); // stable IDs cited in diagnostics, e.g. "V-SW-1"
```

Check catalogue (each check ⇒ one negative fixture set):

| ID | Check | Fails when |
|---|---|---|
| `V-SCH` | schemas valid/consistent | grammar errors; cross-package schema hash mismatch on same topic |
| `V-SW` | single writer | zero publishers for consumed topic; ≥ 2 publishers |
| `V-PLC` | placement validity | node placed twice; unknown compute; unknown link endpoint |
| `V-RT` | route plan existence + fit | no path; ambiguous (no policy); `ConflictingRouteOverride` (force∧deny / multiple force); encoded frame > `max_datagram_payload` on datagram route; shaped bucket with `burst < max charge` (lanes-shaping I-LS9 — such a frame could never send); `safety_budget.rate > link_sustainable_rate`, or `safety_budget.rate + non_safety_shaper.rate > link_sustainable_rate` (I-LS12 capacity carving) |
| `V-REL` | reliability capability | `reliable` requirement (ConsumeMeta.reliability, workspace-model.md) routed onto best-effort transport (UDP/serial adapter) |
| `V-MEM` | budgets | worst-case > drm_budget; precommit locked > lockable; `pool_size < R+1` |
| `V-CMP` | consumer compatibility | incompatible shared-topic requirements (e.g. `reject` with zero required sequential consumers). Different per-consumer deadlines are **valid** (architecture.md: publisher cadence is capability, consumer deadline is requirement); nominal cadence-vs-deadline plausibility is a feasibility warning, not a failure |
| `V-ID` | identity | duplicate topic/node/compute names; handle table conflicts |
| `V-SPAT` | spatial contracts | two writers on one dynamic edge; write+transform conflict; dangling spatial frame; undeclared `[spatial] read`; index.drm capacity exceeding memory budget (I-MB9); history requirement unsatisfiable by its structural bound (`max_samples < floor(duration/min_sample_period)+2`, REQ-SPATIAL-037); naked-coordinate schema fields in spatial-typed topics |
| `V-RPC` | RPC route reliability + shape | generated request/cancel/event topics resolve to best-effort-only routes; event topic not single-writer server-owned; declared concurrency exceeds capacity model (rpc-actions.md) |
| `V-REC` | recorder backpressure declaration | `on_full = backpressure` on a `reject` topic without a declared required sequential recorder (record-replay.md) |

## Invariants

- **I-V1 CatalogueComplete**: every check above has ≥ 1 negative fixture + ≥ 1 positive fixture. Test: `verify_catalogue`.
- **I-V2 Deterministic**: same workspace ⇒ identical report (check order fixed; no fs-order dependence). Test: `verify_determinism`.
- **I-V3 REQcited**: every failure message cites ≥ 1 REQ ID + check ID. Test: `verify_msg_citations`.
- **I-V4 BoundaryHonest**: report never labels an estimate as a fact; estimates section explicitly marked (architecture rule — "never present an estimate as a guarantee"). Test: `verify_boundary_labels`.
- **I-V5 CollectAll**: every check contributes all its failures; route derivation returns `{plans, errors}` and its errors are appended without aborting the remaining checks. Feasibility returns a report (estimates + warnings + errors), never short-circuits. No short-circuit on any failure. Test: `verify_no_shortcircuit`.
- **I-V6 DeadlineSemantics**: per-consumer deadlines on one topic are independent requirements, never compared for conflict. A deadline tighter than the publisher's nominal interval ⇒ feasibility **warning**; actual misses are drm-diag runtime observations. Test: `verify_deadlines_independent`.

## Functions

```rust
fn verify(model: &WorkspaceModel, deploy: &DeploymentConfig) -> VerifyReport {
 let mut r = VerifyReport::default();
 check_schemas(model, &mut r); // V-SCH
 check_single_writer(model, &mut r); // V-SW (model builds with ≥ 2 publishers — workspace-model I-D12)
 check_placement(model, &mut r); // V-PLC
 let routes = route_derivation::derive(&graph_of(model)); // RouteResult {plans, errors}
 for e in routes.errors { r.facts.push(FactResult::Fail(e)); } // I-V5: collect, never short-circuit
 check_routes(model, &routes.plans, &mut r); // V-RT (forwarding-tree existence + framing fit)
 check_reliability(model, &routes.plans, &mut r); // V-REL (requirement vs link capability)
 memory_budget::verify_memory(model, deploy).into_report(&mut r); // V-MEM (memory-budget.md)
 check_compatibility(model, &mut r); // V-CMP (read_mode/required; deadlines independent — I-V6)
 check_identity(model, &mut r); // V-ID
 check_spatial(model, &mut r); // V-SPAT (spatial-core/workspace-model I-D6/I-D7; spatial-index I-X9)
 let f = feasibility::estimate(model, &routes); // FeasibilityReport (feasibility-estimation.md)
 r.estimates.extend(f.estimates);
 r.warnings.extend(f.warnings); // incl. cadence-vs-deadline plausibility (I-V6)
 for e in f.errors { r.facts.push(FactResult::Fail(e)); } // e.g. over-capacity, overflow — collected (I-V5)
 r
}

fn fail(check: CheckId, msg: impl Display, reqs: &[ReqId]) -> VerifyError {
 VerifyError { check, message: format!("{check}: {msg} [REQ: {}]", reqs.join(", ")) }
}
```

## Protocols

*`drm verify` invocation:*

1. Build WorkspaceModel (workspace-model.md) — parse/structural errors surface first with locations. Model build never enforces architectural policy (workspace-model I-D12), so all catalogue checks remain reachable.
2. Run checks in catalogue order; collect all failures (no short-circuit — full report per run).
3. Exit non-zero on any fact failure; warnings/estimates printed with explicit labels.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Parse errors | reported before checks; no partial results presented as verified |
| `reject` policy with no required sequential consumer | `V-CMP` failure (overflow-policies.md degenerate case) |
| Two consumers, deadlines 20 ms and 500 ms, same topic | valid — independent requirements (I-V6); no failure |
| Consumer deadline tighter than publisher nominal interval | feasibility warning (estimate); runtime misses measured by drm-diag |
| `reliable` consumer requirement routed over UDP/serial | `V-REL` failure |
| `multiplexer` convergence (two sources → one command topic) | valid — sources publish distinct topics; multiplexer node owns the canonical topic (single-writer preserved) |
| spatial two-writer edge; index capacity over budget | `V-SPAT` failures |
| Route ambiguity | `V-RT` failure naming the topic and the two paths |
| Contradictory overrides (force∧deny, multiple force) | `V-RT` `ConflictingRouteOverride` |
| Budget warning band | warning only (memory-budget I-MB3) |
| Lane caps or class budgets < 2× declared worst case | warning (lanes-shaping sizing guidance) — headroom, not invalid |
| Safety worst-case latency bound exceeds declared deadline | feasibility estimate/warning (lanes-shaping I-LS11), naming `max_nonpreemptible_wire_time`; remedy smaller max frame, faster link, or preemption-capable link — never a fact failure (timing is never provable) |
| Serial `blocking_max` vs declared deadlines (serial-mcu I-SR7) | feasibility estimate/warning; `blocking_max` is the `T_inflight_lower_priority` term of I-LS11 — never a fact failure |
| Estimates exceed 100% link | feasibility error collected into facts (I-V5); report still complete |

## Test Matrix

| Case | Expected |
|---|---|
| negative fixture per check ID (≥ 20 fixtures) | exact check ID + message + REQ citation |
| spatial contract fixtures (I-D6/I-D7 + index capacity) | `V-SPAT` exact |
| positive full-workspace fixture | zero fact failures |
| determinism ×100 | identical report |
| multiple independent failures | I-V5: all reported, none suppressed |
| independent deadlines on one topic | I-V6: valid; no V-CMP failure |
| tight deadline vs nominal cadence | warning, labeled estimate |
| estimate-labeling | `ESTIMATE` prefix present; no "guarantee" wording |
| example workspaces in CI | `drm verify` green |
| `verify_catalogue` | I-V1: every check ID has ≥ 1 negative + ≥ 1 positive fixture |
| `verify_determinism` (×100) | I-V2: identical report |
| `verify_msg_citations` | I-V3: every message cites ≥ 1 REQ ID + check ID |
| `verify_boundary_labels` | I-V4: `ESTIMATE` labeled; no "guarantee" wording |
| `verify_no_shortcircuit` (multiple failures) | I-V5: all failures collected |
| `verify_deadlines_independent` | I-V6: valid; no `V-CMP` failure |
