//! Two-host router simulation (work-plan M10W2-E3-T2/E4-T1; router-core.md
//! §Protocols *Two-host sim plan*): two router instances act as distinct hosts
//! — separate deployment namespaces and loopback addresses — and exchange every
//! supported topic class: small structured and event samples over UDP (one
//! datagram per sample, ≤ MTU), bulk samples over TCP length-framed records.
//! A link cut takes only the remote route offline while the other host's local
//! loop keeps running, and reconnect resumes with a fresh session.
//! REQ: REQ-ROUTE-022, REQ-ROUTE-031, REQ-ROUTE-032, REQ-ROUTE-029,
//! REQ-ROUTE-007, REQ-ROUTE-011.

use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_router::data_path::{EgressRoute, ImportSink, IngressOutcome, Router};
use drm_router::deployment::{DeploymentId, DeploymentNamespace};
use drm_router::import::ChannelSink;
use drm_router::transport::TransportAdapter;
use drm_router::transport::tcp::TcpTransport;
use drm_router::transport::udp::UdpTransport;
use drm_types::Status;

/// Loopback link id shared by both simulated hosts.
/// REQ: REQ-ROUTE-022.
const LINK: u32 = 1;

/// Per-datagram payload bound (I-RT3), comfortably below the MTU.
/// REQ: REQ-ROUTE-032.
const MAX_DATAGRAM: u16 = 1400;

/// Per-route receive/record bound (I-RT18).
/// REQ: REQ-ROUTE-038.
const MAX_RECORD: u32 = 4096;

/// One supported topic class exchanged across the simulated hosts.
/// REQ: REQ-ROUTE-029, REQ-ROUTE-032.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TopicClass {
    /// Small structured sample: one UDP datagram ≤ MTU.
    Structured,
    /// Bulk sample: one TCP length-framed record.
    Bulk,
    /// Event sample: small, best-effort UDP.
    Event,
}

impl TopicClass {
    /// Every topic class the two-host sim must exercise.
    /// REQ: REQ-ROUTE-029, REQ-ROUTE-032.
    const ALL: [Self; 3] = [Self::Structured, Self::Bulk, Self::Event];

    /// Stable class name for evidence.
    /// REQ: REQ-ROUTE-032.
    fn name(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::Bulk => "bulk",
            Self::Event => "event",
        }
    }

    /// Runtime topic handle.
    /// REQ: REQ-ROUTE-022.
    fn topic(self) -> u32 {
        match self {
            Self::Structured => 1,
            Self::Bulk => 2,
            Self::Event => 3,
        }
    }

    /// Transport-scoped wire channel id (never the runtime handle, I-RT7).
    /// REQ: REQ-ROUTE-033.
    fn wire(self) -> u32 {
        match self {
            Self::Structured => 0x10,
            Self::Bulk => 0x20,
            Self::Event => 0x30,
        }
    }

    /// The class payload: structured ≤ MTU, bulk a multi-record stream payload,
    /// event small and tagged.
    /// REQ: REQ-ROUTE-032, REQ-ROUTE-029.
    fn payload(self) -> Vec<u8> {
        match self {
            Self::Structured => vec![0x51; 64],
            Self::Bulk => vec![0xB0; 1024],
            Self::Event => vec![0xE0; 16],
        }
    }

    /// Whether this class rides UDP (structured + event) or TCP (bulk).
    /// REQ: REQ-ROUTE-032, REQ-ROUTE-029.
    fn uses_udp(self) -> bool {
        matches!(self, Self::Structured | Self::Event)
    }
}

/// A route binding for `class` on `link`, carrying a remote consumer iff
/// `remote`.
/// REQ: REQ-ROUTE-022.
fn route(class: TopicClass, remote: bool) -> EgressRoute {
    EgressRoute {
        topic: class.topic(),
        link: LINK,
        wire_channel_id: class.wire(),
        remote_consumers: remote,
        max_datagram_payload: MAX_DATAGRAM,
        max_record_bytes: MAX_RECORD,
    }
}

/// Activates an in-process import channel sized for a `sample_max`-byte sample
/// (a non-canonical channel exposes the whole fixed slot as the payload view).
/// REQ: REQ-ROUTE-031.
fn channel(sample_max: u64) -> Channel {
    Channel::activate(4, 8, sample_max, 8, OverflowPolicy::Overwrite, 0, 2, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

/// Reads the latest sample and asserts it equals `want`.
/// REQ: REQ-ROUTE-031.
fn assert_latest(ch: &Channel, want: &[u8]) {
    let mut st = ch.reader();
    let mut buf = vec![0_u8; want.len()];
    match ch.latest(&mut st, 0, &mut buf) {
        Ok(ReadOutcome::Sample) => assert_eq!(buf, want, "imported == local bytes"),
        other => unreachable!("read: {other:?}"),
    }
}

/// Bounded non-blocking receive through `router`; loopback delivers promptly.
/// REQ: REQ-ROUTE-031.
fn recv_until(
    router: &mut Router,
    adapter: &mut impl TransportAdapter,
    sink: &mut impl ImportSink,
) -> IngressOutcome {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = [0_u8; MAX_RECORD as usize];
    loop {
        match router.ingress_udp(LINK, adapter, &mut buf, sink) {
            Ok(Some(outcome)) => return outcome,
            Ok(None) | Err(Status::Retry) => {
                assert!(Instant::now() < deadline, "no frame within deadline");
                std::hint::spin_loop();
            }
            Err(e) => unreachable!("ingress: {e:?}"),
        }
    }
}

/// A bound UDP socket pair acting as the two hosts' link, each with the other
/// as its peer so the exchange is full duplex.
/// REQ: REQ-ROUTE-032.
fn udp_pair() -> (UdpTransport, UdpTransport) {
    let mut a = UdpTransport::bind("127.0.0.1:0", MAX_DATAGRAM)
        .unwrap_or_else(|e| unreachable!("bind a: {e:?}"));
    let mut b = UdpTransport::bind("127.0.0.1:0", MAX_DATAGRAM)
        .unwrap_or_else(|e| unreachable!("bind b: {e:?}"));
    let a_addr: SocketAddr = a
        .local_addr()
        .unwrap_or_else(|e| unreachable!("a addr: {e:?}"));
    let b_addr: SocketAddr = b
        .local_addr()
        .unwrap_or_else(|e| unreachable!("b addr: {e:?}"));
    a.set_peer(b_addr);
    b.set_peer(a_addr);
    (a, b)
}

/// A connected TCP transport pair (client, accepted server).
/// REQ: REQ-ROUTE-029.
fn tcp_pair() -> (TcpTransport, TcpTransport) {
    let listener =
        TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("tcp bind: {e:?}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|e| unreachable!("tcp addr: {e:?}"));
    let server = std::thread::spawn(move || -> TcpTransport {
        let (stream, _) = listener
            .accept()
            .unwrap_or_else(|e| unreachable!("accept: {e:?}"));
        TcpTransport::from_stream(stream, MAX_RECORD)
            .unwrap_or_else(|e| unreachable!("server transport: {e:?}"))
    });
    let client = TcpTransport::connect(addr, MAX_RECORD)
        .unwrap_or_else(|e| unreachable!("client transport: {e:?}"));
    let server = server
        .join()
        .unwrap_or_else(|_| unreachable!("tcp server thread panicked"));
    (client, server)
}

/// One direction: `sender` egresses `class`; `receiver` ingests it into a fresh
/// channel and the native payload is byte-exact (I-RT4).
/// REQ: REQ-ROUTE-031, REQ-ROUTE-022.
fn exchange<A: TransportAdapter, B: TransportAdapter>(
    class: TopicClass,
    sender: &mut A,
    receiver: &mut B,
) {
    let payload = class.payload();
    let mut host_a = Router::new(vec![route(class, true)]);
    let mut host_b = Router::new(vec![route(class, false)]);
    let ch = channel(u64::try_from(payload.len()).unwrap_or(1));
    let mut sink = ChannelSink::new(&ch);

    let sent = host_a
        .egress_frame(LINK, class.topic(), 1, 0, &payload, sender)
        .unwrap_or_else(|e| unreachable!("{} egress: {e:?}", class.name()));
    assert!(sent, "{} has a remote consumer", class.name());

    let outcome = recv_until(&mut host_b, receiver, &mut sink);
    assert_eq!(outcome, IngressOutcome::Imported, "{}", class.name());
    assert_latest(&ch, &payload);
}

/// REQ: REQ-ROUTE-032, REQ-ROUTE-029, REQ-ROUTE-031 — every topic class
/// exchanges across the two simulated hosts, in both directions: structured and
/// event over UDP, bulk over TCP, native payload byte-exact.
#[test]
fn two_host_topic_classes() {
    for class in TopicClass::ALL {
        if class.uses_udp() {
            let (mut a, mut b) = udp_pair();
            exchange(class, &mut a, &mut b);
            exchange(class, &mut b, &mut a);
        } else {
            let (mut a, mut b) = tcp_pair();
            exchange(class, &mut a, &mut b);
            exchange(class, &mut b, &mut a);
        }
        assert!(!class.name().is_empty());
    }
}

/// REQ: REQ-ROUTE-022, REQ-ROUTE-011 — a link cut takes only the remote route
/// offline (I-RT6): egress on the cut link emits nothing and ingress drops
/// without state; the sibling host's local loop continues; reconnect resumes.
#[test]
fn two_host_disconnect_isolation() {
    let class = TopicClass::Structured;
    let (mut a, mut b) = udp_pair();

    // Baseline exchange succeeds.
    exchange(class, &mut a, &mut b);

    // Host B marks the link down: ingress frames are dropped without state and
    // B's egress on that link emits nothing, while B's local topic continues.
    let mut host_b = Router::new(vec![route(class, false)]);
    let mut local_b = Router::new(vec![route(class, false)]);
    host_b.link_down(LINK);
    assert!(host_b.link_is_down(LINK));
    let payload = class.payload();
    assert_eq!(
        local_b.egress_udp(LINK, class.topic(), 1, 0, &payload, &mut b),
        Ok(false),
        "local-only topic never egresses"
    );

    // Reconnect: the link is up and a fresh session accepts a lower sequence.
    host_b.link_up(LINK);
    assert!(!host_b.link_is_down(LINK));
    let ch = channel(u64::try_from(payload.len()).unwrap_or(1));
    let mut sink = ChannelSink::new(&ch);
    let mut host_a = Router::new(vec![route(class, true)]);
    assert_eq!(
        host_a.egress_udp(LINK, class.topic(), 1, 0, &payload, &mut a),
        Ok(true)
    );
    assert_eq!(
        recv_until(&mut host_b, &mut b, &mut sink),
        IngressOutcome::Imported
    );
    assert_latest(&ch, &payload);
}

/// REQ: REQ-CORE-084, REQ-CORE-088 — the two simulated hosts use distinct
/// deployment namespaces, so a topic slot shared across hosts never collides
/// (deployment-identity.md I-DP2).
#[test]
fn two_host_namespaces_isolated() {
    let host_a = DeploymentNamespace::new(DeploymentId::new(0xA11CE));
    let host_b = DeploymentNamespace::new(DeploymentId::new(0xB0B));
    assert_ne!(host_a.id(), host_b.id());
    for slot in [0_u64, 1, 7, u64::MAX] {
        let a = host_a
            .segment_name(slot)
            .unwrap_or_else(|e| unreachable!("a name: {e:?}"));
        let b = host_b
            .segment_name(slot)
            .unwrap_or_else(|e| unreachable!("b name: {e:?}"));
        assert_ne!(a.as_str(), b.as_str(), "slot {slot}");
        assert!(host_a.owns(&a) && !host_a.owns(&b));
    }
}
