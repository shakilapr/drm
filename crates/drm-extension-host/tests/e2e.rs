//! Extension end-to-end fixture (work-plan M22W4-E3-T1; `EXT_E2E_GREEN`). A
//! sample extension is loaded into the `wasmtime` host, makes one call on the
//! `studio-extension-api` surface, and is torn down deterministically within
//! budget. `cargo test -p drm-extension-host`.
//!
//! REQ: REQ-STUDIO-004, REQ-STUDIO-021.

use drm_extension_host::{ExtensionApi, ExtensionBudget, ExtensionHost};

/// The sample extension under test (work-plan M22W4-E3-T1).
/// REQ: REQ-STUDIO-021.
const SAMPLE: &str = include_str!("../fixtures/sample_extension.wat");

/// A doubling `studio-extension-api` implementation: replies with `2·request`.
/// REQ: REQ-STUDIO-021.
struct DoublingApi;

impl ExtensionApi for DoublingApi {
    /// Returns `2·request` (saturating). REQ: REQ-STUDIO-021.
    fn call(&mut self, request: i32) -> i32 {
        request.saturating_mul(2)
    }
}

/// REQ: REQ-STUDIO-021 — `load` → `call` → `teardown` is green: the sample
/// extension loads, calls the host API exactly once, and tears down
/// deterministically within budget; the lifecycle is repeatable byte-for-byte.
#[test]
fn extension_e2e_load_call_teardown() {
    let host = ExtensionHost::new(ExtensionBudget::new(1_000_000, 1 << 20))
        .unwrap_or_else(|e| unreachable!("{e}"));

    let mut session = host
        .open(SAMPLE.as_bytes(), Box::new(DoublingApi))
        .unwrap_or_else(|e| unreachable!("load: {e}"));
    let call = session.call().unwrap_or_else(|e| unreachable!("call: {e}"));
    assert_eq!(call.api_calls(), 1, "sample makes one API call");
    assert!(call.fuel_consumed() > 0, "call consumes fuel");
    let teardown = session
        .teardown()
        .unwrap_or_else(|e| unreachable!("teardown: {e}"));
    assert_eq!(teardown.api_calls(), 1, "teardown makes one API call");

    // Deterministic teardown: a second full lifecycle reports the same metrics.
    let first = host
        .run_with_api(SAMPLE.as_bytes(), Box::new(DoublingApi))
        .unwrap_or_else(|e| unreachable!("run 1: {e}"));
    let second = host
        .run_with_api(SAMPLE.as_bytes(), Box::new(DoublingApi))
        .unwrap_or_else(|e| unreachable!("run 2: {e}"));
    assert_eq!(first, second, "lifecycle is deterministic");
    assert_eq!(first.api_calls(), 2, "run + teardown each call the API");
    assert!(first.fuel_consumed() > 0, "lifecycle consumes fuel");
}

/// REQ: REQ-STUDIO-004 — the sample extension respects a tight budget: with a
/// small fuel budget it still completes, and the engine stays reusable across
/// sessions (`I-ST5`).
#[test]
fn extension_e2e_budget_reusable() {
    let host = ExtensionHost::new(ExtensionBudget::new(100_000, 1 << 20))
        .unwrap_or_else(|e| unreachable!("{e}"));
    for _ in 0..3 {
        let run = host
            .run_with_api(SAMPLE.as_bytes(), Box::new(DoublingApi))
            .unwrap_or_else(|e| unreachable!("run: {e}"));
        assert_eq!(run.api_calls(), 2);
    }
}
