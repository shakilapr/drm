//! `drm-rpc` — request/response services and long-running, cancellable action
//! workflows built as a clean library layer over correlated topic pairs
//! (`docs/algorithms/ops/rpc-actions.md`, `docs/architecture.md` §drm-rpc).
//!
//! Each client owns `request/<client_id>` and `cancel/<client_id>` and consumes
//! the server's single ordered `event/<client_id>` stream. The server
//! deterministically multiplexes requests, admits work by reserving an
//! operation slot before it consumes a request (`I-RPC7`), and publishes only
//! through bounded outboxes (`I-RPC6`/`I-RPC7`). RPC adds zero new core ring
//! semantics (`I-RPC4`): it composes [`drm_core::channel::Channel`] and nothing
//! else.
//!
//! Layer `ops` (codebase.md 2.1): depends on `drm-types` + `drm-core` only,
//! never on another ops crate. `#![forbid(unsafe_code)]`.
//!
//! M16W2 scope adds the action phase machine with phase-boundary cancellation
//! (`I-RPC2`): a long-running action executes one declared [`action::Phase`]
//! per reactor worker step and stops at the next boundary once the cancel token
//! is observed, with `Terminal(Cancelled)` following. `expected_max_duration_ns`
//! is SLO metadata only (`I-RPC8`). The unary request/response slice
//! (`call`/`serve`), cid dispatch (`I-RPC1`), relative-budget deadlines
//! (`I-RPC3`), and server-session identity (`I-RPC9`) landed in M16W1.
//! REQ: REQ-OPS-010, REQ-OPS-011, REQ-OPS-012, REQ-OPS-013, REQ-OPS-020,
//! REQ-OPS-021, REQ-OPS-022, REQ-OPS-023, REQ-OPS-024, REQ-OPS-025.

#![forbid(unsafe_code)]

pub mod action;
pub mod client;
pub mod codec;
pub mod server;

pub use action::{ActionContext, ActionExec, ActionState, ActionStep, Phase};

use drm_types::Status;

/// Correlation identity (`rpc-actions.md` §Data Structures
/// `CorrelationId`). `{client_id, session_nonce, counter}` is unique until
/// session close and echoed verbatim on every event, so concurrent responses
/// never cross-correlate.
/// REQ: REQ-OPS-012, REQ-OPS-013, REQ-OPS-025.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CorrelationId {
    /// Client identity owning the correlated topic pair.
    pub client_id: u32,
    /// Client session nonce; changes on client restart.
    pub session_nonce: u64,
    /// Monotonic per-client-session counter; never reused, never wrapped.
    pub counter: u64,
}

/// Terminal operation outcome (`rpc-actions.md` §Data Structures
/// `Outcome`).
/// REQ: REQ-OPS-010.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Handler completed with `Status::Ok`.
    Succeeded,
    /// Handler reported a non-success status.
    Aborted,
    /// Cancellation was observed before the terminal.
    Cancelled,
}

/// Event discriminator for one `event/<client_id>` message
/// (`rpc-actions.md` §Data Structures `EventKind`).
/// REQ: REQ-OPS-025.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Operation admitted; carries the server session echo (`I-RPC9`).
    Accepted,
    /// Work started.
    Started,
    /// Advisory progress update (replaceable; `I-RPC7`).
    Progress,
    /// Cancellation observed and committed (`I-RPC2`).
    CancelAccepted,
    /// Cancellation refused for a non-cancellable phase (`I-RPC2`).
    CancelRejected,
    /// Terminal event; the operation is complete.
    Terminal(Outcome),
}

/// RPC-layer error (`rpc-actions.md` §Edge Cases & Errors). Maps the core
/// [`Status`] taxonomy plus RPC-specific conditions; `call` never panics.
/// REQ: REQ-OPS-011, REQ-OPS-020, REQ-OPS-021, REQ-OPS-023, REQ-OPS-025.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// Underlying channel/status error.
    Status(Status),
    /// Client deadline elapsed (`RpcTimeout`); a best-effort `Cancel` follows.
    Timeout,
    /// Admission or correlation capacity is full (`RpcBusy`).
    Busy,
    /// The event-topic epoch changed; open calls fail `RpcPeerRestarted`.
    PeerRestarted,
    /// The correlation counter space is exhausted; the session closes.
    SequenceExhausted,
    /// An event arrived out of the per-cid order (`I-RPC10`).
    InvalidOrder,
    /// The server returned a non-success terminal outcome.
    Remote {
        /// Terminal outcome reported by the server.
        outcome: Outcome,
        /// Terminal status reported by the server.
        status: Status,
    },
}

impl From<Status> for RpcError {
    /// REQ: REQ-OPS-021, REQ-OPS-023 — `QueueFull`/`Busy` surface as
    /// [`RpcError::Busy`]; counter exhaustion as [`RpcError::SequenceExhausted`].
    fn from(status: Status) -> Self {
        match status {
            Status::QueueFull | Status::Busy => Self::Busy,
            Status::SequenceExhausted => Self::SequenceExhausted,
            other => Self::Status(other),
        }
    }
}

impl RpcError {
    /// The mapped core [`Status`], or `Status::Ok` for RPC-only conditions.
    /// REQ: REQ-OPS-021, REQ-OPS-023.
    #[must_use]
    pub const fn status(&self) -> Status {
        match self {
            Self::Status(s) | Self::Remote { status: s, .. } => *s,
            Self::Timeout
            | Self::Busy
            | Self::PeerRestarted
            | Self::SequenceExhausted
            | Self::InvalidOrder => Status::Ok,
        }
    }
}

#[cfg(test)]
mod tests;
