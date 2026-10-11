//! `drm build` — the orchestrator facade: discover declarations, run the
//! `drm-workspace` orchestration sequence (I-BO1), persist the stable handle
//! table, and report. DRM never parses application sources; native adapters own
//! object freshness (I-BO2). REQ: REQ-TOOL-012, REQ-TOOL-011, REQ-TOOL-014.

use std::collections::BTreeSet;
use std::path::Path;

use drm_workspace::model::handles::HandleTable;
use drm_workspace::orchestrator::{self, BuildError, BuildRequest, BuildStep, SpawnRunner};

use crate::error::CliError;
use crate::ids;

/// Runs `drm build` against `root`, optionally restricted to `targets`.
/// REQ: REQ-TOOL-012.
///
/// # Errors
/// [`CliError`] mapped from the orchestration failure class.
pub fn run(root: &Path, targets: &[String]) -> Result<(), CliError> {
    let request = BuildRequest {
        workspace: root.to_path_buf(),
        targets: if targets.is_empty() {
            None
        } else {
            Some(targets.iter().cloned().collect::<BTreeSet<_>>())
        },
    };
    let prior = load_prior(root)?;
    let mut runner = SpawnRunner;
    let mut steps: Vec<BuildStep> = Vec::new();
    let report = orchestrator::build(&request, &prior, &mut runner, &mut |step| steps.push(step))
        .map_err(map_error)?;

    // Normal builds preserve assigned handles (REQ-TOOL-036): persist the table.
    ids::write(&root.join(ids::LOCKFILE_REL), &report.handles)?;

    println!(
        "build: {} generated ({} rendered), {} verified, {} built [{}]",
        report.generated,
        report.dirty,
        report.verified,
        report.built.len(),
        report.built.join(", ")
    );
    println!("steps: {}", steps.len());
    Ok(())
}

/// Loads the committed handle table, or an empty table when no lockfile exists.
/// REQ: REQ-TOOL-036.
///
/// # Errors
/// [`CliError::Lockfile`] when a present lockfile is malformed.
fn load_prior(root: &Path) -> Result<HandleTable, CliError> {
    let path = root.join(ids::LOCKFILE_REL);
    if path.is_file() {
        ids::load(&path)
    } else {
        Ok(HandleTable::default())
    }
}

/// Maps an orchestration failure to the CLI taxonomy.
/// REQ: REQ-TOOL-012.
fn map_error(err: BuildError) -> CliError {
    match err {
        BuildError::Io(m) => CliError::Io(m),
        BuildError::Model(errs) => {
            let detail = errs
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            CliError::Manifest(format!("workspace model invalid: {detail}"))
        }
        BuildError::Verify(findings) => {
            let detail = findings
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            CliError::Verify(detail)
        }
        BuildError::Adapter { node, message } => {
            CliError::Io(format!("adapter for node `{node}`: {message}"))
        }
        BuildError::Handle(s) | BuildError::Deploy(s) | BuildError::Codegen(s) => {
            CliError::Io(format!("build step failed ({s:?})"))
        }
    }
}
