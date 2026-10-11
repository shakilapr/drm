//! Deployment-artifact generation ([build-orchestration.md](../../docs/algorithms/toolchain/build-orchestration.md)
//! `deploy::generate`): emits the deterministic handle table consumed at
//! activation. Deployment artifacts may embed runtime numeric handles; generated
//! bindings never do (codegen I-G9). This slice writes the handle table to
//! `.drm/deploy/deployment.json`; routing/runtime tables land with the router
//! phases. REQ: REQ-TOOL-012.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use drm_types::Status;

use crate::canon::json_escape;
use crate::model::WorkspaceModel;
use crate::model::handles::HandleTable;
use crate::orchestrator::DEPLOY_DIR_REL;

/// Deployment-artifact file name under `.drm/deploy`. REQ: REQ-TOOL-012.
pub const DEPLOYMENT_FILE: &str = "deployment.json";

/// Deployment artifact `format_version`. REQ: REQ-TOOL-012.
pub const FORMAT_VERSION: u64 = 1;

/// Path of the deployment artifact for a workspace root.
/// REQ: REQ-TOOL-012.
#[must_use]
pub fn deployment_path(root: &Path) -> PathBuf {
    root.join(DEPLOY_DIR_REL).join(DEPLOYMENT_FILE)
}

/// Writes the deterministic deployment artifact for `model` and `handles`,
/// returning the number of handle entries emitted. Output is sorted by canonical
/// topic path and carries no timestamps, so identical inputs render byte-identical
/// bytes. REQ: REQ-TOOL-012.
///
/// # Errors
/// [`Status::InvalidConfig`] when the directory or file cannot be written.
pub fn generate(
    model: &WorkspaceModel,
    handles: &HandleTable,
    root: &Path,
) -> Result<usize, Status> {
    let path = deployment_path(root);
    let Some(parent) = path.parent() else {
        return Err(Status::InvalidConfig);
    };
    fs::create_dir_all(parent).map_err(|_| Status::InvalidConfig)?;
    let body = render(model, handles);
    fs::write(&path, body.as_bytes()).map_err(|_| Status::InvalidConfig)?;
    Ok(model.topics.len())
}

/// Canonical deployment render: `format_version` then the sorted handle table.
/// REQ: REQ-TOOL-012.
#[must_use]
pub fn render(model: &WorkspaceModel, handles: &HandleTable) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"format_version\": {FORMAT_VERSION},");
    out.push_str("  \"handles\": ");
    if model.topics.is_empty() {
        out.push_str("{}\n");
    } else {
        let entries: Vec<String> = model
            .topics
            .keys()
            .map(|topic| {
                let handle = handles.assignments.get(topic).copied().unwrap_or(0);
                format!("    \"{}\": {handle}", json_escape(topic))
            })
            .collect();
        out.push_str("{\n");
        out.push_str(&entries.join(",\n"));
        out.push_str("\n  }\n");
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FORMAT_VERSION, render};
    use crate::model::handles::HandleTable;
    use crate::model::{PackageInput, TreeInput, build};

    /// REQ: REQ-TOOL-012 — the deployment render is deterministic and sorted.
    #[test]
    fn render_is_deterministic_and_sorted() {
        let tree = TreeInput {
            packages: vec![PackageInput {
                name: "pkg".to_string(),
                topics: vec![
                    ("two".to_string(), b"v: u32\n".to_vec()),
                    ("one".to_string(), b"v: u32\n".to_vec()),
                ],
            }],
            nodes: Vec::new(),
            topology: None,
        };
        let model = build(&tree).unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut handles = HandleTable {
            next_free: 2,
            ..HandleTable::default()
        };
        handles.assignments = BTreeMap::from([
            ("pkg/one".to_string(), 0_u32),
            ("pkg/two".to_string(), 1_u32),
        ]);
        let body = render(&model, &handles);
        assert!(body.contains(&format!("\"format_version\": {FORMAT_VERSION}")));
        let one = body
            .find("pkg/one")
            .unwrap_or_else(|| unreachable!("pkg/one"));
        let two = body
            .find("pkg/two")
            .unwrap_or_else(|| unreachable!("pkg/two"));
        assert!(one < two, "sorted: {body}");
        assert_eq!(render(&model, &handles), body);
    }
}
