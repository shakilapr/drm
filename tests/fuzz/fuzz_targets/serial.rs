//! Serial-bomb fuzz target (work-plan M18W1-E3-T1): arbitrary byte streams
//! into the serial framing decoder — one-shot `frame_decode`, the streaming
//! `SerialTransport::recv_frame` resynchronizing on `0x00`, and the COBS
//! escape round-trip — must drop/resync or return a status, never panic or
//! hang (I-SR3/I-SR4). `-timeout=5` budgets the hang side.
//! REQ: REQ-QAL-004, REQ-QAL-007, REQ-QAL-009, REQ-ROUTE-025, REQ-ROUTE-026.

#![no_main]

use std::io::Cursor;

use drm_router::transport::TransportAdapter;
use drm_router::transport::serial::{
    SerialLinkConfig, SerialTransport, cobs_decode, cobs_encode, cobs_max_encoded_len, frame_decode,
};
use libfuzzer_sys::fuzz_target;

/// Drives the serial decoder with `data` as a raw byte stream.
/// REQ: REQ-QAL-009, REQ-ROUTE-025.
fn bomb(data: &[u8]) {
    // A decoder bound in the legal range (`>= HEADER_LEN + CRC`), derived from
    // the input so the fuzzer explores the bound as well as the bytes.
    let bound = 32_u16 + u16::from(data.first().copied().unwrap_or(0));
    let Ok(cfg) = SerialLinkConfig::new(115_200, bound) else {
        return;
    };

    // One-shot decode of the whole buffer: payload-or-status, never a panic.
    let mut decoded = vec![0_u8; cfg.max_coded_input_len()];
    let _ = frame_decode(&cfg, data, &mut decoded);

    // Streaming decode: feed the same bytes through the resynchronizing
    // transport decoder, consuming whole frames until EOF or the first error.
    let mut transport = SerialTransport::new(Cursor::new(data.to_vec()), cfg.clone());
    let mut frame_buf = vec![0_u8; cfg.max_decoded_len()];
    for _ in 0..data.len().saturating_add(1) {
        match transport.recv_frame(&mut frame_buf) {
            Ok(Some(_)) => {}
            _ => break,
        }
    }

    // COBS is reversible for arbitrary bytes, including zero (I-SR4).
    let mut encoded = vec![0_u8; cobs_max_encoded_len(data.len())];
    if let Ok(n) = cobs_encode(data, &mut encoded) {
        let mut round = vec![0_u8; data.len().max(1)];
        if let Ok(m) = cobs_decode(&encoded[..n], &mut round) {
            assert_eq!(&round[..m], data, "COBS encode/decode must be reversible");
        }
    }
}

fuzz_target!(|data: &[u8]| {
    bomb(data);
});
