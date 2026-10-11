//! Selection-vs-borrows property tests (hazard-protocol.md §Functions
//! `select_slot`; payload-pool I-P5 drop-free step 1).
//! REQ: REQ-CORE-055 (pick a slot neither history-referenced nor
//! hazard-protected), REQ-CORE-004 (borrowed slot never overwritten).

use std::sync::atomic::Ordering;

use super::support;
use super::{ConsumerRec, select_slot};

/// With H borrows held at distinct slots and R history-referenced slots, at
/// `pool_size = R + H + 1` a free slot always exists, and the chosen slot is
/// never history-referenced nor protected. REQ: REQ-CORE-055, REQ-CORE-004.
#[test]
fn hazard_select_free_exists_at_r_h_1() {
    const R: u64 = 4;
    const H: usize = 3;
    let pool = R + u64::try_from(H).unwrap_or(0) + 1;
    let ring = support::ring(R);
    let ccb = support::ccb();
    // R history-referenced slots {0,1,2,3}, one per accepted sequence.
    for seq in 0..R {
        support::commit(&ring, &ccb, seq, u32::try_from(seq).unwrap_or(0), seq + 1);
    }
    // H distinct protected slots outside history {4,5,6}.
    let recs: Vec<ConsumerRec> = (0..H)
        .map(|i| ConsumerRec::new(u32::try_from(i + 1).unwrap_or(1)))
        .collect();
    let mut held = Vec::with_capacity(H);
    for (i, rec) in recs.iter().enumerate() {
        let g = rec.generation.load(Ordering::Acquire);
        let target = u32::try_from(R + u64::try_from(i).unwrap_or(0)).unwrap_or(0);
        support::protect(rec, g, target);
        held.push(target);
    }
    let mut scratch = Vec::new();
    let chosen = select_slot(&ring, &ccb, &recs, &mut scratch, pool)
        .unwrap_or_else(|| unreachable!("free slot must exist at R+H+1"));
    // The sole free slot is 7; it is neither history-referenced nor protected.
    assert_eq!(chosen, u32::try_from(pool - 1).unwrap_or(0));
    assert!(!held.contains(&chosen));
}

/// Property: over many publishes, with H distinct held borrows, `select_slot`
/// returns only free slots and — at `pool_size = R + H + 1` — always returns
/// `Some` (publisher never drops). REQ: REQ-CORE-055, REQ-CORE-004.
#[test]
fn hazard_select_returns_only_free() {
    const R: u64 = 3;
    const H: usize = 4;
    let pool = R + u64::try_from(H).unwrap_or(0) + 1;
    let pool_u32 = u32::try_from(pool).unwrap_or(u32::MAX);
    let ring = support::ring(R);
    let ccb = support::ccb();
    let recs: Vec<ConsumerRec> = (0..H)
        .map(|i| ConsumerRec::new(u32::try_from(i + 1).unwrap_or(1)))
        .collect();
    let gens: Vec<u32> = recs
        .iter()
        .map(|r| r.generation.load(Ordering::Acquire))
        .collect();
    let mut scratch = Vec::new();
    for seq in 0..5_000_u64 {
        support::commit(
            &ring,
            &ccb,
            seq,
            u32::try_from(seq % R).unwrap_or(0),
            seq + 1,
        );
        // Each endpoint holds a fresh borrow of a distinct slot outside history.
        let mut held: Vec<u32> = Vec::with_capacity(H);
        for (i, rec) in recs.iter().enumerate() {
            support::unprotect(rec, gens[i]);
            let target = u32::try_from((R + u64::try_from(i).unwrap_or(0)) % pool).unwrap_or(0);
            support::protect(rec, gens[i], target);
            held.push(target);
        }
        // Independent forbidden set: history-referenced + protected slots.
        let ws = ccb.write_seq.load(Ordering::Acquire);
        let fv = ccb.first_valid_seq.load(Ordering::Acquire);
        let oldest = ws.saturating_sub(ring.depth()).max(fv);
        let mut forbidden: Vec<u32> = Vec::new();
        for t in oldest..ws {
            if let Ok(entry) = ring.lookup_sc(&ccb, t) {
                forbidden.push(entry.slot);
            }
        }
        forbidden.extend(held.iter().copied());
        let chosen = select_slot(&ring, &ccb, &recs, &mut scratch, pool)
            .unwrap_or_else(|| unreachable!("no free slot at R+H+1 seq={seq}"));
        assert!(
            !forbidden.contains(&chosen),
            "seq={seq} chose forbidden slot {chosen}"
        );
        assert!(chosen < pool_u32, "seq={seq} slot {chosen} out of pool");
    }
}
