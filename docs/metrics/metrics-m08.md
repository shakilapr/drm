# Metrics — Month 08 (M08)

Normative source: [work-plan.md](../../work-plan.md) W3 Bindings, Toolchain &
Verification. One section per weekly tranche.

## M08W1 — Phase 21 Gate (Rust Binding), Python Binding Start (Phase 22)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 21)` | PASS | PASS — [phase-21.md](../gates/phase-21.md) |
| `RUST_SAFE_GREEN` | safe lifetimes prevent use-after-release (I-B4) | `cargo test -p drm`: 6 unit + 1 trybuild + 3 doc-tests green; trybuild 3/3 fixtures (post-drop use, moved-from guard, cross-thread move/share) |
| `SEMVER_BASELINE` | public API frozen, no accidental break | `cargo semver-checks check-release -p drm --baseline-rev e513afe`: 202 checks, 202 pass, 58 skip, 0 fail (new `read_next`/`ReadOutcome` are additive) |
| `PY_SMOKE` | PyO3 open/publish smoke green | `maturin build` wheel for CPython 3.14; `python -m pytest bindings/python/tests`: 11 new tests green (import/ABI probe/open/publish/owned read/empty-channel `None`) |
| `PY_OWNED_DEFAULT` | I-B5 green (memory-identity test) | `test_numpy_is_owned_not_a_shared_view`: `Copy.numpy().ctypes.data` ∉ the backing channel's committed-payload address range; `Copy.bytes()` is a fresh application-owned copy; retained copies survive later publishes |
| `PY_CONTRACT_ERROR` | I-B7 green | `test_contract_error.py`: undeclared name/id ⇒ `ContractError` at first use; `ContractError` derives from `DrmError`; scope-exit close succeeds (no pinned hazard) |
| `ROUTER_LOOPBACK` | loopback perf vs M06W3 baseline; drift explained | `cargo run --release -p drm-router --example udp_pair`: 10 000 exchanges ×5 = 41.5–51.9 ms (~4.2 µs/exchange round trip); **no prior baseline existed** — M06W3 `ROUTER_BASELINE` was deferred to the pinned-machine perf lane, so this run establishes the reference |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables:

- Rust SDK polish: `Consumer::read_next` + `ReadOutcome` owned sequential read,
  doc-tests for `read_latest`/`read_next`/borrow, hazard-protocol borrow-lifecycle
  docs (`bindings/rust/crates/drm`).
- Phase 21 gate doc ([phase-21.md](../gates/phase-21.md)) with the
  criteria→artifact→test→result table.
- Python binding start: PyO3 native core `bindings/python/native` (workspace
  member `drm-python`, layer `bindings`), pure facade `bindings/python/drm`,
  maturin `pyproject.toml`, type stub, and owned-read/ContractError tests.
- `udp_pair` example now reports elapsed time and per-exchange cost (E3 perf
  evidence).

Notes:

- The Python native classes serialize endpoint calls on a `Mutex` and hold their
  owning `Context` alive (`Py<Context>`); the full free-threaded/GIL matrix
  (I-B10, REQ-BIND-022) is the Phase 22 gate.
- The pure facade resolves topic names against a small declared table
  (`control/target` ⇒ id 0) and raises `ContractError` at first use; the
  generated per-manifest table lands with codegen in Phase 24.
- The `drm-ffi` in-process backing does not reclaim borrower records within a
  context (`next_borrower` is monotonic); tests open a fresh context per case.

## M08W2 — Python Binding Completion (Phase 22), Codegen C Start (Phase 24)

| Variable | Target | Measured |
|---|---|---|
| `GEN_C_GREEN` | C generator outputs compile + pass golden (I-G1..I-G3) | `cargo test -p drm-workspace codegen_c`: 9/9 green — `gen_determinism` (double-run byte-identical, I-G1), `gen_affected_only` (one-topic change dirties only its units, I-G3), `gen_mtime_preserved` + `gen_mtime_preserved_fp_bump` (identical bytes ⇒ no rewrite, I-G2), `gen_reconciliation` (orphan prune), `gen_expected_vs_dirty`, `gen_node_header_compiles`, `gen_variable_topic_compiles`, `gen_compiles_and_decodes_golden` (`cc -std=c11 -Werror` compiles the rendered header and decodes a golden vector) |
| `PY_SCOPE_PINNED` | I-B5 forced-GC/exception suite green | native hooks landed (`Consumer._borrow_outstanding`); pytest suite deferred — no PyO3/maturin runtime in the gate environment |
| `PY_NUMPY` | views reference correct bytes | `Copy.numpy(dtype, shape)` derives a read-only, C-contiguous owned array from the generated schema descriptor (I-B5); byte-level pytest deferred with the same environment limit |
| `PY_INTEROP` | Python ↔ {C, C++, Rust} cells green | not exercised this week — carried to M08W3 Day 1 per plan carry-over |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Codegen slice in `drm-workspace::codegen` — `GenPlan`/`GenUnit`, directed
  capability fingerprints, stable `.drm/gen/c/<topic>/target.h` paths,
  content-compare atomic writes, and orphan reconciliation against the complete
  expected tree; C renderer in `codegen::render` (per-topic struct + inline
  encode/decode, node-scoped API surface).
- `crates/drm-workspace/tests/codegen_c.rs` — determinism, affected-only,
  mtime preservation, fingerprint-bump, reconciliation, and `cc` compile +
  golden-decode suites.
- Python binding polish — `Copy.numpy(dtype, shape)` schema-driven views and the
  `Consumer._borrow_outstanding` diagnostic hook for the forced-GC suite.

Notes:

- `drm-workspace::codegen` renders C only this slice; C++/Rust/Python renderers
  land in later phases (`LANGS` grows accordingly).
- The Python pytest suites (I-B5 scope, NumPy bytes, interop cells) need the
  maturin/PyO3 runtime that the local gate environment does not provide; the
  native surface they exercise is compiled and clippy-clean.
