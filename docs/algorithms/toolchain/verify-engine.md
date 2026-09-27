# Static Verification Engine Algorithms

Scope: `drm verify` — the complete catalogue of deterministic configuration checks (facts), their negative fixtures, and the strict boundary to feasibility estimates (Phase 27, [feasibility-estimation.md](feasibility-estimation.md)). Implements architecture.md §Verification and Runtime Observation: Facts, Feasibility, and Reality. Owner: E2 (crate `drm-workspace::verifier`). Phase 26. Depends on [workspace-model.md](workspace-model.md), [memory-budget.md](../memory-budget.md), [route-derivation.md](route-derivation.md), [handle-system.md](handle-system.md), [payload-pool.md](../runtime/payload-pool.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-verify-schemas` | schemas valid and consistent across packages | — |
| `REQ-TOOL-verify-single-writer` | every consumed topic has exactly one publisher | — |
| `REQ-TOOL-verify-placement` | placements/computes/links valid, no conflicting identifiers | — |
| `REQ-TOOL-verify-route-fit` | routes exist; encoded frames fit `max_datagram_payload` | — |
| `REQ-TOOL-verify-reliability` | `reliable` only on reliable-capability transports | — |
| `REQ-TOOL-verify-budgets` | channels fit deployment memory budget; precommit fits lockable budget | — |
| `REQ-TOOL-verify-compatibility` | consumer requirements on shared topics mutually compatible | — |
| `REQ-TOOL-verify-estimates-boundary` | estimates never reported as guarantees (never prove timing) | — |

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
| `V-RT` | route plan existence + fit | no path; ambiguous (no policy); `ConflictingRouteOverride` (force∧deny / multiple force); encoded frame > `max_datagram_payload` on datagram route |
| `V-REL` | reliability capability | `reliable` contract on best-effort transport (UDP/serial adapter) |
| `V-MEM` | budgets | worst-case > drm_budget; precommit locked > lockable; `pool_size < R+1` |
| `V-CMP` | consumer compatibility | incompatible shared-topic requirements (e.g. `reject` with zero required sequential consumers; conflicting deadlines policy) |
| `V-ID` | identity | duplicate topic/node/compute names; handle table conflicts |
| `V-SPAT` | spatial contracts | two writers on one dynamic edge; write+transform conflict; dangling spatial frame; undeclared `[spatial] read`; index.drm capacity exceeding memory budget (I-MB9); naked-coordinate schema fields in spatial-typed topics |

## Invariants

- **I-V1 CatalogueComplete**: every check above has ≥ 1 negative fixture + ≥ 1 positive fixture. Test: `verify_catalogue`.
- **I-V2 Deterministic**: same workspace ⇒ identical report (check order fixed; no fs-order dependence). Test: `verify_determinism`.
- **I-V3 REQcited**: every failure message cites ≥ 1 REQ ID + check ID. Test: `verify_msg_citations`.
- **I-V4 BoundaryHonest**: report never labels an estimate as a fact; estimates section explicitly marked (architecture rule — "never present an estimate as a guarantee"). Test: `verify_boundary_labels`.
- **I-V5 CollectAll**: every check contributes all its failures; route derivation returns `{plans, errors}` and its errors are appended without aborting the remaining checks. No short-circuit on any failure. Test: `verify_no_shortcircuit`.

## Functions

```rust
fn verify(model: &WorkspaceModel, deploy: &DeploymentConfig) -> VerifyReport {
 let mut r = VerifyReport::default();
 check_schemas(model, &mut r); // V-SCH
 check_single_writer(model, &mut r); // V-SW
 check_placement(model, &mut r); // V-PLC
 let routes = route_derivation::derive(&graph_of(model)); // RouteResult {plans, errors}
 for e in routes.errors { r.facts.push(FactResult::Fail(e)); } // I-V5: collect, never short-circuit
 check_routes(model, &routes.plans, &mut r); // V-RT (forwarding-tree existence + framing fit)
 check_reliability(model, &routes.plans, &mut r); // V-REL (capability table)
 memory_budget::verify_memory(model, deploy).into_report(&mut r); // V-MEM (memory-budget.md)
 check_compatibility(model, &mut r); // V-CMP
 check_identity(model, &mut r); // V-ID
 r.estimates = feasibility::estimate(model, &routes); // feasibility-estimation.md (Phase 27)
 r
}

fn fail(check: CheckId, msg: impl Display, reqs: &[ReqId]) -> VerifyError {
 VerifyError { check, message: format!("{check}: {msg} [REQ: {}]", reqs.join(", ")) }
}
```

## Protocols

*`drm verify` invocation:*

1. Build WorkspaceModel (workspace-model.md) — parse errors surface first with locations.
2. Run checks in catalogue order; collect all failures (no short-circuit — full report per run).
3. Exit non-zero on any fact failure; warnings/estimates printed with explicit labels.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Parse errors | reported before checks; no partial results presented as verified |
| `reject` policy with no required sequential consumer | `V-CMP` failure (overflow-policies.md degenerate case) |
| Route ambiguity | `V-RT` failure naming the topic and the two paths |
| Contradictory overrides (force∧deny, multiple force) | `V-RT` `ConflictingRouteOverride` |
| Budget warning band | warning only (memory-budget I-MB3) |
| Estimates exceed 100% link | `V-RT`-family hard error per feasibility doc (impossible configuration) |

## Test Matrix

| Case | Expected |
|---|---|
| negative fixture per check ID (≥ 20 fixtures) | exact check ID + message + REQ citation |
| positive full-workspace fixture | zero fact failures |
| determinism ×100 | identical report |
| multiple independent failures | I-V5: all reported, none suppressed |
| estimate-labeling | `ESTIMATE` prefix present; no "guarantee" wording |
| example workspaces in CI | `drm verify` green |
