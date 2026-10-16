# xtask Command Registry

All repository automation is invoked through `xtask` (codebase.md §5.1). No one-off scripts.

## Core commands (codebase.md §5.1)

| Command | Purpose |
|---|---|
| `xtask fmt` | rustfmt, clang-format, ruff, prettier, taplo, markdownlint (`--check` in CI). Prettier's scope is set by [.prettierignore](../.prettierignore): the byte-exact `tests/fixtures/` vectors, the hand-wrapped markdown tree, and the separate `apps/studio` project are excluded — prettier would rewrite the goldens and reflow the docs |
| `xtask lint` | clippy `-D warnings`, mypy, eslint, yamllint, markdownlint |
| `xtask test` | unit + property + doc tests, all languages |
| `xtask check-layering` | crate dependency-layer verification (codebase §2.2 R6) |
| `xtask gen-check` | strict generation check (codebase §3.1 R6): **the default subject is the mandatory fixture** `tests/integration/primitive`, which must declare at least one topic and one node and ship its generated artifacts — its absence, emptiness, or missing output each fail. A workspace passed with `--workspace <dir>` may legitimately declare no DRM content (reported as "nothing to check", never as a bare "clean"), but any content it *does* declare must ship its artifacts. `--ts-only` checks only the generated TypeScript surface (used for the repo-root `studio-protocol`) |
| `xtask audit-zero-alloc` | data-path allocation audit (codebase §3.2) |
| `xtask perf-matrix` | system performance matrix: measures the fixture payload classes (`tests/perf/workspace`), checks the p99 budgets + baseline bands, and records the cross-process segment lifecycle; `--workspace <dir>`, `--json <path>`, `--quick` (`xtask/src/perf.rs`, [bench-methodology.md](../docs/spec/bench-methodology.md) §System perf matrix) |
| `xtask audit-traceability` | REQ citation coverage; flags `--registry`, `--citations`, `--matrix` |
| `xtask workplan` | 24-month work-plan progress: `status` (what's done / in progress), `next`, `watch [--interval <s>]`, `mark-done <week>` |
| `xtask docs` | build docs + execute examples headless |

## Extension commands (ADR 0003)

| Command | Purpose |
|---|---|
| `xtask integration` | Phase 0 integration fixture: `--workspace <dir>` (default `tests/integration/primitive`), `--check`. Generates the fixture's artifacts, proves an unchanged re-plan is fully cached, runs the strict `gen-check` over it, builds the **two independent fixture applications** and runs them against one shared-memory segment, asserting every sample arrives in order. `--ts-only` delegates to the TypeScript surface check (`xtask/src/integration.rs`, [tests/integration/primitive/README.md](../tests/integration/primitive/README.md)) |
| `xtask golden-gen` | regenerate `tests/fixtures/golden/` byte vectors (idempotent) |
| `xtask soak` | long-duration stability suite; `--duration <h>` or `--iterations <n>`, `--topology <single-machine\|two-machine-sim\|physical-pair>`, `--parallel <n>` (isolated target dirs), `--memory-bound <mb>` (I-QAL2 memory-boundedness, I-MB9), `--report <path>`, `--ledger <path>`, `--dashboard`. Rotating topologies, I-QAL2 evidence ledger, three-consecutive-clean gate, zero-false-positive anomaly threshold band (`xtask/src/soak.rs`, [soak-ledger.md](../docs/spec/soak-ledger.md)) |
| `xtask fault` | fault-injection campaigns: `--campaign <publisher-crash\|exhaustion-burst\|subscriber-kill\|serial-bomb\|link-storm>`, `--seed <n>`, `--steps <n>`, `--list`, `--dry-run`, `--json <path>`. Runs the five runtime/transport campaigns (`FAULT_CAMPAIGNS`) and records the deterministic replay digest (`xtask/src/fault.rs`, [fault-injection.md](../docs/spec/fault-injection.md)) |
| `xtask qualify` | qualification gate packet assembly: `--check` (fail on an incomplete packet), `--json <path>`. Aggregates the `I-QAL1` traceability matrix, `I-QAL2` fault/soak evidence, `I-QAL4` conformance matrix closure, the `I-QAL8`/`I-QAL9` coverage, the API freeze/pilot/packaging legs, and the perf-invariant/repro/docs evidence (`xtask/src/qualify.rs`, [qualification.md](../docs/algorithms/program/qualification.md)) |
| `xtask freeze` | v1.0 API freeze: `--check` (fail while unfrozen), `--json <path>`. Locks the crate/binding versions, re-resolves every frozen surface append-only, and stamps artifacts with their ABI version + contract fingerprint (`API_FROZEN`; `xtask/src/freeze.rs`, [api-freeze.md](../docs/spec/api-freeze.md)) |
| `xtask pilot` | pilot deployment + rollback drill: `--check`, `--json <path>`. Verifies the field environment (router + MCU + Studio UI), the stamped deployment identity, and the deploy ⇒ injected-fault ⇒ rollback drill (`PILOT_DEPLOYED`; `xtask/src/pilot.rs`) |
| `xtask package` | pilot packaging dry-run: `--check`, `--dry-run`, `--json <path>`. Resolves the declared bundle set (crates, headers, Rust SDK, Python wheel, Studio packages) and renders the bundle manifest (`BUNDLES_VERIFIED`; `xtask/src/packaging.rs`, [packaging.md](../docs/spec/packaging.md)) |
| `xtask release` | v1.0 release candidate fold: `--check` (fail while the candidate is unmet), `--json <path>`. Folds the completed Phase 34 packet into the RC tagged with the stamped deployment identity (`REQ-CORE-085`), requires the drafted release notes, and enforces waiver discipline — a red evidence line needs a recorded waiver with a rationale and a review quorum (`WAIVER_QUORUM`) (`V10_RC`; `xtask/src/release.rs`, [release-notes.md](../docs/spec/release-notes.md), [waiver-template.md](../docs/spec/waiver-template.md)) |
| `xtask closeout` | v1.0 release + program close-out: `--check` (fail while `V10_RELEASED`/`POST_10_BACKLOG`/`RETRO_DONE` is unmet), `--json <path>`. Builds/publishes the release across the declared-supported targets, stamps the deployment identity, grooms the post-1.0 backlog, and completes the program retrospective (`GATE_STATUS(M24W4) = PASS`; `xtask/src/closeout.rs`, [release-notes.md](../docs/spec/release-notes.md), [post-1.0-backlog.md](../docs/spec/post-1.0-backlog.md), [program-retrospective.md](../docs/spec/program-retrospective.md)) |
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
