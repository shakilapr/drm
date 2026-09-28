# Memory Budget & Precommit Algorithms

Scope: deterministic memory verification against configured deployment budgets — overflow-checked worst-case capacity calculation, budget comparison rules, drop-free shortfall warning (statically known `H`), warning band, per-region lockable accounting (page-rounded, conservative), per-process locked working set — and the hard-real-time precommit protocol (backend-defined lock proof, refuse-on-failure at activation). Implements architecture.md §Memory Budgets as Build-Time Verification, §Hard Real-Time Precommitment, §Payload Pool Invariant (sizing rules). Owner: E2 (verifier math in `drm-workspace::verifier`) + E1 (activation in `drm-core::channel`). Phase 19. Depends on [payload-pool.md](runtime/payload-pool.md) (invariant), [channel-metadata.md](runtime/channel-metadata.md) (attach check), [shm-backend.md](runtime/shm-backend.md) (platform locking), [windows-backends.md](runtime/windows-backends.md) (Windows proof), [router-core.md](router/router-core.md) (lane buffers).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-budget-config-declared` | each compute target declares DRM memory budget in deployment config | `REQ-TOOL-004` |
| `REQ-TOOL-budget-worst-case-calc` | Σ worst-case capacity if every slot filled to max bound simultaneously | `REQ-TOOL-010` |
| `REQ-TOOL-budget-checked-arith` | all capacity math checked; overflow ⇒ `CapacityOverflow`, never a wrapped small value | `REQ-TOOL-003` |
| `REQ-TOOL-budget-hard-error` | worst-case > configured budget ⇒ hard error; locked > lockable budget ⇒ hard error | `REQ-TOOL-005` |
| `REQ-TOOL-budget-ram-warning` | > physical RAM but ≤ budget ⇒ strong warning, not invalid | `REQ-TOOL-008` |
| `REQ-TOOL-budget-nominal-informational` | nominal estimate informational only | `REQ-TOOL-006` |
| `REQ-TOOL-budget-per-process-locked` | each precommit node's own mapped locked set fits the lockable budget | `REQ-TOOL-007` |
| `REQ-TOOL-budget-router-buffers` | router-owned lane buffers counted against the compute budget | `REQ-TOOL-009` |
| `REQ-CORE-precommit-refuse-activation` | pre-fault+lock fails ⇒ refuse active state, never continue pageable | `REQ-CORE-090` |
| `REQ-CORE-precommit-static-checks` | verifier checks backend locking capability + lockable budget coverage | `REQ-CORE-091` |

## Data Structures

```rust
pub struct MemoryBudget {
 drm_budget_bytes: u64, // configured DRM contribution ceiling (general targets)
 lockable_budget_bytes: u64, // ceiling for mlock/VirtualLock (RT/precommit targets)
 physical_ram_bytes: u64, // advisory reference for the warning band
 precommit: bool, // target policy (backend capability)
 page_granularity: u32, // target-declared (4 KiB typical; ARM64 kernels may run 16/64 KiB pages)
}

pub struct MemoryReport { // returned, never a Result — one run reports ALL errors (verify-engine I-V5)
 errors: Vec<VerifyError>, // empty ⇒ V-MEM passes; any entry ⇒ build failure. The only gate.
 warnings: Vec<VerifyWarning>, // e.g. OverPhysicalRam, DropFreeShortfall — printed, never failing
 estimates: Vec<ComputeMemoryReport>, // nominal + per-class worst cases; informational only
}
impl MemoryReport {
 fn into_report(self, r: &mut VerifyReport) { /* folds errors→facts, warnings, estimates (verify-engine.md) */ }
}

 pub struct ChannelCost { // per active channel on a compute unit
  pool_slots: u64, // validated ≥ R+1 (payload-pool I-P4)
  slot_bytes: u64, // max sample size: fixed header + max tail capacity (canon-codec two-region)
  control_block: u64, // CCB, hazard array (declared borrower records + diagnostic range), ring — fixed overhead
  precommit: bool, // drives the lockable sum (per channel, not per compute)
 }

 pub struct SpatialIndexCost { // per compute unit with declared spatial indexes (spatial-index.md I-X9)
  cell_hash_slots: u64, // S = ceil(M / alpha_max) open-addressing cell entries (alpha_max <= 0.70)
  membership_pool: u64, // M = max_objects × max_cells_per_object fixed membership nodes
  object_records: u64, // per-object envelope, occupied cells, dependency vector at max_objects
  bvh_nodes: u64, // every guarded compiled/calibration bank
  query_scratch: u64, // fixed per declared concurrent query endpoint
  // no heap growth is possible — capacity is declared; verified exactly like channel buffers
 }
```

## Invariants

- **I-MB1 WorstCaseExact**: `worst_case(compute) = Σ_ch (pool_slots × slot_bytes + control_block) + router_buffers + spatial_index_memory` over active channels — matches independently derived reference values on ≥ 5 fixture deployments. Test: `budget_reference_match`.
- **I-MB2 HardErrors**: worst-case > `drm_budget` ⇒ build error; `Σ locked spans` > `lockable_budget` ⇒ build error. The lockable sum covers every precommit-flagged region: channels with `ch.precommit == true`, plus router lane buffers and spatial banks/scratch when the owning compute is precommit (channel-level flag for channels; compute-level for router/spatial, which have no per-channel flag). Never gated the other way. Test: `budget_hard_errors` (negative matrix).
- **I-MB3 WarningBand**: worst-case > physical RAM ∧ ≤ drm_budget ⇒ strong warning (not error), names the overflow amount. Test: `budget_warning_band`.
- **I-MB4 PoolInvariant**: `pool_slots < R+1` ⇒ build error (payload-pool I-P4 at verify). Test: `budget_pool_invariant`.
- **I-MB5 RefuseActivation**: precommit channel failing pre-fault/lock ⇒ node refuses active state (explicit status), never silently continues pageable. Test: `precommit_refusal`.
- **I-MB6 CheckedArithmetic**: every capacity sum/product — including `R+1` — uses `checked_add`/`checked_mul`; any overflow ⇒ `CapacityOverflow` naming the compute/channel, collected with the remaining checks (never a wrapped pass, never a short-circuit). Test: `budget_overflow` (boundary values 0, 1, `u32::MAX`, `u64::MAX` on every quantity).
- **I-MB7 PerProcessLocked**: a precommit node's own locked working set (the pages of its own mappings it touches) is checked against the lockable budget, in addition to the compute-wide shared-memory capacity; hard-RT consumers need their own page-table path prefaulted. Test: `node_locked_set`.
- **I-MB8 RouterBuffersBudgeted**: router-owned lane buffers (router-core I-RT8) are added to `worst_case` for the compute where the router runs. Test: `budget_router_buffers`.
- **I-MB9 SpatialIndexesBudgeted**: fixed cell-hash table, membership pool, object records, query scratch, and BVH banks are included for every deployed spatial index. Test: `spatial_index_memory_verified`.
- **I-MB10 LockedSpanRounding**: each region's locked cost is its page-rounded span (length rounded up to the target's declared page granularity); per-region rounding keeps the sum ≥ the true locked-page total even when regions share OS pages. The static locked sum is a conservative estimate — exact residency is proven at activation (I-MB5) and measured live by `drm-diag`; static checks never model OS virtual-memory policy (architecture §Memory Budgets). Test: `locked_span_rounding`.
- **I-MB11 DropFreeShortfall**: the verifier computes `H_topic = Σ declared borrowers + diagnostic-consumer range` (payload-pool.md proof step 0) and emits a **warning** (never an error) when `pool_slots < R + H_topic + 1`, naming the shortfall; `R+1` remains the hard minimum (I-MB4). A shortfall means the drop-free guarantee is unproven, not that the configuration is invalid. Test: `budget_dropfree_warning` (REQ-CORE-042).

## Functions

```rust
// ---- Verifier (build time) ----
fn verify_memory(model: &WorkspaceModel, deploy: &DeploymentConfig) -> MemoryReport {
 let mut r = MemoryReport::default(); // no Result: collect ALL errors per run (verify-engine I-V5)
 for compute in model.computes() {
  let budget = deploy.memory_budgets[compute];
  let mut worst = 0u64; let mut locked = 0u64; let mut nominal = 0u64;
  for ch in active_channels(model, compute) {
  // I-MB4/I-MB6: R+1 itself is checked arithmetic — never a raw `+ 1`
  let Some(min_pool) = ch.history_depth.checked_add(1)
   else { r.errors.push(CapacityOverflow { compute, channel: ch.id }); continue; };
  if ch.pool_slots < min_pool { r.errors.push(PoolInvariantViolation(ch)); } // I-MB4
  // I-MB11: drop-free is a warning, not a gate (R+1 is the floor). H is static.
  let h = declared_borrowers(model, ch).saturating_add(diagnostic_range(ch));
  let Some(dropfree_pool) = ch.history_depth.checked_add(h).and_then(|x| x.checked_add(1))
   else { r.errors.push(CapacityOverflow { compute, channel: ch.id }); continue; };
  if ch.pool_slots < dropfree_pool {
   r.warnings.push(DropFreeShortfall { channel: ch.id, have: ch.pool_slots, need: dropfree_pool });
  }
  let ch_bytes = match ch.pool_slots.checked_mul(ch.slot_bytes) // I-MB6
   .and_then(|b| b.checked_add(ch.control_block)) {
   Some(b) => b,
   None => { r.errors.push(CapacityOverflow { compute, channel: ch.id }); continue; } // collect, don't abort
  };
  worst = worst.checked_add(ch_bytes) // I-MB6
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute, channel: ch.id }); worst });
  if ch.precommit { // I-MB2: per channel; I-MB10: page-rounded span, conservative
  locked = locked.checked_add(page_round_up(ch_bytes, budget.page_granularity))
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute, channel: ch.id }); locked });
  }
  nominal = nominal.checked_add(nominal_estimate(ch))
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute, channel: ch.id }); nominal });
  }
  // I-MB8: router-owned lane buffers on this compute
  let rb = deploy.router_buffers(compute);
  worst = worst.checked_add(rb)
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute }); worst });
  if budget.precommit { // I-MB2/I-MB10: router lanes lock too on precommit computes
  locked = locked.checked_add(page_round_up(rb, budget.page_granularity))
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute }); locked });
  }
  // I-MB9: cell-hash table + membership pool + object records + BVH banks + query scratch — declared capacity only
  let si = spatial_index_cost(model, compute);
  worst = worst.checked_add(si)
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute }); worst });
  if budget.precommit { // I-MB2/I-MB10: spatial banks/scratch lock too
  locked = locked.checked_add(page_round_up(si, budget.page_granularity))
   .unwrap_or_else(|| { r.errors.push(CapacityOverflow { compute }); locked });
  }

  if worst > budget.drm_budget_bytes { r.errors.push(BudgetExceeded { compute, worst, budget }); } // I-MB2
  if locked > budget.lockable_budget_bytes { r.errors.push(LockableExceeded { compute, locked, budget }); } // I-MB2
  else if worst > budget.physical_ram_bytes { r.warnings.push(OverPhysicalRam { compute, worst }); } // I-MB3
  r.estimates.push(ComputeMemoryReport { compute, worst, locked, nominal }); // informational only
 }
 // I-MB7: per-process locked working set (each precommit node's own mapping pages)
 for node in model.precommit_nodes() {
  let own = node_locked_working_set(model, node);
  let budget = deploy.memory_budgets[node.compute];
  if own > budget.lockable_budget_bytes { r.errors.push(NodeLockableExceeded { node, own, budget }); }
 }
 r // V-MEM gate (verify-engine.md): r.errors.is_empty() ⇒ pass; any error ⇒ build failure
}

// ---- Activation (runtime, per precommit node mapping) ----
fn activate_precommitted(mapping: &Mapping, budget: &MemoryBudget) -> Result<(), Status> {
 // One backend-defined proof — NOT a generic two-step. Kernels do not expose identical concepts:
 //   Linux:   mlock(range) success IS the proof (all intersecting pages resident until munlock);
 //            mincore is a staleable snapshot and is never part of the proof.
 //   Windows: committed section + VirtualLock(range); QueryWorkingSetEx Valid|Locked bits are an
 //            optional diagnostic (windows-backends.md). Reservation-only ranges are invalid input.
 lock_backend.precommit_and_prove(mapping)?; // I-MB5: failure ⇒ caller refuses active state
 Ok(())
}
```

## Protocols

*Precommit activation (node startup):*

1. Verifier passed statically (backend capability + lockable budget coverage + per-process locked set — I-MB2/I-MB7).
2. Map segment (shm-backend).
3. `precommit_and_prove`: Linux — `mlock` success is the residency proof (no `mincore` dependency); Windows — committed pages + `VirtualLock`, optional `QueryWorkingSetEx` diagnostic. Never touch payload bytes merely to fault pages.
4. Any proof failure ⇒ `LockFailed` and refuse active state.
5. DRM never decommits or tracks pages afterward (platform-managed backing; architecture §Platform-Managed Paging).

*Reference cross-check protocol (I-MB1):*

1. E1 derives expected worst-case for 5 fixture deployments by hand/independent script.
2. E2 implements verifier math; outputs must equal references exactly (bytes).
3. Disagreement ⇒ fix before Phase 19 gate.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Worst-case ≤ budget but > physical RAM | strong warning with numbers (I-MB3); deployment allowed |
| Capacity sum overflows `u64` | `CapacityOverflow` collected with other errors (I-MB6) — never a wrapped pass, never a short-circuit |
| `history_depth + 1` overflow | `CapacityOverflow` — `R+1` is checked arithmetic (I-MB4/I-MB6) |
| Channel `precommit`, compute flag false | lockable check still applies (I-MB2) |
| Precommit compute with router/spatial regions | their page-rounded spans enter `locked` (I-MB2/I-MB10) |
| Regions packed into one OS mapping share pages | per-region page-rounded sums stay ≥ true locked pages — conservative (I-MB10) |
| 4097-byte span on 4 KiB pages | counted as 2 pages (I-MB10 `locked_span_rounding`) |
| Per-node locked set > lockable budget | `NodeLockableExceeded` (I-MB7) |
| Router buffers on compute | counted in worst-case (I-MB8) and `locked` when precommit |
| `precommit: true` on backend without locking | build error (capability check) |
| Lock succeeds but budget accounting drifts at runtime | `drm-diag` measures RSS/commit live (architecture: runtime reality) — no static guess |
| pool invariant discovered at attach (stale binary) | `InvalidConfig` (channel-metadata) — belt-and-braces after verify |
| `pool_size < R+H+1` (H from declared borrowers + diagnostic range) | warning `DropFreeShortfall` (I-MB11); valid, but drop-free is unproven |
| Nominal estimate exceeding worst case | impossible by construction (nominal ≤ worst); assert in tests |

## Test Matrix

| Case | Expected |
|---|---|
| `budget_reference_match` (5 fixtures) | I-MB1 exact |
| `spatial_index_memory_verified` (index.drm at max_objects) | I-MB9: cell hash + membership pool + BVH in budget |
| `budget_hard_errors` (budget/lockable exceeded) | I-MB2 |
| `budget_overflow` (0, 1, `u32::MAX`, `u64::MAX` on every quantity) | I-MB6 `CapacityOverflow` collected, no wrap-pass |
| `budget_warning_band` | I-MB3 |
| `budget_pool_invariant` | I-MB4 |
| `budget_dropfree_warning` (R+H+1 shortfall, incl. borrowers>0) | I-MB11 warning, not error |
| `precommit_refusal` (injected failure) | I-MB5 |
| `node_locked_set` (per-process mapping) | I-MB7 |
| `budget_router_buffers` (incl. precommit compute) | I-MB8 + locked coverage |
| `locked_span_rounding` (4097 B span; shared-page packing) | I-MB10 conservative page-rounded sums |
| `precommit_success` (adequate budget) | locked + active |
| nominal ≤ worst-case property | holds |
