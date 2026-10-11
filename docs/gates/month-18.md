# Month-18 Review — Hardening Close, Soak Escalation, M19 Spatial Go

Chaired by E3 (review chair) at M18W4, Friday. Verdict: **ON_TRACK** — the
hardening sprint closed with every gate green (`HARDENING_GATE = PASS`), the
daily 24 h soak is scheduled over rotating topologies
(`SOAK_DAILY = scheduled`), and the plan is go for M19 (spatial wave start).

Normative sources: [work-plan.md](../../work-plan.md) W6 hardening →
W8 qualification; [qualification.md](../algorithms/program/qualification.md)
(I-QAL2/I-QAL5/I-QAL6/I-QAL7), [soak-ledger.md](../spec/soak-ledger.md),
[spatial-core.md](../algorithms/spatial/spatial-core.md),
[spatial-index.md](../algorithms/spatial/spatial-index.md),
[spatial-queries.md](../algorithms/spatial/spatial-queries.md),
[studio-protocol.md](../algorithms/studio/studio-protocol.md). REQ:
REQ-QAL-005, REQ-QAL-006, REQ-QAL-007, REQ-QAL-009, REQ-QAL-010, REQ-QAL-011,
REQ-QAL-012, REQ-QAL-013, REQ-QAL-014.

## 1. Variables actuals vs targets (M18)

| Week | Variable | Target | Measured |
|---|---|---|---|
| M18W1 | `FUZZ_TARGETS_LIVE` | schema + framing + hazard + datagram + serial targets nightly | 5/5 + verify-fixture — [m18w1.md](m18w1.md) |
| M18W1 | `CHAOS_GREEN` | kill -9 matrix + PID reuse + exhaustion storms green | 1 — [m18w1.md](m18w1.md) |
| M18W1 | `FUZZ_CRASHES` | 0 untriaged | 0 — [fuzz-triage-board.md](../spec/fuzz-triage-board.md) |
| M18W2 | `BENCH_BASELINE` | latency/throughput/router p50/p99 recorded | recorded — [bench-baseline.md](../spec/bench-baseline.md) |
| M18W2 | `BENCH_INVARIANTS` | zero-alloc + bounded-time in-bench | green — `REQ-QAL-012` |
| M18W2 | `STUDIO_PANELS` | Graph + Run functional | functional — `REQ-STUDIO-017/018` |
| M18W3 | `MATRIX_FULL_GREEN` | all supported cells green ×1 | 1 — [m18w3.md](m18w3.md) |
| M18W3 | `SANITIZER_FULL` | ASan/TSan/Miri matrix green | 1 — [sanitizers.yml](../../.github/workflows/sanitizers.yml) |
| M18W3 | `P0P1_COUNT` | P0 = 0; P1 ≤ documented open set | P0 = 0 — [m18w3.md](m18w3.md) |
| M18W4 | `HARDENING_GATE` | fuzz/chaos/bench/matrix/sanitizer green | PASS — [m18w4.md](m18w4.md) |
| M18W4 | `SOAK_DAILY` | 24 h soak scheduled daily (I-QAL2) | scheduled — [soak-nightly.yml](../../.github/workflows/soak-nightly.yml) |
| M18W4 | `MONTH18_REVIEW` | ON_TRACK | ON_TRACK (this document) |
| M18W4 | `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## 2. Hardening gate ledger

| Gate | Source | Verdict |
|---|---|---|
| Fuzz fleet (6 targets, nightly) | [fuzz-nightly.yml](../../.github/workflows/fuzz-nightly.yml) | green (`FUZZ_CRASHES = 0`) |
| Chaos/stress (kill matrix, PID reuse, storms, link flapping) | `cargo xtask test --stress` | green (`CHAOS_GREEN = 1`) |
| Bench invariants (zero-alloc + bounded-time) | `crates/drm-core/benches`, `crates/drm-router/benches` | green (`REQ-QAL-012`) |
| Conformance matrix (OS×language×transport) | `cargo xtask test --cross-os` | full green ([m18w3.md](m18w3.md)) |
| Sanitizers (ASan/TSan/Miri) | [sanitizers.yml](../../.github/workflows/sanitizers.yml) | 0 findings |
| Defect ledger | [fuzz-triage-board.md](../spec/fuzz-triage-board.md) | `P0 = 0` |
| Soak (daily 24 h, rotating topologies) | [soak-nightly.yml](../../.github/workflows/soak-nightly.yml), [soak-ledger.md](../spec/soak-ledger.md) | scheduled (first clean run; gate `PENDING`, 1/3) |

## 3. Studio protocol-level status

| Item | Doc | Status |
|---|---|---|
| Protocol doc (I-ST1..I-ST13) + host | [studio-protocol.md](../algorithms/studio/studio-protocol.md) | merged |
| Graph (read-only route/link projection) | `REQ-STUDIO-017` | functional |
| Run (supervisor start/stop, `runtimeStats`) | `REQ-STUDIO-018` | functional |
| Electron packaging (bundled `drm`, sandboxed renderer) | `REQ-STUDIO-019` | functional |

## 4. Risks

| Risk | Mitigation | Status |
|---|---|---|
| Spatial wave starts E2/E1-heavy | Studio maintenance runs parallel (E4); spatial scope pinned to [spatial-core.md](../algorithms/spatial/spatial-core.md) | contained |
| Soak findings late in the hardening sprint | three-consecutive-clean rule; findings carry to M19 (E1) without blocking the spatial start | contained |
| Matrix cell regression | regression permanence: every green M18 suite remains a blocking gate | green |

## 5. M19–M22 commitment + M23–M24 qualification

- **M19–M22 — spatial wave**: spatial types/spaces/collapse
  ([spatial-core.md](../algorithms/spatial/spatial-core.md)), the cell-hash
  spatial index ([spatial-index.md](../algorithms/spatial/spatial-index.md)),
  and spatial queries ([spatial-queries.md](../algorithms/spatial/spatial-queries.md));
  Studio maintenance in parallel (E4). Soak continues daily; findings land in
  M19 (E1).
- **M23–M24 — qualification**: whole-system qualification
  ([qualification.md](../algorithms/program/qualification.md)) — traceability
  closure (I-QAL1), soak ledgers (I-QAL2), reproducible builds (I-QAL3),
  matrix closure (I-QAL4), bench invariants (I-QAL5), defect ledger (I-QAL6),
  fuzz/sanitizer resolution (I-QAL7).

## 6. Carry-over

| Item | Origin | Disposition |
|---|---|---|
| Soak three-consecutive-clean clock | M18W4 | continues daily; gate turns PASS after 3 clean runs |
| P1 remainder | M18W3/W4 | tracked on [fuzz-triage-board.md](../spec/fuzz-triage-board.md) |
| Continuous-aging (≥ 72 h) + accelerated boundary suite | qualification I-QAL2 | M23 |
| MCU 2 h target soak | M15/M16 | deferred to `mcu-cross` hardware lane |

REQ: REQ-QAL-005, REQ-QAL-006, REQ-QAL-007, REQ-QAL-009, REQ-QAL-010,
REQ-QAL-011, REQ-QAL-012, REQ-QAL-013, REQ-QAL-014, REQ-STUDIO-017,
REQ-STUDIO-018, REQ-STUDIO-019.
