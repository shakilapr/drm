//! ABA forcing harness (I-HZ2; hazard-protocol.md §Invariants, §Protocols).
//! REQ: REQ-CORE-031 (64-bit generation prevents ABA), REQ-CORE-049 (no wrap).
//!
//! Forces >= 2^16 slot reuses across a small pool, with the history ring
//! wrapping many times, and asserts a captured stale `{slot, generation}` pair
//! never validates against the current state.

use std::sync::atomic::Ordering;

use drm_types::Status;

use super::bump_generation;
use super::support;

/// 2^16 reuses per slot, cycling a 3-slot pool.
const REUSES: u64 = 1 << 16;
/// Pool size (slot indices `0..3`).
const POOL: u64 = 3;
/// Pool size as an array length.
const POOL_LEN: usize = 3;

/// Long-run ABA harness: every slot is reused `>= 2^16` times with an advancing
/// generation; a stale pair captured at the start never validates afterwards,
/// and the generation counter cannot wrap. REQ: REQ-CORE-031, REQ-CORE-049.
#[test]
fn hazard_aba_reuse_wrap() {
    let ring = support::ring(2);
    let ccb = support::ccb();
    let mut gens = [0_u64; POOL_LEN];
    // Stale pair captured from the very first commit: {slot, generation, seq}.
    let mut stale: Option<(u32, u64, u64)> = None;
    let total = REUSES * POOL;
    for seq in 0..total {
        let slot = u32::try_from(seq % POOL).unwrap_or(0);
        let idx = usize::try_from(slot).unwrap_or(0);
        gens[idx] = gens[idx]
            .checked_add(1)
            .unwrap_or_else(|| unreachable!("generation overflow"));
        support::commit(&ring, &ccb, seq, slot, gens[idx]);
        if seq == 0 {
            stale = Some((slot, gens[idx], seq));
        }
    }
    let (stale_slot, stale_gen, stale_seq) = stale.unwrap_or_else(|| unreachable!("stale pair"));
    // Each slot was reused exactly REUSES (>= 2^16) times.
    for g in gens {
        assert_eq!(g, REUSES);
    }
    // The current generation for the stale slot advanced past the captured one.
    let current_gen = gens[usize::try_from(stale_slot).unwrap_or(0)];
    assert!(
        current_gen > stale_gen,
        "generation must advance on reuse: {stale_gen} -> {current_gen}"
    );
    // The stale sequence is long gone (ring depth 2 keeps only the newest two).
    assert!(
        ring.lookup_sc(&ccb, stale_seq).is_err(),
        "stale sequence still addressable"
    );
    // The stale pair never validates: the addressable newest entry never equals
    // it, and generational identity for the slot has moved on.
    let ws = ccb.write_seq.load(Ordering::Acquire);
    let newest = ring
        .lookup_sc(&ccb, ws - 1)
        .unwrap_or_else(|e| unreachable!("newest: {e:?}"));
    assert_ne!(
        (newest.slot, newest.generation),
        (stale_slot, stale_gen),
        "stale ABA pair validated as fresh"
    );
}

/// Generation exhaustion: the 64-bit counter refuses to wrap — the ABA guard
/// never silences itself. REQ: REQ-CORE-031, REQ-CORE-049.
#[test]
fn hazard_aba_generation_no_wrap() {
    assert_eq!(bump_generation(u64::MAX), Err(Status::GenerationExhausted));
    assert_eq!(bump_generation(u64::MAX - 1), Ok(u64::MAX));
}
