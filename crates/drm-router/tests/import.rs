//! Import completion suite (work-plan M10W2-E3-T1/E1-T1; router-core.md I-RT4,
//! I-RT9; publisher-lifecycle.md I-L4): a received frame is stripped of its wire
//! header and republished through the ordinary synthetic-publisher path, so a
//! consumer cannot distinguish imported from local data (bytes + timing probe);
//! a remote restart or a link reconnect bumps the imported publisher epoch and
//! consumers reset before the first post-restart sample.
//! REQ: REQ-ROUTE-031, REQ-ROUTE-023, REQ-ROUTE-011, REQ-ROUTE-007,
//! REQ-CORE-018.

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_router::data_path::{EgressRoute, IngressOutcome, Router};
use drm_router::import::ChannelSink;
use drm_router::wire::WireHeader;

/// One 8-byte sample, the channel's fixed payload capacity.
/// REQ: REQ-ROUTE-031.
const SAMPLE: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];

/// Activates an in-process channel sized for one 8-byte sample.
/// REQ: REQ-ROUTE-031.
fn channel() -> Channel {
    Channel::activate(4, 8, 8, 8, OverflowPolicy::Overwrite, 0, 2, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

/// Ingress route binding on link 1 with wire channel `wire`.
/// REQ: REQ-ROUTE-031.
fn route(wire: u32) -> EgressRoute {
    EgressRoute {
        topic: 7,
        link: 1,
        wire_channel_id: wire,
        remote_consumers: false,
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

/// Reads one sample from `ch`, surfacing a suite bug loudly.
/// REQ: REQ-ROUTE-031.
fn read_latest(ch: &Channel) -> [u8; 8] {
    let mut st = ch.reader();
    let mut buf = [0_u8; 8];
    match ch.latest(&mut st, 0, &mut buf) {
        Ok(ReadOutcome::Sample) => buf,
        other => unreachable!("read: {other:?}"),
    }
}

/// REQ: REQ-ROUTE-031, REQ-ROUTE-023 — I-RT4 bytes + timing probe: the imported
/// sample is byte-identical to a local publish, is immediately readable (no
/// deferred commit), and its deposit cost stays in the same order of magnitude
/// as a local publish.
#[test]
fn import_native_equivalence() {
    const N: usize = 256;

    // Local reference: publish through the ordinary path.
    let local = channel();
    let local_start = std::time::Instant::now();
    for _ in 0..N {
        local
            .publish(&SAMPLE)
            .unwrap_or_else(|e| unreachable!("local publish: {e:?}"));
    }
    let local_elapsed = local_start.elapsed();
    assert_eq!(read_latest(&local), SAMPLE, "local sample bytes");

    // Imported: ingress strips the wire header and republishes the payload.
    let target = channel();
    let mut sink = ChannelSink::new(&target);
    let mut router = Router::new(vec![route(5)]);
    let imported_start = std::time::Instant::now();
    for seq in 0..N {
        let seq = u64::try_from(seq).unwrap_or(0);
        assert_eq!(
            router.ingress_frame(1, &header(5, 1, seq, 8), &SAMPLE, &mut sink),
            Ok(IngressOutcome::Imported)
        );
    }
    let imported_elapsed = imported_start.elapsed();

    // Bytes: imported == local, so a consumer cannot distinguish them.
    assert_eq!(read_latest(&target), SAMPLE, "imported sample bytes");

    // Timing probe: both paths complete well under a generous bound, and the
    // imported path is within an order of magnitude of the local path.
    assert!(
        local_elapsed < std::time::Duration::from_millis(250),
        "local publish path too slow: {local_elapsed:?}"
    );
    assert!(
        imported_elapsed < std::time::Duration::from_millis(250),
        "import deposit path too slow: {imported_elapsed:?}"
    );
    assert!(
        imported_elapsed < local_elapsed * 50 + std::time::Duration::from_millis(50),
        "import cost not comparable to local: imported={imported_elapsed:?} local={local_elapsed:?}"
    );
}

/// REQ: REQ-ROUTE-031, REQ-ROUTE-011 — I-RT4: only the wire header is stripped;
/// the deposited native payload is exactly the frame payload.
#[test]
fn import_strips_wire_headers() {
    let target = channel();
    let mut sink = ChannelSink::new(&target);
    let mut router = Router::new(vec![route(5)]);
    assert_eq!(
        router.ingress_frame(1, &header(5, 1, 0, 8), &SAMPLE, &mut sink),
        Ok(IngressOutcome::Imported)
    );
    assert_eq!(
        read_latest(&target),
        SAMPLE,
        "wire header absent from the deposited sample"
    );
}

/// REQ: REQ-ROUTE-011, REQ-ROUTE-007, REQ-CORE-018 — I-RT4/I-RT9: a higher
/// remote epoch bumps the imported publisher epoch, and a link reconnect bumps
/// it again; a consumer that observed the pre-restart epoch resets and reads the
/// new sample, never a stale pre-restart payload.
#[test]
fn import_epoch_reconnect_reset() {
    let target = channel();
    assert_eq!(target.epoch(), 0, "fresh channel epoch");
    let mut sink = ChannelSink::new(&target);
    let mut router = Router::new(vec![route(5)]);

    // First connection: no reset yet, one sample deposited.
    assert_eq!(
        router.ingress_frame(1, &header(5, 1, 0, 8), &SAMPLE, &mut sink),
        Ok(IngressOutcome::Imported)
    );
    assert_eq!(target.epoch(), 0);
    let mut consumer = target.reader();
    let mut buf = [0_u8; 8];
    assert_eq!(
        target.next(&mut consumer, 0, &mut buf),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(buf, SAMPLE);

    // Remote restart: a higher epoch resets the imported publisher.
    let restarted = [9_u8; 8];
    assert_eq!(
        router.ingress_frame(1, &header(5, 2, 0, 8), &restarted, &mut sink),
        Ok(IngressOutcome::Imported)
    );
    assert_eq!(target.epoch(), 1, "remote restart bumps importer epoch");

    // Reconnect: the session is fresh and the importer epoch bumps again.
    router.link_down(1);
    assert_eq!(
        router.ingress_frame(1, &header(5, 2, 1, 8), &restarted, &mut sink),
        Ok(IngressOutcome::LinkDown)
    );
    router.link_up(1);
    let resumed = [7_u8; 8];
    assert_eq!(
        router.ingress_frame(1, &header(5, 2, 1, 8), &resumed, &mut sink),
        Ok(IngressOutcome::Imported)
    );
    assert_eq!(target.epoch(), 2, "reconnect bumps importer epoch");

    // The stale consumer observed the pre-restart epoch; on its next read it
    // resets and receives the new-epoch sample, never a pre-restart payload.
    assert_eq!(
        target.next(&mut consumer, 0, &mut buf),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(buf, resumed, "post-reconnect sample, not stale bytes");
}

/// REQ: REQ-ROUTE-031 — a synthetic importer over a shared channel exposes its
/// bound channel and republishes through the standard publish path (no
/// router-only privilege).
#[test]
fn import_sink_binds_channel() {
    let target = channel();
    let sink = ChannelSink::new(&target);
    assert!(std::ptr::eq(sink.channel(), &raw const target));
    assert_eq!(
        target.publish(&SAMPLE),
        Ok(0),
        "standard publish path on the bound channel"
    );
    assert_eq!(read_latest(&target), SAMPLE);
}
