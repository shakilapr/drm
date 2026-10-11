//! `drm info` — a read-only projection of the in-memory `WorkspaceModel` plus
//! the `drm verify` summary. It introduces no semantics of its own, performs no
//! mutation, no codegen, and no adapter invocation (architecture.md §Tiered
//! Developer Commands). Output is deterministic: sorted `BTree` iteration only.
//! REQ: REQ-TOOL-045, REQ-TOOL-047.

use std::path::Path;

use drm_workspace::orchestrator;
use drm_workspace::verify::verify_lite;

use crate::error::CliError;
use crate::ws;

/// Runs `drm info` against `root`, printing the model render and verify summary.
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// [`CliError::Io`] on discovery failure, [`CliError::Manifest`] on a
/// structurally invalid workspace.
pub fn run(root: &Path) -> Result<(), CliError> {
    let discovered = ws::discover(root)?;
    let model = ws::build_model(&discovered).map_err(|errs| {
        CliError::Manifest(format!(
            "workspace model invalid: {}",
            errs.iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ))
    })?;
    let findings = verify_lite(&model);
    print!("{}", orchestrator::info(&model, &findings));
    Ok(())
}
