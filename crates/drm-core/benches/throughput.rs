//! Throughput benches for the ring and pool algebra: sequence-ring commit +
//! lookup and payload-pool slot writes. The same fixed methodology and in-bench
//! invariants as `latency.rs` apply.
//!
//! REQ: REQ-QAL-011 (throughput baseline), REQ-QAL-012 (in-bench invariants).

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_main};

use drm_core::channel::ccb::ChannelControlBlock;
use drm_core::channel::pool::{PoolLayout, slot_offset, write_checked};
use drm_core::ring::HistoryRing;

/// Ring depth for the algebra bench.
const DEPTH: u64 = 64;
/// Pool slot count.
const SLOTS: u64 = 64;
/// Pool slot stride (bytes).
const STRIDE: u64 = 64;
/// A 64-byte sample.
const SAMPLE: [u8; 64] = [0x3C; 64];

/// REQ: REQ-QAL-011 — sequence-ring commit and lookup throughput.
fn bench_ring(c: &mut Criterion) {
    let ring = match HistoryRing::new(DEPTH) {
        Ok(ring) => ring,
        Err(e) => unreachable!("ring: {e:?}"),
    };
    let ccb = ChannelControlBlock::zeroed();
    let mut seq: u64 = 0;

    c.bench_function("ring_commit", |b| {
        b.iter(|| {
            seq = seq.wrapping_add(1);
            match ring.preflight_commit(seq) {
                Ok(stamp) => {
                    let slot = u32::try_from(seq % DEPTH).unwrap_or(0);
                    ring.commit(seq, slot, 1, stamp);
                    black_box(seq)
                }
                Err(_) => black_box(0),
            }
        });
    });

    c.bench_function("ring_lookup", |b| {
        b.iter(|| {
            let entry = ring.lookup(&ccb, black_box(seq));
            black_box(entry.is_ok())
        });
    });
}

/// REQ: REQ-QAL-011 — payload-pool slot-write throughput.
fn bench_pool(c: &mut Criterion) {
    let layout = PoolLayout {
        base: 0,
        stride: STRIDE,
        count: SLOTS,
        header_size: 0,
        sample_max: STRIDE,
        slot_align: 8,
    };
    let arena_bytes = usize::try_from(STRIDE * SLOTS).unwrap_or(0);
    let mut arena = vec![0u8; arena_bytes];
    let mut slot: u64 = 0;

    c.bench_function("pool_write", |b| {
        b.iter(|| {
            let result = write_checked(&mut arena, &layout, slot, black_box(&SAMPLE));
            slot = (slot + 1) % SLOTS;
            black_box(result)
        });
    });

    c.bench_function("pool_offset", |b| {
        b.iter(|| {
            let offset = slot_offset(&layout, black_box(slot));
            black_box(offset)
        });
    });
}

/// REQ: REQ-QAL-012 — zero-allocation invariants for the algebra paths.
fn bench_invariants(c: &mut Criterion) {
    let ring = match HistoryRing::new(DEPTH) {
        Ok(ring) => ring,
        Err(e) => unreachable!("ring: {e:?}"),
    };
    let ccb = ChannelControlBlock::zeroed();
    let layout = PoolLayout {
        base: 0,
        stride: STRIDE,
        count: SLOTS,
        header_size: 0,
        sample_max: STRIDE,
        slot_align: 8,
    };
    let arena_bytes = usize::try_from(STRIDE * SLOTS).unwrap_or(0);
    let mut arena = vec![0u8; arena_bytes];
    let mut seq: u64 = 0;
    let mut slot: u64 = 0;

    common::assert_zero_alloc("ring_commit+lookup", || {
        for _ in 0..1024 {
            seq = seq.wrapping_add(1);
            if let Ok(stamp) = ring.preflight_commit(seq) {
                ring.commit(seq, u32::try_from(seq % DEPTH).unwrap_or(0), 1, stamp);
                let _ = ring.lookup(&ccb, seq);
            }
        }
    });
    common::assert_zero_alloc("pool_write", || {
        for _ in 0..1024 {
            let _ = write_checked(&mut arena, &layout, slot, &SAMPLE);
            slot = (slot + 1) % SLOTS;
        }
    });

    let samples = common::measure(2048, || {
        seq = seq.wrapping_add(1);
        if let Ok(stamp) = ring.preflight_commit(seq) {
            ring.commit(seq, u32::try_from(seq % DEPTH).unwrap_or(0), 1, stamp);
            let _ = ring.lookup(&ccb, seq);
        }
    });
    let p99 = common::percentile(&samples, 99);
    println!("core ring commit+lookup: p99={p99:?}");
    assert!(
        p99 <= Duration::from_micros(50),
        "ring commit+lookup p99 {p99:?} exceeds the 50us budget"
    );

    c.bench_function("invariant_ring", |b| {
        b.iter(|| {
            seq = seq.wrapping_add(1);
            if let Ok(stamp) = ring.preflight_commit(seq) {
                ring.commit(seq, u32::try_from(seq % DEPTH).unwrap_or(0), 1, stamp);
                let _ = ring.lookup(&ccb, seq);
            }
        });
    });
}

/// Runs every throughput benchmark group.
fn run_benches() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_ring(&mut criterion);
    bench_pool(&mut criterion);
    bench_invariants(&mut criterion);
    criterion.final_summary();
}

criterion_main!(run_benches);
