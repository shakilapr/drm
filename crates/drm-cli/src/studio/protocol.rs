//! Studio wire protocol: typed request/response enums, capability handshake,
//! bounded line parsing. JSON-RPC carries commands and state only — topic
//! sample bytes never travel here (I-ST3, `REQ-STUDIO-013`).
//!
//! REQ: REQ-STUDIO-007 (capability handshake), REQ-STUDIO-013 (plane
//! separation), REQ-STUDIO-014 (typed command set).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum bytes accepted for a single control-plane JSON line (I-ST13);
/// longer input is rejected before any allocation-heavy parse.
/// REQ: REQ-STUDIO-013.
pub const MAX_CONTROL_BYTES: usize = 1 << 20;

/// Protocol version advertised in the handshake (capability-driven, never an
/// exact-version requirement). REQ: REQ-STUDIO-007.
pub const PROTOCOL_VERSION: u32 = 1;

/// Host version string advertised in `HelloAck`. REQ: REQ-STUDIO-007.
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A renderer capability the host declares in the handshake; the frontend
/// hides features whose capability is absent (ui-arch §Protocol Versioning).
/// REQ: REQ-STUDIO-001, REQ-STUDIO-007.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Workspace snapshot/mutation surface (v2 schema).
    #[serde(rename = "workspace.v2")]
    WorkspaceV2,
    /// Web extension sandbox.
    #[serde(rename = "extensions.web")]
    ExtensionsWeb,
    /// Binary framed data streams for topic samples/video.
    #[serde(rename = "binaryStreams.v1")]
    BinaryStreamsV1,
    /// Runtime statistics telemetry channel.
    #[serde(rename = "runtime.stats")]
    RuntimeStats,
    /// Workspace trust (enables Build/Run/extensions).
    #[serde(rename = "trust")]
    Trust,
}

/// The capabilities a host advertises given its trust state.
/// REQ: REQ-STUDIO-001, REQ-STUDIO-012.
#[must_use]
pub fn capabilities(trusted: bool) -> Vec<Capability> {
    let mut caps = vec![
        Capability::WorkspaceV2,
        Capability::BinaryStreamsV1,
        Capability::RuntimeStats,
    ];
    if trusted {
        caps.push(Capability::ExtensionsWeb);
        caps.push(Capability::Trust);
    }
    caps
}

/// First request from a renderer: protocol + client versions only.
/// REQ: REQ-STUDIO-007.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Client's protocol version.
    pub protocol_version: u32,
    /// Client's product version string.
    pub client_version: String,
}

/// Host reply to [`Hello`]: negotiated version + declared capabilities.
/// REQ: REQ-STUDIO-007.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    /// Host protocol version.
    pub protocol_version: u32,
    /// Host product version string.
    pub host_version: String,
    /// Capabilities the renderer may gate features on.
    pub capabilities: Vec<Capability>,
}

/// A single verify finding rendered for the renderer.
/// REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity label (`error`/`warning`).
    pub severity: String,
    /// Human-readable message.
    pub message: String,
}

/// Revisioned workspace snapshot: monotonic `revision` + canonical model render
/// + diagnostics. REQ: REQ-STUDIO-009, REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    /// Monotonic workspace revision.
    pub revision: u64,
    /// Canonical model render.
    pub model: String,
    /// Problems observed by the engine verifier.
    pub problems: Vec<Diagnostic>,
}

/// Verify result: green/red plus findings. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifySummary {
    /// Whether the workspace verified with no findings.
    pub green: bool,
    /// Rendered findings, one clause each.
    pub findings: Vec<String>,
}

/// Package creation/update payload. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    /// Package name.
    pub name: String,
    /// Optional package location.
    pub path: Option<String>,
}

/// Node creation/update payload. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Node name.
    pub name: String,
    /// Topics the node publishes.
    #[serde(default)]
    pub publishes: Vec<String>,
    /// Topics the node consumes.
    #[serde(default)]
    pub consumes: Vec<String>,
}

/// Topic creation/update payload. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicSpec {
    /// Canonical topic path.
    pub path: String,
    /// Declared field count.
    #[serde(default)]
    pub fields: u32,
}

/// Compute creation/update payload. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeSpec {
    /// Compute unit name.
    pub name: String,
    /// Platform label.
    pub platform: Option<String>,
}

/// Link creation/update payload. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSpec {
    /// Source compute.
    pub from: String,
    /// Destination compute.
    pub to: String,
    /// Transport label.
    pub transport: Option<String>,
}

/// Run-session configuration; `leave_running` is the disconnect policy.
/// REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfig {
    /// Nodes to start.
    pub nodes: Vec<String>,
    /// Whether a disconnect leaves nodes running.
    pub leave_running: bool,
}

/// The typed ui-arch command set. REQ: REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ModelCommand {
    /// Create a package.
    PackageCreate(PackageSpec),
    /// Rename a package.
    PackageRename {
        /// Source package name.
        from: String,
        /// Destination package name.
        to: String,
    },
    /// Delete a package.
    PackageDelete {
        /// Package name.
        name: String,
    },
    /// Create a node.
    NodeCreate(NodeSpec),
    /// Update a node's interface (format-preserving manifest edit).
    NodeUpdate(NodeSpec),
    /// Delete a node.
    NodeDelete {
        /// Node name.
        name: String,
    },
    /// Create a topic.
    TopicCreate(TopicSpec),
    /// Update a topic.
    TopicUpdate(TopicSpec),
    /// Rename a topic.
    TopicRename {
        /// Source topic path.
        from: String,
        /// Destination topic path.
        to: String,
    },
    /// Delete a topic.
    TopicDelete {
        /// Topic path.
        path: String,
    },
    /// Create a compute unit.
    ComputeCreate(ComputeSpec),
    /// Update a compute unit.
    ComputeUpdate(ComputeSpec),
    /// Create a link.
    LinkCreate(LinkSpec),
    /// Update a link.
    LinkUpdate(LinkSpec),
}

/// A renderer request. Internally tagged by `method` (JSON-RPC method name).
/// REQ: REQ-STUDIO-007, REQ-STUDIO-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum StudioRequest {
    /// Mandatory first request.
    Hello(Hello),
    /// Open a workspace root and build its snapshot.
    OpenWorkspace {
        /// Workspace root path.
        path: String,
    },
    /// Request the current revisioned snapshot.
    Snapshot,
    /// Run the engine verifier.
    Verify,
    /// Apply a model command guarded by `base_revision`.
    Mutate {
        /// Revision the renderer based the edit on.
        base_revision: u64,
        /// Domain command.
        command: ModelCommand,
    },
    /// Start a build (trust-gated).
    BuildStart {
        /// Optional target node.
        target: Option<String>,
    },
    /// Cancel the in-flight build.
    BuildCancel,
    /// Start a run session (trust-gated).
    RunStart {
        /// Session configuration.
        config: RunConfig,
    },
    /// Stop a run session.
    RunStop {
        /// Session id.
        session: String,
    },
    /// Restart a run session.
    RunRestart {
        /// Session id.
        session: String,
    },
    /// Conditionally undo a journaled `ChangeSet`.
    Undo {
        /// Target `ChangeSet` id.
        changeset: String,
        /// Force revert despite external changes (requires trust).
        force: bool,
    },
    /// Request a binary-plane token; sample bytes never travel JSON (I-ST3).
    SubscribeTelemetry {
        /// Requested telemetry channels.
        channels: Vec<String>,
    },
}

/// A host reply. Internally tagged by `result`. REQ: REQ-STUDIO-007.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum StudioResponse {
    /// Handshake acknowledgement.
    HelloAck(HelloAck),
    /// Revisioned snapshot.
    Snapshot(WorkspaceSnapshot),
    /// Verify summary.
    Verify(VerifySummary),
    /// Successful mutation: new revision + canonical render + the journaled
    /// `ChangeSet` id the renderer passes back to `undo`.
    MutationResult {
        /// New workspace revision.
        revision: u64,
        /// Canonical model render.
        render: String,
        /// Journaled `ChangeSet` id.
        changeset: String,
    },
    /// Stale `base_revision`; UI offers Review/Reload/Retry (I-ST7).
    Conflict {
        /// Current revision.
        revision: u64,
    },
    /// Undo blocked by an external change (I-ST9); UI offers Compare/Cancel/Force.
    ConflictExternalChanged {
        /// `ChangeSet` id.
        changeset: String,
    },
    /// Build accepted (headless completion is supervisor-owned).
    BuildAccepted {
        /// Revision the build was accepted at.
        revision: u64,
    },
    /// Run session accepted; the session outlives the UI (I-ST10).
    RunAccepted {
        /// Session id.
        session: String,
    },
    /// Binary-plane token (I-ST3).
    TelemetryToken {
        /// Opaque stream token.
        token: String,
    },
    /// Structured error reply.
    Error {
        /// Stable error class.
        code: String,
        /// Human-readable message.
        message: String,
    },
}

/// A studio-host failure with a stable class name.
/// REQ: REQ-STUDIO-003, REQ-STUDIO-007.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudioError {
    /// Input exceeded the bounded control frame (I-ST13).
    Bounds(String),
    /// Malformed JSON-RPC.
    Parse(String),
    /// First request was not `Hello` (handshake mandatory).
    Handshake(String),
    /// Operation refused because the workspace is untrusted (I-ST11).
    Untrusted(String),
    /// Requested object is absent.
    Unknown(String),
    /// Command not implemented in this slice.
    Unsupported(String),
    /// Post-edit verify failed; no partial apply.
    Verify(String),
    /// Filesystem failure.
    Io(String),
    /// Journaled commit/recovery failure.
    Journal(String),
}

impl StudioError {
    /// Stable machine-readable error class. REQ: REQ-STUDIO-007.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Bounds(_) => "bounds",
            Self::Parse(_) => "parse",
            Self::Handshake(_) => "handshake",
            Self::Untrusted(_) => "untrusted",
            Self::Unknown(_) => "unknown",
            Self::Unsupported(_) => "unsupported",
            Self::Verify(_) => "verify",
            Self::Io(_) => "io",
            Self::Journal(_) => "journal",
        }
    }

    /// Renders this failure as a wire [`StudioResponse::Error`].
    /// REQ: REQ-STUDIO-007.
    #[must_use]
    pub fn to_response(&self) -> StudioResponse {
        StudioResponse::Error {
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }
}

impl fmt::Display for StudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bounds(m)
            | Self::Parse(m)
            | Self::Handshake(m)
            | Self::Untrusted(m)
            | Self::Unknown(m)
            | Self::Unsupported(m)
            | Self::Verify(m)
            | Self::Io(m)
            | Self::Journal(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for StudioError {}

/// Parses one bounded control line into a typed request.
/// REQ: REQ-STUDIO-007, REQ-STUDIO-013.
///
/// # Errors
/// [`StudioError::Bounds`] when `line` exceeds [`MAX_CONTROL_BYTES`];
/// [`StudioError::Parse`] when the JSON does not decode to a [`StudioRequest`].
pub fn parse_bounded(line: &str) -> Result<StudioRequest, StudioError> {
    if line.len() > MAX_CONTROL_BYTES {
        return Err(StudioError::Bounds(format!(
            "control frame {} bytes exceeds limit {MAX_CONTROL_BYTES}",
            line.len()
        )));
    }
    serde_json::from_str(line).map_err(|e| StudioError::Parse(format!("invalid request: {e}")))
}

/// Encodes a response as a single JSON line (no trailing newline).
/// REQ: REQ-STUDIO-007.
///
/// # Errors
/// [`StudioError::Parse`] when serialization fails (never for these types).
pub fn to_json_line<T: Serialize>(value: &T) -> Result<String, StudioError> {
    serde_json::to_string(value).map_err(|e| StudioError::Parse(format!("encode: {e}")))
}
