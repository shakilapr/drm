# Metrics — Month 23 (M23)

Normative source: [work-plan.md](../../work-plan.md) W8 qualification drive and
the M23 month plan. Month 23 is the Phase 34 qualification drive: traceability,
fault injection, soak, deterministic builds, supported-platform closure,
invariant verification, and docs/examples CI. One section per weekly tranche.

## M23W1 — Qualification Kickoff: Traceability Matrix Build

| Variable | Target | Measured |
|---|---|---|
| `REQ_ID_COUNT` | extracted from source + docs | registry rows resolved by `cargo xtask audit-traceability --matrix` |
| `TRACE_MATRIX_COVERAGE` | 100% REQ-IDs mapped to tests | 100% — `xtask/src/traceability.rs` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Traceability matrix generator** (`xtask/src/traceability.rs`): builds the
  `I-QAL1` requirement → implementation → test matrix from
  [registry.csv](../req/registry.csv); a requirement with no resolvable test
  target, or a test-bearing source file with no REQ citation, is an orphan and
  fails the gate. `cargo xtask audit-traceability --matrix` reports 100% coverage
  and zero orphans (`REQ-QAL-003`, `REQ-QAL-004`, `REQ-QAL-006`).

## M23W2 — Fault Injection + Soak 3×24h

| Variable | Target | Measured |
|---|---|---|
| `FAULT_CAMPAIGNS` | serial-bomb, link-storm, subscriber-kill, publisher-crash, exhaustion-burst green | 5/5 — `cargo xtask fault` ([m23w2.md](../gates/m23w2.md)) |
| `SOAK_3X24H` | 3×24 h soak: 0 leaks, 0 crashes, bounded memory, log clean | clean — `cargo xtask soak --duration 24 --parallel 3 --memory-bound 2048` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Fault injector** (`xtask/src/fault.rs`): `cargo xtask fault` executes the
  five runtime/transport campaigns — `publisher-crash` (mid-publish teardown),
  `exhaustion-burst` (pool exhaustion under
  [exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md)
  I-E1/I-E3/I-E4), `subscriber-kill` (hazard-slot correctness,
  [hazard-protocol.md](../algorithms/runtime/hazard-protocol.md) I-HZ1/I-HZ7),
  `serial-bomb` (transport framing), and `link-storm` (router-core I-RT6) — and
  reports `FAULT_CAMPAIGNS = 5/5`; a red campaign fails the command
  (`REQ-QAL-010`, `REQ-QAL-015`). Campaign/scenario detail:
  [fault-injection.md](../spec/fault-injection.md).
- **Fault scenarios + determinism** (`Scenario`, `FaultEvent`, `PoolModel`): a
  seeded xorshift64 schedule and a stable `replay_digest`; the `PoolModel` replay
  reproduces the published/dropped/reclaimed counters from the seed, so a
  campaign failure replays from its recorded seed (`REQ-QAL-016`).
- **Soak harness 3×24 h** (`xtask/src/soak.rs`): `--parallel 3` runs three
  concurrent runs in isolated `target/soak-slot-<i>` directories; `--memory-bound
  <mb>` samples each run's peak resident set (`VmHWM`) against the ceiling
  ([memory-budget.md](../algorithms/memory-budget.md) I-MB9 included);
  `--report <path>` writes the JSON auto-report. The ledger gains `peak_rss_kb`
  and `bound_kb`, defaulted on load so an older ledger still reads
  (`REQ-QAL-017`).
- **Threshold EXP** (`sweep_threshold`): the anomaly-detector sweep picks the
  smallest counter threshold with zero clean-run false positives and reports the
  anomalous runs it detects; on the clean soak ledger the band is
  `threshold = 0, false_positives = 0` (`REQ-QAL-017`).
- **Gate evidence** ([m23w2.md](../gates/m23w2.md)); **soak ledger** format
  ([soak-ledger.md](../spec/soak-ledger.md)).

### Weekly gate checklist (chaired by E1, Friday)

- [x] `FAULT_CAMPAIGNS = 5/5`; `SOAK_3X24H = clean`
- [x] Threshold band recorded (`false_positives = 0`); metrics merged
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M23W2) = PASS`**

### How to reproduce

```text
cargo xtask fault
cargo xtask soak --duration 24 --parallel 3 --memory-bound 2048 --report target/soak-report.json
cargo xtask soak --dashboard
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

REQ: REQ-QAL-003, REQ-QAL-004, REQ-QAL-005, REQ-QAL-006, REQ-QAL-010,
REQ-QAL-013, REQ-QAL-014, REQ-QAL-015, REQ-QAL-016, REQ-QAL-017.

## M23W3 — Deterministic Builds + Docs/Examples CI

| Variable | Target | Measured |
|---|---|---|
| `REPRO_BUILDS` | two clean builds on different runners bit-identical — workspace crates + `drm.h` artifacts (I-QAL3) | protocol + declared artifact list in [qualification.md](../algorithms/program/qualification.md) §Build reproducibility |
| `DOCS_CI` | docs build + link check + examples compile green | docs evidence leg of the M23W4 gate packet (`cargo xtask qualify`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Build reproducibility protocol** (`I-QAL3`): the pinned toolchain,
  `--locked`, `SOURCE_DATE_EPOCH`, path/env normalization, the declared primary
  artifacts (`drm.h` + workspace crate outputs), and the two-independent-runner
  comparison are specified in
  [qualification.md](../algorithms/program/qualification.md) §Build
  reproducibility and consumed by the M23W4 gate packet (`repro` leg).
- **Docs/examples CI**: the docs evidence leg of the qualification packet
  (`cargo xtask qualify`); the nightly docs job is tracked with the M24W1
  coverage-closure carry-over.

## M23W4 — Perf Invariants + Qualification Gate Assembly, Month-23 Review

| Variable | Target | Measured |
|---|---|---|
| `PERF_INVARIANTS` | all bench invariants within budgets (publish/consume, exhaustion path, canon-codec, router lane, UDP/serial) | all green — [m23w4.md](../gates/m23w4.md) |
| `MATRIX_CELLS` | all declared OS×language×transport cells pass; undeclared absent (I-QAL4) | closed — `cargo xtask qualify --check` |
| `QUAL_GATE_DOC` | qualification gate packet assembled (trace + faults + soak + repro + matrix + docs + perf) | assembled — `cargo xtask qualify --check` |
| `MONTH23_REVIEW` | ON_TRACK | ON_TRACK |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Runtime bench invariants** ([latency.rs](../../crates/drm-core/benches/latency.rs)):
  publish/consume latency and zero-allocation assertions plus the
  exhaustion/backpressure (`QueueFull`) publish path; the channel footprint is
  checked against the declared budget ([memory-budget.md](../algorithms/memory-budget.md)
  I-MB1) (`REQ-CORE-040`, `REQ-QAL-012`).
- **Budget verifier wiring**: the `budget` CI job runs
  `cargo test -p drm-workspace --test budget --test spatial_memory`, making the
  I-MB1..I-MB11 terms — including the I-MB9 spatial-index memory — a required
  gate (`REQ-TOOL-003`, `REQ-TOOL-004`, `REQ-TOOL-005`, `REQ-TOOL-009`,
  `REQ-TOOL-010`, `REQ-SPATIAL-018`).
- **Transport + codec benches** ([transport.rs](../../crates/drm-router/benches/transport.rs),
  [codec.rs](../../crates/drm-core/benches/codec.rs), [router.rs](../../crates/drm-router/benches/router.rs)):
  UDP datagram loopback, serial COBS/CRC32C round trip, canon-codec throughput,
  and router lane shaping, each with bounded-time p99 ceilings
  (`REQ-QAL-011`, `REQ-QAL-012`, `REQ-QAL-019`).
- **Bench harness + gate packet** ([qualify.rs](../../xtask/src/qualify.rs)):
  `cargo xtask qualify --check --json <path>` assembles the `I-QAL1` traceability
  matrix, `I-QAL2` fault/soak evidence, `I-QAL3` repro evidence, `I-QAL4`
  conformance closure (96 declared cells, 24 core, every cell classified), docs
  CI, and `I-QAL5` perf invariants; benches are CI-required (`REQ-QAL-018`).
- **Gate evidence** ([m23w4.md](../gates/m23w4.md)).

### Weekly gate checklist (chaired by E3, Friday)

- [x] `PERF_INVARIANTS = all green`; `MATRIX_CELLS = closed (I-QAL4)`; `QUAL_GATE_DOC = assembled`
- [x] Month-23 review: ON_TRACK; M24 commitment — API freeze, pilot, v1.0
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M23W4) = PASS`**, `MONTH23_REVIEW = ON_TRACK`

### How to reproduce

```text
cargo test -p drm-workspace --test budget --test spatial_memory
taskset -c 1 cargo bench -p drm-core --bench latency --bench throughput --bench codec
taskset -c 1 cargo bench -p drm-router --bench router --bench transport
cargo xtask qualify --check --json target/qualification-packet.json
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

REQ: REQ-QAL-001, REQ-QAL-002, REQ-QAL-005, REQ-QAL-006, REQ-QAL-008,
REQ-QAL-011, REQ-QAL-012, REQ-QAL-018, REQ-QAL-019, REQ-TOOL-003,
REQ-TOOL-004, REQ-TOOL-005, REQ-TOOL-009, REQ-TOOL-010, REQ-SPATIAL-018,
REQ-CORE-040.
