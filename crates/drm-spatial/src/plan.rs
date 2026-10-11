//! Compiled transform plans and the versioned per-process cache
//! ([spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Data Structures `TransformPlan`/`PlanCache`, §Invariants `I-Q1`/`I-Q2`;
//! [`spatial_architecture.md`](../../docs/spatial_architecture.md) §Versioned
//! Local Cache). Runtime queries **execute** a plan — multiply steps plus
//! version-checked edge reads — and never traverse the base graph (`I-Q1`).
//!
//! The per-process [`PlanCache`] stores only the cached **geometry** and an
//! exact input-version vector. When every observed version is unchanged the
//! cached transform is reused with zero multiplications; freshness, staleness,
//! and clock uncertainty are always derived per query from timestamps, so
//! version equality never implies `VALID` (`I-Q2`).
//!
//! A runtime overlay query resolves through a bounded overlay walk (≤
//! `max_overlay_depth` parent steps) to a compiled-base attachment frame, then
//! the precompiled base plan; the small prefix is cached under
//! `topology_generation`, so a reparent or overlay mutation re-walks once
//! ([`OverlayPlanCache`], [`compile_overlay_plan`], `I-Q16`).
//!
//! REQ: REQ-SPATIAL-034, REQ-SPATIAL-002, REQ-SPATIAL-050, REQ-SPATIAL-008.

use crate::cache::EdgeSlot;
use crate::clock::TimePoint;
use crate::types::{SpatialError, Transform3};

/// A compiled transform-plan step ([spatial-queries.md] §Data Structures).
/// REQ: REQ-SPATIAL-034.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanStep {
    /// A build-time collapsed static run.
    Constant(Transform3),
    /// A runtime cache read of dynamic edge `edge` (version-checked).
    DynamicEdge(u32),
    /// A live calibration slot: the effective value is `nominal × correction`,
    /// so a slot update invalidates dependent caches without recompilation
    /// (`I-Q11`).
    CalibrationSlot {
        /// The nominal static transform embedded at compile time.
        nominal: Transform3,
        /// The shared calibration slot id.
        slot: u32,
    },
    /// Replaces the top of the stack with its inverse (`T⁻¹`). Emitted when a
    /// calibration-bound edge is traversed in the reverse direction, so the
    /// inverted effective value is exactly `(Nominal × Correction)⁻¹` (`I-Q11`).
    Invert,
    /// Fold the top two stack transforms (`a ∘ b`).
    Multiply,
}

/// A compiled plan for one `(target, reference)` query pair. The step slice is a
/// generated, fixed artifact (`I-Q1`).
/// REQ: REQ-SPATIAL-034.
#[derive(Debug)]
pub struct TransformPlan {
    steps: &'static [PlanStep],
    max_stack: u16,
}

impl TransformPlan {
    /// Wraps a generated step slice, checking the stack effect (final depth
    /// exactly one). Compilation happens at build time; this only guards against
    /// a malformed artifact.
    ///
    /// # Errors
    /// [`SpatialError::NonFinite`] when the stack underflows or does not end at
    /// depth one (a malformed compiled plan).
    /// REQ: REQ-SPATIAL-034.
    pub fn new(steps: &'static [PlanStep]) -> Result<Self, SpatialError> {
        let mut depth: i64 = 0;
        let mut max_depth: i64 = 0;
        for step in steps {
            match step {
                PlanStep::Constant(_)
                | PlanStep::DynamicEdge(_)
                | PlanStep::CalibrationSlot { .. } => {
                    depth += 1;
                }
                PlanStep::Invert => {
                    if depth < 1 {
                        return Err(SpatialError::NonFinite);
                    }
                }
                PlanStep::Multiply => {
                    depth -= 1;
                    if depth < 1 {
                        return Err(SpatialError::NonFinite);
                    }
                }
            }
            max_depth = max_depth.max(depth);
        }
        if depth != 1 {
            return Err(SpatialError::NonFinite);
        }
        let max_stack = u16::try_from(max_depth).map_err(|_| SpatialError::NonFinite)?;
        Ok(Self { steps, max_stack })
    }

    /// The generated plan steps.
    /// REQ: REQ-SPATIAL-034.
    #[must_use]
    pub fn steps(&self) -> &'static [PlanStep] {
        self.steps
    }

    /// The maximum fixed-stack depth the plan needs.
    /// REQ: REQ-SPATIAL-034.
    #[must_use]
    pub fn max_stack(&self) -> u16 {
        self.max_stack
    }
}

/// Maximum fixed transform-stack depth for plan execution (`I-Q15`).
/// REQ: REQ-SPATIAL-034.
pub const MAX_PLAN_DEPTH: usize = 32;

/// Bit `0` of [`QualityFlags`]: the result is stale relative to its timing
/// contract.
/// REQ: REQ-SPATIAL-032.
pub const FLAG_STALE: u32 = 1 << 0;
/// Bit `1` of [`QualityFlags`]: the result was extrapolated (`predict`).
/// REQ: REQ-SPATIAL-032.
pub const FLAG_EXTRAPOLATED: u32 = 1 << 1;
/// Bit `2` of [`QualityFlags`]: a discontinuity lies nearby in the query window.
/// REQ: REQ-SPATIAL-032.
pub const FLAG_DISCONTINUITY_NEARBY: u32 = 1 << 2;

/// One hard query error ([spatial-queries.md] §Data Structures; these are the
/// spatial-only outcomes, distinct from the combinable quality flags).
/// REQ: REQ-SPATIAL-032.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformError {
    /// The target frame is not connected to the query frame (`I-S3`).
    PathNotFound,
    /// No valid clock mapping exists (`I-S7`).
    ClockUnresolved,
    /// The requested timestamp is outside retained history.
    OutOfHistory,
    /// A bounded snapshot failed under concurrent update (`I-Q14`).
    ConcurrentUpdate,
}

/// Combinable quality flags (`STALE | EXTRAPOLATED | DISCONTINUITY_NEARBY`).
/// REQ: REQ-SPATIAL-032.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QualityFlags(pub u32);

impl QualityFlags {
    /// No flags set.
    /// REQ: REQ-SPATIAL-032.
    pub const NONE: Self = Self(0);

    /// Whether all bits in `mask` are set.
    /// REQ: REQ-SPATIAL-032.
    #[must_use]
    pub const fn contains(self, mask: u32) -> bool {
        self.0 & mask == mask
    }

    /// Returns `self` with `mask` bits set.
    /// REQ: REQ-SPATIAL-032.
    #[must_use]
    pub const fn with(mut self, mask: u32) -> Self {
        self.0 |= mask;
        self
    }
}

/// A query result: a transform or one hard error, combinable quality flags, and
/// status metadata. Freshness/staleness/uncertainty are derived per query, never
/// cached (`I-Q2`, REQ-SPATIAL-002).
/// REQ: REQ-SPATIAL-032, REQ-SPATIAL-002.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformResult {
    /// The transform, when resolved.
    pub transform: Option<Transform3>,
    /// The single hard error, when unresolved.
    pub error: Option<TransformError>,
    /// Combinable quality flags.
    pub flags: QualityFlags,
    /// The sample time the result corresponds to (`t*` for `latest`).
    pub sample_time: Option<TimePoint>,
    /// Age against the query clock, nanoseconds.
    pub sample_age_ns: Option<u64>,
    /// Receive-time age (staleness), nanoseconds.
    pub receive_age_ns: Option<u64>,
    /// Clock-synchronization uncertainty, nanoseconds (from mapping estimates).
    pub clock_uncertainty_ns: u64,
}

impl TransformResult {
    /// A successful result carrying `transform` and clock `uncertainty_ns`.
    /// REQ: REQ-SPATIAL-032.
    #[must_use]
    pub const fn ok(transform: Transform3, uncertainty_ns: u64) -> Self {
        Self {
            transform: Some(transform),
            error: None,
            flags: QualityFlags::NONE,
            sample_time: None,
            sample_age_ns: None,
            receive_age_ns: None,
            clock_uncertainty_ns: uncertainty_ns,
        }
    }

    /// A hard-error result.
    /// REQ: REQ-SPATIAL-032.
    #[must_use]
    pub const fn err(error: TransformError) -> Self {
        Self {
            transform: None,
            error: Some(error),
            flags: QualityFlags::NONE,
            sample_time: None,
            sample_age_ns: None,
            receive_age_ns: None,
            clock_uncertainty_ns: 0,
        }
    }

    /// Whether the result carries a transform.
    /// REQ: REQ-SPATIAL-032.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.transform.is_some()
    }
}

/// Executes a plan at one instant, resolving each dynamic-edge input through
/// `edge_at` and each calibration input through `slot_at` (both version-checked
/// reads supplied by the caller). Zero base-graph traversal: only stack work and
/// slot reads (`I-Q1`). The plan is a straight-line fold; the fixed stack is
/// [`MAX_PLAN_DEPTH`] deep and no heap allocation occurs (`I-Q15`).
///
/// # Errors
/// [`SpatialError`] from `edge_at`/`slot_at`, or [`SpatialError::NonFinite`] for
/// a malformed plan / stack underflow.
/// REQ: REQ-SPATIAL-034, REQ-SPATIAL-002.
pub fn execute_plan(
    plan: &TransformPlan,
    edge_at: impl FnMut(u32) -> Result<Transform3, SpatialError>,
    slot_at: impl FnMut(u32) -> Result<Transform3, SpatialError>,
) -> Result<Transform3, SpatialError> {
    execute_steps(plan.steps(), edge_at, slot_at)
}

/// Executes a plan step slice at one instant. This is the single executor shared
/// by the generated [`TransformPlan`] and runtime [`compile_plan`] output: it
/// pushes steps onto a fixed [`MAX_PLAN_DEPTH`] stack, resolving each
/// `DynamicEdge`/`CalibrationSlot` through the caller's temporal resolvers
/// (`I-Q1`). `slot_at` returns the calibration **correction**; the effective
/// value is `nominal × correction` (`I-Q11`).
///
/// # Errors
/// [`SpatialError`] from `edge_at`/`slot_at`, or [`SpatialError::NonFinite`] for
/// a malformed plan / stack underflow.
/// REQ: REQ-SPATIAL-034, REQ-SPATIAL-002, REQ-SPATIAL-003.
pub fn execute_steps(
    steps: &[PlanStep],
    mut edge_at: impl FnMut(u32) -> Result<Transform3, SpatialError>,
    mut slot_at: impl FnMut(u32) -> Result<Transform3, SpatialError>,
) -> Result<Transform3, SpatialError> {
    let mut stack: [Transform3; MAX_PLAN_DEPTH] = [Transform3::IDENTITY; MAX_PLAN_DEPTH];
    let mut top = 0_usize;
    for step in steps {
        match step {
            PlanStep::Constant(t) => {
                if top >= MAX_PLAN_DEPTH {
                    return Err(SpatialError::NonFinite);
                }
                stack[top] = *t;
                top += 1;
            }
            PlanStep::DynamicEdge(edge) => {
                if top >= MAX_PLAN_DEPTH {
                    return Err(SpatialError::NonFinite);
                }
                stack[top] = edge_at(*edge)?;
                top += 1;
            }
            PlanStep::CalibrationSlot { nominal, slot } => {
                if top >= MAX_PLAN_DEPTH {
                    return Err(SpatialError::NonFinite);
                }
                let correction = slot_at(*slot)?;
                stack[top] = crate::transform::compose(nominal, &correction);
                top += 1;
            }
            PlanStep::Multiply => {
                if top < 2 {
                    return Err(SpatialError::NonFinite);
                }
                let b = stack[top - 1];
                let a = stack[top - 2];
                top -= 1;
                stack[top - 1] = crate::transform::compose(&a, &b);
            }
            PlanStep::Invert => {
                if top < 1 {
                    return Err(SpatialError::NonFinite);
                }
                stack[top - 1] = crate::transform::invert(&stack[top - 1]);
            }
        }
    }
    if top != 1 {
        return Err(SpatialError::NonFinite);
    }
    Ok(stack[0])
}

/// A runtime-compiled, owned transform plan for one `(target, reference)` pair
/// ([spatial-queries.md] §Functions `compile_plan`). Unlike the `'static`
/// generated [`TransformPlan`], a [`CompiledPlan`] owns its steps so the runtime
/// can compile plans from the live [`SpaceForest`](crate::forest::SpaceForest)
/// (`I-Q1`).
/// REQ: REQ-SPATIAL-034.
#[derive(Debug, Clone)]
pub struct CompiledPlan {
    steps: Vec<PlanStep>,
    max_stack: u16,
}

impl CompiledPlan {
    /// Wraps runtime-compiled steps, verifying the stack effect (final depth
    /// exactly one).
    ///
    /// # Errors
    /// [`SpatialError::NonFinite`] for a malformed step sequence.
    /// REQ: REQ-SPATIAL-034.
    pub fn new(steps: Vec<PlanStep>) -> Result<Self, SpatialError> {
        let mut depth: i64 = 0;
        let mut max_depth: i64 = 0;
        for step in &steps {
            match step {
                PlanStep::Constant(_)
                | PlanStep::DynamicEdge(_)
                | PlanStep::CalibrationSlot { .. } => depth += 1,
                PlanStep::Invert => {
                    if depth < 1 {
                        return Err(SpatialError::NonFinite);
                    }
                }
                PlanStep::Multiply => {
                    depth -= 1;
                    if depth < 1 {
                        return Err(SpatialError::NonFinite);
                    }
                }
            }
            max_depth = max_depth.max(depth);
        }
        if depth != 1 {
            return Err(SpatialError::NonFinite);
        }
        let max_stack = u16::try_from(max_depth).map_err(|_| SpatialError::NonFinite)?;
        Ok(Self { steps, max_stack })
    }

    /// The compiled steps.
    /// REQ: REQ-SPATIAL-034.
    #[must_use]
    pub fn steps(&self) -> &[PlanStep] {
        &self.steps
    }

    /// The maximum fixed-stack depth the plan needs.
    /// REQ: REQ-SPATIAL-034.
    #[must_use]
    pub fn max_stack(&self) -> u16 {
        self.max_stack
    }
}

/// Compiles a plan for the `(reference → target)` query pair from a live
/// [`SpaceForest`](crate::forest::SpaceForest): a build-time collapsed static
/// run becomes [`PlanStep::Constant`], a dynamic edge becomes
/// [`PlanStep::DynamicEdge`], bridge transforms collapse to constants, and
/// adjacent terms are folded by [`PlanStep::Multiply`]. Compilation is the only
/// place graph traversal happens (`I-Q1`).
///
/// # Errors
/// [`SpatialError::PathNotFound`] for an unknown/disconnected pair (`I-S3`).
/// REQ: REQ-SPATIAL-034, REQ-SPATIAL-010.
pub fn compile_plan(
    forest: &crate::forest::SpaceForest,
    reference: &str,
    target: &str,
) -> Result<CompiledPlan, SpatialError> {
    use crate::forest::PathEdge;
    let path = forest.path_between(reference, target)?;
    if path.is_empty() {
        return Ok(CompiledPlan {
            steps: vec![PlanStep::Constant(Transform3::IDENTITY)],
            max_stack: 1,
        });
    }
    let mut steps: Vec<PlanStep> = Vec::with_capacity(path.len() * 2);
    for (i, edge) in path.iter().enumerate() {
        match edge {
            PathEdge::Constant(t) | PathEdge::Bridge(t) => {
                steps.push(PlanStep::Constant(*t));
            }
            PathEdge::Dynamic(e) => steps.push(PlanStep::DynamicEdge(*e)),
            PathEdge::Calibrated {
                nominal,
                slot,
                inverted,
            } => {
                steps.push(PlanStep::CalibrationSlot {
                    nominal: *nominal,
                    slot: *slot,
                });
                if *inverted {
                    steps.push(PlanStep::Invert);
                }
            }
        }
        if i != 0 {
            steps.push(PlanStep::Multiply);
        }
    }
    CompiledPlan::new(steps)
}

/// A per-pair overlay-plan cache keyed by `topology_generation`
/// ([spatial-queries.md] §Protocols plan lifecycle step 3; §Invariants `I-Q16`).
/// A runtime overlay query resolves through a **bounded overlay walk** — at most
/// `max_overlay_depth` parent steps from the overlay frame up to its
/// compiled-base attachment frame, then the precompiled base plan. The compiled
/// overlay plan is cached under the current `topology_generation`; a matching
/// generation reuses it with **no re-walk**, and a generation bump (a reparent
/// or overlay create/delete) re-walks exactly once on the next query.
/// REQ: REQ-SPATIAL-008, REQ-SPATIAL-034.
#[derive(Debug, Clone)]
pub struct OverlayPlanCache {
    topology_generation: u64,
    depth: u16,
    attachment: String,
    plan: CompiledPlan,
    walks: u64,
    built: bool,
}

impl OverlayPlanCache {
    /// A cache primed for `topology_generation`, not yet built (the first query
    /// re-walks once).
    /// REQ: REQ-SPATIAL-008.
    #[must_use]
    pub fn new(topology_generation: u64) -> Self {
        Self {
            topology_generation,
            depth: 0,
            attachment: String::new(),
            plan: CompiledPlan {
                steps: vec![PlanStep::Constant(Transform3::IDENTITY)],
                max_stack: 1,
            },
            walks: 0,
            built: false,
        }
    }

    /// The `topology_generation` the cache currently holds (`I-Q16`).
    /// REQ: REQ-SPATIAL-008.
    #[must_use]
    pub fn topology_generation(&self) -> u64 {
        self.topology_generation
    }

    /// The overlay walk depth of the cached plan (`I-Q16`).
    /// REQ: REQ-SPATIAL-008.
    #[must_use]
    pub fn depth(&self) -> u16 {
        self.depth
    }

    /// The compiled-base attachment frame of the cached overlay walk (`I-Q16`).
    /// REQ: REQ-SPATIAL-008.
    #[must_use]
    pub fn attachment(&self) -> &str {
        &self.attachment
    }

    /// The number of overlay-walk recompilations performed (`I-Q16`): one per
    /// first build plus one per observed `topology_generation` bump.
    /// REQ: REQ-SPATIAL-008.
    #[must_use]
    pub fn walks(&self) -> u64 {
        self.walks
    }
}

/// Resolves the overlay plan for the `(reference → target)` pair, honoring the
/// bounded overlay walk and the `topology_generation` cache (`I-Q16`). When
/// `cache.topology_generation == topology_generation` the cached plan is reused
/// with no graph walk; otherwise the plan is (re)compiled via [`compile_plan`]
/// and stored once, so a reparent or overlay mutation costs exactly one re-walk.
///
/// # Errors
/// [`SpatialError::InvalidConfig`] when either endpoint's overlay depth exceeds
/// `max_overlay_depth` (the walk is refused, never unbounded), or
/// [`SpatialError::PathNotFound`] for an unknown/disconnected pair (`I-S3`).
/// REQ: REQ-SPATIAL-008, REQ-SPATIAL-034.
pub fn compile_overlay_plan(
    forest: &crate::forest::SpaceForest,
    reference: &str,
    target: &str,
    max_overlay_depth: u16,
    topology_generation: u64,
    cache: &mut OverlayPlanCache,
) -> Result<CompiledPlan, SpatialError> {
    let depth = forest
        .overlay_depth(reference)
        .max(forest.overlay_depth(target));
    if depth > max_overlay_depth {
        return Err(SpatialError::InvalidConfig);
    }
    if cache.built && cache.topology_generation == topology_generation {
        return Ok(cache.plan.clone());
    }
    let plan = compile_plan(forest, reference, target)?;
    cache.plan = plan.clone();
    cache.depth = depth;
    // Record the compiled-base attachment of the overlay endpoint (the deeper
    // of the two), not of an arbitrary endpoint.
    cache.attachment = if forest.overlay_depth(reference) >= forest.overlay_depth(target) {
        forest.overlay_attachment(reference)
    } else {
        forest.overlay_attachment(target)
    };
    cache.topology_generation = topology_generation;
    cache.walks += 1;
    cache.built = true;
    Ok(plan)
}

/// The exact observed-version vector for a plan's dynamic inputs; no hash
/// collision, so version equality is exact (`I-Q2`).
/// REQ: REQ-SPATIAL-002.
#[derive(Debug, Clone)]
pub struct ObservedVersions {
    versions: Vec<u64>,
    slots: Vec<u32>,
}

impl ObservedVersions {
    /// An empty version vector.
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn new() -> Self {
        Self {
            versions: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Number of tracked inputs.
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether the vector tracks no inputs.
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }
}

impl Default for ObservedVersions {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-process cache over declared `[spatial]` reads — **geometry only**. The
/// cache records the plan, the cached transform, the coherent sample time, and
/// the exact observed input-version vector. Freshness/staleness/uncertainty are
/// always derived per query from timestamps; version equality never implies
/// `VALID` (`I-Q2`, REQ-SPATIAL-002).
/// REQ: REQ-SPATIAL-002.
#[derive(Debug)]
pub struct PlanCache<'p> {
    plan: &'p TransformPlan,
    cached: Transform3,
    coherent_sample: TimePoint,
    observed: ObservedVersions,
}

impl<'p> PlanCache<'p> {
    /// Creates an empty cache bound to a compiled plan.
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn new(plan: &'p TransformPlan) -> Self {
        Self {
            plan,
            cached: Transform3::IDENTITY,
            coherent_sample: TimePoint::new(
                0,
                crate::clock::ClockIdentity::new(crate::clock::ClockDomain::HostMonotonic),
            ),
            observed: ObservedVersions::new(),
        }
    }

    /// The compiled plan this cache serves.
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn plan(&self) -> &'p TransformPlan {
        self.plan
    }

    /// The coherent sample time of the cached composition.
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn coherent_sample(&self) -> TimePoint {
        self.coherent_sample
    }

    /// The cached transform geometry (valid only under a version match).
    /// REQ: REQ-SPATIAL-002.
    #[must_use]
    pub fn cached(&self) -> Transform3 {
        self.cached
    }

    /// Whether every input version still matches the cached vector (`I-Q2`).
    /// REQ: REQ-SPATIAL-002, REQ-SPATIAL-050.
    #[must_use]
    pub fn matches(&self, versions: &[u64]) -> bool {
        !self.observed.is_empty() && self.observed.versions == versions
    }

    /// Records a fresh geometry result and the versions observed at `sample`
    /// (`I-Q2`).
    /// REQ: REQ-SPATIAL-002.
    pub fn update(&mut self, transform: Transform3, sample: TimePoint, versions: Vec<u64>) {
        self.cached = transform;
        self.coherent_sample = sample;
        self.observed.versions = versions;
        self.observed.slots.clear();
    }
}

/// Reads the versions of the given edge slots; the exact vector a query records
/// after execution (`I-Q2`).
/// REQ: REQ-SPATIAL-002, REQ-SPATIAL-050.
#[must_use]
pub fn observe_versions(slots: &[&EdgeSlot]) -> Vec<u64> {
    slots.iter().map(|s| s.version()).collect()
}

#[cfg(test)]
mod tests {
    use super::{MAX_PLAN_DEPTH, PlanCache, PlanStep, TransformPlan, compile_plan, execute_plan};
    use crate::clock::{ClockDomain, ClockIdentity, TimePoint};
    use crate::forest::SpaceForest;
    use crate::types::{SpatialError, Transform3};

    fn chain_plan() -> &'static [PlanStep] {
        static STEPS: [PlanStep; 3] = [
            PlanStep::DynamicEdge(0),
            PlanStep::Constant(Transform3::IDENTITY),
            PlanStep::Multiply,
        ];
        &STEPS
    }

    /// REQ: REQ-SPATIAL-034 — the plan's stack effect is verified and a malformed
    /// plan is rejected at construction.
    #[test]
    fn plan_stack_effect() {
        static BAD: [PlanStep; 1] = [PlanStep::Multiply];
        let ok = TransformPlan::new(chain_plan()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(ok.max_stack(), 2);
        assert!(TransformPlan::new(&BAD).is_err());
    }

    /// REQ: REQ-SPATIAL-034 — executing a plan folds dynamic edges through the
    /// supplied resolver and never touches the base graph.
    #[test]
    fn execute_folds_edges() {
        let plan = TransformPlan::new(chain_plan()).unwrap_or_else(|e| unreachable!("{e}"));
        let want = Transform3::new([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0])
            .unwrap_or_else(|e| unreachable!("{e}"));
        let got = execute_plan(
            &plan,
            |_edge| Ok(want),
            |_slot| Err(SpatialError::ClockUnresolved),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(got, want);
    }

    /// REQ: REQ-SPATIAL-002 — a version-vector match reuses cached geometry and a
    /// change forces re-execution (`I-Q2`).
    #[test]
    fn cache_shortcircuit() {
        let plan = TransformPlan::new(chain_plan()).unwrap_or_else(|e| unreachable!("{e}"));
        let t = Transform3::new([5.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0])
            .unwrap_or_else(|e| unreachable!("{e}"));
        let sample = TimePoint::new(10, ClockIdentity::new(ClockDomain::HostMonotonic));
        let mut cache = PlanCache::new(&plan);
        assert!(!cache.matches(&[2]));
        cache.update(t, sample, vec![2]);
        assert!(cache.matches(&[2]));
        assert!(!cache.matches(&[3]));
        assert_eq!(cache.cached(), t);
        const { assert!(MAX_PLAN_DEPTH >= 2) };
    }

    /// REQ: REQ-SPATIAL-002 — an unresolved edge read propagates as a hard error.
    #[test]
    fn execute_propagates_error() {
        let plan = TransformPlan::new(chain_plan()).unwrap_or_else(|e| unreachable!("{e}"));
        let got = execute_plan(
            &plan,
            |_edge| Err(SpatialError::ClockUnresolved),
            |_slot| Err(SpatialError::ClockUnresolved),
        );
        assert_eq!(got, Err(SpatialError::ClockUnresolved));
    }

    /// REQ: REQ-SPATIAL-034 — `compile_plan` folds a forest path into constant
    /// and dynamic-edge steps plus multiply folds; traversal happens only at
    /// compile time (`I-Q1`).
    #[test]
    fn compile_plan_from_forest() {
        let mut f = SpaceForest::new();
        f.add_root("world".to_string());
        f.add_static(
            "base".to_string(),
            "world".to_string(),
            Transform3::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap_or(Transform3::IDENTITY),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        f.add_dynamic("odom".to_string(), "base".to_string(), 7);
        let plan = compile_plan(&f, "odom", "world").unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(plan.max_stack(), 2);
        assert_eq!(
            plan.steps(),
            &[
                PlanStep::DynamicEdge(7),
                PlanStep::Constant(
                    Transform3::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0])
                        .unwrap_or(Transform3::IDENTITY)
                ),
                PlanStep::Multiply,
            ]
        );
        // Executing resolves the dynamic edge through the supplied resolver.
        let dyn_t =
            Transform3::new([0.0, 2.0, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap_or(Transform3::IDENTITY);
        let composed = super::execute_steps(
            plan.steps(),
            |_edge| Ok(dyn_t),
            |_slot| Err(SpatialError::OutOfHistory),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        // odom→world = base_to_odom(identity) ∘ world_to_base translation.
        assert!((composed.translation[0] - 1.0).abs() < 1e-12);
        assert!((composed.translation[1] - 2.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-034 — a self-query compiles to the identity constant.
    #[test]
    fn compile_plan_self_is_identity() {
        let mut f = SpaceForest::new();
        f.add_root("world".to_string());
        let plan = compile_plan(&f, "world", "world").unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(plan.steps(), &[PlanStep::Constant(Transform3::IDENTITY)]);
    }
}
