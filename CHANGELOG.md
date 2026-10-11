# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Sections are dated when tagged; dateless sections have not been tagged.

## [Unreleased]

### Added

- Week M19 W1 tranche: spatial types and static collapse (Phase 33 start). New
  `crates/drm-spatial` runtime crate: distinct typed quantities
  (`Point3`/`Direction3`/`Vector3`/`Pose3`/`Transform3`/`Twist3`), rigid `SE(3)`
  apply/compose, the WGS84→ECEF→ENU geodetic chain, affine clock-domain mapping
  with identity epoch and unresolved rejection, and versioned dynamic-edge
  cache slots (lockless seqlock over `AtomicU64` `f64` bit patterns, single
  writer per edge) ([spatial-core.md](docs/algorithms/spatial/spatial-core.md);
  REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-012, REQ-SPATIAL-004,
  REQ-SPATIAL-041, REQ-SPATIAL-051, REQ-SPATIAL-050, REQ-SPATIAL-030).
  `drm-workspace` gains spatial grammar extensions (six keywords, `space:`
  binding, dynamic `SpatialPoint3`; REQ-SPATIAL-005/009/047) and the
  build-time static-collapse engine v0 that parses `spaces/**.drm`, classifies
  edges, folds contiguous static chains to one constant, and emits a
  `@generated` C artifact (REQ-SPATIAL-031/048/049/017). An independent `NumPy`
  oracle (`tests/oracle/spatial_oracle.py` +
  `tests/fixtures/spatial/oracle_vectors.json`) is the numeric ground truth for
  `cargo test -p drm-spatial math`. Gate evidence
  ([m19w1.md](docs/gates/m19w1.md), [phase-33.md](docs/gates/phase-33.md));
  metrics [metrics-m19.md](docs/metrics/metrics-m19.md).
- Week M18 W4 tranche: hardening close, soak escalation, month-18 review. The
  soak harness escalates to daily 24 h runs over rotating topologies
  (`single-machine` → `two-machine-sim` → `physical-pair`) with an I-QAL2
  evidence ledger (`docs/metrics/soak-ledger.json`), a per-run anomaly
  dashboard, and the three-consecutive-clean gate rule
  (`xtask/src/soak.rs`, [soak-ledger.md](docs/spec/soak-ledger.md);
  `REQ-QAL-013`, `REQ-QAL-014`); `.github/workflows/soak-nightly.yml` schedules
  the daily run. The hardening gate closes with every fuzz/chaos/benchmark/
  matrix/sanitizer suite green (`HARDENING_GATE = PASS`, `SOAK_DAILY =
  scheduled`), and the month-18 review commits M19–M22 to the spatial wave
  ([month-18.md](docs/gates/month-18.md), [m18w4.md](docs/gates/m18w4.md),
  [m18w3.md](docs/gates/m18w3.md)); [metrics-m18.md](docs/metrics/metrics-m18.md)
  records the M18W3/M18W4 variables.
- Week M18 W2 tranche: hardening — benchmark baseline + Studio Graph/Run
  panels. The benchmark harness lands with a fixed pinned-CPU methodology
  (`crates/drm-core/benches/{latency,throughput}.rs`,
  `crates/drm-router/benches/router.rs`) and a local counting allocator: the
  publish/read latency, borrow throughput, ring/pool algebra, and router lane
  loopback p50/p99 are recorded as the regression reference
  ([bench-baseline.md](docs/spec/bench-baseline.md),
  [bench-methodology.md](docs/spec/bench-methodology.md)), and the
  zero-allocation + bounded-time invariants are asserted inside the benches so
  a violation fails `cargo bench` (REQ-QAL-011, REQ-QAL-012); a pinned-CPU CI
  `bench` job re-runs them. The Studio protocol gains read-only `RunSessions`
  and `RuntimeStats` requests over the supervisor (I-ST10, REQ-STUDIO-018); the
  renderer adds the read-only Graph activity (REQ-STUDIO-017) and the Run
  activity, and Electron packaging lands (`apps/studio/electron`,
  REQ-STUDIO-019). Gate evidence [m18w2.md](docs/gates/m18w2.md);
  [metrics-m18.md](docs/metrics/metrics-m18.md) records
  `BENCH_BASELINE = recorded`, `BENCH_INVARIANTS = green`,
  `STUDIO_PANELS = functional`.
- Week M18 W1 tranche: hardening — fuzz expansion + chaos suites. The fuzz
  fleet grows to `FUZZ_TARGETS_LIVE = 5/5` core targets plus a verify-fixture
  target: `hazard` (a structured storm through the public publish/read/reclaim
  surface), `serial` (one-shot decode, streaming `0x00` resync, COBS
  reversibility), and `verify` (malformed topic bytes + node/topology manifests
  through `model::build` and `verify_lite`, diagnostics-only). All six targets
  run nightly at 3600 s and the three new ones smoke per PR
  (`.github/workflows/fuzz-nightly.yml`, `.github/workflows/ci.yml`). The chaos
  suites (`crates/drm-core/tests/stress/chaos_kill_matrix.rs`,
  `crates/drm-router/tests/link_storm.rs`) kill every borrow/publish stage,
  exercise PID reuse and pool-exhaustion storms, and flap links × routers under
  I-RT6/I-RT5 — recovery holds every time (`CHAOS_GREEN = 1`). `shm_kill`'s
  child driver is repaired to invoke the ignored test (`--ignored`), so the 8
  real `SIGKILL` cells actually run. Evidence: chaos inventory
  [chaos-inventory.md](docs/spec/chaos-inventory.md), triage board
  [fuzz-triage-board.md](docs/spec/fuzz-triage-board.md), coverage
  [fuzz-coverage.md](docs/spec/fuzz-coverage.md), gate
  [m18w1.md](docs/gates/m18w1.md); [metrics-m18.md](docs/metrics/metrics-m18.md)
  records `FUZZ_TARGETS_LIVE = 5/5`, `CHAOS_GREEN = 1`, `FUZZ_CRASHES = 0`.
- Week M17 W4 tranche: Studio inspectors + run-session supervisor, month-17
  review. The run-session supervisor
  (`crates/drm-cli/src/studio/supervisor.rs`) owns `RunSession`s independently
  of any renderer connection: sessions carry id/start time/node processes
  (pid, captured stdout/stderr, status) and a `leave_running` disconnect
  policy; a disconnect keeps sessions running, and closing with running nodes
  prompts Leave-running/Stop — a close never kills or orphans a node process
  (I-ST10, `REQ-STUDIO-010`). `apps/studio` adds the read-only inspectors
  (Workspace/Topics/Nodes/System) and the Problems panel over TanStack Query,
  with verify diagnostics carrying the typed `Severity` classes
  (`error`/`warning`/`runtime`/`info`) that mirror the verify model; the
  renderer holds zero DRM logic (I-ST1). Mutation audit
  [studio-mutation-audit.md](docs/spec/studio-mutation-audit.md) clean. Gate
  evidence [m17w4.md](docs/gates/m17w4.md) and month review
  [month-17.md](docs/gates/month-17.md); [metrics-m17.md](docs/metrics/metrics-m17.md)
  records `STUDIO_MVP = 1`, `RUN_SUPERVISOR = 1`, `MONTH17_REVIEW = ON_TRACK`.
- Week M17 W3 tranche: Studio protocol implementation (host + scaffold).
  `drm studio --stdio [--trust]` lands the long-lived host
  (`crates/drm-cli/src/studio/`): a mandatory `Hello` capability handshake, typed
  JSON-RPC dispatch through engine calls only, `base_revision`/`CONFLICT`
  revisioning, journaled `ChangeSet` commits with idempotent crash recovery,
  conditional undo guarded by input hashes, and `toml_edit` format-preserving
  manifest edits (I-ST1/I-ST2/I-ST4/I-ST6/I-ST7/I-ST8/I-ST9/I-ST11/I-ST13).
  `packages/studio-protocol` is generated TS (`@generated by drm-studio-codegen
  v1`); `xtask gen-check` is extended to reject hand edits (R6). `apps/studio`
  adds the Vite + React 19 + TS strict renderer scaffold with ESLint
  renderer-import boundaries and a Zustand presentation store. Gate evidence
  [m17w3.md](docs/gates/m17w3.md); [metrics-m17.md](docs/metrics/metrics-m17.md)
  records `STUDIO_PROTOCOL_GREEN = 1`, `MUTATIONS_REVISIONED = 1`,
  `STUDIO_SCAFFOLD = 1`.
- Week M17 W2 tranche: Phase 32 gate (operational tooling) + Studio protocol doc.
  The Phase 32 gate closes with every submodule conformance suite green
  (introspection/diag, trace, RPC, record/replay, serial interop, handles/verify,
  MCU host parity + zero-forbidden-heap audit); evidence
  [m17w2.md](docs/gates/m17w2.md), tracker
  [phase-32.md](docs/gates/phase-32.md) status → PASS. The protocol-level Studio
  doc [studio-protocol.md](docs/algorithms/studio/studio-protocol.md) is merged:
  `StudioTransport` abstraction, capability handshake, typed command set, named
  events, revisioned optimistic concurrency, journaled ChangeSets, conditional
  undo, format-preserving editing, run-session supervisor, control/data-plane
  separation, workspace-trust gate, and extension-host budgets; invariants
  I-ST1..I-ST13 defined with an explicit post-1.0 deferred list (creation
  dialogs, editors, Inspector UI, command palette/keyboard, DataStore UI, remote
  web mode, web sandbox, native adapters, `.drmx`, `drm-visual-schemas`).
  [metrics-m17.md](docs/metrics/metrics-m17.md) records
  `GATE_STATUS(Phase 32) = PASS`, `STUDIO_PROTOCOL_DOC = merged`.
- Week M17 W1 tranche: Phase 32 close-out gates — introspection, trace, RPC.
  `drm-rpc` adds `rpc_capacity_busy_gap_forward` (the `rpc_capacity` cell): a full
  client correlation table returns `RpcError::Busy`; a full server operation table
  consumes nothing and the retained request is forwarded and served once a slot
  frees (`publish ≠ accepted`, `I-RPC6`/`I-RPC7`). Introspection gate re-runs
  `introspect_no_pin` (`I-OB5` no-pin) and `diag_observed_rate` (`I-OB3` rate
  tolerance); trace gate re-runs `trace_never_blocks`/`trace_drop_counter`
  (`I-OB1`/`I-OB6`); RPC gate re-runs `dispatch_concurrent_no_cross_correlation`
  (`I-RPC1`, `C ∈ 1..=16`), `cancel_at_each_phase_boundary` (`I-RPC2`), and
  `dispatch_timeout_expires_and_late_terminal_stale` (`I-RPC3`). The MCU host
  parity + zero-forbidden-heap suites are wired as a permanent blocking `mcu` job
  in [ci.yml](.github/workflows/ci.yml) on every PR/push, with `push: [main]`
  added to [mcu-cross.yml](.github/workflows/mcu-cross.yml) (regression
  permanence). Gate evidence [m17w1.md](docs/gates/m17w1.md); Phase 32 tracker
  [phase-32.md](docs/gates/phase-32.md) records the deferred post-1.0 Studio UI
  scope; [metrics-m17.md](docs/metrics/metrics-m17.md) records
  `INTROSPECT_GATE = PASS`, `TRACE_GATE = PASS`, `RPC_GATE = PASS`.
- Week M16 W4 tranche: record/replay gate + serial interop gate + month-16
  review. `drm-record` adds `record_mcap_interop`: a recorded session read back
  through the pinned third-party `mcap` crate — `Summary::read` reports the
  standard statistics (messages, channels/schemas, DRM metadata count) and
  `MessageStream` yields one message per sample with `publish_time = log_time`
  and the recorded payload bytes verbatim (`I-RC6`). Record gate evidence
  re-runs `record_borrow_scope`/`record_bounded`/`record_policies`/
  `replay_roundtrip` (`I-RC1`..`I-RC4`); serial gate evidence re-runs the
  byte-exact host↔MCU cross-decode matrix (`I-SR6`), corruption injection
  (`I-SR3`), oversize rejection (`I-SR2`), and the zero-forbidden-heap linker
  audit on the MCU data path (`I-SR5`, `make host-audit`). Gate evidence
  [m16w4.md](docs/gates/m16w4.md); month review [month-16.md](docs/gates/month-16.md);
  [metrics-m16.md](docs/metrics/metrics-m16.md) records `RECORD_GATE = PASS`,
  `SERIAL_GATE = PASS`, `MONTH16_REVIEW = ON_TRACK`.
- Week M16 W3 tranche: RPC per-phase state transitions, handle compaction,
  serial host↔MCU interop. `RpcClient` counts each lifecycle event kind as it
  dispatches (one `Accepted`, one `Started`, one `Progress` per non-final phase,
  one `Terminal`, exactly one `CancelAccepted` on a boundary cancel; `I-RPC2`/
  `I-RPC10`), asserted by `action_runs_all_phases` and
  `cancel_at_each_phase_boundary`. `HandleTable::compact(ack)` adds the explicit
  compaction gate and dense re-pack, and `rebuild_guidance` emits a deterministic
  coordinated-rebuild checklist (rebuild/restart every node binary; schema
  bindings never regenerated — `I-H3`/`I-H4`/`I-H7`); `drm ids compact --yes`
  prints it. The `drm-router` serial codec pins the shared COBS golden vectors
  and runs a byte-exact host↔MCU cross-decode matrix (`serial_cobs_golden`,
  `mcu_host_interop_matrix`), mirrored by the MCU host self-test
  (`bindings/c/mcu/selftest.c`, fixed `DRM_SERIAL_MAX_WIRE = 264` buffers).
  Gate evidence [m16w3.md](docs/gates/m16w3.md); [metrics-m16.md](docs/metrics/metrics-m16.md)
  records `RPC_CANCEL_GREEN = 1`, `IDS_COMPACT_GREEN = 1`, `SERIAL_MATRIX = 1`.
- Week M16 W2 tranche: RPC action phase machine + trace/record gate prep. NEW
  [action.rs](crates/drm-rpc/src/action.rs) (`ActionState`/`ActionContext`/
  `Phase`/`ActionExec`) executes one declared phase per reactor worker step so
  the reactor observes the cancel token at every phase boundary; no new phase
  starts after observation and the terminal is `Terminal(Cancelled)` (`I-RPC2`),
  with `expected_max_duration_ns` SLO metadata only (`I-RPC8`). The
  [server](crates/drm-rpc/src/server.rs) gains a shared cancel token,
  `UnaryService::action`/`finish`, and the `action_phases_run` counter; the
  dispatch fixture now covers `C ∈ 1..=16`. `drm-record` adds
  `record_slab_full_boundary_exact` (slab boundary exactness; backpressure loses
  nothing). Trace gates re-run (`trace_never_blocks`/`trace_drop_counter`, I-OB1/
  I-OB6). Gate evidence [m16w2.md](docs/gates/m16w2.md); [metrics-m16.md](docs/metrics/metrics-m16.md)
  records `TRACE_GATES = 1`, `RPC_DISPATCH_FULL = 1`, `RPC_CANCEL_GREEN = 1`,
  `DIAG_POLISH = 1`.
- Week M16 W1 tranche: RPC implementation start. NEW [drm-rpc](crates/drm-rpc)
  ops crate (`#![forbid(unsafe_code)]`; depends on `drm-types` + `drm-core` only):
  a bounded wire [codec](crates/drm-rpc/src/codec.rs) for
  `Request`/`Cancel`/`RpcEvent`, a [client](crates/drm-rpc/src/client.rs)
  (`call`/`submit`/`pump`, correlation table, `u64` cid that never wraps,
  per-cid state machine, `RpcTimeout` + best-effort `Cancel`, late terminals
  dropped stale), and a [server](crates/drm-rpc/src/server.rs) reactor
  (`serve_step` reserve-then-consume admission, random `server_session`,
  bounded per-client outboxes, `run_workers` bounded worker pool). Implements
  rpc-actions I-RPC1/3/4/6/7/9/10 (REQ-OPS-011/012/013/020/021/022/023/025).
  Replay E2E (`replay_roundtrip`, I-RC4) re-run as gate M16W4 evidence. Gate
  evidence [m16w1.md](docs/gates/m16w1.md); [metrics-m16.md](docs/metrics/metrics-m16.md)
  records `RPC_CALL_GREEN = 1`, `RPC_CID_GREEN = 1`, `REPLAY_E2E = 1`.
- Week M15 W4 tranche: replay pipeline + record disk/MCAP stage. `drm-record`
  gains the from-scratch MCAP writer/reader ([mcap.rs](crates/drm-record/src/mcap.rs))
  and recorder-owned disk stage ([disk.rs](crates/drm-record/src/disk.rs)) —
  standard MCAP framing plus `drm.sample`/`drm.gap` DRM metadata records
  (`I-RC6`, REQ-OPS-006), optional zstd chunks, and no DRM handle on the disk
  thread (`I-RC1`, REQ-OPS-007). NEW replay module
  ([replay.rs](crates/drm-record/src/replay.rs)): `Session::decode` reconstructs
  per-topic streams ordered by `(publisher_epoch, seq)` and gap boundaries
  (`I-RC4`/`I-RC5`, REQ-OPS-009/018); `SchedulerMode {Realtime, Rate, AsFast}`
  schedules the recorder-observed global order `(recv_mono_ns, ingress_order)`;
  `ReplayPublisher` republishes under an isolated `ReplayNamespace` derived from
  the live deployment id (REQ-CORE-088/089). Fixture spec
  [replay-fixtures.md](docs/spec/replay-fixtures.md); gate evidence
  [m15w4.md](docs/gates/m15w4.md); [metrics-m15.md](docs/metrics/metrics-m15.md)
  records `REPLAY_FAITHFUL = 1`, `MCU_INTEROP = 1`.
- Week M15 W2 tranche: diag aggregation/deadline matrix, record-core start,
  MCU zero-heap data path. `drm-diag` gains the out-of-band `Aggregator` +
  `DiagReport` (channel counters, per-consumer deadline states, link stats;
  I-OB2), the `DeadlineState` machine + `deadline_tick` (I-OB4), and the
  `Condition`/`DiagSignals::detect` matrix for missed deadline, gap, pool
  exhaustion, epoch change, and link down. NEW [drm-record](crates/drm-record)
  ops crate: `Recorder::ingress_step` borrow-copy-release (`I-RC1`,
  REQ-OPS-005/007), per-topic `TopicSlab` pools with an exact byte budget
  (`I-RC2`, REQ-OPS-019), `OnFull {Drop,Stop,Backpressure}` (`I-RC3`,
  REQ-OPS-008), and the coalesced `GapLedger` (`I-RC5`, REQ-OPS-018). MCU
  freestanding subset gains the zero-heap static SPSC ring
  ([drm_node.c](bindings/c/mcu/drm_node.c); `I-SR5`, REQ-CORE-087) and bounded
  COBS/CRC32C serial framing ([drm_serial.c](bindings/c/mcu/drm_serial.c);
  I-SR2..I-SR4, REQ-OPS-004) with a host parity self-test and zero-forbidden-heap
  symbol audit wired into [mcu-cross.yml](.github/workflows/mcu-cross.yml). Gate
  evidence [m15w2.md](docs/gates/m15w2.md); [metrics-m15.md](docs/metrics/metrics-m15.md)
  records `DIAG_RATE_GREEN = 1`, `DIAG_DEADLINES = 1`, `DIAG_NO_OVERHEAD = 1`,
  `RECORDER_INGRESS = 1`.
- Week M15 W1 tranche: introspection tools (echo/hz/log) + diag daemon start.
  NEW [drm-diag](crates/drm-diag) ops crate: `diag_sample` epoch sandwich
  (I-OB7), `deadline_tick` state machine (I-OB4), sliding-window `RateWindow`
  (I-OB3), and the read-only `Observer` (I-OB5). `drm-core` gains the wait-free
  per-producer SPSC trace lanes + build-time dictionary
  ([trace/mod.rs](crates/drm-core/src/trace/mod.rs); I-OB1/I-OB6,
  REQ-OPS-014/015/017); the lock-free primitive stays in the runtime layer
  (codebase.md §2.2 confines `unsafe`). `drm` gains read-only `echo`/`hz`/`log`
  subcommands ([echo.rs](crates/drm-cli/src/echo.rs),
  [hz.rs](crates/drm-cli/src/hz.rs), [log.rs](crates/drm-cli/src/log.rs)) that
  resolve deployment metadata and attach as non-publishing observers. Phase 33
  parser groundwork behind feature `spatial-grammar`
  ([parser/spatial.rs](crates/drm-workspace/src/parser/spatial.rs); non-breaking,
  corpus unaffected). MCU bring-up skeleton
  ([bindings/c/mcu](bindings/c/mcu), `.github/workflows/mcu-cross.yml`). Gate
  evidence [m15w1.md](docs/gates/m15w1.md), phase tracker
  [phase-32.md](docs/gates/phase-32.md), [metrics-m15.md](docs/metrics/metrics-m15.md)
  records `ECHO_GREEN = 1`, `HZ_GREEN = 1`, `DIAG_SKELETON = 1`.
- Week M14 W4 tranche: tooling-wave design lock + month-14 review. NEW
  [rpc-design-seed.md](docs/spec/rpc-design-seed.md) seeds `drm-rpc`
  (correlated topic pairs, `u64` cid per client session never reused,
  deterministic phase-boundary cancel, relative-budget deadlines, core-untouched
  rule I-R4); NEW [studio-prep-survey.md](docs/spec/studio-prep-survey.md) locks
  the `drm studio --stdio` JSON-RPC host shape, the generated TS types pipeline,
  the engine-only mutation flow, the capability model, and the deferred scope;
  NEW [tooling-design-lock.md](docs/spec/tooling-design-lock.md) consolidates
  every Phase 32 sub-design (`echo`/`hz`/`log`, `diag`, `record`, `trace`,
  RPC/actions, serial/MCU, Studio bridge) with sub-owner signatures.
  [rpc-actions.md](docs/algorithms/ops/rpc-actions.md),
  [studio-protocol.md](docs/algorithms/studio/studio-protocol.md), and
  [serial-mcu.md](docs/algorithms/router/serial-mcu.md) gain M14W4 design
  confirmation / MCU-plan-final sections; [phase-31.md](docs/gates/phase-31.md)
  status → PASS (M14W2 chair verdict). Executable design-lock tests in
  [main.rs](xtask/src/main.rs) (`design_lock`) make the locks a permanent gate.
  Gate evidence in [m14w4.md](docs/gates/m14w4.md) and
  [month-14.md](docs/gates/month-14.md); [metrics-m14.md](docs/metrics/metrics-m14.md)
  records `TOOLING_DESIGN_LOCK = 1`, `MONTH14_REVIEW = ON_TRACK`.
- Week M14 W3 tranche: spatial types/clocks + record core design confirmations.
  NEW [spatial-types-design.md](docs/spec/spatial-types-design.md) confirms the
  six typed quantities, compile-time `Point3<CameraFront>` vs dynamic
  `SpatialPoint3{id}` + `Stamped<T>` binding, the grammar-extension EBNF draft,
  `[spatial]` contract parsing, the I-D6 static/dynamic inference matrix, and the
  four clock domains with affine mapping, identity epochs, and PTP/GNSS/SimReplay
  degradation paths. NEW [mcu-bring-up.md](docs/spec/mcu-bring-up.md) locks the
  reference MCU/RTOS (STM32H743 + FreeRTOS; fallback STM32F407), the pinned
  toolchain, the CI cross-compile/zero-heap/stack jobs, the host↔MCU bring-up
  steps, and the preventive-first linker-section audit.
  [observability-plane.md](docs/algorithms/ops/observability-plane.md) pins the
  trace dictionary versioning (`dict_version`, append-only per deployment) and
  records the trace/diag design confirmation;
  [record-replay.md](docs/algorithms/ops/record-replay.md) records the record
  design confirmation. Executable design-lock tests in
  [main.rs](xtask/src/main.rs) (`design_lock`) make the confirmations a permanent
  gate. Gate evidence in [m14w3.md](docs/gates/m14w3.md);
  [metrics-m14.md](docs/metrics/metrics-m14.md) records
  `SPATIAL_TYPES_DESIGN = 1`, `CLOCK_DESIGN = 1`, `RECORD_DESIGN = 1`.
- Week M14 W1 tranche: Windows↔Linux full interop matrix + serial-MCU doc
  final. The cross-OS interop harness
  ([cross_os.rs](xtask/src/cross_os.rs)) now spans the full work-plan matrix
  `{windows, linux} × {udp, tcp, serial} × {c, cpp, rust, python}`: `InteropLang`
  adds C++, `InteropTransport` adds `Serial` (cross-platform in-process
  host-loopback through the byte-exact `drm-router` COBS/CRC32C codec),
  `interop_core_matrix` is the 24-cell gate-blocking same-language core and
  `interop_matrix` the 96-cell full table split by `CellClass {Core, Tracked}`;
  `run_interop` writes the complete cell table to
  `target/interop/matrix-report.txt`. Physical-matrix runbook + loopback
  evidence in [phase-31-runbook.md](docs/gates/phase-31-runbook.md);
  [serial-mcu.md](docs/algorithms/router/serial-mcu.md) finalized (COBS binding;
  absorbs the M02W3/M05W3/M06W1/M13W1 drafts). Gate evidence in the M14W1
  section of [phase-31.md](docs/gates/phase-31.md); conformance matrix and port
  board updated ([tests/conformance/README.md](tests/conformance/README.md));
  `ci.yml`/`windows-nightly.yml` gain the serial interop step;
  [metrics-m14.md](docs/metrics/metrics-m14.md) records `FULL_MATRIX = 1`,
  `SERIAL_DOC = 1`.
- Week M13 W4 tranche: Windows crash-recovery matrix + Phase 31 gate prep +
  month-13 review. Windows publisher kill/restart ×50
  ([windows.rs](crates/drm-core/src/channel/lifecycle/tests/windows.rs)
  `win_lifecycle_crash_x50` + `win_lifecycle_crash_child`): kernel `LockFileEx`
  release, exactly-once epoch advance, preserved floor, consumer observes each
  epoch (I-W3/I-L2/I-L3/I-L4/I-L6). Windows reader-kill reclamation matrix
  ([reclaim/tests/windows.rs](crates/drm-core/src/channel/reclaim/tests/windows.rs)
  `reclaim_kill_matrix_windows`): kill at every borrow stage over real Win32
  identity, proof-before-reclaim under the registration lock, live reader never
  mis-reclaimed, reused PID is a dead incarnation (I-E5/REQ-CORE-057). Link-cut
  isolation ([data_path.rs](crates/drm-router/src/data_path.rs)
  `Router::link_down`/`link_up`/`link_is_down` + `IngressOutcome::LinkDown`;
  suite [disconnect_isolation.rs](crates/drm-router/tests/disconnect_isolation.rs)):
  a down link takes only its routes offline; local shared memory and sibling
  links continue; reconnect resumes with a fresh ingress session (I-RT6).
  Cross-OS manifest ([cross_os.rs](xtask/src/cross_os.rs)) adds three green rows
  (`win-lifecycle-crash`, `win-reclaim-kill`, `router-disconnect-isolation`).
  Evidence in [phase-31.md](docs/gates/phase-31.md) (M13W4 section),
  [month-13.md](docs/gates/month-13.md), and
  [tests/conformance/README.md](tests/conformance/README.md);
  [metrics-m13.md](docs/metrics/metrics-m13.md) records
  `WIN_CRASH_MATRIX = 1`, `PHASE31_EVIDENCE = 1`, `MONTH13_REVIEW = ON_TRACK`.
- Week M13 W3 tranche: Windows conformance full port —
  [cross_os.rs](xtask/src/cross_os.rs) grows the cross-OS manifest from 12 to 33
  rows (runtime platform, lifecycle, toolchain, router, and bindings families),
  a `PortGap`/`PORT_BOARD` gap-tracking board with `board_complete()`, and
  `CONFORMANCE_RUNS`/`run_full()` executed by `cargo xtask test --cross-os
  --full` (per-run evidence under `target/conformance/`; flag parsing in
  `xtask/src/main.rs`); every scheduled row carries an owner, target week, and
  gate-risk note (zero silent skips). Windows platform fixes in
  [windows.rs](crates/drm-core/src/shm/windows.rs): `win_open_semantics` now
  asserts the undersized `StaleIncomplete` classification (I-W2/I-W3 size
  parity) and `win_precommit_refusal` forces a `VirtualLock` refusal and asserts
  the explicit `QuotaExceeded` (I-W6). Evidence in
  [phase-31.md](docs/gates/phase-31.md) (M13W3 section),
  [tests/conformance/README.md](tests/conformance/README.md) full-port table,
  and `windows-nightly.yml` full-run ×2 step;
  [metrics-m13.md](docs/metrics/metrics-m13.md) records
  `WIN_CONFORMANCE_FULL = 1`, `PORT_FIX_COUNT = 6`.
- Week M13 W2 tranche: Windows↔Linux interop matrix + spatial extraction p2 —
  the parameterized cross-OS interop harness
  ([cross_os.rs](xtask/src/cross_os.rs) `InteropCell`/`interop_matrix`/
  `cell_state`/`run_cell`/`run_interop`, `cargo xtask test --cross-os
  --transport udp` / `tcp`; flag parsing in `xtask/src/main.rs`) runs the Rust
  pair over loopback in both directions for UDP and TCP with per-cell evidence
  under `target/interop/`, C/Python cells scheduled until their
  router-transport bindings land (Phase 32/33); serial route integration
  ([serial.rs](crates/drm-router/src/transport/serial.rs) `SerialTransport`
  implementing `TransportAdapter`, [data_path.rs](crates/drm-router/src/data_path.rs)
  `Router::egress_frame` sharing the I-RT1 filter/header build with `egress_udp`)
  with the full-path suite
  ([serial_route.rs](crates/drm-router/tests/serial_route.rs): registry, route
  shaping, socketpair loopback); spatial extraction p2 adds 15 `REQ-SPATIAL-*`
  rows (total 52) with matching slugs in
  [spatial-core.md](docs/algorithms/spatial/spatial-core.md)/
  [spatial-index.md](docs/algorithms/spatial/spatial-index.md)/
  [spatial-queries.md](docs/algorithms/spatial/spatial-queries.md); the
  `interop-cells` CI job and `windows-nightly.yml` interop step; gate draft
  [phase-31.md](docs/gates/phase-31.md); [metrics-m13.md](docs/metrics/metrics-m13.md)
  records `INTEROP_CELLS = 1`, `SPATIAL_REQ_P2 = 52`, `SERIAL_ADAPTER = 1`.
- Week M13 W1 tranche: Windows platform completion + serial transport — Windows
  publisher/consumer lifecycle over `LockFileEx` + pagefile sections with
  in-place `Init` recovery ([lifecycle.rs](crates/drm-core/src/channel/lifecycle.rs)
  `reinit_in_place`; pagefile sections have no unlink) and the real-backend
  suite ([lifecycle/tests/windows.rs](crates/drm-core/src/channel/lifecycle/tests/windows.rs):
  authority race, consumer open-only, 20-cycle restart epoch parity,
  reclamation-authority and identity subsets); bounded serial COBS/CRC32C
  socketpair loopback ×10k
  ([serial.rs](crates/drm-router/src/transport/serial.rs) `serial_loopback_x10k`);
  the cross-OS subset closes both M12W4 carry-overs (`win-race-create`,
  `win-init-reinit` now green) and adds green `serial-codec` plus scheduled
  `serial-loopback`; Windows CI jobs and the conformance matrix updated;
  [metrics-m13.md](docs/metrics/metrics-m13.md) records `WIN_PLATFORM_FULL = 1`,
  `SERIAL_FRAMES = 1`, `SPATIAL_REQ_MINT_START = 37`.
- Week M12 W4 tranche: Windows conformance subset + month-12 review — the
  cross-OS subset manifest `xtask/src/cross_os.rs` (`SUBSET`,
  `covers_required_invariants`, `cargo xtask test --cross-os`; registered in
  `xtask/README.md`/ADR 0003) that lists every Linux suite row which must run on
  Windows, each green or scheduled with a port note (`PORT_GAPS_CLOSED = 1`);
  Windows platform lifecycle parity
  ([platform_windows.rs](crates/drm-core/tests/platform_windows.rs),
  `I-W4b`/`I-W6`/`I-W7`) and guarded cleanup
  (`shm::windows::tests::win_cleanup`, `I-W4`); the month-12 review packet
  ([month-12.md](docs/gates/month-12.md), verdict ON_TRACK); `windows-nightly.yml`
  and the `windows-smoke` CI job now run the one subset (nightly ×3); new
  requirements `REQ-CORE-101` (Windows lifecycle suite parity) and
  `REQ-QAL-008` (cross-OS conformance subset).
- Week M12 W3 tranche: Windows reclamation + precommit exercised through the
  kill/PID-reuse (`win_lock_release_on_death` `I-W3`, `win_pid_reuse` `I-W5`)
  and `VirtualLock` (`win_precommit_roundtrip` `I-W6`) fixtures; the cross-OS
  matrix harness carried into M12W4.
- Week M12 W2 tranche: Phase 26 gate ([docs/gates/phase-26.md](docs/gates/phase-26.md),
  the eleven work-plan #26 rejection classes ⇄ executable verify-engine fixtures
  with `I-V1`..`I-V6` supporting evidence) and the Windows identity + locks
  continuation ([windows-backends.md](docs/algorithms/runtime/windows-backends.md))
  — `drm-core::shm::windows` release-on-death (`I-W3`), incarnation/PID-reuse
  (`I-W5`), active-hold lifecycle (`I-W8`), create-conflict rollback (`I-W7`),
  and last-handle teardown (`I-W4b`) tests; platform-neutral socket error-mapping
  parity fixtures in `drm-router::transport::{udp,tcp}`; `test_guard` compiled on
  every test host; extended `windows-smoke` CI, a scheduled `windows-nightly.yml`
  platform subset, and a scheduled MSVC `windows-asan` job in `sanitizers.yml`.
- Week M12 W1 tranche: Phase 30 gate ([docs/gates/phase-30.md](docs/gates/phase-30.md),
  the four work-plan #30 criteria ⇄ executable lane/shaping/reliability tests) and the
  Windows runtime backend start ([windows-backends.md](docs/algorithms/runtime/windows-backends.md))
  — `drm-core::shm::windows` (`WinShm` pagefile-backed sections via
  `CreateFileMappingW`/`MapViewOfFile` with `ERROR_ALREADY_EXISTS` ⇒ `AlreadyExists` and
  undersized `StaleIncomplete` classification; `WinLockBackend` non-blocking `LockFileEx`
  on non-inheritable handles; `VirtualLock` precommit refusal; PID + creation-`FILETIME`
  diagnostic identity), platform-selected `SegmentName`/`backend_segment_name` with
  `windows_section_name`/`windows_name_conforms`, `drm-core` `cfg(windows)` `windows-sys`
  dependency, and a `windows-smoke` CI job; POSIX-only suites are `cfg(unix)`-gated so
  `cargo test -p drm-core --lib` compiles on both hosts.
- Week M10 W1 tranche: router UDP path completion and TCP streams —
  `Router::routes_on(link)` (I-RT1: egress scans only the serviced link's route
  bindings, so a local-only topic never egresses; [route_filter.rs](crates/drm-router/tests/route_filter.rs)
  observes zero datagrams for it); the synthetic-publisher SHM hook `ChannelSink`
  ([import.rs](crates/drm-router/src/import.rs)) depositing header-stripped
  payloads through the standard `Channel::publish` path with a `restart_run`
  epoch bump, exercised end-to-end by [shm_hooks.rs](crates/drm-router/tests/shm_hooks.rs);
  an actionable `V-RT` route-fit failure that names the canon-encoded payload and
  directs oversize datagram topics to a stream transport (`REQ-ROUTE-018`,
  `verify_negatives_v2_route_fit_actionable`); the UDP datagram fuzz target
  ([datagram.rs](tests/fuzz/fuzz_targets/datagram.rs)) wired into the CI fuzz-PR
  smoke; and a nightly UDP loopback pair matrix
  ([fuzz-nightly.yml](.github/workflows/fuzz-nightly.yml)).
- Week M09 W4 tranche: verify catalogue completion ([verify-engine.md](docs/algorithms/toolchain/verify-engine.md) §Check catalogue) — `V-SPAT` (dynamic-edge single writer, write+transform conflict, declared `[spatial]` reads, `index.drm` capacity within budget, satisfiable history bound, no naked-coordinate fields), `V-RPC` (request/cancel/event route reliability, single-writer server-owned event topic, concurrency within the capacity model), and `V-REC` (`on_full = backpressure` on a `reject` topic requires a declared required sequential consumer), taking `ALL_CHECKS` to 11 ids with a negative + positive fixture each ([verify_engine.rs](crates/drm-workspace/tests/verify_engine.rs)); router core data path ([data_path.rs](crates/drm-router/src/data_path.rs), router-core.md §Functions `egress`/`ingress`) — `EgressRoute` (I-RT1 remote-consumer filter, I-RT3 datagram bound, I-RT18 record bound), `Router::egress`/`ingress_frame`, header-stripped synthetic-publisher `ImportSink` deposit with per-`(link, wire_channel_id)` epoch/sequence ordering (I-RT9), and `IngressOutcome`, with the UDP egress/ingress + imported-vs-local native-equivalence suite ([router_core.rs](crates/drm-router/tests/router_core.rs)).
- Week M09 W3 tranche: Phase 25 build-orchestration gate ([docs/gates/phase-25.md](docs/gates/phase-25.md), I-BO1..I-BO4 with I-BO5..I-BO8 and the adapter matrix as supporting evidence); verify negatives v2 suite ([verify_negatives_v2.rs](crates/drm-workspace/tests/verify_negatives_v2.rs)) — multiple `V-RT` (framing overflow, ambiguous route, no route), `V-REL` (reliable over UDP/serial), `V-MEM` (budget/lockable/pool invariant), and `V-CMP` (reject without a required sequential consumer) negatives with valid equivalents passing, `I-V5` collect-all, and route integration proving `V-RT` consumes the single `topology::derive` output; deterministic `drm dev` smoke suite ([dev_smoke.rs](crates/drm-cli/tests/dev_smoke.rs)) over scripted declaration/source edits on a scratch `examples/mixed_robot`; the `docs/spec/routing-derivation.md` draft is absorbed into a pointer to the normative [route-derivation.md](docs/algorithms/toolchain/route-derivation.md); CI `toolchain` job extended with the negatives-v2 and dev-smoke steps.
- Week M09 W1 tranche: Phase 24 codegen gate finalized ([docs/gates/phase-24.md](docs/gates/phase-24.md), I-G1..I-G9/R6, I-G7 POSIX-only pending the Windows backend) and build-orchestration implementation ([build-orchestration.md](docs/algorithms/toolchain/build-orchestration.md), I-BO1..I-BO8) — `drm-workspace::orchestrator` (`BuildRequest`, `BuildAdapter{Cmake,Cargo,Command}`, `BuildStep` sequence probe, injectable `AdapterRunner`, `DRM_GEN_INCLUDE`/`DRM_DEPLOY_METADATA` adapter hooks), `drm-workspace::deploy` (deterministic `.drm/deploy/deployment.json` handle table), `drm-workspace::watch` (two-category classification, reload-before-dispatch, pending-dirty retention, rescan recovery, path-only ownership), node `language`/`[build]` manifest fields, and the `drm build`/`drm info`/`drm dev [--once]` CLI commands; `examples/mixed_robot` (one node per language: python + c + rust) builds green via `drm build`; tests `crates/drm-workspace/tests/{build_order,watch_dispatch}.rs` and `crates/drm-cli/tests/example_build.rs`.
- Week M08 W4 tranche: Phase 23 workspace-model gate finalized ([docs/gates/phase-23.md](docs/gates/phase-23.md)) — deterministic model (shuffled-fs ×100), conflict rejection with all duplicate sites reported, communication loops harmless, fingerprint isolation, and a 5 000-case parse→model roundtrip; model runtime hook `drm-workspace::model::runtime_plan` (descriptor expectations, history depth, payload-pool sizing) with the composition-root e2e `drm-cli` model → codegen → live `Channel`; codegen invalidation suite ([codegen_invariants.rs](crates/drm-workspace/tests/codegen_invariants.rs)) — `gen_atomicity` kill harness (I-G4), fingerprint pruning (I-G5), node-API invalidation (I-G6), atomic replacement (I-G7), direction sensitivity (I-G8), handle-free bindings (I-G9), and R6 regen-diff hand-edit detection; `xtask gen-check` (R6) wired blocking with a red→green self-test; Phase 24 evidence pack ([docs/gates/phase-24-evidence.md](docs/gates/phase-24-evidence.md)).
- Week M08 W2 tranche: codegen start — `drm-workspace::codegen` (`GenPlan`/`GenUnit`, directed capability fingerprints, stable `.drm/gen/c/<topic>/target.h` paths, content-compare atomic writes, orphan reconciliation) renders canonical C (per-topic struct + inline encode/decode, node-scoped API surface), with `codegen_c` determinism/affected-only/mtime-preservation/fingerprint-bump/reconciliation suites plus a `cc -std=c11 -Werror` compile-and-decode golden test (I-G1..I-G3, REQ-TOOL-024..030/065); Python binding polish adds schema-driven `Copy.numpy(dtype, shape)` read-only views and the `Consumer._borrow_outstanding` diagnostic hook for the I-B5 forced-GC suite.
- Week M08 W1 tranche: Phase 21 Rust-binding gate finalized ([docs/gates/phase-21.md](docs/gates/phase-21.md)) — safe lifetimes, encoded `!Send + !Sync` guards, the trybuild compile-fail set, and a `cargo-semver-checks` baseline (202/202 checks vs the SDK's introduction revision); Rust SDK polish adds the owned sequential read `Consumer::read_next` + `ReadOutcome` (REQ-BIND-002) with doc-tests and hazard-protocol borrow-lifecycle docs; Python binding start — PyO3 native core `drm-python` ([bindings/python/native](bindings/python/native), new workspace member, layer `bindings`) and pure facade [bindings/python/drm](bindings/python/drm) expose owned reads only (`Copy.bytes()`/`Copy.numpy()`), context managers, and `ContractError` for undeclared topics at first use (I-B5/I-B7, REQ-BIND-014/015/022) via a maturin build; `udp_pair` reports loopback throughput as the router perf reference.
- Week M07 W4 tranche: Phase 20 C++ gate finalized ([docs/gates/phase-20.md](docs/gates/phase-20.md)); a self-contained decode-only C++ golden reader ([bindings/cpp/tests/golden](bindings/cpp/tests/golden)) verifies **35/35** golden vectors (24 primitives + 10 composites + descriptor) under g++ 13.3 and clang++ 18.1 — the 4th independent decode path, closing the Phase 4 C++ deferral (REQ-BIND-013); [phase-04.md](docs/gates/phase-04.md) updated to PASS.
- Week M07 W3 tranche: safe Rust SDK crate `drm` ([bindings/rust/crates/drm](bindings/rust/crates/drm), new workspace member, layer `bindings`) — `BorrowGuard<'chan>` with a lifetime-bound `payload(&self) -> &[u8]`, `!Send + !Sync` via `PhantomData<*mut ()>`, RAII `Drop` release, and a trybuild compile-fail set (payload outliving guard, moved-from guard, Send/Sync violation); `xtask` layering map extended so the new crate's layer is enforced; Phase 20 gate draft.
- Week M07 W2 tranche: C++ binding ([bindings/cpp/include/drm/drm.hpp](bindings/cpp/include/drm/drm.hpp)) — header-only move-only RAII `Borrow<T>`/`Context`/`Publisher`/`Consumer` with a noexcept destructor that releases on every scope exit (incl. exception unwinding); assert-based RAII driver (normal/early-return/exception/move), a two-TU ODR link test, compile-fail fixtures for move-only/undeclared-op/pointer-escape, and a `run.sh` that compiles and runs under g++ 13.3 and clang++ 18.1 (also links against the real `drm-ffi` cdylib).
- Week M07 W1 tranche: Phase 19 memory/precommit gate ([docs/gates/phase-19.md](docs/gates/phase-19.md)); C ABI completion — close symmetry, the full negative/edge matrix (undeclared topic ⇒ `Permission`, empty-channel borrow ⇒ `NoSample`, oversize, double-release, null pointers), and a fail-stop policy test (release `panic = "abort"`, exports return `Status` on bad non-null args; no `catch_unwind`, per I-A6); `drm-core` counter surface (`ChannelCounters`, `counters` test) and a `precommit_e2e` (activation + refusal re-checked after an identity change).
- Week M06 W4 tranche: Phase 18 deployment-identity gate finalized ([docs/gates/phase-18.md](docs/gates/phase-18.md)); precommit activation (`drm-core::channel::precommit`) — content-preserving pre-fault write-touch then `mlock` with a `PrecommitProof` token and a refusal path that never leaves an active (pageable) channel (real-mlock success + injected quota refusal covered); memory-budget verifier (`drm-workspace::budget`) — worst-case arithmetic, 5 byte-exact reference fixtures, and explicit error/warning verdicts (lockable/budget exceeded, pool invariant, RAM and drop-free warnings).
- Week M06 W3 tranche: deployment-identity e2e (`drm-core` `identity_e2e`) — an unrelated contract change leaves an unaffected node's fingerprint byte-identical, an own-contract change is refused before any SHM touch, and two live deployments clean per the decision tree with zero cross-namespace attachment; FFI parity sweep — `ffi_matrix` (Rust + C-ABI paths, borrow round-trip, read ordering), `ffi_negative` (null/oversize/too-small/double-release/foreign-token/lease-exhaustion ⇒ explicit Status), a `cdylib` target and a test-only Python `ctypes` harness that runs green against `libdrm_ffi.so`; Phase 18 gate draft.
- Week M06 W2 tranche: Phase 17 publisher-lifecycle gate ([docs/gates/phase-17.md](docs/gates/phase-17.md)) — exactly-one-authority race, kernel lock release on kill, epoch exactly-once ×100, run-state reset, no cross-epoch reads, and a consumer read-loop e2e that observes restart; real in-process C ABI endpoints in `drm-ffi` (context/publisher/consumer open/close, copy publish, copy latest/next with Gap out-param, borrow/release with explicit release and double-release detection) with validate-once-at-open, C↔Rust byte parity, and the matching `drm.h` export declarations.
- Week M06 W1 tranche: Phase 16 dead-reader-recovery gate ([docs/gates/phase-16.md](docs/gates/phase-16.md)) — reclaim kill matrix (5 stages), PID-reuse guard, trigger-only reclamation, delayed-not-corrupt and identity-across-restart regressions, plus the `reclaim_e2e` stress harness (exact counters, 0 I-HZ1 violations); C ABI surface start (`drm-ffi`, `bindings/c/include/drm/drm.h`) — opaque `drm_context`/`drm_publisher`/`drm_consumer` handles, `drm_status` mirror extended with the append-only `NotImplemented` code (-22), no-unwind exports, Rust null-arg/status-propagation tests, and the C spike verified under gcc 13.3 and clang 18.1.
- Week M05 W4 tranche: Phase 15 exhaustion gate finalized ([docs/gates/phase-15.md](docs/gates/phase-15.md)) — hold-every-slot drop, latest preserved, exact counter, instant resume, proof-gated reclamation; `drm-core::deploy` deployment-identity start (fingerprint guard refuses before any SHM touch, deployment-namespaced segment names, `clean` decision tree with `ActiveUntouched`/`Cleaned`); `drm-router::deployment` namespace helper reusing `drm_core::deploy::DeploymentId` with cross-deployment isolation tests.
- Week M05 W3 tranche: publisher lifecycle coverage (`drm-core::channel::lifecycle`) — exact fresh epoch/init, authority race preservation, restart ×100 epoch/floor continuity, run-state reset, consumer epoch reset; bounded COBS+CRC32C best-effort serial codec (`drm-router::transport::serial`) with malformed/CRC rejection, 10k deterministic roundtrip, and byte-level MCU/host vectors; Phase 15 gate draft (`docs/gates/phase-15.md`).
- Week M05 W2 tranche: Phase 14 hazard gate finalized ([docs/gates/phase-14.md](docs/gates/phase-14.md)) — R1–R6 forced, ABA across ≥ 2¹⁶ reuses, selection-vs-borrows property, storm scaffold at reduced cycles; dead-reader reclamation slow path in `drm-core::channel::reclaim` (trigger-gated `try_reclaim`, `identity_alive` start-ticks incarnation guard, per-consumer registration lock + under-lock re-read, `HAZARD_NONE` CAS reclaim) with the reclaim kill matrix and PID-reuse guard.
- Week M05 W1 tranche: pool-exhaustion suites in `drm-core::channel` (`exhaustion_drop_and_counter`, `exhaustion_latest_preserved`, `exhaustion_resume_on_release`, `exhaustion_counter_exact`); priority-saturation and mixed-class two-peer lane tests in `drm-router`.
- Week M04 W4 tranche: hazard race-matrix forcing tests R1–R6 (roll-before-intent, roll-after-intent, generation-wrap, concurrent-acquire over a `Barrier`, release-during-scan, reuse-during-reread), the ABA harness (≥ 2¹⁶ slot reuses, stale pair never validates) and the selection-vs-borrows property in `drm-core::hazard`; per-peer egress classification and priority wiring in `drm-router::egress`; Phase 14 gate draft ([docs/gates/phase-14.md](docs/gates/phase-14.md)).
- Week M04 W3 tranche: Phase 13 payload-pool gate ([docs/gates/phase-13.md](docs/gates/phase-13.md)) — max/empty/intermediate sizes, slot offsets never relocate, per-topic isolation, oversize rejected before overwrite; hazard slot + acquisition + selection suites in `drm-core::hazard` (NONE sentinel, set/clear CAS, newest-eligible acquire, lookup-and-busy, intent-store `Retry`, selection property with H synthetic borrows) plus the ignored `hazard_storm` continuous-check scaffold (`DRM_STRESS_CYCLES`-bounded, 1 publisher × H readers, shadow validator); per-peer lane socket connect/disconnect lifecycle in `drm-router::lane`.
- Week M04 W2 tranche: Phase 12 overflow gate ([docs/gates/phase-12.md](docs/gates/phase-12.md)) — overwrite exact gaps, `reject` never blocks, required-consumer gating exact, `Reject` with zero required rejected; named payload-pool suites (I-P1 offsets static, I-P2 per-topic isolation, I-P3 reject-before-write with sentinel bytes); TCP reconnect with a deterministic bounded backoff schedule and session-scoped reliability — a reconnect exposes the boundary gap and never stitches (`drm-router::transport::session`).
- Week M04 W1 tranche: Phase 11 cursor gate ([docs/gates/phase-11.md](docs/gates/phase-11.md)) — independence, exact GAP, newest `latest`, no-skip property; multi-lap overwrite fixtures (laps 1/2/R/R+1) and the overflow combination matrix in `drm-core::channel`; TCP stream transport adapter with partial-write robustness, length-framed decode (one record = one sample, no reassembly) and malformed/oversized-length rejection (`drm-router::transport::tcp`).
- Week M03 W4 tranche: Phase 6 handle gate ([docs/gates/phase-06.md](docs/gates/phase-06.md)) and Phase 10 ring gate ([docs/gates/phase-10.md](docs/gates/phase-10.md)) with criteria ⇄ evidence tables and I-H1–I-H7 / I1–I8 invariant coverage; GAP delivery in `drm-core::channel` — lap-exact fixtures extended to k ∈ 1..=32, a lapped-reader independence case, and a `next_no_skip` property over an independently derived delivery model (I-C1/I-C2/I-C4, plus `latest_newest`/`next_resume_oldest`); `drm ids` report + `drm ids compact --yes` (explicit-only, I-H4) and the `drm assign` manifest-mutation CLI; transport-capability registry (unknown mapping ⇒ startup config error) and per-peer `PeerLanes` realtime+bulk lanes with static caps, FIFO-within-class and strict class service order.
- Week M03 W3 tranche: Phase 9 structured-channel gate (`docs/gates/phase-09.md`) — 3 × 10⁹ publish/read cycles over one publisher per depth (1/3/16) with four copy readers at 0 torn / 0 mixed-generation / 0 invalid / 0 monotonic violations; exhaustive depth 1..=64 plus sampled 65..4096 retention-window tests and a nightly pseudo-random sweep to 8192; pre-stream priority class queues (`drm-router::priority`, strict class order + per-class FIFO, static caps); bounded drop-newest UDP receive queue with drop counter.
- Week M03 W1–W2 tranche: Phase 5 metadata-layering gate (`docs/gates/phase-05.md`) with the derived `docs/spec/metadata-layout.md` and transport/language conformance cell scaffold; `TransportAdapter` + `Reliability` v1 with the non-blocking UDP implementation reporting best-effort and a framed round-trip; patterned-payload integrity scheme (`drm-core::channel::pattern`) classifying torn / mixed-generation / invalid without a CRC, with a permanent byte-flip fault-injection test; structured-channel stress harness (one publisher per depth 1/3/16, four copy readers, continuous monotonic check) run to 3e9 cycles at 0 torn / 0 mixed / 0 invalid / 0 monotonic violations; ring coherence, release-order, layout-golden, and depth 1..=64 retention tests.
- Week-4 tranche (work-plan M02W4): model topology (computes, links, placement with referential validation) carried into the model and roundtrip; route override pipeline completed to the 8-fixture matrix (deny prune, force pin with viability/conflict errors, prefer tie-break); `publisher_open`/`consumer_open` lifecycle over the SHM backends (held-or-refuse authority, create-or-classify with `Init`/undersized recovery, floor+epoch restart published last, `EpochExhausted`; consumer open-only with bounded startup wait) with in-memory and real-POSIX fork evidence, plus the state-first attach validation fix per channel-metadata I-M6; canonical `Gap` outcome scaffold with the one-to-one status mapping; byte-exact descriptor/CCB/prefix offset maps and sample metadata rules in the channel-metadata spec; `xtask soak` duration-bounded stability skeleton (118 clean iterations over a 1 h budget); Phase 4 gate evidence (`docs/gates/phase-04.md`) with the C++ runner closure recorded for Phase 20.
- Week-3 tranche (work-plan M02W3): clean-room BLAKE3 in the zero-dependency types crate, verified against all 35 official test vectors, with the schema hash pinned to BLAKE3/128 and the Rust ABI-version mirror of the handwritten `DRM_ABI_VERSION`; channel-descriptor attach validation (compare-once, exact-layout matrix, init refusal, byte-exact descriptor golden with zero-padding sweep and read/write symmetry), publish-ordering two-thread smoke; node manifests (TOML, closed surface) with unknown-topic and duplicate-identity negatives, multi-publisher query, contract fingerprints over pinned serialization (isolation proven), canonical layout hashes, and a 5 000-case model roundtrip; router configuration parsing, handle-keyed route table, CRC-32C known-answer vectors, serial frame draft.

### Fixed

- The `platform::cleanup` flock test flaked under parallel runs (same class as the lib `shm::lock` flake): a concurrent `Command::spawn` (fork+exec) child inherits the lock descriptors and keeps a released `flock` alive until `exec`. `spawn_child` now takes a process-wide guard shared with the flock assertions (0/30 failures after; stale `/dev/shm` segments cleared).
- `drm_consumer_close` now returns `Busy` when an outstanding borrow exists (consumer-registration I-CR4) and leaves the endpoint claimed, instead of releasing the borrow and reporting `Ok` — closing a doc/impl divergence.
- `shm::lock` semantic tests were flaky (~10–30% under parallel runs): a fork/exec child inherits the lock descriptors (`O_CLOEXEC` covers only `exec`, not `fork`), which kept a peer test's `flock` alive after the parent closed its fds. A process-wide test guard now serializes fork/spawn tests against the flock assertions (AGENTS.md 14).
- Router framing decode no longer relies on the never-type `?` special case (`map_err(|_| unreachable!())?` → `unwrap_or_else`), which the first `From<Gap> for Status` implementation exposed.

## [0.4.0]

### Added

- I6 layout crosscheck — `emit_layout_json` toolchain artifact for all 10 golden composites with regen gate, plus the permanent `drm-core` consumer test (independent runtime re-derivation, half-max/full-max codec writes, structural validation, short-buffer/over-capacity probes).
- Handle assignment (`HandleTable`/`assign`/`compact_repack`) with determinism ×100, add-no-renumber, delete/readd tombstones, typed exhaustion at the u32 boundary.
- Record framing (length-prefixed one-sample-per-record with truncation-rejection matrix) and datagram multiplex framing (exact-fit/one-over bound guard, no-partial-write) with a 20 000-case property sweep and a libfuzzer framing target.
- Model topic attributes (`@history`/`@pool_size`/`@overflow` normalized into `TopicAttrs`).

## [0.3.0]

### Added

- Phase 8 gate evidence (`docs/gates/phase-08.md` — criteria ⇄ test mapping over the shm backend, multiprocess kill matrix `shm_kill` 8 scenarios × `DRM_STRESS_ITERS`, fd-leak guards over 10k create/open cycles, `/dev/shm` quota edge with ENOSPC ⇒ explicit error and clean namespace, full-extent reservation proof; stress lane via `cargo xtask test --stress`, `tests/stress/README.md`).
- `xtask audit-traceability` citation enforcement now CI-blocking with a red→green self-test.
- `drm-workspace::model` start slice — topics parse→model, sorted discovery, conflict collection, schema content hash, deterministic render with ×100 shuffled-fs-order byte-identical gates (REQ-TOOL-045/047).
- `drm-workspace::topology` route derivation — per-topic forwarding trees, unique/ambiguous/no-route classification via BFS + bounded alternate-path check (no simple-path enumeration), deny-prune + force∧deny conflict validation, external endpoints as first-class vertices, export_limit carried into plan egress, 12 fixtures (REQ-ROUTE-004/010/003/005/009/015/019).
- Golden composites extended to 10 vectors with 3-lang parity; canonical type-universe property harness (10 000 cases); router-core UDP transport notes (recvmmsg batching, SO_RCVBUF sizing, two-host sim plan).

## [0.2.0]

### Added

- Schema type system — two-region layout computation (`TypeLayout`/`StructLayout`, checked u64 arithmetic, u32::MAX sample cap), interpreter codec over parsed types (two-phase encode with gap-zeroing determinism, structural decode re-deriving canonical packing), 48-cell boundary matrix with completeness gate, unbounded-structure rejection (7/7 with exact diagnostics), six hand-authored two-region golden composite vectors with three independent decoders (Rust frozen-oracle runner, C11 two-region reader, Python struct reader — 3/3 langs, primitives 24/24 + composites 6/6), ABI spike probe (exhaustive status switch, opaque-handle pattern, ABI-version guard; gcc+clang × c99+c11), MTU edge matrix (68/1280/1492/1472 + below-floor override rejection), `canon_le_assert` LE byte-oracle guard for aarch64 parity, budget static/activation split + dual-derivation worked example, serial escape/stuffing options table (COBS selected), and the metrics ledger (`docs/metrics/metrics-m01.md`).

## [0.1.0]

### Added

- Canonical codec read/write primitives with bounds-first errors and a 24-vector frozen primitive suite (`xtask golden-gen`, idempotent; `tests/fixtures/golden/primitives`) with three decoders — Rust canon runner (`drm-core/tests/golden_primitives.rs`), independent C11 reader (`bindings/c/tests/golden/golden_read.c`, 24/24 on gcc+clang), conformance matrix in `tests/conformance/README.md`.
- Schema parser (closed 21-reason diagnostics vocabulary, `docs/spec/DIAGNOSTICS.md`, grammar `docs/spec/schema-grammar.md`) with a 60-fixture corpus (30 valid / 30 invalid, byte-exact via `.gitattributes`), 10k-case AST-bounded round-trip proptest, 42-seed fuzz corpus, and an in-process mutation campaign (`tests/fuzz_campaign.rs`, `--ignored`, evidence in `docs/metrics/fuzz-campaign.log`).
- Linux SHM bodies (`PosixShm` create/open/rollback/precommit via mlock, `FlockBackend` LOCK_EX|LOCK_NB, `ProcessIdentity` diagnostic-only) cross-target checked, plus the platform conformance suite (`open_semantics`, `same_channel`, `race_startup` 8×100 multiprocess, `cleanup`) — cfg(unix), CI-gated.
- Router `max_datagram_payload`/`auto_suggest` with IPv4 68 / IPv6 1280 floors, 28-byte LE wire header codec with CRC32C (Castagnoli) trailer, wrap-free ingress ordering (I-RT9), record-bound check (I-RT18), and the token-bucket seed (`lanes-shaping.md` §Token Bucket, bit·ns fixed point) with a 10k-trace shaping property.
- `drm.h` v0.1 handle-validity rules + `docs/spec/abi-policy.md` (append-only, layout change = version bump) verified syntax-only under c99/c11 × gcc/clang; exhaustive drm.h↔`Status` 1:1 map gate in drm-ffi (wildcard-free compile-time check).
- Counting `GlobalAlloc` harness (`test-count-allocs` feature, `harness=false` gate asserting the codec block is 0/0 allocations) and a random-byte read-fuzz no-panic gate (canon I5).
- CI: blocking corpus-matrix and conformance jobs, fuzz PR smoke (120 s) + nightly (3600 s) budgets, sanitizer suite (ASan platform + Miri) wiring.

### Changed

- SHM platform traits frozen v1 (`drm-core::shm::{ShmBackend, LockBackend}`): method sets and semantics are stable from this point; any breaking change requires a drm-core version bump. Linux bodies (`PosixShm`, `FlockBackend`) are the tagged reference implementation; Windows contract lives in `windows-backends.md` for M06.
- Toolchain pin bumped 1.89.0 → 1.94.0 (`rust-toolchain.toml`): the 1.89.0 rustfmt tarball is quarantined by Microsoft Defender on dev hosts (false positive); functionally equivalent for edition 2024.

## [0.0.1]

### Added

- Cargo workspace (`drm-types`, `drm-core`, `drm-workspace`, `drm-router`, `drm-cli`, `drm-ffi`, `xtask`) with layer gates and workspace lints; `drm-types` status taxonomy mirrored 1:1 to the drm.h C codes (REQ-BIND-007/017); `drm-core::shm` platform trait skeletons verbatim from `shm-backend.md` §Data Structures (REQ-CORE-072/088/098/073/069/070/090/036); Linux SHM spike example (round-trip, unlink+re-create, EEXIST race ×100; REQ-CORE-072/071); `xtask` `fmt/lint/test/check-layering/audit-traceability` (REQ-QAL-003/006); `ci.yml` three-runner matrix job with sccache; fuzz skeleton `tests/fuzz`; handwritten `bindings/c/include/drm/drm.h` skeleton + ADR 0002; ADR 0003 xtask command registry; `docs/spec/routing-derivation.md` + `docs/spec/transport-survey.md` seeds (E3); ambiguity report `docs/req/ambiguity-01.md` (E2).
- Engineering standards and monorepo layout specification (`codebase.md`).
- Governance scaffolding: licenses, contribution and security policy, ownership map, CI workflow stubs, requirement registry and ADR directories.

### Changed

- Pre-implementation documentation reconciliation (no implementation yet). Directives: AGENTS gains traceability, falsification/ADR, layering, generated-code, and regression-permanence rules; R1 names all three normative architecture docs. Layout: `docs/spec/` derived-spec home, `tests/platform/` suite, `xtask/README.md` command registry, and `drm-core/src/channel/` added to `codebase.md`; `references/` and `docs/work-plan/` restated as local-only/untracked. Registry: area enum completed (`OPS|QAL`), column semantics documented, phase-resolution rule fixed in `migrate-registry.ps1`, and duplicate `REQ-CORE-052` split to `REQ-CORE-100`. ADRs disambiguated (0002 drm.h, 0003 xtask registry, 0004 ids compaction). Algorithm docs: missing Phase declarations added, `I-R` prefix collision resolved to `I-RPC*`, `V-SPATIAL`→`V-SPAT`, and invariant→Test-Matrix rows completed. Docs: `work-plan/README.md` algorithm count 34→36; architecture C++ edition fixed to C++20; `codebase.md` license allow-list (adds Zlib), ownership map, and audit cadence aligned; SECURITY cadence restated.
- Runtime algorithm docs revised after cross-document audit: consumer lifecycle gains `Closing` state with atomic close-vs-acquire hazard arbitration, re-entrant close, and word-is-protection semantics — a hazard word protects by generation match regardless of record state, so a `Closing` record with a live borrow stays protected (REQ-CORE-005); reclamation authority moved from process inspection to endpoint registration locks — process identity is diagnostic only (REQ-CORE-036/037/056/057); hard-RT publish path performs zero inspection syscalls, reclamation deferred via `reclaim_requested` (REQ-CORE-095); fork contract: inherited DRM objects invalid in child, atfork sanitization, `O_CLOEXEC`, no `LOCK_UN` from duplicates (REQ-CORE-099); OS object names generated from deployment ID + topic handle/hash, never canonical topic strings (REQ-CORE-088); POSIX stale-object classification before exact-size attach — crash before `ftruncate` is recoverable (REQ-CORE-098); observed epoch persisted across read calls (REQ-CORE-096); bounded read primitives `try_next`/`try_latest` with `WouldRetry` and all retry paths budgeted (REQ-CORE-097); one hazard acquisition primitive for all four read APIs; `hazard_protected_on_sc` specified as a single-pass `O(R+H)` generation-match rule; single physical `publish_busy` owner in the CCB; Windows `Init` recovery reinitializes in place under the publisher lock; session-scoped persistence restated as opportunistic.
- Router algorithm docs revised after falsification review: per-link scheduler spans all transports (REQ-ROUTE-034), route-scoped egress admission (REQ-ROUTE-035), independent fan-out branches with reliable-branch session failure (REQ-ROUTE-036/037), generated stream receive bounds (REQ-ROUTE-038), defined shaping charge + burst ≥ max frame (REQ-ROUTE-039), serial blocking bound (REQ-ROUTE-040), u64 stream epochs (REQ-ROUTE-011), bounded strict priority + DRR anti-starvation (REQ-ROUTE-020), DRM-boundary isolation wording.
- Serial/MCU: serial CRC covers e2e trailer (I-SR3), CRC guarantees restated (bursts ≤ 32 bits), MCU zero-heap enforced by preventive build barriers (REQ-CORE-087), COBS boundary golden vectors.
- Qualification: evidence-classed traceability + mutation audit (REQ-QAL-006), three soak gates (REQ-QAL-005), per-platform reproducible builds (REQ-QAL-001), fuzz/sanitizer resolution gate (REQ-QAL-007).
- Binding/ABI algorithm docs revised after boundary review: C ABI gains opaque `drm_context`, copy publish + zero-copy write-loan (`write_acquire`/`commit`/`abort`, channel-publish loan form), full read surface (`borrow_latest/next`, `copy_latest/next` with exact GAP out-params); lengths validated against structural bound before any Rust slice (REQ-BIND-005); `drm_status` fixed `int32_t` with explicit values, no C enum (REQ-BIND-017); fail-stop FFI — panic terminates the node, `Internal` status removed (REQ-BIND-020); one-way ABI compatibility policy (REQ-BIND-018); per-handle thread-safety: publisher endpoint concurrent-safe (`Busy` loser), consumer endpoint single-borrower (REQ-BIND-002); consumer multiplicity declared as manifest `borrowers = N` with activation-time record/hazard reservation, `H = Σ borrowers + diagnostic range` (REQ-BIND-010); Python owned-reads-only — buffer protocol/memoryview/NumPy views removed, hazard-free after every call (REQ-BIND-015); C++ `Borrow` is the owning accessor (no view/span escape), shared types layer vs node facade split (REQ-BIND-012); Rust guards `!Send + !Sync` via `PhantomData` with trybuild proofs (REQ-BIND-016); Python threading model: `Send + Sync` pyclass, per-`Topic` native lock, free-threaded CPython (REQ-BIND-022); out-params zeroed on every failure path; close-with-borrow aligned to `Busy`/`Closing` (I-CR4); `drm_log` added to the canonical surface (REQ-BIND-023); bounded core read primitives surface `DRM_ERR_RETRY` (REQ-CORE-097); `write_acquire` capacity is the topic fixed size for fixed-size topics (I-A10); `drm_borrow_view.len` distinguishes bulk (stored u32 header) from struct-variable (derived `sample_len`, canon-codec); Python `latest()/next()` return `None` on empty/caught-up and `GapError` on gap; verifier gains drop-free shortfall warning `pool_size < R+H+1` (I-MB11, REQ-CORE-042); loan `write_commit` validates the caller-written canonical sample before visibility (canon-codec I5); write-loan token discipline mirrors borrow tokens (0 ⇒ `InvalidArg`, stale/foreign ⇒ `InvalidBorrow`); loan/pool contract aligned to codec-owned sizing (no pool-authored length header).
- Observability-plane algorithms revised after falsification review: deadline state moved to per-consumer `ConsumerStats` (`WaitingFirst/Current/Late`) — deadlines are consumer properties, never channel properties (REQ-OPS-001); honest `timing = auto/exact/off` modes where `auto` compiles commit timestamps (`last_commit_mono_ns` + per-entry `commit_mono_ns`) only for channels with a declared deadline or jitter diagnostics (REQ-OPS-016, REQ-CORE-094); exact promises limited to latest-sample age and retained inter-arrival — no historical miss-count warranty (safety logic stays in the controller); epoch-sandwich snapshots (REQ-OPS-002); trace rings replaced by fixed per-producer SPSC lanes with build-time arg-size enforcement (REQ-OPS-014/015/017).
- Record/replay algorithms revised: per-topic slab pools replace `[u8; MAX_SAMPLE]` slots (REQ-OPS-019); separate per-topic loss ledger with coalesced ranges and `gap_detail_lost` fixes the full-buffer gap-marker contradiction (REQ-OPS-018); recorder declared never an implicit required consumer (REQ-OPS-008); tiered fidelity (payload bytes / `(epoch, seq)` order / `(recv_mono_ns, ingress_order)`) with explicit replay scheduler (REQ-OPS-009); DRM u64 sequence/epoch/ingress order in MCAP metadata records (REQ-OPS-006).
- RPC/action algorithms revised: one ordered `event/<client>` stream replaces `response/` + `state/`; reactor/worker split with cooperative cancellation (REQ-OPS-011); operation + terminal capacity reserved before a request is consumed — `publish ≠ accepted` (REQ-OPS-021); bounded outbox with guaranteed lifecycle/terminal and advisory progress (REQ-OPS-022); relative `remaining_budget_ns` on the wire (REQ-OPS-023); random server session prevents stale-request re-execution across restarts (REQ-OPS-020); reliable routes required (REQ-OPS-024); per-cid event ordering (REQ-OPS-025); `V-RPC`/`V-REC` verify checks added (verify-engine.md).
- Spatial algorithm docs revised after falsification audit: `latest()` is now temporally coherent — every path input evaluated at one common timestamp `t* = min_i(t_newest,i)`, empty interval intersection ⇒ `OUT_OF_HISTORY`, and the per-process cache reuses geometry only while freshness/status are always derived per query (REQ-SPATIAL-002/024); transform slots carry sample + receive timestamps with a per-edge clock identity/epoch, and clock mappings are affine (reference pair, rate ratio, bounded uncertainty growth) so a device reset invalidates prior mappings (REQ-SPATIAL-004/035); `predict` integrates a body-frame twist on SE(3) via the exponential map (REQ-SPATIAL-025); `history = H` is a guaranteed sample-time window, `max_samples = N` the capacity, and `min_sample_period = P` the producer-density contract — verification requires `N ≥ ⌊H/P⌋ + 2`, faster-than-P samples are rejected and counted, and rings are sample-time ordered with deterministic duplicate/late policy (REQ-SPATIAL-033/037; `REQ-SPATIAL-037` added to the registry); the result object is split into one hard error plus combinable quality flags (REQ-SPATIAL-032); runtime overlay frames resolve through a bounded, topology-generation-cached overlay prefix with bounded topology-event history (REQ-SPATIAL-008/034); reparenting marks discontinuities per jumped local edge and Preserve-World keeps the world trajectory continuous (REQ-SPATIAL-016/029); the dynamic index uses incremental affected-cell updates under a grid epoch with a single non-blocking updater and full dependency-vector invalidation (REQ-SPATIAL-013/015); the cell-hash table is sized with load-factor headroom (α ≤ 0.70) and backward-shift deletion, queries bound cell enumeration by `max_query_cells` with object-table fallback, and candidate overflow returns `OutputTooSmall{required}` instead of truncating (REQ-SPATIAL-014/018/020); `SpatialId` documented as a persistent object identity, not a slot index (REQ-SPATIAL-009).

[unreleased]: https://github.com/shakilapr/drm/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/shakilapr/drm/releases/tag/v0.4.0
[0.3.0]: https://github.com/shakilapr/drm/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/shakilapr/drm/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/shakilapr/drm/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/shakilapr/drm/releases/tag/v0.0.1
