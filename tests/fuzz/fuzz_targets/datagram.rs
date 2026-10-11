//! UDP datagram fuzz target (work-plan M10W1-E4-T1): arbitrary bytes through
//! the datagram decode path — wire-header decode, encoded-length/record-bound
//! probes, and multiplex demux — must yield a value or a status error, never a
//! panic, abort, or huge allocation (I-RT3/I-RT18).
//! REQ: REQ-QAL-004, REQ-ROUTE-032, REQ-ROUTE-038, REQ-ROUTE-033.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // A received datagram begins with a wire header. Decode validates version,
    // flags, and reserved bits before any payload access (I-RT7).
    if let Ok(header) = drm_router::wire::WireHeader::decode(data) {
        // Declared length is bounded against the generated receive budget
        // before buffering; the probe must never overflow (I-RT18).
        let _ = drm_router::wire::record_bound_violated(header.payload_len, u32::MAX);
        // The framed size is header + payload + optional e2e trailer (I-RT3).
        let framed = header.encoded_len();
        assert!(framed >= drm_router::wire::HEADER_LEN);
    }

    // Multiplex datagrams concatenate length-prefixed records; malformed or
    // truncated input is an error, never a panic. Successful decode implies
    // whole sub-slices of the input.
    if let Ok(records) = drm_router::framing::datagram::decode_multiplex(data) {
        let total: usize = records
            .iter()
            .map(|r| drm_router::framing::record::record_len(r.payload.len()))
            .sum();
        assert!(total <= data.len(), "decoded records exceed the datagram");
    }
});
