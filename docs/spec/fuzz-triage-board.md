# Fuzz Triage Board & Nightly Wiring (M18W1)

Work-plan [M18W1-E4-T1](../../docs/work-plan/work-plan-m18-w1.md): every live
fuzz target and chaos suite is scheduled nightly, and every finding lands on
this board with an owner and a severity. `FUZZ_CRASHES = 0` means **0
untriaged**: a crash is triaged the night it is found, never carried.

## Nightly schedule

| Job | Targets / suites | Budget | Owner |
|---|---|---|---|
| `fuzz-nightly` → `fuzz` | `schema`, `framing`, `datagram`, `hazard`, `serial`, `verify` | 3600 s each, `-timeout=5` | E1 (`hazard`), E2 (`verify`), E3 (`serial`), E4 (wiring) |
| `fuzz-nightly` → `chaos` | `stress_chaos_*` (drm-core), `stress_shm_kill_*` (drm-core), `stress_link_storm*` (drm-router) | `DRM_STRESS_ITERS=200` | E1 (kill matrix), E3 (link storm), E4 (wiring) |
| `fuzz-nightly` → `udp-pair` | UDP loopback pair ×3 (I-RT9) | 3 runs | E3 |
| `fuzz-nightly` → `stress` | structured billion-cycle + ring depth sweep | 1e8 cycles | E1 |
| `ci.yml` → `fuzz-pr` | `schema`, `datagram`, `hazard`, `serial`, `verify` | 120 s each | E4 (wiring) |

Target sources live in [tests/fuzz](../../tests/fuzz/README.md); the workflow is
[.github/workflows/fuzz-nightly.yml](../../.github/workflows/fuzz-nightly.yml).

## Severity classes

| Class | Definition | Response |
|---|---|---|
| **P0** | Nightly crash, hang, OOM, or sanitizer finding in a live target | Same-night triage; minimal repro added to the target corpus; fix or quarantine within 1 week (AGENTS #14) |
| **P1** | Reproducible wrong-result or invariant break without a crash (e.g. a recovery assertion in a chaos suite) | Triage next business day; owner assigned; fix or explicit defer with ADR |
| **P2** | Non-reproducible / environmental flake | Quarantine the cell ≤1 week, repair or delete (AGENTS #14); never a silent skip |

## Board

| Finding | Target / suite | Class | Owner | State |
|---|---|---|---|---|
| _(none open)_ | — | — | — | — |

## Triage rules

1. Every nightly failure is filed here before the next run; `FUZZ_CRASHES`
   counts untriaged findings, not historical ones.
2. A triaged crash becomes a permanent regression input in the target's corpus
   (fuzz) or a new assertion (chaos); the repro is never discarded
   (qualification.md I-QAL7).
3. A target graduates to a permanent PR-blocking gate only after two
   consecutive clean nightly runs (codebase.md 4.1).
4. Flake quarantine is time-boxed to one week; the quarantine note names the
   owner and the expiry date.

REQ: REQ-QAL-004, REQ-QAL-007, REQ-QAL-009, REQ-QAL-010, REQ-TOOL-053.
