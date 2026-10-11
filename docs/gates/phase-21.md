# Phase 21 Gate — Rust Binding

Evidence is executable and stays green under the permanent gates.
Verdict: **PASS** (safe lifetimes, encoded `!Send + !Sync` guards, compile-fail
set, semver baseline, and cross-language byte identity). The generated
per-manifest node API lands with the code generator in Phase 24.

Implementation: safe SDK `bindings/rust/crates/drm`
([bindings-cpp-rust-python.md](../algorithms/bindings/bindings-cpp-rust-python.md)
§Rust) over the C ABI ([c-abi-surface.md](../algorithms/bindings/c-abi-surface.md)).

## Criteria ⇄ evidence

| Criterion | Artifact | Test | Result |
|---|---|---|---|
| Borrow lifetimes prevent use-after-release in safe code | `BorrowGuard<'chan>` with `payload(&self) -> &[u8]` bound to the guard, `Drop` release (`bindings/rust/crates/drm/src/lib.rs`) | `cargo test -p drm` (`sdk_borrow_guard_payload_access`, `sdk_release_on_drop`); `tests/ui/payload_outlives_guard.rs` | PASS |
| `PhantomData<*mut ()>` makes guards `!Send + !Sync`; cross-thread move/share rejected | `_not_send_sync: PhantomData<*mut ()>` on `BorrowGuard` | trybuild `tests/ui/guard_not_send_sync.rs` (E0277) | PASS |
| Compile-fail set covers forbidden lifetime/contract cases | `tests/ui/{payload_outlives_guard,moved_from_guard,guard_not_send_sync}.rs` | `cargo test -p drm --test compile_fail` 3/3 | PASS |
| Generated layouts match golden bytes | SDK defines no `#[repr(C)]` type; ABI layout lives in `drm-ffi` and is golden-checked (`borrow_view_layout_matches_header`, `bindings/c/tests/golden/`) | `sdk_defines_no_repr_c_structs`; `cargo test -p drm-ffi`; C/C++/Python golden decoders | PASS |
| Interoperability matrix passes | one `drm-ffi` cdylib driven from C, C++, Rust, and Python (ctypes + PyO3) | `drm-ffi` `ffi_matrix`; `bindings/cpp/tests/golden`; `bindings/python/tests` | PASS (4-language golden; full transport matrix carries to Phase 24) |

## Test Matrix coverage (bindings-cpp-rust-python.md §Rust §Test Matrix)

| Case | Expected | Evidence |
|---|---|---|
| trybuild set (post-drop, cross-thread move/share) | I-B4 | `tests/compile_fail.rs` 3 fixtures |
| Owned reads (`read_latest` / `read_next`) | REQ-BIND-002 | `sdk_owned_read_latest`, `sdk_owned_read_next`, 3 doc-tests |
| No detached `repr(C)` layout in the SDK | golden identity | `sdk_defines_no_repr_c_structs` |
| API stability | no accidental break | `cargo semver-checks check-release -p drm --baseline-rev e513afe`: 202 checks, 202 pass, 0 fail |

## Invariant coverage (bindings-cpp-rust-python.md I-B4)

| Invariant | Test |
|---|---|
| I-B4 RustSafeNoUAF | `sdk` suite + `tests/ui/*.rs` trybuild set |
| I-B8 ConsumerLeasePerBorrower | `drm-ffi` `ffi_consumer_lease` (shared substrate) |

## Variables (M07W3 → M08W1)

| Variable | Target | Measured |
|---|---|---|
| `RUST_SAFE_GREEN` | lifetime-bound borrow guard (I-B4) | `cargo test -p drm`: 6 unit + 1 trybuild + 3 doc-tests |
| `COMPILE_FAIL_SET` | forbidden cases rejected at compile time | 3/3 trybuild fixtures |
| `SEMVER_BASELINE` | public API frozen, no accidental break | `cargo-semver-checks` 202/202 pass vs `e513afe` |
| `GOLDEN_MATCH` | SDK carries no layout type | `sdk_defines_no_repr_c_structs` |

## Requirements covered

REQ-BIND-016 (safe lifetimes, encoded `!Send + !Sync`), REQ-BIND-001/006
(borrow discipline), REQ-BIND-002 (owned reads + close), REQ-BIND-017 (status
mirror), REQ-BIND-021 (context lifecycle).

## Notes

- The generated node-API surface (per-manifest, schema-mapped) lands with the
  code generator in Phase 24; this gate proves the hand-written safe substrate
  and the scoping contract it must satisfy.
- `cargo-semver-checks` runs against the SDK's introduction revision
  (`e513afe`); the API additions in M08W1 (`read_next`, `ReadOutcome`) are
  non-breaking.
