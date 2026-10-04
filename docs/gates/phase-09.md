# Phase 9 Gate — Structured Local Channels

Evidence mapping, [work-plan.md](../../work-plan.md) #9 criteria ⇄ tests.
Implementation: `drm-core::channel` publish/read paths
([channel-publish.md](../algorithms/runtime/channel-publish.md),
[cursor-delivery.md](../algorithms/runtime/cursor-delivery.md)).
Verdict: **PASS** (local release-build run; harness is cross-platform).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| One publisher + many readers | `stress_structured_billions.rs`: one publisher per depth (1/3/16) + 4 copy readers over the live `Channel` | PASS |
| Billions of publish/read cycles | 3 × 10⁹ cycles (1 × 10⁹ per depth), local release build, `published=3000000000` | PASS |
| 0 torn samples | patterned-payload probe blocks; `torn=0` | PASS |
| 0 mixed generations | generation probe blocks; `mixed=0` | PASS |
| 0 invalid payloads | derived-fill check per read; `invalid=0`; fault injection proves the detector fires with the exact class (`pattern_corruption`) | PASS |
| Monotonically valid sequence behavior | sequence checked on every read; `monotonic=0` violations | PASS |

## Variables (M03W3)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 9)` | PASS | PASS |
| `RING_DEPTHS_GREEN` | R ∈ 1..=64 exhaustive; sampled 65..4096 | `retention_depths_1_to_64`, `ring_depths_property` (65/100/333/1000/4096) |
| `RETENTION_EXACT` | exactly R addressable (I1) | `retention_exact`, `ring_depths_property`, `wraparound` |
| `PRIORITY_ORDER` | class order exact | `priority_order`, `priority_fifo_and_empty` |
| `CURSOR_STRUCT` | `ReaderState`/`ReadOutcome` + empty-channel tests | `drm-core::channel` `ReaderState`/`ReadOutcome`; empty-channel path in `publish_read_roundtrip`/`retention_exact` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Integrity scheme

Patterned payload (`drm-core::channel::pattern`): self-describing
`{sequence, slot_echo, generation}` plus repeated probe blocks and derived fill
bytes. Classification is exact per anomaly class without a software CRC
(architecture §Local Integrity). Negative proof: `pattern_corruption` flips one
byte and asserts `Invalid`; blended halves assert `Torn`/`MixedGeneration`.

## Fast-path audit

- Alloc: `cargo test -p drm-core --features test-count-allocs --test alloc_counter` ⇒ 0 alloc / 0 dealloc on the steady codec block.
- Syscall: `strace`-filtered audit is Linux-only; deferred to the Linux lane (no syscall on the copy publish/read path beyond the activation-time setup by construction).
- Publishing under the harness performed zero pool drops (`dropped=0` across 3 × 10⁹), so the slot/hazard path never fell back.

## Requirements covered

REQ-CORE-050/051/052/053 (publish transaction/linearization/zero-alloc),
REQ-CORE-061–067 (ring sequence/history/wrap/order/watermark), REQ-CORE-082
(zero-allocation codec path), REQ-QAL-006 (impl + evidence-classed test).
Priority seed: REQ-ROUTE-020/021.

## Notes

- The billion-cycle evidence run is repeatable:
  `$env:DRM_STRESS_CYCLES=1000000000; cargo test --release -p drm-core --test stress_structured_billions -- --ignored --nocapture` (CI nightly uses 1 × 10⁸ via [fuzz-nightly.yml](../../.github/workflows/fuzz-nightly.yml)).
- Phase 10 (rings) and Phase 11 (cursors) gates follow; depth/window evidence
  above feeds both.