# Spatial Index Algorithms

Scope: runtime spatial proximity indexes mirroring the dual hierarchy — a **fixed-capacity open-addressing hash grid** for dynamic overlay objects and a **compiled static BVH** for static environment frames with **live calibration-slot subtree swaps**. Zero-heap steady-state indexing/queries; frame-typed query API; candidate completeness; exact spatial memory verification. Implements [spatial_architecture.md](../../spatial_architecture.md) §Runtime Spatial Index, §Hash Grid for Dynamic Objects, §Compiled Static BVH, §Ownership and Update Discipline, §Proximity Query Semantics. Owner: E2 (compiler/BVH) + E1 (runtime grid). Phase 33 (implemented M21–M22). Depends on [spatial-core.md](spatial-core.md), [spatial-queries.md](spatial-queries.md), [memory-budget.md](../memory-budget.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-index-declaration` | `index.drm`: `anchor`, `cell_size`, `max_objects` — capacity declared before deployment, never tuned at runtime | `REQ-SPATIAL-014` |
| `REQ-SPATIAL-hash-grid` | integer cell coordinates hashed into a fixed-capacity open-addressing table; steady-state zero heap allocation | `REQ-SPATIAL-013` |
| `REQ-SPATIAL-motion-slack` | fat-cell envelope per object; re-bucket only on envelope exit; idle/slow objects cost zero per sample | `REQ-SPATIAL-019` |
| `REQ-SPATIAL-compiled-bvh` | static environment compiles to BVH at build time (spatial analogue of static collapse) | `REQ-SPATIAL-006` |
| `REQ-SPATIAL-bvh-live-calibration` | recalibration swaps affected subtrees live via calibration slots — no recompilation, no restarts | `REQ-SPATIAL-001` |
| `REQ-SPATIAL-index-derived-cache` | index is a derived cache; each node indexes only its declared `[spatial] read`; updates ride version counters | `REQ-SPATIAL-015` |
| `REQ-SPATIAL-proximity-frame-typed` | queries declare a frame and typed geometry; naked coordinates never enter the index API | `REQ-SPATIAL-021` |
| `REQ-SPATIAL-proximity-complete` | candidate set is a deliberate superset — callers exact-check; misses forbidden | `REQ-SPATIAL-020` |
| `REQ-SPATIAL-memory-verified` | worst-case index memory (buckets + slots at `max_objects`) verified exactly like channel buffers | `REQ-SPATIAL-018` |
| `REQ-SPATIAL-rejected-structures` | octrees, k-d trees, sweep-and-prune excluded | `REQ-SPATIAL-026` |

## Data Structures

```rust
// packages/perception/spaces/index.drm (declaration — capacity fixed before deployment)
// index:
//   anchor: map          # grid anchored to a declared frame
//   cell_size: 0.5       # meters — declared statically, never tuned at runtime
//   max_objects: 4096

// Fixed-capacity open-addressing table — zero heap allocation in steady state (§Hash Grid)
pub struct HashGrid {
    slots: [Slot; CAPACITY],          // CAPACITY = f(max_objects) — compile-time/layout-fixed, page-declared
    cell_size: f64,                   // from index.drm
    anchor: FrameId,                  // grid anchored to declared frame
}
pub struct Slot {
    state: SlotState,                 // Empty | Occupied{ id, gen } | Deleted{ gen }
    id: EntityId,
    generation: u32,                  // ABA protection; delete bumps generation (overlay lifetime rules)
}
pub struct CellKey { cx: i32, cy: i32, cz: i32 }   // floor(p / cell_size), hashed into the table

// Motion slack — fat-cell envelope (§Hash Grid: re-bucket only on envelope exit)
pub struct SlackEnvelope {
    entity: EntityId,
    center: Point3,                   // last bucketed position (in anchor frame)
    slack: f64,                       // declared motion slack (fat envelope, physics-engine fat-AABB pattern)
}
// envelope cell span = cells overlapping cube(center ± slack); entity re-buckets ONLY when p exits the span.

// Compiled static BVH (§Compiled Static BVH) — build-time artifact, fixed layout
pub struct CompiledBvh {
    nodes: Vec<BvhNode>,              // emitted into codegen artifacts; layout fixed per deployment
    subtrees: Vec<SubtreeSlot>,       // live-swap calibration slots (per recalibratable subtree)
}
pub struct BvhNode { aabb: Aabb, left: u32, right: u32, leaf: Option<StaticEntitySet> }
```

## Invariants

- **I-X1 ZeroAllocSteadyState**: indexing and queries perform zero heap allocation — all capacity declared (slots table, per-cell bounds); runtime overlay handles come from a pre-allocated pool. Test: `spatial_grid_zero_alloc` (counting-alloc harness).
- **I-X2 CapacityDeclared**: table capacity and per-cell slots derive from `max_objects`/`cell_size` at deployment; insertion beyond capacity ⇒ bounded policy (reject + counter), never growth. Test: `spatial_grid_capacity`.
- **I-X3 MotionSlack**: an object re-buckets only when it exits its slack envelope; idle/slow objects cost zero work per sample. Test: `spatial_motion_slack` (sub-threshold jitter ⇒ zero re-buckets).
- **I-X4 GenerationSafety**: overlay delete bumps generation; stale handles/iterations observe invalidation immediately. Test: `spatial_grid_generation`.
- **I-X5 FrameTypedAPI**: `nearby(frame, typed_point, radius)` / `region(frame, typed_aabb)` — naked coordinates never enter the index API (Golden Rule 1). Test: API audit + compile-fail.
- **I-X6 Completeness**: union(grid candidates, BVH candidates) ⊇ brute-force set — false positives allowed, misses forbidden. Test: `spatial_proximity_complete` (10k random scenes property).
- **I-X7 DerivedCacheOnly**: the index is a derived cache, not authoritative state; authoritative object state remains versioned dynamic edges; a node builds indexes exactly over its declared `[spatial] read`; updates ride existing version counters — unchanged versions touch nothing; no polling, no cross-node protocol, no central registry. Test: `spatial_index_derived`.
- **I-X8 BvhLiveCalibration**: recalibration swaps affected subtrees live through calibration slots (same slot mechanism as [spatial-queries.md](spatial-queries.md) §Calibration); no recompilation, no node restarts. Test: `spatial_bvh_live_calib`.
- **I-X9 MemoryVerified**: worst-case index memory (buckets + slots at `max_objects`, BVH nodes) is part of the compute-unit budget math, verified exactly like channel buffers. Test: `spatial_index_memory_verified`.
- **I-X10 RejectedStructures**: octrees/k-d/sweep-and-prune absent from the codebase. Test: audit.

## Functions

```rust
// ---- Dynamic grid (§Hash Grid for Dynamic Objects) ----
fn on_sample(&mut self, id: EntityId, p_typed: TypedPoint, ver: u32) {
    // Derived-cache discipline (I-X7): called when the versioned dynamic edge for `id` advances.
    if ver == self.last_seen_version(id) { return; }            // unchanged version ⇒ touch nothing
    let p = p_typed.in_frame(self.anchor)?;                     // typed → anchor-frame conversion (I-X5)
    let env = self.envelope(id);
    if env.contains(p) { self.update_center(id, p); return; }   // I-X3: inside slack ⇒ zero re-bucket work
    self.rebucket(id, p, env);                                  // remove old cells, insert new span cells
}

fn rebucket(&mut self, id: EntityId, p: Point3, old: SlackEnvelope) {
    for cell in old.span() { self.table.remove(cell_key(cell), id); }     // generation-aware removal
    let env = SlackEnvelope { center: p, slack: self.slack, .. };
    for cell in env.span() { self.table.insert(cell_key(cell), id, bump_gen(id)); }
}

// ---- Proximity queries (§Proximity Query Semantics — frame + typed geometry) ----
fn nearby(&self, frame: FrameId, point: TypedPoint, radius: f64) -> Result<CandidateSet, Status> {
    let p = point.in_frame(self.anchor)?;                       // typed conversion through standard path
    let mut set = CandidateSet::new();
    for key in cells_overlapping_sphere(frame, p, radius, self.cell_size) {   // superset cells — I-X6
        set.merge(self.table.scan_cell(key)?);                  // generation-valid entries only
    }
    set.merge(self.bvh.query(frame, aabb_of(p, radius)));
    Ok(set)                                                     // deliberate superset; caller exact-checks
}

fn region(&self, frame: FrameId, aabb: TypedAabb) -> Result<CandidateSet, Status> { /* same two-source union */ }

// ---- Compiled BVH (§Compiled Static BVH) ----
fn compile_bvh(static_frames: &StaticEnv) -> CompiledBvh { /* SAH split, deterministic ordering; build-time */ }

fn swap_subtree(&mut self, slot: SubtreeSlotId, new_geom: &StaticGeom) -> Result<(), Status> {
    // Recalibration WITHOUT recompilation (I-X8): swap the affected subtree via its calibration slot,
    // bump slot version; dependent queries see the new AABB set on next execution. Mirrors
    // Effective = Nominal × Correction from spatial-queries.md — BVH edition.
    self.subtrees[slot].publish(new_geom)                       // version bump; readers coherent via generation
}
```

## Protocols

*Index build and update discipline (§Ownership and Update Discipline):*

1. Node declares `[spatial] read` frames; index constructs over exactly those (I-X7).
2. Versioned dynamic edge advances ⇒ affected buckets refresh; unchanged versions ⇒ no work (I-X3/I-X7).
3. No polling, no cross-node index protocol, no central registry.

*Proximity query:*

1. Caller supplies frame + typed geometry (I-X5).
2. Grid: superset cells of the sphere/region in the anchor frame.
3. BVH: AABB overlap over compiled static tree (with live-swapped calibration subtrees).
4. Union returned with status metadata; caller exact-distance-checks typed points through the standard conversion path; per-candidate historical state resolves through `at()` ([spatial-queries.md](spatial-queries.md)).

*BVH recalibration:*

1. Calibration source publishes new static geometry for a subtree region.
2. `swap_subtree` publishes into the subtree's calibration slot; slot version bumps.
3. Queries execute the new AABBs immediately; history before the swap resolves through `at()` rules (temporal consistency with spatial-queries I-Q8).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Insert beyond declared capacity | reject + counter (`IndexFull` status) — never grow (I-X2) |
| Entity crosses cells within slack | no re-bucket (I-X3) |
| Entity exits slack envelope | bounded remove+insert across span cells |
| Delete with outstanding handles | generation bump ⇒ immediate invalidation (I-X4) |
| Query frame ≠ anchor frame | typed conversion through standard path; conversion failure ⇒ explicit status (I-X5) |
| Radius exceeding grid extent | full-set fallback (completeness preserved, perf warning) |
| Stale BVH vs runtime statics | impossible without slot bump — fingerprint check at deploy; live swaps only via slots (I-X8) |
| Query at historical timestamp | latest cached state for candidates; historical state via `at()` (spatial-queries) |
| Grid memory vs budget | verify rejects config exceeding declared budget (I-X9) |

## Test Matrix

| Case | Expected |
|---|---|
| `spatial_grid_zero_alloc` | I-X1 |
| `spatial_grid_capacity` (fill to max_objects) | I-X2 |
| `spatial_motion_slack` (jitter < slack) | I-X3: zero re-buckets |
| `spatial_grid_generation` | I-X4 |
| API audit / compile-fail | I-X5 |
| `spatial_proximity_complete` (10k scenes vs brute force) | I-X6: 0 misses |
| `spatial_index_derived` (version-counter updates only) | I-X7 |
| `spatial_bvh_live_calib` (slot swap, no restart) | I-X8 |
| `spatial_index_memory_verified` | I-X9 in budget math |
| code audit | I-X10 |
| perf: candidate factor vs brute force | documented |
