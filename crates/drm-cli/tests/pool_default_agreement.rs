//! Rule duplication across the layering boundary: the default `@pool_size` is
//! implemented in `drm-core` (runtime activation) and in `drm-workspace`
//! (build-time plan), because `drm-workspace` must never depend on `drm-core`.
//!
//! That duplication is the only way to satisfy `codebase.md` layering, but it is
//! also how the two drifted apart: the workspace side shipped `R + 1` for every
//! topic while the core side held the documented triple buffer of 3, so no
//! deployment ever got it and each side's own unit test still passed.
//!
//! This test sits one layer above both, which is the only place the pair is
//! visible at once.
//!
//! REQ: REQ-CORE-041, REQ-CORE-045.
#![allow(clippy::expect_used, clippy::unwrap_used)]

/// The build-time plan and the runtime must answer the same number.
///
/// REQ: REQ-CORE-041 — one rule, one answer, whichever side asks.
#[test]
fn default_pool_sizes_agree_across_the_layering_boundary() {
    for latest in [true, false] {
        let planned = drm_workspace::model::runtime::default_pool_size(latest);
        let runtime = drm_core::channel::pool::default_pool_size(latest);
        assert_eq!(
            planned, runtime,
            "default_pool_size({latest}) differs: build plan says {planned}, runtime says {runtime}"
        );
    }

    // Pin the values the documentation promises, not merely agreement: two
    // implementations that drift together would still be wrong.
    assert_eq!(
        drm_workspace::model::runtime::default_pool_size(true),
        3,
        "a `@history: latest` topic defaults to the documented triple buffer"
    );
}

/// A built topic carries the documented default, end to end.
///
/// REQ: REQ-CORE-041 — the plan a deployment ships, not just a constant.
#[test]
fn built_latest_topic_carries_the_triple_buffer() {
    use drm_workspace::model::{PackageInput, TreeInput, build, runtime_plan};

    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![("t".to_string(), b"v: f32\n".to_vec())],
        }],
        nodes: Vec::new(),
        topology: None,
    };
    let model = build(&tree).expect("valid workspace");
    let plan = runtime_plan(&model);
    let topic = plan.get("pkg/t").expect("topic present");
    assert_eq!(
        topic.pool_size,
        drm_core::channel::pool::default_pool_size(true),
        "the shipped plan must use the same default the runtime would"
    );
    assert_eq!(topic.pool_size, 3);
}
