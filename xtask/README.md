# xtask Command Registry

All repository automation is invoked through `xtask` (codebase.md §5.1). No one-off scripts.

## Core commands (codebase.md §5.1)

| Command | Purpose |
|---|---|
| `xtask fmt` | rustfmt, clang-format, ruff, prettier, taplo, markdownlint (`--check` in CI) |
| `xtask lint` | clippy `-D warnings`, mypy, eslint, yamllint, markdownlint |
| `xtask test` | unit + property + doc tests, all languages |
| `xtask check-layering` | crate dependency-layer verification (codebase §2.2 R6) |
| `xtask gen-check` | regenerate against fixtures; fail on diff (codebase §3.1 R6) |
| `xtask audit-zero-alloc` | data-path allocation audit (codebase §3.2) |
| `xtask audit-traceability` | REQ citation coverage; flags `--registry`, `--citations`, `--matrix` |
| `xtask workplan` | 24-month work-plan progress: `status` (what's done / in progress), `next`, `watch [--interval <s>]`, `mark-done <week>` |
| `xtask docs` | build docs + execute examples headless |

## Extension commands (ADR 0003)

| Command | Purpose |
|---|---|
| `xtask golden-gen` | regenerate `tests/fixtures/golden/` byte vectors (idempotent) |
| `xtask soak` | long-duration stability suite; `--duration <h>` or `--iterations <n>`, `--topology <single-machine\|two-machine-sim\|physical-pair>`, `--parallel <n>` (isolated target dirs), `--memory-bound <mb>` (I-QAL2 memory-boundedness, I-MB9), `--report <path>`, `--ledger <path>`, `--dashboard`. Rotating topologies, I-QAL2 evidence ledger, three-consecutive-clean gate, zero-false-positive anomaly threshold band (`xtask/src/soak.rs`, [soak-ledger.md](../docs/spec/soak-ledger.md)) |
| `xtask fault` | fault-injection campaigns: `--campaign <publisher-crash\|exhaustion-burst\|subscriber-kill\|serial-bomb\|link-storm>`, `--seed <n>`, `--steps <n>`, `--list`, `--dry-run`, `--json <path>`. Runs the five runtime/transport campaigns (`FAULT_CAMPAIGNS`) and records the deterministic replay digest (`xtask/src/fault.rs`, [fault-injection.md](../docs/spec/fault-injection.md)) |
| `xtask qualify` | qualification gate packet assembly: `--check` (fail on an incomplete packet), `--json <path>`. Aggregates the `I-QAL1` traceability matrix, `I-QAL2` fault/soak evidence, `I-QAL4` conformance matrix closure, and the perf-invariant/repro/docs evidence (`xtask/src/qualify.rs`, [qualification.md](../docs/algorithms/program/qualification.md)) |
| `xtask test --stress` | chaos/stress suites (`tests/stress`) |
| `xtask test --two-host` | two-host router/transport matrix |
| `xtask test --cross-os` | cross-OS conformance manifest (`xtask/src/cross_os.rs` `SUBSET`); every Linux suite row green on Windows or scheduled with a port board entry (owner + gate-risk) |
| `xtask test --cross-os --full` | full conformance run ×`CONFORMANCE_RUNS` (run 2 must be green), per-run evidence under `target/conformance/` |
| `xtask test --cross-os --transport <udp\|tcp\|serial>` | cross-OS interop matrix (`xtask/src/cross_os.rs` `interop_matrix`): `{publisher_os, consumer_os} × {C, C++, Rust, Python} × {udp, tcp, serial}` cells executed over loopback (or the two-netns simulation fallback) with per-cell evidence under `target/interop/` and the full cell table at `target/interop/matrix-report.txt`; the 24 same-language cells are core (gate-blocking), mixed-language pairs tracked; `serial` is a host-loopback of the byte-exact codec |
| `xtask bench --report-regressions` | benchmark regression report (audit.yml) |
| `xtask abi-check --against <ref>` | ABI/layout compatibility vs a previous release (release.yml) |
| `xtask publish` | lockstep crates.io publishing (release.yml) |
| `xtask sbom --out <file>` | SBOM generation (release.yml). *Registered, not yet implemented — planned for the release phase (docs/standards.md Planned).* |
| `xtask audit syscall` | syscall audit on the data path |

Adding, renaming, or removing a command requires updating this registry and ADR 0003 in the same PR.
