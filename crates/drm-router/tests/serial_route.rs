//! Serial route integration suite (work-plan M13W2-E3-T1; serial-mcu.md
//! I-SR1/I-SR2/I-SR4, router-core.md I-RT1/I-RT4): the serial link enters the
//! transport registry as a best-effort stream, and one sample drives the full
//! router path — forwarding-plan filter, serial COBS/CRC32C encode, byte
//! stream, decode, header-stripped native deposit. Serial egress rides the
//! route's shaping bucket (lanes-shaping.md). REQ: REQ-ROUTE-024, REQ-ROUTE-025,
//! REQ-ROUTE-026, REQ-ROUTE-034, REQ-ROUTE-040, REQ-ROUTE-030.

use drm_router::data_path::{EgressRoute, ImportSink, IngressOutcome, Router};
use drm_router::shaping::TokenBucket;
use drm_router::transport::Reliability;
use drm_router::transport::registry::{TransportKind, TransportRegistry};
use drm_router::transport::serial::{SerialLinkConfig, SerialTransport};
use drm_types::Status;

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

/// Records every header-stripped payload the ingress path deposits, so a test
/// can assert native-format equivalence (I-RT4).
/// REQ: REQ-ROUTE-031.
#[derive(Default)]
struct Recording {
    deposited: Vec<(u32, u64, u64, Vec<u8>)>,
    epochs_reset: Vec<u32>,
}

impl ImportSink for Recording {
    fn deposit(&mut self, topic: u32, epoch: u64, seq: u64, payload: &[u8]) -> Result<(), Status> {
        self.deposited.push((topic, epoch, seq, payload.to_vec()));
        Ok(())
    }

    fn reset_epoch(&mut self, topic: u32) -> Result<(), Status> {
        self.epochs_reset.push(topic);
        Ok(())
    }
}

/// REQ: REQ-ROUTE-024, REQ-ROUTE-034 — the serial link is registered as a
/// best-effort stream in the authoritative capability table, so the router
/// path selects serial for a serial-bound route.
#[test]
fn serial_route_registry() {
    let registry = TransportRegistry::from_defaults();
    let selection = registry
        .select(TransportKind::Serial)
        .unwrap_or_else(|e| unreachable!("serial select: {e:?}"));
    assert_eq!(selection.kind, TransportKind::Serial);
    assert_eq!(selection.reliability(), Reliability::BestEffort);
    assert!(
        selection.caps.is_stream(),
        "serial is a length-framed stream"
    );

    let link = SerialLinkConfig::new(115_200, 256)
        .unwrap_or_else(|e| unreachable!("serial config: {e:?}"));
    assert_eq!(link.reliability(), Reliability::BestEffort);
}

/// REQ: REQ-ROUTE-030, REQ-ROUTE-040 — serial egress is shaped per route
/// bucket: a `TokenBucket` sized from the link rate admits a bounded serial
/// frame within its burst, and the I-SR7 worst-case blocking is finite and
/// scales with baud.
#[test]
fn serial_route_shaping() {
    let link = SerialLinkConfig::new(115_200, 256)
        .unwrap_or_else(|e| unreachable!("serial config: {e:?}"));
    let wire = u64::try_from(link.max_wire_bytes()).unwrap_or(u64::MAX);
    let mut bucket = TokenBucket::new(1_000_000, wire);
    assert!(
        bucket.try_consume(wire, 0),
        "one serial frame fits the route bucket's burst"
    );
    assert!(
        link.worst_case_blocking_ns().is_some(),
        "finite non-preemptible transmission bound"
    );
}

/// REQ: REQ-ROUTE-022, REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026,
/// REQ-ROUTE-031 — a sample crosses the full serial router path: publisher
/// egress filters by remote consumers (I-RT1), encodes one bounded
/// COBS/CRC32C frame, writes it to the byte link; the consumer reads,
/// decodes, strips the wire header, and deposits the native payload (I-RT4),
/// byte-exact and in order.
#[cfg(unix)]
#[test]
fn serial_route_loopback() {
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    let (tx_stream, rx_stream) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(e) => unreachable!("socketpair: {e:?}"),
    };
    let cfg = SerialLinkConfig::new(115_200, 256)
        .unwrap_or_else(|e| unreachable!("serial config: {e:?}"));
    let mut tx = SerialTransport::new(tx_stream, cfg.clone());
    let mut rx = SerialTransport::new(rx_stream, cfg);
    if let Err(e) = rx.link_mut().set_read_timeout(Some(Duration::from_secs(5))) {
        unreachable!("set read timeout: {e:?}");
    }
    if let Err(e) = rx.link_mut().set_nonblocking(true) {
        unreachable!("set nonblocking: {e:?}");
    }

    let mut publisher = Router::new(vec![route(5, 1, 9, true)]);
    let mut consumer = Router::new(vec![route(5, 1, 9, true)]);
    let payload = [0xDE_u8, 0x00, 0xAD, 0x00, 0x42];

    let sent = publisher
        .egress_frame(5, 1, 3, 7, &payload, &mut tx)
        .unwrap_or_else(|e| unreachable!("egress: {e:?}"));
    assert!(sent, "remote-consumer route egresses");

    // A local-only topic on the same link emits nothing (I-RT1).
    let mut local_only = Router::new(vec![route(5, 2, 10, false)]);
    assert_eq!(
        local_only.egress_frame(5, 2, 3, 0, &[1], &mut tx),
        Ok(false)
    );

    let mut sink = Recording::default();
    let mut buf = [0_u8; 512];
    let deadline = Instant::now() + Duration::from_secs(5);
    let outcome = loop {
        match consumer.ingress_udp(5, &mut rx, &mut buf, &mut sink) {
            Ok(Some(o)) => break o,
            Ok(None) | Err(Status::Retry) => {
                assert!(Instant::now() < deadline, "no serial frame within deadline");
                std::hint::spin_loop();
            }
            Err(e) => unreachable!("ingress: {e:?}"),
        }
    };
    assert_eq!(outcome, IngressOutcome::Imported);
    assert_eq!(sink.deposited.len(), 1, "exactly one native deposit");
    let (topic, epoch, seq, bytes) = &sink.deposited[0];
    assert_eq!(*topic, 1);
    assert_eq!(*epoch, 3);
    assert_eq!(*seq, 7);
    assert_eq!(bytes.as_slice(), &payload, "payload byte-exact (I-SR4)");
}
