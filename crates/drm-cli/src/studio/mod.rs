//! `drm studio --stdio` host: capability handshake, typed JSON-RPC dispatch,
//! revisioned optimistic concurrency, journaled `ChangeSet` commits,
//! conditional undo, and format-preserving manifest editing. The host is the
//! `drm` executable in long-lived stdio mode (no HTTP server, no ports) and
//! holds no DRM semantics of its own — every mutation routes through the
//! `drm-workspace` engine (`REQ-STUDIO-003`, `REQ-STUDIO-011`).
//!
//! Module layout:
//! - [`protocol`]: wire types + bounded parsing (I-ST13).
//! - [`host`]: [`host::StudioHost`] state machine + [`host::serve_stdio`].
//! - [`journal`]: journaled multi-file commit + crash recovery (I-ST8/I-ST12).
//! - [`edit`]: `toml_edit`-based format-preserving manifest mutation (I-ST6).
//! - [`supervisor`]: run sessions owned independently of the UI (I-ST10).
//!
//! REQ: REQ-STUDIO-001, REQ-STUDIO-003, REQ-STUDIO-007, REQ-STUDIO-009,
//! REQ-STUDIO-010, REQ-STUDIO-011, REQ-STUDIO-013, REQ-STUDIO-014.

pub mod edit;
pub mod host;
pub mod journal;
pub mod protocol;
pub mod supervisor;

use std::io;
use std::path::Path;

use crate::error::CliError;

pub use host::{StudioHost, serve_stdio};
pub use protocol::{
    Capability, Hello, HelloAck, ModelCommand, NodeSpec, PackageSpec, StudioRequest,
    StudioResponse, WorkspaceSnapshot,
};
pub use supervisor::{
    CloseChoice, CloseDecision, CommandRunner, NodeHandle, ProcessRunner, ProcessStatus,
    RunSession, RunSessionView, SessionStatus, Supervisor,
};

/// Runs the `drm studio --stdio` host over stdin/stdout. REQ: REQ-STUDIO-011.
///
/// # Errors
/// [`CliError::Io`] on a transport failure or a mandatory-handshake rejection.
pub fn run(root: &Path, trusted: bool) -> Result<(), CliError> {
    let mut host = StudioHost::new(root.to_path_buf(), trusted);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rx = stdin.lock();
    let mut tx = stdout.lock();
    let result = serve_stdio(&mut host, &mut rx, &mut tx).map_err(|e| CliError::Io(e.to_string()));
    // Renderer disconnected: run sessions persist per their `leave_running`
    // policy — the supervisor is never killed with the UI (I-ST10).
    host.on_disconnect();
    result
}

#[cfg(test)]
mod tests;
