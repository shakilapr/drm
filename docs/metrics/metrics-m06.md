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
