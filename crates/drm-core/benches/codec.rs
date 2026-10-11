//! Canonical-codec throughput bench: fixed-width primitive read/write, the
//! two-region descriptor check (`body_range`), and whole-sample validation
//! (`validate_sample`). The codec path must be zero-allocation and bounded
//! (canon-codec.md I4/I5); both invariants are asserted inside the bench and
//! fail it on violation.
//!
//! REQ: REQ-QAL-011 (codec throughput target), REQ-QAL-012 (in-bench invariants),
//! REQ-QAL-019 (bench-harness gating), REQ-TOOL-043 (LE primitives), REQ-CORE-082
//! (codec path zero allocation).

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_main};

use drm_core::canon::{
    CanonLayout, Descriptor, body_range, read_u32, read_u64, validate_sample, write_u32, write_u64,
};

/// p99 budget for one codec round trip. The codec operates on already-mapped
/// bytes with no serialization loop; 5 µs is a hard ceiling well above the
/// measured nanosecond-scale cost (canon-codec.md §Functions, bench CI gate).
const CODEC_P99_BUDGET: Duration = Duration::from_micros(5);

/// A valid two-region layout: 8-byte header + one `bytes[<=16]` body at offset 8.
/// REQ: REQ-TOOL-042.
fn layout() -> CanonLayout {
    CanonLayout {
        header_end: 8,
        sample_max: 64,
        struct_align: 4,
        descriptors: vec![Descriptor {
            count: 4,
            max: 16,
            offset: 8,
            header_end: 8,
            elem_size: 1,
            elem_align: 1,
        }],
    }
}

/// REQ: REQ-QAL-011, REQ-TOOL-043 — primitive LE write/read throughput.
fn bench_primitives(c: &mut Criterion) {
    let mut buf = [0u8; 64];
    let mut v: u64 = 0;

    c.bench_function("codec_u32_roundtrip", |b| {
        b.iter(|| {
            write_u32(&mut buf, 0, black_box(u32::try_from(v).unwrap_or(u32::MAX))).unwrap_or(());
            black_box(read_u32(&buf, 0))
        });
    });

    c.bench_function("codec_u64_roundtrip", |b| {
        b.iter(|| {
            v = v.wrapping_add(1);
            write_u64(&mut buf, 0, black_box(v)).unwrap_or(());
            black_box(read_u64(&buf, 0))
        });
    });
}

/// REQ: REQ-QAL-011 — two-region descriptor and whole-sample validation cost.
fn bench_validate(c: &mut Criterion) {
    let l = layout();
    let buf = [0u8; 64];
    let d = l.descriptors[0];

    c.bench_function("codec_body_range", |b| {
        b.iter(|| black_box(body_range(black_box(64), &d, black_box(12))));
    });

    c.bench_function("codec_validate_sample", |b| {
        b.iter(|| black_box(validate_sample(black_box(&buf), &l, black_box(12))));
    });
}

/// REQ: REQ-QAL-012 — zero-allocation + bounded-time invariants under load.
fn bench_invariants(c: &mut Criterion) {
    let l = layout();
    let mut buf = [0u8; 64];
    let d = l.descriptors[0];
    let mut v: u64 = 0;

    common::assert_zero_alloc("codec_primitive_roundtrip", || {
        for _ in 0..1024 {
            v = v.wrapping_add(1);
            write_u64(&mut buf, 0, v).unwrap_or(());
            let _ = read_u64(&buf, 0);
        }
    });
    common::assert_zero_alloc("codec_validate_sample", || {
        for _ in 0..1024 {
            let _ = validate_sample(&buf, &l, 12);
        }
    });

    let samples = common::measure(4096, || {
        v = v.wrapping_add(1);
        write_u64(&mut buf, 0, v).unwrap_or(());
        let _ = read_u64(&buf, 0);
        let _ = body_range(64, &d, 12);
        let _ = validate_sample(&buf, &l, 12);
    });
    let p99 = common::percentile(&samples, 99);
    println!("canon codec round trip: p99={p99:?}");
    assert!(
        p99 <= CODEC_P99_BUDGET,
        "codec round trip p99 {p99:?} exceeds the {CODEC_P99_BUDGET:?} budget"
    );

    c.bench_function("invariant_codec_roundtrip", |b| {
        b.iter(|| {
            v = v.wrapping_add(1);
            write_u64(&mut buf, 0, v).unwrap_or(());
            black_box(read_u64(&buf, 0))
        });
    });
}

/// Runs every codec benchmark group.
fn run_benches() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_primitives(&mut criterion);
    bench_validate(&mut criterion);
    bench_invariants(&mut criterion);
    criterion.final_summary();
}

criterion_main!(run_benches);
