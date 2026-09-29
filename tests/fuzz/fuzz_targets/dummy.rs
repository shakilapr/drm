//! Placeholder fuzz target: smoke-proofs the harness wiring (work-plan M01W1-E4-T3).
//! REQ: REQ-QAL-004 (requirement with no test = 0).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    std::hint::black_box(data.len());
});
