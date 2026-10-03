//! Node manifest parsing and reference checks (work-plan M02W3-E2-T2):
//! publish/consume sets, unknown-topic errors, duplicate node identity,
//! multi-publisher precheck query (model builds; verify owns policy).
//! REQ: REQ-TOOL-047, REQ-TOOL-045, REQ-TOOL-013, REQ-TOOL-070.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use drm_workspace::model::{
    ModelError, PackageInput, TreeInput, WorkspaceModel, build, conflicting_publishers,
};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// Inverse fixture: every defect of an intentionally-failing build.
fn fixture_errs(what: &str, r: Result<WorkspaceModel, Vec<ModelError>>) -> Vec<ModelError> {
    match r {
        Ok(_) => unreachable!("fixture invariant violated ({what}): build must fail"),
        Err(errs) => errs,
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

/// REQ: REQ-TOOL-047 — valid manifests parse into sorted node entries.
#[test]
fn nodes_parse_into_model() {
    let t = tree_with(&[(
        "controller",
        manifest("controller", &[], &["pkg/telemetry", "pkg/front"]),
    )]);
    let m = fixture(build(&t), "build");
    let node = &m.nodes["controller"];
    let expected: BTreeSet<String> = ["pkg/front", "pkg/telemetry"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(node.consumes, expected);
    assert!(node.publishes.is_empty());
}

/// REQ: REQ-TOOL-047 — unknown topic in publish or consume is an error with
/// the referencing node named (I-D2).
#[test]
fn nodes_unknown_topic_rejected() {
    let t = tree_with(&[("motor", manifest("motor", &["pkg/nope"], &[]))]);
    let errs = fixture_errs("unknown topic", build(&t));
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ModelError::UnknownTopic { node, topic }
                if node == "motor" && topic == "pkg/nope"
        )),
        "errors: {errs:?}"
    );
}

/// REQ: REQ-TOOL-045 — duplicate node identity reports both file stems.
#[test]
fn nodes_duplicate_identity_rejected() {
    let a = manifest("dup", &[], &[]);
    let t = tree_with(&[("one", a.clone()), ("two", a)]);
    let errs = fixture_errs("duplicate node", build(&t));
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ModelError::DuplicateNode { node, first, second }
                if node == "dup" && first == "one" && second == "two"
        )),
        "errors: {errs:?}"
    );
}

/// REQ: REQ-TOOL-047 — manifest surface is closed: unknown keys, empty
/// names, and wrong-typed entries are rejected with the stem named.
#[test]
fn nodes_manifest_surface_closed() {
    let bad_key = "name = \"x\"\nweird = true\n";
    let errs = fixture_errs(
        "unknown key",
        build(&tree_with(&[("x", bad_key.to_string())])),
    );
    assert!(
        errs.iter()
            .any(|e| matches!(e, ModelError::NodeManifest { stem, .. } if stem == "x")),
        "errors: {errs:?}"
    );
    let errs = fixture_errs(
        "empty name",
        build(&tree_with(&[("y", "name = \"\"\n".to_string())])),
    );
    assert!(
        errs.iter()
            .any(|e| matches!(e, ModelError::NodeManifest { stem, .. } if stem == "y")),
        "errors: {errs:?}"
    );
    let errs = fixture_errs(
        "wrong-typed entries",
        build(&tree_with(&[("z", "publishes = [1]\n".to_string())])),
    );
    assert!(
        errs.iter()
            .any(|e| matches!(e, ModelError::NodeManifest { stem, .. } if stem == "z")),
        "errors: {errs:?}"
    );
}

/// REQ: REQ-TOOL-070 — two publishers of one topic: the model builds
/// (structural integrity only) and the precheck query names the conflict.
#[test]
fn nodes_multi_publisher_is_query_not_error() {
    let t = tree_with(&[
        ("a", manifest("a", &["pkg/front"], &[])),
        ("b", manifest("b", &["pkg/front"], &[])),
        ("c", manifest("c", &[], &["pkg/telemetry"])),
    ]);
    let m = fixture(build(&t), "build");
    let conflicts = conflicting_publishers(&m);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].0, "pkg/front");
    assert_eq!(conflicts[0].1, vec!["a", "b"]);
}
