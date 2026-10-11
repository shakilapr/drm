//! Lockless dynamic-edge slot coherence suite (`cargo test -p drm-spatial slot`):
//! the single-writer seqlock never yields torn geometry — a concurrent reader
//! observes a coherent snapshot or `ConcurrentUpdate`, and the protocol is
//! model-checked over **all** interleavings ([spatial-queries.md] §Invariants
//! `I-Q14`, §Protocols slot write/snapshot;
//! [`spatial_architecture.md`] §Versioned Local Cache).
//!
//! REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.

use drm_spatial::{
    ClockDomain, ClockIdentity, EdgeSample, SpatialEdgeCache, TimePoint, Transform3,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// A host-monotonic [`TimePoint`] at `ns`.
/// REQ: REQ-SPATIAL-035.
fn host(ns: u64) -> TimePoint {
    TimePoint::new(ns, ClockIdentity::new(ClockDomain::HostMonotonic))
}

/// REQ: REQ-SPATIAL-050 — `spatial_slot_coherence`: a single writer publishing
/// two distinct transforms while readers spin observes only coherent snapshots
/// (`A` or `B`, never a mix), an even stable version, and no torn geometry
/// (`I-Q14`). The slot is seeded with `A` first: an unwritten slot is the
/// distinct empty state (version `0`, uninitialized payload), which is not a
/// published dual-epoch sample.
#[test]
fn spatial_slot_coherence() {
    let a = Transform3 {
        translation: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };
    let b = Transform3 {
        translation: [-4.0, -5.0, -6.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };
    let cache = SpatialEdgeCache::new(1);
    let claim = cache
        .claim_writer(0, 1)
        .unwrap_or_else(|e| unreachable!("{e}"));
    // Seed a valid sample so every subsequent read observes a published epoch
    // (version > 0), never the empty slot.
    cache
        .publish_sample(&claim, &EdgeSample::new(a, host(0), host(0)))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let torn = AtomicU64::new(0);
    let reads = AtomicU64::new(0);

    std::thread::scope(|scope| {
        let writer = &cache;
        let wclaim = claim;
        scope.spawn(move || {
            for k in 0..50_000_u64 {
                let t = if k % 2 == 0 { &a } else { &b };
                let _ = writer.publish_sample(&wclaim, &EdgeSample::new(*t, host(k), host(k)));
            }
        });
        for _ in 0..4 {
            let reader = &cache;
            let torn = &torn;
            let reads = &reads;
            scope.spawn(move || {
                for _ in 0..100_000_u64 {
                    if let Some(s) = reader.read(0) {
                        reads.fetch_add(1, Ordering::Relaxed);
                        assert!(s.version > 0, "a published slot has version > 0");
                        assert_eq!(s.version % 2, 0, "stable version must be even");
                        let ok = s.transform == a || s.transform == b;
                        if !ok {
                            torn.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });

    assert_eq!(torn.load(Ordering::Relaxed), 0, "no torn geometry observed");
    assert!(reads.load(Ordering::Relaxed) > 0, "readers made progress");
}

/// The seqlock protocol as a tiny explicitly-enumerated state machine used by
/// [`spatial_slot_model`]. Fields `f1`/`f2` are separate atomics so a reader
/// that accepts a `f1`/`f2` from different writer epochs models torn geometry.
/// REQ: REQ-SPATIAL-050.
#[derive(Clone, Copy)]
struct ModelState {
    version: i32,
    f1: u8,
    f2: u8,
    r_v1: i32,
    r_f1: u8,
    r_f2: u8,
    r_v2: i32,
    w_done: u8,
    r_done: u8,
}

/// Depth-first enumeration of every interleaving of the writer program
/// (`W0` store odd; `W1a` f1:=new; `W1b` f2:=new; `W2` store even) and reader
/// program (`R0` v1:=version; `R1` f1,f2:=fields; `R2` v2:=version), preserving
/// each thread's program order. Counts accepted-but-torn snapshots.
/// REQ: REQ-SPATIAL-050.
fn enumerate(state: ModelState, torn: &mut u64, accepted_clean: &mut u64) {
    // Terminal: both programs complete.
    if state.w_done == 4 && state.r_done == 3 {
        // A read is accepted only when the two version loads agree and are even.
        if state.r_v1 == state.r_v2 && state.r_v1 % 2 == 0 {
            let all_old = state.r_f1 == 0 && state.r_f2 == 0;
            let all_new = state.r_f1 == 1 && state.r_f2 == 1;
            if all_old || all_new {
                *accepted_clean += 1;
            } else {
                *torn += 1;
            }
        }
        return;
    }
    // Writer step (program order): version goes odd, then the two fields, then
    // the version goes even.
    if state.w_done < 4 {
        let mut s = state;
        match s.w_done {
            1 => s.f1 = 1,
            2 => s.f2 = 1,
            _ => s.version += 1,
        }
        s.w_done += 1;
        enumerate(s, torn, accepted_clean);
    }
    // Reader step (program order).
    if state.r_done < 3 {
        let mut s = state;
        match s.r_done {
            0 => s.r_v1 = s.version,
            1 => {
                s.r_f1 = s.f1;
                s.r_f2 = s.f2;
            }
            _ => {
                s.r_v2 = s.version;
            }
        }
        s.r_done += 1;
        enumerate(s, torn, accepted_clean);
    }
}

/// REQ: REQ-SPATIAL-050 — `spatial_slot_model`: exhaustive interleaving model of
/// the single-writer seqlock. Over every schedule, an accepted snapshot equals
/// the all-old or all-new field state — never a torn mix — and coherent reads
/// are possible, so the protocol is both safe and non-vacuous (`I-Q14`).
#[test]
fn spatial_slot_model() {
    let init = ModelState {
        version: 0,
        f1: 0,
        f2: 0,
        r_v1: -1,
        r_f1: 2,
        r_f2: 2,
        r_v2: -1,
        w_done: 0,
        r_done: 0,
    };
    let mut torn = 0_u64;
    let mut clean = 0_u64;
    enumerate(init, &mut torn, &mut clean);
    assert_eq!(torn, 0, "no interleaving yields a torn accepted snapshot");
    assert!(clean > 0, "some interleaving yields a coherent read");
}
