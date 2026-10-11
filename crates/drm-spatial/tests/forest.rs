//! Forest-disconnected-trees suite (`cargo test -p drm-spatial forest`):
//! cross-tree queries return `PathNotFound`; explicit bridges and common
//! ancestors connect trees (`I-S3`, [spatial-core.md] §Invariants `I-S3`,
//! §Test Matrix `spatial_forest`).
//!
//! REQ: REQ-SPATIAL-010, REQ-SPATIAL-039.

use drm_spatial::Point3;
use drm_spatial::{PathEdge, SpaceForest, SpatialError, Transform3, transform_point};

/// A static translation transform.
/// REQ: REQ-SPATIAL-048.
fn t(x: f64, y: f64, z: f64) -> Transform3 {
    Transform3 {
        translation: [x, y, z],
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// Folds a resolved path (reference toward target) into one transform.
/// REQ: REQ-SPATIAL-039.
fn fold(path: &[PathEdge]) -> Transform3 {
    let mut acc = Transform3::IDENTITY;
    for edge in path {
        let next = match edge {
            PathEdge::Constant(t) | PathEdge::Bridge(t) => *t,
            PathEdge::Dynamic(_) => Transform3::IDENTITY,
        };
        acc = drm_spatial::compose(&next, &acc);
    }
    acc
}

/// Two independent robot trees: `robot/a/{world,base}` and `robot/b/{world,base}`.
/// REQ: REQ-SPATIAL-010.
fn two_robots() -> SpaceForest {
    let mut f = SpaceForest::new();
    f.add_root("robot/a/world".to_string());
    f.add_static(
        "robot/a/base".to_string(),
        "robot/a/world".to_string(),
        t(1.0, 0.0, 0.0),
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    f.add_root("robot/b/world".to_string());
    f.add_static(
        "robot/b/base".to_string(),
        "robot/b/world".to_string(),
        t(5.0, 0.0, 0.0),
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    f
}

/// REQ: REQ-SPATIAL-010 — `spatial_forest`: disconnected trees coexist; a query
/// between them returns `PathNotFound` (`I-S3`).
#[test]
fn spatial_forest_disconnected() {
    let f = two_robots();
    assert!(f.path_between("robot/a/base", "robot/a/world").is_ok());
    assert_eq!(
        f.path_between("robot/a/base", "robot/b/world"),
        Err(SpatialError::PathNotFound)
    );
    assert!(!f.connected("robot/a/base", "robot/b/base"));
}

/// REQ: REQ-SPATIAL-039 — a registered bridge makes a cross-tree query resolve.
#[test]
fn spatial_forest_bridge() {
    let mut f = two_robots();
    f.add_bridge(
        "robot/a/world".to_string(),
        "robot/b/world".to_string(),
        t(10.0, 0.0, 0.0),
    );
    let path = f
        .path_between("robot/a/base", "robot/b/base")
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(path.iter().any(|e| matches!(e, PathEdge::Bridge(_))));
    // Folding the path maps a/base points into b/base: 1 + 10 − 5 = 6.
    let folded = fold(&path);
    let probe = transform_point(&folded, Point3::default());
    assert!((probe.x - 6.0).abs() < 1e-9, "x = {}", probe.x);
}

/// REQ: REQ-SPATIAL-039 — multi-robot resolution through a shared common
/// ancestor frame (no bridge needed).
#[test]
fn spatial_forest_common_ancestor() {
    let mut f = SpaceForest::new();
    f.add_root("earth".to_string());
    f.add_static("a/base".to_string(), "earth".to_string(), t(1.0, 0.0, 0.0))
        .unwrap_or_else(|e| unreachable!("{e}"));
    f.add_static("b/base".to_string(), "earth".to_string(), t(-2.0, 0.0, 0.0))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let path = f
        .path_between("a/base", "b/base")
        .unwrap_or_else(|e| unreachable!("{e}"));
    let folded = fold(&path);
    let probe = transform_point(&folded, Point3::default());
    assert!((probe.x - 3.0).abs() < 1e-9, "x = {}", probe.x);
}

/// REQ: REQ-SPATIAL-048 — dynamic edges surface as `PathEdge::Dynamic`.
#[test]
fn spatial_forest_dynamic_edge() {
    let mut f = SpaceForest::new();
    f.add_root("map".to_string());
    f.add_dynamic("odom".to_string(), "map".to_string(), 3);
    f.add_static("base".to_string(), "odom".to_string(), t(0.0, 1.0, 0.0))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let path = f
        .path_between("base", "map")
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(path.len(), 2);
    assert_eq!(path[0], PathEdge::Constant(t(0.0, 1.0, 0.0)));
    assert_eq!(path[1], PathEdge::Dynamic(3));
}
