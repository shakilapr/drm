//! MCU spatial subset vs mathematical references (work-plan M21W3-E1-T1/E2-T1,
//! spatial-core.md §Functions `emit_mcu_constants`, §Invariants I-S13). Builds a
//! static chain, folds it with the workspace compiler, emits the freestanding C
//! artifact, and checks the emitted constants and the MCU subset algebra against
//! the runtime `SE(3)` reference within a documented tolerance band
//! (host↔MCU float determinism, work-plan M21W4-E1-T3). `cargo test -p
//! drm-workspace mcu`.
//!
//! REQ: REQ-SPATIAL-017, REQ-SPATIAL-049, REQ-CORE-087.

use std::collections::BTreeMap;

use drm_spatial::{
    Direction3, Point3, SpatialError, Transform3 as RefTransform, UnitQuat, compose,
    transform_direction, transform_point,
};
use drm_workspace::spatial::{
    McuTarget, classify, collapse_static, emit_mcu_constants, parse_space_file,
};

/// Documented host↔MCU tolerance band for the subset reference checks.
/// REQ: REQ-SPATIAL-017.
const TOL: f64 = 1e-12;

/// A raw rigid transform, matching the emitted `drm_spatial_transform` layout.
/// REQ: REQ-SPATIAL-017.
#[derive(Debug, Clone, Copy, PartialEq)]
struct McuTransform {
    t: [f64; 3],
    q: [f64; 4],
}

/// The MCU subset `drm_spatial_compose`, mirrored from the emitted C.
/// REQ: REQ-SPATIAL-017.
fn mcu_compose(a: &McuTransform, b: &McuTransform) -> McuTransform {
    let [aqx, aqy, aqz, aqw] = a.q;
    let [bqx, bqy, bqz, bqw] = b.q;
    let (rx, ry, rz) = (b.t[0], b.t[1], b.t[2]);
    let tx = 2.0 * (aqy * rz - aqz * ry);
    let ty = 2.0 * (aqz * rx - aqx * rz);
    let tz = 2.0 * (aqx * ry - aqy * rx);
    let mut out = McuTransform {
        t: [
            a.t[0] + rx + aqw * tx + (aqy * tz - aqz * ty),
            a.t[1] + ry + aqw * ty + (aqz * tx - aqx * tz),
            a.t[2] + rz + aqw * tz + (aqx * ty - aqy * tx),
        ],
        q: [
            aqw * bqx + aqx * bqw + aqy * bqz - aqz * bqy,
            aqw * bqy - aqx * bqz + aqy * bqw + aqz * bqx,
            aqw * bqz + aqx * bqy - aqy * bqx + aqz * bqw,
            aqw * bqw - aqx * bqx - aqy * bqy - aqz * bqz,
        ],
    };
    let n = (out.q[0] * out.q[0] + out.q[1] * out.q[1] + out.q[2] * out.q[2] + out.q[3] * out.q[3])
        .sqrt();
    for c in &mut out.q {
        *c /= n;
    }
    out
}

/// The MCU subset `drm_spatial_apply_point`, mirrored from the emitted C.
/// REQ: REQ-SPATIAL-017.
fn mcu_apply_point(t: &McuTransform, p: [f64; 3]) -> [f64; 3] {
    let [qx, qy, qz, qw] = t.q;
    let tx = 2.0 * (qy * p[2] - qz * p[1]);
    let ty = 2.0 * (qz * p[0] - qx * p[2]);
    let tz = 2.0 * (qx * p[1] - qy * p[0]);
    [
        p[0] + qw * tx + (qy * tz - qz * ty) + t.t[0],
        p[1] + qw * ty + (qz * tx - qx * tz) + t.t[1],
        p[2] + qw * tz + (qx * ty - qy * tx) + t.t[2],
    ]
}

/// The MCU subset `drm_spatial_apply_direction`, mirrored from the emitted C.
/// REQ: REQ-SPATIAL-017.
fn mcu_apply_direction(t: &McuTransform, v: [f64; 3]) -> [f64; 3] {
    let [qx, qy, qz, qw] = t.q;
    let tx = 2.0 * (qy * v[2] - qz * v[1]);
    let ty = 2.0 * (qz * v[0] - qx * v[2]);
    let tz = 2.0 * (qx * v[1] - qy * v[0]);
    [
        v[0] + qw * tx + (qy * tz - qz * ty),
        v[1] + qw * ty + (qz * tx - qx * tz),
        v[2] + qw * tz + (qx * ty - qy * tx),
    ]
}

/// Parses the `static const drm_spatial_transform <name>` initializer from the
/// emitted artifact. `Err` when the named constant is absent.
/// REQ: REQ-SPATIAL-017.
fn parse_constant(body: &str, name: &str) -> Result<McuTransform, SpatialError> {
    let needle = format!("static const drm_spatial_transform {name} = {{");
    let start = body.find(&needle).ok_or(SpatialError::InvalidConfig)? + needle.len();
    let end = body[start..]
        .find("};")
        .ok_or(SpatialError::InvalidConfig)?
        + start;
    let nums: Vec<f64> = body[start..end]
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .filter(|s| !s.is_empty() && *s != "-" && *s != "+")
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();
    if nums.len() < 7 {
        return Err(SpatialError::InvalidConfig);
    }
    Ok(McuTransform {
        t: [nums[0], nums[1], nums[2]],
        q: [nums[3], nums[4], nums[5], nums[6]],
    })
}

/// Runtime reference `SE(3)` for the emitted constant. `Err` on a non-unit
/// quaternion (the compiler always emits a normalized constant).
/// REQ: REQ-SPATIAL-017.
fn ref_transform(t: &McuTransform) -> Result<RefTransform, SpatialError> {
    RefTransform::new(t.t, t.q)
}

/// REQ: REQ-SPATIAL-017 — the emitted MCU constants equal the runtime folded
/// constants, and the MCU subset algebra matches the runtime `SE(3)` reference
/// for compose, point, and direction application (I-S13).
#[test]
fn spatial_mcu_constant() {
    let files = vec![
        parse_space_file("s/spaces/world.drm", "").unwrap_or_default(),
        // `base` is dynamic (writer declared); the static chain below it folds.
        parse_space_file("s/spaces/base.drm", "parent = \"world\"\n").unwrap_or_default(),
        parse_space_file(
            "s/spaces/rack.drm",
            "parent = \"base\"\ntranslation = [0.1, 0.2, 0.3]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        )
        .unwrap_or_default(),
        parse_space_file(
            "s/spaces/camera.drm",
            "parent = \"rack\"\ntranslation = [0.0, 0.0, 0.5]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        )
        .unwrap_or_default(),
    ];
    let mut writers = BTreeMap::new();
    writers.insert("base".to_string(), "odom".to_string());
    let model = classify(&files, &writers).unwrap_or_else(|e| unreachable!("{e:?}"));
    let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
    let artifact = emit_mcu_constants(&plan, McuTarget::CortexM);

    // The emitted `camera` constant is the collapsed `base_to_camera` offset.
    let camera = parse_constant(&artifact.body, "camera")
        .unwrap_or_else(|e| unreachable!("camera constant: {e}"));
    let folded = plan
        .constants
        .get("camera")
        .unwrap_or_else(|| unreachable!("camera"));
    for i in 0..3 {
        assert!((camera.t[i] - folded.transform.translation[i]).abs() < TOL);
    }
    for i in 0..4 {
        assert!((camera.q[i] - folded.transform.rotation[i]).abs() < TOL);
    }

    // I-S13: one straight-line multiply of the collapsed constant against the
    // dynamic `world_to_base` edge, checked against the runtime reference.
    let world_to_base = McuTransform {
        t: [1.0, -2.0, 0.5],
        q: UnitQuat::from_axis_angle(Direction3::Z, core::f64::consts::FRAC_PI_4)
            .unwrap_or(UnitQuat::IDENTITY)
            .into_components(),
    };
    let mcu_world_to_camera = mcu_compose(&world_to_base, &camera);

    let ref_base = ref_transform(&world_to_base).unwrap_or_else(|e| unreachable!("{e}"));
    let ref_camera = ref_transform(&camera).unwrap_or_else(|e| unreachable!("{e}"));
    let ref_world_to_camera = compose(&ref_base, &ref_camera);
    for i in 0..3 {
        assert!(
            (mcu_world_to_camera.t[i] - ref_world_to_camera.translation[i]).abs() < TOL,
            "compose translation axis {i}"
        );
    }
    for i in 0..4 {
        assert!(
            (mcu_world_to_camera.q[i] - ref_world_to_camera.rotation[i]).abs() < TOL,
            "compose rotation axis {i}"
        );
    }

    // Point application: rotation then translation.
    let p = [0.25, -0.5, 0.75];
    let mcu_point = mcu_apply_point(&mcu_world_to_camera, p);
    let ref_point = transform_point(
        &ref_world_to_camera,
        Point3::new(p[0], p[1], p[2]).unwrap_or_default(),
    );
    assert!((mcu_point[0] - ref_point.x).abs() < TOL);
    assert!((mcu_point[1] - ref_point.y).abs() < TOL);
    assert!((mcu_point[2] - ref_point.z).abs() < TOL);

    // Direction application: rotation only.
    let d = [1.0, 0.0, 0.0];
    let mcu_dir = mcu_apply_direction(&mcu_world_to_camera, d);
    let ref_dir = transform_direction(
        &ref_world_to_camera,
        Direction3::try_new(d[0], d[1], d[2]).unwrap_or(Direction3::X),
    );
    let rd = ref_dir.components();
    assert!((mcu_dir[0] - rd[0]).abs() < TOL);
    assert!((mcu_dir[1] - rd[1]).abs() < TOL);
    assert!((mcu_dir[2] - rd[2]).abs() < TOL);
}

/// REQ: REQ-SPATIAL-017 — the emitted artifact is deterministic and identifies
/// the dynamic edge the runtime must receive over the link.
#[test]
fn spatial_mcu_artifact_shape() {
    let files = vec![
        parse_space_file("s/spaces/world.drm", "").unwrap_or_default(),
        parse_space_file("s/spaces/base.drm", "parent = \"world\"\n").unwrap_or_default(),
        parse_space_file(
            "s/spaces/camera.drm",
            "parent = \"base\"\ntranslation = [1.0, 2.0, 3.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        )
        .unwrap_or_default(),
    ];
    let mut writers = BTreeMap::new();
    writers.insert("base".to_string(), "odom".to_string());
    let model = classify(&files, &writers).unwrap_or_else(|e| unreachable!("{e:?}"));
    let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
    let a = emit_mcu_constants(&plan, McuTarget::CortexM);
    let b = emit_mcu_constants(&plan, McuTarget::CortexM);
    assert_eq!(a, b);
    assert!(a.body.contains("dynamic edge `base`"));
}
