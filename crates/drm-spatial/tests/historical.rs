//! Historical-evaluation suite (`cargo test -p drm-spatial historical`):
//! conversions evaluate the space graph at the query's historical sample
//! timestamp, never "now" (`I-S9`, [spatial-core.md] §Invariants `I-S9`,
//! §Test Matrix `spatial_historical_evaluation`).
//!
//! REQ: REQ-SPATIAL-029, REQ-SPATIAL-010, REQ-SPATIAL-039.

use drm_spatial::{
    HistoricalForest, PathEdge, SpaceForest, SpatialError, TopologyEvent, Transform3,
};

/// A static translation transform.
/// REQ: REQ-SPATIAL-048.
fn t(x: f64, y: f64, z: f64) -> Transform3 {
    Transform3 {
        translation: [x, y, z],
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// A base two-frame tree.
/// REQ: REQ-SPATIAL-010.
fn tree() -> SpaceForest {
    let mut f = SpaceForest::new();
    f.add_root("world".to_string());
    f.add_static("base".to_string(), "world".to_string(), t(1.0, 0.0, 0.0))
        .unwrap_or_else(|e| unreachable!("{e}"));
    f
}

/// REQ: REQ-SPATIAL-029 — `spatial_historical_evaluation`: a conversion at a
/// historical timestamp uses the hierarchy in effect then, not the current one.
#[test]
fn spatial_historical_evaluation() {
    let mut h = HistoricalForest::new(tree());
    // Camera is attached at t=100, then reparented to a new anchor at t=200.
    h.record(TopologyEvent::AddStatic {
        at_ns: 100,
        frame: "camera".to_string(),
        parent: "base".to_string(),
        transform: t(0.0, 0.0, 2.0),
    });
    h.record(TopologyEvent::AddStatic {
        at_ns: 200,
        frame: "camera".to_string(),
        parent: "world".to_string(),
        transform: t(9.0, 0.0, 0.0),
    });

    // At t=150 the original hierarchy applies: base → camera with 2 m offset.
    let before = h
        .path_between_at("camera", "world", 150)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(
        before,
        vec![
            PathEdge::Constant(t(0.0, 0.0, 2.0)),
            PathEdge::Constant(t(1.0, 0.0, 0.0))
        ]
    );

    // At t=250 the new hierarchy applies: direct world → camera.
    let after = h
        .path_between_at("camera", "world", 250)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(after, vec![PathEdge::Constant(t(9.0, 0.0, 0.0))]);
}

/// REQ: REQ-SPATIAL-029 — history before a topology event is never rewritten.
#[test]
fn historical_never_rewrites_past() {
    let mut h = HistoricalForest::new(tree());
    h.record(TopologyEvent::AddStatic {
        at_ns: 500,
        frame: "camera".to_string(),
        parent: "base".to_string(),
        transform: t(0.0, 0.0, 2.0),
    });
    // Well before the event the frame does not exist.
    assert_eq!(
        h.path_between_at("camera", "world", 0),
        Err(SpatialError::PathNotFound)
    );
    // And the current graph (no event applied) still lacks it.
    assert!(!h.current().contains("camera"));
}

/// REQ: REQ-SPATIAL-039 — a bridge registered at t₁ connects the trees only from
/// t₁ onward.
#[test]
fn historical_bridge_effective_time() {
    let mut base = tree();
    base.add_root("sim/world".to_string());
    let mut h = HistoricalForest::new(base);
    h.record(TopologyEvent::AddBridge {
        at_ns: 300,
        from: "world".to_string(),
        to: "sim/world".to_string(),
        transform: t(50.0, 0.0, 0.0),
    });
    assert_eq!(
        h.path_between_at("base", "sim/world", 299),
        Err(SpatialError::PathNotFound)
    );
    assert!(h.path_between_at("base", "sim/world", 300).is_ok());
}
