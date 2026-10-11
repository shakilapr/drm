//! Rigid `SE(3)` transform application and composition
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
//! §Functions). Point transforms apply rotation + translation; direction and
//! vector transforms apply rotation only (`I-S1`). `compose` renormalizes the
//! quaternion to bound drift.
//!
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-049.

use crate::types::{Direction3, Point3, Transform3, Vector3};

/// Transforms a point: rotation then translation (`I-S1`).
///
/// REQ: REQ-SPATIAL-036.
#[must_use]
pub fn transform_point(t: &Transform3, p: Point3) -> Point3 {
    let r = t.quat().rotate(p.as_vector());
    Point3 {
        x: r.x + t.translation[0],
        y: r.y + t.translation[1],
        z: r.z + t.translation[2],
    }
}

/// Transforms a direction: rotation only; translation is ignored (`I-S1`).
///
/// REQ: REQ-SPATIAL-036.
#[must_use]
pub fn transform_direction(t: &Transform3, d: Direction3) -> Direction3 {
    let r = t.quat().rotate(d.as_vector());
    Direction3::try_new(r.x, r.y, r.z).unwrap_or(d)
}

/// Transforms a displacement vector: rotation only (`I-S1`).
///
/// REQ: REQ-SPATIAL-036.
#[must_use]
pub fn transform_vector(t: &Transform3, v: Vector3) -> Vector3 {
    t.quat().rotate(v)
}

/// Composes `a` then `b` (i.e. `a ∘ b`, the transform that first applies `b`).
/// Translation adds `a`'s rotation applied to `b`'s translation; the composed
/// quaternion is renormalized (`I-S1`).
///
/// REQ: REQ-SPATIAL-038, REQ-SPATIAL-049.
#[must_use]
pub fn compose(a: &Transform3, b: &Transform3) -> Transform3 {
    let ra = a.quat();
    let rb = b.quat();
    let bt = Vector3 {
        x: b.translation[0],
        y: b.translation[1],
        z: b.translation[2],
    };
    let rotated = ra.rotate(bt);
    Transform3 {
        translation: [
            a.translation[0] + rotated.x,
            a.translation[1] + rotated.y,
            a.translation[2] + rotated.z,
        ],
        rotation: {
            let q = ra.compose(rb);
            [q.x, q.y, q.z, q.w]
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::transform::{compose, transform_direction, transform_point, transform_vector};
    use crate::types::{Direction3, Point3, Transform3, UnitQuat, Vector3};

    /// REQ: REQ-SPATIAL-036 — a point transform applies rotation then
    /// translation; a direction/vector transform ignores translation.
    #[test]
    fn point_vs_direction_semantics() {
        let t = Transform3::new(
            [1.0, 2.0, 3.0],
            UnitQuat::from_axis_angle(Direction3::Z, core::f64::consts::FRAC_PI_2)
                .unwrap_or(UnitQuat::IDENTITY)
                .into_components(),
        )
        .unwrap_or(Transform3::IDENTITY);
        let p = transform_point(&t, Point3::new(1.0, 0.0, 0.0).unwrap_or_default());
        assert!((p.x - 1.0).abs() < 1e-12);
        assert!((p.y - 3.0).abs() < 1e-12);
        assert!((p.z - 3.0).abs() < 1e-12);

        let d = transform_direction(&t, Direction3::X);
        assert!((d.components()[1] - 1.0).abs() < 1e-12);

        let v = transform_vector(&t, Vector3::new(1.0, 0.0, 0.0).unwrap_or_default());
        assert!((v.y - 1.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-038 — composing with the identity is a no-op and the
    /// result stays a unit quaternion.
    #[test]
    fn compose_identity_and_renormalize() {
        let t = Transform3::new(
            [1.0, -2.0, 0.5],
            UnitQuat::from_axis_angle(Direction3::Y, 0.7)
                .unwrap_or(UnitQuat::IDENTITY)
                .into_components(),
        )
        .unwrap_or(Transform3::IDENTITY);
        let c = compose(&Transform3::IDENTITY, &t);
        assert!((c.translation[0] - 1.0).abs() < 1e-12);
        assert!((c.translation[1] + 2.0).abs() < 1e-12);
        assert!((c.quat().norm() - 1.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-049 — composing a chain then transforming a point equals
    /// applying the transforms one at a time (associativity).
    #[test]
    fn compose_associative_chain() {
        let a = Transform3::new([0.5, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap_or_default();
        let b = Transform3::new(
            [0.0, 0.0, 1.0],
            UnitQuat::from_axis_angle(Direction3::X, 0.3)
                .unwrap_or(UnitQuat::IDENTITY)
                .into_components(),
        )
        .unwrap_or(Transform3::IDENTITY);
        let p = Point3::new(0.2, 0.4, 0.6).unwrap_or_default();
        let stepwise = transform_point(&a, transform_point(&b, p));
        let folded = transform_point(&compose(&a, &b), p);
        assert!((stepwise.x - folded.x).abs() < 1e-12);
        assert!((stepwise.y - folded.y).abs() < 1e-12);
        assert!((stepwise.z - folded.z).abs() < 1e-12);
    }
}
