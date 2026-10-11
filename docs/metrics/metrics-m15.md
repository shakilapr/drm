# Metrics — Month 15 (M15)

Normative source: [work-plan.md](../../work-plan.md) W6 operational tooling.
Phase 32 opens with wave A (introspection, trace, diag, MCU start). One section
per weekly tranche.

## M15W1 — Introspection Tools (echo/hz/log), Diag Daemon Start

| Variable | Target | Measured |
|---|---|---|
| `ECHO_GREEN` | `drm echo` read-only, momentary borrow+copy (I-OB5) | 1 — `drm-cli` `echo_copies_and_reports_empty` copies the newest sample and leaves the diagnostic hazard word `HAZARD_NONE`; `drm-diag` `introspect_no_pin` runs 10 000 `copy_latest` reads with zero pins and the pool still exhausts/recover |
| `HZ_GREEN` | `drm hz` observed rates match injected knowns (I-OB3) | 1 — `drm-diag` `diag_observed_rate` matches 10/100/1000 Hz injected sequences within 1%; `drm-cli` `hz_tolerance` reports a 100 Hz schedule within 1% |
| `DIAG_SKELETON` | out-of-band attach + counter read green (I-OB2) | 1 — `diag_sample` epoch sandwich (`diag_epoch_sandwich`), no data-path mutation (`diag_no_overhead`), deadline matrix (`diag_deadline_matrix`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Trace runtime** (`crates/drm-core/src/trace/mod.rs`): wait-free per-producer
  SPSC lanes (`TraceRingSet`/`TraceLane`/`TraceEvent`), `trace_emit`,
  `trace_register_thread`; drop counters (`I-OB6`). Tests
  `trace_never_blocks` (1/2/4/16 producers, stopped reader; exact drop counts,
  bounded tail cost), `trace_lane_exhaustion`, `trace_drop_counter`.
- **Trace dictionary**: append-only `TraceDict` with
  `dict_version = (format_version, BLAKE3(dict bytes)[..16])`; unknown id renders
  raw hex + version-mismatch marker. Test `trace_dict_roundtrip`.
- **Diagnostics** (`crates/drm-diag`, ops): `ChannelStats`, `ConsumerStats`,
  `DeadlineState`, `LinkStats`, `diag_sample` (epoch sandwich), `deadline_tick`,
  `RateWindow`, `Observer`, `LogCatalog`. Tests `diag_no_overhead`,
  `diag_epoch_sandwich`, `diag_observed_rate`, `diag_deadline_matrix`,
  `introspect_no_pin`, `observer_copies_owned_bytes`, `log_level_filter`,
  `fresh_channel_stats`.
- **Introspection tools** (`drm echo`/`hz`/`log`): read-only subcommands over
  `drm-diag`, resolving deployment metadata and attaching as non-publishing
  observers. Tests `render_modes`, `echo_copies_and_reports_empty`,
  `hz_tolerance`, `render_and_filter`, `parse_levels`, `introspection_shapes`.
- **Spatial parser groundwork**: `parser/spatial.rs` behind feature
  `spatial-grammar` (six typed keywords, frame-path conformance, static/dynamic
  classification); corpus byte-for-byte unaffected.
- **MCU bring-up**: `bindings/c/mcu/` hello-world + Makefile for the two locked
  targets; `.github/workflows/mcu-cross.yml` cross-compile + zero-forbidden-heap
  symbol audit.
- Gate evidence [m15w1.md](../gates/m15w1.md); phase tracker
  [phase-32.md](../gates/phase-32.md); [CHANGELOG.md](../../CHANGELOG.md) M15W1
  tranche.

Notes:

- No architecture divergence: wave A implements pinned invariants
  (observability-plane I-OB1..I-OB7); no ADR landed.
- Live SHM attach for the tools is deferred to the diag session; without a
  running publisher they report `NotFound` (namespace scoping).
- MCU cross-compile executes in CI; the Linux authoring host lacks the pinned
  `arm-none-eabi` toolchain, so the local suite carries the skeleton.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `ECHO_GREEN = 1`; `HZ_GREEN = 1`; `DIAG_SKELETON = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M15W1) = PASS`**

### How to reproduce

```text
cargo test -p drm-core --test trace
cargo test -p drm-diag
cargo test -p drm-cli --lib
cargo test -p drm-workspace --features spatial-grammar spatial
cargo xtask audit-traceability
cargo xtask check-layering
```

## M15W2 — Diag Daemon, Record Core Start

| Variable | Target | Measured |
|---|---|---|
| `DIAG_RATE_GREEN` | observed rate matches injected knowns (I-OB3) | 1 — `drm-diag` `RateWindow` `diag_observed_rate` matches 10/100/1000 Hz injected sequences within 1%, and a burst window reports its own high rate then decays to idle |
| `DIAG_DEADLINES` | injected deadline-miss matrix detected (I-OB4) | 1 — `diag_deadline_matrix` (burst vs even traffic, multi-deadline consumers, startup silence, publisher death, recover-between-polls) + `diag_condition_matrix` (missed deadline, gap, pool exhaustion, epoch change, link down) |
| `DIAG_NO_OVERHEAD` | with/without diag latency unchanged (I-OB2) | 1 — `diag_sample` reads a `ChannelView` only; `diag_no_overhead` shows counters bit-identical before/after, `diag_epoch_sandwich` discards torn reads |
| `RECORDER_INGRESS` | borrow-copy-release loop green (I-RC1) | 1 — `drm-record` `record_borrow_scope`: 0 borrows held during disk-stage drains, peak ≤ 1 across 200 samples |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Diag aggregation** (`crates/drm-diag`): `Aggregator` + `DiagReport::render`
  (channel counters, per-consumer deadline states, link stats), `ChannelView`
  decoupling the sampler from the runtime type. Tests `diag_aggregate_report`,
  `fresh_channel_stats`.
- **Deadline matrix** (`crates/drm-diag`): `DeadlineState`
  (`WaitingFirst/Current/Late`), `ConsumerStats`, `deadline_tick` (one
  `miss_count` per `Current → Late`), `Condition`/`DiagSignals::detect` for the
  five architecture-defined conditions. Tests `diag_deadline_matrix`,
  `diag_condition_matrix`.
- **Record core** (`crates/drm-record`, ops): `Recorder::ingress_step`
  (borrow → copy into a per-topic slab slot → release, I-RC1), per-topic
  `TopicSlab` sized to each topic's bound (I-RC2), `OnFull {Drop,Stop,
  Backpressure}` (I-RC3), `GapLedger` coalesced ranges + `gap_detail_lost`
  (I-RC5), global-order `DescEntry` ring. `#![forbid(unsafe_code)]`. Tests
  `record_borrow_scope`, `record_bounded`, `record_policies`,
  `record_gap_ledger`, `desc_fields`.
- **MCU zero-heap data path** (`bindings/c/mcu`): `drm_node.{h,c}` static SPSC
  topic ring (I-SR5, `REQ-CORE-087`); `drm_serial.{h,c}` COBS + CRC32C bounded
  framing (I-SR2..I-SR4, `REQ-OPS-004`); `selftest.c` host parity over the golden
  COBS/CRC vectors. `make host-test`, `make host-audit`; `mcu-cross.yml` runs
  parity + zero-forbidden-heap symbol audit on both locked targets.

Notes:

- No architecture divergence: pinned invariants only (observability-plane
  I-OB2/3/4/5/7, record-replay I-RC1/2/3/5, serial-mcu I-SR2..I-SR6); no ADR.
- `drm-record` is projected in codebase.md §2.1 as an ops crate; disk threads,
  MCAP framing, and replay are carried to M15W3/M15W4.
- MCU parity/audit legs run with the host compiler locally; the Arm cross-build
  executes in CI (`mcu-cross.yml`).

### Weekly gate checklist (chaired by E1, Friday)

- [x] `DIAG_RATE_GREEN = 1`; `DIAG_DEADLINES = 1`; `DIAG_NO_OVERHEAD = 1`; `RECORDER_INGRESS = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M15W2) = PASS`**

### How to reproduce

```text
cargo test -p drm-diag
cargo test -p drm-record
make -C bindings/c/mcu host-test
make -C bindings/c/mcu host-audit
cargo xtask audit-traceability
cargo xtask check-layering
```

## M15W4 — Replay Pipeline, MCU Serial Interop Gate Prep

| Variable | Target | Measured |
|---|---|---|
| `REPLAY_FAITHFUL` | replay preserves samples/order/timestamps/gaps (I-RC4) | 1 — `drm-record` `replay_roundtrip` (record → wipe → replay; payload bytes, per-topic `(publisher_epoch, seq)`, global `(recv_mono_ns, ingress_order)`, gaps, source timestamps), `replay_publisher_faithful` (namespace-isolated republish), `replay_reader_epoch` (`PublisherEpoch` boundary), `replay_summary_metrics` |
| `MCU_INTEROP` | host↔MCU byte-for-byte serial exchange green | 1 — `drm-router` `mcu_host_interop` (canonical COBS/CRC sequence byte-identical, deterministic encode, cross-decode), `serial_loopback_x10k`, `serial_corruption`, `serial_oversize`; `bindings/c/mcu` `make host-test` parity; `mcu-cross.yml` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Replay reader** (`crates/drm-record/src/replay.rs`): `Session::decode`
  reconstructs channels, per-topic streams ordered by `(publisher_epoch, seq)`,
  and gap boundaries from `drm.gap`; `global_order` reproduces
  `(recv_mono_ns, ingress_order)`. Tests `replay_reader_epoch`,
  `replay_roundtrip`.
- **Replay scheduler**: `SchedulerMode {Realtime, Rate, AsFast}` over an exact
  rational `Rate`; target = `(recv_mono_ns − start) / rate`, ties broken by
  `ingress_order`. Test `replay_scheduler_modes`.
- **Replay publisher + namespace**: `ReplayPublisher` republishes under
  `ReplayNamespace::derive(base, run)` (`I-DP2` isolation; source timestamps
  verbatim, replay time = schedule); `ReplayReport` (samples/topics/gaps/
  gap_sequences/drops/namespace). Tests `replay_publisher_faithful`,
  `replay_namespace_isolated`, `replay_summary_metrics`.
- **Record disk + MCAP** (`crates/drm-record/src/{mcap,disk}.rs`, M15W3
  carry-over): from-scratch MCAP writer/reader (header/schema/channel/message/
  chunk/statistics/index/footer), `drm.sample`/`drm.gap` metadata, optional zstd
  chunks; `disk::Stage` drains recorder-owned memory only (`I-RC1`). Tests
  `record_disk_mcap`, `record_mcap_drm_meta`, `record_mcap_zstd`,
  `record_slow_disk_policy`, `record_disk_isolation`.
- **Replay fixtures spec**: [replay-fixtures.md](../spec/replay-fixtures.md)
  (`record_known_session` with injected history lap + policy drops).

Notes:

- No architecture divergence: pinned invariants only (record-replay
  I-RC1..I-RC6, deployment-identity I-DP2, serial-mcu I-SR2..I-SR7); no ADR.
- `drm-record` remains an ops crate depending only on `drm-types` + `drm-core`
  (`#![forbid(unsafe_code)]`); `zstd` is used only for optional MCAP chunk
  compression.

### Weekly gate checklist (chaired by E3, Friday)

- [x] `REPLAY_FAITHFUL = 1`; `MCU_INTEROP = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M15W4) = PASS`**

### How to reproduce

```text
cargo test -p drm-record replay
cargo test -p drm-record
cargo test -p drm-router mcu_host_interop
make -C bindings/c/mcu host-test
cargo xtask audit-traceability
cargo xtask check-layering
```
