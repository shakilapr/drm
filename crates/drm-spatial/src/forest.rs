//! Forests of spaces and path search
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §The Five
//! Golden Rules rule 2, §Invariants `I-S3`; [`spatial_architecture.md`](../../docs/spatial_architecture.md)
//! §A Forest of Spaces). Spatial graphs form a forest of independent trees;
//! disconnected trees coexist and a query across them returns
//! [`SpatialError::PathNotFound`] unless an explicit bridge transform or a common
//! ancestor frame connects them (`REQ-SPATIAL-010`, `REQ-SPATIAL-039`).
//!
//! The search walks the compiled base graph plus runtime overlay frames
//! (`I-S12`) and honors registered bridges. Multi-robot systems resolve
//! transforms through a common ancestor frame or through explicit inter-robot
//! spatial links (bridges). Path classification is a bounded ancestor walk —
//! never simple-path enumeration (`I-R11` pattern reuse). Each edge is returned
//! directed from `reference`'s side toward `target`'s side so a caller folds the
//! plan in order.
//!
//! REQ: REQ-SPATIAL-010, REQ-SPATIAL-039, REQ-SPATIAL-008, REQ-SPATIAL-048.

use std::collections::{BTreeMap, BTreeSet};

use crate::transform::invert;
use crate::types::{SpatialError, Transform3};

/// Canonical frame identity: `/`-separated lowercase segments derived from the
/// filesystem path (`REQ-SPATIAL-047`).
pub type FrameId = String;

/// One edge transform on a resolved path, directed from the reference side
/// toward the target side.
/// REQ: REQ-SPATIAL-039, REQ-SPATIAL-048.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathEdge {
    /// A build-time collapsed static run (`I-S13`).
    Constant(Transform3),
    /// A runtime dynamic edge, addressed by numeric id.
    Dynamic(u32),
    /// An explicit bridge transform connecting two trees (`REQ-SPATIAL-039`).
    Bridge(Transform3),
    /// A calibration-bound static edge: the effective transform is
    /// `nominal × correction`, resolved live from slot `slot` (`I-Q11`). When
    /// traversed in the reverse direction, the folded value is the inverse of
    /// that effective transform, so the plan emits an `Invert` step rather than
    /// inverting the nominal alone.
    Calibrated {
        /// The nominal static transform.
        nominal: Transform3,
        /// The shared calibration slot id.
        slot: u32,
        /// Whether this edge is traversed in the reverse direction.
        inverted: bool,
    },
}

/// A classified frame edge toward the frame's parent (`REQ-SPATIAL-048`).
/// REQ: REQ-SPATIAL-010.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameEdge {
    /// A root frame with no parent (identity anchor).
    Root,
    /// A static relationship: nominal offset relative to the parent.
    Static(Transform3),
    /// A dynamic edge owned by a single writer (`I-S11`).
    Dynamic(u32),
    /// A calibration-bound static relationship: effective
    /// `nominal × correction` from shared slot `slot` (`I-Q11`).
    Calibrated {
        /// The nominal static transform.
        nominal: Transform3,
        /// The shared calibration slot id.
        slot: u32,
    },
}

impl FrameEdge {
    /// The path edge this frame contributes when walked toward its parent.
    /// REQ: REQ-SPATIAL-048.
    #[must_use]
    fn as_path_edge(self) -> PathEdge {
        match self {
            Self::Static(t) => PathEdge::Constant(t),
            Self::Dynamic(e) => PathEdge::Dynamic(e),
            Self::Calibrated { nominal, slot } => PathEdge::Calibrated {
                nominal,
                slot,
                inverted: false,
            },
            Self::Root => PathEdge::Constant(Transform3::IDENTITY),
        }
    }
}

/// A runtime space graph: compiled base frames plus runtime overlay frames and
/// explicit bridges (forest, no mandatory universal world).
/// REQ: REQ-SPATIAL-010, REQ-SPATIAL-039.
#[derive(Debug, Clone, Default)]
pub struct SpaceForest {
    edges: BTreeMap<FrameId, FrameEdge>,
    parents: BTreeMap<FrameId, FrameId>,
    bridges: Vec<(FrameId, FrameId, Transform3)>,
}

impl SpaceForest {
    /// An empty forest.
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a root frame (identity anchor, no parent).
    /// REQ: REQ-SPATIAL-048.
    pub fn add_root(&mut self, frame: FrameId) {
        self.parents.remove(&frame);
        self.edges.insert(frame, FrameEdge::Root);
    }

    /// Registers a static frame with its nominal offset relative to `parent`.
    ///
    /// # Errors
    /// [`SpatialError::NonFinite`] when the transform is not finite.
    /// REQ: REQ-SPATIAL-048.
    pub fn add_static(
        &mut self,
        frame: FrameId,
        parent: FrameId,
        transform: Transform3,
    ) -> Result<(), SpatialError> {
        if !transform.translation.iter().all(|c| c.is_finite())
            || !transform.rotation.iter().all(|c| c.is_finite())
        {
            return Err(SpatialError::NonFinite);
        }
        self.edges
            .insert(frame.clone(), FrameEdge::Static(transform));
        self.parents.insert(frame, parent);
        Ok(())
    }

    /// Registers a dynamic frame whose pose is produced by edge `edge` and whose
    /// parent is declared for graph connectivity.
    /// REQ: REQ-SPATIAL-048.
    pub fn add_dynamic(&mut self, frame: FrameId, parent: FrameId, edge: u32) {
        self.edges.insert(frame.clone(), FrameEdge::Dynamic(edge));
        self.parents.insert(frame, parent);
    }

    /// Registers a calibration-bound static frame whose effective offset is
    /// `nominal × correction` from shared slot `slot` (`I-Q11`).
    ///
    /// # Errors
    /// [`SpatialError::NonFinite`] when the nominal transform is not finite.
    /// REQ: REQ-SPATIAL-003.
    pub fn add_calibration(
        &mut self,
        frame: FrameId,
        parent: FrameId,
        nominal: Transform3,
        slot: u32,
    ) -> Result<(), SpatialError> {
        if !nominal.translation.iter().all(|c| c.is_finite())
            || !nominal.rotation.iter().all(|c| c.is_finite())
        {
            return Err(SpatialError::NonFinite);
        }
        self.edges
            .insert(frame.clone(), FrameEdge::Calibrated { nominal, slot });
        self.parents.insert(frame, parent);
        Ok(())
    }

    /// Registers an explicit bridge transform connecting two frames (possibly in
    /// different trees). Bridges are undirected for connectivity; the registered
    /// transform is the `from → to` offset.
    /// REQ: REQ-SPATIAL-039.
    pub fn add_bridge(&mut self, from: FrameId, to: FrameId, transform: Transform3) {
        self.edges.entry(from.clone()).or_insert(FrameEdge::Root);
        self.edges.entry(to.clone()).or_insert(FrameEdge::Root);
        self.bridges.push((from, to, transform));
    }

    /// Removes a frame from the forest (`space.delete` historically, `I-S12`).
    /// Any frame that named the removed frame as its parent becomes a root so
    /// the remaining forest stays well formed.
    /// REQ: REQ-SPATIAL-010, REQ-SPATIAL-008.
    pub fn remove(&mut self, frame: &str) {
        self.edges.remove(frame);
        self.parents.remove(frame);
        let orphans: Vec<FrameId> = self
            .parents
            .iter()
            .filter(|(_, parent)| parent.as_str() == frame)
            .map(|(child, _)| child.clone())
            .collect();
        for child in orphans {
            self.parents.remove(&child);
        }
        self.bridges
            .retain(|(from, to, _)| from != frame && to != frame);
    }

    /// Number of registered frames (base + overlay + bridge endpoints).
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether the forest holds no frames.
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Whether `frame` is registered.
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn contains(&self, frame: &str) -> bool {
        self.edges.contains_key(frame)
    }

    /// The edge declared on `frame`.
    /// REQ: REQ-SPATIAL-048.
    #[must_use]
    pub fn edge(&self, frame: &str) -> Option<FrameEdge> {
        self.edges.get(frame).copied()
    }

    /// The parent of `frame`, if any.
    /// REQ: REQ-SPATIAL-048.
    #[must_use]
    pub fn parent(&self, frame: &str) -> Option<&FrameId> {
        self.parents.get(frame)
    }

    /// Is `frame` connected to `other` within the same tree (bridges ignored)?
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn connected(&self, frame: &str, other: &str) -> bool {
        self.ancestors(frame).contains(other) || self.ancestors(other).contains(frame)
    }

    /// The set of `frame` and all of its ancestors (the tree path to the root).
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn ancestors(&self, frame: &str) -> BTreeSet<FrameId> {
        let mut out = BTreeSet::new();
        let mut cur = Some(frame.to_string());
        while let Some(f) = cur {
            if !out.insert(f.clone()) {
                break; // cycle guard: never loop forever
            }
            cur = self.parents.get(&f).cloned();
        }
        out
    }

    /// Resolves the path from `reference` to `target`, or
    /// [`SpatialError::PathNotFound`] when the two frames lie in disconnected
    /// trees with no explicit bridge (`I-S3`). The returned edges are directed
    /// from the reference side toward the target side: edges walked up from the
    /// reference to the common ancestor appear first, then edges down to the
    /// target are inverted so a caller folds them in order.
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unknown frame or a cross-tree query
    /// with no bridge.
    /// REQ: REQ-SPATIAL-010, REQ-SPATIAL-039.
    pub fn path_between(
        &self,
        reference: &str,
        target: &str,
    ) -> Result<Vec<PathEdge>, SpatialError> {
        if !self.edges.contains_key(reference) || !self.edges.contains_key(target) {
            return Err(SpatialError::PathNotFound);
        }
        if reference == target {
            return Ok(Vec::new());
        }
        if let Some(path) = self.tree_path(reference, target) {
            return Ok(path);
        }
        if let Some(path) = self.bridged_path(reference, target) {
            return Ok(path);
        }
        Err(SpatialError::PathNotFound)
    }

    /// Walks up from `reference` and `target` to their lowest common ancestor;
    /// `None` when they are in different trees. Edges from the reference up to
    /// the ancestor are used as stored (`T_parent_child`); edges from the
    /// ancestor down to the target are inverted.
    /// REQ: REQ-SPATIAL-010.
    fn tree_path(&self, reference: &str, target: &str) -> Option<Vec<PathEdge>> {
        let ref_chain = self.up_chain(reference);
        let target_chain = self.up_chain(target);
        let target_set: BTreeSet<&FrameId> = target_chain.iter().map(|(f, _)| f).collect();
        let split = ref_chain.iter().position(|(f, _)| target_set.contains(f))?;
        // Reference → common ancestor: edges from the reference upward.
        let mut out: Vec<PathEdge> = ref_chain[..split].iter().map(|(_, e)| *e).collect();
        let common = ref_chain[split].0.clone();
        // Common ancestor → target: invert the target's upward edges.
        let pos = target_chain.iter().position(|(f, _)| *f == common)?;
        for (_, edge) in target_chain[..pos].iter().rev() {
            out.push(invert_edge(*edge));
        }
        Some(out)
    }

    /// The upward chain `(frame, edge_toward_parent)` from `frame` to its root.
    /// The final entry is the root with an identity edge.
    /// REQ: REQ-SPATIAL-010.
    fn up_chain(&self, frame: &str) -> Vec<(FrameId, PathEdge)> {
        let mut out = Vec::new();
        let mut cur = frame.to_string();
        let mut seen = BTreeSet::new();
        while seen.insert(cur.clone()) {
            if let Some(parent) = self.parents.get(&cur) {
                let edge = self
                    .edges
                    .get(&cur)
                    .copied()
                    .unwrap_or(FrameEdge::Root)
                    .as_path_edge();
                out.push((cur.clone(), edge));
                cur.clone_from(parent);
            } else {
                out.push((cur, PathEdge::Constant(Transform3::IDENTITY)));
                break;
            }
        }
        out
    }

    /// Upward edges from `frame` to the ancestor `anc`, exclusive.
    /// REQ: REQ-SPATIAL-010.
    fn up_to(&self, frame: &str, anc: &str) -> Option<Vec<PathEdge>> {
        let chain = self.up_chain(frame);
        let pos = chain.iter().position(|(f, _)| f == anc)?;
        Some(chain[..pos].iter().map(|(_, e)| *e).collect())
    }

    /// Begins an atomic mutation transaction over a staged clone of this forest
    /// (`I-Q10`). Relinks are applied to the stage and become visible only when
    /// [`TreeTxn::commit`] swaps the whole forest in one step, so a concurrent
    /// reader never observes a partially updated tree.
    /// REQ: REQ-SPATIAL-027.
    #[must_use]
    pub fn begin(&self) -> TreeTxn {
        TreeTxn {
            staged: self.clone(),
        }
    }

    /// Attempts a path that crosses exactly one explicit bridge. The bridge
    /// transform is `T_from_to`; when the reference side attaches to `to` the
    /// inverse is used so the folded path stays directional.
    /// REQ: REQ-SPATIAL-039.
    fn bridged_path(&self, reference: &str, target: &str) -> Option<Vec<PathEdge>> {
        let ref_anc = self.ancestors(reference);
        let target_anc = self.ancestors(target);
        for (from, to, transform) in &self.bridges {
            // Reference attaches to one endpoint; target to the other (either
            // orientation, since the bridge is undirected for connectivity).
            let (ref_end, target_end, bridge) = if ref_anc.contains(from) && target_anc.contains(to)
            {
                (from, to, *transform)
            } else if ref_anc.contains(to) && target_anc.contains(from) {
                (to, from, invert(transform))
            } else {
                continue;
            };
            let Some(mut a) = self.up_to(reference, ref_end) else {
                continue;
            };
            let Some(b) = self.up_to(target, target_end) else {
                continue;
            };
            a.push(PathEdge::Bridge(bridge));
            for edge in b.iter().rev() {
                a.push(invert_edge(*edge));
            }
            return Some(a);
        }
        None
    }
}

/// Inverts a path edge direction (`T_child_parent = T_parent_child⁻¹`). Dynamic
/// edges inverts to identity geometry here; the runtime resolves them at query
/// time.
/// REQ: REQ-SPATIAL-039.
fn invert_edge(edge: PathEdge) -> PathEdge {
    match edge {
        PathEdge::Constant(t) | PathEdge::Bridge(t) => PathEdge::Constant(invert(&t)),
        PathEdge::Dynamic(e) => PathEdge::Dynamic(e),
        PathEdge::Calibrated {
            nominal,
            slot,
            inverted,
        } => PathEdge::Calibrated {
            nominal,
            slot,
            inverted: !inverted,
        },
    }
}

/// An atomic tree-mutation transaction ([spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
/// §Protocols transactional reparenting, §Invariants `I-Q10`). Mutations are
/// staged on a clone of the base forest and applied together on [`Self::commit`],
/// so readers never observe a partially updated tree. Dropping an uncommitted
/// transaction leaves the base forest untouched.
/// REQ: REQ-SPATIAL-027.
#[derive(Debug)]
pub struct TreeTxn {
    staged: SpaceForest,
}

impl TreeTxn {
    /// Relinks `frame` to static `parent` with nominal `transform` in the stage
    /// (`I-Q10`).
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unknown parent frame (the forest
    /// has no universal world; the parent must exist), or
    /// [`SpatialError::NonFinite`] for a non-finite transform.
    /// REQ: REQ-SPATIAL-027, REQ-SPATIAL-048.
    pub fn relink_static(
        &mut self,
        frame: FrameId,
        parent: FrameId,
        transform: Transform3,
    ) -> Result<(), SpatialError> {
        if !self.staged.contains(&parent) {
            return Err(SpatialError::PathNotFound);
        }
        self.staged.add_static(frame, parent, transform)
    }

    /// Relinks `frame` to `parent`, keeping a dynamic edge handle (`I-Q10`).
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unknown parent frame.
    /// REQ: REQ-SPATIAL-027, REQ-SPATIAL-048.
    pub fn relink_dynamic(
        &mut self,
        frame: FrameId,
        parent: FrameId,
        edge: u32,
    ) -> Result<(), SpatialError> {
        if !self.staged.contains(&parent) {
            return Err(SpatialError::PathNotFound);
        }
        self.staged.add_dynamic(frame, parent, edge);
        Ok(())
    }

    /// The staged forest, for computing new local offsets before commit
    /// (`I-Q9`).
    /// REQ: REQ-SPATIAL-027.
    #[must_use]
    pub fn staged(&self) -> &SpaceForest {
        &self.staged
    }

    /// Commits the transaction, replacing `forest` with the staged tree in one
    /// step (`I-Q10`).
    ///
    /// REQ: REQ-SPATIAL-027.
    pub fn commit(self, forest: &mut SpaceForest) {
        *forest = self.staged;
    }
}

#[cfg(test)]
mod tests {
    use super::{PathEdge, SpaceForest};
    use crate::types::{SpatialError, Transform3};

    fn t(x: f64, y: f64, z: f64) -> Transform3 {
        Transform3 {
            translation: [x, y, z],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn single_tree() -> SpaceForest {
        let mut f = SpaceForest::new();
        f.add_root("world".to_string());
        f.add_static("base".to_string(), "world".to_string(), t(1.0, 0.0, 0.0))
            .unwrap_or_else(|e| unreachable!("{e}"));
        f.add_static("camera".to_string(), "base".to_string(), t(0.0, 0.0, 3.0))
            .unwrap_or_else(|e| unreachable!("{e}"));
        f
    }

    /// REQ: REQ-SPATIAL-010 — a same-tree query resolves through the tree walk.
    #[test]
    fn same_tree_path() {
        let f = single_tree();
        let path = f
            .path_between("camera", "world")
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(path.len(), 2);
        assert_eq!(path[0], PathEdge::Constant(t(0.0, 0.0, 3.0)));
        assert_eq!(path[1], PathEdge::Constant(t(1.0, 0.0, 0.0)));
    }

    /// REQ: REQ-SPATIAL-010, REQ-SPATIAL-039 — a cross-tree query returns
    /// `PathNotFound` until an explicit bridge is registered (`I-S3`).
    #[test]
    fn cross_tree_path_not_found() {
        let mut f = single_tree();
        f.add_root("sim/world".to_string());
        assert_eq!(
            f.path_between("camera", "sim/world"),
            Err(SpatialError::PathNotFound)
        );
        f.add_bridge(
            "world".to_string(),
            "sim/world".to_string(),
            t(100.0, 0.0, 0.0),
        );
        let path = f
            .path_between("camera", "sim/world")
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert!(path.contains(&PathEdge::Bridge(t(100.0, 0.0, 0.0))));
    }

    /// REQ: REQ-SPATIAL-010 — an unknown frame is path-not-found.
    #[test]
    fn unknown_frame_not_found() {
        let f = single_tree();
        assert_eq!(
            f.path_between("camera", "missing"),
            Err(SpatialError::PathNotFound)
        );
    }

    /// REQ: REQ-SPATIAL-048 — a dynamic edge resolves as a `PathEdge::Dynamic`.
    #[test]
    fn dynamic_edge_on_path() {
        let mut f = SpaceForest::new();
        f.add_root("map".to_string());
        f.add_dynamic("odom".to_string(), "map".to_string(), 7);
        let path = f
            .path_between("odom", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(path, vec![PathEdge::Dynamic(7)]);
    }
}
