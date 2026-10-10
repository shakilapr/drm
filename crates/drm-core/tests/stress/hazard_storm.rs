//! Hazard storm suite skeleton (work-plan M04W3-E1; hazard-protocol.md §Protocols
//! *Storm suite*): one publisher × H readers over a live `Channel`, with a shadow
//! validator asserting I-HZ1 (never overwritten) continuously.
//!
//! Ignored by default; run with `cargo xtask test --stress`. Scale the cycle
//! budget with `DRM_STRESS_CYCLES` (default 1e6; hours-long locally for the
//! phase gate). A full randomized run (random depths, x86-64 + ARM64) and the
//! borrow-form reader (Phase 14) are the follow-on work; this file is the
//! continuous-check scaffold (phase-13.md).
//! REQ: REQ-CORE-004 (I-HZ1), REQ-CORE-032 (I-HZ4), REQ-QAL-006.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use drm_core::channel::overflow::OverflowPolicy;
use drm_core::channel::pattern::{Anomaly, validate_pattern, write_pattern};
use drm_core::channel::{Channel, ReadOutcome};

/// Patterned sample size (>= `pattern::MIN_LEN`).
const SAMPLE: usize = 128;
/// History depth per channel.
const DEPTH: u64 = 8;
/// Reader endpoints (H >= 2 for the multi-reader share invariant).
const READERS: usize = 4;

/// Continuous shadow check for I-HZ1. Each reader sample is validated while the
/// borrow is conceptually held; a slot reused under an active borrow would
/// surface as a torn or mixed-generation payload (a hazard semantic violation).
/// When the Phase-14 borrow-form read API lands, `observe` is fed the raw
/// borrowed slice and this same structure asserts the reader's protected slot is
/// never the publisher's selected slot.
#[derive(Default)]
struct ShadowValidator {
    reads: AtomicU64,
    torn: AtomicU64,
    mixed: AtomicU64,
    invalid: AtomicU64,
}

impl ShadowValidator {
    /// Classify one observed sample (I-HZ1 violation detector).
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

/// Publish `cycles` patterned samples, then signal completion. One publisher per
/// topic (brokerless single-writer rule).
fn publisher(ch: &Channel, cycles: u64, published: &AtomicU64, done: &AtomicBool) {
    let mut body = [0_u8; SAMPLE];
    for seq in 0..cycles {
        write_pattern(&mut body, seq, 0, seq);
        loop {
            match ch.publish(&body) {
                Ok(_) => {
                    published.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(drm_types::Status::Busy) => std::hint::spin_loop(),
                Err(_) => break,
            }
        }
    }
    done.store(true, Ordering::Release);
}

/// Latest-sample reader feeding the shadow validator until the publisher stops.
fn reader(ch: &Channel, idx: usize, done: &AtomicBool, shadow: &ShadowValidator) {
    let mut st = ch.reader();
    let mut buf = [0_u8; SAMPLE];
    while !done.load(Ordering::Acquire) {
        match ch.try_latest(&mut st, idx, &mut buf, 64) {
            Ok(ReadOutcome::Sample) => shadow.observe(&buf),
            Ok(_) => std::hint::spin_loop(),
            Err(_) => break,
        }
    }
}

/// REQ: REQ-CORE-004, REQ-CORE-032, REQ-QAL-006 — publisher × H readers storm
/// reports zero I-HZ1 shadow violations.
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_hazard_storm() {
    let cycles = std::env::var("DRM_STRESS_CYCLES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1_000_000);
    let pool = DEPTH + u64::try_from(READERS).unwrap_or(0) + 2;
    let ch = match Channel::activate(
        DEPTH,
        pool,
        u64::try_from(SAMPLE).unwrap_or(128),
        8,
        OverflowPolicy::Overwrite,
        0,
        READERS,
        None,
    ) {
        Ok(ch) => ch,
        Err(e) => unreachable!("activate: {e:?}"),
    };
    let done = AtomicBool::new(false);
    let published = AtomicU64::new(0);
    let shadow = ShadowValidator::default();
    let t0 = Instant::now();
    std::thread::scope(|s| {
        let ch = &ch;
        let published = &published;
        let done = &done;
        let shadow = &shadow;
        s.spawn(|| publisher(ch, cycles, published, done));
        for idx in 0..READERS {
            s.spawn(move || reader(ch, idx, done, shadow));
        }
    });
    let elapsed = t0.elapsed();
    println!(
        "hazard_storm depth={DEPTH} readers={READERS} published={} reads={} elapsed={:?}",
        published.load(Ordering::Relaxed),
        shadow.reads.load(Ordering::Relaxed),
        elapsed,
    );
    assert_eq!(shadow.violations(), 0, "I-HZ1 shadow violations");
    assert!(
        shadow.reads.load(Ordering::Relaxed) > 0,
        "no samples observed by the shadow validator"
    );
}
