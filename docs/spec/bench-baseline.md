# Benchmark Baseline

Regression reference for the runtime/router benchmark harness
([bench-methodology.md](bench-methodology.md)). Numbers are criterion medians
unless marked p50/p99 (nearest-rank, in-bench), recorded on a quiescent x86-64
host (`cargo bench`, bench profile, optimized; the CI runner pins the CPU).
Re-recorded each
hardening month; a sustained move outside the noise bands is a benchmark
regression.

REQ: REQ-QAL-011, REQ-QAL-012.

## M18W2

### `drm-core` — `benches/latency.rs`

| Bench | Metric | Baseline |
|---|---|---|
| `publish_copy` | median | 150 ns |
| `copy_latest` | median | 23 ns |
| `copy_next` | median | 8 ns |
| `borrow_loan_roundtrip` | median | 172 ns |
| `invariant_publish_latest` | p50 / p99 | 190 ns / 200–460 ns |

### `drm-core` — `benches/throughput.rs`

| Bench | Metric | Baseline |
|---|---|---|
| `ring_commit` | median | 14 ns |
| `ring_lookup` | median | 4 ns |
| `pool_write` | median | 9 ns |
| `pool_offset` | median | 4 ns |
| `invariant_ring` | p99 | 50 ns |

### `drm-router` — `benches/router.rs`

| Bench | Metric | Baseline |
|---|---|---|
| `lane_loopback` | median | 11 ns |
| `lane_loopback` | p50 / p99 | 31 ns / 40 ns |
| `token_bucket_try_consume` | median | 5 ns |
| `priority_enqueue_pop` | median | 6 ns |

### Stability

Five consecutive pinned runs (`--warm-up-time 0.1 --measurement-time 0.2
--sample-size 10`) held within the bands: core publish+latest p50 180–190 ns,
ring commit+lookup p99 50–51 ns, router lane loopback p50 30–31 ns / p99 40 ns.
