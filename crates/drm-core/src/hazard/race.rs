//! Acquisition race matrix forcing tests (hazard-protocol.md §Protocols R1-R6;
//! I-HZ3). REQ: REQ-CORE-030, REQ-CORE-031, REQ-CORE-032, REQ-CORE-033,
//! REQ-CORE-034, REQ-CORE-055, REQ-CORE-004.
//!
//! R1-R3 force the publisher interleavings deterministically on one thread via
//! a test-local replica of `acquire` (`acquire_with_roll`) carrying a roll hook
//! at the exact race point; the replica mirrors the production comparison in
//! `hazard::acquire`. R4 uses real threads aligned on a [`Barrier`] (no sleeps
//! as synchronization). No production seam was added.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};

use crate::ring::{HistoryRing, RingEntry};
use drm_types::Status;

use super::support;
use super::{AcquireErr, Borrow, ConsumerRec, HAZARD_NONE, acquire, pack, select_slot};

/// Race point at which the publisher roll is injected.
#[derive(Debug, Clone, Copy)]
enum RollPoint {
    /// Between read(1) and the intent-store(2) CAS (R1).
    BeforeIntent,
    /// Between the intent-store(2) and the re-read(3) (R2, R3).
    AfterIntent,
}

/// Test-local replica of [`acquire`] with a publisher roll injected at `point`.
/// Mirrors the production comparison; the hook forces one interleaving on a
/// single thread. REQ: REQ-CORE-030, REQ-CORE-034.
fn acquire_with_roll(
    ring: &HistoryRing,
    ccb: &crate::channel::ccb::ChannelControlBlock,
    target_seq: u64,
    rec: &ConsumerRec,
    point: RollPoint,
    roll: impl Fn(),
) -> Result<Borrow, AcquireErr> {
    let g = rec.generation.load(Ordering::Acquire);
    let empty = pack(g, HAZARD_NONE);
    let e1 = ring
        .lookup_sc(ccb, target_seq)
        .map_err(AcquireErr::Lookup)?;
    if matches!(point, RollPoint::BeforeIntent) {
        roll();
    }
    if !rec.hazard.compare_exchange(empty, pack(g, e1.slot)) {
        return Err(AcquireErr::Status(Status::Busy));
    }
    let installed = pack(g, e1.slot);
    if matches!(point, RollPoint::AfterIntent) {
        roll();
    }
    let e2 = match ring.lookup_sc(ccb, target_seq) {
        Ok(entry) => entry,
        Err(err) => {
            rec.hazard.compare_exchange(installed, empty);
            return Err(AcquireErr::Lookup(err));
        }
    };
    if e1 != e2 {
        rec.hazard.compare_exchange(installed, empty);
        return Err(AcquireErr::Retry);
    }
    Ok(Borrow::install(e2, &rec.hazard, installed))
}

/// R1 — publisher rolls the entry between read(1) and intent-store(2); acquire
/// retries cleanly and leaves no torn protection installed.
/// REQ: REQ-CORE-030, REQ-CORE-034.
#[test]
fn race_r1_roll_before_intent() {
    let ring = support::ring(4);
    let ccb = support::ccb();
    support::commit(&ring, &ccb, 0, 1, 7);
    let rec = ConsumerRec::new(9);
    let outcome = acquire_with_roll(&ring, &ccb, 0, &rec, RollPoint::BeforeIntent, || {
        // Publisher rolls the target entry to a new slot/generation first.
        let stable = ring
            .preflight_commit(0)
            .unwrap_or_else(|e| unreachable!("preflight: {e:?}"));
        ring.commit(0, 3, 8, stable);
    });
    assert_eq!(outcome.err(), Some(AcquireErr::Retry));
    // No torn install: the exact installed word was cleared back to empty form.
    assert_eq!(rec.hazard.load(Ordering::SeqCst), (9, HAZARD_NONE));
    // A fresh acquire observes the rolled entry.
    let borrow = acquire(&ring, &ccb, 0, &rec).unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
    assert_eq!(borrow.slot(), 3);
}

/// R2 — publisher rolls the entry between intent-store(2) and re-read(3); the
/// mismatch is detected, protection cleared, and `Retry` returned.
/// REQ: REQ-CORE-030, REQ-CORE-034.
#[test]
fn race_r2_roll_after_intent() {
    let ring = support::ring(4);
    let ccb = support::ccb();
    support::commit(&ring, &ccb, 0, 1, 7);
    let rec = ConsumerRec::new(9);
    let outcome = acquire_with_roll(&ring, &ccb, 0, &rec, RollPoint::AfterIntent, || {
        let stable = ring
            .preflight_commit(0)
            .unwrap_or_else(|e| unreachable!("preflight: {e:?}"));
        ring.commit(0, 3, 8, stable);
    });
    assert_eq!(outcome.err(), Some(AcquireErr::Retry));
    assert_eq!(rec.hazard.load(Ordering::SeqCst), (9, HAZARD_NONE));
    let borrow = acquire(&ring, &ccb, 0, &rec).unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
    assert_eq!(borrow.slot(), 3);
}

/// R3 — slot reuse with a generation change between intent and re-read; the
/// stale `{slot, generation}` pair is rejected even though the slot is identical.
/// REQ: REQ-CORE-031, REQ-CORE-034.
#[test]
fn race_r3_generation_wrap() {
    let ring = support::ring(4);
    let ccb = support::ccb();
    support::commit(&ring, &ccb, 0, 1, 7);
    let rec = ConsumerRec::new(9);
    let outcome = acquire_with_roll(&ring, &ccb, 0, &rec, RollPoint::AfterIntent, || {
        // Same slot, new generation: only the generation distinguishes the pair.
        let stable = ring
            .preflight_commit(0)
            .unwrap_or_else(|e| unreachable!("preflight: {e:?}"));
        ring.commit(0, 1, 8, stable);
    });
    assert_eq!(outcome.err(), Some(AcquireErr::Retry));
    assert_eq!(rec.hazard.load(Ordering::SeqCst), (9, HAZARD_NONE));
    // The re-read now carries the fresh generation; the stale pair never installs.
    assert_eq!(
        ring.lookup_sc(&ccb, 0).map(|e| e.generation),
        Ok(8),
        "stale generation 7 replaced by 8"
    );
}

/// R4 — two consumers acquire the same sequence concurrently (real threads
/// aligned on a barrier). Both succeed sharing protection of one slot via their
/// own hazard words; no corruption. REQ: REQ-CORE-030, REQ-CORE-032.
#[test]
fn race_r4_concurrent_acquire() {
    const ROUNDS: usize = 64;
    let ring = Arc::new(support::ring(8));
    let ccb = Arc::new(support::ccb());
    support::commit(&ring, &ccb, 0, 2, 11);
    let expected = RingEntry {
        sequence: 0,
        slot: 2,
        generation: 11,
    };
    for round in 0..ROUNDS {
        let recs = [Arc::new(ConsumerRec::new(1)), Arc::new(ConsumerRec::new(2))];
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = recs
            .iter()
            .map(|rec| {
                let ring = Arc::clone(&ring);
                let ccb = Arc::clone(&ccb);
                let barrier = Arc::clone(&barrier);
                let rec = Arc::clone(rec);
                std::thread::spawn(move || {
                    barrier.wait();
                    acquire(&ring, &ccb, 0, &rec)
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| unreachable!("join round={round}"))
            })
            .collect();
        for res in &results {
            match res {
                Ok(b) => assert_eq!(b.entry, expected, "round={round}"),
                Err(e) => unreachable!("round={round}: {e:?}"),
            }
        }
        // Each endpoint's own word carries the shared slot.
        assert_eq!(recs[0].hazard.load(Ordering::SeqCst), (1, 2));
        assert_eq!(recs[1].hazard.load(Ordering::SeqCst), (2, 2));
        drop(results);
        assert_eq!(recs[0].hazard.load(Ordering::SeqCst), (1, HAZARD_NONE));
        assert_eq!(recs[1].hazard.load(Ordering::SeqCst), (2, HAZARD_NONE));
    }
}

/// R5 — a consumer releases while the publisher scans hazard words: the scan
/// observes either the protected word (slot skipped) or the cleared word (slot
/// free) atomically — never a torn state. REQ: REQ-CORE-055, REQ-CORE-004.
#[test]
fn race_r5_release_during_scan() {
    let ring = support::ring(2);
    let ccb = support::ccb();
    support::commit(&ring, &ccb, 0, 0, 1);
    support::commit(&ring, &ccb, 1, 1, 2);
    let recs = [ConsumerRec::new(1)];
    let mut scratch = Vec::new();
    support::protect(&recs[0], 1, 2);
    // History {0,1} + protected {2} => no free slot at pool 3.
    assert_eq!(select_slot(&ring, &ccb, &recs, &mut scratch, 3), None);
    // Release observed by the next scan: slot 2 becomes selectable.
    support::unprotect(&recs[0], 1);
    assert_eq!(select_slot(&ring, &ccb, &recs, &mut scratch, 3), Some(2));
}

/// R6 — publisher reuses a slot exactly at the consumer's re-read: while the
/// hazard word names the slot, selection must skip it; the protected slot is
/// never handed to a new publish. REQ: REQ-CORE-004, REQ-CORE-055.
#[test]
fn race_r6_reuse_during_reread() {
    let ring = support::ring(2);
    let ccb = support::ccb();
    support::commit(&ring, &ccb, 0, 0, 1);
    support::commit(&ring, &ccb, 1, 1, 2);
    let recs = [ConsumerRec::new(1)];
    let mut scratch = Vec::new();
    // Consumer holds slot 0 (its re-read target) while the publisher selects.
    support::protect(&recs[0], 1, 0);
    let chosen =
        select_slot(&ring, &ccb, &recs, &mut scratch, 3).unwrap_or_else(|| unreachable!("free"));
    assert_ne!(chosen, 0, "protected slot must not be reused");
    assert_eq!(chosen, 2);
}
