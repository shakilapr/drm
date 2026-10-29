//! The default `@pool_size` rule must hold in one place, and hold in both places.
//!
//! `payload-pool.md` I-P6 / `architecture.md` §Bulk Latest Topics specify a
//! **triple buffer (3)** for a `@history: latest` topic that declares no
//! `@pool_size`, and `R + 1` as the mandatory floor (I-P4) for everything else.
//!
//! The rule necessarily exists twice: `drm-core::channel::pool` owns it for
//! runtime activation, and `drm-workspace::model` owns it for the build-time plan,
//! because layering forbids `drm-workspace -> drm-core`. They drifted — the model
//! shipped `R + 1` for every topic, so no deployment ever got the documented
//! triple buffer and every defaulted topic was one slot short of drop-free.
//!
//! The cross-crate agreement check lives in `crates/drm-cli/tests/`, the first
//! layer above both, because neither crate may depend on the other.
//!
//! REQ: REQ-CORE-041, REQ-CORE-045.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use drm_workspace::model::{PackageInput, TreeInput, build, runtime_plan};

/// Builds a workspace with one topic and returns its runtime plan entry.
fn plan_for(topic_text: &str) -> drm_workspace::model::TopicRuntime {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![("t".to_string(), topic_text.as_bytes().to_vec())],
        }],
        nodes: Vec::new(),
        topology: None,
    };
    let model = build(&tree).expect("valid workspace");
    runtime_plan(&model)
        .get("pkg/t")
        .cloned()
        .expect("topic present")
}

/// I-P6: an undeclared `@pool_size` on a `@history: latest` topic resolves to the
/// documented triple buffer, whether `latest` is written out or implied.
///
/// REQ: REQ-CORE-041.
#[test]
fn pool_default_latest() {
    // Implied latest: no `@history` at all.
    let implied = plan_for("v: f32\n");
    assert_eq!(implied.history_depth, 1, "undeclared history is latest");
    assert_eq!(
        implied.pool_size, 3,
        "implied latest gets the triple buffer"
    );

    // Written out.
    let declared = plan_for("@history: latest\nv: f32\n");
    assert_eq!(
        declared.pool_size, 3,
        "`@history: latest` gets the triple buffer"
    );

    // The triple buffer is exactly the drop-free floor for R=1, H=1: one latest,
    // one in-progress write, one held by a reader (architecture.md).
    // `pool_size >= R + H + 1` with R = 1, H = 1 is exactly `pool_size >= 3`.
    assert!(
        declared.pool_size > declared.history_depth + 1,
        "3 must satisfy R + H + 1 for H = 1"
    );
}

/// I-P4: the default is a floor, never a ceiling. A deep history must not receive
/// the smaller sequential default.
///
/// REQ: REQ-CORE-045.
#[test]
fn default_pool_always_satisfies_r_plus_one() {
    for depth in [1_u64, 2, 4, 8, 9, 64, 128, 1024] {
        let plan = plan_for(&format!("@history: {depth}\nv: f32\n"));
        assert_eq!(plan.history_depth, depth);
        assert!(
            plan.pool_size > plan.history_depth,
            "@history: {depth} got pool {} — below the R+1 floor",
            plan.pool_size
        );
    }
    // A shallow explicit history keeps the documented sequential default.
    let shallow = plan_for("@history: 4\nv: f32\n");
    assert_eq!(shallow.pool_size, 8, "sequential default is 8");
}

/// An explicit `@pool_size` always wins: the default must never override a
/// declared value, even when the declaration is larger or smaller (I-P4 is the
/// verifier's job, not the default's).
///
/// REQ: REQ-CORE-045.
#[test]
fn explicit_pool_size_wins() {
    let big = plan_for("@history: 4\n@pool_size: 64\nv: f32\n");
    assert_eq!(big.pool_size, 64);
    let minimal = plan_for("@history: 4\n@pool_size: 5\nv: f32\n");
    assert_eq!(
        minimal.pool_size, 5,
        "an explicit floor-legal value is kept"
    );
}
