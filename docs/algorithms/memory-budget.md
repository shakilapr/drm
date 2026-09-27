# Memory Budget & Precommit Algorithms

Scope: deterministic memory verification against configured deployment budgets — overflow-checked worst-case capacity calculation, budget comparison rules, warning band, per-channel lockable accounting, per-process locked working set — and the hard-real-time precommit protocol (pre-fault + memory-lock, refuse-on-failure at activation). Implements architecture.md §Memory Budgets as Build-Time Verification, §Hard Real-Time Precommitment, §Payload Pool Invariant (sizing rules). Owner: E2 (verifier math in `drm-workspace::verifier`) + E1 (activation in `drm-core::channel`). Phase 19. Depends on [payload-pool.md](runtime/payload-pool.md) (invariant), [channel-metadata.md](runtime/channel-metadata.md) (attach check), [shm-backend.md](runtime/shm-backend.md) (platform locking), [router-core.md](router/router-core.md) (lane buffers).

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
}

 pub struct ChannelCost { // per active channel on a compute unit
  pool_slots: u64, // validated ≥ R+1 (payload-pool I-P4)
  slot_bytes: u64, // capacity + variable header
  control_block: u64, // CCB, hazard array, ring — fixed overhead
  precommit: bool, // drives the lockable sum (per channel, not per compute)
 }

 pub struct SpatialIndexCost { // per compute unit with declared spatial indexes (spatial-index.md I-X9)
  grid_buckets: u64, // fixed open-addressing buckets from index.drm capacity
  grid_slots: u64, // entity slots at max_objects × slack span factor
  bvh_nodes: u64, // compiled BVH node array (fixed layout)
  // no heap growth is possible — capacity is declared; verified exactly like channel buffers
 }
```

## Invariants

- **I-MB1 WorstCaseExact**: `worst_case(compute) = Σ_ch (pool_slots × slot_bytes + control_block) + router_buffers + spatial_index_memory` over active channels — matches independently derived reference values on ≥ 5 fixture deployments. Test: `budget_reference_match`.
- **I-MB2 HardErrors**: worst-case > `drm_budget` ⇒ build error; `Σ locked-bytes over channels with ch.precommit == true` > `lockable_budget` ⇒ build error. The lockable check is driven by the **channel** precommit flag, never gated on the compute-level `budget.precommit`. Test: `budget_hard_errors` (negative matrix).
- **I-MB3 WarningBand**: worst-case > physical RAM ∧ ≤ drm_budget ⇒ strong warning (not error), names the overflow amount. Test: `budget_warning_band`.
- **I-MB4 PoolInvariant**: `pool_size < R+1` ⇒ build error (payload-pool I-P4 at verify). Test: `budget_pool_invariant`.
- **I-MB5 RefuseActivation**: precommit channel failing pre-fault/lock ⇒ node refuses active state (explicit status), never silently continues pageable. Test: `precommit_refusal`.
- **I-MB6 CheckedArithmetic**: every capacity sum/product uses `checked_add`/`checked_mul`; any overflow ⇒ `CapacityOverflow` error naming the compute/channel (a `u64` wrap must never make a huge invalid config pass). Test: `budget_overflow`.
- **I-MB7 PerProcessLocked**: a precommit node's own locked working set (the pages of its own mappings it touches) is checked against the lockable budget, in addition to the compute-wide shared-memory capacity; hard-RT consumers need their own page-table path prefaulted. Test: `node_locked_set`.
- **I-MB8 RouterBuffersBudgeted**: router-owned lane buffers (router-core I-RT8) are added to `worst_case` for the compute where the router runs. Test: `budget_router_buffers`.

## Functions

```rust
// ---- Verifier (build time) ----
fn verify_memory(deploy: &DeploymentConfig, model: &WorkspaceModel) -> Result<Report, Vec<VerifyError>> {
 let mut errs = Vec::new();
 for compute in model.computes() {
 let budget = deploy.memory_budgets[compute];
 let mut worst = 0u64; let mut locked = 0u64; let mut nominal = 0u64;
 for ch in active_channels(model, compute) {
 if ch.pool_size < ch.history_depth + 1 { errs.push(PoolInvariantViolation(ch)); } // I-MB4
 let ch_bytes = ch.pool_slots.checked_mul(ch.slot_bytes) // I-MB6
 .and_then(|b| b.checked_add(ch.control_block))
 .ok_or_else(|| CapacityOverflow { compute, channel: ch.id })?;
 worst = worst.checked_add(ch_bytes).ok_or_else(|| CapacityOverflow { compute, .. })?;
 if ch.precommit { // I-MB2: per channel
 locked = locked.checked_add(ch_bytes).ok_or_else(|| CapacityOverflow { compute, .. })?;
 }
 nominal = nominal.checked_add(nominal_estimate(ch)).ok_or_else(|| CapacityOverflow { compute, .. })?;
 }
 // I-MB8: router-owned lane buffers on this compute
 worst = worst.checked_add(deploy.router_buffers(compute)).ok_or_else(|| CapacityOverflow { compute, .. })?;
 // I-MB9: spatial index memory (grid buckets + entity slots at max_objects + compiled BVH) — declared capacity only
 worst = worst.checked_add(spatial_index_cost(model, compute)).ok_or_else(|| CapacityOverflow { compute, .. })?;

 if worst > budget.drm_budget_bytes { errs.push(BudgetExceeded { compute, worst, budget }); } // I-MB2
 if locked > budget.lockable_budget_bytes { errs.push(LockableExceeded { compute, locked, budget }); } // I-MB2
 else if worst > budget.physical_ram_bytes { warn(OverPhysicalRam { compute, worst }); } // I-MB3
 }
 // I-MB7: per-process locked working set (each precommit node's own mapping pages)
 for node in model.precommit_nodes() {
 let own = node_locked_working_set(model, node);
 let budget = deploy.memory_budgets[node.compute];
 if own > budget.lockable_budget_bytes { errs.push(NodeLockableExceeded { node, own, budget }); }
 }
 Ok(report(nominal_informational, errs))
}

// ---- Activation (runtime, per precommit node mapping) ----
fn activate_precommitted(mapping: &Mapping, budget: &MemoryBudget) -> Result<(), Status> {
 if !lock_backend.lock_and_populate(mapping) { // mlock / VirtualLock; no payload reads or writes
 return Err(LockFailed); // I-MB5: caller refuses active state
 }
 lock_backend.verify_resident(mapping)?; // mincore / QueryWorkingSetEx
 Ok(())
}
```

## Protocols

*Precommit activation (node startup):*

1. Verifier passed statically (backend capability + lockable budget coverage + per-process locked set — I-MB2/I-MB7).
2. Map segment (shm-backend).
3. Use the backend's eager lock/populate operation; never touch payload bytes merely to fault pages.
4. Verify every page resident; any failure ⇒ `LockFailed` and refuse active state.
5. DRM never decommits or tracks pages afterward (platform-managed backing; architecture §Platform-Managed Paging).

*Reference cross-check protocol (I-MB1):*

1. E1 derives expected worst-case for 5 fixture deployments by hand/independent script.
2. E2 implements verifier math; outputs must equal references exactly (bytes).
3. Disagreement ⇒ fix before Phase 19 gate.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Worst-case ≤ budget but > physical RAM | strong warning with numbers (I-MB3); deployment allowed |
| Capacity sum overflows `u64` | `CapacityOverflow` error (I-MB6) — never a wrapped pass |
| Channel `precommit`, compute flag false | lockable check still applies (I-MB2) |
| Per-node locked set > lockable budget | `NodeLockableExceeded` (I-MB7) |
| Router buffers on compute | counted in worst-case (I-MB8) |
| `precommit: true` on backend without locking | build error (capability check) |
| Lock succeeds but budget accounting drifts at runtime | `drm-diag` measures RSS/commit live (architecture: runtime reality) — no static guess |
| pool_size < R+1 discovered at attach (stale binary) | `InvalidConfig` (channel-metadata) — belt-and-braces after verify |
| Nominal estimate exceeding worst case | impossible by construction (nominal ≤ worst); assert in tests |

## Test Matrix

| Case | Expected |
|---|---|
| udget_reference_match (5 fixtures) | I-MB1 exact |
| `spatial_index_memory_verified` (index.drm at max_objects) | I-MB9: grid buckets + slots + BVH in budget |
| `budget_hard_errors` (budget/lockable exceeded) | I-MB2 |
| `budget_overflow` (huge slot counts) | I-MB6 `CapacityOverflow` |
| `budget_warning_band` | I-MB3 |
| `budget_pool_invariant` | I-MB4 |
| `precommit_refusal` (injected failure) | I-MB5 |
| `node_locked_set` (per-process mapping) | I-MB7 |
| `budget_router_buffers` | I-MB8 |
| `precommit_success` (adequate budget) | locked + active |
| nominal ≤ worst-case property | holds |
