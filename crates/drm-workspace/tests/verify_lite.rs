//! Verify-lite V-SW slice (work-plan M03W1-E2-T1): single-writer policy over
//! the built model — consumed topic with no publisher, topic with multiple
//! publishers — plus positive and determinism fixtures.
//! REQ: REQ-TOOL-064, REQ-TOOL-070.

use std::fmt::Write as _;

use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build};
use drm_workspace::verify::{
    V_SW_MULTI_PUBLISHER, V_SW_NO_PUBLISHER, VerifyLiteFinding, verify_lite,
};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

fn manifest(name: &str, publishes: &[&str], consumes: &[&str]) -> String {
    let list = |v: &[&str]| {
        v.iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut s = format!("name = \"{name}\"\n");
    let _ = write!(
        s,
        "publishes = [{}]\nconsumes = [{}]\n",
        list(publishes),
        list(consumes)
    );
    s
}

fn tree_with(nodes: &[(&str, String)]) -> TreeInput {
    TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![
                ("front".to_string(), "v: u32\n".to_string().into_bytes()),
                ("telemetry".to_string(), "v: u32\n".to_string().into_bytes()),
            ],
        }],
        nodes: nodes
            .iter()
            .map(|(stem, src)| ((*stem).to_string(), src.clone().into_bytes()))
            .collect(),
        topology: None,
    }
}

fn build_model(nodes: &[(&str, String)]) -> WorkspaceModel {
    fixture(build(&tree_with(nodes)), "build")
}

/// REQ: REQ-TOOL-064 — a consumed topic with no publisher is V-SW-1.
#[test]
fn no_publisher_flagged() {
    let m = build_model(&[("c", manifest("c", &[], &["pkg/front"]))]);
    let findings = verify_lite(&m);
    assert_eq!(
        findings,
        vec![VerifyLiteFinding::NoPublisher {
            topic: "pkg/front".to_string(),
            consumers: vec!["c".to_string()],
        }]
    );
    let msg = findings[0].to_string();
    assert!(msg.contains(V_SW_NO_PUBLISHER), "check id: {msg}");
    assert!(msg.contains("REQ-TOOL-064"), "req id: {msg}");
}

/// REQ: REQ-TOOL-064 — two publishers of one topic is V-SW-2.
#[test]
fn multiple_publishers_flagged() {
    let m = build_model(&[
        ("a", manifest("a", &["pkg/front"], &[])),
        ("b", manifest("b", &["pkg/front"], &[])),
    ]);
    let findings = verify_lite(&m);
    assert_eq!(
        findings,
        vec![VerifyLiteFinding::MultiplePublishers {
            topic: "pkg/front".to_string(),
            publishers: vec!["a".to_string(), "b".to_string()],
        }]
    );
    let msg = findings[0].to_string();
    assert!(msg.contains(V_SW_MULTI_PUBLISHER), "check id: {msg}");
    assert!(msg.contains("REQ-TOOL-064"), "req id: {msg}");
}

/// REQ: REQ-TOOL-064 — one publisher per consumed topic: no findings.
#[test]
fn clean_workspace_passes() {
    let m = build_model(&[
        ("pub_node", manifest("pub_node", &["pkg/front"], &[])),
        ("sub_node", manifest("sub_node", &[], &["pkg/front"])),
    ]);
    assert_eq!(verify_lite(&m), vec![]);
}

/// REQ: REQ-TOOL-064 — deterministic report for the same model (I-V2).
#[test]
fn verify_lite_deterministic() {
    let m = build_model(&[
        ("a", manifest("a", &["pkg/front"], &["pkg/telemetry"])),
        ("b", manifest("b", &["pkg/front"], &[])),
    ]);
    assert_eq!(verify_lite(&m), verify_lite(&m));
}
