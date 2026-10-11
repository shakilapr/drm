//! `I-S2` compile-space payload: compile-time frame bindings carry zero frame
//! bytes on the wire — the canonical layout is raw numerics only. The wire
//! sizes are fixed `f64` counts and the Rust layout matches them exactly.
//!
//! REQ: REQ-SPATIAL-005, REQ-SPATIAL-009.

use drm_spatial::{
    Direction3, Point3, Pose3, SpatialHandle, SpatialId, Transform3, Twist3, Vector3,
};

/// Every spatial type's wire size equals its raw-numerics size (no frame
/// strings or ids). REQ: REQ-SPATIAL-005.
#[test]
fn wire_sizes_are_raw_numerics() {
    assert_eq!(Point3::WIRE_BYTES, 24);
    assert_eq!(Vector3::WIRE_BYTES, 24);
    assert_eq!(Direction3::WIRE_BYTES, 24);
    assert_eq!(Pose3::WIRE_BYTES, 56);
    assert_eq!(Transform3::WIRE_BYTES, 56);
    assert_eq!(Twist3::WIRE_BYTES, 48);

    assert_eq!(std::mem::size_of::<Point3>(), Point3::WIRE_BYTES);
    assert_eq!(std::mem::size_of::<Vector3>(), Vector3::WIRE_BYTES);
    assert_eq!(std::mem::size_of::<Transform3>(), Transform3::WIRE_BYTES);
    assert_eq!(std::mem::size_of::<Twist3>(), Twist3::WIRE_BYTES);
}

/// Dynamic identity and handles are fixed-width numeric, never frame strings.
/// REQ: REQ-SPATIAL-009.
#[test]
fn dynamic_identity_is_numeric() {
    assert_eq!(std::mem::size_of::<SpatialId>(), 4);
    assert_eq!(std::mem::size_of::<SpatialHandle>(), 8);
    let id = SpatialId(42);
    assert_eq!(id.0, 42);
    let handle = SpatialHandle {
        index: 3,
        generation: 7,
    };
    assert_eq!(handle.generation, 7);
}
