# Spatial Query Algorithms

Scope: spatial query surface — **compiled transform plans** per (target, reference) pair, the four query operations `latest/at/predict/delayed_view`, the transform-status result object, SLERP/linear interpolation with the discontinuity guard, **transactional temporal reparenting** (Preserve World/Local/Snap), **live calibration via calibration slots** (no recompilation), versioned per-process caches with consumer-driven time-based history. Implements [spatial_architecture.md](../../spatial_architecture.md) §Versioned Local Cache, §Query Semantics and History, §Discontinuities and Teleports, §Transactional Reparenting, §Transform Status and Quality, §Calibration Without Recompilation. Owner: E2 (query-plan compiler) + E1 (runtime). Phase 33 (implemented M20–M21). Depends on [spatial-core.md](spatial-core.md), [channel-metadata.md](../runtime/channel-metadata.md), [cursor-delivery.md](../runtime/cursor-delivery.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-transform-plans` | per-(target, reference) precompiled plan; runtime executes the plan, never walks the graph | `REQ-SPATIAL-034` |
| `REQ-SPATIAL-query-latest` | `latest(target, reference)` — newest valid state in memory | `REQ-SPATIAL-024` |
| `REQ-SPATIAL-query-at` | `at(target, reference, t)` — linear translation + SLERP rotation; never silently extrapolates | `REQ-SPATIAL-022` |
| `REQ-SPATIAL-query-predict` | `predict(target, reference, max_horizon)` — explicit forward extrapolation only | `REQ-SPATIAL-025` |
| `REQ-SPATIAL-query-delayed-view` | `delayed_view(offset)` — fixed-duration trailing view | `REQ-SPATIAL-023` |
| `REQ-SPATIAL-status-object` | result carries status ∈ {VALID, STALE, DISCONTINUOUS, CLOCK_UNRESOLVED, OUT_OF_HISTORY, EXTRAPOLATED} + sample age + clock uncertainty | `REQ-SPATIAL-032` |
| `REQ-SPATIAL-cache-versioned` | per-process cache over declared reads; version check ⇒ cached result or plan re-execution | `REQ-SPATIAL-002` |
| `REQ-SPATIAL-time-history` | rings allocated only on upstream dynamic edges with declared requirements (`history = "2s"`) | `REQ-SPATIAL-033` |
| `REQ-SPATIAL-interp-guard` | never interpolate across a discontinuity; queries spanning a jump return DISCONTINUOUS | `REQ-SPATIAL-016` |
| `REQ-SPATIAL-reparent-temporal` | reparent at t₁ never alters history before t₁; earlier queries traverse the original hierarchy | `REQ-SPATIAL-029` |
| `REQ-SPATIAL-reparent-modes` | Preserve World / Preserve Local / Snap | `REQ-SPATIAL-028` |
| `REQ-SPATIAL-reparent-atomic` | hierarchy mutations atomic — readers never observe partially updated trees | `REQ-SPATIAL-027` |
| `REQ-SPATIAL-calibration-slots` | Effective = Nominal × Correction; shared calibration slot with effective timestamp; live cache invalidation, no recompilation, no restart | `REQ-SPATIAL-003` |
| `REQ-SPATIAL-generational-handles` | runtime overlay handles generational; single-writer edges | `REQ-SPATIAL-011` |

## Data Structures

```rust
// Compiled transform plan — emitted by the compiler per (target, reference) query pair
// declared in a node's [spatial] read set.
pub enum PlanStep {
    Constant(Transform3),                       // collapsed static run (build-time)
    CalibrationSlot { nominal: Transform3, slot: CalibrationSlotId },  // Effective = Nominal × Correction
    DynamicEdge { edge: EdgeId },               // runtime cache read (version-checked)
    Multiply,                                   // fold two top-of-stack transforms
}
pub struct TransformPlan { steps: Vec<PlanStep> }   // straight-line; zero graph traversal at runtime

// Calibration slot (shared memory; updated live — spatial_architecture §Calibration Without Recompilation)
pub struct CalibrationSlot {
    version: AtomicU32,                // bump on update ⇒ dependent caches invalidate live
    correction: Transform3,            // Correction factor
    effective_ns: u64,                 // effective timestamp of the correction
}

// Versioned dynamic edge in shared memory (authoritative state)
pub struct DynamicEdge {
    version: AtomicU32,
    xform: Transform3,
    sample_ns: TimePoint,              // TimePoint { value_ns, clock_domain }
    discontinuous: bool,               // discontinuity flag (teleport/relocalize/reparent boundary)
}

// Per-process cache over declared [spatial] reads
pub struct PlanCache {
    plan: *const TransformPlan,        // compiled plan (static)
    cached: Transform3,
    cached_version_stamp: u64,         // tuple of observed edge versions + calibration versions
}

// Time-based history — allocated ONLY on dynamic edges with declared consumer requirements
pub struct TimeHistory {
    ring: RingBuf<Stamped<Transform3>>,     // capacity = ceil(duration × expected_rate)
    duration_ns: u64,                       // e.g. history = "2s" from consumer declarations
}

// Query result — §Transform Status and Quality (exact status set)
pub enum TransformStatus { Valid, Stale, Discontinuous, ClockUnresolved, OutOfHistory, Extrapolated }
pub struct TransformResult {
    status: TransformStatus,
    transform: Option<Transform3>,
    sample_age_ns: u64,                      // age at query time
    clock_uncertainty_ns: u64,               // from clock mapping estimates
}

pub enum ReparentMode { PreserveWorld, PreserveLocal, Snap }
```

## Invariants

- **I-Q1 PlanExecution**: runtime query = execute the compiled plan (multiply steps + cache reads); graph traversal happens only at compile time. Test: `spatial_plan_execution` (no traversal instrumented).
- **I-Q2 CacheVersionShortcircuit**: version stamp unchanged ⇒ cached transform returned with zero multiplications; any version/calibration change ⇒ plan re-executes. Test: `spatial_cache_shortcircuit`.
- **I-Q3 LatestNewest**: `latest(target, reference)` returns the newest valid state currently held. Test: `spatial_query_latest`.
- **I-Q4 AtExactOrInterpolated**: `at(t)` returns exact sample at t; otherwise linear translation + SLERP rotation between bracketing samples; **never silently extrapolates beyond the newest sample** (`OUT_OF_HISTORY`). Test: `spatial_query_at`.
- **I-Q5 PredictExplicitOnly**: forward extrapolation exists only in `predict`, bounded by `max_horizon`; beyond ⇒ `EXTRAPOLATED` status. Test: `spatial_query_predict`.
- **I-Q6 DelayedViewBound**: `delayed_view(offset)` resolves `now − offset` through `at`; offset beyond retained history ⇒ `OUT_OF_HISTORY`. Test: `spatial_delayed_view`.
- **I-Q7 DiscontinuityGuard**: samples can be flagged discontinuous; interpolation never spans a flagged boundary — queries spanning it return `DISCONTINUOUS`, never an invalid intermediate path. Test: `spatial_interp_guard`.
- **I-Q8 ReparentTemporal**: reparent at t₁ leaves history before t₁ untouched; earlier queries traverse the original hierarchy (plan versions are time-scoped). Test: `spatial_reparent_temporal`.
- **I-Q9 ReparentModes**: Preserve World keeps global pose; Preserve Local keeps local offset; Snap pins to new parent origin — all oracle-verified. Test: `spatial_reparent_modes`.
- **I-Q10 ReparentAtomic**: hierarchy mutations execute atomically; readers never observe partially updated trees. Test: `spatial_reparent_atomic`.
- **I-Q11 CalibrationLive**: calibration-slot update invalidates dependent caches live — no recompilation, no node restart; Effective = Nominal × Correction composed via the plan's `CalibrationSlot` step. Test: `spatial_calibration_live`.
- **I-Q12 GenerationalEdges**: overlay edge handles generational; delete invalidates immediately; single-writer per dynamic edge. Test: `spatial_generational_edges`.
- **I-Q13 HistoryTimeSized**: history rings exist only where consumers declared requirements; capacity from duration × rate; static constants never stored across timestamps. Test: `spatial_history_sizing`.

## Functions

```rust
// ---- Plan compilation (build time, per declared read pair) ----
fn compile_plan(tree: &SpaceTree, target: FrameId, reference: FrameId) -> Result<TransformPlan, Status> {
    let path = tree.path_between(reference, target)?;      // compile-time only (forest-aware)
    let mut steps = vec![];
    for seg in path.segments() {
        match seg.kind {
            Static => steps.push(Constant(seg.collapsed())),                    // static run (pre-collapsed)
            StaticWithCalibration { slot } => steps.push(CalibrationSlot { nominal: seg.nominal(), slot }),
            Dynamic { edge } => steps.push(DynamicEdge { edge }),
        }
        steps.push(Multiply);
    }
    Ok(TransformPlan { steps })
}

// ---- Runtime execution ----
fn execute(plan: &TransformPlan, caches: &PlanCaches) -> Result<Transform3, Status> {
    let mut stack = Vec::with_capacity(4);                  // bounded by plan depth (static)
    for step in &plan.steps {
        match step {
            Constant(t) => stack.push(*t),
            CalibrationSlot { nominal, slot } => {
                let c = slot.load();                        // version-checked read
                stack.push(c.correction * nominal);         // Effective = Correction × Nominal
            }
            DynamicEdge { edge } => stack.push(caches.edge_value(edge)?),   // version-checked cache
            Multiply => { let b = stack.pop()?; let a = stack.pop()?; stack.push(a * b); }
        }
    }
    Ok(stack.pop()?)
}

fn query(&self, target: FrameId, reference: FrameId, t: QueryTime) -> TransformResult {
    let plan = self.plan_for(target, reference);            // compiled once per (target, reference)
    if self.cache.fresh(plan.versions(), self.calibration_versions()) {
        return result(Valid, self.cache.cached);            // I-Q2 shortcircuit
    }
    let x = execute(plan, &self.caches);
    self.cache.update(x, versions);
    classify(x, t)                                          // VALID | STALE | DISCONTINUOUS | CLOCK_UNRESOLVED
}

// ---- Interpolation (at) ----
fn at(&self, target: FrameId, reference: FrameId, t_ns: TimePoint) -> TransformResult {
    let hist = self.history_for(target, reference)?;        // time-based ring (I-Q13)
    let (s0, s1) = match hist.bracket(t_ns)? {
        Exact(s) => return result(Valid, s),
        Bracket(a, b) => (a, b),
        None => return result(OutOfHistory, None),          // never silently extrapolate (I-Q4)
    };
    if s0.discontinuous || s1.discontinuous || s0.sample_ns == boundary_of_jump {
        return result(Discontinuous, None);                 // I-Q7: never interpolate across the jump
    }
    let tp = map_timestamp_pair(s0.sample, s1.sample, t_ns)?;   // CLOCK_UNRESOLVED possible (I-Q spatial-core)
    result(Valid, interp_linear_translation(s0, s1, tp)
              .with_rotation(slerp(s0.rotation, s1.rotation, tp)))
}

// ---- Transactional temporal reparenting ----
fn reparent(&self, obj: FrameId, new_parent: FrameId, mode: ReparentMode, at_ns: TimePoint) -> Result<(), Status> {
    let txn = self.tree.begin_atomic();                     // readers never see partial trees (I-Q10)
    let old_pose = self.at(obj, WORLD, at_ns)?;             // pose at the reparent instant
    match mode {
        PreserveWorld => txn.relink_preserving_world(obj, new_parent, old_pose),   // adjust local edge
        PreserveLocal => txn.relink_keep_local(obj, new_parent),                   // global pose changes
        Snap          => txn.relink_snap(obj, new_parent),                         // pin to parent origin
    }
    txn.mark_discontinuity(obj, at_ns);                     // interpolation guard boundary (I-Q7)
    txn.commit()?;                                          // atomic: version bump + plan invalidation together
    Ok(())
}
// Temporality (I-Q8): plans are versioned per validity interval; `at(t < t₁)` executes the
// ORIGINAL-hierarchy plan (retained via the edge's time-scoped plan registry), `at(t ≥ t₁)`
// executes the new-hierarchy plan. History before t₁ is never rewritten.
```

## Protocols

*Plan lifecycle:*

1. **Compile**: for every `(target, reference)` pair in the workspace's declared `[spatial] read` sets, compile one plan (static runs collapsed; calibration slots and dynamic edges as steps).
2. **Deploy**: plans ship in codegen artifacts; per-process `PlanCaches` attach to edge segments (channel-metadata validation).
3. **Execute**: version-check → cached value, else straight-line execution (I-Q1).
4. **Invalidate live**: calibration-slot version bump or edge version change ⇒ next query re-executes (I-Q11) — no recompilation, no restart.

*Transactional reparenting (§Transactional Reparenting):*

1. Begin atomic tree transaction (I-Q10).
2. Resolve pose at the reparent instant via `at` (temporal — history preserved).
3. Apply mode (I-Q9); mark discontinuity at t₁.
4. Commit: new dynamic-edge version + time-scoped plan registry entry (old plan retained for pre-t₁ queries) — I-Q8.

*Live calibration:*

1. Calibration writer updates the shared `CalibrationSlot` (correction + effective timestamp), bumps slot version.
2. Consumer caches observe version change on next query ⇒ re-execute only plans containing that slot.
3. Update also flags a discontinuity when the correction jump exceeds the declared threshold.

## Edge Cases & Errors

| Case | Result |
|---|---|
| `at(t)` before first sample | `OUT_OF_HISTORY` (or Hold-to-first per declared policy) |
| `at(t)` beyond newest sample | `OUT_OF_HISTORY` — `predict` is the only extrapolation path (I-Q4/I-Q5) |
| `predict` beyond max_horizon | `EXTRAPOLATED` status with bounded value (I-Q5) |
| Query spanning discontinuity | `DISCONTINUOUS` status (I-Q7) |
| Timestamp without valid clock mapping | `CLOCK_UNRESOLVED` (I-S7, spatial-core) |
| Query across disconnected trees | `PathNotFound` (spatial-core I-S3) |
| Stale sample past declared freshness | `STALE` status (timing contracts) |
| Old overlay handle after `space.delete` | generation mismatch ⇒ invalid-handle status (spatial-core I-S12) |
| Reparent racing concurrent query | atomic commit ⇒ query sees pre- or post-tree, never partial (I-Q10) |
| `delayed_view(δ)` with δ > retained history | `OUT_OF_HISTORY` (I-Q6) |
| Near-antipodal quaternion slerp | documented stable path + oracle test |

## Test Matrix

| Case | Expected |
|---|---|
| `spatial_plan_execution` (instrumented) | I-Q1: zero traversal at runtime |
| `spatial_cache_shortcircuit` | I-Q2 |
| `spatial_query_latest/at/predict/delayed_view` | I-Q3..I-Q6 vs oracle |
| `spatial_interp_guard` (relocalize + reparent + calibration-jump fixtures) | I-Q7 |
| `spatial_reparent_temporal` (query t < t₁) | I-Q8: original hierarchy |
| `spatial_reparent_modes` ×3 vs oracle | I-Q9 |
| `spatial_reparent_atomic` (concurrent readers) | I-Q10 |
| `spatial_calibration_live` (slot bump → cache invalidation) | I-Q11 |
| `spatial_generational_edges` | I-Q12 |
| `spatial_history_sizing` (`history = "2s"` declarations) | I-Q13 |
| interpolation numerics vs oracle (slerp/linear × rates) | exactness |
