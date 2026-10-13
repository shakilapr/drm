# Results & Benchmarks

Consolidated test, gate, benchmark and integration results for the DRM workspace.
Every number here was produced by a command in this section on the host below; nothing
is carried over from an earlier session. Raw artifacts are listed in
[Artifacts](#artifacts).

| Item | Value |
|---|---|
| Host | Linux x86_64, kernel 6.8.0-146-generic |
| Rust | workspace toolchain (edition 2024) |
| Date | 2026-10-12/13 |
| Bench pinning | `taskset -c 1` (per `docs/spec/bench-methodology.md`); matrix runs unpinned |
| Commit under test | `2181d72` |

## 1. Gate suite

| Gate | Command | Result |
|---|---|---|
| Rust formatting | `cargo fmt --all -- --check` | **PASS** (exit 0) |
| Multi-language formatting | `cargo xtask fmt --check` | **PASS** (exit 0) — rustfmt + clang-format + prettier |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** (0 errors, 0 warnings) |
| Tests | `cargo xtask test` | **947 passed / 0 failed / 25 ignored** (119 targets) |
| Layering | `cargo xtask check-layering` | **OK** (14 workspace crates) |
| Traceability | `cargo xtask audit-traceability` | registry 376 rows, **0 findings**; citations **280/280** `.rs` files |
| Qualification packet | `cargo xtask qualify --check` | **PASS** — `CRIT_COVERAGE = 6/6`, `API_INVENTORY = 52/52` (45 freeze candidates, 7 deferred) |

Critical-path coverage from the qualification packet: `ring-history`, `overflow-policies`,
`hazard-protocol`, `exhaustion-reclamation` (2 test files), `transport-udp`,
`transport-serial` — 6/6 resolved.

## 2. Test suite detail

947 tests over 119 targets. Largest contributors:

| Test target | Passed | Failed | Ignored |
|---|---|---|---|
| `drm_core` (lib + integration) | 135 | 0 | 1 |
| `xtask` | 86 | 0 | 0 |
| `drm_spatial` | 73 | 0 | 0 |
| `drm_router` | 55 | 0 | 0 |
| `drm_cli` | 48 | 0 | 0 |
| `drm_workspace` | 40 | 0 | 0 |
| `budget` | 25 | 0 | 0 |
| `topology_graph` | 19 | 0 | 0 |
| `drm_record` | 18 | 0 | 0 |
| `types_codec` | 13 | 0 | 0 |
| `verify_engine` | 13 | 0 | 0 |
| `ffi_negative` | 12 | 0 | 0 |
| `drm_rpc` | 12 | 0 | 0 |
| `codegen_all` | 12 | 0 | 0 |
| … 105 further targets ≤ 11 tests each | 386 | 0 | 24 |

Ignored (25) are the deliberately-gated long/host-dependent suites: `shm_kill` (9),
`stress` (5), `no_fd_leak` (2), `quota_edge` (2), `link_storm` (2), four single cases
(`canon_read_fuzz`, `codegen_invariants`, `fuzz_campaign`, `stress_structured_billions`),
and one in `drm_core`. They are not counted as passes.

### Codegen gates (the ones that matter for this tranche)

| Test | Purpose | Result |
|---|---|---|
| `codegen_all::gen_rust_topic_compiles_and_decodes_golden` | fixed topic compiles + golden parity | 12 passed |
| `codegen_all::gen_rust_bounded_topic_compiles_and_round_trips` | **new** — bounded `string[64]`/`bytes[<=256]`/`u8[64]` topic compiles + round-trips | PASS |
| `codegen_c` (C/C++/Python compile + round-trip) | 4 languages | 9 passed |
| `codegen_invariants` | determinism, fingerprint invalidation, hand-edit detection | 8 passed, 1 ignored |

## 3. Criterion benchmarks (fresh run)

`taskset -c 1 cargo bench -p drm-core -p drm-router --bench latency --bench throughput --bench codec --bench router --bench transport -- --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10`

22 measurements, all green (in-bench zero-alloc and bounded-time invariant assertions included).
Baseline column is the archived reference in `docs/spec/bench-baseline.md`
(M18W2 medians, M23W4 invariant p99 where marked).

| Benchmark | Baseline | Lower | Median | Upper | vs baseline |
|---|---|---|---|---|---|
| `publish_copy` | 150 ns | 148.78 ns | 149.16 ns | 149.58 ns | within band |
| `copy_latest` | 23 ns | 23.254 ns | 23.283 ns | 23.306 ns | within band |
| `copy_next` | 8 ns | 7.583 ns | 7.593 ns | 7.603 ns | within band |
| `borrow_loan_roundtrip` | 172 ns | 170.77 ns | 171.43 ns | 172.38 ns | within band |
| `publish_queue_full` | 40 ns (p99) | 8.128 ns | 8.185 ns | 8.250 ns | faster |
| `invariant_publish_latest` | 190 ns (p50) | 174.99 ns | 175.14 ns | 175.31 ns | faster |
| `ring_commit` | 14 ns | 13.395 ns | 13.774 ns | 13.942 ns | within band |
| `ring_lookup` | 4 ns | 4.073 ns | 4.127 ns | 4.259 ns | within band |
| `pool_write` | 9 ns | 8.719 ns | 8.759 ns | 8.841 ns | within band |
| `pool_offset` | 4 ns | 3.880 ns | 3.920 ns | 3.987 ns | within band |
| `invariant_ring` | 50 ns (p99) | 13.421 ns | 13.753 ns | 13.916 ns | faster |
| `codec_u32_roundtrip` | — | 3.138 ns | 3.149 ns | 3.161 ns | — |
| `codec_u64_roundtrip` | — | 4.335 ns | 4.358 ns | 4.375 ns | — |
| `codec_body_range` | — | 4.976 ns | 4.997 ns | 5.020 ns | — |
| `codec_validate_sample` | — | 4.730 ns | 4.736 ns | 4.752 ns | — |
| `invariant_codec_roundtrip` | 40 ns (p99) | 3.956 ns | 3.962 ns | 3.968 ns | faster |
| `lane_loopback` | 11 ns | 11.432 ns | 11.445 ns | 11.460 ns | within band |
| `invariant_lane_loopback` | 40 ns (p99) | 2.733 ns | 2.743 ns | 2.754 ns | faster |
| `token_bucket_try_consume` | 5 ns | 4.609 ns | 4.629 ns | 4.652 ns | within band |
| `priority_enqueue_pop` | 6 ns | 6.032 ns | 6.049 ns | 6.075 ns | within band |
| `serial_frame_roundtrip` | 1.19 µs (p99) | 1.1632 µs | 1.1658 µs | 1.1699 µs | within band |
| `udp_loopback` | 3.04 µs (p99) | 2.1090 µs | 2.1141 µs | 2.1204 µs | faster |

Caveat: the p99-marked baselines are compared against criterion medians, so those rows
overstate the margin. Median-vs-median rows (`publish_copy`, `copy_latest`, `copy_next`,
`borrow_loan_roundtrip`, `ring_*`, `pool_*`, `lane_loopback`, `token_bucket_*`,
`priority_enqueue_pop`) are directly comparable and all sit inside the ±20 % band.

## 4. System performance matrix (`cargo xtask perf-matrix`)

Fixture workspace `tests/perf/workspace` — 5 test packages/nodes, measured through the
production `drm_core::channel::Channel`. Percentiles are floor-indexed p50/p99 of
per-operation latencies (ns); `ops/s` and `MiB/s` are sustained publish with one live
reader; `seg B` is the deployed segment footprint from `segment_layout`.

| Topic | Bytes | R | pool | policy | pub p50 | pub p99 | rd p50 | rd p99 | borrow p50/p99 | ops/s | MiB/s | seg B |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `control/command` | 32 | 1 | 2 | reject | 40 | 40 | 40 | 40 | 40 / 40 | 0 ¹ | 0.0 | 424 |
| `control/pose` | 128 | 16 | 20 | overwrite | 170 | 200 | 60 | 80 | 180 / 220 | 6,862,658 | 837.7 | 4,024 |
| `control/telemetry` | 4,116 | 4 | 6 | overwrite | 150 | 171 | 90 | 100 | 170 / 191 | 8,583,945 | 33,694.8 | 25,312 |
| `sensor/image` | 1,048,584 | 1 | 2 | overwrite | 26,950 | 39,051 | 25,129 | 37,431 | 26,781 / 37,941 | 35,656 | 35,657.0 | 2,097,528 |
| `sensor/imu` | 64 | 8 | 12 | overwrite | 120 | 140 | 60 | 60 | 131 / 151 | 10,591,089 | 646.4 | 1,656 |

¹ `@overflow: reject` with a pool no consumer retires: publishes are refused by design once
the pool is full, so the sustained rate reads 0 (the refusal cost is `publish_queue_full`).

**Budgets — 10/10 PASS** (ceiling 50,000 ns on publish and read p99). Tightest:
`sensor/image` publish p99 **39,051 ns = 78.1 % of budget**. Smallest: 40 ns.

**Baseline bands** (`bench-baseline.md` M23W4: p50 190 ns / p99 200 ns, applied only to
payloads ≤ 4 KiB): `control/pose` within-band; `control/imu`/`command` faster;
`control/telemetry` and `sensor/image` `n/a` (baseline class is 64 B). No `SLOWER` verdict.

**Cross-process segment lifecycle** (one segment, creator process + separate consumer
process + refused second publisher, I-L1):

| Fact | Value |
|---|---|
| Segment | 424 B |
| `publisher_open` | 90,771 ns |
| Publisher epoch | 1 |
| `consumer_open` (child process) | 26,420 ns |
| Consumer attached | true |
| Second publisher refused | true |

## 5. ROS 2 integration

Reference set `references/ros2/` (gitignored; `rolling` ref, fetched from
raw.githubusercontent.com): **66 message definitions** across `std_msgs`,
`geometry_msgs`, `sensor_msgs`, `builtin_interfaces`; 4 C++ node sources
(`demo_nodes_cpp` talker/listener + rclcpp minimal pub/sub); 2 Python node sources.

### 5.1 Translation verdicts (66 messages)

| Verdict | Count | Meaning |
|---|---|---|
| clean | 28 | 1:1 after flattening nested structs to dotted prims |
| needs-bounds | 22 | expressible only after choosing a bound for an unbounded ROS string/sequence |
| cannot-map | 16 | arrays of structs, or array depth > 1 — inexpressible in DRM |

Per-message table: `references/ros2/TRANSLATION.md`.

### 5.2 Build integration

| Fixture | Contents | `drm build` |
|---|---|---|
| `tests/ros2_compat/translated` | adapted translation (bounds chosen, nested messages flattened): 13 topics, 3 nodes | **PASS** — `64 generated (64 rendered), 13 verified, 3 built [chatter_src, listener, talker]` |
| `tests/ros2_compat/mechanical` | naive 1:1 translation | **REJECTED** — `workspace model invalid: topic sensor_msgs/joint_state: 1 defect(s); topic sensor_msgs/laser_scan: 1 defect(s); topic std_msgs/header: 3 defect(s); topic std_msgs/string: 1 defect(s)` |

Conclusion: not by simple translation, but yes after an explicit, mechanical adaptation
step. Every string and sequence needs a bound; `JointState.name` (array of strings) needs a
structure DRM forbids outright.

### 5.3 Translated topics running in DRM (13 topics)

| Topic | Bytes | pub p50 | pub p99 | rd p99 | ops/s | MiB/s | seg B |
|---|---|---|---|---|---|---|---|
| `std_msgs/bool` | 1 | 60 | 71 | 60 | 27,594,646 | 26.3 | 376 |
| `std_msgs/int32` | 4 | 60 | 71 | 60 | 27,589,570 | 105.2 | 376 |
| `std_msgs/float64` | 8 | 60 | 71 | 60 | 28,599,763 | 218.2 | 376 |
| `geometry_msgs/twist` | 48 | 60 | 80 | 70 | 27,251,435 | 1,247.5 | 576 |
| `geometry_msgs/pose` | 56 | 60 | 71 | 60 | 26,247,289 | 1,401.8 | 608 |
| `std_msgs/header` | 76 | 60 | 80 | 70 | 25,892,121 | 1,876.6 | 696 |
| `geometry_msgs/pose_stamped` | 132 | 69 | 71 | 60 | 25,822,301 | 3,250.6 | 936 |
| `geometry_msgs/transform_stamped` | 208 | 69 | 71 | 60 | 25,959,572 | 5,149.5 | 1,224 |
| `std_msgs/string` | 260 | 70 | 90 | 70 | 25,144,126 | 6,234.6 | 1,432 |
| `demos/chatter` | 260 | 70 | 89 | 60 | 24,839,007 | 6,159.0 | 1,432 |
| `sensor_msgs/imu` | 372 | 130 | 140 | 80 | 10,261,311 | 3,640.4 | 5,400 |
| `sensor_msgs/laser_scan` | 16,508 | 320 | 340 | 350 | 3,419,903 | 53,840.4 | 49,912 |
| `sensor_msgs/compressed_image` | 1,048,704 | 22,830 | 35,711 | 35,271 | 40,123 | 40,128.2 | 2,097,784 |

**Budgets — 26/26 PASS.** Tightest: `compressed_image` publish p99 35,711 ns (71.4 %).
Cross-process: one 1,432 B segment, `publisher_open` 58,712 ns, epoch 1,
`consumer_open` (separate process) 15,281 ns, second publisher refused.

### 5.4 Fake-input round-trip over the generated codec

`tests/ros2_compat/roundtrip/main.rs` includes the DRM-generated Rust module for each
translated topic and round-trips synthetic values through
`encode → bytes → decode`, asserting field equality:

```
std_msgs/Bool                      roundtrip OK
std_msgs/Int32                     roundtrip OK
std_msgs/String (bounded 256)      roundtrip OK
std_msgs/Header                    roundtrip OK
geometry_msgs/Twist                roundtrip OK
geometry_msgs/Pose                 roundtrip OK
geometry_msgs/PoseStamped          roundtrip OK
geometry_msgs/TransformStamped     roundtrip OK
sensor_msgs/Imu                    roundtrip OK
sensor_msgs/LaserScan (4 beams)    roundtrip OK
sensor_msgs/CompressedImage        roundtrip OK
demos/chatter (talker)             roundtrip OK
ROS2_ROUNDTRIP pass=12 fail=0
```

Fake inputs used: `"Hello World: 42"` for bounded strings, `base_link`/`map`/`laser` for
frames, a 4-beam scan (`ranges [1.0, 2.5, 3.0, 0.5]`, `intensities [10, 20, 30, 40]`), a
fake JPEG header (`FF D8 FF E0 …`), IMU `linear_acceleration_z = 9.81`,
`orientation_covariance[0] = -1.0`.

## 6. Defects found and their state

| # | Defect | Severity | State |
|---|---|---|---|
| 1 | Generated Rust did not compile for any bounded string/array > 32 B (`[u8; 256]: Default` unsatisfied) — most string-bearing topics | build-breaking for users | **Fixed** `b07d072`, gated by `gen_rust_bounded_topic_compiles_and_round_trips` |
| 2 | Every codegen compile test used `a: u8, b: u64` (no array field), so (1) was invisible | gate blindness | **Fixed** `b07d072` |
| 3 | Two node directories with the same name silently dropped a node, then failed with a misleading `V-SW-1 … no publisher` | silent data/interface loss | **Fixed** `b04dc71`, gated by `nodes_duplicate_directory_rejected` + end-to-end build |
| 4 | `cargo xtask fmt --check` red with prettier installed; write mode would have rewritten all 40 byte-exact golden JSON vectors and reflowed the markdown tree | could corrupt goldens | **Fixed** `aa72ff2` (`.prettierignore`) |
| 5 | Docs named generated artifacts `…/mod.rs` and `cpp/types/…`; renderer writes `…/target.rs`, `…/<pkg>/<topic>/target.hpp` | doc drift | **Fixed** `b07d072` |
| 6 | Generated TS union arm violated the project's own prettier gate | gate red | **Fixed** `aa72ff2` |
| 7 | A renderer change does not invalidate existing `.drm/gen` trees (`64 generated (0 rendered)`); stale broken modules persist until the tree is deleted | stale artifacts | **Open** — `gen-check` catches it, a plain `drm build` does not |
| 8 | Cross-process sample movement is not implemented: the node-facing C ABI serves a single in-process channel (`crates/drm-ffi/src/lib.rs`), and no API publishes through a mapped segment | capability gap | **Open** (by design, "later phase") |

## 7. Limitations of these numbers

- **No cross-process sample movement is measured** — see defect 8. Cross-process evidence
  covers segment create/adopt/validation/authority, not payload exchange.
- Per-operation `Duration` resolution on this host is ~40 ns, so the 40/60 ns figures in
  the matrices are at the clock floor; sub-40 ns differences are not resolvable there.
- The matrix runs are unpinned; only the criterion benches were pinned (`taskset -c 1`).
- No ROS 2 runtime is installed here, so the ROS 2 side is source-level plus DRM's own
  validator/runtime — nothing was executed against `rclcpp`/`rclpy`.
- 25 tests are ignored by design (long/host-dependent suites); they are not counted as
  passes.
- The ROS 2 reference set is `rolling` at fetch time; message definitions drift upstream.

## 8. Reproduce

```text
# gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
cargo xtask qualify --check --json target/qualification-packet.json

# criterion benchmarks (pinned, short methodology)
taskset -c 1 cargo bench -p drm-core -p drm-router --bench latency --bench throughput \
  --bench codec --bench router --bench transport -- \
  --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10

# system performance matrix
cargo xtask perf-matrix --json target/perf-matrix.json
cargo xtask perf-matrix --workspace tests/ros2_compat/translated --json target/ros2-perf-matrix.json

# ROS 2 integration
cargo run -p drm-cli -- --workspace tests/ros2_compat/translated build     # PASS
cargo run -p drm-cli -- --workspace tests/ros2_compat/mechanical build     # expected FAIL
rustc --edition 2021 -O tests/ros2_compat/roundtrip/main.rs -o /tmp/rt && /tmp/rt
```

## Artifacts

| Path | Contents |
|---|---|
| `target/perf-matrix.json` | fixture-workspace matrix (topics, budgets, cross-process) |
| `target/ros2-perf-matrix.json` | translated-ROS 2 matrix |
| `target/qualification-packet.json` | qualification packet |
| `docs/spec/bench-methodology.md` | methodology, budgets, noise bands, §System perf matrix |
| `docs/spec/bench-baseline.md` | archived baselines used for comparison |
| `docs/metrics/perf-system-matrix.md` | perf matrix write-up |
| `docs/metrics/ros2-compat-matrix.md` | ROS 2 compatibility write-up + findings |
| `tests/perf/` | perf fixture workspace (packages, nodes) |
| `tests/ros2_compat/` | translated + mechanical fixtures, round-trip harness |
| `references/ros2/` | fetched ROS 2 sources, inventory, translation + node mapping |

Commits in this tranche: `e3fb2b7` (fixtures), `aedda6a` (perf-matrix runner),
`a095467` (results docs), `b07d072` (codegen `Default` + gate), `b04dc71` (duplicate node
directory), `aa72ff2` (prettier scope + generated TS), `2181d72` (match coverage).
