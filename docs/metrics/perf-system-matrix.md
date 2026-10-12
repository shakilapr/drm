# System perf matrix — measured

Measured 2026-10-12 with `cargo xtask perf-matrix` (release xtask) on
`linux x86_64 (6.8.0-146-generic)`, unpinned CPU, quiescent host. Methodology and
budgets: [bench-methodology.md](../spec/bench-methodology.md) §System perf matrix.
Fixtures: [tests/perf/workspace](../../tests/perf/README.md).

Reproduce:

```text
cargo xtask perf-matrix --json target/perf-matrix.json
cargo xtask perf-matrix --workspace tests/ros2_compat/translated --json target/ros2-perf-matrix.json
```

## In-process data plane (`tests/perf/workspace`)

Percentiles are floor-indexed p50/p99 of per-operation latencies (ns); `seg B` is
the deployed segment footprint from `segment_layout`. `ops/s` and `MiB/s` are the
sustained publish rate with one live reader.

| Topic | Bytes | R | pool | pub p50 | pub p99 | rd p50 | rd p99 | ops/s | MiB/s | seg B |
|---|---|---|---|---|---|---|---|---|---|---|
| `control/command` | 32 | 1 | 2 | 40 | 40 | 40 | 49 | 0 ¹ | 0.0 | 424 |
| `control/pose` | 128 | 16 | 20 | 170 | 191 | 60 | 61 | 6,864,012 | 837.9 | 4,024 |
| `control/telemetry` | 4,116 | 4 | 6 | 150 | 180 | 90 | 100 | 8,651,641 | 33,960.5 | 25,312 |
| `sensor/image` | 1,048,584 | 1 | 2 | 25,480 | 37,810 | 25,540 | 38,431 | 35,127 | 35,127.8 | 2,097,528 |
| `sensor/imu` | 64 | 8 | 12 | 120 | 131 | 60 | 60 | 10,914,484 | 666.2 | 1,656 |

¹ `@overflow: reject` with a pool no consumer retires: publishes are refused by
design once the pool is full, so the sustained rate reads 0 (the refusal cost is
what `publish_latest`/`publish_queue_full` cover in `benches/latency.rs`).

## Budgets

All ten verdicts PASS (`all_pass: true`): every publish and read p99 is at or
below the 50 µs core ceiling. The largest is `sensor/image` publish p99
37,810 ns (75.6 % of budget); the smallest is `control/command` publish p99
40 ns.

## Baseline bands (M23W4: p50 190 ns / p99 200 ns)

| Topic | pub p50 | pub p99 |
|---|---|---|
| `control/command` | faster | faster |
| `control/pose` | within-band | within-band |
| `control/telemetry` | n/a (4,116 B > 4 KiB baseline class) | — |
| `sensor/image` | n/a (1,048,584 B > 4 KiB baseline class) | — |
| `sensor/imu` | faster | faster |

No `SLOWER` verdict. The 128 B `control/pose` channel lands within band; the
64 B `sensor/imu` channel is faster than the archived baseline (different
harness: per-op timing here vs criterion medians there).

## Cross-process segment lifecycle

One segment, two processes (`consumer_open` in a spawned child, creator in the
parent), plus a refused second publisher:

| Fact | Value |
|---|---|
| Segment | 424 B |
| `publisher_open` | 96,743 ns |
| Publisher epoch | 1 |
| `consumer_open` (separate process) | 24,320 ns |
| Consumer attached | true |
| Second publisher refused (I-L1) | true |

## Limitations (recorded, not hidden)

- **No cross-process sample movement is measured** — because none exists yet. The
  node-facing C ABI serves a single in-process channel
  (`crates/drm-ffi/src/lib.rs` module docs: "deliberately NOT the cross-process
  shared-memory endpoint … Cross-process SHM endpoints are a later phase"), and
  the only channel constructor is `Channel::activate`, which allocates its arena
  on the heap (`crates/drm-core/src/channel/mod.rs:101`). The cross-process
  plumbing that does exist (`publisher_open` / `consumer_open` over `PosixShm`)
  creates, adopts, validates and governs authority, but no runtime API publishes
  *through* a mapped segment. Multi-process nodes therefore cannot yet exchange
  samples; the matrix measures the data plane in-process and the segment
  lifecycle across processes.
- Per-operation `Duration` resolution on this host is ~40 ns, so sub-40 ns
  latency differences are not resolvable by this harness.
- Unpinned CPU: compare against the pinned criterion baselines only via the
  noise bands, not absolutely.
