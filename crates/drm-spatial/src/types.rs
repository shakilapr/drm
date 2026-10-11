//! Distinct spatial semantic types and the spatial error taxonomy
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §Data
//! Structures, §Edge Cases & Errors). Distinct types make naked `f64` triplets
//! impossible across a transform (`I-S1`); rotations are unit quaternions in
//! `X, Y, Z, W` order and scale/shear are excluded (`I-S2`/`I-S8`).
//!
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-009, REQ-SPATIAL-035.

use core::fmt;

/// Spatial error taxonomy ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
/// §Edge Cases & Errors). Generic transport failures keep using
/// [`drm_types::Status`]; this enum names the spatial-only outcomes.
///
/// REQ: REQ-SPATIAL-004, REQ-SPATIAL-010, REQ-SPATIAL-035.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialError {
    /// A quaternion had zero or non-finite norm (`I-S1`).
    NonUnitQuaternion,
    /// A direction/axis vector had zero or non-finite length.
    InvalidDirection,
    /// A coordinate carried a non-finite value.
    NonFinite,
    /// No valid clock mapping exists for the requested identity (`I-S7`).
    ClockUnresolved,
    /// The target frame is not connected to the query frame (`I-S3`).
    PathNotFound,
    /// The requested timestamp predates the retained history.
    OutOfHistory,
    /// Interpolating across a flagged discontinuity boundary was refused; the
    /// query is near a jump and carries no cross-jump value (`I-Q7`).
    DiscontinuityNearby,
    /// A runtime overlay handle generation no longer matches (`I-S12`).
    InvalidHandle,
    /// A declared history bound is unsatisfiable (`N < floor(H/P) + 2`) or a
    /// calibration/topology declaration is malformed (`I-Q13`).
    InvalidConfig,
    /// A second node attempted to claim an already-owned dynamic edge (`I-S11`).
    WriterConflict,
    /// An argument was non-finite, out of range, or otherwise invalid
    /// (`I-X14`).
    InvalidArg,
    /// An insert would exceed the index's declared fixed capacity (`I-X2`);
    /// the index never grows.
    IndexFull,
}

impl SpatialError {
    /// Maps the spatial outcome onto the shared [`drm_types::Status`] taxonomy so
    /// FFI and transport callers keep one error vocabulary (codebase.md 3.1 R4).
    /// REQ: REQ-BIND-007, REQ-SPATIAL-004.
    #[must_use]
    pub const fn status(self) -> drm_types::Status {
        match self {
            Self::NonUnitQuaternion | Self::DiscontinuityNearby => drm_types::Status::InvalidData,
            Self::InvalidDirection | Self::NonFinite | Self::InvalidArg => {
                drm_types::Status::InvalidArg
            }
            Self::ClockUnresolved | Self::InvalidConfig => drm_types::Status::InvalidConfig,
            Self::PathNotFound => drm_types::Status::NotFound,
            Self::OutOfHistory => drm_types::Status::NoSample,
            Self::InvalidHandle => drm_types::Status::Stale,
            Self::WriterConflict => drm_types::Status::Busy,
            Self::IndexFull => drm_types::Status::QueueFull,
        }
    }
}

impl fmt::Display for SpatialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::NonUnitQuaternion => "non-unit quaternion",
            Self::InvalidDirection => "invalid direction",
            Self::NonFinite => "non-finite coordinate",
            Self::ClockUnresolved => "clock unresolved",
            Self::PathNotFound => "path not found",
            Self::OutOfHistory => "out of history",
            Self::DiscontinuityNearby => "discontinuity nearby",
            Self::InvalidHandle => "invalid handle",
            Self::InvalidConfig => "invalid configuration",
            Self::WriterConflict => "writer conflict",
            Self::InvalidArg => "invalid argument",
            Self::IndexFull => "index full",
        };
        f.write_str(s)
    }
}

impl std::error::Error for SpatialError {}

/// A displacement vector (`Vector3`); transforms apply rotation only (`I-S1`).
///
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector3 {
    /// `X` component, meters.
    pub x: f64,
    /// `Y` component, meters.
    pub y: f64,
    /// `Z` component, meters.
    pub z: f64,
}

impl Vector3 {
    /// Canonical wire size in bytes (`3` `f64`; rotation-only, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    pub const WIRE_BYTES: usize = 3 * 8;

    /// Constructs a vector; rejects non-finite components.
    ///
    /// # Errors
    /// [`SpatialError::NonFinite`] for a `NaN`/infinite component.
    /// REQ: REQ-SPATIAL-036.
    pub fn new(x: f64, y: f64, z: f64) -> Result<Self, SpatialError> {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            Ok(Self { x, y, z })
        } else {
            Err(SpatialError::NonFinite)
        }
    }

    /// Uniform scale.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn scale(self, s: f64) -> Self {
        Self {
            x: self.x * s,
            y: self.y * s,
            z: self.z * s,
        }
    }

    /// Dot product.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Cross product.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// Euclidean length.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// Components as `[X, Y, Z]` (wire order).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn components(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

impl core::ops::Add for Vector3 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}

impl core::ops::Sub for Vector3 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }
}

/// A location in a space; transforms apply rotation **and** translation (`I-S1`).
///
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point3 {
    /// `X` component, meters.
    pub x: f64,
    /// `Y` component, meters.
    pub y: f64,
    /// `Z` component, meters.
    pub z: f64,
}

impl Point3 {
    /// Canonical wire size in bytes (`3` `f64`, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    pub const WIRE_BYTES: usize = 3 * 8;

    /// Constructs a point; rejects non-finite components.
    ///
    /// # Errors
    /// [`SpatialError::NonFinite`] for a `NaN`/infinite component.
    /// REQ: REQ-SPATIAL-036.
    pub fn new(x: f64, y: f64, z: f64) -> Result<Self, SpatialError> {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            Ok(Self { x, y, z })
        } else {
            Err(SpatialError::NonFinite)
        }
    }

    /// Position as a displacement vector.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn as_vector(self) -> Vector3 {
        Vector3 {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    /// Components as `[X, Y, Z]` (wire order).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn components(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

/// A unit vector representing orientation; transforms apply rotation only
/// (`I-S1`). Construction normalizes, so the unit invariant always holds.
///
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Direction3 {
    x: f64,
    y: f64,
    z: f64,
}

impl Direction3 {
    /// Canonical wire size in bytes (`3` `f64`, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    pub const WIRE_BYTES: usize = 3 * 8;

    /// The `+X` canonical forward direction.
    /// REQ: REQ-SPATIAL-007.
    pub const X: Self = Self {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    /// The `+Y` canonical left direction.
    /// REQ: REQ-SPATIAL-007.
    pub const Y: Self = Self {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    /// The `+Z` canonical up direction.
    /// REQ: REQ-SPATIAL-007.
    pub const Z: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    /// Normalizes `(x, y, z)` into a unit direction.
    ///
    /// # Errors
    /// [`SpatialError::InvalidDirection`] for a zero or non-finite vector.
    /// REQ: REQ-SPATIAL-036.
    pub fn try_new(x: f64, y: f64, z: f64) -> Result<Self, SpatialError> {
        let n = (x * x + y * y + z * z).sqrt();
        if !n.is_finite() || n <= f64::EPSILON {
            return Err(SpatialError::InvalidDirection);
        }
        Ok(Self {
            x: x / n,
            y: y / n,
            z: z / n,
        })
    }

    /// Components as `[X, Y, Z]`.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn components(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// The underlying displacement vector (already unit length).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn as_vector(self) -> Vector3 {
        Vector3 {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }
}

/// A unit quaternion rotation in `X, Y, Z, W` order ([spatial-core.md]
/// §Data Structures). Every constructor normalizes; `compose` renormalizes to
/// bound drift (`I-S1`).
///
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-038.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitQuat {
    /// `X` component.
    pub x: f64,
    /// `Y` component.
    pub y: f64,
    /// `Z` component.
    pub z: f64,
    /// `W` (scalar) component.
    pub w: f64,
}

impl UnitQuat {
    /// The identity rotation.
    /// REQ: REQ-SPATIAL-036.
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// Constructs a unit quaternion, normalizing `(x, y, z, w)`.
    ///
    /// # Errors
    /// [`SpatialError::NonUnitQuaternion`] for a zero or non-finite quaternion.
    /// REQ: REQ-SPATIAL-036.
    #[allow(clippy::many_single_char_names)]
    pub fn try_new(x: f64, y: f64, z: f64, w: f64) -> Result<Self, SpatialError> {
        let n = (x * x + y * y + z * z + w * w).sqrt();
        if !n.is_finite() || n <= f64::EPSILON {
            return Err(SpatialError::NonUnitQuaternion);
        }
        Ok(Self {
            x: x / n,
            y: y / n,
            z: z / n,
            w: w / n,
        })
    }

    /// Rotation about a unit `axis` by `angle` radians.
    ///
    /// # Errors
    /// [`SpatialError::InvalidDirection`] for a zero/non-finite axis, or
    /// [`SpatialError::NonFinite`] for a non-finite angle.
    /// REQ: REQ-SPATIAL-036.
    pub fn from_axis_angle(axis: Direction3, angle: f64) -> Result<Self, SpatialError> {
        if !angle.is_finite() {
            return Err(SpatialError::NonFinite);
        }
        let half = angle * 0.5;
        let s = half.sin();
        Self::try_new(axis.x * s, axis.y * s, axis.z * s, half.cos())
    }

    /// Euclidean norm.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn norm(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt()
    }

    /// Components as `[X, Y, Z, W]` (wire order).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn into_components(self) -> [f64; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// Renormalizes, returning the identity for a degenerate input (compose
    /// drift recovery, [spatial-core.md] §Edge Cases).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn normalized(self) -> Self {
        let n = self.norm();
        if !n.is_finite() || n <= f64::EPSILON {
            return Self::IDENTITY;
        }
        Self {
            x: self.x / n,
            y: self.y / n,
            z: self.z / n,
            w: self.w / n,
        }
    }

    /// Rotates a displacement vector (`I-S1`: rotation only).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn rotate(self, v: Vector3) -> Vector3 {
        let qv = Vector3 {
            x: self.x,
            y: self.y,
            z: self.z,
        };
        let t = qv.cross(v).scale(2.0);
        v + t.scale(self.w) + qv.cross(t)
    }

    /// Hamilton product, renormalized (`I-S1`).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn compose(self, other: Self) -> Self {
        let a = self;
        let b = other;
        Self::try_new(
            a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        )
        .unwrap_or(Self::IDENTITY)
    }

    /// Spherical linear interpolation between `self` and `other` at `t ∈ [0, 1]`,
    /// taking the shorter arc (handles near-`180°` dot products; oracle-covered).
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn slerp(self, other: Self, t: f64) -> Self {
        let mut d = self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w;
        let mut end = other;
        if d < 0.0 {
            d = -d;
            end = Self {
                x: -other.x,
                y: -other.y,
                z: -other.z,
                w: -other.w,
            };
        }
        if d > 0.999_5 {
            // Nearly parallel: linear blend avoids a 0/0 in the trig branch.
            return Self {
                x: self.x + (end.x - self.x) * t,
                y: self.y + (end.y - self.y) * t,
                z: self.z + (end.z - self.z) * t,
                w: self.w + (end.w - self.w) * t,
            }
            .normalized();
        }
        let theta0 = d.clamp(-1.0, 1.0).acos();
        let theta = theta0 * t;
        let sin_theta0 = theta0.sin();
        let s0 = ((1.0 - t) * theta0).sin() / sin_theta0;
        let s1 = theta.sin() / sin_theta0;
        Self::try_new(
            self.x * s0 + end.x * s1,
            self.y * s0 + end.y * s1,
            self.z * s0 + end.z * s1,
            self.w * s0 + end.w * s1,
        )
        .unwrap_or(self)
    }
}

impl Default for UnitQuat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A combined position and orientation (`Point3` + `UnitQuat`).
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose3 {
    /// Position.
    pub position: Point3,
    /// Orientation.
    pub rotation: UnitQuat,
}

impl Pose3 {
    /// Canonical wire size in bytes (`Point3` + quaternion = `7` `f64`, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    pub const WIRE_BYTES: usize = 7 * 8;
}

/// An `SE(3)` rigid transform: translation plus unit-quaternion rotation
/// ([spatial-core.md] §Data Structures). Wire layout is `[f64; 3]` translation
/// then `[f64; 4]` quaternion (`X, Y, Z, W`), 56 bytes, raw numerics only
/// (`I-S2`).
///
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-005.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform3 {
    /// Translation `[X, Y, Z]`, meters.
    pub translation: [f64; 3],
    /// Unit quaternion `[X, Y, Z, W]`.
    pub rotation: [f64; 4],
}

impl Transform3 {
    /// Canonical wire size in bytes (`3 + 4` `f64`; no frame bytes, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    pub const WIRE_BYTES: usize = 7 * 8;

    /// The identity transform.
    /// REQ: REQ-SPATIAL-038.
    pub const IDENTITY: Self = Self {
        translation: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };

    /// Constructs a transform, normalizing and validating the quaternion.
    ///
    /// # Errors
    /// [`SpatialError::NonUnitQuaternion`] for a zero/non-finite quaternion or
    /// [`SpatialError::NonFinite`] for a non-finite translation.
    /// REQ: REQ-SPATIAL-038.
    pub fn new(translation: [f64; 3], rotation: [f64; 4]) -> Result<Self, SpatialError> {
        if !translation.iter().all(|c| c.is_finite()) {
            return Err(SpatialError::NonFinite);
        }
        let q = UnitQuat::try_new(rotation[0], rotation[1], rotation[2], rotation[3])?;
        Ok(Self {
            translation,
            rotation: [q.x, q.y, q.z, q.w],
        })
    }

    /// The stored rotation as a [`UnitQuat`].
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn quat(&self) -> UnitQuat {
        UnitQuat {
            x: self.rotation[0],
            y: self.rotation[1],
            z: self.rotation[2],
            w: self.rotation[3],
        }
    }
}

impl Default for Transform3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Linear and angular velocity.
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Twist3 {
    /// Linear velocity, meters/second.
    pub linear: Vector3,
    /// Angular velocity, radians/second.
    pub angular: Vector3,
}

impl Twist3 {
    /// Canonical wire size in bytes (`6` `f64`, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    pub const WIRE_BYTES: usize = 6 * 8;
}

/// A persistent object identity for heterogeneous spatial streams
/// (`SpatialPoint3`); never a slot index ([spatial-core.md] §Data Structures).
/// REQ: REQ-SPATIAL-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpatialId(pub u32);

/// A generational runtime-overlay frame handle (`I-S12`); a delete bumps the
/// generation so outstanding handles invalidate immediately.
/// REQ: REQ-SPATIAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpatialHandle {
    /// Slot index.
    pub index: u32,
    /// Generation counter.
    pub generation: u32,
}

/// A timed measurement: geometry plus sample and receive time (`I-S6`).
/// REQ: REQ-SPATIAL-035.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped<T> {
    /// The measured value.
    pub value: T,
    /// The instant the physical pose was true.
    pub sample: crate::clock::TimePoint,
    /// The instant DRM received the sample locally.
    pub receive: crate::clock::TimePoint,
}

#[cfg(test)]
mod tests {
    use super::{Direction3, Point3, SpatialError, Transform3, UnitQuat, Vector3};

    /// REQ: REQ-SPATIAL-036 — quaternion rotation of `+X` by `90°` about `+Z`
    /// yields `+Y` (right-handed convention).
    #[test]
    fn quat_rotate_quarter_turn() {
        let q = UnitQuat::from_axis_angle(Direction3::Z, core::f64::consts::FRAC_PI_2)
            .unwrap_or_else(|e| unreachable!("axis: {e}"));
        let v = q.rotate(Vector3::new(1.0, 0.0, 0.0).unwrap_or_default());
        assert!((v.x).abs() < 1e-12);
        assert!((v.y - 1.0).abs() < 1e-12);
        assert!((v.z).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-036 — normalization makes the unit invariant hold and
    /// rejects a degenerate quaternion.
    #[test]
    fn quat_normalization() {
        let q = UnitQuat::try_new(0.0, 0.0, 0.0, 2.0).unwrap_or_else(|e| unreachable!("{e}"));
        assert!((q.norm() - 1.0).abs() < 1e-15);
        assert_eq!(
            UnitQuat::try_new(0.0, 0.0, 0.0, 0.0),
            Err(SpatialError::NonUnitQuaternion)
        );
    }

    /// REQ: REQ-SPATIAL-038 — `Transform3::new` normalizes and rejects a
    /// non-finite translation.
    #[test]
    fn transform_validation() {
        let t = Transform3::new([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 5.0])
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert!((t.rotation[3] - 1.0).abs() < 1e-15);
        assert_eq!(
            Transform3::new([f64::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
            Err(SpatialError::NonFinite)
        );
    }

    /// REQ: REQ-SPATIAL-036 — `Point3`/`Vector3` reject non-finite values.
    #[test]
    fn point_rejects_non_finite() {
        assert_eq!(
            Point3::new(f64::INFINITY, 0.0, 0.0),
            Err(SpatialError::NonFinite)
        );
        assert_eq!(
            Vector3::new(0.0, f64::NAN, 0.0),
            Err(SpatialError::NonFinite)
        );
    }
}
