# DRM Monorepo Codebase & Engineering Standards

Normative sources: [docs/architecture.md](docs/architecture.md), [docs/spatial_architecture.md](docs/spatial_architecture.md), [docs/ui-architecture.md](docs/ui-architecture.md), [work-plan.md](work-plan.md), [AGENTS.md](AGENTS.md).

Precedence: architecture docs > work-plan > this document. This document defines codebase organization and engineering practice; it never overrides architecture.

---

## 1. Repository Layout

```text
drm/
├── .github/
│   ├── workflows/                  # ci.yml · sanitizers.yml · audit.yml · release.yml (§5.2)
│   ├── ISSUE_TEMPLATE/             # bug_report.yml, feature_request.yml
│   └── PULL_REQUEST_TEMPLATE.md
│
├── .cargo/config.toml              # target flags, sccache, cross-compilation profiles
├── Cargo.toml                      # virtual workspace; [workspace.dependencies], [workspace.lints]
├── Cargo.lock                      # committed; reproducible builds on every platform
├── rust-toolchain.toml             # pinned toolchain + components
├── deny.toml                       # cargo-deny: licenses, bans, advisories, sources
├── rustfmt.toml                    # max_width = 100
├── pnpm-workspace.yaml             # frontend workspace + version catalogs
├── package.json                    # workspace scripts (fmt, lint, build, test-all)
├── tsconfig.base.json              # shared strict TS compiler options
├── .editorconfig                   # universal whitespace, indentation, encoding rules
├── .clang-format                   # C11 / C++20 formatting specification
├── pyproject.toml                  # ruff, mypy --strict, pytest
├── .gitignore                      # build caches, generated artifacts, runtime state
├── .gitattributes                  # LF normalization, binary markers
├── LICENSE-MIT                     # dual license: MIT OR Apache-2.0 (§7.2)
├── LICENSE-APACHE
├── README.md                       # entry point and quickstart
├── CONTRIBUTING.md                 # environment, workflow, review rules
├── SECURITY.md                     # vulnerability reporting policy (§7.2)
├── CHANGELOG.md                    # Keep-a-Changelog format (§6.5)
├── CODEOWNERS                      # subsystem ownership (§7.1)
├── AGENTS.md                       # directives; normative for all agents
├── work-plan.md                    # 24-month program schedule (Phases 1–34)
├── codebase.md                     # this document
│
├── crates/                         # Rust workspace members; layering in §2.1
│   ├── drm-types/                  # primitives, handles, status codes; zero dependencies
│   ├── drm-core/                   # SHM backends, rings, hazard pools, epochs, precommit
│   │   └── src/{shm,hazard,ring,epoch,alloc}/
│   ├── drm-ffi/                    # canonical C ABI exports; drm.h single source of truth
│   ├── drm-router/                 # boundary router: framing, priority lanes, traffic shaping
│   ├── drm-spatial/                # spatial-temporal engine (Phase 33)
│   ├── drm-workspace/              # parser, WorkspaceModel, verifier, codegen, topology
│   │   └── src/{parser,model,verifier,codegen,topology}/
│   ├── drm-record/                 # record/replay; recorder-owned buffers; no disk I/O on borrows
│   ├── drm-diag/                   # out-of-band diagnostics aggregation daemon
│   ├── drm-trace/                  # non-blocking best-effort event tracing
│   ├── drm-rpc/                    # request/response and actions over correlated topic pairs
│   ├── drm-cli/                    # `drm` executable + `drm studio --stdio` host
│   ├── drm-extension-host/         # WASM extension host process (wasmtime resource budgets)
│   └── xtask/                      # all repository automation (§5.1)
│
├── bindings/                       # first-party bindings over drm-ffi
│   ├── c/                          # drm.h distribution + pure C11 ABI conformance suite
│   │   ├── include/drm/drm.h
│   │   └── tests/
│   ├── cpp/                        # C++20 header-only RAII (drm::Borrow<T>) + Catch2 tests
│   │   ├── include/drm/drm.hpp
│   │   └── tests/
│   ├── rust/crates/drm/            # safe Rust SDK; borrow guards (root workspace member)
│   └── python/                     # PyO3 native module + pure facade; maturin build
│       ├── drm/
│       ├── tests/
│       └── pyproject.toml
│
├── packages/                       # shared TypeScript packages (§2.3)
│   ├── studio-protocol/            # generated TS types + JSON-RPC schema (codegen output)
│   ├── studio-extension-api/       # stable public API surface for Studio extensions
│   └── drm-visual-schemas/         # drm.visual.* vocabulary; never merged into core
│
├── apps/studio/                    # DRM Studio: React 19 + TS + Vite + Tailwind + Radix
│   ├── electron/                   # main process, window lifecycle, stdio RPC bridge
│   ├── src/
│   │   ├── activities/             # Workspace, Nodes, Topics, System, Graph, Run, Settings
│   │   ├── components/             # Inspector, Problems, resizable panels
│   │   ├── panels/                 # built-in visualization panels
│   │   ├── rpc/                    # typed JSON-RPC client (studio-protocol)
│   │   └── stores/                 # Zustand UI state only — zero DRM logic
│   ├── vite.config.ts
│   └── electron-builder.json5
│
├── examples/                       # CI-executed workspace + node per binding language
│
├── tests/                          # whole-system harness (§4); per-crate tests live inside crates
│   ├── conformance/                # 4-language cross-process ABI and interop matrix
│   ├── fixtures/                   # manifests, .drm schemas, golden/ byte vectors
│   ├── fuzz/                       # cargo-fuzz targets: schema, framing, hazard protocol
│   ├── stress/                     # chaos: kill -9 races, PID reuse, pool exhaustion
│   └── soak/                       # long-duration stability
│
├── docs/
│   ├── architecture.md             # normative core specification
│   ├── spatial_architecture.md     # normative spatial specification
│   ├── ui-architecture.md          # normative Studio specification
│   ├── work-plan/                  # milestone execution documents (weekly plans reference ../algorithms/)
│   ├── algorithms/                 # canonical algorithm pseudocode, grouped per subsystem (runtime/toolchain/router/bindings/ops/studio/spatial/program)
│   ├── req/                        # requirement registry (§6.2)
│   └── adr/                        # decision records (§6.3)
│
└── references/                     # read-only; strictly clean-room; never edited, never copied
```

Artifact rules:
- Generated bindings are never committed to this repo and never hand-edited (§3.1 R6).
- Runtime deployment state (`.drm/deploy/`, `studio.local.json`) is machine-local and git-ignored.
- `references/` sits outside the workspace graph; CI fails on any diff touching it.

---

## 2. Codebase Structure

### 2.1 Crate Layers
| Layer | Crates | May depend on | Must not depend on |
|---|---|---|---|
| types | `drm-types` | nothing | — |
| runtime | `drm-core`, `drm-ffi`, `drm-router`, `drm-spatial` | types; lower runtime layers | toolchain, ops, facades |
| toolchain | `drm-workspace` | types | runtime, ops, facades |
| ops | `drm-record`, `drm-diag`, `drm-trace`, `drm-rpc` | core, types | each other, toolchain, facades |
| facade | `drm-cli`, `drm-extension-host`, `xtask` | any lower layer | — |
| bindings | `bindings/{c,cpp,rust,python}` | ffi (rust also core) | toolchain, ops, facades |

### 2.2 Structural Rules
1. `drm-types` has zero dependencies.
2. `drm-core` has no networking, disk, or compiler dependencies (architecture MVP boundary).
3. `drm-workspace` never depends on `drm-core` — toolchain and runtime evolve independently.
4. Ops crates never depend on each other; composition happens only in `drm-cli`.
5. Every new crate declares its layer and a CODEOWNERS entry in its creation PR.
6. Enforcement is mechanical, not cultural: `xtask check-layering` (cargo-metadata graph) + `cargo-deny` bans, blocking in CI (§5.2).

### 2.3 Frontend Package Rules
1. `packages/*` are consumed by apps; packages never import apps.
2. `studio-protocol` is generated output (§3.1 R6); hand edits are rejected by `xtask gen-check`.
3. `studio-extension-api` is the only permitted extension surface; internals, stores, and Electron APIs are never exposed (ui-arch).
4. `drm-visual-schemas` stays out of DRM core, permanently.

### 2.4 Directory Conventions
1. Per-crate unit/property tests live inside each crate; `tests/` holds only whole-system suites (§4.1).
2. `examples/` contains one buildable, CI-executed workspace per binding language.
3. All automation lives in `xtask/` (§5.1); no one-off shell scripts.
4. Documentation set: normative specs (architecture docs), execution plan (work-plan), requirement registry (§6.2), decision records (§6.3).

---

## 3. Coding Standards

### 3.1 Universal Rules (all languages)
| # | Rule |
|---|---|
| R1 | **Architecture-first** (AGENTS R1): no implementation edit without architecture alignment; architecture-affecting PRs carry the arch edit and the REQ registry diff (§6.2) in the same PR. |
| R2 | **Traceability**: every public function, error branch, and test cites ≥1 registry ID: `// REQ: REQ-CORE-042 (Hazard slot acquisition and ABA generation check)`. Enforced by `xtask audit-traceability`; untraceable code is rejected at review. |
| R3 | **Comments**: terse, rationale-only (AGENTS R7). Mandatory: `// SAFETY:` on every unsafe block; `// REQ:` citations. Prohibited: code narration, history comments. |
| R4 | **Errors**: one status taxonomy in `drm-types`, mirrored 1:1 by stable C ABI codes. Data paths return errors — never panic, `unwrap`, or `expect`. Init/teardown may `expect` with an invariant-stating message. |
| R5 | **Data-path discipline**: steady-state publish/consume performs no allocation, no logging, no blocking locks, no unbounded syscalls. Atomics carry explicitly documented acquire/release ordering. Diagnostics flow only through the observability plane (`drm-trace`, channel counters, `drm-diag`). |
| R6 | **Generated code**: never hand-edited. Emitted with a `@generated` banner naming the generator version; CI regenerates and fails on diff; unchanged outputs preserve timestamps to avoid rebuild cascades. |
| R7 | **Naming**: crates `drm-*`; modules/fields `snake_case`; types `UpperCamel`; topics `domain/name` (kebab); REQ IDs `REQ-<AREA>-<NNN>` with areas `CORE \| TOOL \| ROUTE \| SPATIAL \| BIND \| STUDIO`. |
| R8 | **Platform isolation**: OS-specific code exists only in `drm-core::shm` backends and router transport adapters; no scattered `#[cfg(windows)]`; backend traits frozen in W1 (work-plan risk register). |
| R9 | **API surface**: `missing_docs` denied; public items document their REQ contract; internals are `pub(crate)` or `#[doc(hidden)]`. |
| R10 | **Files**: UTF-8, LF, final newline; code ≤100 columns; enforced by `.editorconfig` and formatters. |
| R11 | **Shared atomics**: shared-memory layouts use explicit fixed-width atomic fields, never aggregate or packed atomics. Activation proves required atomics lock-free. Protocol docs name linearization points and crash states. |

### 3.2 Rust (`crates/*`, `bindings/rust`)
- Edition 2024; toolchain pinned via `rust-toolchain.toml`; formatting via rustfmt (`max_width = 100`).
- Lints via `[workspace.lints]`: all crates deny warnings; runtime crates additionally deny `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::pedantic`. Init/teardown exceptions use narrow module-level allows with an invariant comment (R4).
- `unsafe_op_in_unsafe_fn` denied globally; `#![forbid(unsafe_code)]` in toolchain, ops, and facade crates; `unsafe` confined to `drm-core` and `drm-ffi`.
- Zero-allocation data path proven by a counting `GlobalAlloc` test harness plus `xtask audit-zero-alloc` (linker/symbol audit; MCU gate via linker sections, not review).
- Errors: typed enums via `thiserror`, mapping 1:1 to the status taxonomy (R4).
- API stability: `cargo-semver-checks` on every public crate (§6.4).

### 3.3 C and C++ (`bindings/c`, `bindings/cpp`)
- Standards: C11 (freestanding-safe subset on MCU targets); ISO C++20.
- Flags: GCC/Clang `-Wall -Wextra -Wpedantic -Wconversion -Wshadow -Werror`; MSVC `/W4 /WX`.
- MCU subset (`bindings/c` on RTOS/bare-metal): no heap, no recursion, statically bounded loops, `stdint.h` fixed-width types only, no stdio on the data path.
- ABI: opaque relocatable handles `{slot, generation, sequence}`; zero raw cross-process pointers; no exceptions across the C ABI boundary.
- C++: strict RAII — `drm::Borrow<T>` releases hazard slots on every scope exit, including exceptions; clang-tidy (`bugprone-*`, `cert-*`) runs on public headers.

### 3.4 Python (`bindings/python`)
- Python 3.10+; `ruff check` + `ruff format`; `mypy --strict`; pytest.
- Default reads return owned copies (`latest()`, `next()`); zero-copy only inside `with topic.borrow_latest()/borrow_next()` context managers; `.numpy()` views valid only inside the scope (forced-GC and exception tests per Phase 22).
- The generated API exposes only manifest-declared topics; violations raise `ContractError`.

### 3.5 TypeScript and React (`apps/studio`, `packages/*`)
- Node 22 LTS, pnpm, Vite; TS strict + `exactOptionalPropertyTypes`; ESLint flat config + Prettier; renderer import boundaries enforced (no Electron main, no Node built-ins).
- State separation (ui-arch): TanStack Query (workspace semantics) / Zustand (presentation) / DataStore (binary streams); high-rate telemetry never enters React global state.
- Zero DRM logic in the UI: validation, manifests, and mutations execute in the Rust engine over JSON-RPC; the renderer receives domain capabilities, never machine access.

### 3.6 Config and Documentation Files
- TOML: `taplo` fmt + check. YAML: `yamllint`. Markdown: `markdownlint` enforcing the terse house style. All three run in CI (§5.2).

---

## 4. Testing Standards

### 4.1 Levels
| Level | Location | Tooling | Scope | Cadence |
|---|---|---|---|---|
| Unit | per-crate `#[cfg(test)]`, crate `tests/` | built-in | module invariants | every PR |
| Property | per-crate | proptest | encode→decode round-trips, ring/pool algebra | every PR |
| Compile-fail | binding crates, schema fixtures | trybuild | forbidden borrow lifetimes, contract violations | every PR |
| Fuzz | `tests/fuzz` | cargo-fuzz | schema parser, framing, hazard protocol | PR smoke + nightly |
| Sanitizer | core crates | ASan/TSan/Miri | memory and races; Miri on lock-free modules (OS SHM excluded) | PR (core) + nightly |
| Conformance | `tests/conformance` | matrix harness | 4-language ABI + interop | every PR |
| Stress/chaos | `tests/stress` | harness | kill -9 at every borrow stage, PID reuse, pool exhaustion | nightly |
| Soak | `tests/soak` | harness | long-duration zero corruption/deadlock | weekly + W8 gate |
| Golden vectors | `tests/fixtures/golden/` | all bindings | byte-identical layouts C/C++/Rust/Python | every PR |
| Examples | `examples/` | CI jobs | sample workspace builds and runs headless | every PR |

### 4.2 Testing Rules
1. Tests cite REQ IDs in name or doc comment; the registry maps REQ → implementation → test → result (§6.2).
2. **Regression permanence**: once a phase's suite is green, it becomes a permanent blocking CI gate (work-plan).
3. Golden vectors are the single shared fixture set; bindings never maintain private layout expectations.
4. **Flaky policy**: quarantine label; repaired or deleted within one week; quarantine count is a tracked metric.
5. Zero-alloc audits run per release of each runtime phase; enforcement is tooling, never review alone.

---

## 5. Tooling and CI

### 5.1 xtask Commands
| Command | Purpose |
|---|---|
| `xtask fmt` | rustfmt, clang-format, ruff, prettier, taplo, markdownlint —i |
| `xtask lint` | clippy `-D warnings`, mypy, eslint, yamllint, markdownlint |
| `xtask test` | unit + property + doc tests, all languages |
| `xtask check-layering` | crate dependency-layer verification (§2.2 R6) |
| `xtask gen-check` | regenerate against fixtures; fail on diff (§3.1 R6) |
| `xtask audit-zero-alloc` | data-path allocation audit (§3.2) |
| `xtask audit-traceability` | REQ citation coverage (§3.1 R2) |
| `xtask docs` | build docs + execute examples headless |

`xtask` is the single automation entry point — cross-platform by construction (Windows is a first-class platform, Phase 31), no external runner binaries required.

### 5.2 Workflows
| Workflow | Trigger | Contents |
|---|---|---|
| `ci.yml` | every PR / push | Matrix: Linux x86_64 + aarch64, Windows x86_64. `xtask lint`, `xtask test`, conformance, `check-layering`, semver-checks, frontend lint/build |
| `sanitizers.yml` | PR (core paths) + nightly | ASan/TSan full matrix; Miri where applicable |
| `audit.yml` | nightly + release | cargo-deny, advisories, `audit-zero-alloc`, `audit-traceability`, `gen-check`, docs/examples execution, benchmark regression report |
| `release.yml` | tag `v*` | crates.io publish, Python wheels (maturin), Studio artifacts, SBOM, ABI compatibility check vs previous release |

Required checks: all `ci.yml` jobs. Nightly `audit.yml` failures auto-file issues.

### 5.3 Caching and Build Performance
- sccache (Rust) and ccache (C/C++) keyed per toolchain; pnpm store cache; incremental Rust compilation disabled in CI.
- Layer separation keeps the `drm-workspace` subtree parallel to the `drm-core` subtree.
- Codegen fingerprints plus timestamp preservation make regeneration a no-op for unchanged inputs (§3.1 R6).

---

## 6. Change Management

### 6.1 Versioning and Release
1. All `drm-*` crates share one lockstep version (0.x until API freeze); `Cargo.lock` committed.
2. `drm.h` carries an ABI version with a compatibility suite against previously supported versions (Phase 7 gate).
3. Releases flow only through `release.yml` (§5.2) — never ad-hoc publishing.

### 6.2 Requirement Registry
- Location: `docs/req/` — `registry.csv` (`id, area, statement, source_ref, phase, test, status`) plus per-area normative statements.
- Sync rule: the registry changes in the same PR as its architecture statement or implementation (work-plan risk register).
- Terminal gate: 100% REQ → implementation → test → passing result by Phase 34.

### 6.3 Decision Records
- `docs/adr/NNNN-<slug>.md` records any decision altering structure, layering, public API, or toolchain.
- Accepted ADRs are immutable; supersession creates a new record.
- ADRs capture rationale and context only; the architecture docs remain normative and win conflicts.

### 6.4 Deprecation and Breaking Changes
- Public crates: `cargo-semver-checks` blocks accidental breaking changes in CI.
- Deprecation: ≥1 minor release with changelog notice plus `#[deprecated]` / schema `deprecated:` annotation; removal only at a major or ABI-version bump.

### 6.5 Changelog and Commits
- Root `CHANGELOG.md`, Keep-a-Changelog format, updated in the same PR as any user-visible change.
- Commits follow Conventional Commits (`feat(core): ...`), scope = crate or area name; release notes are generated from them.

---

## 7. Governance

### 7.1 Ownership
| Role | Subsystems | CODEOWNERS paths |
|---|---|---|
| E1 | Runtime core, epochs, precommit, Windows port, trace/RPC runtime | `crates/drm-core`, `crates/drm-ffi`, `crates/drm-trace`, `crates/drm-rpc` |
| E2 | Toolchain, schema grammar, codegen, verify, spatial compiler | `crates/drm-workspace`, `crates/drm-spatial`, `docs/architecture.md` |
| E3 | Boundary router, transports, serial/embedded integration | `crates/drm-router` |
| E4 | Bindings, CI, fuzz/property/sanitizer infrastructure, benchmarks | `bindings/*`, `tests/*`, `.github/`, `xtask` |

Rotating test lead per wave owns the exit gate (work-plan). Two approvals required for `drm-core` and `drm-workspace` changes.

### 7.2 Licensing and Security
- Dual license: `MIT OR Apache-2.0`, matching the Rust ecosystem convention; `deny.toml` allow-list enforces third-party compatibility (MIT, Apache-2.0, BSD, ISC, Unicode-3.0).
- Vulnerability reports follow `SECURITY.md`; advisory scanning runs weekly in `audit.yml`.
