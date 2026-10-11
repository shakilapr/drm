# Metrics — Month 07 (M07)

Normative source: [work-plan.md](../../work-plan.md) W3 Bindings, Toolchain &
Verification. One section per weekly tranche.

## M07W1 — Phase 19 Gate (Memory & Precommit), C ABI Completion

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 19)` | PASS | PASS — `docs/gates/phase-19.md` |
| `FFI_COMPLETE` | full op set real-impl green (open/publish/read/borrow/copy/close) | `drm-ffi` 30 tests; undeclared topic ⇒ `Permission`; empty-channel borrow ⇒ `NoSample`; close symmetry |
| `NO_UNWIND` | fail-stop policy (I-A6: no `catch_unwind`) | `no_unwind` 2 tests: release profile `panic = "abort"`; every export returns a `Status` on bad-but-non-null args |
| `FFI_NEGATIVES` | full edge-case matrix green | `ffi_negative` 12 tests |
| `COUNTERS` | CCB counter surface complete | `drm-core` `counters` (write_seq-derived last_sequence, drop/overrun/pool_exhausted/epoch/reclaim flag) |
| `PRECOMMIT_E2E` | activation success + refusal re-checked after identity change | `drm-core` `precommit_e2e` |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `drm-ffi` close symmetry + negative matrix + fail-stop policy
test; `drm-core` `ChannelCounters` accessor + `counters`/`precommit_e2e` suites.
**Fixed:** `drm_consumer_close` now returns `Busy` on an outstanding borrow
(consumer-registration I-CR4) instead of releasing it — removes a doc/impl
divergence. The week plan's "catch_unwind shield" wording is superseded by the
normative I-A6 fail-stop policy.

## M07W2 — C++ Binding (Phase 20)

| Variable | Target | Measured |
|---|---|---|
| `CPP_BORROW_GREEN` | RAII release on every scope exit incl. exceptions (I-B1) | `bindings/cpp/tests/borrow_raii.cpp`: normal exit, early return, exception unwinding, move semantics — release exactly once proven by counter + ABI single-borrow rule; g++ 13.3 + clang++ 18.1 |
| `CPP_NODE_API` | undeclared ops absent (compile-fail, I-B2) | 3 compile-fail fixtures (copy-borrow, undeclared op, raw-pointer escape) rejected by both compilers |
| `CPP_INTEROP` | C++↔C/Rust/Python cells green | driver also passes linked against the real `drm-ffi` cdylib; the full 4-language matrix is the Phase 20 completion gate |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: header-only `bindings/cpp/include/drm/drm.hpp` (move-only RAII
`Borrow<T>`/`Context`/`Publisher`/`Consumer`), assert-based driver, two-TU ODR
test, compile-fail fixtures, and `bindings/cpp/tests/run.sh` (g++ + clang++).
E3 lane work was already delivered (M03W3/M04W3/M05W1: `PeerLanes`,
`LaneSocket`, `PeerEgress`, saturation/mixed-class tests).

## M07W3 — Rust Binding (Phase 21), C++ Gate Prep

| Variable | Target | Measured |
|---|---|---|
| `RUST_SAFE_GREEN` | borrow lifetimes prevent use-after-release in safe code (I-B3) | `drm` SDK (`bindings/rust/crates/drm`): `BorrowGuard<'chan>` with `payload(&self) -> &[u8]` bound to the guard, `!Send + !Sync` via `PhantomData<*mut ()>`, RAII `Drop` release; `cargo test -p drm sdk` 4 tests |
| `COMPILE_FAIL_SET` | forbidden lifetime/contract cases rejected at compile time | trybuild 3 fixtures: payload outliving guard (E0515), moved-from guard (E0382), `!Send`/`!Sync` violation (E0277) |
| `GOLDEN_MATCH` | Rust SDK layouts match golden bytes | SDK defines no `#[repr(C)]` type (ABI layout lives in `drm-ffi`); the `sdk_defines_no_repr_c_structs` test enforces this so a stray layout type fails the suite |
| `LANES_E2E` | two-peer lanes under load + reconnect green | delivered earlier (`drm-router` `lane`/`lane_egress`/`lane_mixed_class`/`transport::session`) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `bindings/rust/crates/drm` safe SDK (new workspace member, layer
`bindings`); `xtask` layering map extended so the crate is **enforced** (not
skipped); Phase 20 gate draft.
**Fixed:** the `platform::cleanup` flock flake (same class as the lib flake) —
`spawn_child` now serializes against the flock assertions via a process-wide
guard; 0/30 failures after (was intermittent), and stale `/dev/shm` segments
cleared.
