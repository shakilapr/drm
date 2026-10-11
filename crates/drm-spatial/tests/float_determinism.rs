//! Reference-suite float determinism (work-plan M21W4-E1-T3; spatial-core.md
//! §Reference-Suite Float Determinism, `REQ-SPATIAL-053`). A fused
//! multiply-add (`f64::mul_add`, one rounding) and a separate multiply-then-add
//! (two roundings) may be selected differently by a host toolchain and an MCU
//! toolchain for the same source expression. The spatial reference suite
//! compares with one combined absolute + relative band, never bit equality, so
//! every such rounding-contraction difference must fall inside the band while a
//! genuine geometry error is still rejected.
//!
//! `cargo test -p drm-spatial float_determinism`.
//!
//! REQ: REQ-SPATIAL-053.

// Quaternion components are conventionally `x/y/z/w`; the short names mirror the
// scalar math and the emitted artifact.
#![allow(clippy::many_single_char_names)]

use drm_spatial::{
    Direction3, Point3, ToleranceBand, Transform3, UnitQuat, Vector3, compose, transform_point,
    transform_vector, within_reference_band,
};

/// Rotates `v` by the unit quaternion `q` using separate multiply-then-add (two
/// roundings per fused term), mirroring `UnitQuat::rotate`.
/// REQ: REQ-SPATIAL-053.
fn rotate_separate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let tx = 2.0 * (y * v[2] - z * v[1]);
    let ty = 2.0 * (z * v[0] - x * v[2]);
    let tz = 2.0 * (x * v[1] - y * v[0]);
    [
        v[0] + w * tx + (y * tz - z * ty),
        v[1] + w * ty + (z * tx - x * tz),
        v[2] + w * tz + (x * ty - y * tx),
    ]
}

/// Rotates `v` by the unit quaternion `q` using a fused multiply-add (one
/// rounding per fused term), mirroring a toolchain that contracts `a*b + c`.
/// REQ: REQ-SPATIAL-053.
fn rotate_fused(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let tx = 2.0 * y.mul_add(v[2], -(z * v[1]));
    let ty = 2.0 * z.mul_add(v[0], -(x * v[2]));
    let tz = 2.0 * x.mul_add(v[1], -(y * v[0]));
    [
        w.mul_add(tx, v[0] + y.mul_add(tz, -(z * ty))),
        w.mul_add(ty, v[1] + z.mul_add(tx, -(x * tz))),
        w.mul_add(tz, v[2] + x.mul_add(ty, -(y * tx))),
    ]
}

/// Quaternion product `a ∘ b` using separate multiply-then-add.
/// REQ: REQ-SPATIAL-053.
fn compose_q_separate(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// Quaternion product `a ∘ b` using a fused multiply-add.
/// REQ: REQ-SPATIAL-053.
fn compose_q_fused(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw.mul_add(bx, ax.mul_add(bw, ay.mul_add(bz, -(az * by)))),
        aw.mul_add(by, -(ax * bz) + ay.mul_add(bw, az * bx)),
        aw.mul_add(bz, ax.mul_add(by, -(ay * bx) + az * bw)),
        aw.mul_add(bw, -(ax.mul_add(bx, ay.mul_add(by, az * bz)))),
    ]
}

/// A test quaternion from a Z axis-angle rotation, or identity on error.
/// REQ: REQ-SPATIAL-053.
fn quat_z(angle: f64) -> [f64; 4] {
    UnitQuat::from_axis_angle(Direction3::Z, angle)
        .unwrap_or(UnitQuat::IDENTITY)
        .into_components()
}

/// REQ: REQ-SPATIAL-053 — for a spread of rotations and magnitudes (near zero,
/// order one, and large), the fused and separate point-rotation paths agree
/// within the documented reference band, and the runtime `rotate` path itself
/// stays inside the band of both.
#[test]
fn spatial_float_determinism() {
    let band = ToleranceBand::SPATIAL_REFERENCE;
    let magnitudes = [1e-6_f64, 1.0, 1.0e6, 1.0e9];
    let mut worst = 0.0_f64;
    for k in 0..24 {
        let angle = f64::from(k) * 0.261_799;
        let q = quat_z(angle);
        for m in magnitudes {
            let v = [m, -0.7 * m, 0.3 * m];
            let sep = rotate_separate(q, v);
            let fused = rotate_fused(q, v);
            for i in 0..3 {
                assert!(
                    band.accepts(sep[i], fused[i]),
                    "fused vs separate diverge beyond band at k={k} m={m} axis={i}"
                );
                let bound = band.error_bound(sep[i]);
                worst = worst.max((fused[i] - sep[i]).abs() / bound.max(f64::MIN_POSITIVE));
            }
            // The runtime crate path (separate arithmetic) matches both.
            let runtime = transform_vector(
                &Transform3::new([0.0, 0.0, 0.0], q).unwrap_or(Transform3::IDENTITY),
                Vector3 {
                    x: v[0],
                    y: v[1],
                    z: v[2],
                },
            );
            let runtime = [runtime.x, runtime.y, runtime.z];
            for i in 0..3 {
                assert!(within_reference_band(sep[i], runtime[i]));
                assert!(within_reference_band(fused[i], runtime[i]));
            }
        }
    }
    // The measured spread consumes a small fraction of the band (documented
    // margin for the final-gate evidence rule).
    assert!(worst < 0.5, "FMA spread {worst} of the band is too close");
}

/// REQ: REQ-SPATIAL-053 — quaternion composition: the fused and separate
/// Hamilton products agree within the band across the same input spread, so a
/// toolchain choosing either contraction cannot flip a reference comparison.
#[test]
fn float_determinism_compose_within_band() {
    let band = ToleranceBand::SPATIAL_REFERENCE;
    for i in 0..16 {
        let a = quat_z(f64::from(i) * 0.39);
        for j in 0..16 {
            let b = quat_z(f64::from(j) * -0.21);
            let sep = compose_q_separate(a, b);
            let fused = compose_q_fused(a, b);
            for k in 0..4 {
                assert!(
                    band.accepts(sep[k], fused[k]),
                    "compose diverges beyond band at {i}/{j} component {k}"
                );
            }
            // The runtime `compose` renormalizes; compare the rotated point of a
            // composed transform against the pure-quaternion contract band.
            let ta = Transform3::new([0.0, 0.0, 0.0], a).unwrap_or(Transform3::IDENTITY);
            let tb = Transform3::new([0.0, 0.0, 0.0], b).unwrap_or(Transform3::IDENTITY);
            let composed = compose(&ta, &tb);
            let p = transform_point(&composed, Point3::new(1.0, 2.0, 3.0).unwrap_or_default());
            let reference = rotate_separate(sep, [1.0, 2.0, 3.0]);
            assert!(within_reference_band(reference[0], p.x));
            assert!(within_reference_band(reference[1], p.y));
            assert!(within_reference_band(reference[2], p.z));
        }
    }
}

/// REQ: REQ-SPATIAL-053 — the band is a real gate, not a rubber stamp: a
/// perturbation larger than the bound is rejected, so the determinism tolerance
/// cannot mask a geometry regression.
#[test]
fn float_determinism_band_rejects_regression() {
    let band = ToleranceBand::SPATIAL_REFERENCE;
    let reference = 1.0_f64;
    let just_outside = reference + band.error_bound(reference) * 1.001;
    assert!(!band.accepts(reference, just_outside));
    assert!(band.accepts(reference, just_outside - band.error_bound(reference) * 0.5));
    // Near zero the absolute floor governs; a milli-metre error is still a
    // regression while a sub-picometre roundoff is not.
    assert!(!band.accepts(0.0, 1e-3));
    assert!(band.accepts(0.0, 1e-13));
}
