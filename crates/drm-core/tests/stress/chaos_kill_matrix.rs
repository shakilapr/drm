//! Chaos kill matrix (work-plan M18W1-E1-T2): the `reclaim_kill_matrix` unit
//! cells extended to integration scope — every borrow/publish stage is killed
//! (simulated dead incarnation, or a real `SIGKILL`ed shm stage in
//! `shm_kill`), then the suite proves the recovery contract every time: only a
//! lock-proven-dead owner's hazard word is cleared, the latest sample is never
//! corrupted, and publishing resumes once a slot is selectable. A PID-reuse
//! cell and a concurrent pool-exhaustion storm close the matrix. `#[ignore]`d:
//! run via `cargo xtask test --stress` (`DRM_STRESS_ITERS`, default 100).
//! REQ: REQ-QAL-005, REQ-QAL-010, REQ-CORE-056, REQ-CORE-057, REQ-CORE-058,
//! REQ-CORE-095, REQ-CORE-022, REQ-CORE-023.

#![cfg(unix)]

use std::sync::Barrier;
use std::sync::atomic::Ordering;

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_core::hazard::{HAZARD_NONE, acquire, pack};
use drm_core::shm::{ProcessIdentity, current_identity};

/// The dead-owner incarnation (never a live process): the synthetic stand-in
/// for a `SIGKILL`ed reader. REQ: REQ-CORE-057.
const DEAD: ProcessIdentity = ProcessIdentity {
    pid: 0,
    start_ticks: 0,
};

/// Iteration budget shared with the rest of the stress suite.
fn iters() -> usize {
    match std::env::var("DRM_STRESS_ITERS") {
        Ok(v) => v.parse().unwrap_or(100),
        Err(_) => 100,
    }
}

/// Small overwrite channel with `borrowers` consumer records.
fn ch(depth: u64, pool: u64, borrowers: usize) -> Channel {
    Channel::activate(
        depth,
        pool,
        16,
        8,
        OverflowPolicy::Overwrite,
        0,
        borrowers,
        None,
    )
    .unwrap_or_else(|e| unreachable!("chaos activate: {e:?}"))
}

fn pat(v: u8) -> [u8; 16] {
    [v; 16]
}

/// Claims `slot` on `rec(i)` from its empty word; returns the registration
/// generation observed. REQ: REQ-CORE-033.
fn hold(c: &Channel, i: usize, slot: u32) -> u32 {
    let rec = c.rec(i).unwrap_or_else(|| unreachable!("rec {i}"));
    let g = rec.generation.load(Ordering::Acquire);
    assert!(
        rec.hazard
            .compare_exchange(pack(g, HAZARD_NONE), pack(g, slot)),
        "hold rec {i} failed"
    );
    g
}

fn word(c: &Channel, i: usize) -> (u32, u32) {
    c.rec(i)
        .unwrap_or_else(|| unreachable!("rec {i}"))
        .hazard
        .load(Ordering::SeqCst)
}

fn freed(c: &Channel, i: usize, g: u32) -> bool {
    word(c, i) == (g, HAZARD_NONE)
}

fn identity(c: &Channel, i: usize, me: ProcessIdentity) {
    c.rec(i)
        .unwrap_or_else(|| unreachable!("rec {i}"))
        .record_identity(me);
}

/// Stage 1 — pre-intent: registered, no borrow started (word `NONE`).
fn stage_pre_intent() {
    let c = ch(1, 3, 2);
    assert_eq!(c.publish(&pat(1)), Ok(0));
    c.ccb().reclaim_requested.store(true, Ordering::Release);
    assert_eq!(c.try_reclaim(), 0, "pre-intent: nothing protected");
}

/// Stage 2 — post-intent, before the payload borrow: dead owner, word set.
fn stage_post_intent() {
    let c = ch(1, 3, 2);
    assert_eq!(c.publish(&pat(1)), Ok(0));
    identity(&c, 0, DEAD);
    let g = hold(&c, 0, 1);
    c.ccb().reclaim_requested.store(true, Ordering::Release);
    assert_eq!(c.try_reclaim(), 1, "post-intent: dead owner reclaimed");
    assert!(freed(&c, 0, g));
}

/// Stage 3 — mid-borrow: a real live borrow survives, a dead one is reclaimed.
fn stage_mid_borrow() {
    let c = ch(1, 3, 2);
    assert_eq!(c.publish(&pat(1)), Ok(0));
    identity(&c, 0, current_identity());
    // A genuine acquisition installs the live owner's hazard word.
    let borrow = acquire(
        c.ring(),
        c.ccb(),
        0,
        c.rec(0).unwrap_or_else(|| unreachable!("rec0")),
    )
    .unwrap_or_else(|e| unreachable!("live acquire: {e:?}"));
    identity(&c, 1, DEAD);
    let g1 = hold(&c, 1, 1);
    c.ccb().reclaim_requested.store(true, Ordering::Release);
    assert_eq!(c.try_reclaim(), 1, "dead mid-borrow reclaimed");
    assert_eq!(
        word(&c, 0).1,
        borrow.slot(),
        "live mid-borrow not reclaimed"
    );
    drop(borrow);
    assert!(freed(&c, 1, g1));
}

/// Stage 4 — post-release: the owner released before dying (word `NONE`).
fn stage_post_release() {
    let c = ch(1, 3, 2);
    assert_eq!(c.publish(&pat(1)), Ok(0));
    let g = hold(&c, 0, 1);
    let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec0"));
    assert!(
        rec.hazard
            .compare_exchange(pack(g, 1), pack(g, HAZARD_NONE))
    );
    c.ccb().reclaim_requested.store(true, Ordering::Release);
    assert_eq!(c.try_reclaim(), 0, "post-release: nothing protected");
}

/// Stage 5 — publisher restart: a stale-generation word protects nothing.
fn stage_restart_stale_word() {
    let c = ch(1, 3, 2);
    assert_eq!(c.publish(&pat(1)), Ok(0));
    let g = hold(&c, 0, 1);
    assert_eq!(c.restart_run(), 1, "epoch advances");
    // After restart the record keeps its generation; the stale word is cleared.
    c.ccb().reclaim_requested.store(true, Ordering::Release);
    assert_eq!(c.try_reclaim(), 1, "stale word cleared after restart");
    assert!(freed(&c, 0, g));
    assert_eq!(c.publish(&pat(2)), Ok(1), "publishing resumes post-restart");
}

/// REQ: REQ-QAL-010, REQ-CORE-056, REQ-CORE-057, REQ-CORE-058, REQ-CORE-023 —
/// the full borrow/publish stage matrix, repeated: recovery holds at every
/// stage on every iteration (`CHAOS_GREEN`). The registration-lock-held cell
/// lives in the `reclaim_kill_matrix` unit matrix (the lock guard is
/// crate-private); the process/segment kill stages (create/hold/unlink) are
/// covered by the repaired `shm_kill` suite.
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_chaos_kill_matrix() {
    for _ in 0..iters() {
        stage_pre_intent();
        stage_post_intent();
        stage_mid_borrow();
        stage_post_release();
        stage_restart_stale_word();
    }
}

/// REQ: REQ-QAL-010, REQ-CORE-057 — PID reuse: a same-PID different-incarnation
/// owner is dead and reclaimed; an exactly-matching live incarnation survives.
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_chaos_pid_reuse() {
    let me = current_identity();
    assert!(me.pid > 0);
    for _ in 0..iters() {
        let c = ch(1, 3, 2);
        assert_eq!(c.publish(&pat(1)), Ok(0));
        identity(&c, 0, me);
        let ga = hold(&c, 0, 1);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 0, "a live incarnation survives");
        assert_eq!(word(&c, 0), (ga, 1));

        let c2 = ch(1, 3, 2);
        assert_eq!(c2.publish(&pat(1)), Ok(0));
        identity(
            &c2,
            0,
            ProcessIdentity {
                pid: me.pid,
                start_ticks: me.start_ticks.wrapping_add(1),
            },
        );
        let gb = hold(&c2, 0, 1);
        c2.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(
            c2.try_reclaim(),
            1,
            "a reused PID is a different incarnation"
        );
        assert!(freed(&c2, 0, gb));
    }
}

/// REQ: REQ-QAL-010, REQ-CORE-022, REQ-CORE-023, REQ-CORE-021 — pool-exhaustion
/// storm: `H` readers hold every slot (dead incarnations) while a publisher
/// drops with exact counters; the latest sample stays valid (I-E2) and the
/// maintenance pass frees the slots so publishing resumes (I-E1/I-E4).
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_chaos_exhaustion_storm() {
    const H: usize = 4;
    // One extra borrower record is left unprotected so the main thread can read
    // the latest sample while every reader slot is held.
    let c = ch(2, 3, H + 1);
    assert_eq!(c.publish(&pat(0xA1)), Ok(0));
    assert_eq!(c.publish(&pat(0xB2)), Ok(1));

    let barrier = Barrier::new(H);
    std::thread::scope(|scope| {
        for i in 0..H {
            let c = &c;
            let barrier = &barrier;
            scope.spawn(move || {
                // Every reader is a dead incarnation that abandons its slot.
                identity(c, i, DEAD);
                let g = c
                    .rec(i)
                    .unwrap_or_else(|| unreachable!("rec {i}"))
                    .generation
                    .load(Ordering::Acquire);
                let slot = u32::try_from(i % 3).unwrap_or(0);
                let _ = c
                    .rec(i)
                    .unwrap_or_else(|| unreachable!("rec {i}"))
                    .hazard
                    .compare_exchange(pack(g, HAZARD_NONE), pack(g, slot));
                barrier.wait();
            });
        }
    });

    // The pool is fully protected: every publish drops (I-E1) and the exact
    // counter is kept (I-E3); the latest sample stays readable (I-E2).
    let mut st = c.reader();
    let mut buf = [0_u8; 16];
    let drops = iters().max(1);
    for _ in 0..drops {
        assert_eq!(c.publish(&pat(0xC3)), Err(drm_types::Status::PoolExhausted));
        assert_eq!(c.latest(&mut st, H, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(0xB2), "latest uncorrupted through the storm");
    }
    assert_eq!(c.pool_exhausted(), drops as u64, "exact drop accounting");

    // Maintenance reclaims the dead owners and publishing resumes (I-E4).
    assert!(c.try_reclaim() >= 1, "at least one dead slot recovered");
    assert_eq!(c.publish(&pat(0xD4)), Ok(2), "publishing resumes");
}
