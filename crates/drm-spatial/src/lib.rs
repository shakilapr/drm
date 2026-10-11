//! DRM Spatial foundations ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)):
//! distinct typed quantities (`I-S1`), rigid `SE(3)` transform math, geodetic
//! boundary conversions (`I-S5`), canonical conventions with explicit boundary
//! conversions (`I-S4`), forests of spaces with disconnected-tree and bridge
//! semantics (`I-S3`), historical evaluation at a query's sample timestamp
//! (`I-S9`), affine clock-domain mapping (`I-S7`), the versioned
//! dynamic-edge cache groundwork, live calibration slots (`I-Q11`), generational
//! overlay handles (`I-S12`), and single-writer edge authority (`I-S11`).
//!
//! Layer `runtime` (codebase.md 2.1): depends on `drm-types` only. The crate is
//! `#![forbid(unsafe_code)]`; the lockless edge cache stores `f64` payloads as
//! fixed-width `AtomicU64` bit patterns, and single-writer authority uses a std
//! `File` advisory lock, so no unsafe cell is needed.
//!
//! REQ: REQ-SPATIAL-036 (distinct typed quantities), REQ-SPATIAL-038 (rigid
//! transforms only), REQ-SPATIAL-004 (clock domains), REQ-SPATIAL-012 (geodetic
//! chain), REQ-SPATIAL-007 (canonical conventions), REQ-SPATIAL-010 (forests),
//! REQ-SPATIAL-050 (lockless dynamic-edge slots), REQ-SPATIAL-003 (live
//! calibration slots), REQ-SPATIAL-011 (generational handles, single-writer
//! edges), REQ-SPATIAL-030 (single-writer per dynamic edge).

#![forbid(unsafe_code)]
// f64 math and wire-layout arithmetic use `as`/`from` conversions between the
// f64 and integer domains; precision loss is inherent to those conversions.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

pub mod cache;
pub mod clock;
pub mod conventions;
pub mod forest;
pub mod geodetic;
pub mod handle;
pub mod historical;
pub mod history;
pub mod plan;
pub mod query;
pub mod transform;
pub mod types;
pub mod writer;

pub use cache::{
    CachedSample, EdgeSample, EdgeSlot, FLAG_DISCONTINUOUS, SpatialEdgeCache, WriterClaim,
};
pub use clock::{
    ClockDomain, ClockIdentity, ClockMapping, ClockTable, GlobalSource, TimePoint, map_timestamp,
    map_timestamp_with_uncertainty,
};
pub use conventions::BoundaryConvention;
pub use forest::{FrameEdge, FrameId, PathEdge, SpaceForest, TreeTxn};
pub use geodetic::{
    Ecef, Enu, GEODETIC_LOCAL_BOUND_M, Lla, geodetic_to_enu, tangent_plane_enu, wgs84_to_ecef,
};
pub use handle::HandleRegistry;
pub use historical::{HistoricalForest, TopologyEvent};
pub use history::{
    HistorySample, Ingest, TimeHistory, interpolate_transform, stamp, transform_from_quat,
};
pub use plan::{
    CompiledPlan, FLAG_DISCONTINUITY_NEARBY, FLAG_EXTRAPOLATED, FLAG_STALE, MAX_PLAN_DEPTH,
    ObservedVersions, PlanCache, PlanStep, QualityFlags, TransformError, TransformPlan,
    TransformResult, compile_plan, execute_plan, execute_steps, observe_versions,
};
pub use query::{CalibrationSlot, QueryEngine, ReparentMode, correction_jump};
pub use transform::{
    compose, exp_twist, invert, transform_direction, transform_point, transform_vector,
};
pub use types::{
    Direction3, Point3, Pose3, SpatialError, SpatialHandle, SpatialId, Stamped, Transform3, Twist3,
    UnitQuat, Vector3,
};
pub use writer::{EdgeAuthority, acquire as acquire_edge_authority, authority_path};
