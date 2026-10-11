//! Two-peer mixed-class lane test (lanes-shaping.md §Test Matrix per-peer lane
//! isolation): two peers each own their own per-peer lanes — one via
//! [`PeerEgress`], one via [`LaneSocket`] — shaped by the per-link scheduler
//! that spans transports (REQ-ROUTE-034). Servicing one peer's classes never
//! perturbs the other peer's queues, and each peer drains its own lanes in
//! strict class order with FIFO inside a class.
//! REQ: REQ-ROUTE-012, REQ-ROUTE-013, REQ-ROUTE-034.

use drm_router::egress::{Classified, PeerEgress};
use drm_router::lane::{LaneCaps, LaneSocket};
use drm_router::priority::PriorityClass;
use drm_router::transport::Reliability;

/// Test payload: a value plus its declared service class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Msg {
    value: u32,
    class: PriorityClass,
}

impl Classified for Msg {
    fn class(&self) -> PriorityClass {
        self.class
    }
}

/// Builds a test [`Msg`] in `class` carrying `value`.
fn msg(class: PriorityClass, value: u32) -> Msg {
    Msg { value, class }
}

/// Sends `item` through a classifying peer egress, reaching `unreachable!` only
/// if the fixture invariant is violated (full lane or disconnected peer).
fn send(egress: &mut PeerEgress<Msg>, item: Msg) {
    egress
        .send(item)
        .unwrap_or_else(|e| unreachable!("send invariant violated: {e:?}"));
}

/// Enqueues `item` at the tail of `class` on a lane socket, reaching
/// `unreachable!` only if the fixture invariant is violated.
fn enqueue(socket: &mut LaneSocket<Msg>, class: PriorityClass, item: Msg) {
    socket
        .enqueue(class, item)
        .unwrap_or_else(|e| unreachable!("enqueue invariant violated: {e:?}"));
}

/// Services one item from a classifying peer egress (fixture invariant check).
fn one_egress(peer: &mut PeerEgress<Msg>) -> (PriorityClass, Msg) {
    peer.service()
        .unwrap_or_else(|| unreachable!("peer A item expected"))
}

/// Services one item from a lane socket (fixture invariant check).
fn one_socket(peer: &mut LaneSocket<Msg>) -> (PriorityClass, Msg) {
    peer.dequeue()
        .unwrap_or_else(|| unreachable!("peer B item expected"))
}

/// Drains every remaining item of a classifying peer egress in service order.
fn drain_egress(peer: &mut PeerEgress<Msg>) -> Vec<(PriorityClass, Msg)> {
    let mut out = Vec::new();
    while let Some(entry) = peer.service() {
        out.push(entry);
    }
    out
}

/// Drains every remaining item of a lane socket in service order.
fn drain_socket(peer: &mut LaneSocket<Msg>) -> Vec<(PriorityClass, Msg)> {
    let mut out = Vec::new();
    while let Some(entry) = peer.dequeue() {
        out.push(entry);
    }
    out
}

/// Per-class queue depths of a classifying peer egress, in class order.
fn depths_egress(peer: &PeerEgress<Msg>) -> [usize; 4] {
    [
        peer.class_len(PriorityClass::Safety),
        peer.class_len(PriorityClass::Control),
        peer.class_len(PriorityClass::Status),
        peer.class_len(PriorityClass::Diagnostics),
    ]
}

/// Per-class queue depths of a lane socket, in class order.
fn depths_socket(peer: &LaneSocket<Msg>) -> [usize; 4] {
    [
        peer.class_len(PriorityClass::Safety),
        peer.class_len(PriorityClass::Control),
        peer.class_len(PriorityClass::Status),
        peer.class_len(PriorityClass::Diagnostics),
    ]
}

/// Reference service order for peer A's interleaved mixed-class traffic.
fn expected_a() -> Vec<(PriorityClass, Msg)> {
    vec![
        (PriorityClass::Safety, msg(PriorityClass::Safety, 2)),
        (PriorityClass::Control, msg(PriorityClass::Control, 4)),
        (PriorityClass::Status, msg(PriorityClass::Status, 1)),
        (
            PriorityClass::Diagnostics,
            msg(PriorityClass::Diagnostics, 0),
        ),
        (
            PriorityClass::Diagnostics,
            msg(PriorityClass::Diagnostics, 3),
        ),
        (
            PriorityClass::Diagnostics,
            msg(PriorityClass::Diagnostics, 5),
        ),
    ]
}

/// Reference service order for peer B's mixed-class traffic.
fn expected_b() -> Vec<(PriorityClass, Msg)> {
    vec![
        (PriorityClass::Safety, msg(PriorityClass::Safety, 12)),
        (PriorityClass::Control, msg(PriorityClass::Control, 10)),
        (
            PriorityClass::Diagnostics,
            msg(PriorityClass::Diagnostics, 11),
        ),
    ]
}

/// REQ: REQ-ROUTE-012, REQ-ROUTE-013, REQ-ROUTE-034 — two peers each own
/// separate per-peer lanes; servicing one peer leaves the other's queue depths
/// untouched, and each peer drains its own lanes in strict class order with
/// FIFO inside a class.
#[test]
fn lane_mixed_class_two_peer() {
    // Peer A: a reliable ordered link exposed through the classifying egress.
    let caps_a = LaneCaps::new([4, 4, 4], 4);
    let mut peer_a: PeerEgress<Msg> = PeerEgress::new(caps_a, Reliability::Reliable);
    peer_a.connect();

    // Peer B: a best-effort link with its own caps; only reliable ordered
    // streams use ordered lanes (REQ-ROUTE-034), but the per-link scheduler
    // still shapes every transport through the same lane logic.
    let caps_b = LaneCaps::new([2, 2, 2], 8);
    let mut peer_b: LaneSocket<Msg> = LaneSocket::new(caps_b, Reliability::BestEffort);
    peer_b.connect();
    assert_eq!(peer_b.bulk_reliability(), Reliability::BestEffort);
    assert_eq!(peer_b.caps(), caps_b);

    // Peer A: bulk flood interleaved with every realtime class.
    for item in [
        msg(PriorityClass::Diagnostics, 0),
        msg(PriorityClass::Status, 1),
        msg(PriorityClass::Safety, 2),
        msg(PriorityClass::Diagnostics, 3),
        msg(PriorityClass::Control, 4),
        msg(PriorityClass::Diagnostics, 5),
    ] {
        send(&mut peer_a, item);
    }
    // Peer B: its own mixed classes with distinct values and caps.
    for item in [
        msg(PriorityClass::Control, 10),
        msg(PriorityClass::Diagnostics, 11),
        msg(PriorityClass::Safety, 12),
    ] {
        enqueue(&mut peer_b, item.class, item);
    }

    // Per-peer isolation: each peer reports only its own queue depth.
    assert_eq!(depths_egress(&peer_a), [1, 1, 1, 3]);
    assert_eq!(peer_a.len(), 6);
    assert_eq!(depths_socket(&peer_b), [1, 1, 0, 1]);
    assert_eq!(peer_b.len(), 3);

    // Interleave servicing: draining peer B's realtime class must leave peer
    // A's lanes intact, and vice versa.
    let b_first = one_socket(&mut peer_b);
    assert_eq!(
        b_first,
        (PriorityClass::Safety, msg(PriorityClass::Safety, 12))
    );
    assert_eq!(
        peer_a.class_len(PriorityClass::Safety),
        1,
        "peer B left A intact"
    );
    assert_eq!(peer_a.class_len(PriorityClass::Diagnostics), 3);

    let a_first = one_egress(&mut peer_a);
    assert_eq!(
        a_first,
        (PriorityClass::Safety, msg(PriorityClass::Safety, 2))
    );
    assert_eq!(
        peer_b.class_len(PriorityClass::Control),
        1,
        "peer A left B intact"
    );
    assert_eq!(peer_b.class_len(PriorityClass::Diagnostics), 1);

    // Each peer drains the remainder of its own lanes in class order.
    let mut a_served = vec![a_first];
    a_served.extend(drain_egress(&mut peer_a));
    let mut b_served = vec![b_first];
    b_served.extend(drain_socket(&mut peer_b));

    assert_eq!(
        a_served,
        expected_a(),
        "peer A: class order, FIFO inside a class"
    );
    assert_eq!(b_served, expected_b(), "peer B: class order, bulk last");
    assert!(peer_a.is_empty());
    assert!(peer_b.is_empty());
}
