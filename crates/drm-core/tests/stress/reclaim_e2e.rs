//! Phase 16 dead-reader reclamation end-to-end stress (work-plan M06W1-E1-T1;
//! `docs/algorithms/runtime/exhaustion-reclamation.md` §Protocols steps 1–5 and
//! §Test Matrix `reclaim_*`): one publisher storms patterned samples while live
//! readers hold hazard borrows (their identity snapshots recorded live) and
//! zombie endpoints leave abandoned words naming every pool slot (dead identity).
//! Each exhaustion drop arms the reclamation trigger; a maintenance thread
//! drains it (trigger-only, never speculative polling) and every reclaimed word
//! must belong to a dead owner — a live reader's held slot is never reclaimed
//! (I-E5). A shadow validator asserts I-HZ1 throughout: no torn, mixed-generation,
//! or invalid sample is ever observed while a borrow is conceptually held.
//!
//! Unlike copy-form delivery this holds the transient borrow across a bounded
//! spin window, so a mis-reclaimed live slot would surface as a corrupted read.
//! All coordination is atomics + `spin_loop` hints; there are no sleeps.
//!
//! Ignored by default; run with `cargo xtask test --stress`. Scale the cycle
//! budget with `DRM_STRESS_CYCLES` (default 1e6).
//! REQ: REQ-CORE-004 (I-HZ1), REQ-CORE-021 (I-E2), REQ-CORE-023 (I-E4),
//! REQ-CORE-056 (I-E5 lock authority), REQ-CORE-057 (identity diagnostic),
//! REQ-CORE-058 (trigger-only), REQ-QAL-006.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use drm_core::channel::Channel;
use drm_core::channel::overflow::OverflowPolicy;
use drm_core::channel::pattern::{Anomaly, validate_pattern, write_pattern};
use drm_core::hazard::{AcquireErr, HAZARD_NONE, acquire, pack};
use drm_core::shm::current_identity;
use drm_types::Status;

/// Patterned sample size (`>= pattern::MIN_LEN`).
const SAMPLE: usize = 128;
/// History depth R.
const DEPTH: u64 = 8;
/// Pool slots; `DEPTH + 1` keeps the reclamation path on the critical edge.
const POOL: u64 = DEPTH + 1;
/// Live reader endpoints (identity snapshot recorded as the running process).
const LIVE: usize = 3;
/// Zombie endpoints — one per pool slot, so a fully-installed churn pins every
/// slot and forces exhaustion regardless of history placement.
const ZOMBIE: usize = 9;
/// Declared consumer records.
const BORROWERS: usize = LIVE + ZOMBIE;
/// Bounded spin window a live borrow is held for (widens the mis-reclaim sensor).
const HOLD: u32 = 32;

/// Storm counters (single writer per field; relaxed reads are exact after join).
#[derive(Default)]
struct Counters {
    published: AtomicU64,
    dropped: AtomicU64,
    reads: AtomicU64,
    haunts: AtomicU64,
    reclaimed: AtomicU64,
    passes: AtomicU64,
    errors: AtomicU64,
}

/// Continuous I-HZ1 shadow check: classify every observed held-borrow sample.
#[derive(Default)]
struct Shadow {
    reads: AtomicU64,
    torn: AtomicU64,
    mixed: AtomicU64,
    invalid: AtomicU64,
}

impl Shadow {
    /// Classify one observed sample.
    fn observe(&self, buf: &[u8]) {
        self.reads.fetch_add(1, Ordering::Relaxed);
        match validate_pattern(buf) {
            Ok(()) => {}
            Err(Anomaly::Torn) => {
                self.torn.fetch_add(1, Ordering::Relaxed);
            }
            Err(Anomaly::MixedGeneration) => {
                self.mixed.fetch_add(1, Ordering::Relaxed);
            }
            Err(Anomaly::Invalid) => {
                self.invalid.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Total I-HZ1 shadow violations.
    fn violations(&self) -> u64 {
        self.torn.load(Ordering::Relaxed)
            + self.mixed.load(Ordering::Relaxed)
            + self.invalid.load(Ordering::Relaxed)
    }
}

/// Publish `cycles` patterned samples (single publisher per topic). Exhaustion
/// drops are counted and never retried; the publish-drop path arms the trigger.
fn publisher(ch: &Channel, cycles: u64, c: &Counters, done: &AtomicBool) {
    let mut body = [0_u8; SAMPLE];
    for seq in 0..cycles {
        write_pattern(&mut body, seq, 0, seq);
        loop {
            match ch.publish(&body) {
                Ok(_) => {
                    c.published.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(Status::PoolExhausted) => {
                    c.dropped.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(Status::Busy) => std::hint::spin_loop(),
                Err(_) => {
                    c.errors.fetch_add(1, Ordering::Relaxed);
                    break;
                }
            }
        }
    }
    done.store(true, Ordering::Release);
}

/// Zombie endpoint (index `idx`): install one abandoned word naming slot
/// `idx % POOL` with a dead identity, then hold it until the reclaimer clears it;
/// reinstall and repeat. No release path exists — the slot is freed only by the
/// reclaimer's proof-by-lock (I-E5).
fn haunter(ch: &Channel, idx: usize, c: &Counters, done: &AtomicBool) {
    let Some(rec) = ch.rec(idx) else {
        return;
    };
    let slot = u32::try_from(idx % usize::try_from(POOL).unwrap_or(1)).unwrap_or(0);
    let g = rec.generation.load(Ordering::Acquire);
    while !done.load(Ordering::Acquire) {
        if rec
            .hazard
            .compare_exchange(pack(g, HAZARD_NONE), pack(g, slot))
        {
            c.haunts.fetch_add(1, Ordering::Relaxed);
            while !done.load(Ordering::Acquire)
                && rec.hazard.load(Ordering::SeqCst) != (g, HAZARD_NONE)
            {
                std::hint::spin_loop();
            }
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Live reader (index `idx`): acquire a transient borrow of the newest committed
/// sample, hold it across a bounded spin window, and validate the payload on both
/// ends. Its record carries the running process's identity, so reclamation must
/// never clear its word; a violation would corrupt the second validation.
fn reader(ch: &Channel, idx: usize, c: &Counters, done: &AtomicBool, shadow: &Shadow) {
    let Some(rec) = ch.rec(idx) else {
        return;
    };
    while !done.load(Ordering::Acquire) {
        let ws = ch.write_seq();
        if ws == 0 {
            std::hint::spin_loop();
            continue;
        }
        match acquire(ch.ring(), ch.ccb(), ws - 1, rec) {
            Ok(mut borrow) => {
                shadow.observe(ch.payload(&borrow.entry));
                for _ in 0..HOLD {
                    std::hint::spin_loop();
                }
                shadow.observe(ch.payload(&borrow.entry));
                borrow.release();
                c.reads.fetch_add(1, Ordering::Relaxed);
            }
            Err(AcquireErr::Status(_) | AcquireErr::Lookup(_) | AcquireErr::Retry) => {
                std::hint::spin_loop();
            }
        }
    }
}

/// Maintenance reclaimer: drain the exhaustion trigger (once per pass) and
/// accumulate reclaimed slots. Never runs speculatively — no flag, no pass.
fn reclaimer(ch: &Channel, c: &Counters, done: &AtomicBool) {
    while !done.load(Ordering::Acquire) {
        if ch.reclaim_requested() {
            let n = ch.try_reclaim();
            c.reclaimed
                .fetch_add(u64::try_from(n).unwrap_or(0), Ordering::Relaxed);
            c.passes.fetch_add(1, Ordering::Relaxed);
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Activate the storm channel, pre-fill `DEPTH` patterned samples, and record
/// the running process's identity on the first `LIVE` records (the zombie
/// records keep the default dead identity `{0,0}`). Recording before any thread
/// starts means the reclaimer can never observe a live borrow as a dead one.
fn activate_storm() -> Channel {
    let ch = match Channel::activate(
        DEPTH,
        POOL,
        u64::try_from(SAMPLE).unwrap_or(128),
        8,
        OverflowPolicy::Overwrite,
        0,
        BORROWERS,
        None,
    ) {
        Ok(ch) => ch,
        Err(e) => unreachable!("activate: {e:?}"),
    };
    let mut body = [0_u8; SAMPLE];
    for seq in 0..DEPTH {
        write_pattern(&mut body, seq, 0, seq);
        assert!(ch.publish(&body).is_ok(), "prefill seq={seq}");
    }
    let me = current_identity();
    for idx in 0..LIVE {
        match ch.rec(idx) {
            Some(rec) => rec.record_identity(me),
            None => unreachable!("live rec {idx}"),
        }
    }
    ch
}

/// Spawn the storm and join: one publisher, one maintenance reclaimer, `LIVE`
/// held-borrow readers, and one abandoned-word haunter per pool slot.
fn run_storm(ch: &Channel, cycles: u64, c: &Counters, shadow: &Shadow, done: &AtomicBool) {
    std::thread::scope(|s| {
        s.spawn(|| publisher(ch, cycles, c, done));
        s.spawn(|| reclaimer(ch, c, done));
        for idx in 0..LIVE {
            s.spawn(move || reader(ch, idx, c, done, shadow));
        }
        for i in 0..ZOMBIE {
            let idx = LIVE + i;
            s.spawn(move || haunter(ch, idx, c, done));
        }
    });
}

/// REQ: REQ-CORE-004, REQ-CORE-056, REQ-CORE-057, REQ-CORE-058, REQ-CORE-021,
/// REQ-CORE-023, REQ-QAL-006 — a publisher storm with held borrows and a zombie
/// churn: every exhaustion drop is counted exactly, every reclaimed slot was
/// proven dead, publishing resumes, and zero I-HZ1 shadow violations are seen.
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_reclaim_e2e() {
    let cycles = std::env::var("DRM_STRESS_CYCLES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1_000_000);
    let ch = activate_storm();
    let c = Counters::default();
    let shadow = Shadow::default();
    let done = AtomicBool::new(false);
    let t0 = Instant::now();
    run_storm(&ch, cycles, &c, &shadow, &done);
    let elapsed = t0.elapsed();

    // Drain any trigger still armed after the storm so a late drop is reclaimed
    // deterministically (the zombie churn has stopped).
    if ch.reclaim_requested() {
        let n = ch.try_reclaim();
        c.reclaimed
            .fetch_add(u64::try_from(n).unwrap_or(0), Ordering::Relaxed);
        c.passes.fetch_add(1, Ordering::Relaxed);
    }

    println!(
        "reclaim_e2e depth={DEPTH} pool={POOL} live={LIVE} zombie={ZOMBIE} cycles={cycles} \
         published={} dropped={} reclaimed={} passes={} haunts={} reads={} elapsed={:?}",
        c.published.load(Ordering::Relaxed),
        c.dropped.load(Ordering::Relaxed),
        c.reclaimed.load(Ordering::Relaxed),
        c.passes.load(Ordering::Relaxed),
        c.haunts.load(Ordering::Relaxed),
        c.reads.load(Ordering::Relaxed),
        elapsed,
    );

    // I-HZ1: no live reader ever observed a torn/mixed/invalid held sample.
    assert_eq!(shadow.violations(), 0, "I-HZ1 shadow violations");
    assert_eq!(
        c.errors.load(Ordering::Relaxed),
        0,
        "unexpected publish errors"
    );
    assert!(
        c.reads.load(Ordering::Relaxed) > 0,
        "no live held-borrow reads observed"
    );
    assert!(
        c.published.load(Ordering::Relaxed) > 0,
        "publisher never committed a sample"
    );
    assert!(
        c.dropped.load(Ordering::Relaxed) > 0,
        "exhaustion never triggered the reclaim path"
    );
    assert!(
        c.reclaimed.load(Ordering::Relaxed) > 0,
        "no dead slot was ever reclaimed"
    );
    assert!(
        c.passes.load(Ordering::Relaxed) > 0,
        "the trigger was never drained"
    );
    // I-E3: the exhaustion counters are exact for a single publisher.
    assert_eq!(
        ch.pool_exhausted(),
        c.dropped.load(Ordering::Relaxed),
        "pool_exhausted counter exact"
    );
    assert_eq!(
        ch.drop_count(),
        c.dropped.load(Ordering::Relaxed),
        "drop_count counter exact"
    );
}
