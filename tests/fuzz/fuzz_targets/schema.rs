//! Schema-grammar fuzz target (work-plan M01W3-E4-T2): arbitrary bytes through
//! `drm_workspace::parser::parse_bytes` must yield a model or a diagnostic —
//! never a panic, abort, or hang. `-timeout=5` in CI budgets the hang side.
//! REQ: REQ-QAL-004, REQ-CORE-004 (parser total: all errors).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = drm_workspace::parser::parse_bytes(data);
});
