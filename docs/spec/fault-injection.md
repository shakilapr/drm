# Fault Injection Campaigns and Deterministic Scenarios

Normative terminology follows [docs/standards.md](../standards.md). Scope: the
M23W2 fault-injection campaigns (`FAULT_CAMPAIGNS`) and the deterministic
scenario engine that makes every campaign failure reproducible. Implements
[qualification.md](../algorithms/program/qualification.md) I-QAL2 and
`REQ-QAL-010` (chaos recovery: kill at every borrow/publish stage, PID reuse,
pool-exhaustion storms, link flapping recover every time). Runtime faults follow
[hazard-protocol.md](../algorithms/runtime/hazard-protocol.md) and
[exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md);
transport faults follow
[router-core.md](../algorithms/router/router-core.md). Owner: E1 (runtime
faults) + E2 (scenarios) + E3 (transport topology). Phase 34.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-QAL-fault-campaigns` | five fault campaigns (publisher-crash, exhaustion-burst, subscriber-kill, serial-bomb, link-storm) run green | `REQ-QAL-015` |
| `REQ-QAL-fault-deterministic-replay` | a seeded fault schedule replays to a stable digest | `REQ-QAL-016` |
| `REQ-QAL-chaos-recovery` | kill at every borrow/publish stage recovers every time | `REQ-QAL-010` |

## Harness

`cargo xtask fault` (`xtask/src/fault.rs`) executes the five campaigns in report
order and prints `FAULT_CAMPAIGNS = n/5`. A red campaign fails the command
(triage-first, [qualification.md](../algorithms/program/qualification.md)
I-QAL2). The `--json` report records the campaign verdicts plus the scenario seed
and replay digest.

| Flag | Meaning |
|---|---|
| `--campaign <name>` | run one campaign (repeatable); default is all five |
| `--seed <n>` | scenario seed (default `0`) |
| `--steps <n>` | scenario step count (default `64`) |
| `--list` | list the campaigns and their descriptions |
| `--dry-run` | print each campaign's command without running it |
| `--json <path>` | write the `FaultReport` as JSON evidence |

## Campaigns

| Campaign | Fault | Suite |
|---|---|---|
| `publisher-crash` | publisher `SIGKILL`/restart at every publish stage | `cargo test -p drm-core publisher_kill` |
| `exhaustion-burst` | pool exhaustion under the reclamation policies | `cargo test -p drm-core --test stress -- --ignored stress_chaos_exhaustion_storm` |
| `subscriber-kill` | reader kill at every hazard-borrow stage | `cargo test -p drm-core --test shm_kill -- --ignored --skip stress_shm_kill_child_driver` |
| `serial-bomb` | serial framing corruption, truncation, and resync | `cargo test -p drm-router serial` |
| `link-storm` | router link flap and reconnect under load | `cargo test -p drm-router --test link_storm -- --ignored` |

`FAULT_CAMPAIGNS = 5/5` iff every campaign's suite finishes clean. The runtime
campaigns assert the recovery contract of
[exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md)
(I-E1 never waits, I-E3 exact counter, I-E4 resume) and
[hazard-protocol.md](../algorithms/runtime/hazard-protocol.md) (I-HZ1 never
overwritten, I-HZ7 single outstanding borrow); the transport campaigns assert
[router-core.md](../algorithms/router/router-core.md) link/session isolation
(I-RT6).

## Deterministic scenarios

A [`Scenario`](../../xtask/src/fault.rs) is a seeded xorshift64 schedule of
`FaultEvent`s. The schedule is reproducible: the same seed always produces the
same event sequence, and `replay_digest` folds the schedule into a stable 64-bit
digest. A campaign failure is therefore reproducible from its recorded seed — the
`--seed`/`--steps` pair and the digest are the reproduction key.

The scenario also replays over a discrete `PoolModel` that mirrors
[exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md): a
protected slot is never reused, a full pool drops and counts (`I-E1`/`I-E3`), and
the maintenance pass reclaims lock-proven-dead slots (`I-E4`). The replay reports
`published`, `dropped`, and `reclaimed` counters plus the digest.

```text
cargo xtask fault --list
cargo xtask fault --dry-run
cargo xtask fault
cargo xtask fault --campaign link-storm --seed 42 --steps 128
cargo xtask fault --json target/fault-report.json
```

REQ: REQ-QAL-010, REQ-QAL-015, REQ-QAL-016.
