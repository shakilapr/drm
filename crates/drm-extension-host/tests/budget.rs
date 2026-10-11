//! Extension host budget fixtures
//! ([studio-protocol.md](../../docs/algorithms/studio/studio-protocol.md)
//! §Invariants `I-ST5`, §Test Matrix `ext_budget` ×{fuel, memory}). A runaway
//! extension is stopped deterministically by fuel metering or the memory cap,
//! and the host engine stays reusable — the engine is unaffected by a stopped
//! extension. `cargo test -p drm-extension-host`.
//!
//! REQ: REQ-STUDIO-004, REQ-STUDIO-020.

use drm_extension_host::{ExtensionBudget, ExtensionError, ExtensionHost, PreemptionStrategy};

/// A benign extension: computes `1 + 2` and returns.
/// REQ: REQ-STUDIO-004.
const BENIGN: &str = r#"
(module
  (func (export "drm_ext_run")
    (i32.add (i32.const 1) (i32.const 2))
    drop))
"#;

/// A runaway extension: an unbounded loop that never returns.
/// REQ: REQ-STUDIO-020.
const RUNAWAY: &str = r#"
(module
  (func (export "drm_ext_run")
    (loop $spin (br $spin))))
"#;

/// An oversize extension: declares 1000 pages (≈64 MiB) of linear memory,
/// far above a small declared cap. REQ: REQ-STUDIO-004.
const OVERSIZE: &str = r#"
(module
  (memory 1000)
  (func (export "drm_ext_run")))
"#;

/// REQ: REQ-STUDIO-020 — the M22W1 SPIKE records fuel metering as the chosen
/// preemption strategy: it is deterministic and needs no background ticker
/// thread (`I-ST5`).
#[test]
fn preemption_strategy_recorded() {
    assert_eq!(
        PreemptionStrategy::CHOSEN,
        PreemptionStrategy::Fuel,
        "SPIKE chose fuel metering over epoch interruption"
    );
}

/// REQ: REQ-STUDIO-004 — a runaway extension is stopped by the fuel budget, the
/// stop is deterministic across repeated runs, and the engine is unaffected: a
/// benign extension still runs afterward on the same host (`I-ST5`).
#[test]
fn ext_fuel_exhaustion_deterministic() {
    let host = ExtensionHost::new(ExtensionBudget::new(200_000, 1 << 20))
        .unwrap_or_else(|e| unreachable!("host: {e}"));
    host.run(BENIGN.as_bytes())
        .unwrap_or_else(|e| unreachable!("benign: {e}"));

    let first = host.run(RUNAWAY.as_bytes());
    let second = host.run(RUNAWAY.as_bytes());
    assert_eq!(
        first,
        Err(ExtensionError::FuelExhausted),
        "runaway extension must stop on fuel"
    );
    assert_eq!(
        first, second,
        "fuel exhaustion stop is deterministic across runs"
    );

    host.run(BENIGN.as_bytes())
        .unwrap_or_else(|e| unreachable!("engine reusable after stop: {e}"));
}

/// REQ: REQ-STUDIO-004 — an oversize memory declaration is refused by the memory
/// cap without running the guest, and the engine remains usable (`I-ST5`).
#[test]
fn ext_memory_cap() {
    let cap = 1 << 20;
    let host = ExtensionHost::new(ExtensionBudget::new(200_000, cap))
        .unwrap_or_else(|e| unreachable!("host: {e}"));
    match host.run(OVERSIZE.as_bytes()) {
        Err(ExtensionError::MemoryLimit { limit }) => assert_eq!(limit, cap),
        other => unreachable!("expected memory cap, got {other:?}"),
    }
    host.run(BENIGN.as_bytes())
        .unwrap_or_else(|e| unreachable!("engine reusable after memory stop: {e}"));
}

/// REQ: REQ-STUDIO-004 — the default budget admits a benign extension and the
/// fuel metric is reported (`I-ST5`).
#[test]
fn ext_budget_default_reports_fuel() {
    let host =
        ExtensionHost::new(ExtensionBudget::default()).unwrap_or_else(|e| unreachable!("{e}"));
    let run = host
        .run(BENIGN.as_bytes())
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(run.fuel_consumed() > 0, "benign call consumes fuel");
    assert_eq!(host.budget(), ExtensionBudget::default());
}
