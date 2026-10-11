//! Two-source proximity union runtime
//! ([spatial-index.md](../../docs/algorithms/spatial/spatial-index.md)
//! §Functions `nearby`/`region`, `I-X5`/`I-X6`; [`spatial_architecture.md`](../../docs/spatial_architecture.md)
//! §Proximity Query Semantics). A [`ProximityIndex`] owns the dynamic
//! [`HashGrid`] and the runtime-compiled static [`StaticBvh`] and answers a
//! query with the union of the two candidate sources: the grid's superset cells
//! plus the static tree's AABB-overlap hits, deduplicated and delivered in the
//! anchor frame. The candidate set is a deliberate superset — false positives
//! are allowed, misses are forbidden — and a query frame other than the anchor
//! is rejected through the standard conversion path rather than silently
//! misclassified (`I-X5`). The grid epoch is re-read after both scans so a
//! racing move yields [`SpatialError::WriterConflict`] (`CONCURRENT_UPDATE`),
//! never a partial union (`I-X11`).
//!
//! REQ: REQ-SPATIAL-006, REQ-SPATIAL-013, REQ-SPATIAL-018, REQ-SPATIAL-020,
//! REQ-SPATIAL-043, REQ-SPATIAL-045, REQ-SPATIAL-046.

use crate::grid::{
    CandidateSet, GridConfig, GridDeps, HashGrid, ProximityOutcome, TypedAabb, TypedPoint,
};
use crate::types::{Point3, SpatialError, SpatialHandle};

/// An axis-aligned bounding box in the anchor frame, meters (`I-X3`).
/// REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticAabb {
    /// Minimum corner, meters.
    pub min: Point3,
    /// Maximum corner, meters.
    pub max: Point3,
}

impl StaticAabb {
    /// The smallest box containing both boxes (`I-X3`: union over-approximates).
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: Point3 {
                x: self.min.x.min(other.min.x),
                y: self.min.y.min(other.min.y),
                z: self.min.z.min(other.min.z),
            },
            max: Point3 {
                x: self.max.x.max(other.max.x),
                y: self.max.y.max(other.max.y),
                z: self.max.z.max(other.max.z),
            },
        }
    }

    /// Whether every component is finite and `min <= max`.
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let finite = |p: &Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
        finite(&self.min)
            && finite(&self.max)
            && self.min.x <= self.max.x
            && self.min.y <= self.max.y
            && self.min.z <= self.max.z
    }

    /// Whether the two boxes overlap on every axis (`I-X3`).
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
            && self.min.z <= other.max.z
            && other.min.z <= self.max.z
    }

    /// The centroid of the box. REQ: REQ-SPATIAL-006.
    #[must_use]
    fn centroid(&self) -> Point3 {
        Point3 {
            x: (self.min.x + self.max.x) * 0.5,
            y: (self.min.y + self.max.y) * 0.5,
            z: (self.min.z + self.max.z) * 0.5,
        }
    }

    /// Half the surface area (the SAH cost metric). REQ: REQ-SPATIAL-006.
    #[must_use]
    fn half_area(&self) -> f64 {
        if !self.is_valid() {
            return 0.0;
        }
        let dx = self.max.x - self.min.x;
        let dy = self.max.y - self.min.y;
        let dz = self.max.z - self.min.z;
        dx * dy + dy * dz + dz * dx
    }
}

/// One static environment primitive (walls, shelves, keep-out zones) with a
/// stable entity id. REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticObject {
    /// Stable entity identity.
    pub entity: u32,
    /// Bounds in the anchor frame.
    pub aabb: StaticAabb,
}

/// A compiled static BVH node in the fixed output layout (`I-X3`); node `0` is
/// the root. REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticNode {
    /// Node bounds (union of children for internal nodes).
    pub aabb: StaticAabb,
    /// Left child index, or [`STATIC_LEAF`] for a leaf.
    pub left: u32,
    /// Right child index, or [`STATIC_LEAF`] for a leaf.
    pub right: u32,
    /// First index into the leaf table (leaf nodes only).
    pub leaf_start: u32,
    /// Primitive count (leaf nodes only).
    pub leaf_count: u32,
}

/// Sentinel marking a static node as a leaf.
/// REQ: REQ-SPATIAL-006.
pub const STATIC_LEAF: u32 = u32::MAX;

/// Namespace offset applied to a static primitive's entity id when it becomes a
/// [`SpatialHandle`] candidate. Dynamic grid handles index the fixed object
/// table (`[0, max_objects)`), so static handles are lifted into a disjoint id
/// range; without this the union's deduplication could collapse a dynamic and a
/// static candidate into one handle and silently miss (`I-X6`).
/// REQ: REQ-SPATIAL-020.
pub const STATIC_ENTITY_BASE: u32 = 1_000_000;

/// The maximum leaf size before a deterministic SAH split is attempted.
/// REQ: REQ-SPATIAL-006.
const STATIC_MAX_LEAF: usize = 4;

/// A runtime-compiled static BVH: the runtime counterpart of the build-time
/// compiler in `drm-workspace`, compiled in the anchor frame. Every internal
/// box is the union of its children, so a query may return false positives but
/// never a miss (`I-X3`). REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticBvh {
    nodes: Vec<StaticNode>,
    leaves: Vec<StaticObject>,
    root: StaticAabb,
    empty: bool,
}

impl StaticBvh {
    /// Whether the tree holds no primitives. REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// The compiled node count (fixed after compile; feeds `I-X9`).
    /// REQ: REQ-SPATIAL-018.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The compiled primitive count. REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.leaves.len()
    }

    /// Merges every primitive whose box overlaps `query` into `scratch`, as
    /// [`SpatialHandle`]s keyed by entity lifted above the dynamic object-table
    /// range ([`STATIC_ENTITY_BASE`]) with generation `0`, deduplicating against
    /// handles already present (`I-X6`). REQ: REQ-SPATIAL-006, REQ-SPATIAL-020.
    pub fn merge_overlap(&self, query: &StaticAabb, scratch: &mut CandidateSet) {
        if self.empty || !self.root.overlaps(query) {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(idx) = stack.pop() {
            let Some(node) = self.nodes.get(idx as usize) else {
                continue;
            };
            if !node.aabb.overlaps(query) {
                continue;
            }
            if node.left == STATIC_LEAF {
                let start = node.leaf_start as usize;
                let end = start.saturating_add(node.leaf_count as usize);
                for prim in self.leaves.get(start..end).unwrap_or_default() {
                    if prim.aabb.overlaps(query) {
                        let handle = SpatialHandle {
                            index: prim.entity.saturating_add(STATIC_ENTITY_BASE),
                            generation: 0,
                        };
                        if !scratch.contains(handle) {
                            scratch.merge(handle);
                        }
                    }
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
    }
}

/// Compiles static primitives into a runtime BVH with a deterministic
/// surface-area-heuristic split (largest centroid extent, ties by entity id),
/// so the same input compiles identically twice (`I-X3`/`I-D1`).
/// REQ: REQ-SPATIAL-006.
#[must_use]
pub fn compile_static_bvh(primitives: &[StaticObject]) -> StaticBvh {
    let mut items: Vec<StaticObject> = primitives
        .iter()
        .copied()
        .filter(|p| p.aabb.is_valid())
        .collect();
    items.sort_unstable_by(|a, b| a.entity.cmp(&b.entity));
    if items.is_empty() {
        return StaticBvh {
            nodes: Vec::new(),
            leaves: Vec::new(),
            root: StaticAabb {
                min: Point3 {
                    x: f64::INFINITY,
                    y: f64::INFINITY,
                    z: f64::INFINITY,
                },
                max: Point3 {
                    x: f64::NEG_INFINITY,
                    y: f64::NEG_INFINITY,
                    z: f64::NEG_INFINITY,
                },
            },
            empty: true,
        };
    }
    let root = items.iter().fold(empty_box(), |acc, p| acc.union(p.aabb));
    let mut builder = StaticBuilder {
        nodes: Vec::new(),
        leaves: Vec::new(),
    };
    let idx = builder.build(&mut items);
    debug_assert_eq!(idx, 0);
    StaticBvh {
        nodes: builder.nodes,
        leaves: builder.leaves,
        root,
        empty: false,
    }
}

/// The empty box (identity for [`StaticAabb::union`]). REQ: REQ-SPATIAL-006.
fn empty_box() -> StaticAabb {
    StaticAabb {
        min: Point3 {
            x: f64::INFINITY,
            y: f64::INFINITY,
            z: f64::INFINITY,
        },
        max: Point3 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
            z: f64::NEG_INFINITY,
        },
    }
}

/// Recursive static builder producing parent-before-child indices.
/// REQ: REQ-SPATIAL-006.
struct StaticBuilder {
    nodes: Vec<StaticNode>,
    leaves: Vec<StaticObject>,
}

impl StaticBuilder {
    /// Builds one node over `items`, returns its index. REQ: REQ-SPATIAL-006.
    fn build(&mut self, items: &mut [StaticObject]) -> u32 {
        let aabb = items.iter().fold(empty_box(), |acc, p| acc.union(p.aabb));
        let idx = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(StaticNode {
            aabb,
            left: STATIC_LEAF,
            right: STATIC_LEAF,
            leaf_start: 0,
            leaf_count: 0,
        });
        if items.len() <= STATIC_MAX_LEAF || static_extent(items) <= 0.0 {
            let leaf_start = u32::try_from(self.leaves.len()).unwrap_or(u32::MAX);
            self.leaves.extend_from_slice(items);
            self.nodes[idx as usize] = StaticNode {
                aabb,
                left: STATIC_LEAF,
                right: STATIC_LEAF,
                leaf_start,
                leaf_count: u32::try_from(items.len()).unwrap_or(u32::MAX),
            };
            return idx;
        }
        let mid = static_sah_split(items);
        let (left_items, right_items) = items.split_at_mut(mid);
        let left = self.build(left_items);
        let right = self.build(right_items);
        self.nodes[idx as usize] = StaticNode {
            aabb,
            left,
            right,
            leaf_start: 0,
            leaf_count: 0,
        };
        idx
    }
}

/// The largest centroid extent axis, or `0.0` when degenerate.
/// REQ: REQ-SPATIAL-006.
fn static_extent(items: &[StaticObject]) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in items {
        let c = p.aabb.centroid();
        let cs = [c.x, c.y, c.z];
        for i in 0..3 {
            lo[i] = lo[i].min(cs[i]);
            hi[i] = hi[i].max(cs[i]);
        }
    }
    (0..3).map(|i| hi[i] - lo[i]).fold(0.0_f64, f64::max)
}

/// Deterministic static SAH split. REQ: REQ-SPATIAL-006.
fn static_sah_split(items: &mut [StaticObject]) -> usize {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in items.iter() {
        let c = p.aabb.centroid();
        let cs = [c.x, c.y, c.z];
        for i in 0..3 {
            lo[i] = lo[i].min(cs[i]);
            hi[i] = hi[i].max(cs[i]);
        }
    }
    let mut axis = 0usize;
    let mut best_extent = -1.0_f64;
    for i in 0..3 {
        let e = hi[i] - lo[i];
        if e > best_extent {
            best_extent = e;
            axis = i;
        }
    }
    items.sort_unstable_by(|a, b| {
        let ca = a.aabb.centroid();
        let cb = b.aabb.centroid();
        let ca = [ca.x, ca.y, ca.z][axis];
        let cb = [cb.x, cb.y, cb.z][axis];
        ca.partial_cmp(&cb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.entity.cmp(&b.entity))
    });
    let n = items.len();
    let count = |v: usize| f64::from(u32::try_from(v).unwrap_or(u32::MAX));
    let mut best_cost = f64::INFINITY;
    let mut best_mid = n / 2;
    let mut left = empty_box();
    let mut suffix = vec![empty_box(); n];
    for i in (0..n).rev() {
        suffix[i] = if i == n - 1 {
            items[i].aabb
        } else {
            items[i].aabb.union(suffix[i + 1])
        };
    }
    for mid in 1..n {
        left = left.union(items[mid - 1].aabb);
        let right = suffix[mid];
        let cost = left.half_area() * count(mid) + right.half_area() * count(n - mid);
        if cost < best_cost {
            best_cost = cost;
            best_mid = mid;
        }
    }
    best_mid
}

/// A two-source proximity index: the dynamic hash grid and the compiled static
/// BVH, unioned in the declared anchor frame (`I-X5`/`I-X6`).
/// REQ: REQ-SPATIAL-013, REQ-SPATIAL-020.
#[derive(Debug)]
pub struct ProximityIndex {
    grid: HashGrid,
    statics: StaticBvh,
    anchor: u32,
}

impl ProximityIndex {
    /// Activates the index from a declared grid capacity (`index.drm`) and the
    /// static primitives, all fixed before deployment (`I-X2`).
    ///
    /// # Errors
    /// As [`HashGrid::activate`].
    /// REQ: REQ-SPATIAL-013, REQ-SPATIAL-020.
    pub fn activate(
        config: GridConfig,
        anchor: u32,
        statics: &[StaticObject],
    ) -> Result<Self, SpatialError> {
        Ok(Self {
            grid: HashGrid::activate(config)?,
            statics: compile_static_bvh(statics),
            anchor,
        })
    }

    /// The declared anchor frame of the index (`I-X5`).
    /// REQ: REQ-SPATIAL-020.
    #[must_use]
    pub const fn anchor(&self) -> u32 {
        self.anchor
    }

    /// The dynamic grid (for `on_sample`/`remove` updates).
    /// REQ: REQ-SPATIAL-013.
    #[must_use]
    pub fn grid_mut(&mut self) -> &mut HashGrid {
        &mut self.grid
    }

    /// The dynamic grid. REQ: REQ-SPATIAL-013.
    #[must_use]
    pub const fn grid(&self) -> &HashGrid {
        &self.grid
    }

    /// The compiled static BVH. REQ: REQ-SPATIAL-006.
    #[must_use]
    pub const fn statics(&self) -> &StaticBvh {
        &self.statics
    }

    /// A dependency-gated sample into the dynamic grid (`I-X3`/`I-X7`).
    ///
    /// # Errors
    /// As [`HashGrid::on_sample`].
    /// REQ: REQ-SPATIAL-013, REQ-SPATIAL-019.
    pub fn on_sample(&mut self, index: u32, p: Point3, deps: GridDeps) -> Result<(), SpatialError> {
        self.grid.on_sample(index, p, deps)
    }

    /// The two-source union within `radius` of a typed point (`I-X6`): grid
    /// superset candidates plus static AABB-overlap hits, deduplicated, in the
    /// anchor frame. A query frame other than the anchor is rejected through
    /// the standard conversion path (`I-X5`).
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] when `point.frame` is not the anchor;
    /// [`SpatialError::InvalidArg`] for a non-finite/out-of-range query;
    /// [`SpatialError::WriterConflict`] when a move raced the scan (`I-X11`).
    /// REQ: REQ-SPATIAL-006, REQ-SPATIAL-020, REQ-SPATIAL-043.
    pub fn nearby(
        &self,
        point: TypedPoint,
        radius: f64,
        scratch: &mut CandidateSet,
    ) -> Result<ProximityOutcome, SpatialError> {
        scratch.clear();
        if point.frame != self.anchor {
            // Standard frame-conversion path is unavailable here; never treat a
            // foreign frame as anchor-space coordinates (I-X5).
            return Err(SpatialError::PathNotFound);
        }
        let e1 = self.grid.begin_query()?;
        let path = self.grid.merge_nearby(point, radius, scratch)?;
        self.statics
            .merge_overlap(&sphere_aabb(point.point, radius)?, scratch);
        if !self.grid.epoch_unchanged(e1) {
            return Err(SpatialError::WriterConflict);
        }
        Ok(ProximityOutcome {
            path,
            overflow: overflow_of(scratch),
        })
    }

    /// The two-source union overlapping a typed AABB (`I-X6`), same contract as
    /// [`Self::nearby`].
    ///
    /// # Errors
    /// As [`Self::nearby`].
    /// REQ: REQ-SPATIAL-006, REQ-SPATIAL-020, REQ-SPATIAL-043.
    pub fn region(
        &self,
        aabb: TypedAabb,
        scratch: &mut CandidateSet,
    ) -> Result<ProximityOutcome, SpatialError> {
        scratch.clear();
        if aabb.frame != self.anchor {
            return Err(SpatialError::PathNotFound);
        }
        let e1 = self.grid.begin_query()?;
        let path = self.grid.merge_region(aabb, scratch)?;
        let query = StaticAabb {
            min: aabb.min,
            max: aabb.max,
        };
        if !query.is_valid() {
            return Err(SpatialError::InvalidArg);
        }
        self.statics.merge_overlap(&query, scratch);
        if !self.grid.epoch_unchanged(e1) {
            return Err(SpatialError::WriterConflict);
        }
        Ok(ProximityOutcome {
            path,
            overflow: overflow_of(scratch),
        })
    }
}

/// The scratch overflow lower bound, or `None` when every candidate fit
/// (`I-X6`). REQ: REQ-SPATIAL-020.
fn overflow_of(scratch: &CandidateSet) -> Option<u32> {
    let required = scratch.required();
    (required > 0).then_some(required.max(scratch.handles().len() as u32 + 1))
}

/// The anchor-frame AABB enclosing a sphere. REQ: REQ-SPATIAL-046.
fn sphere_aabb(center: Point3, radius: f64) -> Result<StaticAabb, SpatialError> {
    if !radius.is_finite() || radius < 0.0 {
        return Err(SpatialError::InvalidArg);
    }
    let finite = center.x.is_finite() && center.y.is_finite() && center.z.is_finite();
    if !finite {
        return Err(SpatialError::InvalidArg);
    }
    let (x, y, z) = (center.x, center.y, center.z);
    Ok(StaticAabb {
        min: Point3 {
            x: x - radius,
            y: y - radius,
            z: z - radius,
        },
        max: Point3 {
            x: x + radius,
            y: y + radius,
            z: z + radius,
        },
    })
}
