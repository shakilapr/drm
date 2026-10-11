# Benchmark Methodology

Normative terminology follows [docs/standards.md](../standards.md). Scope: the
fixed methodology for the runtime/router benchmark harness, the in-bench
invariant assertions, the CI noise bands, and the archived baseline. Implements
[qualification.md](../algorithms/program/qualification.md) (REQ-QAL-006) and the
performance baselines for [cursor-delivery.md](../algorithms/runtime/cursor-delivery.md)
(REQ-CORE-003/012/013/097) and [lanes-shaping.md](../algorithms/router/lanes-shaping.md)
(REQ-ROUTE-012/013/020/030). Owner: E1 (latency/throughput) + E2 (invariant
assertions). Phase 18 hardening.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-QAL-bench-baseline` | publish/read latency, borrow throughput, router p50/p99 recorded under a fixed methodology | `REQ-QAL-011` |
| `REQ-QAL-bench-invariants` | zero-allocation + bounded-time invariants asserted inside benches under load | `REQ-QAL-012` |

## Harness

| Crate | Bench | What it measures |
|---|---|---|
| `drm-core` | `benches/latency.rs` | `publish`, `copy_latest`, `copy_next`, write-loan borrow round trip |
| `drm-core` | `benches/throughput.rs` | sequence-ring commit + lookup, payload-pool slot write + offset |
| `drm-router` | `benches/router.rs` | peer-lane loopback p50/p99, token-bucket admission, priority enqueue/pop |

The harness is `criterion` (`harness = false`). The runner pins the CPU
(`taskset -c 1 cargo bench`) so numbers are comparable across runs; the CI
runner pins the CPU, and the recorded baseline was taken on a quiescent host. The in-bench percentile helper
(`benches/common/mod.rs`) uses nearest-rank percentiles over a fixed repetition
count and is independent of criterion's statistical analysis.

### Fixed policy

1. Warm-up before every measured block; no lazy first-touch inside the loop.
2. Fixed sample size and measurement time per bench (default criterion:
   `sample_size = 100`, `measurement_time = 5 s`, `warm_up_time = 3 s`).
3. Single-threaded; no concurrent background allocator activity during the
   invariant blocks.
4. Numbers are recorded on a quiescent host and re-run on the pinned CI
   runner; the CI baseline job fails on any invariant violation.

## In-bench invariants

The benches assert architecture invariants under load, not only in unit scope
(REQ-QAL-012):

- **Zero allocation** (`assert_zero_alloc`): a counting `GlobalAlloc` local to
  each bench binary totals allocator entries; a steady-state publish + read /
  ring commit + lookup / lane send + service block must report zero allocator
  entries and zero deallocations (I-P/I-4 style; architecture §Zero Data-Path
  Allocation).
- **Bounded time** (the in-bench `measure`/`percentile` helpers): the p99
  latency of the steady-state round trip must stay under a declared budget —
  `50 µs` for the core publish + read / ring paths and `20 µs` for the router
  lane loopback. A violation aborts the bench with a non-zero exit.

## Noise bands

Run-to-run variance on the pinned runner is expected; a regression is flagged
when a median moves outside the band:

| Metric | Band | Disposition |
|---|---|---|
| p50 latency | ±20 % run-to-run | within band = noise |
| p99 latency | ±30 % run-to-run | within band = noise |
| invariant budgets | hard ceiling (no band) | any violation fails the bench |

Sustained drift outside the band is carried to the next hardening week
(E1/E2) as a benchmark-regression item.

## Baseline (M18W2, x86-64)

Archived as the regression reference in
[bench-baseline.md](bench-baseline.md). Values are criterion medians unless
marked p50/p99 (nearest-rank, in-bench).

## Reproduce

```text
taskset -c 1 cargo bench -p drm-core --bench latency
taskset -c 1 cargo bench -p drm-core --bench throughput
taskset -c 1 cargo bench -p drm-router --bench router
```

REQ: REQ-QAL-011, REQ-QAL-012, REQ-CORE-003, REQ-CORE-012, REQ-CORE-013,
REQ-CORE-097, REQ-ROUTE-012, REQ-ROUTE-013, REQ-ROUTE-020, REQ-ROUTE-030.
