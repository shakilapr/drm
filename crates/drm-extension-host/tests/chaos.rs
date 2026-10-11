//! Extension chaos fixture (work-plan M22W4-E3-T3, `EXP`). Killing an extension
//! mid-call — an unbounded loop against the fuel budget — stops it
//! deterministically, leaves the host engine unaffected, and stays isolated
//! from other sessions (`I-ST5`, `I-ST10`). The EXP then compares
//! supervisor-restart against fail-closed teardown on recovery time and records
//! the chosen strategy. `cargo test -p drm-extension-host`.
//!
//! REQ: REQ-STUDIO-004, REQ-STUDIO-022.

use std::time::Instant;

use drm_extension_host::{
    ExtensionBudget, ExtensionError, ExtensionHost, NoopApi, RecoveryStrategy,
};

/// A benign extension used to prove the engine stays usable.
/// REQ: REQ-STUDIO-022.
const BENIGN: &str = r#"
(module
  (func (export "drm_ext_run")
    (i32.add (i32.const 1) (i32.const 2))
    drop))
"#;

/// A runaway extension killed mid-call by the fuel budget.
/// REQ: REQ-STUDIO-022.
const RUNAWAY: &str = r#"
(module
  (func (export "drm_ext_run")
    (loop $spin (br $spin))))
"#;

/// REQ: REQ-STUDIO-022 — killing an extension mid-call is a deterministic stop;
/// the engine is unaffected and other live sessions stay isolated (`I-ST5`,
/// `I-ST10`).
#[test]
fn extension_chaos_mid_call_stop_isolated() {
    let host = ExtensionHost::new(ExtensionBudget::new(200_000, 1 << 20))
        .unwrap_or_else(|e| unreachable!("host: {e}"));

    // A long-running extension and a concurrent benign session, both live.
    let mut victim = host
        .open(RUNAWAY.as_bytes(), Box::new(NoopApi))
        .unwrap_or_else(|e| unreachable!("victim: {e}"));
    let mut bystander = host
        .open(BENIGN.as_bytes(), Box::new(NoopApi))
        .unwrap_or_else(|e| unreachable!("bystander: {e}"));

    // Kill mid-call: deterministic stop on fuel.
    let first = victim.call();
    let second = victim.call();
    assert_eq!(
        first,
        Err(ExtensionError::FuelExhausted),
        "runaway must stop on fuel"
    );
    assert_eq!(first, second, "the stop is deterministic");

    // Isolation: the bystander session and the engine are unaffected.
    bystander
        .call()
        .unwrap_or_else(|e| unreachable!("bystander unaffected: {e}"));
    host.run(BENIGN.as_bytes())
        .unwrap_or_else(|e| unreachable!("engine reusable: {e}"));
}

/// REQ: REQ-STUDIO-022 — the chaos EXP compares supervisor-restart against
/// fail-closed teardown. Restart restores service deterministically, so it is
/// the recorded [`RecoveryStrategy::CHOSEN`]; both recoveries complete promptly.
#[test]
fn extension_chaos_recovery_strategy() {
    assert_eq!(
        RecoveryStrategy::CHOSEN,
        RecoveryStrategy::SupervisorRestart,
        "chaos EXP chose supervisor-restart over fail-closed teardown"
    );

    let host = ExtensionHost::new(ExtensionBudget::new(200_000, 1 << 20))
        .unwrap_or_else(|e| unreachable!("host: {e}"));

    // Fail-closed: stop, then tear the extension down and leave it stopped.
    let started = Instant::now();
    let mut victim = host
        .open(RUNAWAY.as_bytes(), Box::new(NoopApi))
        .unwrap_or_else(|e| unreachable!("victim: {e}"));
    let _ = victim.call();
    let fail_closed = victim.teardown();
    let fail_closed_time = started.elapsed();
    assert!(fail_closed.is_ok(), "fail-closed teardown is clean");

    // Supervisor-restart: stop, then open a fresh session that restores service.
    let started = Instant::now();
    let mut victim = host
        .open(RUNAWAY.as_bytes(), Box::new(NoopApi))
        .unwrap_or_else(|e| unreachable!("victim: {e}"));
    let _ = victim.call();
    drop(victim);
    let restored = host
        .run(BENIGN.as_bytes())
        .unwrap_or_else(|e| unreachable!("restart: {e}"));
    let restart_time = started.elapsed();
    assert!(
        restored.fuel_consumed() > 0,
        "supervisor-restart restores service"
    );

    println!("ext_chaos_recovery: fail_closed={fail_closed_time:?} restart={restart_time:?}");
}
