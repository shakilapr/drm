# Spatial Index Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: runtime spatial proximity indexes mirroring the dual hierarchy — a **fixed-capacity incremental hash grid** for dynamic overlay objects and a **compiled static BVH** for static environment frames with **live calibration-slot subtree swaps**. Zero-heap steady-state indexing/queries; frame-typed query API; candidate completeness; exact spatial memory verification. Implements [spatial_architecture.md](../../spatial_architecture.md) §Runtime Spatial Index, §Hash Grid for Dynamic Objects, §Compiled Static BVH, §Ownership and Update Discipline, §Proximity Query Semantics. Owner: E2 (compiler/BVH) + E1 (runtime grid). Phase 33 (implemented M21–M22). Depends on [spatial-core.md](spatial-core.md), [spatial-queries.md](spatial-queries.md), [memory-budget.md](../memory-budget.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-index-declaration` | `index.drm`: anchor, cell size, object bound, cells-per-object bound, load factor, and max_query_cells are static | `REQ-SPATIAL-014` |
| `REQ-SPATIAL-hash-grid` | integer cell coordinates hashed into a fixed open-addressing table sized ceil(M/alpha_max) with alpha_max <= 0.70 and backward-shift deletion; incremental affected-cell updates published under a grid epoch; steady-state zero heap allocation | `REQ-SPATIAL-013` |
| `REQ-SPATIAL-motion-slack` | fat-cell envelope per object; re-bucket only on envelope exit; idle/slow objects cost zero per sample | `REQ-SPATIAL-019` |
| `REQ-SPATIAL-compiled-bvh` | static environment compiles to BVH at build time (spatial analogue of static collapse) | `REQ-SPATIAL-006` |
| `REQ-SPATIAL-bvh-live-calibration` | recalibration swaps affected subtrees live via calibration slots — no recompilation, no restarts | `REQ-SPATIAL-001` |
| `REQ-SPATIAL-index-derived-cache` | index is a derived cache; each node indexes only its declared `[spatial] read`; updates ride each object's full anchor-pose dependency vector (own edge, ancestors, calibration, topology) | `REQ-SPATIAL-015` |
| `REQ-SPATIAL-proximity-frame-typed` | queries declare a frame and typed geometry; naked coordinates never enter the index API | `REQ-SPATIAL-021` |
| `REQ-SPATIAL-proximity-complete` | candidate set is a deliberate superset — callers exact-check; misses forbidden; scratch overflow returns OutputTooSmall{required}, never silent truncation | `REQ-SPATIAL-020` |
| `REQ-SPATIAL-memory-verified` | object records, membership pool, cell-hash table, query-scratch, and dual-BVH-bank memory is verified | `REQ-SPATIAL-018` |
| `REQ-SPATIAL-rejected-structures` | octrees, k-d trees, sweep-and-prune excluded | `REQ-SPATIAL-026` |
| `REQ-SPATIAL-grid-incremental` | updates incremental, serialized by a single non-blocking updater touching only changed cells (`remove ∪ add`, intersection kept) — O(k), never a whole-index copy | `REQ-SPATIAL-042` |
| `REQ-SPATIAL-grid-epoch` | global grid epoch (even = stable, odd = updating) makes multi-cell moves transactional; queries accept candidates only on an unchanged epoch, exhaustion returns `CONCURRENT_UPDATE` | `REQ-SPATIAL-043` |
| `REQ-SPATIAL-bvh-banks` | each recalibratable BVH subtree has two fixed-capacity banks; recalibration builds the inactive bank then atomically selects it; oversize geometry rejected without disturbing the active bank | `REQ-SPATIAL-044` |
| `REQ-SPATIAL-proximity-latest` | proximity evaluates only the latest indexed state; per-candidate historical state resolves afterward through `at()` | `REQ-SPATIAL-045` |
| `REQ-SPATIAL-proximity-cell-fallback` | prospective cell count computed with checked arithmetic before enumeration; above `max_query_cells` fall back to the fixed object table, bounding worst-case work independently of radius | `REQ-SPATIAL-046` |
| `REQ-SPATIAL-index-capacity` | generated membership pool has checked capacity `max_objects × max_cells_per_object`; verify rejects a slack/envelope bound that can touch more cells | `REQ-SPATIAL-052` |

## Data Structures

```rust
// packages/perception/spaces/index.drm (declaration — capacity fixed before deployment)
// index:
//   anchor: map          # grid anchored to a declared frame
//   cell_size: 0.5       # meters — declared statically, never tuned at runtime
//   max_objects: 4096
//   max_cells_per_object: 27
//   load_factor: 0.70    # physical cell-hash slots = ceil(M / load_factor)
//   max_query_cells: 512 # enumeration bound before object-table fallback

// Fixed-capacity incremental hash grid — zero heap allocation in steady state (§Hash Grid)
pub struct HashGrid {
    cells: FixedCellTable,             // CellKey -> CellHead; S = ceil(M / alpha_max) slots, alpha_max = 0.70
    nodes: FixedPool<MembershipNode>,  // membership pool; capacity M = max_objects × max_cells_per_object
    objects: [ObjectRecord; MAX_OBJECTS],
    grid_epoch: AtomicU64,             // even = stable, odd = update in progress; multi-cell transaction marker
    writer_busy: AtomicBool,           // single non-blocking updater; second writer ⇒ Busy (single-writer discipline)
    cell_size: f64,                    // from index.drm; > 0 verified at activation
    max_query_cells: u32,              // from index.drm; enumeration bound (I-X13)
    anchor: FrameId,                   // grid anchored to declared frame
}
pub struct CellHead { version: AtomicU32, head: AtomicU32 }   // membership list root; read handshake via version
pub struct MembershipNode { entity: SpatialHandle, next: u32 }
pub struct ObjectRecord {
    handle: SpatialHandle,
    envelope: SlackEnvelope,               // last bucketed anchor-space fat envelope
    occupied_cells: [CellKey; MAX_CELLS_PER_OBJECT],
    dependency_versions: [u64; MAX_POSE_DEPENDENCIES], // full anchor-pose input vector (I-X7)
    topology_generation: u64,
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
    nodes: &'static [BvhNode],         // generated fixed artifact; no runtime growth
    subtrees: &'static [SubtreeSlot],  // fixed banks + query guards; atomic selection
}
pub struct BvhNode { aabb: Aabb, left: u32, right: u32, leaf: Option<StaticEntitySet> }
```

## Invariants

- **I-X1 ZeroAllocSteadyState**: indexing and queries perform zero heap allocation — all capacity declared (slots table, per-cell bounds); runtime overlay handles come from a pre-allocated pool. Test: `spatial_grid_zero_alloc` (counting-alloc harness).
- **I-X2 CapacityDeclared**: membership capacity `M = max_objects × max_cells_per_object` is fixed; the cell-hash table is sized `S = ⌈M/α_max⌉` with `α_max ≤ 0.70` (probe bounds hold at worst-case occupancy); deletion uses backward shift — no tombstones. Verifier rejects envelopes exceeding the declared cell span. Runtime overflow returns `IndexFull`, never grows. Test: `spatial_grid_capacity`, `spatial_hash_occupancy`.
- **I-X3 MotionSlack**: an object re-buckets only when it exits its slack envelope; idle/slow objects cost zero work per sample. Test: `spatial_motion_slack` (sub-threshold jitter ⇒ zero re-buckets).
- **I-X4 GenerationSafety**: overlay delete bumps generation; stale handles/iterations observe invalidation immediately. Test: `spatial_grid_generation`.
- **I-X5 FrameTypedAPI**: `nearby(frame, typed_point, radius)` / `region(frame, typed_aabb)` — naked coordinates never enter the index API (Golden Rule 1). Test: API audit + compile-fail.
- **I-X6 Completeness**: union(grid candidates, BVH candidates) ⊇ brute-force set — false positives allowed, misses forbidden; candidate storage overflow returns `OutputTooSmall{required}`, never silent truncation. Test: `spatial_proximity_complete` (10k random scenes property), `spatial_candidate_saturation`.
- **I-X7 DependencyInvalidation**: the index is a derived cache, not authoritative state; authoritative object state remains versioned dynamic edges; a node builds indexes exactly over its declared `[spatial] read`; each object tracks the full dependency vector of its anchor-space pose (own edge, ancestor edges, calibration slots, topology generation) — a change to **any** dependency invalidates the object's location; an unchanged vector touches nothing; no polling, no cross-node protocol, no central registry. Test: `spatial_index_derived`, `spatial_index_dependency`.
- **I-X8 BvhLiveCalibration**: recalibration swaps affected subtrees live through calibration slots (same slot mechanism as [spatial-queries.md](spatial-queries.md) §Calibration); no recompilation, no node restarts. Test: `spatial_bvh_live_calib`.
- **I-X9 MemoryVerified**: worst-case index memory (cell-hash table, membership pool, object records, query scratch, guarded BVH banks) is part of the compute-unit budget math, verified exactly like channel buffers. Test: `spatial_index_memory_verified`.
- **I-X10 RejectedStructures**: octrees/k-d/sweep-and-prune absent from the codebase. Test: audit.
- **I-X11 CoherentQuery**: query reads `grid_epoch` (must be even), scans cells, rereads `grid_epoch`; unchanged ⇒ accept, else bounded retry ⇒ `ConcurrentUpdate` — never a data race or a partial multi-cell transaction. Cell reads additionally honor per-cell version handshakes. Test: `spatial_query_concurrent`.
- **I-X12 UpdateAtomic**: a move mutates only the `remove ∪ add` cells under an odd epoch and restores the even epoch with one release store — including on failure, so the grid can never latch odd. Failure/crash leaves the previous epoch state coherent. Update cost is `O(k)` in the object's cell span — never a whole-index copy. Test: `spatial_update_atomic`, `spatial_rebucket_scaling`.
- **I-X13 QueryBound**: the prospective cell count is computed with checked arithmetic before enumeration; above `max_query_cells` the query falls back to scanning the fixed object table. Worst-case query work is `O(min(C_q, max_objects))`, independent of radius. Test: `spatial_huge_radius`.
- **I-X14 NumericCellBounds**: `cell_size > 0` verified at activation; NaN/±inf coordinates and out-of-i32-range cell indices are rejected (`InvalidArgument`), never silently clamped or misclassified; floor is toward −∞ with the tie rule tested at ±ULP boundaries. Test: `spatial_numeric_grid`.

## Functions

```rust
// ---- Dynamic grid (§Hash Grid for Dynamic Objects) ----
fn on_sample(&mut self, id: EntityId, p_typed: TypedPoint, deps: &DependencyVersions) -> Result<(), Status> {
    // Derived-cache discipline (I-X7): `deps` is the full anchor-pose dependency vector.
    if self.record(id).dependency_versions == *deps { return Ok(()); }  // no input changed
    let p = p_typed.in_frame(self.anchor)?;                     // typed → anchor-frame conversion (I-X5)
    if self.record(id).envelope.contains(p) {
        self.record(id).update_dependencies(deps);              // envelope holds: versions only (I-X3)
        return Ok(());
    }
    self.rebucket(id, p, deps)
}

fn rebucket(&mut self, id: EntityId, p: Point3, deps: &DependencyVersions) -> Result<(), Status> {
    let next = SlackEnvelope { center: p, slack: self.slack, .. };
    let _writer = self.try_writer_guard()?;                     // Busy; never waits — serializes updaters
    let old_cells = self.record(id).occupied_cells();           // envelope cell span
    let new_cells = cells_of_envelope(&next);
    let (remove, add) = (old_cells − new_cells, new_cells − old_cells);   // intersection kept
    self.preflight_capacity(&remove, &add)?;                    // IndexFull before mutation
    let e = self.grid_epoch.fetch_add(1, AcqRel) + 1;           // odd: update in progress (I-X12)
    let res = self.apply(remove, add, id, &next, deps);         // touches ONLY affected cells — O(k)
    self.grid_epoch.store(e + 1, Release);                      // even again — even on failure
    res
}

// ---- Proximity queries (§Proximity Query Semantics — frame + typed geometry) ----
fn nearby(&self, frame: FrameId, point: TypedPoint, radius: f64) -> Result<CandidateSet, Status> {
    let p = point.in_frame(self.anchor)?;                       // typed conversion (I-X5)
    let e1 = self.stable_epoch_bounded()?;                      // even epoch, else bounded retry (I-X11)
    let mut set = CandidateSet::from_caller_scratch()?;         // overflow ⇒ OutputTooSmall{required} (I-X6)
    if checked_cell_count(p, radius, self.cell_size)? <= self.max_query_cells {
        for key in cells_overlapping_sphere(self.anchor, p, radius, self.cell_size) {  // anchor frame (I-X5)
            set.merge(self.cells.scan_cell(key)?);              // per-cell version handshake
        }
    } else {
        set.merge(self.objects.scan_overlapping(p, radius));    // bounded fallback (I-X13)
    }
    set.merge(self.bvh.query(self.anchor, aabb_of(p, radius))); // anchor frame (I-X5)
    if self.grid_epoch.load(Acquire) != e1 { return Err(ConcurrentUpdate); }  // coherent view (I-X11)
    Ok(set)
}

// Cell conversion (I-X14): floor toward −∞; cell_size > 0 verified at activation;
// NaN/±inf and out-of-i32-range results rejected (`InvalidArgument`), never clamped.
fn cell_of(coord: f64, cell_size: f64) -> Result<i32, Status>

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
2. The node re-resolves each affected object's anchor pose whenever any input in the object's dependency vector advances (own edge, ancestors, calibration, topology); a changed vector triggers `remove/keep/add` cell edits under the grid epoch; an unchanged vector touches nothing; an object still inside its slack envelope updates only its recorded versions (I-X3, I-X7).
3. No polling, no cross-node index protocol, no central registry.

*Proximity query:*

1. Caller supplies frame + typed geometry (I-X5); everything downstream operates in the anchor frame.
2. Grid: checked cell-count precheck (I-X13), then superset cells of the sphere/region in the anchor frame under the epoch handshake (I-X11), or the bounded object-table fallback.
3. BVH: AABB overlap over compiled static tree (with live-swapped calibration subtrees), also in the anchor frame.
4. Union returned with status metadata; caller exact-distance-checks typed points through the standard conversion path; per-candidate historical state resolves through `at()` ([spatial-queries.md](spatial-queries.md)).

*BVH recalibration:*

1. Calibration source publishes new static geometry for a subtree region.
2. `swap_subtree` rebuilds an inactive bank with zero readers, then atomically selects it; if none is free, return `Busy` without mutation.
3. Queries execute the new AABBs immediately; history before the swap resolves through `at()` rules (temporal consistency with spatial-queries I-Q8).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Insert beyond declared capacity | reject + counter (`IndexFull` status) — never grow (I-X2); cell-hash occupancy ≤ α_max by construction |
| Entity crosses cells within slack | no re-bucket; dependency versions updated (I-X3) |
| Entity exits slack envelope | `remove/keep/add` edits across span cells only (I-X12) |
| Ancestor edge moves, own edge unchanged | dependency vector changes ⇒ object invalidated, no misses (I-X7) |
| Delete with outstanding handles | generation bump ⇒ immediate invalidation (I-X4) |
| Query frame ≠ anchor frame | typed conversion through standard path; conversion failure ⇒ explicit status (I-X5) |
| Radius implying > max_query_cells | bounded object-table scan fallback, decided before enumeration (I-X13) |
| All candidates beyond caller scratch | `OutputTooSmall{required}` — never silent truncation (I-X6) |
| NaN/±inf/huge coordinates | `InvalidArgument`, never silent cell misclassification (I-X14) |
| Query racing a move | epoch handshake ⇒ pre- or post-move coherent view, else `ConcurrentUpdate` (I-X11) |
| Stale BVH vs runtime statics | impossible without slot bump — fingerprint check at deploy; live swaps only via slots (I-X8) |
| Historical proximity requested | unsupported: proximity is latest-only; resolve returned candidates through `at()` |
| Grid memory vs budget | verify rejects config exceeding declared budget (I-X9) |
| BVH swap with no free unread bank | `Busy`; active geometry unchanged, updater retries later |
| Update fails mid-move | even epoch restored; previous epoch state coherent (I-X12) |

## Test Matrix

| Case | Expected |
|---|---|
| `spatial_grid_zero_alloc` | I-X1 |
| `spatial_grid_capacity` (fill to max_objects) | I-X2 |
| `spatial_hash_occupancy` (50–95% occupancy, collisions + deletes; probe bound holds) | I-X2 |
| `spatial_motion_slack` (jitter < slack) | I-X3: zero re-buckets; versions updated |
| `spatial_grid_generation` | I-X4 |
| API audit / compile-fail | I-X5 |
| `spatial_proximity_complete` (10k scenes vs brute force) | I-X6: 0 misses |
| `spatial_candidate_saturation` (all objects in one region) | I-X6: `OutputTooSmall{required}` |
| `spatial_index_derived` (dependency-vector updates only) | I-X7 |
| `spatial_index_dependency` (ancestor moves, own edge unchanged ⇒ 0 misses) | I-X7 |
| `spatial_bvh_live_calib` (slot swap, no restart) | I-X8 |
| `spatial_index_memory_verified` | I-X9 in budget math |
| `spatial_query_concurrent` (query vs move) | I-X11: coherent pre/post view or `ConcurrentUpdate` |
| `spatial_update_atomic` (failure/crash injection) | I-X12: previous epoch coherent |
| `spatial_rebucket_scaling` (capacity 256→8192, one mover; p99 flat) | I-X12: O(k) updates |
| `spatial_huge_radius` (fallback decided before enumeration) | I-X13 |
| `spatial_numeric_grid` (±ULP boundaries, NaN, huge values) | I-X14 |
| code audit | I-X10 |
| perf: candidate factor vs brute force | documented |
