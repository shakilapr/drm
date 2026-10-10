//! Per-peer egress classification and lane integration tests
//! (lanes-shaping.md §Test Matrix): items are classified into the right lane,
//! service is strict class order then FIFO inside a class, and a two-lane
//! (realtime + bulk) end-to-end exchange preserves realtime-before-bulk.
//! REQ: REQ-ROUTE-013, REQ-ROUTE-012, REQ-ROUTE-020, REQ-ROUTE-021.

use drm_router::egress::{Classified, PeerEgress};
use drm_router::lane::LaneCaps;
use drm_router::priority::PriorityClass;
use drm_router::transport::Reliability;
use drm_types::Status;

/// Test payload: a value plus its declared service class. The class is what
/// [`Classified`] exposes to the per-peer egress.
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

/// Sends `item`, reaching `unreachable!` only if the fixture invariant is
/// violated (full lane or disconnected peer); tests never `unwrap`.
fn send(egress: &mut PeerEgress<Msg>, item: Msg) {
    egress
        .send(item)
        .unwrap_or_else(|e| unreachable!("send invariant violated: {e:?}"));
}

/// REQ: REQ-ROUTE-021 — `send` classifies each item by its declared class and
/// enqueues it into that class's lane; `class_len`/`len` report the matching
/// lane.
#[test]
fn lane_egress_classification_into_lane() {
    let caps = LaneCaps::new([4, 4, 4], 4);
    let mut egress: PeerEgress<Msg> = PeerEgress::new(caps, Reliability::BestEffort);
    egress.connect();
    send(&mut egress, msg(PriorityClass::Safety, 1));
    send(&mut egress, msg(PriorityClass::Diagnostics, 2));
    send(&mut egress, msg(PriorityClass::Control, 3));
    send(&mut egress, msg(PriorityClass::Safety, 4));

    assert_eq!(egress.class_len(PriorityClass::Safety), 2);
    assert_eq!(egress.class_len(PriorityClass::Control), 1);
    assert_eq!(egress.class_len(PriorityClass::Status), 0);
    assert_eq!(egress.class_len(PriorityClass::Diagnostics), 1);
    assert_eq!(egress.len(), 4);
    assert!(!egress.is_empty());
}

/// REQ: REQ-ROUTE-020, REQ-ROUTE-021 — `service` drains in strict class order
/// (safety → control → status → diagnostics) with FIFO inside each class,
/// regardless of enqueue interleaving.
#[test]
fn lane_egress_service_strict_class_order() {
    let caps = LaneCaps::new([8, 8, 8], 8);
    let mut egress: PeerEgress<Msg> = PeerEgress::new(caps, Reliability::BestEffort);
    egress.connect();
    send(&mut egress, msg(PriorityClass::Diagnostics, 10));
    send(&mut egress, msg(PriorityClass::Control, 1));
    send(&mut egress, msg(PriorityClass::Safety, 0));
    send(&mut egress, msg(PriorityClass::Status, 5));
    send(&mut egress, msg(PriorityClass::Control, 2));
    send(&mut egress, msg(PriorityClass::Safety, 9));
    send(&mut egress, msg(PriorityClass::Diagnostics, 11));

    let mut served = Vec::new();
    while let Some((class, item)) = egress.service() {
        served.push((class, item.value));
    }
    assert_eq!(
        served,
        vec![
            (PriorityClass::Safety, 0),
            (PriorityClass::Safety, 9),
            (PriorityClass::Control, 1),
            (PriorityClass::Control, 2),
            (PriorityClass::Status, 5),
            (PriorityClass::Diagnostics, 10),
            (PriorityClass::Diagnostics, 11),
        ]
    );
    assert!(egress.is_empty());
}

/// REQ: REQ-ROUTE-013, REQ-ROUTE-012, REQ-ROUTE-020 — an in-memory exchange
/// between two connected peers moves every item through the sender's realtime
/// and bulk lanes into the receiver's lanes; the receiver observes strict
/// class order (realtime before bulk, FIFO inside a class) and loses nothing.
#[test]
fn lane_egress_two_lane_exchange() {
    let caps = LaneCaps::new([4, 4, 4], 4);
    let mut sender: PeerEgress<Msg> = PeerEgress::new(caps, Reliability::Reliable);
    let mut receiver: PeerEgress<Msg> = PeerEgress::new(caps, Reliability::Reliable);
    sender.connect();
    receiver.connect();

    // Interleave realtime-lane classes with the bulk (diagnostics) lane.
    for item in [
        msg(PriorityClass::Diagnostics, 0),
        msg(PriorityClass::Status, 1),
        msg(PriorityClass::Safety, 2),
        msg(PriorityClass::Diagnostics, 3),
        msg(PriorityClass::Control, 4),
    ] {
        send(&mut sender, item);
    }

    // In-memory link: service the sender strictly, forward onto the receiver.
    while let Some((_, item)) = sender.service() {
        send(&mut receiver, item);
    }

    let mut delivered = Vec::new();
    while let Some((class, item)) = receiver.service() {
        delivered.push((class, item.value));
    }
    assert_eq!(
        delivered,
        vec![
            (PriorityClass::Safety, 2),
            (PriorityClass::Control, 4),
            (PriorityClass::Status, 1),
            (PriorityClass::Diagnostics, 0),
            (PriorityClass::Diagnostics, 3),
        ],
        "realtime drains before bulk, FIFO inside a class"
    );
    assert!(sender.is_empty());
    assert!(receiver.is_empty());
}

/// REQ: REQ-ROUTE-013, REQ-ROUTE-020 — a disconnected peer refuses classified
/// sends with `NotReady` (never a silent accept); `disconnect` surfaces queued
/// realtime items as ended in class-then-FIFO service order.
#[test]
fn lane_egress_disconnected_send_refused() {
    let caps = LaneCaps::new([2, 2, 2], 2);
    let mut egress: PeerEgress<Msg> = PeerEgress::new(caps, Reliability::Reliable);
    assert_eq!(
        egress.send(msg(PriorityClass::Safety, 1)),
        Err(Status::NotReady)
    );

    egress.connect();
    send(&mut egress, msg(PriorityClass::Diagnostics, 7));
    send(&mut egress, msg(PriorityClass::Control, 8));
    let report = egress.disconnect();
    assert_eq!(
        report.realtime_ended,
        vec![(PriorityClass::Control, msg(PriorityClass::Control, 8))]
    );
    assert!(egress.is_empty());
}
