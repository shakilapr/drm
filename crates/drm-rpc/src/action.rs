//! Action workflow execution (`docs/algorithms/ops/rpc-actions.md` §Data
//! Structures `ActionContext`/`Phase`, §Functions `run_action`). A long-running
//! action is a declared sequence of [`Phase`]s executed one per reactor worker
//! step, so the reactor observes cancellation between phases (`I-RPC2`): no new
//! phase starts after the token is observed and the terminal outcome is then
//! `Cancelled`. `expected_max_duration_ns` is SLO metadata only — declared,
//! never enforced; middleware cannot preempt user code (`I-RPC8`). The action
//! state is owned and initialized from the owned request bytes, so no phase ever
//! retains a channel borrow.
//! REQ: REQ-OPS-010, REQ-OPS-011.

use std::sync::atomic::{AtomicBool, Ordering};

use drm_types::Status;

use crate::Outcome;

/// Owned, opaque per-action state threaded through every phase
/// (`rpc-actions.md` §Data Structures `OwnedActionState`).
/// REQ: REQ-OPS-010.
pub struct ActionState {
    scratch: Vec<u8>,
    status: Status,
}

impl ActionState {
    /// Builds state from the owned request bytes.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn new(scratch: Vec<u8>) -> Self {
        Self {
            scratch,
            status: Status::Ok,
        }
    }

    /// Immutable view of the action's working bytes.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn scratch(&self) -> &[u8] {
        &self.scratch
    }

    /// Mutable working bytes for user phase code.
    /// REQ: REQ-OPS-010.
    pub fn scratch_mut(&mut self) -> &mut Vec<u8> {
        &mut self.scratch
    }

    /// Status recorded by the most recently executed phase.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// Records the status returned by a phase.
    /// REQ: REQ-OPS-010.
    pub fn set_status(&mut self, status: Status) {
        self.status = status;
    }
}

/// Cooperative cancellation context passed to every phase
/// (`rpc-actions.md` §Data Structures `ActionContext`). The token is the same
/// one the reactor sets when it applies a cancel, so a phase observes it at its
/// next checkpoint (`I-RPC2`).
/// REQ: REQ-OPS-010, REQ-OPS-011.
pub struct ActionContext<'a> {
    cancel_requested: &'a AtomicBool,
}

impl<'a> ActionContext<'a> {
    /// Wraps the reactor-owned cancellation token.
    /// REQ: REQ-OPS-011.
    #[must_use]
    pub fn new(cancel_requested: &'a AtomicBool) -> Self {
        Self { cancel_requested }
    }

    /// Whether cancellation has been requested (`I-RPC2`).
    /// REQ: REQ-OPS-011.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancel_requested.load(Ordering::Acquire)
    }

    /// Cooperative checkpoint: `true` when execution may continue, `false` once
    /// cancellation was requested (`I-RPC2`).
    /// REQ: REQ-OPS-011.
    #[must_use]
    pub fn checkpoint(&self) -> bool {
        !self.cancelled()
    }
}

/// One declared action phase (`rpc-actions.md` §Data Structures `Phase`).
/// `expected_max_duration_ns` is SLO metadata — declared, not enforced
/// (`I-RPC8`).
/// REQ: REQ-OPS-010.
#[derive(Clone, Copy)]
pub struct Phase {
    /// Declared SLO bound in nanoseconds (metadata only; never enforced).
    pub expected_max_duration_ns: u64,
    /// Executes the phase and returns its status. May call
    /// [`ActionContext::checkpoint`] for intra-phase responsiveness.
    pub execute: fn(&mut ActionState, &ActionContext) -> Status,
}

/// Outcome of one [`ActionExec::step`].
/// REQ: REQ-OPS-010, REQ-OPS-011.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStep {
    /// A phase executed; more phases remain (emit `Progress`).
    Progress,
    /// The action completed naturally.
    Done(Outcome, Status),
    /// Cancellation was observed at a phase boundary (`I-RPC2`).
    Cancelled,
}

/// Drives one action's declared phases across reactor worker steps.
/// REQ: REQ-OPS-010, REQ-OPS-011.
pub struct ActionExec {
    phases: &'static [Phase],
    state: ActionState,
    next_phase: usize,
    started: bool,
}

impl ActionExec {
    /// Builds an execution over `phases` and the owned request bytes.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn new(phases: &'static [Phase], input: Vec<u8>) -> Self {
        Self {
            phases,
            state: ActionState::new(input),
            next_phase: 0,
            started: false,
        }
    }

    /// Whether the `Started` lifecycle event was already emitted.
    /// REQ: REQ-OPS-025.
    #[must_use]
    pub fn started(&self) -> bool {
        self.started
    }

    /// Marks the `Started` lifecycle event as emitted.
    /// REQ: REQ-OPS-025.
    pub fn mark_started(&mut self) {
        self.started = true;
    }

    /// Number of phases already executed.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn phase_index(&self) -> usize {
        self.next_phase
    }

    /// The action's owned state.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn state(&self) -> &ActionState {
        &self.state
    }

    /// Executes at most one phase. Cancellation is checked before the phase and
    /// again after it, so no new phase starts once the token is observed
    /// (`I-RPC2`).
    /// REQ: REQ-OPS-010, REQ-OPS-011.
    pub fn step(&mut self, ctx: &ActionContext) -> ActionStep {
        if ctx.cancelled() {
            return ActionStep::Cancelled;
        }
        let Some(phase) = self.phases.get(self.next_phase) else {
            let status = self.state.status();
            return ActionStep::Done(outcome_from(status), status);
        };
        let status = (phase.execute)(&mut self.state, ctx);
        self.state.set_status(status);
        self.next_phase += 1;
        if ctx.cancelled() {
            return ActionStep::Cancelled;
        }
        if self.next_phase >= self.phases.len() {
            ActionStep::Done(outcome_from(status), status)
        } else {
            ActionStep::Progress
        }
    }
}

/// Maps a phase status to the terminal outcome (`rpc-actions.md` §Data
/// Structures `Outcome`).
/// REQ: REQ-OPS-010.
#[must_use]
pub(crate) fn outcome_from(status: Status) -> Outcome {
    if status == Status::Ok {
        Outcome::Succeeded
    } else {
        Outcome::Aborted
    }
}
