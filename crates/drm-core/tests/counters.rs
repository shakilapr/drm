//! CCB counter surface e2e (work-plan M07W1-E1-T2; observability-plane.md
//! §Functions `diag_sample`; channel-metadata.md I-M8). Every counter —
//! `last_sequence`, `drop_count`, `overrun_count`, `publisher_epoch`,
//! `pool_exhausted` (+ the exhaustion `reclaim_requested` flag) — is read
//! through the diag-style [`Channel::counters`] accessor and asserted to track
//! the events that move it.
//! REQ: REQ-CORE-093.

use std::sync::atomic::Ordering;

use drm_core::channel::ccb::ChannelCounters;
use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_core::hazard::{HAZARD_NONE, pack};
use drm_types::Status;

fn ch(policy: OverflowPolicy, depth: u64, pool: u64, required: usize) -> Channel {
    Channel::activate(depth, pool, 16, 8, policy, required, 2, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

fn pat(v: u8) -> [u8; 16] {
    [v; 16]
}

/// Hold the sole remaining selectable slot so the next publish exhausts the
/// pool (I-E1 setup). Reproduces `select_slot`'s referenced-slot scan over
/// `[0, pool_size)` and claims the free slot on `rec(0)`; returns
/// `(generation, slot)` for release.
fn hold_last_slot(c: &Channel) -> (u32, u32) {
    let ws = c.write_seq();
    let oldest = c.oldest_retained(ws);
    let mut used = Vec::new();
    for s in oldest..ws {
        if let Ok(e) = c.ring().lookup_sc(c.ccb(), s) {
            used.push(e.slot);
        }
    }
    let pool = u32::try_from(c.pool_size()).unwrap_or_else(|e| unreachable!("pool: {e:?}"));
    let free = (0..pool)
        .find(|s| !used.contains(s))
        .unwrap_or_else(|| unreachable!("no free slot"));
    let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
    let (g, _) = rec.hazard.load(Ordering::SeqCst);
    assert!(
        rec.hazard
            .compare_exchange(pack(g, HAZARD_NONE), pack(g, free)),
        "hold failed"
    );
    (g, free)
}

/// Release the hold installed by [`hold_last_slot`] (owner guard drop).
fn release_hold(c: &Channel, generation: u32, slot: u32) {
    let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
    assert!(
        rec.hazard
            .compare_exchange(pack(generation, slot), pack(generation, HAZARD_NONE)),
        "release failed"
    );
}

/// REQ: REQ-CORE-093 — the diag-style `counters()` accessor exposes every CCB
/// counter and tracks each event that moves one: initial zeros, commit advance
/// (`write_seq`/`last_sequence`), a consumer `Gap` (`overrun_count`), pool
/// exhaustion (`drop_count`/`pool_exhausted` + `reclaim_requested`), and a
/// publisher restart (`publisher_epoch`).
#[test]
fn counters() {
    let c = ch(OverflowPolicy::Overwrite, 2, 3, 0);

    // Fresh channel: every counter is zero; nothing published yet.
    assert_eq!(
        c.counters(),
        ChannelCounters {
            write_seq: 0,
            last_sequence: 0,
            drop_count: 0,
            overrun_count: 0,
            pool_exhausted: 0,
            publisher_epoch: 0,
            reclaim_requested: false,
        },
        "initial counters"
    );

    // Commits advance `write_seq`; `last_sequence` derives the newest commit.
    assert_eq!(c.publish(&pat(1)), Ok(0));
    assert_eq!(c.publish(&pat(2)), Ok(1));
    let after_commit = c.counters();
    assert_eq!(after_commit.write_seq, 2);
    assert_eq!(after_commit.last_sequence, 1);
    assert_eq!(
        c.last_sequence(),
        1,
        "derived accessor agrees with snapshot"
    );

    // A lapped reader reports a Gap; `overrun_count` accumulates exactly the
    // missed count.
    let mut st = c.reader();
    let mut buf = [0_u8; 16];
    assert_eq!(c.next(&mut st, 1, &mut buf), Ok(ReadOutcome::Sample));
    for v in 2..6_u8 {
        assert!(c.publish(&pat(v)).is_ok(), "v={v}");
    }
    assert_eq!(
        c.next(&mut st, 1, &mut buf),
        Ok(ReadOutcome::Gap { missed: 3 })
    );
    assert_eq!(c.counters().overrun_count, 3);

    // Exhaustion: hold the last selectable slot; the drop path bumps both
    // counters and raises the maintenance flag (I-E1, I-E3).
    let (g, slot) = hold_last_slot(&c);
    assert_eq!(c.publish(&pat(9)), Err(Status::PoolExhausted));
    let exhausted = c.counters();
    assert_eq!(exhausted.drop_count, 1);
    assert_eq!(exhausted.pool_exhausted, 1);
    assert!(
        exhausted.reclaim_requested,
        "exhaustion requests reclamation"
    );
    assert_eq!(exhausted.write_seq, 6, "a dropped publish commits nothing");
    assert_eq!(exhausted.last_sequence, 5);
    release_hold(&c, g, slot);

    // Publisher restart bumps the epoch; counters remain cumulative lifetime
    // (observability-plane I-OB7: diag computes epoch-local deltas).
    assert_eq!(c.restart_run(), 1);
    let restarted = c.counters();
    assert_eq!(restarted.publisher_epoch, 1);
    assert_eq!(restarted.drop_count, 1, "counters are lifetime-cumulative");
    assert_eq!(restarted.overrun_count, 3);
    assert_eq!(restarted.pool_exhausted, 1);
}
