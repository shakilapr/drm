# Soak Ledger and Dashboard

Normative terminology follows [docs/standards.md](../standards.md). Scope: the
I-QAL2 soak escalation harness — the rotating-topology daily run, the committed
evidence ledger, the three-consecutive-clean gate rule, and the per-run anomaly
dashboard. Implements [qualification.md](../algorithms/program/qualification.md)
I-QAL2 (`REQ-QAL-005`, `REQ-QAL-013`, `REQ-QAL-014`). Owner: E1 (harness) + E4
(dashboard + metrics). Phase 34.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-QAL-soak-escalation` | daily 24 h runs over rotating topologies; anomaly ⇒ triage → fix → restart; three-consecutive-clean rule | `REQ-QAL-013` |
| `REQ-QAL-soak-dashboard` | per-run anomaly counters trended; evidence ledger in I-QAL2 format | `REQ-QAL-014` |
| `REQ-QAL-soak-memory-bounded` | parallel 24 h runs with a memory-boundedness ceiling and an auto-report | `REQ-QAL-017` |

## Harness

`cargo xtask soak` (`xtask/src/soak.rs`) repeats the workspace suite
(`cargo test --workspace`) for a wall-clock budget. Every iteration must finish
clean; the first failing iteration is recorded as an anomaly and the harness
exits nonzero (triage-first, [qualification.md](../algorithms/program/qualification.md)
I-QAL2). The run is appended to the ledger and the dashboard is printed.

| Flag | Meaning |
|---|---|
| `--duration <h>` | run until the `<h>`-hour budget elapses (at least one iteration) |
| `--iterations <n>` | run exactly `<n>` iterations (short verification / smoke) |
| `--topology <name>` | pin the topology (`single-machine`, `two-machine-sim`, `physical-pair`); default is rotation |
| `--parallel <n>` | run `<n>` runs concurrently, each in an isolated `target/soak-slot-<i>` (three parallel 24 h runs, M23W2) |
| `--memory-bound <mb>` | memory ceiling in MiB; each run's peak resident set is sampled and checked (`I-MB9`, M23W2) |
| `--report <path>` | write the JSON evidence report (gate, memory, threshold band, runs) |
| `--ledger <path>` | ledger path (default `docs/metrics/soak-ledger.json`) |
| `--dashboard` | render the ledger trend and exit (no run) |

Exactly one of `--duration` or `--iterations` is required unless `--dashboard`
is given.

### Topology rotation

Runs rotate through the three I-QAL2 topologies in round-robin order so that
consecutive daily runs exercise different link shapes:

| Index | Topology | Shape |
|---|---|---|
| 0 | `single-machine` | all nodes on one host, shared-memory only |
| 1 | `two-machine-sim` | two-netns simulation of a host pair over the router |
| 2 | `physical-pair` | two physical hosts joined by the router transport |

The next topology is `Topology::ALL[runs.len() % 3]`. `--topology` overrides
the rotation for a targeted re-run (for example, re-running a topology after a
fix).

## Ledger format (I-QAL2)

The ledger is a committed JSON document under `docs/metrics/soak-ledger.json`:
one record per run, append-only.

```json
{
  "runs": [
    {
      "topology": "single_machine",
      "hours": 24,
      "iterations": 1180,
      "anomalies": 0,
      "clean": true,
      "peak_rss_kb": 524288,
      "bound_kb": 2097152,
      "finished_unix_s": 1760000000
    }
  ]
}
```

- `topology` — one of `single_machine`, `two_machine_sim`, `physical_pair`.
- `hours` — the requested wall-clock budget in whole hours (`0` for an
  `--iterations` run).
- `iterations` — completed suite iterations.
- `anomalies` — failed suite iterations in this run (`0` for a clean run).
- `clean` — `true` iff `anomalies == 0`.
- `peak_rss_kb` — peak resident set of the run's child processes (KiB); `0` when
  sampling is unavailable or the field predates M23W2 (defaulted on load).
- `bound_kb` — the `--memory-bound` ceiling in KiB; `0` means unbounded (legacy).
- `finished_unix_s` — run completion time (Unix seconds), for trending.

An absent ledger is an empty ledger (no runs yet).

## Three-consecutive-clean rule

The I-QAL2 regression gate holds iff the trailing clean-run streak is
`REQUIRED_CLEAN_RUNS` (3). An anomaly resets the streak: the three-clean clock
restarts only on an anomaly, never on a topology change. The dashboard prints
the current streak and the gate verdict.

## Dashboard

`cargo xtask soak --dashboard` renders one line per run plus a summary:

```text
soak dashboard — 3 run(s), clean streak 3/3, gate PASS
run  topology         hours  iters  anomalies  clean  finished_unix_s
1    single_machine   24     1180   0          yes    1760000000
2    two_machine_sim  24     1176   0          yes    1760086400
3    physical_pair    24     1172   0          yes    1760172800
```

The anomaly counter column is the trend: any nonzero value is a triage item
carried to the owning engineer, and the gate stays red until three consecutive
clean runs follow.

## Memory-boundedness (M23W2)

`--memory-bound <mb>` samples each run's peak resident set (`VmHWM`) against the
declared ceiling; a run above its ceiling fails the command. The spatial-index
budget is covered by
[memory-budget.md](../algorithms/memory-budget.md) I-MB9, so the soak ceiling is
the runtime envelope around that static proof. A run with no ceiling
(`bound_kb == 0`) is treated as unbounded (legacy), so an older ledger still
loads.

## Anomaly threshold band (EXP, M23W2)

The `EXP` sweep (`sweep_threshold`) chooses the smallest counter threshold with
zero clean-run false positives and reports how many anomalous runs it still
detects. On a clean ledger every counter is zero, so the band is
`threshold = 0, false_positives = 0`; the dashboard prints the band after the
per-run rows. The counters are `corruption`, `deadlock`, and the generic
`anomaly` counter.

## Reproduce

```text
cargo xtask soak --iterations 1 --topology single-machine
cargo xtask soak --duration 24
cargo xtask soak --duration 24 --parallel 3 --memory-bound 2048
cargo xtask soak --iterations 1 --report target/soak-report.json
cargo xtask soak --dashboard
```

REQ: REQ-QAL-005, REQ-QAL-013, REQ-QAL-014, REQ-QAL-017.
