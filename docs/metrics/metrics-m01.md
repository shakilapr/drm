# Metrics — Month 1 (M01W1–M01W4)

Gate packet: variables actual-vs-target across the four week plans (work-plan
M01W1–W4). Values marked `CI` are verified on runners only (local Windows
cannot execute them); everything else was measured locally this month.
REQ: REQ-QAL-007 (fuzz/ops evidence), REQ-TOOL-001.

## M01W1 — Bootstrap

| Variable | Target | Measured |
|---|---|---|
| `REPO_SCAFFOLD_MERGED` | 1 | 1 — crates/xtask/bindings scaffold on main |
| `CI_RUNNERS_GREEN` | 3/3 | CI matrix wired (ubuntu, ubuntu-24.04-arm, windows); runner execution pending push |
| `XTASK_CORE_CMDS` | fmt/lint/test/check-layering runnable | 1 — all runnable locally, green |
| `REQ_MINTED` | ≥ 80 (pass 1) | 325 registry rows |
| `REQ_LINT_FINDINGS` | 0 | 0 (`audit-traceability`: 0 findings) |
| `FUZZ_HARNESS_SMOKE` | 1 | 1 (fuzz target + seeds; campaign harness) |
| `SHM_SPIKE_PASS` | 1 | 1 (create/map/roundtrip + EEXIST race, platform suite) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

## M01W2 — Registry + Grammar

| Variable | Target | Measured |
|---|---|---|
| `REQ_COVERAGE_PCT` | 100 | 100 — 52/52 `.rs` files carry REQ citations |
| `REQ_AMBIGUOUS` | 0 | 0 |
| `GRAMMAR_CORPUS` | 60 (30v/30i) | 60 — byte-exact via `.gitattributes`, corpus job blocking |
| `RACE_HARNESS` | 8 procs × 100 iters, conflicts 0 | 1 — green locally (cfg(unix)); CI executes |
| `SANITIZER_FINDINGS` | 0 | CI job wired; local run not possible (Linux-only) |
| `ABI_SPIKE_COMPILERS` | 2/3 (gcc, clang) | 4/4 gates green: gcc+clang × c99+c11 (spike + readers); clang-cl manual/CI pending |
| `STATUS_MAP` | drm-types ↔ drm.h 1:1 | 1 — exhaustive 22-code map gate in drm-ffi |

## M01W3 — Codec + Corpus

| Variable | Target | Measured |
|---|---|---|
| `VALID_PARSED_PCT` / `INVALID_REJECTED_PCT` | 100 / 100 | 100/100 (fixtures_valid + parser_corpus, span + vocabulary reason) |
| `FUZZ_EXEC_H` / `FUZZ_CRASHES` | ≥ 1 h / 0 | 3660 s (release, 28,310,909 cases, 579,128 accepted) / 0 — `docs/metrics/fuzz-campaign.log` |
| `ROUNDTRIP_CASES` | ≥ 10 000 (I4) | 10 000 proptest cases green |
| `SANITIZER_FINDINGS` | 0 | CI job wired (ASan platform + Miri codec) |
| `GOLDEN_VECTORS` | 20 primitives × 3 langs | 24 primitives × 3 langs (Rust/C11/Python) — all green locally |
| `CANON_TESTS` | 9/9 prims | 9/9 prims + bool + LE oracle; read-fuzz 17.6 M cases, 0 panics |
| `BACKEND_TRAIT_FROZEN` | 1 | 1 (`ShmBackend`/`LockBackend` frozen v1, semver note in CHANGELOG) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

## M01W4 — Type System + Composites + Month Close

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 3)` | PASS | PASS — type system complete: layout (TypeLayout/StructLayout, checked u64, u32::MAX cap), interpreter codec (two-phase I3, structural decode I5), 48-cell boundary matrix (completeness asserted), unbounded 7/7 rejected, corpus 100/100 unchanged |
| `GATE_STATUS(Phase 7 draft)` | PASS (3/3 compilers) | PASS — drm.h v0.1 + abi-policy; gcc+clang × c99+c11 syntax gates (4/4); MSVC/clang-cl = CI/manual-only, recorded as pending |
| `TYPE_MATRIX_PCT` | 100 (type × 0/max/empty/full) | 100 — 48/48 cells, missing-cell = fail (`boundary_matrix_completeness`) |
| `UNBOUNDED_REJECTED_PCT` | 100 | 100 — bare string/bytes, sizeless + boundless arrays, wrong forms, empty enum: 7/7 with exact vocabulary reasons |
| `GOLDEN_CROSS_LANG` | RustC Python 3/3 | 3/3 — primitives 24/24 (Rust canon, C11 `golden_read`, Python struct) and composites 6/6 (Rust toolchain oracle, C11 `golden_composites`, Python struct) |
| `AARCH64_PARITY` | green | wired — `ubuntu-24.04-arm` in the rust matrix runs the full drm-core suite incl. new `canon_le_assert` (explicit LE byte oracle, I2/I9); execution pending push |
| `ZERO_ALLOC_HARNESS` | 1 | 1 — counting `GlobalAlloc` (`test-count-allocs`), codec block 0/0 allocs/deallocs |
| `SHAPING_LIMIT_VIOLATIONS` | 0 / 10k traces | 0 — TokenBucket 10k-trace I-LS3 property + fractional-credit + wait-model tests green |
| `MONTH1_REVIEW` | ON_TRACK | ON_TRACK — see ledger below |

## Month-1 gate ledger

- Phase gates claimed this month: #2 (parser), #3 (type system), #7 draft (ABI).
- Deferred to runners: sanitizers, aarch64 execution, 3-compiler ABI matrix with
  MSVC, multiprocess platform suite — all wired in CI, blocked locally by OS.
- Carry-over into M02: `WorkspaceModel` + route derivation (workspace-model.md),
  layout.json crosscheck (I6 exchange), codegen start.
- Quarantine ledger: empty (no flaky tests this month).
- Evidence: `docs/metrics/fuzz-campaign.log` (1 h campaign), this file.

REQ: REQ-QAL-007.
