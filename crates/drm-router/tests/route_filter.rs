//! Forwarding-plan filter suite (work-plan M10W1-E3-T2; router-core.md I-RT1):
//! egress iterates only the route bindings of the link being serviced, and a
//! topic with only local consumers never egresses — zero datagrams reach the
//! wire. The link scan is observed directly through [`Router::routes_on`].
//! REQ: REQ-ROUTE-022, REQ-ROUTE-032.

use drm_router::data_path::{EgressRoute, Router};
use drm_router::transport::{Reliability, TransportAdapter};
use drm_router::wire::WireHeader;
use drm_types::Status;

/// A recording adapter: every accepted frame is captured so a test can assert
/// exactly what would reach the wire, with no real socket.
struct Recording {
    frames: Vec<(WireHeader, Vec<u8>)>,
}

impl Recording {
    fn new() -> Self {
        Self { frames: Vec::new() }
    }
}

impl TransportAdapter for Recording {
    fn send_frame(&mut self, header: &WireHeader, payload: &[u8]) -> Result<(), Status> {
        self.frames.push((*header, payload.to_vec()));
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

/// A route binding for `topic` on `link` with wire channel `wire`, carrying a
/// remote consumer iff `remote`.
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

/// REQ: REQ-ROUTE-022 — I-RT1 `FilterExact`: a topic with only local consumers
/// never egresses. Iterating `routes_on(link)` and egressing every remote
/// binding sends exactly the remote topics; the local-only topic produces zero
/// frames on the recording wire, and another link's routes are never examined.
#[test]
fn route_filter() {
    let mut router = Router::new(vec![
        route(7, 1, 11, true),  // remote consumer on link 7
        route(7, 2, 12, false), // local-only on link 7 — must never egress
        route(8, 3, 13, true),  // remote but a different link
    ]);
    let mut wire = Recording::new();

    // The link scan sees only link 7's bindings, in registration order.
    let on7: Vec<u32> = router.routes_on(7).map(|r| r.topic).collect();
    assert_eq!(on7, vec![1, 2], "only link-7 routes are scanned");
    assert_eq!(
        router.routes_on(8).map(|r| r.topic).collect::<Vec<_>>(),
        vec![3]
    );

    // Simulate one egress tick per source topic the router holds, driving each
    // through the link-7 scan. Only the remote topic produces a datagram.
    for egress in router.routes_on(7).collect::<Vec<_>>() {
        let byte = u8::try_from(egress.topic).unwrap_or(0);
        let sent = router
            .egress_udp(7, egress.topic, 1, 0, &[byte], &mut wire)
            .unwrap_or_else(|e| unreachable!("egress: {e:?}"));
        assert_eq!(sent, egress.remote_consumers, "topic {}", egress.topic);
    }

    // Exactly one frame, for the remote topic; the local-only topic emitted
    // nothing (zero datagrams observed on the wire).
    assert_eq!(wire.frames.len(), 1, "local-only topic never egresses");
    assert_eq!(wire.frames[0].0.wire_channel_id, 11);
    assert_eq!(wire.frames[0].1, vec![1]);

    // A topic bound to another link is filtered for this link: no work, no
    // reservation, no frame.
    assert_eq!(router.egress_udp(7, 3, 1, 0, &[3], &mut wire), Ok(false));
    // An unregistered topic is likewise filtered, never an error.
    assert_eq!(router.egress_udp(7, 99, 1, 0, &[9], &mut wire), Ok(false));
    assert_eq!(wire.frames.len(), 1, "no frame after filtered egress");
}
