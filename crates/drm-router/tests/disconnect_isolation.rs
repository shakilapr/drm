//! Link-cut isolation suite (work-plan M13W4-E3-T1; router-core.md I-RT6):
//! a link-down event marks only the routes over that link down — egress on the
//! cut link is a no-op, other links and local delivery continue, ingress on the
//! cut link is dropped without touching ordering state, and reconnect resumes
//! with a fresh (empty) ingress session.
//! REQ: REQ-ROUTE-007, REQ-ROUTE-011, REQ-ROUTE-022.

use drm_router::data_path::{EgressRoute, ImportSink, IngressOutcome, Router};
use drm_router::transport::{Reliability, TransportAdapter};
use drm_router::wire::WireHeader;
use drm_types::Status;

/// Records the wire channel id of every frame handed to the media, so a test
/// can assert exactly which egress attempts reached the link.
#[derive(Default)]
struct CountingAdapter {
    sent: Vec<u32>,
}

impl TransportAdapter for CountingAdapter {
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
    deposits: Vec<(u32, u64, u64, Vec<u8>)>,
}

impl ImportSink for Recording {
    fn deposit(&mut self, topic: u32, epoch: u64, seq: u64, payload: &[u8]) -> Result<(), Status> {
        self.deposits.push((topic, epoch, seq, payload.to_vec()));
        Ok(())
    }
}

/// Route binding for `topic` on `link` with wire channel `wire`.
/// REQ: REQ-ROUTE-022.
fn route(link: u32, topic: u32, wire: u32, remote: bool) -> EgressRoute {
    EgressRoute {
        topic,
        link,
        wire_channel_id: wire,
        remote_consumers: remote,
        max_datagram_payload: 1400,
        max_record_bytes: 4096,
    }
}

/// A wire header carrying `len` payload bytes for channel `wire`.
/// REQ: REQ-ROUTE-011.
fn header(wire: u32, epoch: u64, seq: u64, len: u32) -> WireHeader {
    WireHeader {
        flags: 0,
        wire_channel_id: wire,
        stream_epoch: epoch,
        source_seq: seq,
        payload_len: len,
    }
}

/// REQ: REQ-ROUTE-007, REQ-ROUTE-022 — I-RT6: a link cut takes only that link's
/// routes offline; egress on the cut link emits nothing while egress on another
/// link is unaffected, and reconnect resumes the cut link.
#[test]
fn disconnect_isolation_egress() {
    let mut router = Router::new(vec![route(1, 7, 5, true), route(2, 8, 6, true)]);
    let mut adapter = CountingAdapter::default();

    // Both links egress before the cut.
    assert_eq!(router.egress_udp(1, 7, 1, 0, b"a", &mut adapter), Ok(true));
    assert_eq!(router.egress_udp(2, 8, 1, 0, b"b", &mut adapter), Ok(true));
    assert_eq!(adapter.sent, vec![5, 6]);

    // Cut link 1: its route goes offline, link 2 keeps flowing.
    router.link_down(1);
    assert!(router.link_is_down(1));
    assert!(!router.link_is_down(2));
    assert_eq!(router.egress_udp(1, 7, 1, 1, b"a", &mut adapter), Ok(false));
    assert_eq!(router.egress_udp(2, 8, 1, 1, b"b", &mut adapter), Ok(true));
    assert_eq!(
        adapter.sent,
        vec![5, 6, 6],
        "cut link emits nothing; sibling link unaffected"
    );

    // Reconnect: the cut link resumes.
    router.link_up(1);
    assert!(!router.link_is_down(1));
    assert_eq!(router.egress_udp(1, 7, 1, 2, b"a", &mut adapter), Ok(true));
    assert_eq!(adapter.sent, vec![5, 6, 6, 5]);
}

/// REQ: REQ-ROUTE-007, REQ-ROUTE-011 — I-RT6/I-RT9: ingress on a cut link is
/// dropped without state; after reconnect the ingress ordering state is fresh
/// (empty session), so a sequence that would have been a duplicate is accepted.
#[test]
fn disconnect_isolation_ingress_reconnect() {
    let mut router = Router::new(vec![route(1, 7, 5, false)]);
    let mut sink = Recording::default();

    // Establish ordering state at seq 5.
    assert_eq!(
        router.ingress_frame(1, &header(5, 1, 5, 1), &[9], &mut sink),
        Ok(IngressOutcome::Imported)
    );

    // Link cut: frames are dropped with no state change and no deposit.
    router.link_down(1);
    assert_eq!(
        router.ingress_frame(1, &header(5, 1, 6, 1), &[9], &mut sink),
        Ok(IngressOutcome::LinkDown)
    );
    assert_eq!(sink.deposits.len(), 1, "no deposit on a cut link");

    // Reconnect: the ordering state is fresh, so a lower seq is accepted.
    router.link_up(1);
    assert_eq!(
        router.ingress_frame(1, &header(5, 1, 1, 1), &[8], &mut sink),
        Ok(IngressOutcome::Imported)
    );
    assert_eq!(
        sink.deposits.len(),
        2,
        "reconnect resumes with an empty ingress session"
    );
    assert_eq!(sink.deposits[1].2, 1, "post-reconnect frame accepted");
}
