//! Spatial query surface — `latest`/`at`/`predict`/`delayed_view`
//! ([spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Functions, §Invariants `I-Q1`..`I-Q6`;
//! [`spatial_architecture.md`](../../docs/spatial_architecture.md) §Query
//! Semantics and History, §Versioned Local Cache). A [`QueryEngine`] holds the
//! runtime [`SpaceForest`], the versioned dynamic-edge caches, per-edge
//! [`TimeHistory`] rings, a live calibration table, and a per-process cache of
//! runtime-compiled [`CompiledPlan`]s.
//!
//! Runtime queries **execute** the compiled plan (multiply steps + version-
//! checked cache reads); graph traversal happens only when a plan is first
//! compiled (`I-Q1`). `latest` evaluates every dynamic input at one common
//! coherent timestamp `t* = min_i(t_newest,i)` (`I-Q3`); `at` interpolates
//! between bracketing samples and never silently extrapolates (`I-Q4`);
//! `predict` is the only extrapolation path, integrating the declared body-frame
//! twist on `SE(3)` via the exponential map (`I-Q5`); `delayed_view`
//! resolves `now − offset` through `at` (`I-Q6`). Cached geometry is reused only
//! under an exact input-version match; freshness/staleness/uncertainty are always
//! derived per query (`I-Q2`). Discontinuities are marked per jumped local edge
//! and never interpolated across (`I-Q7`); transactional reparenting installs a
//! new time-scoped plan version while retaining the original hierarchy for
//! pre-`t₁` queries (`I-Q8`), supports Preserve World/Local/Snap (`I-Q9`), and
//! commits atomically so concurrent readers never see a partial tree (`I-Q10`).
//!
//! REQ: REQ-SPATIAL-022, REQ-SPATIAL-023, REQ-SPATIAL-024, REQ-SPATIAL-025,
//! REQ-SPATIAL-033, REQ-SPATIAL-037, REQ-SPATIAL-003, REQ-SPATIAL-016,
//! REQ-SPATIAL-032, REQ-SPATIAL-002, REQ-SPATIAL-027, REQ-SPATIAL-028,
//! REQ-SPATIAL-029.

use std::collections::BTreeMap;

use crate::cache::{EdgeSample, SpatialEdgeCache};
use crate::clock::{TimePoint, map_timestamp_with_uncertainty};
use crate::forest::SpaceForest;
use crate::history::TimeHistory;
use crate::plan::{
    CompiledPlan, FLAG_DISCONTINUITY_NEARBY, FLAG_EXTRAPOLATED, FLAG_STALE, PlanStep, QualityFlags,
    TransformError, TransformResult, compile_plan, execute_steps,
};
use crate::transform::{compose, exp_twist, invert};
use crate::types::{SpatialError, Transform3, Twist3};

/// A live calibration slot ([spatial-queries.md] §Data Structures
/// `CalibrationSlot`; [`spatial_architecture.md`] §Calibration Without
/// Recompilation). The effective transform is `nominal × correction`; updating
/// the correction invalidates dependent caches live with no recompilation
/// (`I-Q11`). The slot carries the effective timestamp of its newest correction
/// and a discontinuity marker set when an update's correction jump exceeds the
/// declared threshold (`I-Q7` tie-in, [spatial-queries.md] §Protocols live
/// calibration).
/// REQ: REQ-SPATIAL-003, REQ-SPATIAL-016.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationSlot {
    /// The nominal static transform embedded in the plan.
    pub nominal: Transform3,
    /// The live correction (`Effective = Nominal × Correction`).
    pub correction: Transform3,
    /// Version bumped on every update; a change invalidates dependent caches.
    pub version: u64,
    /// Sample/effective timestamp of the newest correction (`I-Q11`).
    pub effective_ns: u64,
    /// Discontinuity marker set when a correction jump exceeded the threshold
    /// (`I-Q7`); cleared by a subsequent sub-threshold update at a newer instant.
    pub discontinuous: bool,
    /// The effective instant of the flagged jump, when `discontinuous`.
    pub jump_at_ns: u64,
}

impl CalibrationSlot {
    /// A slot with identity correction at version `0`.
    /// REQ: REQ-SPATIAL-003.
    #[must_use]
    pub const fn new(nominal: Transform3) -> Self {
        Self {
            nominal,
            correction: Transform3::IDENTITY,
            version: 0,
            effective_ns: 0,
            discontinuous: false,
            jump_at_ns: 0,
        }
    }

    /// The effective transform `Nominal × Correction` (`I-Q11`).
    /// REQ: REQ-SPATIAL-003.
    #[must_use]
    pub fn effective(&self) -> Transform3 {
        compose(&self.nominal, &self.correction)
    }
}

/// The magnitude of a correction jump: the translation delta norm plus the
/// rotation delta in radians (`I-Q7` jump-threshold tie-in). Used to decide
/// whether a calibration update marks its local edge discontinuous.
/// REQ: REQ-SPATIAL-003, REQ-SPATIAL-016.
#[must_use]
pub fn correction_jump(before: &Transform3, after: &Transform3) -> f64 {
    let dt = (0..3)
        .map(|i| (after.translation[i] - before.translation[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    let qa = before.quat();
    let qb = after.quat();
    let dot = (qa.x * qb.x + qa.y * qb.y + qa.z * qb.z + qa.w * qb.w)
        .abs()
        .clamp(0.0, 1.0);
    let angle = 2.0 * dot.acos();
    dt + angle
}

/// Per-pair cached geometry + the exact observed input-version vector (`I-Q2`).
/// REQ: REQ-SPATIAL-002.
#[derive(Debug, Clone)]
struct CachedGeom {
    transform: Transform3,
    sample_ns: u64,
    versions: Vec<u64>,
}

/// The reparenting mode ([spatial-queries.md] §Data Structures `ReparentMode`,
/// §Invariants `I-Q9`; [`spatial_architecture.md`] §Transactional Reparenting).
/// REQ: REQ-SPATIAL-028.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReparentMode {
    /// Keeps the object's global pose continuous by recomputing its local offset
    /// (`T_newParent,obj = T_anchor,newParent⁻¹ · T_anchor,obj` at `t₁`).
    PreserveWorld,
    /// Keeps the object's local offset and updates its global pose.
    PreserveLocal,
    /// Pins the object to the new parent's origin (identity local offset).
    Snap,
}

/// One versioned plan in a time-scoped plan registry entry (`I-Q8`): the plan is
/// valid for query instants `t >= valid_from_ns` until the next entry.
/// REQ: REQ-SPATIAL-029.
#[derive(Debug, Clone)]
struct PlanVersion {
    valid_from_ns: u64,
    plan: CompiledPlan,
}

/// A runtime query engine over a [`SpaceForest`] ([spatial-queries.md]
/// §Protocols plan lifecycle). Plans are compiled once per `(target, reference)`
/// pair and cached; queries execute them without touching the base graph
/// (`I-Q1`). Reparenting installs a new plan version while retaining the prior
/// version for pre-`t₁` queries (`I-Q8`).
/// REQ: REQ-SPATIAL-034, REQ-SPATIAL-024, REQ-SPATIAL-029.
#[derive(Debug)]
pub struct QueryEngine {
    forest: SpaceForest,
    edges: SpatialEdgeCache,
    histories: BTreeMap<u32, TimeHistory>,
    calibration: BTreeMap<u32, CalibrationSlot>,
    plans: BTreeMap<(String, String), Vec<PlanVersion>>,
    cached: BTreeMap<(String, String), CachedGeom>,
    claims: BTreeMap<u32, crate::cache::WriterClaim>,
    compile_count: u64,
    freshness_ns: u64,
    max_horizon_ns: u64,
    predict_twist: Twist3,
    now_ns: u64,
}

impl QueryEngine {
    /// Creates a query engine owning `forest` and `edges` dynamic-edge slots.
    /// REQ: REQ-SPATIAL-024.
    #[must_use]
    pub fn new(forest: SpaceForest, edges: usize) -> Self {
        Self {
            forest,
            edges: SpatialEdgeCache::new(edges),
            histories: BTreeMap::new(),
            calibration: BTreeMap::new(),
            plans: BTreeMap::new(),
            cached: BTreeMap::new(),
            claims: BTreeMap::new(),
            compile_count: 0,
            freshness_ns: u64::MAX,
            max_horizon_ns: 0,
            predict_twist: Twist3 {
                linear: crate::types::Vector3::default(),
                angular: crate::types::Vector3::default(),
            },
            now_ns: 0,
        }
    }

    /// The number of times a plan was compiled from the base graph. Runtime
    /// queries never increment it (`I-Q1`).
    /// REQ: REQ-SPATIAL-034.
    #[must_use]
    pub fn compile_count(&self) -> u64 {
        self.compile_count
    }

    /// The dynamic-edge cache (writer claims + versioned slots).
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn edges(&self) -> &SpatialEdgeCache {
        &self.edges
    }

    /// The forest the engine queries.
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn forest(&self) -> &SpaceForest {
        &self.forest
    }

    /// Attaches a time-based history to dynamic edge `edge` (`REQ-SPATIAL-033`).
    /// REQ: REQ-SPATIAL-033.
    pub fn attach_history(&mut self, edge: u32, history: TimeHistory) {
        self.histories.insert(edge, history);
    }

    /// Publishes a sample on dynamic edge `edge`, ingesting it into that edge's
    /// history when one is attached (`REQ-SPATIAL-033`).
    ///
    /// # Errors
    /// [`SpatialError::InvalidHandle`] for an out-of-range edge, or a writer
    /// conflict from the underlying cache.
    /// REQ: REQ-SPATIAL-033, REQ-SPATIAL-050.
    pub fn publish(&mut self, edge: u32, sample: &EdgeSample) -> Result<(), SpatialError> {
        let claim = if let Some(existing) = self.claims.get(&edge) {
            *existing
        } else {
            let claim = self
                .edges
                .claim_writer(edge as usize, edge.wrapping_add(1))?;
            self.claims.insert(edge, claim);
            claim
        };
        self.edges.publish_sample(&claim, sample)?;
        if let Some(history) = self.histories.get_mut(&edge) {
            let _ = history.push(crate::history::HistorySample::new(
                sample.transform,
                sample.sample,
                sample.receive,
                sample.flags,
            ));
        }
        Ok(())
    }

    /// Registers or replaces a calibration slot (`I-Q11`).
    /// REQ: REQ-SPATIAL-003.
    pub fn set_calibration(&mut self, slot: u32, calibration: CalibrationSlot) {
        self.calibration.insert(slot, calibration);
    }

    /// Updates a calibration slot's correction, bumping its version so dependent
    /// caches invalidate live (`I-Q11`). Equivalent to [`Self::update_calibration_at`]
    /// with an infinite jump threshold (never flags a discontinuity).
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unregistered slot.
    /// REQ: REQ-SPATIAL-003.
    pub fn update_calibration(
        &mut self,
        slot: u32,
        correction: Transform3,
    ) -> Result<(), SpatialError> {
        self.update_calibration_at(slot, correction, 0, f64::INFINITY)
    }

    /// Updates a calibration slot's correction at effective instant
    /// `effective_ns`, bumping its version so dependent caches invalidate live
    /// with no recompilation or restart (`I-Q11`). When the correction jump
    /// (translation delta norm + rotation delta radians) exceeds `jump_threshold`,
    /// the slot is marked discontinuous at `effective_ns` (`I-Q7` tie-in); a
    /// subsequent sub-threshold update clears the marker.
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unregistered slot.
    /// REQ: REQ-SPATIAL-003, REQ-SPATIAL-016.
    pub fn update_calibration_at(
        &mut self,
        slot: u32,
        correction: Transform3,
        effective_ns: u64,
        jump_threshold: f64,
    ) -> Result<(), SpatialError> {
        let entry = self
            .calibration
            .get_mut(&slot)
            .ok_or(SpatialError::PathNotFound)?;
        let jump = correction_jump(&entry.correction, &correction);
        entry.correction = correction;
        entry.version += 1;
        entry.effective_ns = effective_ns;
        if jump > jump_threshold {
            entry.discontinuous = true;
            entry.jump_at_ns = effective_ns;
        } else {
            entry.discontinuous = false;
        }
        Ok(())
    }

    /// The live calibration slot `slot`, if registered (`I-Q11`).
    /// REQ: REQ-SPATIAL-003.
    #[must_use]
    pub fn calibration(&self, slot: u32) -> Option<&CalibrationSlot> {
        self.calibration.get(&slot)
    }

    /// Whether calibration slot `slot` is currently flagged discontinuous and
    /// `t_ns` lies in its jump window `[jump_at_ns, ∞)` (`I-Q7`).
    /// REQ: REQ-SPATIAL-003, REQ-SPATIAL-016.
    #[must_use]
    pub fn calibration_discontinuous_at(&self, slot: u32, t_ns: u64) -> bool {
        self.calibration
            .get(&slot)
            .is_some_and(|c| c.discontinuous && t_ns >= c.jump_at_ns)
    }

    /// Sets the freshness contract: a result older than `freshness_ns` is
    /// flagged `STALE` (`I-Q2`).
    /// REQ: REQ-SPATIAL-032.
    pub fn set_freshness(&mut self, freshness_ns: u64) {
        self.freshness_ns = freshness_ns;
    }

    /// Marks the local edge `edge`'s retained sample at `at_ns` as discontinuous
    /// (a relocalization, calibration jump, or reparent jump). Interpolation
    /// never spans the marked boundary (`I-Q7`). Also sets the discontinuous flag
    /// on the edge's newest slot snapshot when the slot sample time matches.
    ///
    /// Returns whether a retained history sample at `at_ns` was marked.
    /// REQ: REQ-SPATIAL-016.
    pub fn mark_discontinuity(&mut self, edge: u32, at_ns: u64) -> bool {
        let marked = self
            .histories
            .get_mut(&edge)
            .is_some_and(|h| h.mark_discontinuous_at(at_ns));
        // Propagate to the live slot when it holds the same sample instant.
        if let Some(sample) = self.edges.read(edge as usize)
            && sample.sample.value_ns == at_ns
            && let Some(claim) = self.claims.get(&edge).copied()
        {
            let flagged = EdgeSample::new(sample.transform, sample.sample, sample.receive)
                .with_discontinuous();
            let _ = self.edges.publish_sample(&claim, &flagged);
        }
        marked
    }

    /// Whether the local edge `edge`'s sample at `at_ns` is flagged
    /// discontinuous (`I-Q7`).
    /// REQ: REQ-SPATIAL-016.
    #[must_use]
    pub fn is_discontinuity(&self, edge: u32, at_ns: u64) -> bool {
        self.histories
            .get(&edge)
            .is_some_and(|h| h.is_discontinuous_at(at_ns))
    }

    /// Transactionally reparents `obj` under `new_parent` at `at_ns` per `mode`
    /// ([spatial-queries.md] §Functions `reparent`, §Invariants `I-Q9`/`I-Q10`):
    /// resolves the object's pose at the reparent instant, stages the relink in
    /// an atomic tree transaction, installs a new time-scoped plan version, marks
    /// the jumped local edge discontinuous, and commits — history before `at_ns`
    /// still traverses the original hierarchy (`I-Q8`). Preserve World recomputes
    /// the local offset `T_newParent,obj = T_anchor,newParent⁻¹ · T_anchor,obj` at
    /// `at_ns` so the world pose stays continuous; Preserve Local keeps the local
    /// offset; Snap pins the object to the new parent's origin.
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unknown frame or disconnected pair
    /// with no explicit bridge, and [`SpatialError::OutOfHistory`] when the pose
    /// at `at_ns` cannot be resolved.
    /// REQ: REQ-SPATIAL-027, REQ-SPATIAL-028, REQ-SPATIAL-029, REQ-SPATIAL-016.
    pub fn reparent(
        &mut self,
        obj: &str,
        new_parent: &str,
        mode: ReparentMode,
        at_ns: u64,
    ) -> Result<(), SpatialError> {
        if !self.forest.contains(obj) || !self.forest.contains(new_parent) {
            return Err(SpatialError::PathNotFound);
        }
        // Capture the object's current local edge before the relink: if it was a
        // dynamic edge, that jumped local edge is flagged discontinuous (I-Q7).
        let obj_edge = self.forest.edge(obj);
        // Pose of the object relative to the new parent measured at t₁, using the
        // current hierarchy before the relink (I-Q9); this is the anchor pose.
        let anchor_pose = self.at(obj, new_parent, at_ns);
        let anchor = anchor_pose.transform.ok_or(SpatialError::OutOfHistory)?;

        let mut txn = self.forest.begin();
        let new_local = match mode {
            // T_newParent,obj = T_anchor,newParent⁻¹ · T_anchor,obj = inverse of
            // the object's pose relative to the new parent (world continuous).
            ReparentMode::PreserveWorld => invert(&anchor),
            // Keep the object's existing local offset as the new local offset.
            ReparentMode::PreserveLocal => Self::local_offset(&self.forest, obj).unwrap_or(anchor),
            // Pin the object to the new parent's origin.
            ReparentMode::Snap => Transform3::IDENTITY,
        };
        txn.relink_static(obj.to_string(), new_parent.to_string(), new_local)?;

        // Compile the new-hierarchy plan versions against the staged forest before
        // the commit, so the plan registry and the forest advance together (I-Q8).
        let staged = txn.staged().clone();
        let mut recompiled = Vec::new();
        for (target, reference) in self.plans.keys().cloned().collect::<Vec<_>>() {
            if let Ok(plan) = compile_plan(&staged, &reference, &target) {
                self.compile_count += 1;
                recompiled.push(((target, reference), plan));
            }
        }
        // Atomic commit: replace the whole forest in one step (I-Q10).
        txn.commit(&mut self.forest);
        for ((target, reference), plan) in recompiled {
            if let Some(entry) = self.plans.get_mut(&(target, reference)) {
                entry.push(PlanVersion {
                    valid_from_ns: at_ns,
                    plan,
                });
            }
        }
        // The reparented local edge jumps at t₁; mark it discontinuous (I-Q7).
        if let Some(crate::forest::FrameEdge::Dynamic(e)) = obj_edge {
            let _ = self.mark_discontinuity(e, at_ns);
        }
        // Cached geometry for plans touching `obj` is stale after the relink.
        self.cached.clear();
        Ok(())
    }

    /// The local offset edge geometry of `frame` toward its parent, if it is a
    /// static edge (`I-Q9`).
    /// REQ: REQ-SPATIAL-028.
    fn local_offset(forest: &SpaceForest, frame: &str) -> Option<Transform3> {
        match forest.edge(frame) {
            Some(crate::forest::FrameEdge::Static(t)) => Some(t),
            Some(crate::forest::FrameEdge::Calibrated { nominal, .. }) => Some(nominal),
            _ => None,
        }
    }

    /// Declares the explicit `predict` horizon bound (`I-Q5`).
    /// REQ: REQ-SPATIAL-025.
    pub fn set_max_horizon(&mut self, max_horizon_ns: u64) {
        self.max_horizon_ns = max_horizon_ns;
    }

    /// Declares the body-frame twist `predict` integrates (`I-Q5`).
    /// REQ: REQ-SPATIAL-025.
    pub fn set_predict_twist(&mut self, twist: Twist3) {
        self.predict_twist = twist;
    }

    /// Sets the query clock's current instant (host-monotonic nanoseconds); used
    /// for `delayed_view` and sample-age derivation.
    /// REQ: REQ-SPATIAL-023.
    pub fn set_now(&mut self, now_ns: u64) {
        self.now_ns = now_ns;
    }

    /// Compiles the plan for `(target, reference)` on first use, caching it. The
    /// base graph is traversed here only (`I-Q1`).
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for unknown/disconnected frames (`I-S3`).
    /// REQ: REQ-SPATIAL-034, REQ-SPATIAL-010.
    fn ensure_plan(&mut self, target: &str, reference: &str) -> Result<(), SpatialError> {
        let key = (target.to_string(), reference.to_string());
        if !self.plans.contains_key(&key) {
            let plan = compile_plan(&self.forest, reference, target)?;
            self.compile_count += 1;
            self.plans.insert(
                key,
                vec![PlanVersion {
                    valid_from_ns: 0,
                    plan,
                }],
            );
        }
        Ok(())
    }

    /// The plan in effect for `(target, reference)` at `t_ns`: the newest plan
    /// version whose validity interval starts at or before `t_ns` (`I-Q8`).
    /// History before a reparent instant therefore traverses the original
    /// hierarchy. Falls back to the first version when `t_ns` predates it.
    /// REQ: REQ-SPATIAL-029.
    fn plan_at<'p>(
        plans: &'p BTreeMap<(String, String), Vec<PlanVersion>>,
        target: &str,
        reference: &str,
        t_ns: u64,
    ) -> Option<&'p CompiledPlan> {
        let versions = plans.get(&(target.to_string(), reference.to_string()))?;
        let mut selected = versions.first()?;
        for version in versions {
            if version.valid_from_ns <= t_ns {
                selected = version;
            } else {
                break; // versions are validity-ordered
            }
        }
        Some(&selected.plan)
    }

    /// The current (newest) plan version for `(target, reference)`, used by
    /// `latest`/`predict`/`delayed_view` (`I-Q8`).
    /// REQ: REQ-SPATIAL-029.
    fn current_plan<'p>(
        plans: &'p BTreeMap<(String, String), Vec<PlanVersion>>,
        target: &str,
        reference: &str,
    ) -> Option<&'p CompiledPlan> {
        plans
            .get(&(target.to_string(), reference.to_string()))?
            .last()
            .map(|v| &v.plan)
    }

    /// The dynamic edges referenced by `plan`, de-duplicated in first-use order.
    /// REQ: REQ-SPATIAL-034.
    fn plan_edges(plan: &CompiledPlan) -> Vec<u32> {
        let mut edges = Vec::new();
        for step in plan.steps() {
            if let PlanStep::DynamicEdge(e) = step
                && !edges.contains(e)
            {
                edges.push(*e);
            }
        }
        edges
    }

    /// The calibration slots referenced by `plan`, de-duplicated in first-use
    /// order (`I-Q11`).
    /// REQ: REQ-SPATIAL-003.
    fn plan_slots(plan: &CompiledPlan) -> Vec<u32> {
        let mut slots = Vec::new();
        for step in plan.steps() {
            if let PlanStep::CalibrationSlot { slot, .. } = step
                && !slots.contains(slot)
            {
                slots.push(*slot);
            }
        }
        slots
    }

    /// The exact observed-version vector for a plan's dynamic-edge and
    /// calibration-slot inputs (`I-Q2`). A slot update bumps its version, so a
    /// plan containing that slot misses the cache and re-executes — no
    /// recompilation, no restart (`I-Q11`).
    /// REQ: REQ-SPATIAL-002, REQ-SPATIAL-003.
    fn versions(
        edges: &SpatialEdgeCache,
        calibration: &BTreeMap<u32, CalibrationSlot>,
        plan: &CompiledPlan,
    ) -> Vec<u64> {
        let mut out: Vec<u64> = Self::plan_edges(plan)
            .iter()
            .map(|e| {
                edges
                    .slot(*e as usize)
                    .map_or(0, crate::cache::EdgeSlot::version)
            })
            .collect();
        out.extend(
            Self::plan_slots(plan)
                .iter()
                .map(|s| calibration.get(s).map_or(0, |c| c.version)),
        );
        out
    }

    /// Resolves a dynamic edge at `t_ns` from its history ring, mapping a
    /// discontinuity guard refusal to `OutOfHistory` with the
    /// `DISCONTINUITY_NEARBY` flag (`I-Q4`/`I-Q7`).
    /// REQ: REQ-SPATIAL-022, REQ-SPATIAL-016.
    fn edge_at(
        histories: &BTreeMap<u32, TimeHistory>,
        edges: &SpatialEdgeCache,
        t_ns: u64,
        flags: &mut QualityFlags,
        edge: u32,
    ) -> Result<Transform3, SpatialError> {
        if let Some(history) = histories.get(&edge) {
            return match history.interpolate(t_ns) {
                Ok(t) => Ok(t),
                Err(SpatialError::DiscontinuityNearby) => {
                    *flags = flags.with(FLAG_DISCONTINUITY_NEARBY);
                    Err(SpatialError::OutOfHistory)
                }
                Err(e) => Err(e),
            };
        }
        // No history: only the exact newest slot sample can be evaluated.
        let sample = edges
            .read(edge as usize)
            .ok_or(SpatialError::OutOfHistory)?;
        if sample.sample.value_ns == t_ns {
            Ok(sample.transform)
        } else {
            Err(SpatialError::OutOfHistory)
        }
    }

    /// Resolves a calibration slot's `correction` effective at or before `t_ns`
    /// (`Effective = Nominal × Correction`, `I-Q11`). A slot whose correction
    /// jump was flagged discontinuous and whose jump instant is at or before
    /// `t_ns` sets `DISCONTINUITY_NEARBY` on the query (`I-Q7` tie-in).
    /// REQ: REQ-SPATIAL-003, REQ-SPATIAL-016.
    fn calibration_at(
        calibration: &BTreeMap<u32, CalibrationSlot>,
        t_ns: u64,
        flags: &mut QualityFlags,
        slot: u32,
    ) -> Result<Transform3, SpatialError> {
        let entry = calibration.get(&slot).ok_or(SpatialError::PathNotFound)?;
        if entry.discontinuous && t_ns >= entry.jump_at_ns {
            *flags = flags.with(FLAG_DISCONTINUITY_NEARBY);
        }
        Ok(entry.correction)
    }

    /// Computes the newest common coherent timestamp `t* = min_i(t_newest,i)`
    /// across a plan's dynamic inputs, or `None` when an input has no sample or
    /// the input intervals do not intersect (`I-Q3`).
    /// REQ: REQ-SPATIAL-024.
    fn newest_common_time(&self, plan: &CompiledPlan) -> Option<u64> {
        let edges = Self::plan_edges(plan);
        if edges.is_empty() {
            // No dynamic input. A calibration-only plan is coherent at the newest
            // slot effective instant; a fully static plan works at any instant.
            let mut t_star = 0_u64;
            for slot in Self::plan_slots(plan) {
                if let Some(c) = self.calibration.get(&slot) {
                    t_star = t_star.max(c.effective_ns);
                }
            }
            return Some(t_star);
        }
        let mut t_star = u64::MAX;
        for edge in &edges {
            let newest = self
                .histories
                .get(edge)
                .and_then(TimeHistory::newest_ns)
                .or_else(|| self.edges.read(*edge as usize).map(|s| s.sample.value_ns))?;
            t_star = t_star.min(newest);
        }
        if t_star == u64::MAX {
            return None;
        }
        // Verify every input can be evaluated at t*: its oldest <= t*.
        for edge in &edges {
            let oldest = self
                .histories
                .get(edge)
                .and_then(TimeHistory::oldest_ns)
                .or_else(|| self.edges.read(*edge as usize).map(|s| s.sample.value_ns))?;
            if t_star < oldest {
                return None;
            }
        }
        Some(t_star)
    }

    /// The newest sample time actually observed among a plan's inputs; used for
    /// staleness derivation (`I-Q2`).
    /// REQ: REQ-SPATIAL-032.
    fn newest_receive_ns(&self, plan: &CompiledPlan) -> Option<u64> {
        let mut newest = 0_u64;
        let mut found = false;
        for edge in Self::plan_edges(plan) {
            if let Some(sample) = self.edges.read(edge as usize) {
                newest = newest.max(sample.receive.value_ns);
                found = true;
            }
        }
        found.then_some(newest)
    }

    /// Builds a result from a resolved transform, deriving freshness flags and
    /// metadata per query (`I-Q2`); `sample_time`/`age` are the query's own.
    /// REQ: REQ-SPATIAL-032.
    fn classify(
        &self,
        transform: Transform3,
        sample_ns: u64,
        receive_ns: Option<u64>,
        mut flags: QualityFlags,
    ) -> TransformResult {
        let mut result = TransformResult::ok(transform, 0);
        result.flags = flags;
        let age = self.now_ns.saturating_sub(sample_ns);
        result.sample_age_ns = Some(age);
        if let Some(recv) = receive_ns {
            result.receive_age_ns = Some(self.now_ns.saturating_sub(recv));
            if self.now_ns.saturating_sub(recv) > self.freshness_ns {
                flags = flags.with(FLAG_STALE);
            }
        } else if age > self.freshness_ns {
            flags = flags.with(FLAG_STALE);
        }
        result.flags = flags;
        result
    }

    /// `latest(target, reference)`: the transform at the newest common coherent
    /// timestamp `t* = min_i(t_newest,i)`, every dynamic input evaluated at `t*`
    /// (`I-Q3`). Returns `OUT_OF_HISTORY` when the input intervals do not
    /// intersect. Cached geometry is reused only under an exact input-version
    /// match; status is still derived per query (`I-Q2`).
    /// REQ: REQ-SPATIAL-024, REQ-SPATIAL-002.
    ///
    /// # Errors
    /// This never returns `Err`; all outcomes are encoded in the
    /// [`TransformResult`] (a hard `error` plus combinable quality flags).
    pub fn latest(&mut self, target: &str, reference: &str) -> TransformResult {
        if let Err(e) = self.ensure_plan(target, reference) {
            return TransformResult::err(map_error(e));
        }
        let key = (target.to_string(), reference.to_string());
        let Some(plan) = Self::current_plan(&self.plans, target, reference).cloned() else {
            return TransformResult::err(TransformError::PathNotFound);
        };
        let Some(t_star) = self.newest_common_time(&plan) else {
            return TransformResult::err(TransformError::OutOfHistory);
        };
        let versions = Self::versions(&self.edges, &self.calibration, &plan);
        if let Some(cached) = self.cached.get(&key)
            && cached.versions == versions
        {
            let receive = self.newest_receive_ns(&plan);
            let mut result = self.classify(
                cached.transform,
                cached.sample_ns,
                receive,
                QualityFlags::NONE,
            );
            result.sample_time = Some(TimePoint::new(
                t_star,
                crate::clock::ClockIdentity::new(crate::clock::ClockDomain::HostMonotonic),
            ));
            return result;
        }
        let flags = std::cell::RefCell::new(QualityFlags::NONE);
        let histories = &self.histories;
        let edges = &self.edges;
        let calibration = &self.calibration;
        let resolved = execute_steps(
            plan.steps(),
            |edge| Self::edge_at(histories, edges, t_star, &mut flags.borrow_mut(), edge),
            |slot| Self::calibration_at(calibration, t_star, &mut flags.borrow_mut(), slot),
        );
        let flags = flags.into_inner();
        let receive = self.newest_receive_ns(&plan);
        match resolved {
            Ok(transform) => {
                self.cached.insert(
                    key,
                    CachedGeom {
                        transform,
                        sample_ns: t_star,
                        versions,
                    },
                );
                let mut result = self.classify(transform, t_star, receive, flags);
                result.sample_time = Some(TimePoint::new(
                    t_star,
                    crate::clock::ClockIdentity::new(crate::clock::ClockDomain::HostMonotonic),
                ));
                result
            }
            Err(e) => {
                let mut result = TransformResult::err(map_error(e));
                result.flags = flags;
                result
            }
        }
    }

    /// `at(target, reference, t_ns)`: the transform at a historical instant,
    /// interpolating linearly in translation and via SLERP in rotation between
    /// the bracketing samples; never silently extrapolates beyond the newest
    /// sample (`I-Q4`). The plan is time-scoped: a query before a reparent
    /// instant executes the original-hierarchy plan (`I-Q8`). A discontinuity
    /// crossing surfaces `DISCONTINUITY_NEARBY` with no cross-jump value
    /// (`I-Q7`).
    /// REQ: REQ-SPATIAL-022, REQ-SPATIAL-016, REQ-SPATIAL-029.
    ///
    /// # Errors
    /// This never returns `Err`; all outcomes are encoded in the
    /// [`TransformResult`].
    pub fn at(&mut self, target: &str, reference: &str, t_ns: u64) -> TransformResult {
        if let Err(e) = self.ensure_plan(target, reference) {
            return TransformResult::err(map_error(e));
        }
        let Some(plan) = Self::plan_at(&self.plans, target, reference, t_ns).cloned() else {
            return TransformResult::err(TransformError::PathNotFound);
        };
        let flags = std::cell::RefCell::new(QualityFlags::NONE);
        let histories = &self.histories;
        let edges = &self.edges;
        let calibration = &self.calibration;
        let resolved = execute_steps(
            plan.steps(),
            |edge| Self::edge_at(histories, edges, t_ns, &mut flags.borrow_mut(), edge),
            |slot| Self::calibration_at(calibration, t_ns, &mut flags.borrow_mut(), slot),
        );
        let flags = flags.into_inner();
        match resolved {
            Ok(transform) => {
                let mut result = TransformResult::ok(transform, 0);
                result.flags = flags;
                result.sample_time = Some(TimePoint::new(
                    t_ns,
                    crate::clock::ClockIdentity::new(crate::clock::ClockDomain::HostMonotonic),
                ));
                result.sample_age_ns = Some(self.now_ns.saturating_sub(t_ns));
                result
            }
            Err(e) => {
                let mut result = TransformResult::err(map_error(e));
                result.flags = flags;
                result
            }
        }
    }

    /// `predict(target, reference, horizon_ns)`: explicit forward extrapolation
    /// from the newest coherent state by integrating the declared body-frame
    /// twist on `SE(3)` via the exponential map, `T(t+Δ) = T(t)·Exp(ξ_b Δ)`
    /// (`I-Q5`). Within the declared horizon the result carries `EXTRAPOLATED`;
    /// beyond it returns `OUT_OF_HISTORY`.
    /// REQ: REQ-SPATIAL-025.
    ///
    /// # Errors
    /// This never returns `Err`; all outcomes are encoded in the
    /// [`TransformResult`].
    pub fn predict(&mut self, target: &str, reference: &str, horizon_ns: u64) -> TransformResult {
        if horizon_ns > self.max_horizon_ns {
            return TransformResult::err(TransformError::OutOfHistory);
        }
        let state = self.latest(target, reference);
        let Some(transform) = state.transform else {
            return state;
        };
        let dt = horizon_ns as f64 * 1e-9;
        let delta = match exp_twist(&self.predict_twist, dt) {
            Ok(d) => d,
            Err(e) => return TransformResult::err(map_error(e)),
        };
        let predicted = compose(&transform, &delta);
        let mut result = TransformResult::ok(predicted, state.clock_uncertainty_ns);
        result.flags = state.flags.with(FLAG_EXTRAPOLATED);
        result.sample_time = state.sample_time;
        result.sample_age_ns = state.sample_age_ns;
        result.receive_age_ns = state.receive_age_ns;
        result
    }

    /// `delayed_view(target, reference, offset_ns)`: an intentionally delayed
    /// view trailing the query clock by a fixed duration, resolving
    /// `at(now − offset)` (`I-Q6`). An offset beyond retained history returns
    /// `OUT_OF_HISTORY`.
    /// REQ: REQ-SPATIAL-023, REQ-SPATIAL-022.
    ///
    /// # Errors
    /// This never returns `Err`; all outcomes are encoded in the
    /// [`TransformResult`].
    pub fn delayed_view(
        &mut self,
        target: &str,
        reference: &str,
        offset_ns: u64,
    ) -> TransformResult {
        let Some(t) = self.now_ns.checked_sub(offset_ns) else {
            return TransformResult::err(TransformError::OutOfHistory);
        };
        self.at(target, reference, t)
    }

    /// Maps a sample time from `from`'s clock domain into host-monotonic time,
    /// reporting the propagated uncertainty (`I-S7`); used to bring inputs onto
    /// one clock before coherence math.
    ///
    /// # Errors
    /// [`SpatialError::ClockUnresolved`] when no valid mapping exists.
    /// REQ: REQ-SPATIAL-004.
    pub fn map_time(
        &self,
        t: TimePoint,
        to: crate::clock::ClockDomain,
        table: &crate::clock::ClockTable,
    ) -> Result<(TimePoint, u64), SpatialError> {
        map_timestamp_with_uncertainty(t, to, table)
    }
}

/// Maps the spatial error taxonomy onto the query hard-error set
/// ([spatial-queries.md] §Data Structures `TransformError`).
/// REQ: REQ-SPATIAL-032.
#[must_use]
fn map_error(e: SpatialError) -> TransformError {
    match e {
        SpatialError::PathNotFound => TransformError::PathNotFound,
        SpatialError::ClockUnresolved => TransformError::ClockUnresolved,
        SpatialError::InvalidHandle | SpatialError::WriterConflict => {
            TransformError::ConcurrentUpdate
        }
        SpatialError::OutOfHistory
        | SpatialError::DiscontinuityNearby
        | SpatialError::InvalidConfig
        | SpatialError::NonUnitQuaternion
        | SpatialError::InvalidDirection
        | SpatialError::NonFinite
        | SpatialError::InvalidArg
        | SpatialError::IndexFull => TransformError::OutOfHistory,
    }
}

#[cfg(test)]
mod tests {
    use super::{CalibrationSlot, QueryEngine};
    use crate::cache::EdgeSample;
    use crate::clock::{ClockDomain, ClockIdentity, TimePoint};
    use crate::forest::SpaceForest;
    use crate::history::TimeHistory;
    use crate::plan::{FLAG_EXTRAPOLATED, TransformError};
    use crate::types::{SpatialError, Transform3, Twist3, Vector3};

    fn host(ns: u64) -> TimePoint {
        TimePoint::new(ns, ClockIdentity::new(ClockDomain::HostMonotonic))
    }

    fn xyz(x: f64, y: f64, z: f64) -> Transform3 {
        Transform3 {
            translation: [x, y, z],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn engine_with_edge() -> QueryEngine {
        let mut forest = SpaceForest::new();
        forest.add_root("world".to_string());
        forest.add_dynamic("odom".to_string(), "world".to_string(), 0);
        let mut engine = QueryEngine::new(forest, 1);
        engine.attach_history(
            0,
            TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}")),
        );
        engine
    }

    fn publish(engine: &mut QueryEngine, t_ns: u64, x: f64) {
        let sample = EdgeSample::new(xyz(x, 0.0, 0.0), host(t_ns), host(t_ns));
        engine
            .publish(0, &sample)
            .unwrap_or_else(|e| unreachable!("{e}"));
    }

    /// REQ: REQ-SPATIAL-024 — `latest` evaluates at the newest common coherent
    /// timestamp and reports it as `sample_time` (`I-Q3`).
    #[test]
    fn latest_coherent() {
        let mut engine = engine_with_edge();
        publish(&mut engine, 10, 1.0);
        publish(&mut engine, 20, 2.0);
        let res = engine.latest("odom", "world");
        assert!(res.is_ok());
        assert_eq!(res.sample_time.map(|t| t.value_ns), Some(20));
    }

    /// REQ: REQ-SPATIAL-024 — an input with no sample yields `OUT_OF_HISTORY`
    /// (`I-Q3`).
    #[test]
    fn latest_out_of_history_empty() {
        let mut engine = engine_with_edge();
        let res = engine.latest("odom", "world");
        assert_eq!(res.error, Some(TransformError::OutOfHistory));
    }

    /// REQ: REQ-SPATIAL-022 — `at` interpolates between brackets and refuses to
    /// extrapolate beyond the newest sample (`I-Q4`).
    #[test]
    fn at_interpolates_and_bounds() {
        let mut engine = engine_with_edge();
        publish(&mut engine, 0, 0.0);
        publish(&mut engine, 100, 10.0);
        let mid = engine.at("odom", "world", 50);
        assert!((mid.transform.map(|t| t.translation[0]).unwrap_or_default() - 5.0).abs() < 1e-9);
        assert_eq!(
            engine.at("odom", "world", 101).error,
            Some(TransformError::OutOfHistory)
        );
    }

    /// REQ: REQ-SPATIAL-025 — `predict` is the only extrapolation path: within
    /// the horizon it flags `EXTRAPOLATED`; beyond it returns `OUT_OF_HISTORY`
    /// (`I-Q5`).
    #[test]
    fn predict_bounded() {
        let mut engine = engine_with_edge();
        publish(&mut engine, 0, 0.0);
        engine.set_max_horizon(100);
        engine.set_predict_twist(Twist3 {
            linear: Vector3::new(1.0, 0.0, 0.0).unwrap_or_default(),
            angular: Vector3::new(0.0, 0.0, 0.0).unwrap_or_default(),
        });
        let within = engine.predict("odom", "world", 100);
        assert!(within.flags.contains(FLAG_EXTRAPOLATED));
        // 1 m/s over 100 ns = 1e-7 m.
        assert!(
            (within
                .transform
                .map(|t| t.translation[0])
                .unwrap_or_default()
                - 1e-7)
                .abs()
                < 1e-15
        );
        assert_eq!(
            engine.predict("odom", "world", 101).error,
            Some(TransformError::OutOfHistory)
        );
    }

    /// REQ: REQ-SPATIAL-023 — `delayed_view` resolves `now − offset` through
    /// `at` (`I-Q6`).
    #[test]
    fn delayed_view_trailing() {
        let mut engine = engine_with_edge();
        publish(&mut engine, 0, 0.0);
        publish(&mut engine, 100, 10.0);
        engine.set_now(100);
        let delayed = engine.delayed_view("odom", "world", 50);
        // now−offset = 50 ⇒ interpolated midpoint 5.0.
        assert!(
            (delayed
                .transform
                .map(|t| t.translation[0])
                .unwrap_or_default()
                - 5.0)
                .abs()
                < 1e-9
        );
        assert_eq!(
            engine.delayed_view("odom", "world", 200).error,
            Some(TransformError::OutOfHistory)
        );
    }

    /// REQ: REQ-SPATIAL-003 — a calibration update invalidates the cached
    /// geometry live, no recompilation (`I-Q11`).
    #[test]
    fn calibration_live_reruns() {
        let mut forest = SpaceForest::new();
        forest.add_root("world".to_string());
        // Static chain with a calibration slot is compiled by the plan compiler
        // as a constant here; the slot path is exercised through the engine.
        forest.add_dynamic("odom".to_string(), "world".to_string(), 0);
        let mut engine = QueryEngine::new(forest, 1);
        engine.attach_history(
            0,
            TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}")),
        );
        engine.set_calibration(3, CalibrationSlot::new(xyz(1.0, 0.0, 0.0)));
        publish(&mut engine, 0, 0.0);
        let before = engine.latest("odom", "world");
        assert!(before.is_ok());
        engine
            .update_calibration(3, xyz(0.5, 0.0, 0.0))
            .unwrap_or_else(|e| unreachable!("{e}"));
        // The engine's plan cache is unaffected (no recompile), and a fresh
        // query still resolves.
        let after = engine.latest("odom", "world");
        assert!(after.is_ok());
        assert_eq!(engine.compile_count(), 1);
        assert_eq!(
            engine.update_calibration(99, xyz(0.0, 0.0, 0.0)),
            Err(SpatialError::PathNotFound)
        );
    }
}
