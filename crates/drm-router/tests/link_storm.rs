//! Link-storm chaos suite (work-plan M18W1-E3-T2; router-core.md I-RT6):
//! rapid connect/disconnect × independent routers × links under a randomized
//! storm. Every step asserts the isolation invariant — a cut link emits
//! nothing and drops ingress without state, while sibling links keep flowing —
//! and that a reconnect always resumes. The session-reliability cell flaps
//! `begin_session`/`end_session` and asserts monotonic generations with exact
//! gap accounting. `#[ignore]`d: run via `cargo xtask test --stress`
//! (`DRM_STRESS_ITERS`, default 100; ×200 steps per iteration).
//! REQ: REQ-QAL-005, REQ-QAL-010, REQ-ROUTE-007, REQ-ROUTE-011, REQ-ROUTE-022,
//! REQ-ROUTE-029.

use drm_router::data_path::{EgressRoute, ImportSink, IngressOutcome, Router};
use drm_router::transport::session::{Delivery, SessionReliability};
use drm_router::transport::{Reliability, TransportAdapter};
use drm_router::wire::WireHeader;
use drm_types::Status;

/// Deterministic xorshift64 RNG so a storm run is reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Records the wire channel id of every frame that reaches the media.
#[derive(Default)]
struct Counting {
    sent: Vec<u32>,
}

impl TransportAdapter for Counting {
    fn send_frame(&mut self, header: &WireHeader, _payload: &[u8]) -> Result<(), Status> {
        self.sent.push(header.wire_channel_id);
        Ok(())
    }

    fn recv_frame<'a>(
        &mut self,
        _buf: &'a mut [u8],
    ) -> Result<Option<(WireHeader, &'a [u8])>, Status> {
        Ok(None)
    }

    fn capabilities(&self) -> Reliability {
        Reliability::BestEffort
    }
}

/// Records every native deposit so ingress isolation is observable.
#[derive(Default)]
struct Recording {
    deposits: usize,
}

impl ImportSink for Recording {
    fn deposit(
        &mut self,
        _topic: u32,
        _epoch: u64,
        _seq: u64,
        _payload: &[u8],
    ) -> Result<(), Status> {
        self.deposits += 1;
        Ok(())
    }
}

/// Route binding for `topic` on `link` with wire channel `wire`.
/// REQ: REQ-ROUTE-022.
fn route(link: u32, wire: u32) -> EgressRoute {
    EgressRoute {
        topic: link,
        link,
        wire_channel_id: wire,
        remote_consumers: true,
        max_datagram_payload: 1400,
        max_record_bytes: 4096,
    }
}

/// A wire header carrying one payload byte for channel `wire`.
/// REQ: REQ-ROUTE-011.
fn header(wire: u32, seq: u64) -> WireHeader {
    WireHeader {
        flags: 0,
        wire_channel_id: wire,
        stream_epoch: 1,
        source_seq: seq,
        payload_len: 1,
    }
}

/// Steps per iteration: enough to hit every flip interleaving.
fn steps() -> usize {
    match std::env::var("DRM_STRESS_ITERS") {
        Ok(v) => v.parse::<usize>().unwrap_or(100).saturating_mul(200),
        Err(_) => 20_000,
    }
}

/// One independent router's flapping-link storm. REQ: REQ-ROUTE-007,
/// REQ-ROUTE-022.
fn storm(router_id: u64, steps: usize) {
    const LINKS: usize = 4;
    let bindings: Vec<EgressRoute> = (1..=LINKS)
        .map(|l| {
            let l = u32::try_from(l).unwrap_or(1);
            route(l, 10 + l)
        })
        .collect();
    let mut router = Router::new(bindings);
    let mut adapters: Vec<Counting> = (0..LINKS).map(|_| Counting::default()).collect();
    let mut sink = Recording::default();
    let mut down = [false; LINKS];
    let mut rng = Rng::new(0x9E37_79B9 ^ router_id.wrapping_mul(0x1000_0001));

    for step in 0..steps {
        let l = usize::try_from(rng.next() % u64::try_from(LINKS).unwrap_or(1)).unwrap_or(0);
        let link = u32::try_from(l + 1).unwrap_or(1);

        // Randomly flap the selected link.
        if rng.next().is_multiple_of(3) {
            if down[l] {
                router.link_up(link);
                down[l] = false;
            } else {
                router.link_down(link);
                down[l] = true;
            }
            assert_eq!(
                router.link_is_down(link),
                down[l],
                "link state tracks the flip"
            );
        }

        // Egress isolation: a cut link emits nothing; a live one emits exactly
        // one frame with its own wire id.
        let before = adapters[l].sent.len();
        let emitted = router
            .egress_udp(link, link, 1, step as u64, b"p", &mut adapters[l])
            .unwrap_or_else(|e| unreachable!("egress: {e:?}"));
        assert_eq!(emitted, !down[l], "egress result matches the link state");
        if down[l] {
            assert_eq!(adapters[l].sent.len(), before, "cut link emits nothing");
        } else {
            assert_eq!(adapters[l].sent.len(), before + 1, "live link emits one");
            assert_eq!(adapters[l].sent.last().copied(), Some(10 + link));
        }

        // Ingress isolation: a cut link drops without depositing.
        let deposits_before = sink.deposits;
        let outcome = router
            .ingress_frame(link, &header(10 + link, step as u64), &[7], &mut sink)
            .unwrap_or_else(|e| unreachable!("ingress: {e:?}"));
        if down[l] {
            assert_eq!(outcome, IngressOutcome::LinkDown, "cut link drops ingress");
            assert_eq!(sink.deposits, deposits_before, "no deposit on a cut link");
        }
    }

    // Recovery: bring every link up and prove all links flow again.
    for (l, adapter) in adapters.iter_mut().enumerate() {
        let link = u32::try_from(l + 1).unwrap_or(1);
        router.link_up(link);
        assert!(!router.link_is_down(link), "every link recovers");
        assert_eq!(
            router.egress_udp(link, link, 1, 0, b"r", adapter),
            Ok(true),
            "recovered link resumes egress"
        );
    }
}

/// REQ: REQ-QAL-010, REQ-ROUTE-007, REQ-ROUTE-022 — the flapping-link storm
/// across independent routers; every link recovers after the storm (I-RT6).
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_link_storm() {
    let budget = steps();
    for router_id in 0..3 {
        storm(router_id, budget);
    }
}

/// REQ: REQ-QAL-010, REQ-ROUTE-029 — rapid session flapping: every reconnect
/// bumps the generation exactly once, a delivery on a closed session is
/// refused, and the first delivery of a new session reports a gap for the
/// sequences skipped across the boundary (never a silent stitch).
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_link_storm_sessions() {
    let mut rel = SessionReliability::new();
    assert_eq!(rel.generation(), 0, "no reconnect yet");
    let mut next = 10_u64;
    assert_eq!(rel.deliver(next), Ok(Delivery::InOrder));
    next += 1;
    assert_eq!(rel.deliver(next), Ok(Delivery::InOrder));
    next += 1;
    // A skip inside the first session is an exact gap.
    let target = next + 1;
    assert_eq!(rel.deliver(target), Ok(Delivery::Gap { lost: 1 }));
    next = target + 1;
    rel.end_session();
    assert_eq!(rel.deliver(next), Err(Status::InvalidArg));

    for cycle in 0..steps().min(10_000) {
        rel.begin_session()
            .unwrap_or_else(|e| unreachable!("begin: {e:?}"));
        assert_eq!(
            rel.generation(),
            u32::try_from(cycle + 1).unwrap_or(u32::MAX),
            "each reconnect bumps the generation once"
        );
        // Skip across the boundary: the retained logical position makes the
        // first delivery a gap, never a stitch (I-RT5).
        let skip = 1 + (cycle % 3);
        let target = next + u64::try_from(skip).unwrap_or(1);
        assert_eq!(
            rel.deliver(target),
            Ok(Delivery::Gap {
                lost: u64::try_from(skip).unwrap_or(1),
            }),
            "a skip across the flap is an exact gap"
        );
        next = target + 1;
        rel.end_session();
    }
}
