//! Read-only attachment for introspection tools (`drm echo`/`hz`/`log`).
//!
//! Tools resolve topic identities from the generated deployment metadata
//! (`.drm/deploy/deployment.json`) and attach as non-publishing observers
//! (`docs/architecture.md` §Tooling and Introspection; observability-plane.md
//! I-OB5). A stale/absent namespace is a `NotFound`, never a silent empty view.
//! REQ: REQ-OPS-003.

use std::path::Path;

use drm_core::channel::Channel;

use crate::error::CliError;

/// Resolves a canonical topic path to its deployment handle.
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::NotFound`] when the deployment metadata or topic is absent;
/// [`CliError::Io`] when the metadata cannot be parsed.
pub fn topic_handle(workspace: &Path, topic: &str) -> Result<u32, CliError> {
    let path = drm_workspace::deploy::deployment_path(workspace);
    let text = std::fs::read_to_string(&path).map_err(|_| {
        CliError::NotFound(format!(
            "no deployment metadata at {} (run `drm build`)",
            path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CliError::Io(format!("parse {}: {e}", path.display())))?;
    let handle = value
        .get("handles")
        .and_then(|h| h.get(topic))
        .and_then(serde_json::Value::as_u64);
    match handle {
        Some(h) => u32::try_from(h)
            .map_err(|_| CliError::NotFound(format!("topic `{topic}` handle out of range"))),
        None => Err(CliError::NotFound(format!(
            "topic `{topic}` not present in deployment metadata"
        ))),
    }
}

/// Opens a live read-only channel for `topic`.
///
/// Live attach is performed by the out-of-band diag session (Phase 32); until a
/// running deployment is present the observer reports `NotFound`. This keeps
/// `drm echo`/`hz`/`log` read-only and non-publishing (`I-OB5`).
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::NotFound`] when no live publisher is attached to `topic`.
pub fn open_channel(workspace: &Path, topic: &str) -> Result<Channel, CliError> {
    let _handle = topic_handle(workspace, topic)?;
    Err(CliError::NotFound(format!(
        "no live publisher for topic `{topic}`; the out-of-band diag session attaches at runtime"
    )))
}
