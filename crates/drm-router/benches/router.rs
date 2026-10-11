//! Router loopback and lane/shaping benches. The loopback drives a frame
//! through a peer's egress lane and services it back out, reporting p50/p99;
//! the shaper and priority-queue algebra are timed separately. The same fixed
//! methodology and in-bench invariants as the core benches apply.
//!
//! REQ: REQ-QAL-011 (router p50/p99 baseline), REQ-QAL-012 (in-bench invariants).

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_main};

use drm_router::egress::{Classified, PeerEgress};
use drm_router::lane::LaneCaps;
use drm_router::priority::{PriorityClass, PriorityQueue};
use drm_router::shaping::TokenBucket;
use drm_router::transport::Reliability;

/// A lane item that names its own service class.
#[derive(Debug, Clone, Copy)]
struct Frame(PriorityClass);

impl Classified for Frame {
    fn class(&self) -> PriorityClass {
        self.0
    }
}

/// A connected peer egress with symmetric realtime/bulk caps.
fn egress() -> PeerEgress<Frame> {
    let mut peer = PeerEgress::new(LaneCaps::new([64, 64, 64], 64), Reliability::BestEffort);
    peer.connect();
    peer
}

/// REQ: REQ-QAL-011 — router lane loopback p50/p99 and throughput.
fn bench_loopback(c: &mut Criterion) {
    let mut peer = egress();
    // Warm the lane's backing storage so the measured loop is steady state.
    let _ = peer.send(Frame(PriorityClass::Control));
    let _ = peer.service();

    let samples = common::measure(4096, || {
        let _ = peer.send(Frame(PriorityClass::Control));
        let _ = peer.service();
    });
    let p50 = common::percentile(&samples, 50);
    let p99 = common::percentile(&samples, 99);
    println!("router lane loopback: p50={p50:?} p99={p99:?}");
    assert!(
        p99 <= Duration::from_micros(20),
        "loopback p99 {p99:?} exceeds the 20us budget"
    );

    c.bench_function("lane_loopback", |b| {
        b.iter(|| {
            let _ = peer.send(Frame(PriorityClass::Control));
            black_box(peer.service())
        });
    });
}

/// REQ: REQ-QAL-011 — shaper and priority-queue algebra throughput.
fn bench_shaping(c: &mut Criterion) {
    let mut bucket = TokenBucket::new(100_000_000, 1500);
    let mut now: u64 = 0;
    c.bench_function("token_bucket_try_consume", |b| {
        b.iter(|| {
            now = now.wrapping_add(1_000);
            black_box(bucket.try_consume(1024, now))
        });
    });

    let mut queue = PriorityQueue::<u64>::new(64);
    c.bench_function("priority_enqueue_pop", |b| {
        b.iter(|| {
            let _ = queue.enqueue(PriorityClass::Control, 1);
            black_box(queue.pop())
        });
    });
}

/// REQ: REQ-QAL-012 — zero-allocation + bounded-time invariants under load.
fn bench_invariants(c: &mut Criterion) {
    let mut peer = egress();
    let _ = peer.send(Frame(PriorityClass::Control));
    let _ = peer.service();

    common::assert_zero_alloc("lane_send_service", || {
        for _ in 0..1024 {
            let _ = peer.send(Frame(PriorityClass::Control));
            let _ = peer.service();
        }
    });

    let samples = common::measure(4096, || {
        let _ = peer.send(Frame(PriorityClass::Control));
        let _ = peer.service();
    });
    let p99 = common::percentile(&samples, 99);
    assert!(
        p99 <= Duration::from_micros(20),
        "lane send+service p99 {p99:?} exceeds the 20us budget"
    );

    c.bench_function("invariant_lane_loopback", |b| {
        b.iter(|| {
            let _ = peer.send(Frame(PriorityClass::Control));
            let _ = peer.service();
        });
    });
}

/// Runs every router benchmark group.
fn run_benches() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_loopback(&mut criterion);
    bench_shaping(&mut criterion);
    bench_invariants(&mut criterion);
    criterion.final_summary();
}

criterion_main!(run_benches);
