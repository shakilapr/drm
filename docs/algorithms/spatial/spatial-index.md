# Spatial Index Algorithms

Scope: runtime spatial proximity indexes mirroring the dual hierarchy — a **fixed-capacity banked hash grid** for dynamic overlay objects and a **compiled static BVH** for static environment frames with **live calibration-slot subtree swaps**. Zero-heap steady-state indexing/queries; frame-typed query API; candidate completeness; exact spatial memory verification. Implements [spatial_architecture.md](../../spatial_architecture.md) §Runtime Spatial Index, §Hash Grid for Dynamic Objects, §Compiled Static BVH, §Ownership and Update Discipline, §Proximity Query Semantics. Owner: E2 (compiler/BVH) + E1 (runtime grid). Phase 33 (implemented M21–M22). Depends on [spatial-core.md](spatial-core.md), [spatial-queries.md](spatial-queries.md), [memory-budget.md](../memory-budget.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-index-declaration` | `index.drm`: anchor, cell size, object bound, and cells-per-object bound are static | `REQ-SPATIAL-014` |
| `REQ-SPATIAL-hash-grid` | integer cell coordinates hashed into a fixed-capacity open-addressing table; steady-state zero heap allocation | `REQ-SPATIAL-013` |
| `REQ-SPATIAL-motion-slack` | fat-cell envelope per object; re-bucket only on envelope exit; idle/slow objects cost zero per sample | `REQ-SPATIAL-019` |
| `REQ-SPATIAL-compiled-bvh` | static environment compiles to BVH at build time (spatial analogue of static collapse) | `REQ-SPATIAL-006` |
| `REQ-SPATIAL-bvh-live-calibration` | recalibration swaps affected subtrees live via calibration slots — no recompilation, no restarts | `REQ-SPATIAL-001` |
| `REQ-SPATIAL-index-derived-cache` | index is a derived cache; each node indexes only its declared `[spatial] read`; updates ride version counters | `REQ-SPATIAL-015` |
| `REQ-SPATIAL-proximity-frame-typed` | queries declare a frame and typed geometry; naked coordinates never enter the index API | `REQ-SPATIAL-021` |
| `REQ-SPATIAL-proximity-complete` | candidate set is a deliberate superset — callers exact-check; misses forbidden | `REQ-SPATIAL-020` |
| `REQ-SPATIAL-memory-verified` | object, membership, query-scratch, and dual-BVH-bank memory is verified | `REQ-SPATIAL-018` |
| `REQ-SPATIAL-rejected-structures` | octrees, k-d trees, sweep-and-prune excluded | `REQ-SPATIAL-026` |

## Data Structures

```rust
// packages/perception/spaces/index.drm (declaration — capacity fixed before deployment)
// index:
//   anchor: map          # grid anchored to a declared frame
//   cell_size: 0.5       # meters — declared statically, never tuned at runtime
//   max_objects: 4096
//   max_cells_per_object: 27

// Fixed-capacity open-addressing table — zero heap allocation in steady state (§Hash Grid)
pub struct HashGrid {
    banks: [GridBank; 3],             // immutable while selected/read; fixed at activation
    selector: AtomicU64,              // packed {version, bank}; one atomic publication
    readers: [AtomicU32; 3],          // transient query guards; no cross-process refcount ownership
    writer_busy: AtomicBool,          // one non-blocking updater
    cell_size: f64,                   // from index.drm
    anchor: FrameId,                  // grid anchored to declared frame
}
pub struct GridBank {
    memberships: [Membership; MEMBERSHIP_CAPACITY],
    objects: [ObjectRecord; MAX_OBJECTS],
}
pub struct Membership { state: SlotState, key: CellKey, handle: SpatialHandle }
pub struct ObjectRecord { handle: SpatialHandle, envelope: SlackEnvelope, last_version: u64 }
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
    nodes: &'static [BvhNode],         // generated fixed artifact; no runtime growth
    subtrees: &'static [SubtreeSlot],  // fixed banks + query guards; atomic selection
}
pub struct BvhNode { aabb: Aabb, left: u32, right: u32, leaf: Option<StaticEntitySet> }
```

## Invariants

- **I-X1 ZeroAllocSteadyState**: indexing and queries perform zero heap allocation — all capacity declared (slots table, per-cell bounds); runtime overlay handles come from a pre-allocated pool. Test: `spatial_grid_zero_alloc` (counting-alloc harness).
- **I-X2 CapacityDeclared**: membership capacity is checked `max_objects × max_cells_per_object`; verifier rejects envelopes exceeding the declared cell span. Runtime overflow returns `IndexFull`, never grows. Test: `spatial_grid_capacity`.
- **I-X3 MotionSlack**: an object re-buckets only when it exits its slack envelope; idle/slow objects cost zero work per sample. Test: `spatial_motion_slack` (sub-threshold jitter ⇒ zero re-buckets).
- **I-X4 GenerationSafety**: overlay delete bumps generation; stale handles/iterations observe invalidation immediately. Test: `spatial_grid_generation`.
- **I-X5 FrameTypedAPI**: `nearby(frame, typed_point, radius)` / `region(frame, typed_aabb)` — naked coordinates never enter the index API (Golden Rule 1). Test: API audit + compile-fail.
- **I-X6 Completeness**: union(grid candidates, BVH candidates) ⊇ brute-force set — false positives allowed, misses forbidden. Test: `spatial_proximity_complete` (10k random scenes property).
- **I-X7 DerivedCacheOnly**: the index is a derived cache, not authoritative state; authoritative object state remains versioned dynamic edges; a node builds indexes exactly over its declared `[spatial] read`; updates ride existing version counters — unchanged versions touch nothing; no polling, no cross-node protocol, no central registry. Test: `spatial_index_derived`.
- **I-X8 BvhLiveCalibration**: recalibration swaps affected subtrees live through calibration slots (same slot mechanism as [spatial-queries.md](spatial-queries.md) §Calibration); no recompilation, no node restarts. Test: `spatial_bvh_live_calib`.
- **I-X9 MemoryVerified**: worst-case index memory (three grid banks, query scratch, guarded BVH banks) is part of the compute-unit budget math, verified exactly like channel buffers. Test: `spatial_index_memory_verified`.
- **I-X10 RejectedStructures**: octrees/k-d/sweep-and-prune absent from the codebase. Test: audit.
- **I-X11 CoherentQuery**: query acquires a selected immutable bank by increment→selector-reread; writers never modify a bank with readers. Bounded acquisition failure returns `ConcurrentUpdate`, never a data race or incomplete set. Test: `spatial_query_concurrent`.
- **I-X12 UpdateAtomic**: re-bucket preflights and builds an inactive bank; one release selector store publishes the complete update. Failure leaves the selected bank unchanged. Test: `spatial_update_atomic`.

## Functions

```rust
// ---- Dynamic grid (§Hash Grid for Dynamic Objects) ----
fn on_sample(&mut self, id: EntityId, p_typed: TypedPoint, ver: u64) -> Result<(), Status> {
    // Derived-cache discipline (I-X7): called when the versioned dynamic edge for `id` advances.
    if ver == self.last_seen_version(id) { return Ok(()); }
    let p = p_typed.in_frame(self.anchor)?;                     // typed → anchor-frame conversion (I-X5)
    let env = self.envelope(id);
    if env.contains(p) { self.record_version_only(id, ver); return Ok(()); } // center stays fixed
    self.rebucket(id, p, env, ver)
}

fn rebucket(&mut self, id: EntityId, p: Point3, old: SlackEnvelope, ver: u64) -> Result<(), Status> {
    let next = SlackEnvelope { center: p, slack: self.slack, .. };
    let _writer = self.try_writer_guard()?;                       // Busy; never waits
    let current = self.selector.load(Acquire);
    let target = self.find_inactive_bank_with_zero_readers(current).ok_or(Busy)?;
    let plan = self.preflight_exact_edits(current.bank, id, &old, &next)?; // probes exact slots
    self.banks[target].copy_from(&self.banks[current.bank]);       // fixed bounded copy
    self.banks[target].apply(plan, id, next, ver);                 // selected bank untouched
    let next_version = current.version.checked_add(1).ok_or(GenerationExhausted)?;
    self.selector.store(pack(next_version, target), Release);      // atomic visibility
    Ok(())
}

// ---- Proximity queries (§Proximity Query Semantics — frame + typed geometry) ----
fn nearby(&self, frame: FrameId, point: TypedPoint, radius: f64) -> Result<CandidateSet, Status> {
    let p = point.in_frame(self.anchor)?;                       // typed conversion through standard path
    let guard = self.acquire_selected_bank_bounded()?;          // increment readers, reread selector
    let mut set = CandidateSet::from_caller_scratch()?;         // fixed capacity; no allocation
    for key in cells_overlapping_sphere(frame, p, radius, self.cell_size) {   // superset cells — I-X6
        set.merge(guard.bank.memberships.scan_cell(key)?);      // generation-valid entries only
    }
    set.merge(self.bvh.query(frame, aabb_of(p, radius)));
    Ok(set)
}

fn region(&self, frame: FrameId, aabb: TypedAabb) -> Result<CandidateSet, Status> { /* same two-source union */ }

// ---- Compiled BVH (§Compiled Static BVH) ----
fn compile_bvh(static_frames: &StaticEnv) -> CompiledBvh { /* SAH split, deterministic ordering; build-time */ }

fn swap_subtree(&mut self, slot: SubtreeSlotId, new_geom: &StaticGeom) -> Result<(), Status> {
    let inactive = self.subtrees[slot].inactive_unread_bank().ok_or(Busy)?;
    inactive.build_checked(new_geom)?;                          // oversize/failure leaves active intact
    self.subtrees[slot].publish(inactive)                       // guarded readers keep old bank immutable
}
```

## Protocols

*Index build and update discipline (§Ownership and Update Discipline):*

1. Node declares `[spatial] read` frames; index constructs over exactly those (I-X7).
2. Versioned dynamic edge advances ⇒ affected buckets refresh in an inactive fixed bank; an object inside its fixed slack envelope changes no memberships.
3. No polling, no cross-node index protocol, no central registry.

*Proximity query:*

1. Caller supplies frame + typed geometry (I-X5).
2. Grid: superset cells of the sphere/region in the anchor frame.
3. BVH: AABB overlap over compiled static tree (with live-swapped calibration subtrees).
4. Union returned with status metadata; caller exact-distance-checks typed points through the standard conversion path; per-candidate historical state resolves through `at()` ([spatial-queries.md](spatial-queries.md)).

*BVH recalibration:*

1. Calibration source publishes new static geometry for a subtree region.
2. `swap_subtree` rebuilds an inactive bank with zero readers, then atomically selects it; if none is free, return `Busy` without mutation.
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
| Historical proximity requested | unsupported: proximity is latest-only; resolve returned candidates through `at()` |
| Grid memory vs budget | verify rejects config exceeding declared budget (I-X9) |
| No inactive grid/BVH bank without readers | `Busy`; active geometry unchanged, updater retries later |
| Update fails during exact probe/build | selected bank unchanged (I-X12) |

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
| `spatial_query_concurrent` | I-X11: complete set or `ConcurrentUpdate` |
| `spatial_update_atomic` (failure/crash injection) | I-X12: old or complete new bank |
| code audit | I-X10 |
| perf: candidate factor vs brute force | documented |
