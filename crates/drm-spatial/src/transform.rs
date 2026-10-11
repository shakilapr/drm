//! Rigid `SE(3)` transform application and composition
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
//! §Functions). Point transforms apply rotation + translation; direction and
//! vector transforms apply rotation only (`I-S1`). `compose` renormalizes the
//! quaternion to bound drift.
//!
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-049.

use crate::types::{Direction3, Point3, SpatialError, Transform3, Twist3, UnitQuat, Vector3};

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

/// The rigid inverse: `inverse(a) ∘ a = a ∘ inverse(a) = identity` (`I-S1`).
/// `R⁻¹ = Rᵀ` (conjugate quaternion), `t⁻¹ = −Rᵀt`.
///
/// REQ: REQ-SPATIAL-038.
#[must_use]
pub fn invert(a: &Transform3) -> Transform3 {
    let q = a.quat();
    let inv = UnitQuat {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        w: q.w,
    };
    let t = Vector3 {
        x: a.translation[0],
        y: a.translation[1],
        z: a.translation[2],
    };
    let rt = inv.rotate(t);
    Transform3 {
        translation: [-rt.x, -rt.y, -rt.z],
        rotation: inv.into_components(),
    }
}

/// The `SE(3)` exponential map applied to a body-frame twist over `dt` seconds
/// ([spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
/// §Functions `predict`, §Invariants `I-Q5`): `Exp(ξ · dt)` with `ξ = (v, ω)`.
/// The translation uses the left Jacobian `V(ω·dt)`; the rotation is the
/// Rodrigues rotation about `ω` by `|ω|·dt`. A zero twist yields the identity.
///
/// # Errors
/// [`SpatialError::NonFinite`] for a non-finite `dt` or twist component.
/// REQ: REQ-SPATIAL-025.
pub fn exp_twist(twist: &Twist3, dt: f64) -> Result<Transform3, SpatialError> {
    let v = twist.linear;
    let w = twist.angular;
    if !dt.is_finite()
        || !v.x.is_finite()
        || !v.y.is_finite()
        || !v.z.is_finite()
        || !w.x.is_finite()
        || !w.y.is_finite()
        || !w.z.is_finite()
    {
        return Err(SpatialError::NonFinite);
    }
    let omega = w.scale(dt);
    let u = v.scale(dt);
    let theta = omega.norm();
    if theta < 1e-12 {
        // Pure translation (or no motion): the Jacobian is the identity.
        return Ok(Transform3 {
            translation: [u.x, u.y, u.z],
            rotation: UnitQuat::IDENTITY.into_components(),
        });
    }
    let axis = Direction3::try_new(omega.x, omega.y, omega.z)?;
    let quat = UnitQuat::from_axis_angle(axis, theta)?;
    let (sin_t, cos_t) = theta.sin_cos();
    let c1 = (1.0 - cos_t) / (theta * theta);
    let c2 = (theta - sin_t) / (theta * theta * theta);
    let wxu = omega.cross(u);
    let wxwxu = omega.cross(wxu);
    let translation = u + wxu.scale(c1) + wxwxu.scale(c2);
    Ok(Transform3 {
        translation: [translation.x, translation.y, translation.z],
        rotation: quat.into_components(),
    })
}

#[cfg(test)]
mod tests {
    use crate::transform::{
        compose, exp_twist, invert, transform_direction, transform_point, transform_vector,
    };
    use crate::types::{Direction3, Point3, Transform3, Twist3, UnitQuat, Vector3};

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

    /// REQ: REQ-SPATIAL-038 — the rigid inverse composed with the original is
    /// the identity.
    #[test]
    fn invert_roundtrip() {
        let t = Transform3::new(
            [1.0, -2.0, 3.0],
            UnitQuat::from_axis_angle(Direction3::Y, 0.9)
                .unwrap_or(UnitQuat::IDENTITY)
                .into_components(),
        )
        .unwrap_or(Transform3::IDENTITY);
        let identity = compose(&invert(&t), &t);
        assert!(identity.translation[0].abs() < 1e-12);
        assert!(identity.translation[1].abs() < 1e-12);
        assert!(identity.translation[2].abs() < 1e-12);
        assert!(identity.rotation[3].abs() - 1.0 < 1e-12);
    }

    /// REQ: REQ-SPATIAL-025 — the exponential map of a pure translation twist is
    /// a translation; a pure rotation twist is the Rodrigues rotation.
    #[test]
    fn exp_twist_pure() {
        let translate = Twist3 {
            linear: Vector3::new(1.0, 0.0, 0.0).unwrap_or_default(),
            angular: Vector3::new(0.0, 0.0, 0.0).unwrap_or_default(),
        };
        let d = exp_twist(&translate, 2.0).unwrap_or_else(|e| unreachable!("{e}"));
        assert!((d.translation[0] - 2.0).abs() < 1e-12);
        assert!((d.quat().norm() - 1.0).abs() < 1e-12);

        // Rotation about +Z by π/2: +X maps to +Y.
        let rotate = Twist3 {
            linear: Vector3::new(0.0, 0.0, 0.0).unwrap_or_default(),
            angular: Vector3::new(0.0, 0.0, 1.0).unwrap_or_default(),
        };
        let r = exp_twist(&rotate, core::f64::consts::FRAC_PI_2)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let mapped = r
            .quat()
            .rotate(Vector3::new(1.0, 0.0, 0.0).unwrap_or_default());
        assert!((mapped.y - 1.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-025 — a zero twist over any interval is the identity.
    #[test]
    fn exp_twist_zero_is_identity() {
        let zero = Twist3 {
            linear: Vector3::new(0.0, 0.0, 0.0).unwrap_or_default(),
            angular: Vector3::new(0.0, 0.0, 0.0).unwrap_or_default(),
        };
        let d = exp_twist(&zero, 5.0).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(d, Transform3::IDENTITY);
    }

    /// REQ: REQ-SPATIAL-025 — a non-finite twist is rejected.
    #[test]
    fn exp_twist_rejects_non_finite() {
        let bad = Twist3 {
            linear: Vector3 {
                x: f64::NAN,
                y: 0.0,
                z: 0.0,
            },
            angular: Vector3::new(0.0, 0.0, 0.0).unwrap_or_default(),
        };
        assert_eq!(
            exp_twist(&bad, 1.0),
            Err(crate::types::SpatialError::NonFinite)
        );
    }
}
