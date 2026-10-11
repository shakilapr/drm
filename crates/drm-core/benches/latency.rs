//! Latency bench harness for the runtime data paths: publish, `copy_latest`,
//! `copy_next`, the publisher write-loan (borrow) round trip, and the
//! exhaustion/backpressure (`QueueFull`) publish path. Methodology is fixed
//! (warmup, fixed repetition count, nearest-rank percentiles); the CI runner
//! pins the CPU (`taskset -c 2 cargo bench`) and archives the numbers as a
//! regression reference. The zero-allocation and bounded-time invariants are
//! asserted inside the bench and fail it on violation.
//!
//! REQ: REQ-QAL-011 (latency baseline), REQ-QAL-012 (in-bench invariants),
//! REQ-CORE-040 (publish never blocks).

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_main};

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};

/// A 64-byte canonical-shaped sample (contents irrelevant to the timing).
const SAMPLE: [u8; 64] = [0x5A; 64];

/// Declared DRM memory budget for the bench compute (memory-budget.md I-MB2).
/// REQ: REQ-TOOL-004.
const MEMORY_BUDGET_BYTES: u64 = 4096;

/// Worst-case footprint of the exhaustion-path channel (memory-budget.md I-MB1):
/// `pool_slots × slot_bytes + control_block`, i.e. 16 × 64 + 1024 = 2048 bytes.
/// REQ: REQ-TOOL-010.
const EXHAUSTION_CHANNEL_BYTES: u64 = 16 * 64 + 1024;

const _: () = assert!(EXHAUSTION_CHANNEL_BYTES <= MEMORY_BUDGET_BYTES);

/// A channel with depth 16 and pool 64 — the minimal drop-free configuration.
fn channel() -> Channel {
    match Channel::activate(16, 64, 64, 8, OverflowPolicy::Overwrite, 0, 2, None) {
        Ok(ch) => ch,
        Err(e) => unreachable!("activate: {e:?}"),
    }
}

/// REQ: REQ-QAL-011 — publish + read latency.
fn bench_publish_read(c: &mut Criterion) {
    let ch = channel();
    let mut state = ch.reader();
    let mut buf = [0u8; 64];

    c.bench_function("publish_copy", |b| {
        b.iter(|| {
            let seq = ch.publish(black_box(&SAMPLE));
            black_box(seq)
        });
    });

    let _ = ch.publish(&SAMPLE);
    c.bench_function("copy_latest", |b| {
        b.iter(|| {
            let out = ch.latest(&mut state, 0, &mut buf);
            black_box(out)
        });
    });

    c.bench_function("copy_next", |b| {
        b.iter(|| {
            let out = ch.next(&mut state, 0, &mut buf);
            black_box(out)
        });
    });
}

/// REQ: REQ-QAL-011 — publisher write-loan (borrow) throughput.
fn bench_borrow(c: &mut Criterion) {
    let ch = channel();
    c.bench_function("borrow_loan_roundtrip", |b| {
        b.iter(|| {
            let outcome = ch.write_acquire(64).and_then(|loan| {
                if let Ok(dst) = ch.loan_bytes(&loan) {
                    dst.copy_from_slice(&SAMPLE);
                }
                ch.write_commit(loan, 64)
            });
            black_box(outcome)
        });
    });
}

/// REQ: REQ-CORE-040, REQ-QAL-012 — exhaustion/backpressure path cost: a
/// `Reject` topic whose required consumer has not acknowledged returns
/// `QueueFull` immediately (never blocks) and touches no allocator. The channel
/// footprint stays inside the declared memory budget (memory-budget.md I-MB1).
fn bench_exhaustion(c: &mut Criterion) {
    let ch = match Channel::activate(8, 16, 64, 8, OverflowPolicy::Reject, 1, 0, None) {
        Ok(ch) => ch,
        Err(e) => unreachable!("activate: {e:?}"),
    };
    // Fill the history window so the required consumer gates retirement.
    for _ in 0..8 {
        let _ = ch.publish(&SAMPLE);
    }

    common::assert_zero_alloc("publish_queue_full", || {
        for _ in 0..1024 {
            let _ = ch.publish(&SAMPLE);
        }
    });

    let samples = common::measure(2048, || {
        let _ = ch.publish(&SAMPLE);
    });
    let p99 = common::percentile(&samples, 99);
    println!("core publish queue-full path: p99={p99:?}");
    assert!(
        p99 <= Duration::from_micros(50),
        "queue-full publish p99 {p99:?} exceeds the 50us budget"
    );
    assert_eq!(
        ch.publish(&SAMPLE),
        Err(drm_types::Status::QueueFull),
        "exhausted Reject topic returns QueueFull"
    );
    assert_eq!(ch.drop_count(), 0, "QueueFull is not a pool drop");

    c.bench_function("publish_queue_full", |b| {
        b.iter(|| black_box(ch.publish(black_box(&SAMPLE))));
    });
}

/// REQ: REQ-QAL-012 — zero-allocation + bounded-time invariants under load.
fn bench_invariants(c: &mut Criterion) {
    let ch = channel();
    let mut state = ch.reader();
    let mut buf = [0u8; 64];

    // Zero-allocation: a steady-state publish + read block allocates nothing.
    common::assert_zero_alloc("publish+latest", || {
        for _ in 0..1024 {
            let _ = ch.publish(&SAMPLE);
            let _ = ch.latest(&mut state, 0, &mut buf);
        }
    });
    common::assert_zero_alloc("publish+next", || {
        for _ in 0..1024 {
            let _ = ch.publish(&SAMPLE);
            let _ = ch.next(&mut state, 0, &mut buf);
        }
    });

    // Bounded-time: the p99 publish + read round trip stays under the budget.
    let samples = common::measure(2048, || {
        let _ = ch.publish(&SAMPLE);
        let _ = ch.latest(&mut state, 0, &mut buf);
    });
    let p50 = common::percentile(&samples, 50);
    let p99 = common::percentile(&samples, 99);
    println!("core publish+latest: p50={p50:?} p99={p99:?}");
    assert!(
        p99 <= Duration::from_micros(50),
        "publish+latest p99 {p99:?} exceeds the 50us budget"
    );
    assert!(
        matches!(ch.latest(&mut state, 0, &mut buf), Ok(ReadOutcome::Sample)),
        "latest must resolve a committed sample"
    );

    c.bench_function("invariant_publish_latest", |b| {
        b.iter(|| {
            let _ = ch.publish(&SAMPLE);
            let _ = ch.latest(&mut state, 0, &mut buf);
        });
    });
}

/// Runs every latency benchmark group.
fn run_benches() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_publish_read(&mut criterion);
    bench_borrow(&mut criterion);
    bench_exhaustion(&mut criterion);
    bench_invariants(&mut criterion);
    criterion.final_summary();
}

criterion_main!(run_benches);
