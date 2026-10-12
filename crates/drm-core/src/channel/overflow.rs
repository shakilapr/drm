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

#[cfg(test)]
mod tests {
    use super::{OverflowPolicy, RequiredSequential, allow_commit};
    use crate::channel::ccb::ChannelControlBlock;
    use drm_types::Status;
    use std::sync::atomic::Ordering;

    /// REQ: REQ-CORE-015 — the descriptor byte decodes to exactly two policies;
    /// any other byte is `InvalidData`; the encoding round-trips.
    #[test]
    fn policy_byte_round_trip() {
        assert_eq!(OverflowPolicy::from_byte(0), Ok(OverflowPolicy::Overwrite));
        assert_eq!(OverflowPolicy::from_byte(1), Ok(OverflowPolicy::Reject));
        assert_eq!(OverflowPolicy::from_byte(2), Err(Status::InvalidData));
        assert_eq!(OverflowPolicy::from_byte(255), Err(Status::InvalidData));
        assert_eq!(OverflowPolicy::Overwrite.to_byte(), 0);
        assert_eq!(OverflowPolicy::Reject.to_byte(), 1);
    }

    /// REQ: REQ-CORE-059, REQ-CORE-001 — a fresh record acknowledges nothing
    /// (init 0, sequence 0 is representable); `may_retire` is strictly past the
    /// watermark and `ack_completed` is monotonic; the sequence-exhaustion guard
    /// fires at `u64::MAX` (REQ-CORE-024).
    #[test]
    fn ack_watermark_and_monotonicity() {
        let rec = RequiredSequential::new();
        assert_eq!(rec.ack_seq.load(Ordering::Acquire), 0, "init 0");
        assert!(!rec.may_retire(u64::MAX), "nothing acknowledged");
        assert!(!rec.may_retire(0), "retire 0 not strictly past init");

        assert_eq!(rec.ack_completed(0), Ok(()));
        assert_eq!(rec.ack_seq.load(Ordering::Acquire), 1);
        assert!(rec.may_retire(0), "retire 0 strictly past ack 1");
        assert!(!rec.may_retire(1), "retire == ack is not past it");

        // Monotonic: a stale acknowledge never lowers the watermark.
        assert_eq!(rec.ack_completed(0), Ok(()));
        assert_eq!(rec.ack_seq.load(Ordering::Acquire), 1, "no regression");
        assert_eq!(rec.ack_completed(9), Ok(()));
        assert_eq!(rec.ack_seq.load(Ordering::Acquire), 10);
        assert_eq!(
            rec.ack_completed(u64::MAX),
            Err(Status::SequenceExhausted),
            "no successor for u64::MAX"
        );
    }

    /// REQ: REQ-CORE-039, REQ-CORE-040 — the commit gate is transparent for the
    /// overwrite policy (never blocks), rejects a zero-required reject topic as
    /// `InvalidConfig` (I-OV5), passes while the retained window is within depth,
    /// and returns `QueueFull` once retirement would overtake an unacknowledged
    /// required consumer.
    #[test]
    fn allow_commit_paths() {
        let ccb = ChannelControlBlock::zeroed();
        let required = [RequiredSequential::new()];

        // Overwrite: never gated, even with an empty required registry.
        assert_eq!(
            allow_commit(OverflowPolicy::Overwrite, &ccb, &[], 4),
            Ok(())
        );

        // Reject with no required consumer is a runtime config violation.
        assert_eq!(
            allow_commit(OverflowPolicy::Reject, &ccb, &[], 4),
            Err(Status::InvalidConfig)
        );

        // Reject within depth: no sequence retires yet, so the gate passes.
        ccb.write_seq.store(3, Ordering::Release);
        assert_eq!(
            allow_commit(OverflowPolicy::Reject, &ccb, &required, 8),
            Ok(())
        );

        // Reject at the depth edge: retiring sequence 0 needs ack > 0.
        ccb.write_seq.store(4, Ordering::Release);
        assert_eq!(
            allow_commit(OverflowPolicy::Reject, &ccb, &required, 4),
            Err(Status::QueueFull),
            "unacknowledged consumer gates retirement"
        );
        assert_eq!(required[0].ack_completed(0), Ok(()));
        assert_eq!(
            allow_commit(OverflowPolicy::Reject, &ccb, &required, 4),
            Ok(()),
            "acknowledged past the retiring sequence"
        );
    }
}
