//! Priority-saturation end-to-end test (lanes-shaping.md §Test Matrix:
//! `priority_saturation`, I-LS1/I-LS6): a synthetic bulk (`Diagnostics`) flood
//! interleaved with sporadic realtime items must not starve any realtime class —
//! after all enqueues, service returns every realtime class strictly before
//! bulk, FIFO inside each class. Deterministic: fixed pattern, no sleeps, no
//! RNG.
//! REQ: REQ-ROUTE-020, REQ-ROUTE-021.

use drm_router::egress::{Classified, PeerEgress};
use drm_router::lane::LaneCaps;
use drm_router::priority::PriorityClass;
use drm_router::transport::Reliability;

/// Number of bulk (`Diagnostics`) items in the flood.
const FLOOD: u32 = 1000;
/// A sporadic `Safety` item is injected every `SAFETY_STRIDE` ticks.
const SAFETY_STRIDE: u32 = 97;
/// A sporadic `Control` item is injected every `CONTROL_STRIDE` ticks.
const CONTROL_STRIDE: u32 = 89;
/// A sporadic `Status` item is injected every `STATUS_STRIDE` ticks.
const STATUS_STRIDE: u32 = 83;

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

/// REQ: REQ-ROUTE-020, REQ-ROUTE-021 — I-LS1/I-LS6: a bulk `Diagnostics` flood
/// interleaved with sporadic `Safety`/`Control`/`Status` items never starves a
/// realtime class; after all enqueues, service drains every realtime item
/// (strict class order, FIFO inside a class) strictly before any bulk item.
#[test]
fn priority_saturation() {
    let caps = LaneCaps::new([64, 64, 64], 2048);
    let mut egress: PeerEgress<Msg> = PeerEgress::new(caps, Reliability::Reliable);
    egress.connect();

    let mut exp_safety = Vec::new();
    let mut exp_control = Vec::new();
    let mut exp_status = Vec::new();
    let mut exp_bulk = Vec::new();

    for tick in 0..FLOOD {
        // Bulk flood: one diagnostics item every tick.
        send(&mut egress, msg(PriorityClass::Diagnostics, tick));
        exp_bulk.push(tick);
        // Sporadic realtime items interleaved into the flood.
        if tick % SAFETY_STRIDE == 0 {
            send(&mut egress, msg(PriorityClass::Safety, tick));
            exp_safety.push(tick);
        }
        if tick % CONTROL_STRIDE == 0 {
            send(&mut egress, msg(PriorityClass::Control, tick));
            exp_control.push(tick);
        }
        if tick % STATUS_STRIDE == 0 {
            send(&mut egress, msg(PriorityClass::Status, tick));
            exp_status.push(tick);
        }
    }

    assert!(!exp_safety.is_empty(), "safety traffic must be non-trivial");
    assert!(
        !exp_control.is_empty(),
        "control traffic must be non-trivial"
    );
    assert!(!exp_status.is_empty(), "status traffic must be non-trivial");

    // Every enqueued item is queued in its own class; the flood dropped none.
    assert_eq!(egress.class_len(PriorityClass::Safety), exp_safety.len());
    assert_eq!(egress.class_len(PriorityClass::Control), exp_control.len());
    assert_eq!(egress.class_len(PriorityClass::Status), exp_status.len());
    assert_eq!(egress.class_len(PriorityClass::Diagnostics), exp_bulk.len());
    let queued = exp_safety.len() + exp_control.len() + exp_status.len() + exp_bulk.len();
    assert_eq!(egress.len(), queued);

    // Drain deterministically and compare against the exact reference order.
    let mut served = Vec::new();
    while let Some((class, item)) = egress.service() {
        served.push((class, item.value));
    }

    let mut expected = Vec::new();
    expected.extend(exp_safety.iter().map(|&v| (PriorityClass::Safety, v)));
    expected.extend(exp_control.iter().map(|&v| (PriorityClass::Control, v)));
    expected.extend(exp_status.iter().map(|&v| (PriorityClass::Status, v)));
    expected.extend(exp_bulk.iter().map(|&v| (PriorityClass::Diagnostics, v)));
    assert_eq!(served, expected, "class order with FIFO inside each class");

    // I-LS1/I-LS6: no realtime item starved — the flood is fully served last.
    let realtime = exp_safety.len() + exp_control.len() + exp_status.len();
    let first_bulk = served
        .iter()
        .position(|(class, _)| *class == PriorityClass::Diagnostics)
        .unwrap_or_else(|| unreachable!("bulk flood must surface a diagnostics item"));
    assert_eq!(
        first_bulk, realtime,
        "every realtime item drains before bulk"
    );
    assert!(egress.is_empty());
}
