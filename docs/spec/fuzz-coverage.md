# Fuzz Coverage Report (M18W1)

Work-plan [M18W1-E4-T2](../../docs/work-plan/work-plan-m18-w1.md): the fuzz and
chaos suites cover the recovery loops — every acquisition, retirement, reclaim,
and resync path that can be interrupted is driven by a target or a chaos cell.
Coverage is reported per target against the algorithm doc that owns the loop.

## Target → recovery loop

| Target / suite | Recovery loop exercised | Owning doc |
|---|---|---|
| `hazard` | acquisition (protect/recheck/abort), publish drop + `reclaim_requested`, `try_reclaim`, `restart_run` stale-word clear | [hazard-protocol.md](../algorithms/runtime/hazard-protocol.md), [exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md) |
| `stress_chaos_kill_matrix` | every borrow stage → reclaim under lock authority → resume; restart stale-word clear | exhaustion-reclamation.md §Protocols |
| `stress_chaos_exhaustion_storm` | all-slots-protected drop storm → exact counter → maintenance reclaim → resume | exhaustion-reclamation.md I-E1..I-E4 |
| `stress_shm_kill_*` | create/write/hold/unlink/stale → leftover classification → re-create | [shm-backend.md](../algorithms/runtime/shm-backend.md) |
| `serial` | streaming resync at `0x00`, COBS escape reversibility, CRC rejection | [serial-mcu.md](../algorithms/router/serial-mcu.md) I-SR3/I-SR4 |
| `verify` | malformed workspace → diagnostics; model build never panics | [workspace-model.md](../algorithms/toolchain/workspace-model.md) |
| `stress_link_storm` | link cut → isolation → reconnect resume (I-RT6) | [router-core.md](../algorithms/router/router-core.md) |
| `stress_link_storm_sessions` | reconnect generation bump → boundary gap (I-RT5) | router-core.md |

## Method

- **libFuzzer coverage**: `cargo fuzz coverage --fuzz-dir tests/fuzz <target>`
  produces per-target line coverage over the instrumented crates; the nightly
  `fuzz` job runs each target for 3600 s and the coverage profile is archived
  with the run artifact.
- **Chaos coverage**: the chaos suites assert the recovery contract directly —
  every cell has a named assertion (`freed`, `pool_exhausted`, `emitted`,
  `Gap { lost }`), so "the recovery loop ran" is a green assertion, not a
  coverage percentage.
- **Regression corpus**: any nightly finding is added to the target corpus
  (fuzz) or as an assertion (chaos); the corpus replay is 100% pass
  (qualification.md I-QAL7).

REQ: REQ-QAL-004, REQ-QAL-007, REQ-QAL-009, REQ-QAL-010.
