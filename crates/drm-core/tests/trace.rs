//! Trace-lane runtime suite (`docs/algorithms/ops/observability-plane.md`
//! §Invariants I-OB1/I-OB6, §Test Matrix `trace_never_blocks`,
//! `trace_lane_exhaustion`, `trace_drop_counter`; §Protocols compiled trace
//! pipeline dictionary round-trip).
//! REQ: REQ-OPS-014, REQ-OPS-015, REQ-OPS-017.

use std::sync::Arc;
use std::time::{Duration, Instant};

use drm_core::trace::{
    DictVersion, TRACE_ARG_BYTES, TraceDict, TraceEvent, TraceRingSet, trace_emit_now,
    trace_register_thread, trace_unregister_thread,
};

/// Builds a zero-arg event for the given id.
fn ev(id: u16) -> [u8; TRACE_ARG_BYTES] {
    let mut args = [0_u8; TRACE_ARG_BYTES];
    args[0] = u8::try_from(id & 0xFF).unwrap_or(0);
    args
}

/// REQ: REQ-OPS-015 — a stopped reader never stalls producers: each of
/// 1/2/4/16 producer threads performs exactly `N` bounded emits; the lane keeps
/// exactly its capacity and counts every excess event as a drop (`I-OB1`,
/// `I-OB6`). Tail cost is reported (bounded, no wait path).
#[test]
fn trace_never_blocks() {
    const CAP: u64 = 8;
    const N: u64 = 200_000;
    for producers in [1_usize, 2, 4, 16] {
        let set = Arc::new(
            TraceRingSet::new(producers, CAP, DictVersion::zero())
                .unwrap_or_else(|e| unreachable!("new: {e:?}")),
        );
        let mut handles = Vec::new();
        for _ in 0..producers {
            let set = Arc::clone(&set);
            handles.push(std::thread::spawn(move || {
                let _lane = trace_register_thread(&set).unwrap_or_else(|e| unreachable!("{e:?}"));
                let start = Instant::now();
                for i in 0..N {
                    trace_emit_now(&set, u16::try_from(i % 8).unwrap_or(0), ev(0));
                }
                let tail = start.elapsed();
                trace_unregister_thread();
                tail
            }));
        }
        for h in handles {
            let tail = h
                .join()
                .unwrap_or_else(|_| unreachable!("producer panicked"));
            assert!(
                tail < Duration::from_secs(60),
                "producers={producers} tail cost {tail:?} not bounded"
            );
        }
        let dropped: u64 = (0..producers)
            .filter_map(|i| set.lane(i).map(drm_core::trace::TraceLane::dropped))
            .sum();
        assert_eq!(
            dropped,
            u64::try_from(producers).unwrap_or(0) * (N - CAP),
            "producers={producers}: every excess emit counted once"
        );
        assert_eq!(set.no_lane_drops(), 0, "all threads were registered");
    }
}

/// REQ: REQ-OPS-017 — lane claiming is fixed at activation: once every lane is
/// claimed the next claim is `Busy` (no growth); an unbound emit drops via
/// `no_lane_drops`.
#[test]
fn trace_lane_exhaustion() {
    let set =
        TraceRingSet::new(2, 4, DictVersion::zero()).unwrap_or_else(|e| unreachable!("new: {e:?}"));
    let a = set.claim_free_lane();
    let b = set.claim_free_lane();
    assert!(a.is_some() && b.is_some());
    assert_eq!(set.claim_free_lane(), None, "no dynamic lane growth");
    set.release_lane(a.unwrap_or(0));
    assert!(set.claim_free_lane().is_some(), "released lane reclaimable");

    let before = set.no_lane_drops();
    set.emit_on(
        None,
        TraceEvent {
            id: 0,
            pad: 0,
            args: [0; TRACE_ARG_BYTES],
            ts_ns: 0,
        },
    );
    assert_eq!(set.no_lane_drops(), before + 1, "unbound emit counted");
}

/// REQ: REQ-OPS-015 — a full lane counts each excess event exactly once and
/// retains exactly its capacity (`I-OB6`).
#[test]
fn trace_drop_counter() {
    let set =
        TraceRingSet::new(1, 4, DictVersion::zero()).unwrap_or_else(|e| unreachable!("new: {e:?}"));
    let lane = set.claim_free_lane().unwrap_or(0);
    for i in 0..10_u16 {
        set.emit_on(
            Some(lane),
            TraceEvent {
                id: i,
                pad: 0,
                args: [0; TRACE_ARG_BYTES],
                ts_ns: u64::from(i),
            },
        );
    }
    let l = set.lane(lane).unwrap_or_else(|| unreachable!("lane"));
    assert_eq!(l.dropped(), 6, "10 emitted, 4 retained");
    let mut out = Vec::new();
    l.drain_into(&mut out);
    assert_eq!(out.len(), 4, "capacity retained");
    assert_eq!(out.first().map(|e| e.ts_ns), Some(0), "oldest retained");
    assert_eq!(out.last().map(|e| e.ts_ns), Some(3), "newest retained");
    assert_eq!(set.total_dropped(), 6);
}

/// REQ: REQ-OPS-014 — emit id + args then render text via the build-time
/// dictionary; an unknown id renders raw hex, never a wrong name.
#[test]
fn trace_dict_roundtrip() {
    let mut dict = TraceDict::new();
    let id = dict.intern("motor.command", "speed={} steer={}");
    let mut args = [0_u8; TRACE_ARG_BYTES];
    args[0..4].copy_from_slice(&42_u32.to_le_bytes());
    args[4..8].copy_from_slice(&7_u32.to_le_bytes());
    let event = TraceEvent {
        id,
        pad: 0,
        args,
        ts_ns: 123,
    };
    assert_eq!(dict.render(&event), "motor.command: speed=42 steer=7");
    assert_eq!(dict.intern("motor.command", "speed={} steer={}"), id);

    let unknown = TraceEvent {
        id: id.wrapping_add(1),
        pad: 0,
        args,
        ts_ns: 0,
    };
    let rendered = dict.render(&unknown);
    assert!(rendered.contains("dict version mismatch"), "{rendered}");
    assert!(!rendered.contains("motor.command"), "{rendered}");

    let v = dict.version();
    assert_eq!(v.format_version, drm_core::trace::TRACE_DICT_FORMAT_VERSION);
    assert_ne!(v.digest, [0_u8; 16]);
    assert_ne!(
        dict.version(),
        TraceDict::new().version(),
        "digest binds bytes"
    );
}
