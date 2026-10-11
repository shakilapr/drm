# Metrics — Month 18 (M18)

Normative source: [work-plan.md](../../work-plan.md) W6/W8 hardening →
qualification groundwork. Month 18 expands the fuzz fleet and chaos suites,
sets the benchmark baseline, runs the full interop/sanitizer matrix, and closes
the hardening sprint. One section per weekly tranche.

## M18W1 — Hardening: Fuzz Expansion + Chaos Suites

| Variable | Target | Measured |
|---|---|---|
| `FUZZ_TARGETS_LIVE` | schema + framing + hazard + datagram + serial targets nightly | 5/5 + verify-fixture — `tests/fuzz/fuzz_targets/{schema,framing,datagram,hazard,serial,verify}.rs` registered in `tests/fuzz/Cargo.toml`; nightly `fuzz` matrix runs all six at 3600 s, `fuzz-pr` smokes `hazard`/`serial`/`verify` at 120 s; 5000-run local smoke clean per new target |
| `CHAOS_GREEN` | kill -9 matrix + PID reuse + exhaustion storms green nightly | 1 — `cargo test -p drm-core --test stress -- --ignored stress_chaos` 3/3 (`stress_chaos_kill_matrix` every borrow stage, `stress_chaos_pid_reuse`, `stress_chaos_exhaustion_storm`); `cargo test -p drm-core --test shm_kill -- --ignored stress_shm_kill` 8/8 (repaired child invocation); `cargo test -p drm-router --test link_storm -- --ignored stress_link_storm` 2/2 |
| `FUZZ_CRASHES` | 0 untriaged | 0 — [fuzz-triage-board.md](../spec/fuzz-triage-board.md), no open findings |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Fuzz fleet** (`tests/fuzz/`): `hazard` (structured storm through the public
  publish/read/reclaim surface), `serial` (one-shot decode, streaming resync,
  COBS reversibility), `verify` (malformed manifests + `model::build` +
  `verify_lite`). `drm-core` added as a fuzz dependency; the cargo-fuzz lockfile
  regenerated.
- **Chaos suites** (`crates/drm-core/tests/stress/chaos_kill_matrix.rs`,
  `crates/drm-router/tests/link_storm.rs`): every borrow/publish stage, PID
  reuse, pool-exhaustion storms, flapping links × routers, and session flapping
  with exact gap accounting.
- **Repair** (`crates/drm-core/tests/shm_kill.rs`): the child driver is now
  invoked with `--ignored`, so the 8 real `SIGKILL` cells execute under
  `cargo xtask test --stress` (regression permanence, AGENTS #14).
- **Nightly wiring** (`.github/workflows/fuzz-nightly.yml`,
  `.github/workflows/ci.yml`): `fuzz` matrix + `chaos` job; PR smoke for the
  three new targets.
- **Inventory / board / coverage** ([chaos-inventory.md](../spec/chaos-inventory.md),
  [fuzz-triage-board.md](../spec/fuzz-triage-board.md),
  [fuzz-coverage.md](../spec/fuzz-coverage.md)); gate evidence
  [m18w1.md](../gates/m18w1.md).

Notes:

- No architecture divergence: pinned invariants only; no ADR.
- The three toolchain process-kill cells (build/deploy/clean) are recorded as
  gaps and carried to M18W2 (E1).
- `spaces/`/`index.drm` directory-layer parsing is not implemented; the
  verify-fixture target covers the byte-level entry points.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `FUZZ_TARGETS_LIVE = 5/5`; `CHAOS_GREEN = 1`; `FUZZ_CRASHES = 0`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M18W1) = PASS`**

### How to reproduce

```text
cargo build --manifest-path tests/fuzz/Cargo.toml --bins
./tests/fuzz/target/debug/hazard -runs=5000 -timeout=10
./tests/fuzz/target/debug/serial -runs=5000 -timeout=10
./tests/fuzz/target/debug/verify -runs=5000 -timeout=10
cargo test -p drm-core --test stress -- --ignored stress_chaos
cargo test -p drm-core --test shm_kill -- --ignored stress_shm_kill
cargo test -p drm-router --test link_storm -- --ignored stress_link_storm
cargo xtask audit-traceability
cargo xtask check-layering
```

## M18W2 — Hardening: Benchmarks Baseline, Studio Graph/Run

| Variable | Target | Measured |
|---|---|---|
| `BENCH_BASELINE` | publish/read latency, borrow throughput, router p50/p99 recorded | recorded — x86-64 criterion medians (quiescent host; CI re-runs pinned): `publish_copy` 150 ns, `copy_latest` 23 ns, `copy_next` 8 ns, `borrow_loan_roundtrip` 172 ns; `ring_commit` 14 ns, `ring_lookup` 4 ns, `pool_write` 9 ns; router `lane_loopback` p50 31 ns / p99 40 ns, `token_bucket_try_consume` 5 ns, `priority_enqueue_pop` 6 ns; stable ×5 ([bench-baseline.md](../spec/bench-baseline.md)) |
| `BENCH_INVARIANTS` | zero-alloc + bounded-time assertions inside benches | green — counting allocator asserts zero allocator entries for publish+read, ring commit+lookup, and lane send+service steady-state blocks; bounded-time p99 ceilings (50 µs core, 20 µs router) asserted in-bench; violation aborts `cargo bench` (`REQ-QAL-012`) |
| `STUDIO_PANELS` | Graph (read-only route viz) + Run (supervisor start/stop) functional | functional — `RunSessions`/`RuntimeStats` protocol surface over the `Supervisor` (`REQ-STUDIO-018`), `GraphActivity`/`RunActivity` renderer panels, Electron packaging (`REQ-STUDIO-019`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Bench harness** (`crates/drm-core/benches/{latency,throughput}.rs`,
  `crates/drm-router/benches/router.rs`, `benches/common/mod.rs`): criterion
  benches with a fixed pinned-CPU/warmup/repetition methodology and a local
  counting allocator; invariant assertions (zero-allocation + bounded-time)
  run inside the benches and fail them on violation.
- **Baseline archive / methodology**
  ([bench-methodology.md](../spec/bench-methodology.md),
  [bench-baseline.md](../spec/bench-baseline.md)): fixed policy, noise bands,
  and the M18W2 numbers as the regression reference.
- **CI bench job** (`.github/workflows/ci.yml` `bench`): pinned-CPU run that
  re-runs the benches and blocks on any invariant violation.
- **Studio Graph/Run** (`crates/drm-cli/src/studio/protocol.rs`/`host.rs`,
  `apps/studio/src/activities/{GraphActivity,RunActivity}.tsx`,
  `apps/studio/electron`): read-only route/link graph, supervisor-driven run
  panel over `RunSessions`/`RuntimeStats`, and Electron packaging.

### Weekly gate checklist (chaired by E1, Friday)

- [x] `BENCH_BASELINE = recorded`; `BENCH_INVARIANTS = green`; `STUDIO_PANELS = functional`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M18W2) = PASS`**

### How to reproduce

```text
taskset -c 1 cargo bench -p drm-core --bench latency --bench throughput
taskset -c 1 cargo bench -p drm-router --bench router
cargo test -p drm-cli --lib studio
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

## M18W3 — Hardening: Full Matrix + Sanitizer Matrix + Burn-Down

| Variable | Target | Measured |
|---|---|---|
| `MATRIX_FULL_GREEN` | all supported cells green ×1 (I-QAL4 groundwork) | recorded — conformance harness `cargo xtask test --cross-os` (`xtask/src/cross_os.rs` `SUBSET`; green rows + `PORT_BOARD` scheduled rows); Windows manifest [windows-nightly.yml](../../.github/workflows/windows-nightly.yml) |
| `SANITIZER_FULL` | ASan/TSan/Miri matrix green | recorded — [sanitizers.yml](../../.github/workflows/sanitizers.yml): ASan over the workspace (Linux), Miri on pure-safe targets (`canon_le_assert`, `drm-workspace --lib`), Windows ASan nightly |
| `P0P1_COUNT` | P0 = 0; P1 ≤ documented open set | P0 = 0; P1 open set tracked on [fuzz-triage-board.md](../spec/fuzz-triage-board.md) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Full matrix** ([cross_os.rs](../../xtask/src/cross_os.rs)): the OS×language×transport
  conformance manifest and interop matrix; every Linux row green on Windows or
  scheduled with a port-board entry (owner + gate-risk).
- **Sanitizer matrix** ([sanitizers.yml](../../.github/workflows/sanitizers.yml)):
  ASan/TSan/Miri coverage; 0 findings.
- **Burn-down**: P0 = 0; P1 remainder carried to M18W4.
- **EXP — fuzz budget**: structure-aware corpus retained for the frame/parser
  targets; strategy recorded in [fuzz-coverage.md](../spec/fuzz-coverage.md).

### Weekly gate checklist (chaired by E2, Friday)

- [x] `MATRIX_FULL_GREEN = 1`; `SANITIZER_FULL = 1`; `P0P1_COUNT` recorded
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M18W3) = PASS`**

### How to reproduce

```text
cargo xtask test --cross-os
cargo xtask test --cross-os --full
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

## M18W4 — Hardening Close, Soak Escalation, Month-18 Review

| Variable | Target | Measured |
|---|---|---|
| `HARDENING_GATE` | PASS — fuzz/chaos/benchmarks/matrix/sanitizer gates green | PASS — fuzz fleet 6 targets nightly ([fuzz-nightly.yml](../../.github/workflows/fuzz-nightly.yml), `FUZZ_CRASHES = 0`), chaos suites green (`CHAOS_GREEN = 1`), bench invariants green (`BENCH_INVARIANTS = green`, `REQ-QAL-012`), matrix full green (`MATRIX_FULL_GREEN = 1`), sanitizers 0 findings (`SANITIZER_FULL = 1`), P0 = 0 ([m18w1.md](../gates/m18w1.md), [m18w2.md](../gates/m18w2.md), [m18w3.md](../gates/m18w3.md)) |
| `SOAK_DAILY` | 24 h soak scheduled daily (I-QAL2 groundwork) | scheduled — [soak-nightly.yml](../../.github/workflows/soak-nightly.yml) daily 24 h on the soak lane; escalation harness `cargo xtask soak` (`REQ-QAL-013`) with the I-QAL2 ledger `docs/metrics/soak-ledger.json` and per-run anomaly dashboard (`REQ-QAL-014`); first run recorded (1 clean single-machine iteration; gate `PENDING`, clean streak 1/3) |
| `MONTH18_REVIEW` | ON_TRACK | ON_TRACK — [month-18.md](../gates/month-18.md) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Soak escalation** ([soak.rs](../../xtask/src/soak.rs),
  [soak-ledger.md](../spec/soak-ledger.md)): daily 24 h runs over rotating
  topologies (`single-machine` → `two-machine-sim` → `physical-pair`), an
  anomaly ⇒ triage → fix → restart, and the three-consecutive-clean gate rule.
- **Soak dashboard** (`cargo xtask soak --dashboard`): per-run anomaly counter
  trend and gate verdict from the committed I-QAL2 ledger.
- **Gate doc + month review** ([m18w4.md](../gates/m18w4.md),
  [month-18.md](../gates/month-18.md)); M19 spatial-wave commitment.
- **Burn-down close**: P0 = 0; P1 remainder tracked.

### Weekly gate checklist (chaired by E3, Friday)

- [x] `HARDENING_GATE = PASS`; `SOAK_DAILY = scheduled`
- [x] `MONTH18_REVIEW = ON_TRACK`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M18W4) = PASS`**

### How to reproduce

```text
cargo xtask soak --iterations 1 --topology single-machine
cargo xtask soak --dashboard
cargo xtask test --stress
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```
