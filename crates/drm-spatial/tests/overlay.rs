//! Bounded runtime-overlay plan suite (`cargo test -p drm-spatial overlay`):
//! a runtime overlay query resolves through a bounded overlay walk (≤
//! `max_overlay_depth` parent steps) to its compiled-base attachment frame, and
//! the small overlay prefix is cached under `topology_generation` so a
//! generation bump re-walks exactly once ([spatial-queries.md] §Invariants
//! `I-Q16`, §Protocols plan lifecycle step 3;
//! [`spatial_architecture.md`] §Dual Hierarchy). An overlay walk deeper than the
//! declared bound is refused, never unbounded.
//!
//! REQ: REQ-SPATIAL-008, REQ-SPATIAL-034, REQ-SPATIAL-010.

// Test fixtures convert between nanosecond integer stamps and `f64` geometry.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use drm_spatial::{OverlayPlanCache, SpaceForest, SpatialError, Transform3, compile_overlay_plan};

/// A static translation transform.
/// REQ: REQ-SPATIAL-038.
fn xyz(x: f64, y: f64, z: f64) -> Transform3 {
    Transform3 {
        translation: [x, y, z],
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// A forest with a compiled base (`world` → `base`) plus `n` stacked overlay
/// frames `overlay/0` … `overlay/n-1` attached to `base`/the previous overlay.
/// REQ: REQ-SPATIAL-008.
fn overlay_forest(n: usize) -> SpaceForest {
    let mut f = SpaceForest::new();
    f.add_root("world".to_string());
    f.add_static("base".to_string(), "world".to_string(), xyz(1.0, 0.0, 0.0))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let mut parent = "base".to_string();
    for i in 0..n {
        let frame = format!("overlay/{i}");
        f.add_overlay(frame.clone(), parent.clone(), xyz(0.0, 1.0, 0.0))
            .unwrap_or_else(|e| unreachable!("{e}"));
        parent = frame;
    }
    f
}

/// REQ: REQ-SPATIAL-008 — `spatial_overlay_plan_bound`: a runtime overlay frame
/// resolves through a bounded overlay walk to its compiled-base attachment, and
/// the compiled plan is reused while `topology_generation` is unchanged; a
/// generation bump re-walks exactly once (`I-Q16`).
#[test]
fn spatial_overlay_plan_bound() {
    let f = overlay_forest(2);
    // The deepest overlay frame is 2 overlay steps from its base attachment.
    assert_eq!(f.overlay_depth("overlay/1"), 2);
    assert_eq!(f.overlay_attachment("overlay/1"), "base");
    assert_eq!(f.overlay_depth("base"), 0);
    assert!(!f.is_overlay("base"));
    assert!(f.is_overlay("overlay/0"));

    let mut cache = OverlayPlanCache::new(7);
    let first = compile_overlay_plan(&f, "overlay/1", "world", 2, 7, &mut cache)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(cache.walks(), 1, "first query walks once");
    assert_eq!(cache.depth(), 2);
    assert_eq!(cache.attachment(), "base");
    // The folded plan resolves the overlay prefix plus the base plan.
    assert!(first.steps().len() >= 3);

    // Same generation: reuse the cached plan with no re-walk.
    let second = compile_overlay_plan(&f, "overlay/1", "world", 2, 7, &mut cache)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(cache.walks(), 1, "no re-walk under an unchanged generation");
    assert_eq!(second.steps(), first.steps());

    // A topology_generation bump (reparent / overlay mutation) re-walks once.
    let _ = compile_overlay_plan(&f, "overlay/1", "world", 2, 8, &mut cache)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(cache.walks(), 2, "one re-walk per generation bump");
    assert_eq!(cache.topology_generation(), 8);
}

/// REQ: REQ-SPATIAL-008 — an overlay walk deeper than `max_overlay_depth` is
/// refused with `InvalidConfig`; the walk is bounded, never unbounded
/// (`I-Q16`).
#[test]
fn spatial_overlay_depth_refused() {
    let f = overlay_forest(3);
    assert_eq!(f.overlay_depth("overlay/2"), 3);
    let mut cache = OverlayPlanCache::new(0);
    assert_eq!(
        compile_overlay_plan(&f, "overlay/2", "world", 2, 0, &mut cache).err(),
        Some(SpatialError::InvalidConfig)
    );
    assert_eq!(cache.walks(), 0, "a refused walk never compiles");
    // Within the bound it resolves.
    assert!(compile_overlay_plan(&f, "overlay/2", "world", 3, 0, &mut cache).is_ok());
}

/// REQ: REQ-SPATIAL-034 — a base (non-overlay) pair compiles at depth 0 through
/// the same overlay-plan path (`I-Q16`).
#[test]
fn spatial_overlay_base_pair() {
    let f = overlay_forest(1);
    let mut cache = OverlayPlanCache::new(1);
    let plan = compile_overlay_plan(&f, "base", "world", 0, 1, &mut cache)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(cache.depth(), 0);
    assert_eq!(plan.steps().len(), 1);
    assert_eq!(
        plan.steps()[0],
        drm_spatial::PlanStep::Constant(xyz(1.0, 0.0, 0.0))
    );
}
