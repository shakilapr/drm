# Contributing to DRM

Normative rules: [AGENTS.md](AGENTS.md), [codebase.md](codebase.md), [docs/architecture.md](docs/architecture.md). Standards and cross-cutting conventions: [docs/standards.md](docs/standards.md).

## Prerequisites

- Rust (pinned via `rust-toolchain.toml`; rustup installs it automatically)
- Node 22 LTS + pnpm (frontend)
- Python 3.10+ (binding tests)
- CMake/Ninja + a C11/C++20 toolchain (binding tests)

## Workflow

1. Read `docs/architecture.md` before touching implementation (AGENTS R1: architecture-first).
2. Branch from `main`; Conventional Commits (`feat(core): ...`).
3. Cite requirement IDs (`REQ-<AREA>-NNN`, registry in `docs/req/`) on public functions, error branches, and tests.
4. Same-PR sync: architecture doc edit + algorithm doc edit + `docs/req/registry.csv` diff + `CHANGELOG.md` where a user-visible change exists; ADR for any structure/layering/public-API/toolchain decision (AGENTS R10/R11, codebase §6.3).
5. Run before pushing:
   ```
   cargo xtask fmt
   cargo xtask lint
   cargo xtask test
   ```
6. PR diff kept small (guidance ≤ ~400 lines). New crates declare layer + CODEOWNERS entry in the same PR (codebase §2.2 R5).

## Review

- CODEOWNERS routes review by subsystem (codebase §7.1).
- Two approvals for `drm-core` and `drm-workspace`.
- Untraceable code (no REQ citation) is rejected.
- Layering is checked mechanically (`xtask check-layering`, AGENTS R12).
- `references/` is local-only and git-ignored; it is never committed, and any reference copy reaching the repo is rejected (AGENTS R2/R3).

## Never

- Hand-edit generated code (`@generated` banner) — regenerate instead (AGENTS R13).
- Copy from `references/` (strict clean-room, AGENTS R3).
- Add allocation, logging, or blocking calls to runtime data paths (codebase §3.1 R5).
- Land an architecture-affecting divergence without an ADR and a same-PR arch/algorithm update (AGENTS R11).
