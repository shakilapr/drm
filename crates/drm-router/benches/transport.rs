//! Transport bench harness: the UDP datagram loopback and the serial
//! COBS/CRC32C frame codec. The serial codec is measured in-memory (encode +
//! decode over fixed buffers) and must be zero-allocation and bounded; the UDP
//! loopback is syscall-bound and only carries the bounded-time invariant.
//!
//! REQ: REQ-QAL-011 (transport throughput baseline), REQ-QAL-012 (in-bench
//! invariants), REQ-QAL-019 (bench-harness gating), REQ-ROUTE-025 (one bounded
//! frame per sample), REQ-ROUTE-032 (one sample per datagram).

mod common;

use std::hint::black_box;
use std::net::UdpSocket;
use std::time::Duration;

use criterion::{Criterion, criterion_main};

use drm_router::transport::serial::{SerialFrame, SerialLinkConfig, frame_decode, frame_encode};
use drm_router::wire::{HEADER_LEN, WireHeader};

/// p99 budget for a serial frame encode + decode round trip (in-memory codec).
const SERIAL_P99_BUDGET: Duration = Duration::from_micros(20);
/// p99 budget for one UDP loopback datagram round trip (syscall-bound).
const UDP_P99_BUDGET: Duration = Duration::from_micros(500);
/// A 64-byte payload carried by each transport bench.
const PAYLOAD: [u8; 64] = [0x7E; 64];
/// Declared serial decoded-frame bound: header + payload (`HEADER_LEN` = 28).
const MAX_FRAME_BYTES: u16 = 92;
const _: () = assert!(MAX_FRAME_BYTES as usize == HEADER_LEN + PAYLOAD.len());

/// Encodes then decodes one serial frame over caller-owned buffers, returning the
/// decoded payload length (`0` on any rejection — never reached on the bench path).
/// REQ: REQ-ROUTE-025.
fn serial_roundtrip(cfg: &SerialLinkConfig, wire: &mut [u8], decoded: &mut [u8]) -> usize {
    let header = WireHeader {
        flags: 0,
        wire_channel_id: 0x00C0_FFEE,
        stream_epoch: 0,
        source_seq: 1,
        payload_len: u32::try_from(PAYLOAD.len()).unwrap_or(u32::MAX),
    };
    let frame = SerialFrame {
        header,
        e2e_crc: None,
        payload: &PAYLOAD,
    };
    let Ok(wire_len) = frame_encode(cfg, &frame, wire) else {
        return 0;
    };
    match frame_decode(cfg, wire.get(..wire_len).unwrap_or(wire), decoded) {
        Ok(out) => out.payload.len(),
        Err(_) => 0,
    }
}

/// REQ: REQ-QAL-011, REQ-QAL-012 — serial frame codec throughput, zero-alloc.
fn bench_serial(c: &mut Criterion) {
    let cfg = match SerialLinkConfig::new(115_200, MAX_FRAME_BYTES) {
        Ok(cfg) => cfg,
        Err(e) => unreachable!("serial config: {e:?}"),
    };
    let mut wire = vec![0u8; cfg.max_wire_bytes()];
    let mut decoded = vec![0u8; cfg.max_coded_input_len()];
    assert_eq!(
        serial_roundtrip(&cfg, &mut wire, &mut decoded),
        PAYLOAD.len(),
        "serial round trip must carry the full payload"
    );

    common::assert_zero_alloc("serial_roundtrip", || {
        for _ in 0..1024 {
            let _ = serial_roundtrip(&cfg, &mut wire, &mut decoded);
        }
    });

    let samples = common::measure(4096, || {
        let _ = serial_roundtrip(&cfg, &mut wire, &mut decoded);
    });
    let p99 = common::percentile(&samples, 99);
    println!("router serial frame round trip: p99={p99:?}");
    assert!(
        p99 <= SERIAL_P99_BUDGET,
        "serial round trip p99 {p99:?} exceeds the {SERIAL_P99_BUDGET:?} budget"
    );

    c.bench_function("serial_frame_roundtrip", |b| {
        b.iter(|| black_box(serial_roundtrip(&cfg, &mut wire, &mut decoded)));
    });
}

/// REQ: REQ-QAL-011, REQ-ROUTE-032 — UDP datagram loopback round trip.
fn bench_udp(c: &mut Criterion) {
    let sock = match UdpSocket::bind("127.0.0.1:0") {
        Ok(sock) => sock,
        Err(e) => unreachable!("udp bind: {e}"),
    };
    let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
    let addr = match sock.local_addr() {
        Ok(addr) => addr,
        Err(e) => unreachable!("udp addr: {e}"),
    };
    let mut buf = [0u8; 64];

    let samples = common::measure(512, || {
        let _ = sock.send_to(&PAYLOAD, addr);
        let _ = sock.recv_from(&mut buf);
    });
    let p99 = common::percentile(&samples, 99);
    println!("router udp loopback: p99={p99:?}");
    assert!(
        p99 <= UDP_P99_BUDGET,
        "udp loopback p99 {p99:?} exceeds the {UDP_P99_BUDGET:?} budget"
    );

    c.bench_function("udp_loopback", |b| {
        b.iter(|| {
            let sent = sock.send_to(black_box(&PAYLOAD), addr);
            let got = sock.recv_from(&mut buf);
            black_box((sent, got))
        });
    });
}

/// Runs every transport benchmark group.
fn run_benches() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_serial(&mut criterion);
    bench_udp(&mut criterion);
    criterion.final_summary();
}

criterion_main!(run_benches);
