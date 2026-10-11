//! Fixed-capacity incremental hash grid for dynamic overlay objects
//! ([spatial-index.md](../../docs/algorithms/spatial/spatial-index.md) §Data
//! Structures, `§Functions`, `§Invariants`; [`spatial_architecture.md`](../../docs/spatial_architecture.md)
//! §Hash Grid for Dynamic Objects, §Proximity Query Semantics). Integer cell
//! coordinates `floor(p / cell_size)` are hashed into a fixed open-addressing
//! table sized `S = ⌈M/α_max⌉` with `α_max = 0.70` and backward-shift deletion
//! (`I-X1`/`I-X2`). Objects carry a fat-cell motion-slack envelope so an idle or
//! slow object re-buckets only on envelope exit (`I-X3`), the full anchor-pose
//! dependency vector invalidates a location even when the object's own edge did
//! not move (`I-X7`), and a global even/odd grid epoch makes multi-cell moves
//! transactional (`I-X11`/`I-X12`). All capacity is declared at activation, so
//! steady-state indexing and queries perform zero heap allocation (`I-X1`).
//!
//! REQ: REQ-SPATIAL-013, REQ-SPATIAL-014, REQ-SPATIAL-019, REQ-SPATIAL-042,
//! REQ-SPATIAL-043, REQ-SPATIAL-045, REQ-SPATIAL-046, REQ-SPATIAL-052,
//! REQ-SPATIAL-020, REQ-SPATIAL-009.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::types::{Point3, SpatialError, SpatialHandle};

/// `α_max` — maximum load factor of the cell-hash table (`I-X2`).
/// REQ: REQ-SPATIAL-014.
pub const ALPHA_MAX: f64 = 0.70;

/// A frame-local integer cell coordinate (`floor(p / cell_size)`, `I-X14`).
/// REQ: REQ-SPATIAL-013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey {
    /// Cell index on `X`.
    pub cx: i32,
    /// Cell index on `Y`.
    pub cy: i32,
    /// Cell index on `Z`.
    pub cz: i32,
}

/// Static `index.drm` declaration: every capacity is fixed before deployment
/// (`I-X2`, `REQ-SPATIAL-014`).
/// REQ: REQ-SPATIAL-014, REQ-SPATIAL-052.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridConfig {
    /// Declared cell edge length in meters (`> 0`, `I-X14`).
    pub cell_size: f64,
    /// Maximum tracked objects (`M_objects`).
    pub max_objects: u32,
    /// Maximum cells a single object's envelope may span.
    pub max_cells_per_object: u32,
    /// Maximum cell count enumerated before the object-table fallback (`I-X13`).
    pub max_query_cells: u32,
    /// Declared motion slack (fat-envelope half-edge, meters).
    pub slack: f64,
}

/// A typed point in the grid's anchor frame (`I-X5`: naked coordinates never
/// enter the index API).
/// REQ: REQ-SPATIAL-020.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedPoint {
    /// The anchor-frame position in meters.
    pub point: Point3,
    /// The declared frame identity of `point`.
    pub frame: u32,
}

/// A typed axis-aligned region in the grid's anchor frame.
/// REQ: REQ-SPATIAL-020.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedAabb {
    /// Region minimum corner, meters.
    pub min: Point3,
    /// Region maximum corner, meters.
    pub max: Point3,
    /// The declared frame identity of the region.
    pub frame: u32,
}

/// The full anchor-pose dependency vector of one object: its own edge, every
/// ancestor edge on the conversion path, calibration slots, and the topology
/// generation (`I-X7`). A change to **any** entry invalidates the object.
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-019.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridDeps {
    /// Version of the object's own dynamic edge.
    pub own: u64,
    /// Version of the nearest dynamic ancestor edge.
    pub ancestor: u64,
    /// Version of the calibration slot feeding the anchor pose.
    pub calibration: u64,
    /// Runtime-overlay topology generation.
    pub topology: u64,
}

/// Caller-supplied scratch for query candidates (`I-X1`: no query-time heap).
/// A capacity that cannot hold the candidate superset is an explicit
/// [`SpatialError::InvalidConfig`] at construction and a reported
/// [`ProximityOutcome::overflow`] at query (`I-X6`).
/// REQ: REQ-SPATIAL-020.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSet {
    handles: Vec<SpatialHandle>,
    dropped: u32,
}

impl CandidateSet {
    /// Allocates a fixed-capacity candidate scratch of `capacity` handles.
    ///
    /// # Errors
    /// [`SpatialError::InvalidConfig`] when `capacity` is zero.
    /// REQ: REQ-SPATIAL-020.
    pub fn with_capacity(capacity: u32) -> Result<Self, SpatialError> {
        if capacity == 0 {
            return Err(SpatialError::InvalidConfig);
        }
        Ok(Self {
            handles: Vec::with_capacity(capacity as usize),
            dropped: 0,
        })
    }

    /// The fixed scratch capacity.
    /// REQ: REQ-SPATIAL-020.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.handles.capacity()
    }

    /// The accepted candidate handles.
    /// REQ: REQ-SPATIAL-020.
    #[must_use]
    pub fn handles(&self) -> &[SpatialHandle] {
        &self.handles
    }

    /// A lower bound on the capacity needed to hold every candidate (accepted
    /// plus dropped); `0` when nothing overflowed. Never silent truncation
    /// (`I-X6`). REQ: REQ-SPATIAL-020.
    #[must_use]
    pub fn required(&self) -> u32 {
        if self.dropped == 0 {
            0
        } else {
            self.handles.len() as u32 + self.dropped
        }
    }

    /// Clears the scratch without freeing its allocation.
    /// REQ: REQ-SPATIAL-020.
    pub fn clear(&mut self) {
        self.handles.clear();
        self.dropped = 0;
    }

    /// Merges `handle` into the scratch, recording a drop when full. Returns
    /// `true` when accepted, `false` on overflow (`OutputTooSmall`). Used by the
    /// two-source proximity union so grid and static hits share one scratch
    /// (`I-X6`). REQ: REQ-SPATIAL-020.
    pub(crate) fn merge(&mut self, handle: SpatialHandle) -> bool {
        self.push(handle)
    }

    /// Whether `handle` is already present in the scratch (union
    /// deduplication). REQ: REQ-SPATIAL-020.
    #[must_use]
    pub(crate) fn contains(&self, handle: SpatialHandle) -> bool {
        self.handles.contains(&handle)
    }

    /// Pushes a candidate, or records a drop when full. Returns `false` on
    /// overflow (the caller surfaces `OutputTooSmall`).
    /// REQ: REQ-SPATIAL-020.
    fn push(&mut self, handle: SpatialHandle) -> bool {
        if self.handles.len() == self.handles.capacity() {
            self.dropped += 1;
            return false;
        }
        self.handles.push(handle);
        true
    }
}

/// A proximity query outcome: the enumeration path and whether every candidate
/// fit the caller's scratch. The candidates themselves live in the caller's
/// [`CandidateSet`] (no query-time ownership copy, `I-X1`). REQ: REQ-SPATIAL-020.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProximityOutcome {
    /// Whether the query enumerated cells or used the object-table fallback.
    pub path: QueryPath,
    /// `Some(required)` when the scratch could not hold every candidate; a
    /// lower bound on the needed capacity. Never silent truncation (`I-X6`).
    pub overflow: Option<u32>,
}

impl ProximityOutcome {
    /// Whether every candidate fit the scratch. REQ: REQ-SPATIAL-020.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.overflow.is_none()
    }
}

/// Whether a query enumerated grid cells or the bounded object-table fallback
/// (`I-X13`). REQ: REQ-SPATIAL-046.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryPath {
    /// Cells of the sphere/region were enumerated under the declared bound.
    Cells,
    /// Prospective cell count exceeded `max_query_cells`; the fixed object
    /// table was scanned instead.
    ObjectFallback,
}

/// One tracked object record (`I-X3`/`I-X7`).
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-019.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ObjectRecord {
    handle: SpatialHandle,
    live: bool,
    center: Point3,
    slack: f64,
    occupied: [CellKey; MAX_OCCUPIED],
    occupied_len: u8,
    deps: GridDeps,
    /// Re-buckets observed for this object (metrics; `I-X3`).
    rebuckets: u64,
}

/// Upper bound on cells a single envelope may occupy. A cube of half-edge
/// `slack` spans `(2·⌈slack/cell_size⌉ + 1)³` cells; this compile-time bound
/// gates `index.drm` (`I-X2`). REQ: REQ-SPATIAL-052.
pub const MAX_OCCUPIED: usize = 27;

impl ObjectRecord {
    /// An empty record at slot `handle`.
    fn empty(handle: SpatialHandle) -> Self {
        Self {
            handle,
            live: false,
            center: Point3::default(),
            slack: 0.0,
            occupied: [CellKey {
                cx: 0,
                cy: 0,
                cz: 0,
            }; MAX_OCCUPIED],
            occupied_len: 0,
            deps: GridDeps {
                own: 0,
                ancestor: 0,
                calibration: 0,
                topology: 0,
            },
            rebuckets: 0,
        }
    }

    /// The occupied cells of this record.
    fn cells(&self) -> &[CellKey] {
        &self.occupied[..usize::from(self.occupied_len)]
    }
}

/// One open-addressing cell-table slot: a key plus the head of its membership
/// list and a read version (`I-X2`/`I-X11`).
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-043.
#[derive(Debug)]
struct CellSlot {
    key: CellKey,
    used: bool,
    version: AtomicU32,
    /// Index into `nodes` of the membership head, or `NODE_NONE`.
    head: u32,
}

/// One membership-pool node: an object slot index plus the next node.
/// REQ: REQ-SPATIAL-013.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MembershipNode {
    object: u32,
    next: u32,
}

/// Sentinel for "no membership node" (`u32::MAX`).
const NODE_NONE: u32 = u32::MAX;

/// Cell coordinate floor conversion (`I-X14`): floor toward `−∞`; rejects
/// `NaN`/`±inf` and results outside the `i32` range with
/// [`SpatialError::InvalidArg`] — never a silent clamp.
///
/// # Errors
/// [`SpatialError::InvalidArg`] for a non-finite coordinate, a non-positive
/// cell size, or an out-of-range cell index.
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-014.
pub fn cell_of(coord: f64, cell_size: f64) -> Result<i32, SpatialError> {
    if !coord.is_finite() || cell_size <= 0.0 || !cell_size.is_finite() {
        return Err(SpatialError::InvalidArg);
    }
    let raw = (coord / cell_size).floor();
    if raw < f64::from(i32::MIN) || raw > f64::from(i32::MAX) {
        return Err(SpatialError::InvalidArg);
    }
    Ok(raw as i32)
}

/// The cells of the envelope centered at `center` with half-edge `slack`:
/// every integer cell whose unit cube intersects `cube(center ± slack)`.
/// Returns the count; fills `out` (bounded by `out.len()`).
///
/// # Errors
/// [`SpatialError::InvalidArg`] when any bound is non-finite or out of the
/// `i32` cell range, or when the span exceeds `out.len()` (`I-X2`).
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-052.
fn cells_of_envelope(
    center: Point3,
    slack: f64,
    cell_size: f64,
    out: &mut [CellKey],
) -> Result<usize, SpatialError> {
    if !slack.is_finite() || slack < 0.0 {
        return Err(SpatialError::InvalidArg);
    }
    let Some(lo) = subtract(center, slack) else {
        return Err(SpatialError::InvalidArg);
    };
    let Some(hi) = add(center, slack) else {
        return Err(SpatialError::InvalidArg);
    };
    let lo_c = CellKey {
        cx: cell_of(lo.x, cell_size)?,
        cy: cell_of(lo.y, cell_size)?,
        cz: cell_of(lo.z, cell_size)?,
    };
    let hi_c = CellKey {
        cx: cell_of(hi.x, cell_size)?,
        cy: cell_of(hi.y, cell_size)?,
        cz: cell_of(hi.z, cell_size)?,
    };
    let mut n = 0usize;
    let mut cx = lo_c.cx;
    while cx <= hi_c.cx {
        let mut cy = lo_c.cy;
        while cy <= hi_c.cy {
            let mut cz = lo_c.cz;
            while cz <= hi_c.cz {
                if n == out.len() {
                    return Err(SpatialError::InvalidConfig);
                }
                out[n] = CellKey { cx, cy, cz };
                n += 1;
                if cz == i32::MAX {
                    break;
                }
                cz += 1;
            }
            if cy == i32::MAX {
                break;
            }
            cy += 1;
        }
        if cx == i32::MAX {
            break;
        }
        cx += 1;
    }
    Ok(n)
}

/// `p - s` on all axes; `None` when either operand is non-finite.
/// REQ: REQ-SPATIAL-013.
fn subtract(p: Point3, s: f64) -> Option<Point3> {
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() || !s.is_finite() {
        return None;
    }
    Some(Point3 {
        x: p.x - s,
        y: p.y - s,
        z: p.z - s,
    })
}

/// `p + s` on all axes; `None` when either operand is non-finite.
/// REQ: REQ-SPATIAL-013.
fn add(p: Point3, s: f64) -> Option<Point3> {
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() || !s.is_finite() {
        return None;
    }
    Some(Point3 {
        x: p.x + s,
        y: p.y + s,
        z: p.z + s,
    })
}

/// `⌈M/α_max⌉` — the physical cell-hash slot count (`I-X2`). The count is
/// rounded up to a power of two so probe arithmetic uses a mask; rounding up
/// keeps occupancy at or below `α_max`.
/// REQ: REQ-SPATIAL-014, REQ-SPATIAL-052.
#[must_use]
pub fn cell_hash_slots(max_objects: u32, max_cells_per_object: u32) -> Option<u32> {
    let m = u64::from(max_objects).checked_mul(u64::from(max_cells_per_object))?;
    let s = (m as f64 / ALPHA_MAX).ceil();
    if s > f64::from(u32::MAX / 2) {
        return None;
    }
    let s = (s as u32).max(1).next_power_of_two();
    Some(s)
}

/// The membership-pool capacity `M = max_objects × max_cells_per_object`
/// (`I-X2`). `None` on overflow.
/// REQ: REQ-SPATIAL-052.
#[must_use]
pub fn membership_capacity(max_objects: u32, max_cells_per_object: u32) -> Option<u32> {
    max_objects.checked_mul(max_cells_per_object)
}

/// A fixed-capacity incremental hash grid (`I-X1`..`I-X4`, `I-X11`..`I-X14`).
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-014.
#[derive(Debug)]
pub struct HashGrid {
    config: GridConfig,
    cells: Vec<CellSlot>,
    nodes: Vec<MembershipNode>,
    free_nodes: Vec<u32>,
    objects: Vec<ObjectRecord>,
    cell_epoch: AtomicU64,
    writer_busy: AtomicBool,
    /// Counters (metrics). REQ: REQ-SPATIAL-013.
    rebuckets: u64,
    keeps: u64,
}

/// Builds a query outcome from the caller's scratch: any drop yields an
/// overflow `required` lower bound (`I-X6`). REQ: REQ-SPATIAL-020.
fn finish(path: QueryPath, scratch: &CandidateSet, overflow: bool) -> ProximityOutcome {
    let required = scratch.required();
    ProximityOutcome {
        path,
        overflow: (overflow || required > 0)
            .then(|| required.max(scratch.handles().len() as u32 + 1)),
    }
}

/// The prospective cell count of a box, `(nx·ny·nz)` with checked arithmetic,
/// decided before enumeration (`I-X13`). REQ: REQ-SPATIAL-046.
fn cell_count(lo: CellKey, hi: CellKey) -> Result<i64, SpatialError> {
    let nx = i64::from(hi.cx) - i64::from(lo.cx) + 1;
    let ny = i64::from(hi.cy) - i64::from(lo.cy) + 1;
    let nz = i64::from(hi.cz) - i64::from(lo.cz) + 1;
    nx.checked_mul(ny)
        .and_then(|v| v.checked_mul(nz))
        .ok_or(SpatialError::InvalidArg)
}

impl HashGrid {
    /// Activates a grid from a declared [`GridConfig`], allocating every
    /// capacity up front (`I-X2`). `cell_size > 0` is verified here (`I-X14`).
    ///
    /// # Errors
    /// [`SpatialError::InvalidConfig`] for a non-positive `cell_size`, a zero
    /// object bound, a zero cells-per-object bound, a span exceeding
    /// [`MAX_OCCUPIED`], or a capacity-product overflow.
    /// REQ: REQ-SPATIAL-014, REQ-SPATIAL-052.
    pub fn activate(config: GridConfig) -> Result<Self, SpatialError> {
        if config.cell_size <= 0.0 || !config.cell_size.is_finite() {
            return Err(SpatialError::InvalidConfig);
        }
        if config.max_objects == 0 || config.max_cells_per_object == 0 {
            return Err(SpatialError::InvalidConfig);
        }
        if config.max_cells_per_object as usize > MAX_OCCUPIED {
            return Err(SpatialError::InvalidConfig);
        }
        let m = membership_capacity(config.max_objects, config.max_cells_per_object)
            .ok_or(SpatialError::InvalidConfig)?;
        let s = cell_hash_slots(config.max_objects, config.max_cells_per_object)
            .ok_or(SpatialError::InvalidConfig)?;
        let objects = (0..config.max_objects)
            .map(|i| {
                ObjectRecord::empty(SpatialHandle {
                    index: i,
                    generation: 0,
                })
            })
            .collect();
        Ok(Self {
            config,
            cells: (0..s as usize)
                .map(|_| CellSlot {
                    key: CellKey {
                        cx: 0,
                        cy: 0,
                        cz: 0,
                    },
                    used: false,
                    version: AtomicU32::new(0),
                    head: NODE_NONE,
                })
                .collect(),
            nodes: vec![
                MembershipNode {
                    object: 0,
                    next: NODE_NONE
                };
                m as usize
            ],
            free_nodes: (0..m).rev().collect(),
            objects,
            cell_epoch: AtomicU64::new(0),
            writer_busy: AtomicBool::new(false),
            rebuckets: 0,
            keeps: 0,
        })
    }

    /// The declared configuration.
    /// REQ: REQ-SPATIAL-014.
    #[must_use]
    pub fn config(&self) -> GridConfig {
        self.config
    }

    /// The current grid epoch: even = stable, odd = update in progress
    /// (`I-X11`/`I-X12`). REQ: REQ-SPATIAL-043.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.cell_epoch.load(Ordering::Acquire)
    }

    /// Cumulative re-bucket count (`I-X3` metric). REQ: REQ-SPATIAL-019.
    #[must_use]
    pub fn rebucket_count(&self) -> u64 {
        self.rebuckets
    }

    /// Cumulative envelope-hold (dependency-only) update count (`I-X3`).
    /// REQ: REQ-SPATIAL-019.
    #[must_use]
    pub fn keep_count(&self) -> u64 {
        self.keeps
    }

    /// The handle of the live object at `index`, if any.
    /// REQ: REQ-SPATIAL-013.
    #[must_use]
    pub fn handle_at(&self, index: u32) -> Option<SpatialHandle> {
        let r = self.objects.get(index as usize)?;
        r.live.then_some(r.handle)
    }

    /// The live object count.
    /// REQ: REQ-SPATIAL-013.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.objects.iter().filter(|r| r.live).count()
    }

    /// `on_sample` dependency-gated entry (`I-X3`/`I-X7`): an unchanged
    /// dependency vector touches nothing; a moved object still inside its
    /// envelope updates only its versions; otherwise it re-buckets.
    ///
    /// # Errors
    /// [`SpatialError::InvalidArg`] for a non-finite position or out-of-range
    /// cell; [`SpatialError::InvalidHandle`] for a dead handle;
    /// [`SpatialError::WriterConflict`] when another updater holds the writer.
    /// REQ: REQ-SPATIAL-013, REQ-SPATIAL-019.
    pub fn on_sample(&mut self, index: u32, p: Point3, deps: GridDeps) -> Result<(), SpatialError> {
        let slot = usize::try_from(index).map_err(|_| SpatialError::InvalidHandle)?;
        if slot >= self.objects.len() {
            return Err(SpatialError::InvalidHandle);
        }
        if !self.objects[slot].live {
            // First sample: bucket at the object's own declared cell.
            return self.with_writer(|g| g.insert_new(slot, p, deps));
        }
        if self.objects[slot].deps == deps {
            return Ok(());
        }
        if envelope_contains(&self.objects[slot], p) {
            self.objects[slot].deps = deps;
            self.keeps += 1;
            return Ok(());
        }
        self.with_writer(|g| g.rebucket(slot, p, deps))
    }

    /// First insertion of an object: allocate membership cells under one epoch.
    /// Caller holds the writer. REQ: REQ-SPATIAL-013.
    fn insert_new(&mut self, slot: usize, p: Point3, deps: GridDeps) -> Result<(), SpatialError> {
        let mut cells = [CellKey {
            cx: 0,
            cy: 0,
            cz: 0,
        }; MAX_OCCUPIED];
        let n = cells_of_envelope(p, self.config.slack, self.config.cell_size, &mut cells)?;
        let required = n as u32;
        self.preflight(0, required)?;
        let e = self.cell_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let result = self.apply_raw(&cells[..n], slot, p, deps);
        self.cell_epoch.store(e + 1, Ordering::Release);
        result
    }

    /// @brief Re-buckets `slot` to `p`: `remove ∪ add`, intersection kept.
    ///
    /// All edits run under an odd grid epoch restored even on failure (`I-X12`).
    /// Caller holds the writer.
    ///
    /// # Errors
    /// [`SpatialError::InvalidArg`] for a non-finite position;
    /// [`SpatialError::IndexFull`] when the pool would overflow.
    /// REQ: REQ-SPATIAL-042, REQ-SPATIAL-043.
    fn rebucket(&mut self, slot: usize, p: Point3, deps: GridDeps) -> Result<(), SpatialError> {
        let mut next = [CellKey {
            cx: 0,
            cy: 0,
            cz: 0,
        }; MAX_OCCUPIED];
        let n = cells_of_envelope(p, self.config.slack, self.config.cell_size, &mut next)?;
        // remove = old − new; add = new − old.
        let mut remove = [CellKey {
            cx: 0,
            cy: 0,
            cz: 0,
        }; MAX_OCCUPIED];
        let mut add = [CellKey {
            cx: 0,
            cy: 0,
            cz: 0,
        }; MAX_OCCUPIED];
        let (mut rn, mut an) = (0usize, 0usize);
        for c in self.objects[slot].cells() {
            if !next[..n].contains(c) {
                remove[rn] = *c;
                rn += 1;
            }
        }
        for c in &next[..n] {
            if !self.objects[slot].cells().contains(c) {
                add[an] = *c;
                an += 1;
            }
        }
        self.preflight(rn as u32, an as u32)?;
        let e = self.cell_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let result = self.apply_raw(&next[..n], slot, p, deps);
        self.cell_epoch.store(e + 1, Ordering::Release);
        result
    }

    /// Fits `remove`/`add` within the declared pool and per-object cell bound
    /// before any mutation (`I-X2`). Returns [`SpatialError::IndexFull`] when a
    /// bound would be exceeded.
    /// REQ: REQ-SPATIAL-052, REQ-SPATIAL-042.
    fn preflight(&self, remove: u32, add: u32) -> Result<(), SpatialError> {
        if add > self.config.max_cells_per_object {
            return Err(SpatialError::IndexFull);
        }
        // Freed `remove` nodes are immediately reusable by `add`.
        let free = self.free_nodes.len() as u64 + u64::from(remove);
        if u64::from(add) > free {
            return Err(SpatialError::IndexFull);
        }
        Ok(())
    }

    /// Core mutation: unlink old memberships, link new ones, record the
    /// envelope. Runs with the epoch already odd.
    /// REQ: REQ-SPATIAL-042.
    fn apply_raw(
        &mut self,
        new_cells: &[CellKey],
        slot: usize,
        p: Point3,
        deps: GridDeps,
    ) -> Result<(), SpatialError> {
        // Unlink every existing membership of this object from its cell lists.
        let old: Vec<CellKey> = self.objects[slot].cells().to_vec();
        for key in &old {
            self.unlink(*key, slot as u32);
        }
        // Link the new cells.
        let mut occupied = [CellKey {
            cx: 0,
            cy: 0,
            cz: 0,
        }; MAX_OCCUPIED];
        for (i, key) in new_cells.iter().enumerate() {
            self.link(*key, slot as u32)?;
            occupied[i] = *key;
        }
        let add_len = new_cells.len();
        let rec = &mut self.objects[slot];
        rec.center = p;
        rec.slack = self.config.slack;
        rec.occupied = occupied;
        rec.occupied_len = add_len as u8;
        rec.deps = deps;
        rec.live = true;
        rec.rebuckets += 1;
        self.rebuckets += 1;
        Ok(())
    }

    /// Links `slot` into `key`'s membership list, creating the cell if absent.
    /// REQ: REQ-SPATIAL-013.
    fn link(&mut self, key: CellKey, slot: u32) -> Result<(), SpatialError> {
        let cell = self.find_or_insert_cell(key)?;
        let node = self.free_nodes.pop().ok_or(SpatialError::InvalidConfig)?;
        let prev_head = self.cells[cell].head;
        self.nodes[node as usize] = MembershipNode {
            object: slot,
            next: prev_head,
        };
        self.cells[cell].head = node;
        self.cells[cell].version.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Unlinks `slot` from `key`'s membership list (backward-shift of the probe
    /// chain on the last member). Creates nothing.
    /// REQ: REQ-SPATIAL-042.
    fn unlink(&mut self, key: CellKey, slot: u32) {
        let Some(cell) = self.find_cell(key) else {
            return;
        };
        let mut prev = NODE_NONE;
        let mut cur = self.cells[cell].head;
        while cur != NODE_NONE {
            let next = self.nodes[cur as usize].next;
            if self.nodes[cur as usize].object == slot {
                if prev == NODE_NONE {
                    self.cells[cell].head = next;
                } else {
                    self.nodes[prev as usize].next = next;
                }
                self.free_nodes.push(cur);
                self.cells[cell].version.fetch_add(1, Ordering::AcqRel);
                break;
            }
            prev = cur;
            cur = next;
        }
        // Drop an empty cell (open addressing: backward-shift the probe chain).
        if self.cells[cell].head == NODE_NONE {
            self.remove_cell(cell);
        }
    }

    /// Probes for `key`; returns its slot index if present.
    /// REQ: REQ-SPATIAL-013.
    fn find_cell(&self, key: CellKey) -> Option<usize> {
        let mask = self.cells.len() - 1;
        let mut i = hash_key(key) as usize & mask;
        loop {
            let c = &self.cells[i];
            if !c.used {
                return None;
            }
            if c.key == key {
                return Some(i);
            }
            i = (i + 1) & mask;
        }
    }

    /// Finds or claims the open-addressing slot for `key`, growing occupancy
    /// only within the declared table.
    /// REQ: REQ-SPATIAL-013, REQ-SPATIAL-052.
    fn find_or_insert_cell(&mut self, key: CellKey) -> Result<usize, SpatialError> {
        let mask = self.cells.len() - 1;
        let mut i = hash_key(key) as usize & mask;
        loop {
            if !self.cells[i].used {
                self.cells[i].key = key;
                self.cells[i].used = true;
                self.cells[i].head = NODE_NONE;
                return Ok(i);
            }
            if self.cells[i].key == key {
                return Ok(i);
            }
            i = (i + 1) & mask;
        }
    }

    /// Removes an empty cell using backward-shift deletion (`I-X2`): shift the
    /// following probe cluster back so no tombstone accumulates.
    /// REQ: REQ-SPATIAL-013.
    fn remove_cell(&mut self, cell: usize) {
        let mask = self.cells.len() - 1;
        self.cells[cell].used = false;
        let mut i = cell;
        loop {
            let j = (i + 1) & mask;
            if !self.cells[j].used {
                return;
            }
            let home = hash_key(self.cells[j].key) as usize & mask;
            // Only shift when `j`'s probe path crossed the emptied slot `i`.
            let crossed = if i <= j {
                home <= i || home > j
            } else {
                home <= i && home > j
            };
            if !crossed {
                return;
            }
            self.cells[i] = CellSlot {
                key: self.cells[j].key,
                used: true,
                version: AtomicU32::new(self.cells[j].version.load(Ordering::Relaxed)),
                head: self.cells[j].head,
            };
            self.cells[j].used = false;
            i = j;
        }
    }

    /// Single-writer critical section: at most one updater mutates the grid at a
    /// time; a second writer gets [`SpatialError::WriterConflict`] without
    /// waiting (`I-X12`, single-writer discipline). The flag is released on
    /// every exit path, including errors.
    /// REQ: REQ-SPATIAL-042.
    fn with_writer<R>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<R, SpatialError>,
    ) -> Result<R, SpatialError> {
        if self
            .writer_busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(SpatialError::WriterConflict);
        }
        let out = f(self);
        self.writer_busy.store(false, Ordering::Release);
        out
    }

    /// Removes `slot` from the index, bumping its generation so stale handles
    /// invalidate immediately (`I-X4`).
    ///
    /// # Errors
    /// [`SpatialError::InvalidHandle`] for a dead handle;
    /// [`SpatialError::WriterConflict`] when another updater holds the writer.
    /// REQ: REQ-SPATIAL-013.
    pub fn remove(&mut self, slot: u32) -> Result<(), SpatialError> {
        let idx = usize::try_from(slot).map_err(|_| SpatialError::InvalidHandle)?;
        let Some(rec) = self.objects.get(idx) else {
            return Err(SpatialError::InvalidHandle);
        };
        if !rec.live {
            return Err(SpatialError::InvalidHandle);
        }
        self.with_writer(|g| {
            let old: Vec<CellKey> = g.objects[idx].cells().to_vec();
            let e = g.cell_epoch.fetch_add(1, Ordering::AcqRel) + 1;
            for key in &old {
                g.unlink(*key, slot);
            }
            let rec = &mut g.objects[idx];
            rec.live = false;
            rec.occupied_len = 0;
            rec.handle.generation = rec.handle.generation.wrapping_add(1);
            g.cell_epoch.store(e + 1, Ordering::Release);
            Ok(())
        })
    }

    /// Queries the candidate superset within `radius` of a typed point
    /// (`I-X5`/`I-X6`). The prospective cell count is computed with checked
    /// arithmetic before enumeration; above `max_query_cells` the query uses the
    /// bounded object-table fallback (`I-X13`). The grid epoch is re-read after
    /// the scan: any change means a partial multi-cell view would have been
    /// observed, so the query returns [`SpatialError::WriterConflict`] (the
    /// doc's `CONCURRENT_UPDATE`) rather than a partial set (`I-X11`).
    ///
    /// # Errors
    /// [`SpatialError::InvalidArg`] for a non-finite/out-of-range query;
    /// [`SpatialError::WriterConflict`] when the epoch changed across the scan.
    /// REQ: REQ-SPATIAL-046, REQ-SPATIAL-043, REQ-SPATIAL-045.
    pub fn nearby(
        &self,
        point: TypedPoint,
        radius: f64,
        scratch: &mut CandidateSet,
    ) -> Result<ProximityOutcome, SpatialError> {
        scratch.clear();
        if !radius.is_finite() || radius < 0.0 {
            return Err(SpatialError::InvalidArg);
        }
        let e1 = self.stable_epoch()?;
        let (lo_c, hi_c) = self.sphere_bounds(point, radius)?;
        let overflow;
        let path;
        if cell_count(lo_c, hi_c)? > i64::from(self.config.max_query_cells) {
            overflow = self.scan_objects(scratch);
            path = QueryPath::ObjectFallback;
        } else {
            overflow = self.scan_cells(lo_c, hi_c, scratch);
            path = QueryPath::Cells;
        }
        if self.cell_epoch.load(Ordering::Acquire) != e1 {
            return Err(SpatialError::WriterConflict);
        }
        Ok(finish(path, scratch, overflow))
    }

    /// The unit-cell bounds of a query sphere, computed with checked arithmetic
    /// before enumeration. REQ: REQ-SPATIAL-046.
    fn sphere_bounds(
        &self,
        point: TypedPoint,
        radius: f64,
    ) -> Result<(CellKey, CellKey), SpatialError> {
        let p = point.point;
        let Some(lo) = subtract(p, radius) else {
            return Err(SpatialError::InvalidArg);
        };
        let Some(hi) = add(p, radius) else {
            return Err(SpatialError::InvalidArg);
        };
        Ok((
            CellKey {
                cx: cell_of(lo.x, self.config.cell_size)?,
                cy: cell_of(lo.y, self.config.cell_size)?,
                cz: cell_of(lo.z, self.config.cell_size)?,
            },
            CellKey {
                cx: cell_of(hi.x, self.config.cell_size)?,
                cy: cell_of(hi.y, self.config.cell_size)?,
                cz: cell_of(hi.z, self.config.cell_size)?,
            },
        ))
    }

    /// Scans the fixed object table into `scratch` (bounded fallback). The
    /// result is a candidate set: each live object contributes exactly one
    /// handle, so the scratch bound is the declared object count, never the
    /// cell span (`I-X6`). REQ: REQ-SPATIAL-046.
    fn scan_objects(&self, scratch: &mut CandidateSet) -> bool {
        let mut overflow = false;
        for rec in &self.objects {
            if rec.live && !scratch.contains(rec.handle) {
                overflow |= !scratch.push(rec.handle);
            }
        }
        overflow
    }

    /// Scans the cells in `lo..=hi` into `scratch`. An object whose fat envelope
    /// spans many cells is delivered once — cells are deduplicated against the
    /// existing scratch so the candidate bound is the declared object count, not
    /// the query cell span (`I-X6`). REQ: REQ-SPATIAL-046.
    fn scan_cells(&self, lo: CellKey, hi: CellKey, scratch: &mut CandidateSet) -> bool {
        let mut overflow = false;
        let mut cx = lo.cx;
        loop {
            let mut cy = lo.cy;
            loop {
                let mut cz = lo.cz;
                loop {
                    if let Some(cell) = self.find_cell(CellKey { cx, cy, cz }) {
                        let mut cur = self.cells[cell].head;
                        while cur != NODE_NONE {
                            let obj = self.nodes[cur as usize].object as usize;
                            let rec = &self.objects[obj];
                            if rec.live && !scratch.contains(rec.handle) {
                                overflow |= !scratch.push(rec.handle);
                            }
                            cur = self.nodes[cur as usize].next;
                        }
                    }
                    if cz == hi.cz {
                        break;
                    }
                    cz += 1;
                }
                if cy == hi.cy {
                    break;
                }
                cy += 1;
            }
            if cx == hi.cx {
                break;
            }
            cx += 1;
        }
        overflow
    }

    /// Queries the candidate superset overlapping a typed AABB (`I-X5`),
    /// applying the same epoch handshake and cell-count bound as [`Self::nearby`].
    ///
    /// # Errors
    /// As [`Self::nearby`].
    /// REQ: REQ-SPATIAL-046, REQ-SPATIAL-043.
    pub fn region(
        &self,
        aabb: TypedAabb,
        scratch: &mut CandidateSet,
    ) -> Result<ProximityOutcome, SpatialError> {
        scratch.clear();
        if !aabb.min.x.is_finite()
            || !aabb.min.y.is_finite()
            || !aabb.min.z.is_finite()
            || !aabb.max.x.is_finite()
            || !aabb.max.y.is_finite()
            || !aabb.max.z.is_finite()
        {
            return Err(SpatialError::InvalidArg);
        }
        let e1 = self.stable_epoch()?;
        let lo_c = CellKey {
            cx: cell_of(aabb.min.x, self.config.cell_size)?,
            cy: cell_of(aabb.min.y, self.config.cell_size)?,
            cz: cell_of(aabb.min.z, self.config.cell_size)?,
        };
        let hi_c = CellKey {
            cx: cell_of(aabb.max.x, self.config.cell_size)?,
            cy: cell_of(aabb.max.y, self.config.cell_size)?,
            cz: cell_of(aabb.max.z, self.config.cell_size)?,
        };
        let overflow;
        let path;
        if cell_count(lo_c, hi_c)? > i64::from(self.config.max_query_cells) {
            overflow = self.scan_objects(scratch);
            path = QueryPath::ObjectFallback;
        } else {
            overflow = self.scan_cells(lo_c, hi_c, scratch);
            path = QueryPath::Cells;
        }
        if self.cell_epoch.load(Ordering::Acquire) != e1 {
            return Err(SpatialError::WriterConflict);
        }
        Ok(finish(path, scratch, overflow))
    }

    /// Merges the grid's candidate superset within `radius` of a typed point
    /// into an existing scratch without clearing it, for the two-source union
    /// (`I-X6`). Applies the checked cell-count bound and object-table fallback
    /// (`I-X13`) and honors the per-cell version handshake. The caller owns the
    /// epoch handshake. Returns the enumeration path.
    ///
    /// # Errors
    /// [`SpatialError::InvalidArg`] for a non-finite/out-of-range query.
    /// REQ: REQ-SPATIAL-020, REQ-SPATIAL-046.
    pub(crate) fn merge_nearby(
        &self,
        point: TypedPoint,
        radius: f64,
        scratch: &mut CandidateSet,
    ) -> Result<QueryPath, SpatialError> {
        if !radius.is_finite() || radius < 0.0 {
            return Err(SpatialError::InvalidArg);
        }
        let (lo_c, hi_c) = self.sphere_bounds(point, radius)?;
        if cell_count(lo_c, hi_c)? > i64::from(self.config.max_query_cells) {
            self.merge_objects(scratch);
            Ok(QueryPath::ObjectFallback)
        } else {
            self.merge_cells(lo_c, hi_c, scratch);
            Ok(QueryPath::Cells)
        }
    }

    /// Merges the grid's candidate superset overlapping a typed AABB into an
    /// existing scratch (`I-X6`). REQ: REQ-SPATIAL-020.
    ///
    /// # Errors
    /// As [`Self::merge_nearby`].
    pub(crate) fn merge_region(
        &self,
        aabb: TypedAabb,
        scratch: &mut CandidateSet,
    ) -> Result<QueryPath, SpatialError> {
        if !aabb.min.x.is_finite()
            || !aabb.min.y.is_finite()
            || !aabb.min.z.is_finite()
            || !aabb.max.x.is_finite()
            || !aabb.max.y.is_finite()
            || !aabb.max.z.is_finite()
        {
            return Err(SpatialError::InvalidArg);
        }
        let lo_c = CellKey {
            cx: cell_of(aabb.min.x, self.config.cell_size)?,
            cy: cell_of(aabb.min.y, self.config.cell_size)?,
            cz: cell_of(aabb.min.z, self.config.cell_size)?,
        };
        let hi_c = CellKey {
            cx: cell_of(aabb.max.x, self.config.cell_size)?,
            cy: cell_of(aabb.max.y, self.config.cell_size)?,
            cz: cell_of(aabb.max.z, self.config.cell_size)?,
        };
        if cell_count(lo_c, hi_c)? > i64::from(self.config.max_query_cells) {
            self.merge_objects(scratch);
            Ok(QueryPath::ObjectFallback)
        } else {
            self.merge_cells(lo_c, hi_c, scratch);
            Ok(QueryPath::Cells)
        }
    }

    /// Merges `lo..=hi` cell memberships into an existing scratch, deduplicating
    /// against handles already present. REQ: REQ-SPATIAL-020.
    fn merge_cells(&self, lo: CellKey, hi: CellKey, scratch: &mut CandidateSet) {
        let mut cx = lo.cx;
        loop {
            let mut cy = lo.cy;
            loop {
                let mut cz = lo.cz;
                loop {
                    if let Some(cell) = self.find_cell(CellKey { cx, cy, cz }) {
                        let mut cur = self.cells[cell].head;
                        while cur != NODE_NONE {
                            let obj = self.nodes[cur as usize].object as usize;
                            let rec = &self.objects[obj];
                            if rec.live && !scratch.contains(rec.handle) {
                                scratch.merge(rec.handle);
                            }
                            cur = self.nodes[cur as usize].next;
                        }
                    }
                    if cz == hi.cz {
                        break;
                    }
                    cz += 1;
                }
                if cy == hi.cy {
                    break;
                }
                cy += 1;
            }
            if cx == hi.cx {
                break;
            }
            cx += 1;
        }
    }

    /// Merges every live object into an existing scratch (bounded fallback),
    /// deduplicating. REQ: REQ-SPATIAL-020.
    fn merge_objects(&self, scratch: &mut CandidateSet) {
        for rec in &self.objects {
            if rec.live && !scratch.contains(rec.handle) {
                scratch.merge(rec.handle);
            }
        }
    }

    /// The grid's current stable epoch, for a caller performing an epoch
    /// handshake around an external union scan (`I-X11`).
    /// REQ: REQ-SPATIAL-043.
    ///
    /// # Errors
    /// [`SpatialError::WriterConflict`] when an update is in flight.
    pub(crate) fn begin_query(&self) -> Result<u64, SpatialError> {
        self.stable_epoch()
    }

    /// Whether the epoch still equals `observed` (coherent union view).
    /// REQ: REQ-SPATIAL-043.
    #[must_use]
    pub(crate) fn epoch_unchanged(&self, observed: u64) -> bool {
        self.cell_epoch.load(Ordering::Acquire) == observed
    }

    /// Reads `grid_epoch`, returning it only when even (stable). `I-X11`.
    /// REQ: REQ-SPATIAL-043.
    fn stable_epoch(&self) -> Result<u64, SpatialError> {
        let e = self.cell_epoch.load(Ordering::Acquire);
        if e % 2 == 1 {
            // A move is in flight; a single bounded retry, then surface it.
            let e2 = self.cell_epoch.load(Ordering::Acquire);
            if e2 % 2 == 1 {
                return Err(SpatialError::WriterConflict);
            }
            return Ok(e2);
        }
        Ok(e)
    }
}

/// Whether `p` still lies inside `rec`'s slack envelope (`I-X3`).
/// REQ: REQ-SPATIAL-019.
fn envelope_contains(rec: &ObjectRecord, p: Point3) -> bool {
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
        return false;
    }
    (p.x - rec.center.x).abs() <= rec.slack
        && (p.y - rec.center.y).abs() <= rec.slack
        && (p.z - rec.center.z).abs() <= rec.slack
}

/// A stable, power-of-two-friendly cell hash (splitmix-style finalizer).
/// REQ: REQ-SPATIAL-013.
fn hash_key(key: CellKey) -> u32 {
    let mut h = 0x9E37_79B9u32;
    h = h.wrapping_add(key.cx as u32).rotate_left(13);
    h = h.wrapping_add(key.cy as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_add(key.cz as u32).rotate_left(7);
    h ^= h >> 17;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(objects: u32) -> GridConfig {
        GridConfig {
            cell_size: 1.0,
            max_objects: objects,
            max_cells_per_object: MAX_OCCUPIED as u32,
            max_query_cells: 512,
            slack: 0.0,
        }
    }

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn deps(v: u64) -> GridDeps {
        GridDeps {
            own: v,
            ancestor: v,
            calibration: v,
            topology: v,
        }
    }

    /// REQ: REQ-SPATIAL-013 — cell floor is toward `−∞` and `NaN`/huge values
    /// are rejected (`I-X14`).
    #[test]
    fn cell_floor_and_rejects() {
        assert_eq!(cell_of(0.0, 1.0), Ok(0));
        assert_eq!(cell_of(-0.5, 1.0), Ok(-1));
        assert_eq!(cell_of(1.0, 0.5), Ok(2));
        assert_eq!(cell_of(f64::NAN, 1.0), Err(SpatialError::InvalidArg));
        assert_eq!(cell_of(f64::INFINITY, 1.0), Err(SpatialError::InvalidArg));
        assert_eq!(cell_of(0.0, 0.0), Err(SpatialError::InvalidArg));
        assert_eq!(
            cell_of(1e300, 1.0),
            Err(SpatialError::InvalidArg),
            "out-of-i32 cell rejected"
        );
    }

    /// REQ: REQ-SPATIAL-052 — the cell-hash slot count is `⌈M/α_max⌉` rounded
    /// up to a power of two.
    #[test]
    fn hash_slots_derivation() {
        // ceil(4096·27 / 0.70) = 157_989 → next power of two 262_144.
        assert_eq!(cell_hash_slots(4096, 27), Some(262_144));
        assert_eq!(membership_capacity(4, 3), Some(12));
        assert_eq!(membership_capacity(u32::MAX, 2), None);
    }

    /// REQ: REQ-SPATIAL-013 — insert, query, and remove round-trip; a removed
    /// object invalidates its handle generation (`I-X4`).
    #[test]
    fn insert_query_remove() {
        let mut g = HashGrid::activate(cfg(8)).unwrap_or_else(|e| unreachable!("{e}"));
        g.on_sample(0, p(1.5, 0.0, 0.0), deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        g.on_sample(1, p(10.0, 0.0, 0.0), deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        let mut scratch = CandidateSet::with_capacity(8).unwrap_or_else(|e| unreachable!("{e}"));
        let outcome = g
            .nearby(
                TypedPoint {
                    point: p(1.5, 0.0, 0.0),
                    frame: 0,
                },
                0.5,
                &mut scratch,
            )
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(outcome.path, QueryPath::Cells);
        assert!(scratch.handles().contains(&SpatialHandle {
            index: 0,
            generation: 0
        }));
        g.remove(0).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(g.live_count(), 1);
        assert_eq!(g.handle_at(0), None);
    }

    /// REQ: REQ-SPATIAL-019 — a sub-slack jitter causes zero re-buckets
    /// (`I-X3`).
    #[test]
    fn motion_slack_zero_rebucket() {
        let mut c = cfg(4);
        c.slack = 0.5;
        let mut g = HashGrid::activate(c).unwrap_or_else(|e| unreachable!("{e}"));
        g.on_sample(0, p(0.0, 0.0, 0.0), deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        let base = g.rebucket_count();
        for i in 1..5 {
            g.on_sample(0, p(0.1 * i as f64, 0.0, 0.0), deps(i + 1))
                .unwrap_or_else(|e| unreachable!("{e}"));
        }
        assert_eq!(g.rebucket_count(), base, "within slack ⇒ no re-bucket");
        assert_eq!(g.keep_count(), 4, "versions updated in place");
    }

    /// REQ: REQ-SPATIAL-019 — crossing the envelope re-buckets exactly once.
    #[test]
    fn motion_slack_exit_rebuckets() {
        let mut c = cfg(4);
        c.slack = 0.25;
        let mut g = HashGrid::activate(c).unwrap_or_else(|e| unreachable!("{e}"));
        g.on_sample(0, p(0.0, 0.0, 0.0), deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        let base = g.rebucket_count();
        g.on_sample(0, p(5.0, 0.0, 0.0), deps(2))
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(g.rebucket_count(), base + 1);
    }

    /// REQ: REQ-SPATIAL-020 — a saturated scratch returns `OutputTooSmall` with
    /// the required capacity, never silent truncation (`I-X6`).
    #[test]
    fn candidate_saturation() {
        let mut g = HashGrid::activate(cfg(8)).unwrap_or_else(|e| unreachable!("{e}"));
        for i in 0..4 {
            g.on_sample(i, p(0.0, 0.0, 0.0), deps(1))
                .unwrap_or_else(|e| unreachable!("{e}"));
        }
        let mut scratch = CandidateSet::with_capacity(1).unwrap_or_else(|e| unreachable!("{e}"));
        let outcome = g
            .region(
                TypedAabb {
                    min: p(0.0, 0.0, 0.0),
                    max: p(0.0, 0.0, 0.0),
                    frame: 0,
                },
                &mut scratch,
            )
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let required = outcome
            .overflow
            .unwrap_or_else(|| unreachable!("expected overflow"));
        assert!(required >= 4);
    }

    /// REQ: REQ-SPATIAL-052 — a capacity smaller than the requested objects
    /// refuses new insertions rather than growing (`I-X2`).
    #[test]
    fn capacity_refused() {
        let mut c = cfg(2);
        c.max_cells_per_object = 1;
        let mut g = HashGrid::activate(c).unwrap_or_else(|e| unreachable!("{e}"));
        g.on_sample(0, p(0.0, 0.0, 0.0), deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        g.on_sample(1, p(1.0, 0.0, 0.0), deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        // Pool `M = 2 × 1 = 2` nodes; a third distinct cell cannot be linked.
        assert!(g.on_sample(3, p(50.0, 0.0, 0.0), deps(1)).is_err());
    }
}
