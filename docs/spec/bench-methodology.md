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
| `drm-core` | `benches/latency.rs` | `publish`, `copy_latest`, `copy_next`, write-loan borrow round trip, exhaustion/backpressure (`QueueFull`) publish |
| `drm-core` | `benches/throughput.rs` | sequence-ring commit + lookup, payload-pool slot write + offset |
| `drm-core` | `benches/codec.rs` | canon-codec primitive read/write, two-region `body_range`, `validate_sample` |
| `drm-router` | `benches/router.rs` | peer-lane loopback p50/p99, token-bucket admission, priority enqueue/pop |
| `drm-router` | `benches/transport.rs` | UDP datagram loopback, serial COBS/CRC32C frame encode + decode |

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
  ring commit + lookup / lane send + service / codec round trip / serial
  encode + decode block must report zero allocator entries and zero
  deallocations (I-P/I-4 style; architecture §Zero Data-Path Allocation). The
  UDP loopback is syscall-bound and carries only the bounded-time invariant.
- **Bounded time** (the in-bench `measure`/`percentile` helpers): the p99
  latency of the steady-state round trip must stay under a declared budget — `50
  µs` for the core publish + read / ring paths, `50 µs` for the exhaustion
  (`QueueFull`) publish path, `5 µs` for a canon-codec round trip, `20 µs` for
  the router lane loopback and the serial frame codec, and `500 µs` for the UDP
  loopback. A violation aborts the bench with a non-zero exit.
- **Budget fit** (`publish_queue_full`): the exhaustion-path channel's
  worst-case footprint (`pool_slots × slot_bytes + control_block`,
  [memory-budget.md](../algorithms/memory-budget.md) I-MB1) is checked against
  the declared bench budget at compile time.

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
taskset -c 1 cargo bench -p drm-core --bench codec
taskset -c 1 cargo bench -p drm-router --bench router
taskset -c 1 cargo bench -p drm-router --bench transport
```

## System perf matrix

`cargo xtask perf-matrix [--workspace <dir>] [--json <path>] [--quick]` measures
the declared payload classes of [tests/perf/workspace](../../tests/perf/README.md)
(64 B fixed, 1 MiB fixed, 32 B `reject`, 4 KiB-bounded variable, 128 B deep
history) through the production `drm_core::channel::Channel`. Each topic's
capacity comes from the codegen output (`SAMPLE_MAX`) and its attributes from the
`.drm` source, so the matrix is driven by the fixtures. It also records the
cross-process segment lifecycle on one segment: creator process, a separate
consumer process (`consumer_open`, open-only), and a refused second publisher
(I-L1).

- Budgets: the p99 ceilings of §In-bench invariants apply unchanged (50 µs for
  the core publish and read paths). Any breach sets `all_pass` false.
- Baseline bands: compared with [bench-baseline.md](bench-baseline.md) (M23W4
  `publish+latest` p50 190 ns / p99 200 ns) within ±20 % / ±30 %. Only topics at
  or below 4 KiB are compared — the recorded baseline is a 64-byte sample, so
  larger payload classes are reported `n/a`. A slower move outside the band is a
  regression (`SLOWER`); a faster move is reported `faster`, never as drift.
- Percentiles use the floor index `(n-1)*pct/100`, matching
  `crates/drm-core/benches/common/mod.rs`, so the values are comparable with the
  archived baselines. Per-operation `Duration` resolution (~40 ns on this host)
  bounds the smallest reportable latency.
- `@overflow: reject` topics: a `reject` channel whose pool is not retired by a
  consumer refuses publishes by design, so the sustained rate reads 0; the
  refusal-path cost is what `publish_queue_full` measures.
- Not covered: sample movement *between* processes. The node-facing C ABI is a
  single in-process channel (`crates/drm-ffi/src/lib.rs` module docs), and no
  runtime API publishes *through* a mapped segment, so cross-process evidence
  stops at segment create/adopt/authority.

REQ: REQ-QAL-011, REQ-QAL-012, REQ-CORE-003, REQ-CORE-012, REQ-CORE-013,
REQ-CORE-097, REQ-ROUTE-012, REQ-ROUTE-013, REQ-ROUTE-020, REQ-ROUTE-030.
