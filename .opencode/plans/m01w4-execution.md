# DRM M01W4 — Approved Execution Plan (docs first)

## Step 0 — land the canon tranche (uncommitted, built)
- `crates/drm-workspace/src/canon/value.rs`:
  - line 60: backtick `drm-core` / `read_u32` in doc comment
  - lines 344 & 486: replace `mut hi` counter + `.nth(hi)` with pre-built
    header-member iterator (`headers.next()`) in BOTH `encode` and `decode`
- `cargo xtask fmt && cargo xtask lint && cargo test --workspace`
- commit: `feat(workspace): canon type system — two-region layout, interpreter codec, 48-cell boundary matrix`

## Step 1 — docs batch
- E1-T3/E1-T4: `docs/algorithms/memory-budget.md` — fill Protocols gaps:
  static-vs-activation split, refuse-on-failure activation (M6), worst-case
  formula (Σ full-bound slots; RT locked>lockable ⇒ hard error; >budget ⇒ hard
  error; >RAM×budget ⇒ warning; nominal informational) + dual-derivation
  worked example (E1+E2 agree = gate)
- E3-T1: expand `docs/algorithms/router/serial-mcu.md` — one sample = one
  bounded frame; CRC where link lacks checksums; no flow-control ⇒ shaped at
  router; MCU opaque payload `bytes[<=N]`; escape/stuffing options table
  (Phase 32 decision)

## Step 2 — code items
- E3-T2: MTU edge tests (68/1280/1492/1472 + override-below-floor) in drm-router
- E4-T2: drm.h spike ext — enumerator-switch test file, opaque-handle pattern,
  `#if DRM_ABI_VERSION >= 0` guard; gcc+clang c99/c11 syntax gates
- E4-T3: golden composites first — ≥5 hand-computed two-region struct vectors
  ({json,hex}); extend Rust runner + C reader; then Python ctypes reader
  (`bindings/python/`, pytest/ruff/mypy-strict) over primitives + composites → 3/3 langs

## Step 3 — aarch64/LE (E1-T5)
- `canon_le_assert` test in drm-core + CI arm64 job wiring (local run blocked)

## Step 4 — metrics-m01 (E4-T4)
- variables actual-vs-target ledger, gate packet, README board, CHANGELOG,
  full final gate run (fmt/lint/test/layering/traceability)

Commit per step; gates after every step.
