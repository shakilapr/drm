//! `I-S1` type semantics: points rotate + translate, directions and vectors
//! rotate only. The distinct types make a naked-coordinate transform a
//! compile error, so this suite exercises the runtime behavior of each type.
//!
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-038.

use drm_spatial::{
    Direction3, Point3, Transform3, UnitQuat, Vector3, transform_direction, transform_point,
    transform_vector,
};

/// A pure translation must move points but leave directions/vectors untouched.
/// REQ: REQ-SPATIAL-036.
#[test]
#[allow(clippy::float_cmp)] // identity rotation + pure translation are exact
fn translation_moves_points_only() {
    let t = Transform3::new([10.0, -4.0, 2.0], UnitQuat::IDENTITY.into_components())
        .unwrap_or_default();

    let p = transform_point(&t, Point3::new(1.0, 1.0, 1.0).unwrap_or_default());
    assert!((p.x - 11.0).abs() < 1e-12);
    assert!((p.y + 3.0).abs() < 1e-12);
    assert!((p.z - 3.0).abs() < 1e-12);

    let d = transform_direction(&t, Direction3::X);
    assert_eq!(d.components(), [1.0, 0.0, 0.0]);
    let v = transform_vector(&t, Vector3::new(0.0, 2.0, 0.0).unwrap_or_default());
    assert_eq!(v.components(), [0.0, 2.0, 0.0]);
}

/// A direction stays unit after a rotation; a vector preserves its length.
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-038.
#[test]
fn rotation_preserves_norm() {
    let t = Transform3::new(
        [0.0, 0.0, 0.0],
        UnitQuat::from_axis_angle(
            Direction3::try_new(1.0, 1.0, 1.0).unwrap_or(Direction3::X),
            1.1,
        )
        .unwrap_or(UnitQuat::IDENTITY)
        .into_components(),
    )
    .unwrap_or_default();
    let d = transform_direction(
        &t,
        Direction3::try_new(1.0, -2.0, 0.5).unwrap_or(Direction3::X),
    );
    let norm: f64 = d.components().iter().map(|c| c * c).sum::<f64>().sqrt();
    assert!((norm - 1.0).abs() < 1e-12);

    let v = transform_vector(&t, Vector3::new(3.0, -1.0, 2.0).unwrap_or_default());
    let vn = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    let expected = (3.0_f64 * 3.0 + 1.0 + 4.0).sqrt();
    assert!((vn - expected).abs() < 1e-12);
}
