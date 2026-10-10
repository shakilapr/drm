# Phase 10 Gate — Shared History Rings

Evidence mapping, [work-plan.md](../../work-plan.md) #10 criteria ⇄ tests.
Implementation: `drm-core::ring` ([ring-history.md](../algorithms/runtime/ring-history.md))
consumed by `drm-core::channel` ([channel-publish.md](../algorithms/runtime/channel-publish.md)).
Verdict: **PASS** (local release-build run; harness is cross-platform).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Exhaustive tests for depths 1…N | `ring::tests::retention_depths_1_to_64` — every depth R ∈ 1..=64 exercised for retention; sampled `ring_depths_property` over 65/100/333/1000/4096; nightly `stress_ring_depth_sweep` (65..=8192, pseudo-random) | PASS |
| Wraparound repeats without changing retained-history semantics | `wraparound` (≥ 1 × 10⁶ publishes across many laps), `retention_depths_1_to_64`, and the Phase 9 billion-cycle run ([phase-09.md](phase-09.md), 3 × 10⁹ publish/read at 0 torn/mixed/invalid) | PASS |
| Exactly the declared `R` completed sequences remain addressable | `retention_exact` (I1), `retention_depths_1_to_64`, `ring_depths_property`; channel-level window in `drm-core::channel` cursor suites | PASS |

## Invariant coverage (ring-history.md I1–I8)

| Invariant | Test |
|---|---|
| I1 RetentionExact | `retention_exact`, `retention_depths_1_to_64`, `ring_depths_property` |
| I2 WrapStable | `wraparound`, `ring_depths_property` |
| I3 EntryCoherence | `ring_entry_coherence` |
| I4 ReleaseOrdered | `ring_release_order` |
| I5 WatermarkExcludesPriorRun | `first_valid_watermark` |
| I6 PortableCell | `ring_layout_golden` |
| I7 CounterNoWrap | `sequence_exhaustion` |
| I8 CrashStable | `repair_odd_stamps` (odd-stamp repair before publish); full crash matrix deferred to the sanitizer/soak lane |

## Variables (M03W4)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 10)` | PASS | PASS |
| `RING_DEPTHS_GREEN` | R ∈ 1..=64 exhaustive; sampled 65..4096 | `retention_depths_1_to_64`, `ring_depths_property` (65/100/333/1000/4096); nightly `stress_ring_depth_sweep` 65..8192 |
| `RETENTION_EXACT` | exactly R addressable (I1) | `retention_exact`, `retention_depths_1_to_64`, `ring_depths_property`, `wraparound` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Requirements covered

REQ-CORE-061 (counter never wraps), REQ-CORE-062 (addressable window never
predates `first_valid_seq`), REQ-CORE-063 (sequence → `{slot_index, generation}`
entry), REQ-CORE-064 (fixed-width lock-free atomics), REQ-CORE-065 (entry
committed before sequence advance), REQ-CORE-066 (`@history: N` retains exactly
R), REQ-CORE-067 (wraparound preserves semantics).

## Notes

- Depth/window evidence here also feeds the Phase 11 cursor gate: the ring's
  addressable window `[ws − R, ws)` is exactly the range `oldest_retained`
  computes in [cursor-delivery.md](../algorithms/runtime/cursor-delivery.md).
- Nightly sweep command: `cargo test --workspace -- --ignored stress_ring_depth_sweep`
  (see [.github/workflows/fuzz-nightly.yml](../../.github/workflows/fuzz-nightly.yml)).
