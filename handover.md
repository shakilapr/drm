# Handover — DRM 24-month solo run (state @ `abcc4b0`)

Continuation guide for the next agent. Read `AGENTS.md` first; it is normative.
This session executed the work plan week-by-week with zero plan edits.

## Where we are

- Repo `E:\doc\drm`, branch `main` only, remote `origin https://github.com/shakilapr/drm.git`.
- Local env: Windows, `pwsh`. Git at `E:\Programs\Git`. Rust stable pinned `1.94.0` (edition 2024:
  `gen` is reserved, let-chains available). Linux target `x86_64-unknown-linux-gnu` installed.
- Last commit `abcc4b0`; CI green (`37220920082`). Working tree: only `tests/fuzz/Cargo.lock` dirty
  (fuzz-independent lockfile churn — leave it or `git checkout` it; do not commit casually).
- Solo completion intent from user: execute existing `docs/work-plan/work-plan-mNN-wN.md` files
  in order, **no plan/roadmap edits**. Linux-only execution suites (stress/sanitizers/fuzzing/
  two-host sim) are **deferred** ("keep for later") — they run on green CI instead.

## Done this session

- **M03W1 -> `841798f`** (CI green): E2-T1 verify-lite `drm-workspace` single-writer checks
  (`14e0ed1`); E3 UDP datagram adapter (`8d76fa6`); docs — NEW `docs/spec/metadata-layout.md`,
  NEW `docs/gates/phase-05.md`, `docs/metrics/metrics-m03.md`, `tests/conformance/README.md` +
  transport conformance cells.
- **M03W2 -> `71dd61b`** (CI green after yamllint wrap): `TransportAdapter` trait + `Reliability
{BestEffort,Reliable}` + UDP impl (`12ae059`); NEW `crates/drm-core/src/channel/pattern.rs` +
  `tests/pattern_corruption.rs`, `Channel: Sync` unsafe impl, NEW
  `tests/stress_structured_billions.rs`, ring coherence/layout/release tests, `channel-publish.md`
  (`36666bb`); metrics/CHANGELOG/`tests/stress/README.md` + nightly `stress` job (`71dd61b`).
- **M03W3 -> `a0012e6`** (CI green): NEW `crates/drm-router/src/priority.rs`
  (`PriorityClass{Safety,Control,Status,Diagnostics}`, `PriorityQueue<T>` strict class order +
  per-class FIFO) + drop-newest `RecvQueue<T>` in `transport/mod.rs` (`e5c40b7`); depth tests
  `retention_depths_1_to_64` + `ring_depths_property` (65/100/333/1000/4096) +
  `stress_ring_depth_sweep` (ignored, 65..=8192) (`c26dc89`); NEW `docs/gates/phase-09.md`,
  metrics-m03, CHANGELOG, `lanes-shaping.md`, nightly ring-sweep lane (`a0012e6`).
- Fix follow-up: `abcc4b0` wrapped overly long `run:` lines in `.github/workflows/fuzz-nightly.yml`
  (yamllint `line-length` 100 gate) — CI green.

### Key evidence captured

- BILLIONS_CYCLES_LOCAL: release, `DRM_STRESS_CYCLES=1000000000`, depths [1,3,16], 4 readers
  => 3e9 total cycles, dropped=0 torn=0 mixed=0 invalid=0 monotonic=0, ~1296s.
- Alloc gate `alloc_counter`: 0/0 on steady-state publish/consume.
- Ring tests depth 1..=64 green; `drm-router` lib 7 tests green (incl. `udp_backpressure`).

## Gate status

| Gate                          | Doc                      | Status         |
| ----------------------------- | ------------------------ | -------------- |
| Phase 5 (metadata layering)   | `docs/gates/phase-05.md` | PASS           |
| Phase 8                       | `docs/gates/phase-08.md` | (pre-existing) |
| Phase 9 (structured channels) | `docs/gates/phase-09.md` | PASS (M03W3)   |
| Phase 6 (handles)             | pending                  | M03W4          |
| Phase 10 (rings)              | pending                  | M03W4          |

## Immediate next: M03W4 (`docs/work-plan/work-plan-m03-w4.md`)

Phase 6 + Phase 10 gates Friday + GAP delivery. Roles E1..E4 (see plan day matrix). Highlights:

- **E1**: GAP aged-out branch per `docs/algorithms/runtime/cursor-delivery.md` §Functions `next`
  (`missed = oldest_retained − last_read − 1`; skip-to-newest `last_read = ws−1` after Gap);
  fixtures lap k ∈ 1..=32 exact (`cargo test -p drm-core gap`); independence I-C1; no-skip I-C4
  proptest.
- **E2**: Phase 10 gate doc (signs) + Phase 6 gate doc; `drm ids` report + `drm ids compact --yes`
  (I-H4) + **ADR 0004** compaction/tombstone; `drm assign <topic> --consume/--publish <node>`
  manifest-mutation CLI (round-trip: assign -> manifest diff -> `drm verify` green).
- **E3**: transport registry (route -> adapter via capability table; unknown mapping => startup
  config error; `cargo test -p drm-router registry`); lane impl start
  (`docs/algorithms/router/lanes-shaping.md` `PeerLanes`/`Lane`, per-peer realtime+bulk, FIFO
  within class, static caps).
- **E4**: cursor property harness (multi-speed readers, lap matrices, no-skip model); evidence
  packs x2; presentations; close `metrics-m03.md`.

### Gate-doc precedent (reuse)

`# Phase N Gate — <title>`, evidence-mapping table `Criterion | Evidence | Status`, verdict PASS
line, criteria<->evidence + totals tables. See `docs/gates/phase-04.md`, `phase-05.md`,
`phase-09.md`.

## Verification commands (run each week, all must pass)

```
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

- `audit-traceability` takes **no** `--registry` flag (defaults to `docs/req/registry.csv`;
  currently 325 rows, 0 findings, 100/100 `.rs` files cited).
- Stress suite: files prefixed `stress_`, annotated
  `#[ignore = "stress suite: cargo xtask test --stress"]`; run with
  `cargo test --workspace -- --ignored stress --nocapture`.

## Hard constraints (do not break)

- Lints deny `missing_docs`/`unwrap`/`expect`/`panic` + clippy all+pedantic. Idioms: `collapsible_if`
  -> use let-chains; `useless_vec` -> `&[...]`. Doc comments need backticks around code items.
- **Never** mutate text via PS `-replace | Set-Content`; use the Edit tool.
- Commits: conventional prefix (`feat|fix|docs|refactor|perf|test|chore`), state what was done,
  **no week numbers**. CHANGELOG bullets may use "Week-N tranche" precedent.
- Architecture-first (AGENTS.md #1): update `docs/architecture*.md` / `codebase.md` before touching
  `crates|bindings|packages|apps|tests|xtask`. Every public fn/error branch/test cites a registry
  ID (`docs/req/registry.csv`); registry changes land in the same PR.
- `codebase.md §5.2`: `.github/workflows/audit.yml` is an intentional placeholder (`if: false`) —
  do **not** "fix" it.
- Layering: `drm-types` zero deps; `drm-workspace` never depends on `drm-core`; ops crates never
  depend on each other.
- C++ golden runner deferred to Phase 20 gate (M07W4, closes REQ-BIND-013).

## Month arc after M03W4

M04 = gates 11,12,13 · M05 = 14,15 · M06 = 16,17,18 · M07 = 19,20 (C++ binds, closes
REQ-BIND-013) · M08 = 21,22,23. Execute matching `docs/work-plan/work-plan-mNN-wN.md` files.

## Key files

- Plans: `docs/work-plan/work-plan-m03-w4.md` .. `-m08-w4.md`; `work-plan.md`.
- Runtime: `crates/drm-core/src/{channel/{mod,ccb,pattern}.rs,ring/mod.rs,cursor*}`;
  tests `crates/drm-core/tests/{attach_semantics,descriptor_golden,alloc_counter,
pattern_corruption,stress_structured_billions}.rs`.
- Router: `crates/drm-router/src/{priority.rs,lib.rs,shaping.rs,transport/{mod,udp}.rs}`,
  `examples/udp_pair.rs`.
- Algorithms: `docs/algorithms/router/{router-core,lanes-shaping}.md`,
  `docs/algorithms/runtime/{cursor-delivery,ring-history}.md`,
  `docs/algorithms/toolchain/handle-system.md`.
- Specs/gates/metrics: `docs/spec/{metadata-layout.md,README.md}`,
  `docs/gates/phase-{04,05,08,09}.md`, `docs/metrics/metrics-m03.md`, `CHANGELOG.md`,
  `docs/req/registry.csv`.
- CI: `.github/workflows/{ci,sanitizers,fuzz-nightly,audit}.yml`.
