//! Topology manifest parsing and model population (work-plan M02W4-E2-T2):
//! computes, links (sorted by id), placement; referential rejections; closed
//! manifest surface. REQ: REQ-TOOL-047, REQ-TOOL-070, REQ-TOOL-045.

use std::collections::BTreeSet;

use drm_workspace::model::{
    ComputeEntry, LinkEntry, ModelError, TreeInput, build, parse_topology_manifest, render_model,
};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// Inverse fixture: every defect of an intentionally-failing build.
fn fixture_errs(
    what: &str,
    r: Result<drm_workspace::model::WorkspaceModel, Vec<ModelError>>,
) -> Vec<ModelError> {
    match r {
        Ok(_) => unreachable!("fixture invariant violated ({what}): build must fail"),
        Err(errs) => errs,
    }
}

/// Fixture for a failing standalone parse.
fn parse_err(src: &[u8], what: &str) -> String {
    match parse_topology_manifest(src) {
        Ok(_) => unreachable!("fixture invariant violated ({what}): parse must fail"),
        Err(message) => message,
    }
}

fn manifest(name: &str) -> String {
    format!("name = \"{name}\"\n")
}

fn tree(nodes: &[&str], topology: Option<&str>) -> TreeInput {
    TreeInput {
        packages: Vec::new(),
        nodes: nodes
            .iter()
            .map(|n| ((*n).to_string(), manifest(n).into_bytes()))
            .collect(),
        topology: topology.map(str::as_bytes).map(Vec::from),
    }
}

/// REQ: REQ-TOOL-047 — computes, links, and placement land in the model; links
/// are sorted by id regardless of manifest input order (I-D1).
#[test]
fn topology_parses_and_populates() {
    let topo = "[computes]\nedge = true\ncloud = true\n\n\
                [[links]]\nid = \"l2\"\na = \"cloud\"\nb = \"edge\"\n\n\
                [[links]]\nid = \"l1\"\na = \"edge\"\nb = \"cloud\"\n\n\
                [placement]\nmotor = \"edge\"\n";
    let m = fixture(build(&tree(&["motor"], Some(topo))), "build");
    let keys: BTreeSet<&str> = m.computes.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["cloud", "edge"]));
    assert_eq!(m.computes.get("edge"), Some(&ComputeEntry));
    assert_eq!(
        m.links,
        vec![
            LinkEntry {
                id: "l1".to_string(),
                a: "edge".to_string(),
                b: "cloud".to_string(),
            },
            LinkEntry {
                id: "l2".to_string(),
                a: "cloud".to_string(),
                b: "edge".to_string(),
            },
        ]
    );
    assert_eq!(m.placement.get("motor"), Some(&"edge".to_string()));
}

/// REQ: REQ-TOOL-047 — a link endpoint referencing an undeclared compute fails
/// the build, naming both the link and the vertex (I-D2).
#[test]
fn unknown_link_endpoint_rejected() {
    let topo = "[computes]\nedge = true\n\n\
                [[links]]\nid = \"l1\"\na = \"edge\"\nb = \"ghost\"\n";
    let errs = fixture_errs("unknown endpoint", build(&tree(&["n0"], Some(topo))));
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ModelError::UnknownLinkEndpoint { link, vertex }
                if link == "l1" && vertex == "ghost"
        )),
        "errors: {errs:?}"
    );
}

/// REQ: REQ-TOOL-047 — placement of an undeclared node and placement onto an
/// undeclared compute are distinct collected errors (I-D2).
#[test]
fn dangling_placement_rejected() {
    let topo1 = "[computes]\nedge = true\n\n[placement]\nghost = \"edge\"\n";
    let errs = fixture_errs("dangling node", build(&tree(&["n0"], Some(topo1))));
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ModelError::DanglingPlacement { node } if node == "ghost"
        )),
        "errors: {errs:?}"
    );
    let topo2 = "[computes]\nedge = true\n\n[placement]\nn0 = \"ghost\"\n";
    let errs = fixture_errs("unknown compute", build(&tree(&["n0"], Some(topo2))));
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ModelError::PlacementUnknownCompute { node, compute }
                if node == "n0" && compute == "ghost"
        )),
        "errors: {errs:?}"
    );
}

/// REQ: REQ-TOOL-047 — a repeated link id is one collected build conflict for
/// the pair (I-D2).
#[test]
fn duplicate_link_rejected() {
    let topo = "[computes]\na = true\nb = true\n\n\
                [[links]]\nid = \"l1\"\na = \"a\"\nb = \"b\"\n\n\
                [[links]]\nid = \"l1\"\na = \"b\"\nb = \"a\"\n";
    let errs = fixture_errs("duplicate link", build(&tree(&["n0"], Some(topo))));
    assert_eq!(errs.len(), 1, "errors: {errs:?}");
    assert!(
        matches!(&errs[0], ModelError::DuplicateLink { link } if link == "l1"),
        "errors: {errs:?}"
    );
}

/// REQ: REQ-TOOL-047 — the manifest surface is closed: unknown top-level keys,
/// wrong-typed values, and self-loop links are parse errors with a message.
#[test]
fn topology_manifest_closed_surface() {
    let unknown = parse_err(b"mystery = true\n", "unknown key");
    assert!(unknown.contains("unknown key"), "{unknown}");
    let typed = parse_err(b"computes = 3\n", "wrong type");
    assert!(typed.contains("`computes` must be a table"), "{typed}");
    let self_loop = parse_err(
        b"[computes]\nx = true\n\n[[links]]\nid = \"l1\"\na = \"x\"\nb = \"x\"\n",
        "self-loop",
    );
    assert!(self_loop.contains("self-loop"), "{self_loop}");
}

/// REQ: REQ-TOOL-047 — the canonical render carries computes, links, and
/// placement in sorted order after the node sections (I-D1/I-D5).
#[test]
fn render_topology_roundtrip() {
    let topo = "[computes]\nedge = true\ncloud = true\n\n\
                [[links]]\nid = \"l2\"\na = \"cloud\"\nb = \"edge\"\n\n\
                [[links]]\nid = \"l1\"\na = \"edge\"\nb = \"cloud\"\n\n\
                [placement]\nmotor = \"edge\"\n";
    let m = fixture(build(&tree(&["motor"], Some(topo))), "build");
    let text = render_model(&m);
    let expect = "node motor\npublishes\nconsumes\nend\n\
                  compute cloud\ncompute edge\n\
                  link l1 edge cloud\nlink l2 cloud edge\n\
                  place motor edge\n";
    assert!(text.ends_with(expect), "render: {text}");
}

/// REQ: REQ-TOOL-047 — a tree without a topology manifest builds an empty
/// topology slice (existing behavior intact).
#[test]
fn no_topology_builds_clean() {
    let m = fixture(build(&tree(&["n0"], None)), "build");
    assert!(m.computes.is_empty() && m.links.is_empty() && m.placement.is_empty());
    assert!(render_model(&m).starts_with("node n0\n"));
}
