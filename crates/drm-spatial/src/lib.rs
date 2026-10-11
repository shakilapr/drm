//! DRM Spatial foundations ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)):
//! distinct typed quantities (`I-S1`), rigid `SE(3)` transform math, geodetic
//! boundary conversions (`I-S5`), affine clock-domain mapping (`I-S7`), and the
//! versioned dynamic-edge cache groundwork.
//!
//! Layer `runtime` (codebase.md 2.1): depends on `drm-types` only. The crate is
//! `#![forbid(unsafe_code)]`; the lockless edge cache stores `f64` payloads as
//! fixed-width `AtomicU64` bit patterns, so no unsafe cell is needed.
//!
//! REQ: REQ-SPATIAL-036 (distinct typed quantities), REQ-SPATIAL-038 (rigid
//! transforms only), REQ-SPATIAL-004 (clock domains), REQ-SPATIAL-012 (geodetic
//! chain), REQ-SPATIAL-050 (lockless dynamic-edge slots).

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
pub mod geodetic;
pub mod plan;
pub mod transform;
pub mod types;

pub use cache::{
    CachedSample, EdgeSample, EdgeSlot, FLAG_DISCONTINUOUS, SpatialEdgeCache, WriterClaim,
};
pub use clock::{
    ClockDomain, ClockIdentity, ClockMapping, ClockTable, GlobalSource, TimePoint, map_timestamp,
    map_timestamp_with_uncertainty,
};
pub use geodetic::{Ecef, Enu, Lla, geodetic_to_enu, tangent_plane_enu, wgs84_to_ecef};
pub use plan::{
    FLAG_DISCONTINUITY_NEARBY, FLAG_EXTRAPOLATED, FLAG_STALE, MAX_PLAN_DEPTH, ObservedVersions,
    PlanCache, PlanStep, QualityFlags, TransformError, TransformPlan, TransformResult,
    execute_plan, observe_versions,
};
pub use transform::{compose, transform_direction, transform_point, transform_vector};
pub use types::{
    Direction3, Point3, Pose3, SpatialError, SpatialHandle, SpatialId, Stamped, Transform3, Twist3,
    UnitQuat, Vector3,
};
