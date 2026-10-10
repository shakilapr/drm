//! Per-peer lane tests (work-plan Phase 30 seed, lanes-shaping.md
//! §Data Structures/Test Matrix): FIFO within a class, strict class service
//! order, realtime-never-behind-bulk, and static cap enforcement.
//! REQ: REQ-ROUTE-013, REQ-ROUTE-012, REQ-ROUTE-020, REQ-ROUTE-021.

use std::collections::VecDeque;

use drm_router::lane::{
    BulkDisposition, LaneCaps, LaneSocket, LaneState, PeerLanes, REALTIME_CLASSES, realtime_index,
};
use drm_router::priority::PriorityClass;
use drm_router::transport::Reliability;
use drm_types::Status;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

fn enqueue<T>(lanes: &mut PeerLanes<T>, class: PriorityClass, item: T, what: &str) {
    fixture(lanes.enqueue(class, item), what);
}

/// REQ: REQ-ROUTE-021 — I-LS2: FIFO order is preserved inside a class; every
/// class keeps its own sequence.
#[test]
fn lane_fifo_within_class() {
    let caps = LaneCaps::new([8, 8, 8], 8);
    let mut lanes = PeerLanes::new(caps);
    for i in 0..5_u32 {
        enqueue(&mut lanes, PriorityClass::Control, i, "control");
    }
    for i in 100..103_u32 {
        enqueue(&mut lanes, PriorityClass::Status, i, "status");
    }
    for expected in 0..5_u32 {
        match lanes.dequeue() {
            Some((PriorityClass::Control, v)) => assert_eq!(v, expected),
            other => unreachable!("expected control {expected}, got {other:?}"),
        }
    }
    for expected in 100..103_u32 {
        match lanes.dequeue() {
            Some((PriorityClass::Status, v)) => assert_eq!(v, expected),
            other => unreachable!("expected status {expected}, got {other:?}"),
        }
    }
    assert!(lanes.is_empty());
}

/// REQ: REQ-ROUTE-020 — strict class service order (safety → control → status)
/// with FIFO inside each class, even when enqueued interleaved.
#[test]
fn lane_priority_order() {
    let caps = LaneCaps::new([8, 8, 8], 8);
    let mut lanes = PeerLanes::new(caps);
    enqueue(&mut lanes, PriorityClass::Diagnostics, "d1", "diag");
    enqueue(&mut lanes, PriorityClass::Control, "c1", "control");
    enqueue(&mut lanes, PriorityClass::Safety, "s1", "safety");
    enqueue(&mut lanes, PriorityClass::Status, "st1", "status");
    enqueue(&mut lanes, PriorityClass::Control, "c2", "control");
    enqueue(&mut lanes, PriorityClass::Safety, "s2", "safety");
    enqueue(&mut lanes, PriorityClass::Diagnostics, "d2", "diag");

    let mut order = Vec::new();
    while let Some(entry) = lanes.dequeue() {
        order.push(entry);
    }
    assert_eq!(
        order,
        vec![
            (PriorityClass::Safety, "s1"),
            (PriorityClass::Safety, "s2"),
            (PriorityClass::Control, "c1"),
            (PriorityClass::Control, "c2"),
            (PriorityClass::Status, "st1"),
            (PriorityClass::Diagnostics, "d1"),
            (PriorityClass::Diagnostics, "d2"),
        ]
    );
}

/// REQ: REQ-ROUTE-012 — bulk never drains ahead of a non-empty realtime class
/// (realtime and bulk are separate lanes, never cross-class queueing).
#[test]
fn lane_no_cross_class_queueing() {
    let caps = LaneCaps::new([4, 4, 4], 4);
    let mut lanes = PeerLanes::new(caps);
    enqueue(&mut lanes, PriorityClass::Diagnostics, "bulk", "diag");
    assert_eq!(lanes.dequeue(), Some((PriorityClass::Diagnostics, "bulk")));

    enqueue(&mut lanes, PriorityClass::Diagnostics, "bulk", "diag");
    enqueue(&mut lanes, PriorityClass::Safety, "rt", "safety");
    // A later realtime enqueue is still served before the older bulk item.
    assert_eq!(lanes.dequeue(), Some((PriorityClass::Safety, "rt")));
    assert_eq!(lanes.dequeue(), Some((PriorityClass::Diagnostics, "bulk")));

    // Diagnostics is the only class mapped to the bulk lane.
    assert_eq!(realtime_index(PriorityClass::Diagnostics), None);
    for class in [
        PriorityClass::Safety,
        PriorityClass::Control,
        PriorityClass::Status,
    ] {
        assert!(realtime_index(class).is_some());
    }
    assert_eq!(REALTIME_CLASSES, 3);
}

/// REQ: REQ-ROUTE-013 — I-LS5: static caps are enforced; a full realtime class
/// returns `QueueFull` (never a silent drop) and the bulk lane applies
/// backpressure at its own cap. `class_len`/`len` track both lanes.
#[test]
fn lane_caps_enforced() {
    let caps = LaneCaps::new([2, 3, 1], 2);
    let mut lanes = PeerLanes::new(caps);
    assert_eq!(caps.cap_for(PriorityClass::Safety), 2);
    assert_eq!(caps.cap_for(PriorityClass::Control), 3);
    assert_eq!(caps.cap_for(PriorityClass::Status), 1);
    assert_eq!(caps.cap_for(PriorityClass::Diagnostics), 2);

    enqueue(&mut lanes, PriorityClass::Safety, 1_u32, "safety");
    enqueue(&mut lanes, PriorityClass::Safety, 2, "safety");
    assert_eq!(
        lanes.enqueue(PriorityClass::Safety, 3),
        Err(Status::QueueFull),
        "realtime class never silently drops"
    );
    assert_eq!(lanes.class_len(PriorityClass::Safety), 2);

    enqueue(&mut lanes, PriorityClass::Diagnostics, 10, "diag");
    enqueue(&mut lanes, PriorityClass::Diagnostics, 11, "diag");
    assert_eq!(
        lanes.enqueue(PriorityClass::Diagnostics, 12),
        Err(Status::QueueFull),
        "bulk backpressure at cap"
    );
    assert_eq!(lanes.class_len(PriorityClass::Diagnostics), 2);
    assert_eq!(lanes.len(), 4);

    assert_eq!(lanes.dequeue(), Some((PriorityClass::Safety, 1)));
    assert_eq!(lanes.dequeue(), Some((PriorityClass::Safety, 2)));
    assert_eq!(lanes.dequeue(), Some((PriorityClass::Diagnostics, 10)));
    assert_eq!(lanes.dequeue(), Some((PriorityClass::Diagnostics, 11)));
    assert_eq!(lanes.dequeue(), None);
    assert!(lanes.is_empty());
}

/// Deterministic per-case RNG (xorshift64* over a hashed case name).
fn rng(seed: &str) -> impl FnMut() -> u64 + use<> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    let mut x = h.finish() | 1;
    move || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        x
    }
}

/// REQ: REQ-ROUTE-021, REQ-ROUTE-020 — I-LS2 property: over randomized
/// enqueue/dequeue traffic the lane reproduces exactly the per-class FIFO
/// reference model, with strict class service order.
#[test]
fn lane_fifo_property() {
    let caps = LaneCaps::new([6, 6, 6], 6);
    for case in 0..2_000 {
        let mut r = rng(&format!("lane-{case}"));
        let mut lanes: PeerLanes<u64> = PeerLanes::new(caps);
        let mut model: [VecDeque<u64>; 4] = std::array::from_fn(|_| VecDeque::new());
        let mut next = 0_u64;
        for _ in 0..64 {
            let op = r() % 2;
            let class = match r() % 4 {
                0 => PriorityClass::Safety,
                1 => PriorityClass::Control,
                2 => PriorityClass::Status,
                _ => PriorityClass::Diagnostics,
            };
            if op == 0 {
                let cap = caps.cap_for(class);
                let full = model[class.index()].len() >= cap;
                let got = lanes.enqueue(class, next);
                if full {
                    assert_eq!(got, Err(Status::QueueFull), "case {case}: full cap");
                } else {
                    assert_eq!(got, Ok(()), "case {case}: room");
                    model[class.index()].push_back(next);
                    next += 1;
                }
            } else {
                let expected = dequeue_model(&mut model);
                let got = lanes.dequeue();
                assert_eq!(got, expected, "case {case}: dequeue mismatch");
            }
        }
        while let Some(expected) = dequeue_model(&mut model) {
            assert_eq!(lanes.dequeue(), Some(expected), "case {case}: drain");
        }
        assert!(lanes.is_empty());
    }
}

/// Reference drain: highest non-empty realtime class (class order), then bulk.
fn dequeue_model(model: &mut [VecDeque<u64>; 4]) -> Option<(PriorityClass, u64)> {
    for class in [
        PriorityClass::Safety,
        PriorityClass::Control,
        PriorityClass::Status,
    ] {
        if let Some(v) = model[class.index()].pop_front() {
            return Some((class, v));
        }
    }
    model[PriorityClass::Diagnostics.index()]
        .pop_front()
        .map(|v| (PriorityClass::Diagnostics, v))
}

fn enqueue_socket<T>(sock: &mut LaneSocket<T>, class: PriorityClass, item: T, what: &str) {
    fixture(sock.enqueue(class, item), what);
}

/// REQ: REQ-ROUTE-013 — a fresh peer socket is disconnected; `connect` flips to
/// connected without creating or draining any queued item, and `disconnect`
/// returns to disconnected. Both transitions are idempotent.
#[test]
fn lane_socket_connect_disconnect_transition() {
    let caps = LaneCaps::new([4, 4, 4], 4);
    let mut sock: LaneSocket<u32> = LaneSocket::new(caps, Reliability::BestEffort);
    assert_eq!(sock.state(), LaneState::Disconnected);
    assert!(sock.is_empty());
    // Disconnected enqueue is refused, not silently accepted.
    assert_eq!(
        sock.enqueue(PriorityClass::Safety, 1),
        Err(Status::NotReady)
    );
    assert_eq!(sock.dequeue(), None);

    sock.connect();
    assert_eq!(sock.state(), LaneState::Connected);
    // Connecting creates nothing implicit.
    assert!(sock.is_empty());
    sock.connect();
    assert_eq!(sock.state(), LaneState::Connected);

    enqueue_socket(&mut sock, PriorityClass::Safety, 7, "safety");
    assert_eq!(sock.state(), LaneState::Connected);
    assert_eq!(sock.class_len(PriorityClass::Safety), 1);

    let report = sock.disconnect();
    assert_eq!(sock.state(), LaneState::Disconnected);
    assert_eq!(report.realtime_ended, vec![(PriorityClass::Safety, 7)]);
    assert!(sock.is_empty());
    // Idempotent: a second disconnect reports nothing.
    let empty = sock.disconnect();
    assert!(empty.realtime_ended.is_empty());
    assert_eq!(empty.bulk, BulkDisposition::Dropped { n: 0 });
}

/// REQ: REQ-ROUTE-013, REQ-ROUTE-020 — disconnect surfaces realtime items as
/// ended in class-then-FIFO service order (never silently dropped, I-LS5);
/// best-effort bulk is dropped-and-counted.
#[test]
fn lane_socket_disconnect_disposition_best_effort() {
    let caps = LaneCaps::new([4, 4, 4], 4);
    let mut sock: LaneSocket<u32> = LaneSocket::new(caps, Reliability::BestEffort);
    sock.connect();
    enqueue_socket(&mut sock, PriorityClass::Diagnostics, 100, "diag");
    enqueue_socket(&mut sock, PriorityClass::Control, 1, "control");
    enqueue_socket(&mut sock, PriorityClass::Safety, 0, "safety");
    enqueue_socket(&mut sock, PriorityClass::Control, 2, "control");
    enqueue_socket(&mut sock, PriorityClass::Diagnostics, 101, "diag");

    let report = sock.disconnect();
    assert_eq!(
        report.realtime_ended,
        vec![
            (PriorityClass::Safety, 0),
            (PriorityClass::Control, 1),
            (PriorityClass::Control, 2),
        ],
        "realtime surfaced as ended in class-then-FIFO order"
    );
    assert_eq!(report.bulk, BulkDisposition::Dropped { n: 2 });
    assert_eq!(sock.bulk_dropped(), 2);
    assert!(sock.is_empty());
}

/// REQ: REQ-ROUTE-013 — for a reliable bulk lane the disconnect surfaces the
/// queued bulk items as ended (a gone peer cannot absorb backpressure) and
/// counts no drops.
#[test]
fn lane_socket_disconnect_disposition_reliable_bulk() {
    let caps = LaneCaps::new([4, 4, 4], 4);
    let mut sock: LaneSocket<u32> = LaneSocket::new(caps, Reliability::Reliable);
    assert_eq!(sock.bulk_reliability(), Reliability::Reliable);
    sock.connect();
    enqueue_socket(&mut sock, PriorityClass::Diagnostics, 10, "diag");
    enqueue_socket(&mut sock, PriorityClass::Diagnostics, 11, "diag");
    enqueue_socket(&mut sock, PriorityClass::Status, 5, "status");

    let report = sock.disconnect();
    assert_eq!(report.realtime_ended, vec![(PriorityClass::Status, 5)]);
    assert_eq!(report.bulk, BulkDisposition::Ended(vec![10, 11]));
    assert_eq!(sock.bulk_dropped(), 0);
    assert!(sock.is_empty());
}

/// REQ: REQ-ROUTE-021, REQ-ROUTE-020, REQ-ROUTE-013 — while connected the
/// socket preserves per-class FIFO (I-LS2), strict class service order, and the
/// static caps (I-LS5): a full realtime class returns `QueueFull`.
#[test]
fn lane_socket_caps_and_fifo_while_connected() {
    let caps = LaneCaps::new([2, 3, 1], 2);
    let mut sock: LaneSocket<u32> = LaneSocket::new(caps, Reliability::BestEffort);
    sock.connect();
    assert_eq!(sock.caps(), caps);
    assert_eq!(sock.caps().cap_for(PriorityClass::Safety), 2);
    assert_eq!(sock.caps().cap_for(PriorityClass::Diagnostics), 2);

    enqueue_socket(&mut sock, PriorityClass::Safety, 1, "safety");
    enqueue_socket(&mut sock, PriorityClass::Safety, 2, "safety");
    assert_eq!(
        sock.enqueue(PriorityClass::Safety, 3),
        Err(Status::QueueFull),
        "realtime class never silently drops while connected"
    );
    assert_eq!(sock.class_len(PriorityClass::Safety), 2);

    enqueue_socket(&mut sock, PriorityClass::Status, 9, "status");
    assert_eq!(
        sock.enqueue(PriorityClass::Status, 8),
        Err(Status::QueueFull),
        "status class at static cap"
    );
    enqueue_socket(&mut sock, PriorityClass::Diagnostics, 20, "diag");
    enqueue_socket(&mut sock, PriorityClass::Diagnostics, 21, "diag");
    assert_eq!(
        sock.enqueue(PriorityClass::Diagnostics, 22),
        Err(Status::QueueFull),
        "bulk backpressure at cap"
    );
    assert_eq!(sock.len(), 5);

    assert_eq!(sock.dequeue(), Some((PriorityClass::Safety, 1)));
    assert_eq!(sock.dequeue(), Some((PriorityClass::Safety, 2)));
    assert_eq!(sock.dequeue(), Some((PriorityClass::Status, 9)));
    assert_eq!(sock.dequeue(), Some((PriorityClass::Diagnostics, 20)));
    assert_eq!(sock.dequeue(), Some((PriorityClass::Diagnostics, 21)));
    assert_eq!(sock.dequeue(), None);
    assert!(sock.is_empty());
}
