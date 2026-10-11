# Metrics — Month 06 (M06)

Normative source: [work-plan.md](../../work-plan.md) W2 Core Runtime / W3 bindings.
One section per weekly tranche.

## M06W1 — Phase 16 Gate (Dead-Reader Recovery), C ABI Surface Start

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 16)` | PASS | PASS — `docs/gates/phase-16.md` |
| `RECLAIM_E2E` | reclamation sustained under real borrow patterns (I-E5) | `stress_reclaim_e2e` (ignored): `published=19281 dropped=980719 reclaimed=1324018 passes=156392 reads=3080972`, 0 I-HZ1 violations at `DRM_STRESS_CYCLES=1000000`; stable across 5×150k runs |
| `KILL_MATRIX` | kill at every borrow stage ⇒ reclaim only after proof | `reclaim_kill_matrix` 5/5; `reclaim_delayed_not_corrupt`, `reclaim_identity_snapshot_across_restart` |
| `FFI_STUB_GREEN` | endpoint stubs compile ×3 compilers | `bindings/c/tests/abi/abi_spike.c` syntax-clean under **gcc 13.3** and **clang 18.1** (`-std=c11 -Wall -Wextra -Werror -Ibindings/c/include`); MSVC not available on this host |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: reclaim e2e stress harness (`drm-core/tests/stress/reclaim_e2e.rs`);
C ABI stub surface — opaque `drm_context`/`drm_publisher`/`drm_consumer`,
`drm_status` mirror with the append-only `NotImplemented` code (`drm-ffi`,
`drm.h`), Rust-side null-arg/status-propagation tests (`drm-ffi/tests/abi_stub.rs`),
extended C spike.

## M06W2 — Phase 17 Gate (Lifecycle), C ABI Endpoints

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 17)` | PASS | PASS — `docs/gates/phase-17.md` |
| `FFI_ENDPOINTS` | open/close/publish/read real impl | `drm-ffi` 15 tests; endpoints backed in-process by a `drm-core` channel (cross-process SHM deferred) |
| `C_RUST_PARITY` | C-driven exchange == Rust-native (identical bytes) | `drm-ffi/tests/inproc_endpoint.rs::c_rust_byte_parity` |
| `VALIDATE_ONCE` | validation once at open, zero per-message checks | `validate_once_at_open_across_many_publishes` (per-message counter stays 0) |
| `BORROW_ABI` | explicit release; double-release detected | `borrow_double_release_is_invalid_arg` |
| `LIFECYCLE_E2E` | epoch observe/reset wired into the read loop | `consumer_read_loop_observes_publisher_restart` (`drm-core` lifecycle 17 passed) |
| `SOAK_24H` | router soak 0 anomalies | `xtask soak` duration-bounded scaffold; 24 h wall-time run is an operator/CI lane, not executed in-session |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `drm-ffi` in-process endpoint backing (open/close/publish/read/
borrow) + `drm.h` export declarations; lifecycle read-loop e2e; Phase 17 gate
doc. Known follow-ups: FFI status for bad release is `InvalidArg` (doc I-A5
provisional `InvalidBorrow`); copy-read export names `drm_read_*` vs doc
`drm_copy_*` — reconcile at the Phase 7 finalize gate.
