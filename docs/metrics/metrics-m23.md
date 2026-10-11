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
