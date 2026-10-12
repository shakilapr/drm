# Program Retrospective

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
`REQ-QAL-032`; the program close-out retrospective (M24W4) mapping outcomes to
the eight [work-plan.md](../../work-plan.md) waves, the final risk-register
state, and the metrics history. Machine form:
[xtask/src/closeout.rs](../../xtask/src/closeout.rs).

## Outcomes vs waves

Each plan wave closed against its declared exit criteria; the phase suites are
permanent blocking gates (AGENTS #14).

| Wave | Months | Outcome |
|---|---|---|
| W1 Foundations & Specification | M1–M3 | Architecture + requirement registry; parser, type system, Linux backend |
| W2 Core Runtime | M3–M9 | Channels, rings, cursors, overflow, pools, hazard, exhaustion, lifecycle, deployment identity |
| W3 Bindings, Toolchain & Verification | M6–M12 | C/C++/Rust/Python bindings, WorkspaceModel, codegen, verify, feasibility, routing |
| W4 Boundary Router & Transports | M10–M16 | Router, UDP/TCP/serial, lanes, shaping, two-host matrix |
| W5 Windows Runtime Backend | M14–M18 | Windows shared memory, locks, lifecycle parity |
| W6 Operational Tooling | M15–M19 | Introspection, diagnostics, record/replay, trace, RPC, embedded/serial |
| W7 DRM Spatial | M19–M22 | Typed spatial core, clock domains, caches, proximity, MCU subset |
| W8 Whole-System Qualification & Release | M22–M24 | Qualification packet, API freeze, pilot, v1.0 release |

## Risk register

Final state of the [work-plan.md](../../work-plan.md) risk register: every risk
closed or mitigated, none carried open into post-1.0.

- Hazard-protocol subtlety (ABA, race windows) — mitigated by Phase 14 race
  forcing and permanent mutation-audited gates; closed.
- Linux/Windows SHM divergence — mitigated by the frozen backend trait and
  per-phase parity tests; closed.
- Requirement drift — mitigated by same-PR registry sync and the `I-QAL1`
  traceability gate; closed (100% coverage).
- Router lane/shaping inversion — mitigated by the artificial-saturation suite
  and two-host soak; closed.
- Spatial scope compression — mitigated by the M19 start and reused verify
  infrastructure; closed.
- Serial/MCU target diversity — mitigated by the locked `h743`/`f407` target set
  and the zero-forbidden-heap audit; closed.
- Architecture falsification — mitigated by SPIKE/EXP tasks, ADRs, and
  same-PR architecture updates; closed.

## Metrics history

The monthly metrics tranches ([metrics-m24.md](../metrics/metrics-m24.md) and
predecessors) are replayed at close-out; the final M24 tranche records the v1.0
release variables.

- Traceability `I-QAL1`: 100% coverage, 0 orphans.
- Soak `I-QAL2` / fault campaigns: green; `FAULT_CAMPAIGNS = 5/5`.
- Coverage `I-QAL8`/`I-QAL9`: all critical paths resolved; mutation sample clean.
- Release: `API_FROZEN = 1`, `PILOT_DEPLOYED = 1`, `BUNDLES_VERIFIED = 7/7`,
  `V10_RC = 1`, `V10_RELEASED = 1`.

## Verification

```text
cargo xtask closeout --check
```

`RETRO_DONE = 1` iff the doc covers all eight waves and records both the risk
register and the metrics history; the retrospective is part of the program
close-out gate (`REQ-QAL-033`).

REQ: REQ-QAL-032.
