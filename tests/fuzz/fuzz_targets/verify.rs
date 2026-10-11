//! Verify-fixture fuzz target (work-plan M18W1-E2-T2): malformed workspace
//! bytes — a topic schema (an `index.drm`-like payload), a node manifest, a
//! topology manifest, and the assembled `TreeInput` — must yield a model or a
//! diagnostic list, never a panic or hang. `parse_bytes`, both manifest
//! parsers, `model::build`, and `verify_lite` are all total over arbitrary
//! bytes. `-timeout=5` budgets the hang side.
//! REQ: REQ-QAL-004, REQ-QAL-007, REQ-QAL-009, REQ-TOOL-052, REQ-TOOL-053,
//! REQ-TOOL-070.

#![no_main]

use drm_workspace::model::{
    PackageInput, TreeInput, build, parse_node_manifest, parse_topology_manifest,
};
use drm_workspace::parser::parse_bytes;
use drm_workspace::verify::verify_lite;
use libfuzzer_sys::fuzz_target;

/// Feeds `data` through every workspace-parsing entry point that must never
/// panic. REQ: REQ-QAL-009, REQ-TOOL-053.
fn fixtures(data: &[u8]) {
    // Topic schema bytes (`index.drm` / `*.drm` payload) and the two TOML
    // manifest families (nodes, topology).
    let _ = parse_bytes(data);
    let _ = parse_node_manifest("n", data);
    let _ = parse_topology_manifest(data);

    // Assemble a full workspace tree from the same bytes and build it: every
    // defect is collected as a diagnostic, never propagated as a panic
    // (REQ-TOOL-070, I-D2).
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "p".to_string(),
            topics: vec![("t".to_string(), data.to_vec())],
        }],
        nodes: vec![("n".to_string(), data.to_vec())],
        topology: Some(data.to_vec()),
    };
    if let Ok(model) = build(&tree) {
        // verify_lite is deterministic and total (REQ-TOOL-064).
        let _ = verify_lite(&model);
    }
}

fuzz_target!(|data: &[u8]| {
    fixtures(data);
});
