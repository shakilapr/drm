//! Router core data-path suite (work-plan M09W4-E3-T1/T2; router-core.md
//! §Test Matrix): route filtering (I-RT1), datagram bound enforcement (I-RT3),
//! header-stripped ingress with ordering (I-RT9), and imported-vs-local native
//! equivalence (I-RT4).
//! REQ: REQ-ROUTE-022, REQ-ROUTE-031, REQ-ROUTE-032, REQ-ROUTE-011.

use drm_router::data_path::{EgressRoute, ImportSink, IngressOutcome, Router};
use drm_router::transport::udp::UdpTransport;
use drm_types::Status;

/// An in-memory importer recording every deposit and epoch reset.
struct VecSink {
    deposits: Vec<(u32, u64, u64, Vec<u8>)>,
    resets: Vec<u32>,
}

impl VecSink {
    fn new() -> Self {
        Self {
            deposits: Vec::new(),
            resets: Vec::new(),
        }
    }
}

impl ImportSink for VecSink {
    fn deposit(&mut self, topic: u32, epoch: u64, seq: u64, payload: &[u8]) -> Result<(), Status> {
        self.deposits.push((topic, epoch, seq, payload.to_vec()));
        Ok(())
    }

    fn reset_epoch(&mut self, topic: u32) -> Result<(), Status> {
        self.resets.push(topic);
        Ok(())
    }
}

/// A route binding for topic `topic` on link 1 with wire channel `wire`.
fn route(topic: u32, wire: u32, remote: bool) -> EgressRoute {
    EgressRoute {
        topic,
        link: 1,
        wire_channel_id: wire,
        remote_consumers: remote,
        max_datagram_payload: 1400,
        max_record_bytes: 4096,
    }
}

/// A wire header carrying `len` payload bytes for channel `wire`.
fn header(wire: u32, epoch: u64, seq: u64, len: u32) -> drm_router::wire::WireHeader {
    drm_router::wire::WireHeader {
        flags: 0,
        wire_channel_id: wire,
        stream_epoch: epoch,
        source_seq: seq,
        payload_len: len,
    }
}

/// REQ: REQ-ROUTE-022, REQ-ROUTE-031 — UDP egress then ingress over a loopback
/// pair: the sender filters by remote consumers, frames one sample, and the
/// receiver strips the headers and deposits the exact payload.
#[test]
fn egress_udp() {
    let mut rx = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
    let dst = rx.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
    let mut tx =
        UdpTransport::bind_peer("127.0.0.1:0", 1400, dst).unwrap_or_else(|e| unreachable!("{e:?}"));

    let mut sender = Router::new(vec![route(7, 5, true), route(8, 6, false)]);
    let mut receiver = Router::new(vec![route(7, 5, false)]);
    let mut sink = VecSink::new();

    assert_eq!(sender.egress_udp(1, 7, 1, 0, b"hello", &mut tx), Ok(true));
    let mut buf = [0_u8; 1400];
    let outcome = loop {
        if let Ok(Some(o)) = receiver.ingress_udp(1, &mut rx, &mut buf, &mut sink) {
            break o;
        }
        std::hint::spin_loop();
    };
    assert_eq!(outcome, IngressOutcome::Imported);
    assert_eq!(sink.deposits.len(), 1);
    assert_eq!(sink.deposits[0], (7, 1, 0, b"hello".to_vec()));

    // The local-only route never emits a frame (I-RT1).
    assert_eq!(sender.egress_udp(1, 8, 1, 0, b"local", &mut tx), Ok(false));
    // An unregistered topic is filtered, not an error.
    assert_eq!(sender.egress_udp(1, 99, 1, 0, b"x", &mut tx), Ok(false));

    // I-RT3: an oversize payload is refused before any syscall.
    let big = vec![0_u8; 1400];
    assert_eq!(
        sender.egress_udp(1, 7, 1, 1, &big, &mut tx),
        Err(Status::InvalidArg)
    );
}

/// REQ: REQ-ROUTE-031 — I-RT4: imported payloads deposit in the same native
/// bytes as a local sample, so a consumer cannot distinguish them; only the
/// header is stripped.
#[test]
fn import_native_equivalence() {
    let local: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02];
    let mut local_sink = VecSink::new();
    local_sink
        .deposit(7, 1, 0, local)
        .unwrap_or_else(|e| unreachable!("{e:?}"));

    let mut receiver = Router::new(vec![route(7, 5, false)]);
    let mut imported = VecSink::new();
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 1, 0, 6), local, &mut imported),
        Ok(IngressOutcome::Imported)
    );

    assert_eq!(imported.deposits.len(), 1);
    assert_eq!(local_sink.deposits[0].3, imported.deposits[0].3);
    assert_eq!(imported.deposits[0].3, local.to_vec());
}

/// REQ: REQ-ROUTE-011 — I-RT9: same-epoch reorder/duplicate is dropped; a
/// higher epoch resets the imported publisher and is accepted.
#[test]
fn ingress_ordering() {
    let mut receiver = Router::new(vec![route(7, 5, false)]);
    let mut sink = VecSink::new();

    assert_eq!(
        receiver.ingress_frame(1, &header(5, 2, 5, 1), &[9], &mut sink),
        Ok(IngressOutcome::Imported)
    );
    // Reorder/duplicate within the epoch.
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 2, 3, 1), &[9], &mut sink),
        Ok(IngressOutcome::Duplicate)
    );
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 2, 5, 1), &[9], &mut sink),
        Ok(IngressOutcome::Duplicate)
    );
    // Higher epoch resets the stream.
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 3, 0, 1), &[8], &mut sink),
        Ok(IngressOutcome::Imported)
    );
    assert_eq!(sink.resets, vec![7]);
    // Lower epoch is stale.
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 2, 9, 1), &[8], &mut sink),
        Ok(IngressOutcome::StaleEpoch)
    );
    assert_eq!(sink.deposits.len(), 2);
}

/// REQ: REQ-ROUTE-031, REQ-ROUTE-038 — I-RT14/I-RT18: an unknown channel is
/// dropped without state; a length mismatch and a receive-bound violation are
/// protocol errors that change no ordering state.
#[test]
fn ingress_rejects_before_state() {
    let bounded = EgressRoute {
        max_record_bytes: 64,
        ..route(7, 5, false)
    };
    let mut receiver = Router::new(vec![bounded]);
    let mut sink = VecSink::new();

    assert_eq!(
        receiver.ingress_frame(1, &header(5, 1, 0, 1), &[1], &mut sink),
        Ok(IngressOutcome::Imported)
    );
    // Length mismatch (declared 4, actual 1).
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 1, 1, 4), &[1], &mut sink),
        Err(Status::InvalidData)
    );
    // Receive bound: declared payload above `max_record_bytes` (64), length matching.
    let big = vec![0_u8; 100];
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 1, 2, 100), &big, &mut sink),
        Err(Status::InvalidData)
    );
    // Unknown wire channel id: dropped, never an error.
    assert_eq!(
        receiver.ingress_frame(1, &header(42, 1, 0, 1), &[1], &mut sink),
        Ok(IngressOutcome::UnknownChannel)
    );
    // Ordering state unchanged: seq 1 is still new, seq 0 duplicate.
    assert_eq!(
        receiver.ingress_frame(1, &header(5, 1, 0, 1), &[1], &mut sink),
        Ok(IngressOutcome::Duplicate)
    );
    assert_eq!(sink.deposits.len(), 1);
}
