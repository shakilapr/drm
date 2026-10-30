//! `drm deploy` — deployment identity and connection-config generation
//! (`docs/algorithms/deployment-identity.md` §Protocols).
//!
//! `drm build` emits the artifact because native adapters receive its path;
//! `drm deploy` is the command that owns *deployment identity*, so it reports
//! whether this deployment kept its identity and prints the slot table a process
//! will attach with.
//! REQ: REQ-TOOL-083, REQ-TOOL-084.

use std::path::Path;

use drm_core::deploy::DeploymentConfig;
use drm_workspace::deploy;

use crate::error::CliError;
use crate::{build, ids, ws};

/// Runs `drm deploy` against `root`.
///
/// Identity semantics (REQ-TOOL-084): the id is reused while the deployment
/// inputs (handle table, topic contracts, activation sizing, node fingerprints)
/// are unchanged, and minted fresh when they move.
///
/// # Errors
/// Build failure, an invalid workspace model, generation failure, or an artifact
/// that cannot be read back.
pub fn run(root: &Path) -> Result<(), CliError> {
    // Read the identity of the *previous* deployment before building: the build
    // refreshes the artifact, so comparing afterwards would always say "reused".
    let before = recorded_id(root);
    build::run(root, &[], false)?;
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
    let handles = ids::load(&root.join(ids::LOCKFILE_REL))?;
    deploy::generate(&model, &handles, root)
        .map_err(|e| CliError::Io(format!("deployment config: {e:?}")))?;

    let path = deploy::deployment_path(root);
    let config = DeploymentConfig::load(&path)
        .map_err(|e| CliError::Io(format!("read {}: {e:?}", path.display())))?;
    let identity = match before {
        Some(previous) if previous == config.deployment_id => "reused",
        _ => "minted",
    };
    println!("deploy: identity {} ({identity})", config.deployment_id);
    println!(
        "deploy: {} topic(s), {} node(s), {}",
        config.topics.len(),
        config.fingerprint_expectations.len(),
        path.display()
    );
    for (topic, slot) in config.slots() {
        println!("  {topic} -> slot {slot}");
    }
    Ok(())
}

/// The identity recorded by the existing artifact, when one loads.
fn recorded_id(root: &Path) -> Option<String> {
    DeploymentConfig::load(&drm_workspace::deploy::deployment_path(root))
        .ok()
        .map(|config| config.deployment_id)
}
