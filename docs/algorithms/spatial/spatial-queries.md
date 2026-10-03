# Spatial Query Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: spatial query surface — **compiled transform plans** per (target, reference) pair, the four query operations `latest/at/predict/delayed_view`, the transform result object (hard error + combinable quality flags), SLERP/linear interpolation with the discontinuity guard, **transactional temporal reparenting** (Preserve World/Local/Snap), **live calibration via calibration slots** (no recompilation), versioned per-process caches with consumer-driven time-based history. Implements [spatial_architecture.md](../../spatial_architecture.md) §Versioned Local Cache, §Query Semantics and History, §Discontinuities and Teleports, §Transactional Reparenting, §Transform Status and Quality, §Calibration Without Recompilation. Owner: E2 (query-plan compiler) + E1 (runtime). Phase 33 (implemented M20–M21). Depends on [spatial-core.md](spatial-core.md), [channel-metadata.md](../runtime/channel-metadata.md), [cursor-delivery.md](../runtime/cursor-delivery.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-transform-plans` | per-(target, reference) precompiled plan; runtime executes the plan; overlay frames add only a bounded, topology-generation-cached overlay prefix — never an unbounded walk | `REQ-SPATIAL-034` |
| `REQ-SPATIAL-query-latest` | `latest(target, reference)` — transform at the newest common coherent timestamp t* = min_i(t_newest,i); every dynamic input evaluated at t* | `REQ-SPATIAL-024` |
| `REQ-SPATIAL-query-at` | `at(target, reference, t)` — linear translation + SLERP rotation between brackets in the sample-time-ordered history; never silently extrapolates | `REQ-SPATIAL-022` |
| `REQ-SPATIAL-query-predict` | `predict(target, reference, max_horizon)` — explicit forward extrapolation only; body-frame twist integrated on SE(3) via the exponential map | `REQ-SPATIAL-025` |
| `REQ-SPATIAL-query-delayed-view` | `delayed_view(offset)` — fixed-duration trailing view | `REQ-SPATIAL-023` |
| `REQ-SPATIAL-status-object` | result carries a transform or one hard error (PATH_NOT_FOUND \| CLOCK_UNRESOLVED \| OUT_OF_HISTORY \| CONCURRENT_UPDATE) + combinable quality flags (STALE \| EXTRAPOLATED \| DISCONTINUITY_NEARBY) + sample time, receive age, clock uncertainty | `REQ-SPATIAL-032` |
| `REQ-SPATIAL-cache-versioned` | per-process cache over declared reads; version equality reuses cached geometry only — freshness/status always derived per query, never cached | `REQ-SPATIAL-002` |
| `REQ-SPATIAL-time-history` | rings allocated only on upstream dynamic edges with declared requirements; ordered by sample time; in-window late arrivals insert in order | `REQ-SPATIAL-033` |
| `REQ-SPATIAL-interp-guard` | never interpolate across a discontinuity; cross-jump queries surface DISCONTINUITY_NEARBY (no cross-jump value); discontinuities marked per jumped local edge | `REQ-SPATIAL-016` |
| `REQ-SPATIAL-reparent-temporal` | reparent at t₁ never alters history before t₁; earlier queries traverse the original hierarchy; topology-event history bounded by the retrievable transform-history horizon | `REQ-SPATIAL-029` |
| `REQ-SPATIAL-reparent-modes` | Preserve World / Preserve Local / Snap | `REQ-SPATIAL-028` |
| `REQ-SPATIAL-reparent-atomic` | hierarchy mutations atomic — readers never observe partially updated trees | `REQ-SPATIAL-027` |
| `REQ-SPATIAL-calibration-slots` | Effective = Nominal × Correction; shared calibration slot with effective timestamp; live cache invalidation, no recompilation, no restart | `REQ-SPATIAL-003` |
| `REQ-SPATIAL-generational-handles` | runtime overlay handles generational; single-writer edges | `REQ-SPATIAL-011` |
| `REQ-SPATIAL-history-bound` | `history = H` guaranteed sample-time window; `max_samples = N` fixed capacity; `min_sample_period = P` producer-density contract; verify `N ≥ ⌊H/P⌋ + 2`; `expected_rate` never sizes correctness storage | `REQ-SPATIAL-037` |

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
pub struct TransformPlan { steps: &'static [PlanStep], max_stack: u16 } // generated fixed artifact

pub struct AtomicTransformSlot {
    version: AtomicU64,                 // odd while writing, even when stable
    fields: [AtomicU64; 7],             // f64 bits: translation + quaternion
    sample_ns: AtomicU64,               // sample time; sample ClockIdentity (domain+epoch) tracked per edge metadata
    receive_ns: AtomicU64,              // host-monotonic receive time (I-S6, staleness)
    flags: AtomicU32,
}
// A device reset bumps the edge's clock epoch; samples written after the bump never resolve
// against a mapping of the previous incarnation (I-S7).

// Calibration slot (shared memory; updated live — spatial_architecture §Calibration Without Recompilation)
pub struct CalibrationSlot {
    value: AtomicTransformSlot,
    history: Option<FixedTimeRing>,     // generated only for declared historical reads
}

// Versioned dynamic edge in shared memory (authoritative state)
pub struct DynamicEdge {
    value: AtomicTransformSlot,
    history: Option<FixedTimeRing>,
}

// Per-process cache over declared [spatial] reads — geometry only.
// Freshness/staleness/uncertainty are ALWAYS derived per query from timestamps;
// version equality never implies VALID (REQ-SPATIAL-002).
pub struct PlanCache {
    plan: *const TransformPlan,        // compiled plan (static)
    cached: Transform3,
    coherent_sample_time: TimePoint,           // t* of the cached composition
    observed_versions: [u64; MAX_PLAN_INPUTS], // exact generated vector; no hash collision
    input_count: u16,
}

// Time-based history — allocated ONLY on dynamic edges with declared consumer requirements
pub struct TimeHistory {
    ring: FixedRing<Stamped<Transform3>>,   // allocated at activation; ORDERED BY SAMPLE TIME
    duration_ns: u64,                       // H — guaranteed sample-time queryability window
    max_samples: u32,                       // N — structural memory bound; verify N >= floor(H/P) + 2
    min_sample_period_ns: u64,              // P — producer-density contract (correctness)
    late_dropped: AtomicU64,                // LateSampleDropped counter (diagnostic)
}

// Query result — hard outcome + combinable quality flags (independent properties)
pub enum TransformError { PathNotFound, ClockUnresolved, OutOfHistory, ConcurrentUpdate }
pub struct QualityFlags(u32);               // STALE | EXTRAPOLATED | DISCONTINUITY_NEARBY
pub struct TransformResult {
    transform: Option<Transform3>,
    error: Option<TransformError>,
    flags: QualityFlags,
    sample_time: Option<TimePoint>,          // t* for latest; t for at
    sample_age_ns: Option<u64>,              // vs query-clock now
    receive_age_ns: Option<u64>,             // staleness
    clock_uncertainty_ns: u64,               // from clock mapping estimates
}

pub enum ReparentMode { PreserveWorld, PreserveLocal, Snap }
```

## Invariants

- **I-Q1 PlanExecution**: runtime query = execute the compiled plan (multiply steps + cache reads); graph traversal happens only at compile time, plus the bounded overlay walk for overlay frames (I-Q16). Test: `spatial_plan_execution` (no traversal instrumented).
- **I-Q2 CacheGeometryOnly**: exact input-version vector unchanged ⇒ reuse cached transform geometry with zero multiplications; freshness/staleness/uncertainty are still derived per query from timestamps — version equality never implies `VALID`; any input change ⇒ re-execute. Test: `spatial_cache_shortcircuit`, `spatial_cache_aging`.
- **I-Q3 LatestCoherent**: `latest()` evaluates every dynamic input at one common timestamp `t* = min_i(t_newest,i)` (after clock conversion) — the composed transform corresponds to a single physical instant, reported as `sample_time`. If the input intervals have no common intersection (`max_i t_oldest,i > min_i t_newest,i`) ⇒ `OUT_OF_HISTORY`. Test: `spatial_query_latest` (asynchronous rates + races vs single-instant oracle).
- **I-Q4 AtExactOrInterpolated**: `at(t)` returns exact sample at t; otherwise linear translation + SLERP rotation between bracketing samples of the sample-time-ordered ring; **never silently extrapolates beyond the newest sample** (`OUT_OF_HISTORY`). Test: `spatial_query_at`.
- **I-Q5 PredictExplicitOnly**: only `predict` extrapolates; within the declared horizon it sets `EXTRAPOLATED`, beyond it returns `OUT_OF_HISTORY`. Extrapolation integrates the body-frame twist on SE(3) via the exponential map, `T(t+Δ) = T(t)·Exp(ξ_b Δ)`; twist frames convert via the SE(3) adjoint. Test: `spatial_query_predict`, `spatial_predict_screw` (screw-motion oracle).
- **I-Q6 DelayedViewBound**: `delayed_view(offset)` resolves `now − offset` through `at`; offset beyond retained history ⇒ `OUT_OF_HISTORY`. Test: `spatial_delayed_view`.
- **I-Q7 DiscontinuityGuard**: samples can be flagged discontinuous; interpolation never spans a flagged boundary — cross-jump queries surface `DISCONTINUITY_NEARBY` with no cross-jump value (t within the jump gap ⇒ `OUT_OF_HISTORY` + flag), never an invalid intermediate path. Test: `spatial_interp_guard`.
- **I-Q8 ReparentTemporal**: reparent at t₁ leaves history before t₁ untouched; earlier queries traverse the original hierarchy (plan versions are time-scoped); the topology-event history is bounded — epochs older than the retrievable transform-history horizon are reclaimable. Test: `spatial_reparent_temporal`.
- **I-Q9 ReparentModes**: Preserve World keeps the global pose **continuous** (local edge recomputed as `T_anchor,newParent⁻¹ · T_anchor,obj` at t₁); Preserve Local keeps local offset; Snap pins to new parent origin — all oracle-verified; discontinuities are marked on jumped local edges only. Test: `spatial_reparent_modes`, `spatial_reparent_world_continuity`.
- **I-Q10 ReparentAtomic**: hierarchy mutations execute atomically; readers never observe partially updated trees. Test: `spatial_reparent_atomic`.
- **I-Q11 CalibrationLive**: calibration-slot update invalidates dependent caches live — no recompilation, no node restart; Effective = Nominal × Correction composed via the plan's `CalibrationSlot` step. Test: `spatial_calibration_live`.
- **I-Q12 GenerationalEdges**: overlay edge handles generational; delete invalidates immediately; single-writer per dynamic edge. Test: `spatial_generational_edges`.
- **I-Q13 HistoryProvable**: `history = H` is a guaranteed sample-time window; capacity is the declared structural `max_samples = N`; the producer contract `min_sample_period = P` lets verification require `N ≥ ⌊H/P⌋ + 2` (one predecessor below the window floor) and reject unsatisfiable declarations — never silent shrinkage. At runtime a sample closer than P to the previous accepted sample is **rejected** and counted (`DensityViolation`), so the bound always holds. Rings ordered by sample time; in-window late arrivals bounded-insert; older-than-floor rejected ⇒ `LateSampleDropped` counter. `expected_rate` never sizes correctness storage. Static constants are never stored across timestamps. Test: `spatial_history_sizing`, `spatial_history_order`.
- **I-Q14 CoherentSlots**: transform fields are fixed-width atomics guarded by an odd/even version; the slot stores sample + receive timestamps; bounded snapshot failure returns `ConcurrentUpdate`, never torn geometry. The single-writer seqlock protocol is model-checked (all interleavings, x86-64 + ARM64); a fixed double-bank + atomic selector is the approved alternative where the proof does not close. Test: `spatial_slot_coherence` (model-checked).
- **I-Q15 ZeroAllocQuery**: latest/at/predict/delayed execution uses generated plans, fixed stacks, and preallocated histories; no query-time heap allocation. Test: `spatial_query_zero_alloc`.
- **I-Q16 OverlayPlans**: runtime overlay frames resolve via a bounded overlay walk (≤ `max_overlay_depth` parent steps) to a compiled-base attachment frame, then the precompiled base plan; the prefix is cached under `topology_generation`. Test: `spatial_overlay_plan_bound`.

## Functions

```rust
// ---- Plan compilation (build time, per declared read pair) ----
fn compile_plan(tree: &SpaceTree, target: FrameId, reference: FrameId) -> Result<TransformPlan, Status> {
    let path = tree.path_between(reference, target)?;      // compile-time only (forest-aware)
    let mut steps = vec![];
    for (i, seg) in path.segments().enumerate() {
        match seg.kind {
            Static => steps.push(Constant(seg.collapsed())),                    // static run (pre-collapsed)
            StaticWithCalibration { slot } => steps.push(CalibrationSlot { nominal: seg.nominal(), slot }),
            Dynamic { edge } => steps.push(DynamicEdge { edge }),
        }
        if i != 0 { steps.push(Multiply); }                // binary fold; never underflow on first term
    }
    let max_stack = verify_stack_effect(&steps)?;          // final depth exactly one
    Ok(TransformPlan { steps, max_stack })
}

// ---- Runtime execution ----
// execute_at is the single executor: pushes plan steps onto a fixed stack, resolving each
// DynamicEdge/CalibrationSlot input through a caller-supplied temporal resolver.
fn execute_at(plan: &TransformPlan, t_ns: TimePoint,
              edge_at: impl FnMut(EdgeId) -> Result<Stamped<Transform3>, Status>,
              slot_at: impl FnMut(CalibrationSlotId) -> Result<Transform3, Status>) -> Result<Transform3, Status> {
    let mut stack = FixedStack::<Transform3, MAX_PLAN_DEPTH>::new();
    for step in &plan.steps {
        match step {
            Constant(t) => stack.push(*t),
            CalibrationSlot { nominal, slot } => {
                let c = slot_at(*slot)?;                    // effective value at/before t_ns
                stack.push(nominal * c);                    // Effective = Nominal × Correction
            }
            DynamicEdge { edge } => stack.push(edge_at(*edge)?.value),  // resolved at t_ns
            Multiply => { let b = stack.pop()?; let a = stack.pop()?; stack.push(a * b); }
        }
    }
    Ok(stack.pop()?)
}
// Convention: in query functions, `?` on a Status maps to a TransformResult carrying that
// hard error; the resolver closures also set quality flags (STALE/EXTRAPOLATED/DISCONTINUITY_NEARBY).

fn latest(&mut self, target: FrameId, reference: FrameId) -> TransformResult {
    let plan = self.plan_for(target, reference);            // compiled; overlay prefix bounded (I-Q16)
    let t_star = self.newest_common_time_bounded(plan)?;    // min_i(t_newest,i); empty intersection ⇒ OutOfHistory (I-Q3)
    let x = if self.cache.matches(plan.input_versions_at(t_star)) {
        self.cache.cached                                   // geometry reuse ONLY (I-Q2)
    } else {
        // Every dynamic input is evaluated at t*. Inputs whose newest > t* resolve through
        // their history ring; a slot-only input newer than t* cannot be evaluated at t*
        // ⇒ OutOfHistory (never silently mixed freshest values — I-Q3).
        let x = execute_at(plan, t_star,
                   |edge| edge.history_or_slot()?.value_at(t_star),
                   |slot| slot.history_value_at(t_star))?;
        self.cache.update(x, t_star);
        x
    };
    classify(x, t_star)                                     // freshness/flags derived here — never cached
}

// ---- Interpolation (at) ----
fn at(&self, target: FrameId, reference: FrameId, t_ns: TimePoint) -> TransformResult {
    let plan = self.time_scoped_plan(target, reference, t_ns)?;
    // Each dynamic edge brackets t independently in its sample-time-ordered ring; calibration
    // uses the latest effective value <= t. A missing bracket/value returns OutOfHistory; any
    // crossed discontinuity refuses interpolation with DISCONTINUITY_NEARBY (I-Q7).
    execute_at(plan, t_ns, |edge| edge.history()?.interpolate_guarded(t_ns),
               |slot| slot.history_value_at(t_ns))
}

fn delayed_view(&self, target: FrameId, reference: FrameId, offset_ns: u64) -> TransformResult {
    self.at(target, reference, mono_now().saturating_sub(offset_ns))
}

fn predict(&mut self, target: FrameId, reference: FrameId, horizon_ns: u64) -> TransformResult {
    if horizon_ns > self.declared_max_horizon(target, reference) { return result_err(OutOfHistory); }
    // Body-frame twist on SE(3): T(t+Δ) = T(t) · Exp(ξ_b Δ); twist taken at the newest
    // coherent state (= latest()); twist frame conversion via the SE(3) adjoint Ad (I-Q5).
    let state = self.newest_coherent_state(target, reference)?;
    integrate_exp_map(state.twist_body, horizon_ns).with_flags(EXTRAPOLATED)
}

// ---- Transactional temporal reparenting ----
fn reparent(&self, obj: FrameId, new_parent: FrameId, mode: ReparentMode, at_ns: TimePoint) -> Result<(), Status> {
    let txn = self.tree.begin_atomic();                     // readers never see partial trees (I-Q10)
    let anchor = txn.common_root_or_explicit_bridge(obj, new_parent)?; // forest-aware; no universal WORLD
    let old_pose = self.at(obj, anchor, at_ns)?;             // pose at the reparent instant
    match mode {
        PreserveWorld => {
            // T_newParent,obj = T_anchor,newParent⁻¹ · T_anchor,obj at t₁ — world pose continuous (I-Q9)
            let new_local = txn.compose_parent_inverse(new_parent, old_pose)?;
            txn.relink(obj, new_parent, new_local);
            txn.mark_discontinuity(jumped_local_edges(obj), at_ns);   // per-edge guard (I-Q7)
        }
        PreserveLocal => { txn.relink_keep_local(obj, new_parent); txn.mark_discontinuity(jumped_local_edges(obj), at_ns); }
        Snap          => { txn.relink_snap(obj, new_parent);       txn.mark_discontinuity(jumped_local_edges(obj), at_ns); }
    }
    txn.commit()?;                          // atomic: version bump + plan invalidation together
    Ok(())
}
// Temporality (I-Q8): plans are versioned per validity interval; `at(t < t₁)` executes the
// ORIGINAL-hierarchy plan (retained via the edge's time-scoped plan registry), `at(t ≥ t₁)`
// executes the new-hierarchy plan. History before t₁ is never rewritten. Topology events are
// retained in a bounded registry — epochs older than the retrievable transform-history horizon
// are reclaimable.
```

## Protocols

*Plan lifecycle:*

1. **Compile**: for every `(target, reference)` pair in the workspace's declared `[spatial] read` sets, compile one plan (static runs collapsed; calibration slots and dynamic edges as steps).
2. **Deploy**: plans ship in codegen artifacts; per-process `PlanCaches` attach to edge segments (channel-metadata validation).
3. **Overlay pairs**: queries naming runtime overlay frames compile a bounded overlay prefix — walk parents ≤ `max_overlay_depth` steps to the compiled-base attachment, then reuse the base plan; cache under `topology_generation`, re-walk once per bump (I-Q16).
4. **Execute**: version-check → cached geometry, else straight-line execution; freshness/status derived per query (I-Q1, I-Q2).
5. **Invalidate live**: calibration-slot version bump or edge version change ⇒ next query re-executes (I-Q11) — no recompilation, no restart.

*Slot write/snapshot:* the single writer is a serialized task (single-writer edges, REQ-SPATIAL-011). It checks `version + 2`, release-stores the odd version, writes fixed-width atomic fields (transform, sample time, receive time, flags), then release-stores the next even version. Readers load even version (Acquire) → fields → version (Acquire), accepting only an unchanged even value. Version exhaustion returns `GenerationExhausted`; bounded snapshot exhaustion returns `ConcurrentUpdate`. The protocol is model-checked over all interleavings on x86-64 and ARM64 (`spatial_slot_model`); where the proof does not close, the approved alternative is a fixed double-bank + atomic selector (the same pattern as the grid/BVH banks).

*History ingestion (sample-time ordered, REQ-SPATIAL-037):*

1. New sample with sample time `T`:
   - `T − previous accepted sample < P` ⇒ reject, count `DensityViolation` (preserves the `N ≥ ⌊H/P⌋ + 2` proof).
   - `T ≥ head` ⇒ append.
   - `T` inside the retained window ⇒ bounded ordered insert (cost ≤ N).
   - `T` equals a retained sample time ⇒ deterministic: keep the newer receive time; drop the other (counted).
   - `T` older than the retained floor ⇒ reject ⇒ `LateSampleDropped` counter.

*Transactional reparenting (§Transactional Reparenting):*

1. Begin atomic tree transaction (I-Q10).
2. Resolve pose at the reparent instant via `at` (temporal — history preserved).
3. Apply mode (I-Q9); mark discontinuity on the jumped local edges at t₁ (per edge; Preserve World keeps the world trajectory continuous).
4. Commit: new dynamic-edge version + time-scoped plan registry entry (old plan retained for pre-t₁ queries; registry bounded by the retrievable history horizon) — I-Q8.

*Live calibration:*

1. Calibration writer sets version odd, writes atomic fields/history, then release-publishes the next even version.
2. Consumer caches observe version change on next query ⇒ re-execute only plans containing that slot.
3. Update also flags a discontinuity when the correction jump exceeds the declared threshold.

## Edge Cases & Errors

| Case | Result |
|---|---|
| `at(t)` before first sample | `OUT_OF_HISTORY` |
| `latest()` with no common input interval | `OUT_OF_HISTORY` (I-Q3) |
| `at(t)` beyond newest sample | `OUT_OF_HISTORY` — `predict` is the only extrapolation path (I-Q4/I-Q5) |
| `predict` within / beyond max_horizon | `EXTRAPOLATED` flag with value / `OUT_OF_HISTORY` without value |
| Query spanning discontinuity | interpolation refused; `DISCONTINUITY_NEARBY` flag; t within the jump gap ⇒ `OUT_OF_HISTORY` + flag (I-Q7) |
| Inputs stop advancing; wall time passes | cached geometry reused; freshness re-derived ⇒ `STALE` flag — never a cached all-clear (I-Q2) |
| Out-of-order / duplicate samples | ordered insert; newer-receive-wins per history ingestion (REQ-SPATIAL-037) |
| Timestamp without valid clock mapping | `CLOCK_UNRESOLVED` (I-S7, spatial-core) |
| Device clock reset / re-time | epoch bump ⇒ old mappings invalid; `CLOCK_UNRESOLVED` until re-estimated (I-S7) |
| Query across disconnected trees | `PathNotFound` (spatial-core I-S3) |
| Stale sample past declared freshness | `STALE` flag + receive age (timing contracts) |
| Old overlay handle after `space.delete` | generation mismatch ⇒ invalid-handle status (spatial-core I-S12) |
| Runtime overlay frame query | bounded overlay prefix cached under `topology_generation` (I-Q16) |
| Reparent racing concurrent query | atomic commit ⇒ query sees pre- or post-tree, never partial (I-Q10) |
| `delayed_view(δ)` with δ > retained history | `OUT_OF_HISTORY` (I-Q6) |
| Near-antipodal quaternion slerp | documented stable path + oracle test |
| Empty or malformed compiled plan | build-time stack-effect error; never deployed |
| Reparent across disconnected trees | `PathNotFound` unless an explicit bridge supplies a common anchor |

## Test Matrix

| Case | Expected |
|---|---|
| `spatial_plan_execution` (instrumented) | I-Q1: zero base-graph traversal at runtime |
| `spatial_cache_shortcircuit` | I-Q2: geometry reuse, status still derived |
| `spatial_cache_aging` (no version change, wall time advances ⇒ all-clear→`STALE`) | I-Q2 |
| `spatial_query_latest` (asynchronous edge rates + update races vs single-instant oracle) | I-Q3: coherent t* |
| `spatial_query_at/predict/delayed_view` | I-Q4..I-Q6 vs oracle |
| `spatial_predict_screw` (screw motion vs exponential-map oracle) | I-Q5 |
| `spatial_query_predict` (body-frame twist integration vs oracle) | I-Q5 |
| `spatial_delayed_view` (delayed composite vs oracle) | I-Q6 |
| `spatial_interp_guard` (relocalize + reparent + calibration-jump fixtures) | I-Q7 |
| `spatial_reparent_temporal` (query t < t₁) | I-Q8: original hierarchy |
| `spatial_reparent_modes` ×3 vs oracle | I-Q9 |
| `spatial_reparent_world_continuity` (Preserve World world-pose continuity) | I-Q9 |
| `spatial_reparent_atomic` (concurrent readers) | I-Q10 |
| `spatial_calibration_live` (slot bump → cache invalidation) | I-Q11 |
| `spatial_generational_edges` | I-Q12 |
| `spatial_history_sizing` (N ≥ ⌊H/P⌋ + 2 enforced; unsatisfiable rejected; faster-than-P ⇒ `DensityViolation`) | I-Q13 |
| `spatial_history_order` (shuffled + duplicate timestamps vs oracle) | I-Q13 |
| `spatial_slot_coherence` (writer/readers) | I-Q14: coherent or `ConcurrentUpdate` |
| `spatial_slot_model` (model-checked interleavings, x86-64 + ARM64) | I-Q14 |
| `spatial_query_zero_alloc` | I-Q15 |
| `spatial_overlay_plan_bound` (overlay walk ≤ max_overlay_depth, generation-cached) | I-Q16 |
| interpolation numerics vs oracle (slerp/linear × rates) | exactness |
