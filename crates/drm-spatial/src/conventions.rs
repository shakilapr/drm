//! Canonical conventions and explicit boundary conversions
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §The Five
//! Golden Rules rule 4, §Invariants `I-S4`;
//! [`spatial_architecture.md`](../../docs/spatial_architecture.md) §Canonical
//! Conventions and Geodetic Boundaries). Internally every value is canonical:
//! right-handed, SI units, quaternion rotations, standard robotics axes
//! `+X` forward / `+Y` left / `+Z` up (`REQ-SPATIAL-007`). Non-canonical
//! frameworks convert **only** at declared boundaries — never implicitly mixed
//! (`REQ-SPATIAL-040`).
//!
//! Boundary conversions are signed axis permutations (orthonormal, unit
//! determinant for right-handed sources; a handedness flip for the left-handed
//! sources). They are declared once per CRS/framework edge and applied to a
//! rigid [`Transform3`] by conjugating with the permutation (`R' = M R Mᵀ`).
//!
//! REQ: REQ-SPATIAL-007, REQ-SPATIAL-040, REQ-SPATIAL-038.

use crate::types::{Direction3, Point3, SpatialError, Transform3, Vector3};

/// A non-canonical external framework that requires an explicit axis conversion
/// at its boundary into the canonical convention (`I-S4`).
/// REQ: REQ-SPATIAL-040.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryConvention {
    /// North-East-Down: `x = North`, `y = East`, `z = Down` (right-handed).
    Ned,
    /// `OpenCV` optical frame: `x = right`, `y = down`, `z = forward` (right-handed).
    OpenCvOptical,
    /// Unity: `x = right`, `y = up`, `z = forward` (left-handed).
    Unity,
    /// Unreal: `x = forward`, `y = right`, `z = up` (left-handed).
    Unreal,
}

type Mat3 = [[f64; 3]; 3];

impl BoundaryConvention {
    /// The canonical source axes spelled in the external frame: each row maps an
    /// external coordinate onto its canonical contribution. Rows are the
    /// canonical basis vectors (`X` forward, `Y` left, `Z` up) expressed in the
    /// external frame.
    /// REQ: REQ-SPATIAL-040.
    #[must_use]
    pub const fn axes(self) -> [[f64; 3]; 3] {
        match self {
            // Canonical +X=fwd, +Y=left, +Z=up in NED (N=E=fwd, E=left(West), D=−up).
            Self::Ned => [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
            // OpenCV: x=right(−Y), y=down(−Z), z=fwd(+X). Canonical axes in external coords.
            Self::OpenCvOptical => [[0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]],
            // Unity (left-handed): x=right(−Y), y=up(+Z), z=fwd(+X).
            Self::Unity => [[0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            // Unreal (left-handed): x=fwd(+X), y=right(−Y), z=up(+Z).
            Self::Unreal => [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    /// The rotation `M(external → canonical)` as a `3 × 3` matrix: each row is a
    /// canonical basis vector (`X` forward, `Y` left, `Z` up) expressed in the
    /// external frame, so `M · v_external = v_canonical`.
    /// REQ: REQ-SPATIAL-040.
    #[must_use]
    pub fn matrix(self) -> Mat3 {
        self.axes()
    }

    /// Converts an external-frame point into the canonical frame.
    /// REQ: REQ-SPATIAL-040.
    #[must_use]
    pub fn point_to_canonical(self, p: Point3) -> Point3 {
        let v = apply(self.matrix(), [p.x, p.y, p.z]);
        Point3 {
            x: v[0],
            y: v[1],
            z: v[2],
        }
    }

    /// Converts a canonical-frame point into the external frame.
    /// REQ: REQ-SPATIAL-040.
    #[must_use]
    pub fn point_from_canonical(self, p: Point3) -> Point3 {
        let v = apply(transpose(self.matrix()), [p.x, p.y, p.z]);
        Point3 {
            x: v[0],
            y: v[1],
            z: v[2],
        }
    }

    /// Converts an external-frame direction into the canonical frame.
    /// REQ: REQ-SPATIAL-040.
    ///
    /// # Errors
    /// [`SpatialError::InvalidDirection`] for a zero/non-finite vector.
    pub fn direction_to_canonical(self, d: Direction3) -> Result<Direction3, SpatialError> {
        let v = apply(self.matrix(), d.components());
        Direction3::try_new(v[0], v[1], v[2])
    }

    /// Converts an external-frame vector into the canonical frame.
    /// REQ: REQ-SPATIAL-040.
    #[must_use]
    pub fn vector_to_canonical(self, v: Vector3) -> Vector3 {
        let r = apply(self.matrix(), v.components());
        Vector3 {
            x: r[0],
            y: r[1],
            z: r[2],
        }
    }

    /// Converts a rigid transform declared in the external frame into the
    /// canonical frame: `R' = M R Mᵀ`, `t' = M t` (`I-S4`).
    ///
    /// A left-handed source (`Unity`, `Unreal`) yields a canonical rotation only
    /// after the handedness flip; the translation maps directly.
    /// REQ: REQ-SPATIAL-040, REQ-SPATIAL-038.
    #[must_use]
    pub fn transform_to_canonical(self, t: &Transform3) -> Transform3 {
        let m = self.matrix();
        let mt = transpose(m);
        let translation = apply(m, t.translation);
        // Rotation matrix of the quaternion, conjugated by M: M R Mᵀ.
        let r = quat_to_mat(t.rotation);
        let r2 = mat_mul(m, mat_mul(r, mt));
        Transform3 {
            translation,
            rotation: mat_to_quat(r2),
        }
    }

    /// Converts a canonical rigid transform into the external frame:
    /// `R' = Mᵀ R M`, `t' = Mᵀ t`.
    /// REQ: REQ-SPATIAL-040, REQ-SPATIAL-038.
    #[must_use]
    pub fn transform_from_canonical(self, t: &Transform3) -> Transform3 {
        let m = self.matrix();
        let mt = transpose(m);
        let translation = apply(mt, t.translation);
        let r = quat_to_mat(t.rotation);
        let r2 = mat_mul(mt, mat_mul(r, m));
        Transform3 {
            translation,
            rotation: mat_to_quat(r2),
        }
    }
}

/// Applies a `3 × 3` matrix to a `[f64; 3]`.
/// REQ: REQ-SPATIAL-040.
fn apply(m: Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Transposes a `3 × 3` matrix.
/// REQ: REQ-SPATIAL-040.
fn transpose(m: Mat3) -> Mat3 {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

/// Multiplies two `3 × 3` matrices.
/// REQ: REQ-SPATIAL-040.
fn mat_mul(a: Mat3, b: Mat3) -> Mat3 {
    let mut out = [[0.0_f64; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// Rotation matrix of a unit quaternion `[X, Y, Z, W]`.
/// REQ: REQ-SPATIAL-038.
#[allow(clippy::many_single_char_names)]
fn quat_to_mat(q: [f64; 4]) -> Mat3 {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// Unit quaternion `[X, Y, Z, W]` from a rotation matrix (Shepperd's method,
/// numerically stable branch selection).
/// REQ: REQ-SPATIAL-038.
fn mat_to_quat(m: Mat3) -> [f64; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            0.25 * s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
            (m[1][0] - m[0][1]) / s,
        ]
    };
    normalize(q)
}

/// Normalizes a quaternion, defaulting to identity when degenerate.
/// REQ: REQ-SPATIAL-038.
fn normalize(q: [f64; 4]) -> [f64; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if !n.is_finite() || n <= f64::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

#[cfg(test)]
mod tests {
    use super::BoundaryConvention;
    use crate::types::{Direction3, Point3, Transform3};

    /// REQ: REQ-SPATIAL-040 — NED maps `+X` forward to `+X`, and canonical `+Y`
    /// left to `−Y` (East), `+Z` up to `−Z` (Down).
    #[test]
    fn ned_axis_mapping() {
        let p = BoundaryConvention::Ned.point_to_canonical(Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        });
        assert!((p.x - 1.0).abs() < 1e-12);
        assert!((p.y + 2.0).abs() < 1e-12);
        assert!((p.z + 3.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-040 — `OpenCV` optical `z` (forward) maps to canonical
    /// `+X` forward.
    #[test]
    fn opencv_forward() {
        let d = BoundaryConvention::OpenCvOptical
            .direction_to_canonical(Direction3::Z)
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert!((d.components()[0] - 1.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-040 — round-tripping a transform through a boundary is
    /// the identity within tolerance.
    #[test]
    fn roundtrip_identity() {
        let t = Transform3::new([1.0, 2.0, 3.0], [0.0, 0.0, 1.0, 0.0])
            .unwrap_or_else(|e| unreachable!("{e}"));
        for c in [
            BoundaryConvention::Ned,
            BoundaryConvention::OpenCvOptical,
            BoundaryConvention::Unity,
            BoundaryConvention::Unreal,
        ] {
            let back = c.transform_from_canonical(&c.transform_to_canonical(&t));
            for i in 0..3 {
                assert!((back.translation[i] - t.translation[i]).abs() < 1e-9);
            }
        }
    }
}
