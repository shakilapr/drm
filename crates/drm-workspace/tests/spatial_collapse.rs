//! Build-time static collapse v0 (work-plan M19W1-E2-T3): parse `spaces/`,
//! classify edges, fold contiguous static chains to one `SE(3)` constant, and
//! emit the codegen artifact. `cargo test -p drm-workspace spatial_collapse`.
//!
//! REQ: REQ-SPATIAL-047, REQ-SPATIAL-048, REQ-SPATIAL-049, REQ-SPATIAL-031,
//! REQ-SPATIAL-017.

use std::collections::BTreeMap;

use drm_workspace::spatial::{
    Transform3, classify, collapse_static, emit_constants, frame_from_path, parse_space_file,
};

/// Builds a static chain `world → base → rack → camera` from `spaces/` sources.
/// REQ: REQ-SPATIAL-047.
fn chain() -> Vec<drm_workspace::spatial::SpaceFile> {
    [
        ("s/spaces/world.drm", ""),
        (
            "s/spaces/base.drm",
            "parent = \"world\"\ntranslation = [1.0, 0.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        ),
        (
            "s/spaces/rack.drm",
            "parent = \"base\"\ntranslation = [0.0, 2.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        ),
        (
            "s/spaces/camera.drm",
            "parent = \"rack\"\ntranslation = [0.0, 0.0, 3.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        ),
    ]
    .into_iter()
    .map(|(path, src)| parse_space_file(path, src).unwrap_or_else(|e| unreachable!("{e}")))
    .collect()
}

/// REQ: REQ-SPATIAL-049 — the full static chain folds to one constant at build.
#[test]
fn spatial_collapse_static_chain() {
    let files = chain();
    let model = classify(&files, &BTreeMap::new()).unwrap_or_else(|e| unreachable!("{e:?}"));
    let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
    let camera = plan
        .constants
        .get("camera")
        .unwrap_or_else(|| unreachable!("camera"));
    assert_eq!(camera.anchor, "world");
    assert_eq!(
        camera.transform,
        Transform3 {
            translation: [1.0, 2.0, 3.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    );
    assert_eq!(plan.constants.len(), 3);
}

/// REQ: REQ-SPATIAL-017 — the emitted artifact carries a `@generated` banner
/// and the folded constants for MCU codegen.
#[test]
fn spatial_collapse_emit() {
    let model = classify(&chain(), &BTreeMap::new()).unwrap_or_else(|e| unreachable!("{e:?}"));
    let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
    let artifact = emit_constants(&plan);
    assert!(artifact.body.contains("@generated"));
    assert!(artifact.body.contains("camera_t[3]"));
    assert!(artifact.body.contains("camera_q[4]"));
}

/// REQ: REQ-SPATIAL-031 — a dynamic writer breaks the chain; the static prefix
/// re-anchors at the dynamic edge and the edge stays a runtime entry.
#[test]
fn spatial_collapse_dynamic_break() {
    let files = vec![
        parse_space_file("s/spaces/world.drm", "").unwrap_or_default(),
        parse_space_file("s/spaces/odom.drm", "parent = \"world\"\n").unwrap_or_default(),
        parse_space_file(
            "s/spaces/base.drm",
            "parent = \"odom\"\ntranslation = [4.0, 0.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        )
        .unwrap_or_default(),
    ];
    let mut writers = BTreeMap::new();
    writers.insert("odom".to_string(), "tracker".to_string());
    let model = classify(&files, &writers).unwrap_or_else(|e| unreachable!("{e:?}"));
    let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
    assert!(plan.dynamic_edges.contains("odom"));
    let base = plan
        .constants
        .get("base")
        .unwrap_or_else(|| unreachable!("base"));
    assert_eq!(base.anchor, "odom");
}

/// REQ: REQ-SPATIAL-049 — collapse is deterministic under shuffled input order
/// (`I-D1`: sorted maps).
#[test]
fn spatial_collapse_deterministic() {
    let mut files = chain();
    let model_a = classify(&files, &BTreeMap::new()).unwrap_or_else(|e| unreachable!("{e:?}"));
    files.reverse();
    let model_b = classify(&files, &BTreeMap::new()).unwrap_or_else(|e| unreachable!("{e:?}"));
    let plan_a = collapse_static(&model_a).unwrap_or_else(|e| unreachable!("{e}"));
    let plan_b = collapse_static(&model_b).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(plan_a, plan_b);
    assert_eq!(emit_constants(&plan_a), emit_constants(&plan_b));
}

/// REQ: REQ-SPATIAL-047 — path identity is the filesystem path.
#[test]
fn spatial_frame_identity() {
    assert_eq!(
        frame_from_path("packages/robot/spaces/camera/front.drm").unwrap_or_default(),
        "camera/front"
    );
}
