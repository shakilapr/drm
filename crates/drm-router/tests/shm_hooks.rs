//! Router SHM-hook e2e (work-plan M10W1-E1-T1; router-core.md I-RT4/I-RT9):
//! export a live local channel sample over UDP, then import it into a second
//! local channel through the standard publish path — no router-only privilege.
//! The wire header is stripped before deposit, and the consumer of the imported
//! channel reads the same native bytes as the producer, so imported and local
//! data are indistinguishable.
//! REQ: REQ-ROUTE-031, REQ-ROUTE-023, REQ-ROUTE-032.

use std::net::SocketAddr;

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_router::data_path::{EgressRoute, IngressOutcome, Router};
use drm_router::import::ChannelSink;
use drm_router::transport::udp::UdpTransport;
use drm_router::wire::{HEADER_LEN, WireHeader};
use drm_types::Status;

/// Activates an in-process channel sized for one 8-byte sample.
/// REQ: REQ-CORE-045 (pool invariant), REQ-ROUTE-031.
fn channel() -> Channel {
    Channel::activate(4, 8, 8, 8, OverflowPolicy::Overwrite, 0, 2, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

/// A route binding on link 1 with wire channel `wire`.
/// REQ: REQ-ROUTE-032.
fn route(wire: u32, remote: bool) -> EgressRoute {
    EgressRoute {
        topic: 7,
        link: 1,
        wire_channel_id: wire,
        remote_consumers: remote,
        max_datagram_payload: 1400,
        max_record_bytes: 4096,
    }
}

/// Bounded non-blocking receive: loopback delivers promptly, so a large spin
/// budget always succeeds; exhaustion is reported rather than hanging.
/// REQ: REQ-ROUTE-032.
fn recv_wait(rx: &UdpTransport, buf: &mut [u8]) -> Result<(usize, SocketAddr), Status> {
    for _ in 0..100_000_000 {
        match rx.recv_from(buf) {
            Ok(v) => return Ok(v),
            Err(Status::Retry) => std::hint::spin_loop(),
            Err(e) => return Err(e),
        }
    }
    Err(Status::Retry)
}

/// REQ: REQ-ROUTE-031 — export a local channel sample over UDP and import it
/// into a second channel via the [`ChannelSink`] hook; the imported bytes equal
/// the local bytes and the wire header is absent from the deposited sample.
#[test]
fn import_export_shm_hooks() {
    const SAMPLE: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

    // Producer channel: publish one native sample through the standard path.
    let source = channel();
    source
        .publish(&SAMPLE)
        .unwrap_or_else(|e| unreachable!("publish: {e:?}"));
    let mut st = source.reader();
    let mut local = [0_u8; 8];
    assert_eq!(
        source.latest(&mut st, 1, &mut local),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(local, SAMPLE, "producer reads its own native sample");

    // Egress over loopback UDP: the router frames the borrowed sample.
    let rx = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
    let dst = rx.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
    let mut tx =
        UdpTransport::bind_peer("127.0.0.1:0", 1400, dst).unwrap_or_else(|e| unreachable!("{e:?}"));
    let mut sender = Router::new(vec![route(55, true)]);
    assert_eq!(
        sender.egress_udp(
            1,
            7,
            u64::from(source.epoch()),
            source.last_sequence(),
            &local,
            &mut tx
        ),
        Ok(true),
        "remote topic egresses"
    );

    // Observe the raw datagram: header + payload, so we can prove the header is
    // stripped on import.
    let mut wire = [0_u8; 1400];
    let (n, _from) = recv_wait(&rx, &mut wire).unwrap_or_else(|e| unreachable!("{e:?}"));
    let wire_header = WireHeader::decode(&wire[..n]).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(wire_header.wire_channel_id, 55);
    assert_eq!(wire_header.payload_len, 8);
    assert_eq!(&wire[HEADER_LEN..n], &SAMPLE);

    // Import into a second channel through the synthetic-publisher hook.
    let target = channel();
    let mut sink = ChannelSink::new(&target);
    let mut receiver = Router::new(vec![route(55, false)]);
    assert_eq!(
        receiver.ingress_frame(1, &wire_header, &wire[HEADER_LEN..n], &mut sink),
        Ok(IngressOutcome::Imported)
    );

    // A consumer of the imported channel reads the exact native bytes.
    let mut tst = target.reader();
    let mut imported = [0_u8; 8];
    assert_eq!(
        target.latest(&mut tst, 1, &mut imported),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(imported, SAMPLE, "imported == local (native equivalence)");
    assert_eq!(target.last_sequence(), 0, "one imported sample committed");
}
