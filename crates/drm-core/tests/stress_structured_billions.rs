//! Structured-channel billion-cycle stress harness (work-plan M03W2-E1-T3):
//! one publisher per depth, multiple copy readers validating the patterned
//! payload (pattern.rs), with continuous monotonic-sequence checking.
//! Counters: torn / mixed-generation / invalid / monotonic violations.
//!
//! Ignored by default; run with `cargo xtask test --stress`. Scale the cycle
//! budget with `DRM_STRESS_CYCLES` (default 1e6; local 1e9 for the phase gate).
//! REQ: REQ-CORE-082, REQ-QAL-006.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use drm_core::channel::overflow::OverflowPolicy;
use drm_core::channel::pattern::{Anomaly, validate_pattern, write_pattern};
use drm_core::channel::{Channel, ReadOutcome};

/// Patterned sample size (>= `pattern::MIN_LEN`).
const SAMPLE: usize = 128;

#[derive(Default)]
struct Counters {
    published: AtomicU64,
    dropped: AtomicU64,
    torn: AtomicU64,
    mixed: AtomicU64,
    invalid: AtomicU64,
    monotonic: AtomicU64,
}

fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

fn publisher(ch: &Channel, cycles: u64, done: &AtomicBool, c: &Counters) {
    let mut body = [0_u8; SAMPLE];
    for seq in 0..cycles {
        write_pattern(&mut body, seq, 0, seq);
        loop {
            match ch.publish(&body) {
                Ok(_) => {
                    bump(&c.published);
                    break;
                }
                Err(drm_types::Status::Busy) => std::hint::spin_loop(),
                Err(drm_types::Status::PoolExhausted) => {
                    bump(&c.dropped);
                    break;
                }
                Err(_) => break,
            }
        }
    }
    done.store(true, Ordering::Release);
}

fn reader(ch: &Channel, idx: usize, done: &AtomicBool, c: &Counters) {
    let mut st = ch.reader();
    let mut buf = [0_u8; SAMPLE];
    let mut last: Option<u64> = None;
    while !done.load(Ordering::Acquire) {
        match ch.try_next(&mut st, idx, &mut buf, 64) {
            Ok(ReadOutcome::Sample) => {
                match validate_pattern(&buf) {
                    Ok(()) => {}
                    Err(Anomaly::Torn) => bump(&c.torn),
                    Err(Anomaly::MixedGeneration) => bump(&c.mixed),
                    Err(Anomaly::Invalid) => bump(&c.invalid),
                }
                let seq = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0_u8; 8]));
                if let Some(prev) = last
                    && seq <= prev
                {
                    bump(&c.monotonic);
                }
                last = Some(seq);
            }
            Ok(ReadOutcome::Gap { .. } | ReadOutcome::Empty | ReadOutcome::WouldRetry) => {
                std::hint::spin_loop();
            }
            Err(_) => {
                bump(&c.invalid);
                break;
            }
        }
    }
}

/// Runs one channel per depth with `readers` concurrent copy readers.
/// Single publisher per topic (brokerless single-writer rule).
fn structured_billions(cycles: u64, readers: usize, depths: &[u64]) -> Counters {
    let totals = Counters::default();
    for &depth in depths {
        let pool = depth + u64::try_from(readers).unwrap_or(0) + 2;
        let ch = match Channel::activate(
            depth,
            pool,
            u64::try_from(SAMPLE).unwrap_or(128),
            8,
            OverflowPolicy::Overwrite,
            0,
            readers,
            None,
        ) {
            Ok(ch) => ch,
            Err(e) => unreachable!("activate depth={depth}: {e:?}"),
        };
        let done = AtomicBool::new(false);
        let local = Counters::default();
        let t0 = Instant::now();
        std::thread::scope(|s| {
            let ch = &ch;
            let done = &done;
            let local = &local;
            s.spawn(move || publisher(ch, cycles, done, local));
            for idx in 0..readers {
                s.spawn(move || reader(ch, idx, done, local));
            }
        });
        let elapsed = t0.elapsed();
        println!(
            "structured depth={depth} cycles={cycles} published={} dropped={} elapsed={:?}",
            local.published.load(Ordering::Relaxed),
            local.dropped.load(Ordering::Relaxed),
            elapsed,
        );
        totals
            .published
            .fetch_add(local.published.load(Ordering::Relaxed), Ordering::Relaxed);
        totals
            .dropped
            .fetch_add(local.dropped.load(Ordering::Relaxed), Ordering::Relaxed);
        totals
            .torn
            .fetch_add(local.torn.load(Ordering::Relaxed), Ordering::Relaxed);
        totals
            .mixed
            .fetch_add(local.mixed.load(Ordering::Relaxed), Ordering::Relaxed);
        totals
            .invalid
            .fetch_add(local.invalid.load(Ordering::Relaxed), Ordering::Relaxed);
        totals
            .monotonic
            .fetch_add(local.monotonic.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    totals
}

/// REQ: REQ-CORE-082, REQ-QAL-006 — billion-cycle run reports zero torn,
/// mixed-generation, invalid, and monotonic-violation events.
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_structured_billions() {
    let cycles = std::env::var("DRM_STRESS_CYCLES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1_000_000);
    let totals = structured_billions(cycles, 4, &[1, 3, 16]);
    assert_eq!(totals.torn.load(Ordering::Relaxed), 0, "torn samples");
    assert_eq!(totals.mixed.load(Ordering::Relaxed), 0, "mixed-generation");
    assert_eq!(totals.invalid.load(Ordering::Relaxed), 0, "invalid samples");
    assert_eq!(totals.monotonic.load(Ordering::Relaxed), 0, "non-monotonic");
    assert!(
        totals.published.load(Ordering::Relaxed) > 0,
        "no samples published"
    );
    println!(
        "structured_billions: published={} dropped={} torn=0 mixed=0 invalid=0 monotonic=0",
        totals.published.load(Ordering::Relaxed),
        totals.dropped.load(Ordering::Relaxed),
    );
}
