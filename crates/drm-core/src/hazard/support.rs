//! Shared fixtures for the hazard forcing suites (`race`, `aba`, `select`).
//! REQ: REQ-CORE-030 (acquisition steps), REQ-CORE-055 (selection scan).

use std::sync::atomic::Ordering;

use crate::channel::ccb::ChannelControlBlock;
use crate::ring::HistoryRing;

use super::{ConsumerRec, HAZARD_NONE, pack};

/// Zeroed control block (fresh channel metadata mirror).
#[must_use]
pub(crate) fn ccb() -> ChannelControlBlock {
    ChannelControlBlock::zeroed()
}

/// Ring of depth `r`.
#[must_use]
pub(crate) fn ring(r: u64) -> HistoryRing {
    HistoryRing::new(r).unwrap_or_else(|e| unreachable!("ring: {e:?}"))
}

/// Commit `seq -> slot` at `generation` and advance the published watermark
/// (single writer, as the publisher transaction does).
pub(crate) fn commit(
    ring: &HistoryRing,
    ccb: &ChannelControlBlock,
    seq: u64,
    slot: u32,
    generation: u64,
) {
    let stable = ring
        .preflight_commit(seq)
        .unwrap_or_else(|e| unreachable!("preflight seq={seq}: {e:?}"));
    ring.commit(seq, slot, generation, stable);
    ccb.write_seq.store(seq + 1, Ordering::Release);
}

/// Protect `slot` for `rec` from its empty word at registration generation `g`.
pub(crate) fn protect(rec: &ConsumerRec, g: u32, slot: u32) {
    assert!(
        rec.hazard
            .compare_exchange(pack(g, HAZARD_NONE), pack(g, slot)),
        "install failed"
    );
}

/// Clear `rec`'s word if it still names a slot at generation `g`.
pub(crate) fn unprotect(rec: &ConsumerRec, g: u32) {
    let (wgen, slot) = rec.hazard.load(Ordering::SeqCst);
    if wgen == g && slot != HAZARD_NONE {
        assert!(
            rec.hazard
                .compare_exchange(pack(g, slot), pack(g, HAZARD_NONE)),
            "clear failed"
        );
    }
}
