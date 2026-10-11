//! Handle assignment ([handle-system.md](../../../docs/algorithms/toolchain/handle-system.md)
//! §Functions `assign`): canonical topic path → stable numeric handle across
//! builds, prior assignments authoritative, tombstones never reused until
//! explicit compaction. §Functions `compact` adds the explicit ack gate and the
//! coordinated-rebuild guidance (I-H4/I-H7). REQ: REQ-TOOL-036, REQ-TOOL-038,
//! REQ-TOOL-039, REQ-TOOL-080.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use drm_types::Status;

use super::WorkspaceModel;

/// Why a numeric handle is reserved (never silently reused, I-H5).
/// REQ: REQ-TOOL-081.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveReason {
    /// Topic deleted; tombstone until explicit compaction (I-H3).
    Deleted,
    /// Reconcile retired a colliding number entirely.
    MergeCollision,
}

/// Committed assignment-history lockfile content (`.drm/ids.json` payload).
/// REQ: REQ-TOOL-080.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssignmentHistory {
    /// Topic path → assigned handle.
    pub assignments: BTreeMap<String, u32>,
    /// Reserved (unassignable) handles.
    pub reserved: BTreeMap<u32, ReserveReason>,
}

/// Live handle table. REQ: REQ-TOOL-039.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HandleTable {
    /// Topic path → assigned handle.
    pub assignments: BTreeMap<String, u32>,
    /// Tombstones + merge collisions — never reused pre-compact.
    pub reserved: BTreeMap<u32, ReserveReason>,
    /// Next fresh allocation point (strictly above every live/reserved ID).
    pub next_free: u32,
}

/// Assignment diff from one `assign` run. REQ: REQ-TOOL-036.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignDelta {
    /// New topic got a fresh handle.
    Added(String, u32),
    /// Prior topic removed; its handle tombstoned.
    Removed(String, u32),
}

impl HandleTable {
    /// Prior assignments + reservations are authoritative (I-H1/I-H2);
    /// fresh allocation starts strictly above every live/reserved ID
    /// (`u32::MAX` is the saturated "table full" sentinel).
    /// REQ: REQ-TOOL-036.
    #[must_use]
    pub fn from_history(prior: &AssignmentHistory) -> Self {
        let max_id = prior
            .assignments
            .values()
            .chain(prior.reserved.keys())
            .copied()
            .max();
        let next_free = match max_id {
            None => 0_u32,
            Some(u32::MAX) => u32::MAX,
            Some(id) => id + 1,
        };
        Self {
            assignments: prior.assignments.clone(),
            reserved: prior.reserved.clone(),
            next_free,
        }
    }

    /// `assign(model, prior)` (handle-system.md §Functions): existing handles
    /// unchanged (I-H2); removed topics tombstone (I-H3/I-H5); new topics
    /// allocate from `next_free` with a checked increment (I-H6). The final
    /// free slot (`u32::MAX`) is allocatable; the table then saturates.
    /// REQ: REQ-TOOL-036, REQ-TOOL-082.
    ///
    /// # Errors
    /// [`Status::SequenceExhausted`] when the u32 handle space is exhausted
    /// at assignment time (typed error, no overflow).
    pub fn assign(&self, topics: &BTreeSet<String>) -> Result<(Self, Vec<AssignDelta>), Status> {
        let mut out = self.clone();
        let mut deltas = Vec::new();
        for topic in topics {
            if out.assignments.contains_key(topic) {
                continue;
            }
            let max_taken = out.assignments.values().any(|v| *v == u32::MAX)
                || out.reserved.contains_key(&u32::MAX);
            if out.next_free == u32::MAX && max_taken {
                return Err(Status::SequenceExhausted);
            }
            let id = out.next_free;
            out.assignments.insert(topic.clone(), id);
            out.next_free = id.saturating_add(1);
            deltas.push(AssignDelta::Added(topic.clone(), id));
        }
        for (topic, id) in &self.assignments {
            if !topics.contains(topic) {
                out.reserved.insert(*id, ReserveReason::Deleted);
                out.assignments.remove(topic);
                deltas.push(AssignDelta::Removed(topic.clone(), *id));
            }
        }
        Ok((out, deltas))
    }

    /// Dense re-pack (compaction core; CLI adds ack gate + new deployment
    /// identity): live assignments renumbered 0..n in sorted order,
    /// reservations dropped — coordinated redeployment makes reuse safe
    /// (I-H4). REQ: REQ-TOOL-038.
    #[must_use]
    pub fn compact_repack(&self) -> Self {
        let mut out = Self::default();
        for (i, topic) in self.assignments.keys().enumerate() {
            out.assignments
                .insert(topic.clone(), u32::try_from(i).unwrap_or(u32::MAX));
        }
        out.next_free = u32::try_from(self.assignments.len()).unwrap_or(u32::MAX);
        out
    }

    /// Explicit compaction gate (`handle-system.md` §Functions `compact`): any
    /// non-compact command leaves assignments untouched; compaction requires
    /// acknowledgement (I-H4). With `ack`, the table is densely re-packed and
    /// reservations are dropped.
    /// REQ: REQ-TOOL-038.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `ack` is `false` (explicit only).
    pub fn compact(&self, ack: bool) -> Result<Self, Status> {
        if !ack {
            return Err(Status::InvalidArg);
        }
        Ok(self.compact_repack())
    }
}

/// Coordinated-rebuild guidance emitted after an explicit compaction
/// (`handle-system.md` §Functions `compact`): a handle change is a new
/// deployment identity, so every node binary compiling a deployment handle
/// table must rebuild and restart together, while schema bindings never
/// rebuild (I-H4/I-H7).
/// REQ: REQ-TOOL-038.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildGuidance {
    /// Nodes (sorted) whose binaries must rebuild and restart.
    pub rebuild_nodes: Vec<String>,
    /// Deterministic operator checklist.
    pub checklist: String,
}

/// Builds the coordinated-rebuild checklist for `packed` from `model`
/// (`handle-system.md` §Functions `compact`: `record_rebuild_guidance`).
/// Every deployed node binary links the runtime that reads the deployment
/// handle table, so all nodes rebuild; the checklist keeps the operator
/// actions explicit and deterministic. Code bindings are never regenerated
/// (I-H7).
/// REQ: REQ-TOOL-038.
#[must_use]
pub fn rebuild_guidance(model: &WorkspaceModel, packed: &HandleTable) -> RebuildGuidance {
    let mut rebuild_nodes: Vec<String> = model.nodes.keys().cloned().collect();
    rebuild_nodes.sort();
    let mut checklist = String::new();
    let _ = writeln!(
        checklist,
        "compacted {} live assignment(s) to dense handles 0..{}; reservations dropped (I-H4/REQ-TOOL-038).",
        packed.assignments.len(),
        packed.assignments.len().saturating_sub(1),
    );
    checklist.push_str("new deployment identity — coordinated redeploy required:\n");
    checklist.push_str(
        "  [ ] regenerate all handle-dependent deployment artifacts \
         (routing/runtime tables; MCU topic tables where compiled in)\n",
    );
    if rebuild_nodes.is_empty() {
        checklist.push_str("  [ ] no node binaries embed deployment tables\n");
    } else {
        for node in &rebuild_nodes {
            let _ = writeln!(checklist, "  [ ] rebuild + restart `{node}`");
        }
    }
    checklist.push_str("  [ ] confirm every affected node restarted before traffic resumes\n");
    checklist.push_str("schema bindings are never regenerated (I-H7).\n");
    RebuildGuidance {
        rebuild_nodes,
        checklist,
    }
}
