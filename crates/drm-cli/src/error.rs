//! Failure taxonomy for the `drm` executable: each class maps to a process
//! exit code so callers (CI, scripts) can branch without parsing text.
//! REQ: REQ-TOOL-038 (refusal is a distinct failure), REQ-TOOL-080 (lockfile
//! defects are reported, never silently repaired).

use std::fmt;

/// A `drm` invocation failure with an associated exit code.
/// REQ: REQ-TOOL-038.
#[derive(Debug)]
pub enum CliError {
    /// Malformed invocation or an undeclared reference (node/topic).
    Usage(String),
    /// Filesystem or IO failure.
    Io(String),
    /// Committed lockfile could not be read or parsed.
    Lockfile(String),
    /// A node manifest could not be read, parsed, or written.
    Manifest(String),
    /// Operation refused pending explicit acknowledgement (I-H4).
    Refused(String),
    /// `drm verify` reported negatives; the change is not safe to land.
    Verify(String),
}

impl CliError {
    /// Stable process exit code for this failure class (non-zero always).
    /// REQ: REQ-TOOL-038.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Io(_)
            | Self::Lockfile(_)
            | Self::Manifest(_)
            | Self::Refused(_)
            | Self::Verify(_) => 1,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(m)
            | Self::Io(m)
            | Self::Lockfile(m)
            | Self::Manifest(m)
            | Self::Refused(m)
            | Self::Verify(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CliError {}
