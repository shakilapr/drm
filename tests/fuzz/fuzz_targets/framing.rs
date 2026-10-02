//! Framing fuzz target (work-plan M02W2-E4-T3): arbitrary bytes through
//! `drm_router::framing` record/multiplex decode must yield payload or a
//! status error — never a panic, abort, or huge allocation.
//! REQ: REQ-QAL-004, REQ-ROUTE-038, REQ-ROUTE-032.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Record decode at a few positions derived from the input: any position
    // and any byte pattern must be payload-or-error, never a panic.
    let positions = [
        0usize,
        1,
        data.len() / 2,
        data.len().saturating_sub(1),
    ];
    for pos in positions {
        if pos < data.len() {
            let _ = drm_router::framing::record::decode_record(data, pos);
        }
    }
    // Multiplex decode of the whole buffer.
    if let Ok(records) = drm_router::framing::datagram::decode_multiplex(data) {
        // Successful decode implies every record is a real sub-slice of the
        // input (no out-of-range slice can have been produced).
        let total: usize = records
            .iter()
            .map(|r| drm_router::framing::record::record_len(r.payload.len()))
            .sum();
        assert!(total <= data.len(), "decoded records exceed the input");
    }
});
