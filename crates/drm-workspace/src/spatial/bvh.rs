//! Build-time compiled static BVH
//! ([spatial-index.md](../../../docs/algorithms/spatial/spatial-index.md)
//! §Compiled Static BVH, `§Functions` `compile_bvh`/`swap_subtree`,
//! `§Invariants` `I-X3`/`I-X8`; [`spatial_architecture.md`](../../../docs/spatial_architecture.md)
//! §Compiled Static BVH). Static environment geometry (walls, shelves, keep-out
//! zones) is split with a deterministic surface-area-heuristic into a
//! fixed-layout tree of axis-aligned bounding boxes. Every internal box is the
//! union of its children, so a node's box over-approximates its subtree — a
//! candidate query may return false positives but never a miss (`I-X3`). The
//! same input compiles byte-identically twice (`I-D1` determinism). Recalibratable
//! subtrees own two fixed-capacity banks; a rebuild fills the inactive bank and
//! then atomically selects it, leaving the active bank untouched on rejection
//! (`I-X8`).
//!
//! REQ: REQ-SPATIAL-006, REQ-SPATIAL-013, REQ-SPATIAL-018.

use std::fmt;

/// An axis-aligned bounding box in the anchor frame, meters.
/// REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Minimum corner `[X, Y, Z]`.
    pub min: [f64; 3],
    /// Maximum corner `[X, Y, Z]`.
    pub max: [f64; 3],
}

impl Aabb {
    /// The empty box (`min > max` on every axis), the identity for [`union`].
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }

    /// A point box.
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub const fn point(p: [f64; 3]) -> Self {
        Self { min: p, max: p }
    }

    /// Whether every component is finite and `min <= max`.
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        (0..3).all(|i| {
            self.min[i].is_finite() && self.max[i].is_finite() && self.min[i] <= self.max[i]
        })
    }

    /// The smallest box containing both boxes (`I-X3`: union over-approximates).
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let mut out = Self::empty();
        for i in 0..3 {
            out.min[i] = self.min[i].min(other.min[i]);
            out.max[i] = self.max[i].max(other.max[i]);
        }
        out
    }

    /// Whether the two boxes overlap on every axis (`I-X3`).
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && other.min[i] <= self.max[i])
    }

    /// Half the surface area (the SAH cost metric, monotone in area).
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    fn half_area(&self) -> f64 {
        if !self.is_valid() {
            return 0.0;
        }
        let dx = self.max[0] - self.min[0];
        let dy = self.max[1] - self.min[1];
        let dz = self.max[2] - self.min[2];
        dx * dy + dy * dz + dz * dx
    }

    /// The centroid of the box.
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    fn centroid(&self) -> [f64; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }
}

/// One static entity's bounds, keyed by a stable entity id.
/// REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticPrimitive {
    /// Stable entity identity.
    pub entity: u32,
    /// Bounds in the anchor frame.
    pub aabb: Aabb,
}

/// A compiled BVH node in the fixed output layout.
/// REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BvhNode {
    /// Node bounds (union of children for internal nodes, `I-X3`).
    pub aabb: Aabb,
    /// Left child index, or [`LEAF`] for a leaf.
    pub left: u32,
    /// Right child index, or [`LEAF`] for a leaf.
    pub right: u32,
    /// First index into the leaf-primitive table (leaf nodes only).
    pub leaf_start: u32,
    /// Primitive count (leaf nodes only).
    pub leaf_count: u32,
}

/// Sentinel marking a node as a leaf.
/// REQ: REQ-SPATIAL-006.
pub const LEAF: u32 = u32::MAX;

impl BvhNode {
    /// Whether this node is a leaf.
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub const fn is_leaf(&self) -> bool {
        self.left == LEAF
    }
}

/// A compiled, fixed-layout static BVH (build-time artifact, `REQ-SPATIAL-006`).
/// REQ: REQ-SPATIAL-006.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledBvh {
    /// Node array; node `0` is the root.
    pub nodes: Vec<BvhNode>,
    /// Primitive table in leaf order.
    pub leaves: Vec<StaticPrimitive>,
    /// Root bounds.
    pub root_aabb: Aabb,
}

impl CompiledBvh {
    /// Whether the tree is empty (no primitives).
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Queries every primitive whose box overlaps `query`, by AABB traversal
    /// (`I-X3`). The result is a deliberate superset; callers exact-check.
    /// REQ: REQ-SPATIAL-006.
    #[must_use]
    pub fn query(&self, query: &Aabb) -> Vec<StaticPrimitive> {
        let mut out = Vec::new();
        if self.nodes.is_empty() || !self.root_aabb.overlaps(query) {
            return out;
        }
        let mut stack = vec![0u32];
        while let Some(idx) = stack.pop() {
            let Some(node) = self.nodes.get(idx as usize) else {
                continue;
            };
            if !node.aabb.overlaps(query) {
                continue;
            }
            if node.is_leaf() {
                let start = node.leaf_start as usize;
                let end = start + node.leaf_count as usize;
                for prim in &self.leaves[start..end] {
                    if prim.aabb.overlaps(query) {
                        out.push(*prim);
                    }
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
        out.sort_unstable_by_key(|p| p.entity);
        out
    }
}

/// The maximum leaf size before a deterministic SAH split is attempted.
/// REQ: REQ-SPATIAL-006.
const MAX_LEAF: usize = 4;

/// Compiles static primitives into a fixed-layout BVH (`I-X3`): a
/// surface-area-heuristic split over the largest centroid extent, ties broken
/// by entity id, so the same input compiles byte-identically twice.
/// REQ: REQ-SPATIAL-006.
#[must_use]
pub fn compile_bvh(primitives: &[StaticPrimitive]) -> CompiledBvh {
    let mut items: Vec<StaticPrimitive> = primitives
        .iter()
        .copied()
        .filter(|p| p.aabb.is_valid())
        .collect();
    // Deterministic input order: entity id, then box (I-D1).
    items.sort_unstable_by(|a, b| a.entity.cmp(&b.entity));
    if items.is_empty() {
        return CompiledBvh {
            nodes: Vec::new(),
            leaves: Vec::new(),
            root_aabb: Aabb::empty(),
        };
    }
    let root_aabb = items.iter().fold(Aabb::empty(), |acc, p| acc.union(p.aabb));
    let mut builder = Builder {
        nodes: Vec::new(),
        leaves: Vec::new(),
    };
    let root = builder.build(&mut items);
    debug_assert_eq!(root, 0);
    CompiledBvh {
        nodes: builder.nodes,
        leaves: builder.leaves,
        root_aabb,
    }
}

/// Recursive builder producing parent-before-child indices.
/// REQ: REQ-SPATIAL-006.
struct Builder {
    nodes: Vec<BvhNode>,
    leaves: Vec<StaticPrimitive>,
}

impl Builder {
    /// Builds one node over `items`, returns its index.
    /// REQ: REQ-SPATIAL-006.
    fn build(&mut self, items: &mut [StaticPrimitive]) -> u32 {
        let aabb = items.iter().fold(Aabb::empty(), |acc, p| acc.union(p.aabb));
        let idx = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        // Reserve this node's slot; children are appended after.
        self.nodes.push(BvhNode {
            aabb,
            left: LEAF,
            right: LEAF,
            leaf_start: 0,
            leaf_count: 0,
        });
        if items.len() <= MAX_LEAF || axis_extent(items) <= 0.0 {
            let leaf_start = u32::try_from(self.leaves.len()).unwrap_or(u32::MAX);
            self.leaves.extend_from_slice(items);
            self.nodes[idx as usize] = BvhNode {
                aabb,
                left: LEAF,
                right: LEAF,
                leaf_start,
                leaf_count: u32::try_from(items.len()).unwrap_or(u32::MAX),
            };
            return idx;
        }
        let mid = sah_split(items);
        let (left_items, right_items) = items.split_at_mut(mid);
        let left = self.build(left_items);
        let right = self.build(right_items);
        self.nodes[idx as usize] = BvhNode {
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
fn axis_extent(items: &[StaticPrimitive]) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in items {
        let c = p.aabb.centroid();
        for i in 0..3 {
            lo[i] = lo[i].min(c[i]);
            hi[i] = hi[i].max(c[i]);
        }
    }
    (0..3).map(|i| hi[i] - lo[i]).fold(0.0_f64, f64::max)
}

/// Deterministic SAH split: sort by the widest centroid axis (ties by entity
/// id) and choose the partition minimizing `left_area·|L| + right_area·|R|`,
/// ties to the lower index.
/// REQ: REQ-SPATIAL-006.
fn sah_split(items: &mut [StaticPrimitive]) -> usize {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in items.iter() {
        let c = p.aabb.centroid();
        for i in 0..3 {
            lo[i] = lo[i].min(c[i]);
            hi[i] = hi[i].max(c[i]);
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
        let ca = a.aabb.centroid()[axis];
        let cb = b.aabb.centroid()[axis];
        ca.partial_cmp(&cb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.entity.cmp(&b.entity))
    });

    let n = items.len();
    let count = |v: usize| f64::from(u32::try_from(v).unwrap_or(u32::MAX));
    let mut best_cost = f64::INFINITY;
    let mut best_mid = n / 2;
    let mut left = Aabb::empty();
    // Precompute suffix unions for the right side.
    let mut suffix = vec![Aabb::empty(); n];
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

/// A live-calibration subtree slot: two fixed-capacity banks and an active
/// selector (`I-X8`). A rebuild writes the inactive bank and atomically selects
/// it; oversize geometry is rejected without disturbing the active bank.
/// REQ: REQ-SPATIAL-018.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtreeSlot {
    banks: [CompiledBvh; 2],
    active: usize,
    capacity: usize,
}

/// A subtree swap error (`I-X8` edge cases). REQ: REQ-SPATIAL-018.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BvhError {
    /// A rebuild would exceed the slot's declared node capacity; the active
    /// bank is untouched.
    Oversize {
        /// Declared node capacity.
        capacity: usize,
        /// Nodes the new geometry requires.
        required: usize,
    },
    /// The inactive bank still has readers; retry later (`Busy`).
    Busy,
}

impl fmt::Display for BvhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversize { capacity, required } => write!(
                f,
                "subtree rebuild requires {required} nodes > capacity {capacity} (I-X8)"
            ),
            Self::Busy => write!(f, "subtree inactive bank has readers (I-X8)"),
        }
    }
}

impl std::error::Error for BvhError {}

impl SubtreeSlot {
    /// Creates a slot from initial geometry with a declared node capacity
    /// (fixed at activation).
    /// REQ: REQ-SPATIAL-018.
    #[must_use]
    pub fn new(initial: &[StaticPrimitive], capacity: usize) -> Self {
        let bvh = compile_bvh(initial);
        let banks = [bvh.clone(), bvh];
        Self {
            banks,
            active: 0,
            capacity,
        }
    }

    /// The currently active (selected) bank.
    /// REQ: REQ-SPATIAL-018.
    #[must_use]
    pub fn active(&self) -> &CompiledBvh {
        &self.banks[self.active]
    }

    /// Rebuilds the inactive bank with `geometry` and selects it, leaving the
    /// active bank intact when the rebuild is rejected (`I-X8`).
    ///
    /// # Errors
    /// [`BvhError::Oversize`] when the tree would exceed the declared capacity.
    /// REQ: REQ-SPATIAL-018.
    pub fn swap(&mut self, geometry: &[StaticPrimitive]) -> Result<(), BvhError> {
        let candidate = compile_bvh(geometry);
        if candidate.nodes.len() > self.capacity {
            return Err(BvhError::Oversize {
                capacity: self.capacity,
                required: candidate.nodes.len(),
            });
        }
        let inactive = 1 - self.active;
        self.banks[inactive] = candidate;
        self.active = inactive;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prim(entity: u32, c: [f64; 3], h: f64) -> StaticPrimitive {
        StaticPrimitive {
            entity,
            aabb: Aabb {
                min: [c[0] - h, c[1] - h, c[2] - h],
                max: [c[0] + h, c[1] + h, c[2] + h],
            },
        }
    }

    /// REQ: REQ-SPATIAL-006 — every internal node's box is the union of its
    /// children (`I-X3`), verified over a random-ish fixture.
    #[test]
    fn aabb_over_approximation() {
        let prims: Vec<StaticPrimitive> = (0..64u32)
            .map(|i| {
                prim(
                    i,
                    [f64::from(i) * 0.7, f64::from(i % 7), f64::from(i % 3)],
                    0.3,
                )
            })
            .collect();
        let bvh = compile_bvh(&prims);
        for node in &bvh.nodes {
            if !node.is_leaf() {
                let l = &bvh.nodes[node.left as usize];
                let r = &bvh.nodes[node.right as usize];
                assert_eq!(node.aabb, l.aabb.union(r.aabb), "union over-approximation");
            }
        }
        // Every primitive is covered by the root (no miss).
        for p in &prims {
            assert!(bvh.root_aabb.overlaps(&p.aabb));
        }
    }

    /// REQ: REQ-SPATIAL-006 — double-compile is byte-identical (`I-D1`).
    #[test]
    fn deterministic_double_compile() {
        let prims: Vec<StaticPrimitive> = (0..50u32)
            .map(|i| prim(i, [f64::from(i) * 0.3, 0.0, 0.0], 0.5))
            .collect();
        assert_eq!(compile_bvh(&prims), compile_bvh(&prims));
        // Shuffled input order yields the same tree.
        let mut shuffled = prims.clone();
        shuffled.reverse();
        assert_eq!(compile_bvh(&prims), compile_bvh(&shuffled));
    }

    /// REQ: REQ-SPATIAL-006 — a query returns exactly the overlapping set.
    #[test]
    fn query_overlap() {
        let prims = vec![
            prim(0, [0.0, 0.0, 0.0], 0.5),
            prim(1, [10.0, 0.0, 0.0], 0.5),
            prim(2, [0.4, 0.0, 0.0], 0.5),
        ];
        let bvh = compile_bvh(&prims);
        let hits = bvh.query(&Aabb {
            min: [-1.0, -1.0, -1.0],
            max: [1.0, 1.0, 1.0],
        });
        let ids: Vec<u32> = hits.iter().map(|p| p.entity).collect();
        assert_eq!(ids, vec![0, 2], "exact overlap, sorted by entity");
    }

    /// REQ: REQ-SPATIAL-018 — a live calibration swap selects the rebuilt bank
    /// without touching the old one; oversize is rejected (`I-X8`).
    #[test]
    fn live_calibration_swap() {
        let initial = vec![prim(0, [0.0, 0.0, 0.0], 0.5)];
        let mut slot = SubtreeSlot::new(&initial, 4);
        let before = slot.active().clone();
        let grown = vec![prim(0, [0.0, 0.0, 0.0], 0.5), prim(1, [5.0, 0.0, 0.0], 0.5)];
        slot.swap(&grown).unwrap_or_else(|e| unreachable!("{e}"));
        assert_ne!(slot.active(), &before);
        assert_eq!(slot.active().leaves.len(), 2);
        // Oversize is rejected without disturbing the active bank.
        let huge: Vec<StaticPrimitive> = (0..100u32)
            .map(|i| prim(i, [f64::from(i), 0.0, 0.0], 0.1))
            .collect();
        let active_before = slot.active().clone();
        assert!(matches!(slot.swap(&huge), Err(BvhError::Oversize { .. })));
        assert_eq!(slot.active(), &active_before);
    }
}
