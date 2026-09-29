//! History overflow policies (docs/algorithms/runtime/overflow-policies.md).
//! REQ: REQ-CORE-039 (overwrite exact gaps), REQ-CORE-040 (reject gating, never
//! blocks), REQ-CORE-060 (required registry), REQ-CORE-059 (ack watermark init 0),
//! REQ-CORE-001 (ack wiring), REQ-CORE-024 (explicit errors).

use std::sync::atomic::{AtomicU64, Ordering};

use crate::channel::ccb::ChannelControlBlock;
use drm_types::Status;

/// Topic-level history overflow policy.
/// REQ: REQ-CORE-039.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Ring rolls unconditionally; lapping produces exact gaps.
    Overwrite,
    /// Required sequential consumers gate retirement with `QueueFull`.
    Reject,
}

impl OverflowPolicy {
    /// Decode from the descriptor byte.
    /// REQ: REQ-CORE-015.
    ///
    /// # Errors
    /// [`Status::InvalidData`] for any byte outside {0, 1}.
    pub fn from_byte(b: u8) -> Result<Self, Status> {
        match b {
            0 => Ok(Self::Overwrite),
            1 => Ok(Self::Reject),
            _ => Err(Status::InvalidData),
        }
    }

    /// Descriptor byte encoding.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Overwrite => 0,
            Self::Reject => 1,
        }
    }
}

/// Required sequential consumer's shared acknowledgement watermark.
/// REQ: REQ-CORE-059. Init 0 — "acknowledged nothing"; sequence 0 is representable.
#[derive(Debug)]
pub struct RequiredSequential {
    /// First sequence not yet acknowledged.
    pub ack_seq: AtomicU64,
}

impl Default for RequiredSequential {
    fn default() -> Self {
        Self::new()
    }
}

impl RequiredSequential {
    /// New record with the watermark at 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ack_seq: AtomicU64::new(0),
        }
    }

    /// REQ: REQ-CORE-040 — publish may retire `retire_seq` iff strictly past it (I-OV3).
    #[must_use]
    pub fn may_retire(&self, retire_seq: u64) -> bool {
        self.ack_seq.load(Ordering::Acquire) > retire_seq
    }

    /// REQ: REQ-CORE-001 — monotonic acknowledge after read completion.
    ///
    /// # Errors
    /// [`Status::SequenceExhausted`] when `seq` has no successor.
    pub fn ack_completed(&self, seq: u64) -> Result<(), Status> {
        let next = seq.checked_add(1).ok_or(Status::SequenceExhausted)?;
        self.ack_seq.fetch_max(next, Ordering::Release);
        Ok(())
    }
}

/// REQ: REQ-CORE-040, REQ-CORE-024 — gate the commit: `reject` topics retire only
/// when every required consumer has acknowledged past the retiring sequence.
/// Zero required consumers under `Reject` is a runtime `InvalidConfig` (I-OV5).
/// # Errors
/// [`Status::QueueFull`] when a required consumer still needs the retiring sequence;
/// [`Status::InvalidConfig`] for the zero-required invariant violation.
pub fn allow_commit(
    policy: OverflowPolicy,
    ccb: &ChannelControlBlock,
    required: &[RequiredSequential],
    depth: u64,
) -> Result<(), Status> {
    if policy != OverflowPolicy::Reject {
        return Ok(());
    }
    if required.is_empty() {
        return Err(Status::InvalidConfig);
    }
    let ws = ccb.write_seq.load(Ordering::Acquire);
    let fv = ccb.first_valid_seq.load(Ordering::Acquire);
    if ws.saturating_sub(fv) >= depth {
        let retire_seq = ws - depth;
        for req in required {
            if !req.may_retire(retire_seq) {
                return Err(Status::QueueFull);
            }
        }
    }
    Ok(())
}
