# Contributing to DRM

Normative rules: [AGENTS.md](AGENTS.md), [codebase.md](codebase.md), [docs/architecture.md](docs/architecture.md).

## Prerequisites

- Rust (pinned via `rust-toolchain.toml`; rustup installs it automatically)
- Node 22 LTS + pnpm (frontend)
- Python 3.10+ (binding tests)
- CMake/Ninja + a C11/C++20 toolchain (binding tests)

## Workflow

1. Read `docs/architecture.md` before touching implementation (AGENTS R1: architecture-first).
2. Branch from `main`; Conventional Commits (`feat(core): ...`).
3. Cite requirement IDs (`REQ-<AREA>-NNN`, registry in `docs/req/`) on public functions, error branches, and tests.
4. Run before pushing:
   ```
   cargo xtask fmt
   cargo xtask lint
   cargo xtask test
   ```
5. PR diff kept small (guidance ≤ ~400 lines). New crates declare layer + CODEOWNERS entry in the same PR (codebase §2.2 R5).

## Review

- CODEOWNERS routes review by subsystem (codebase §7.1).
- Two approvals for `drm-core` and `drm-workspace`.
- Untraceable code (no REQ citation) is rejected.
- `references/` is read-only; any diff touching it fails CI.

## Never

- Hand-edit generated code (`@generated` banner) — regenerate instead.
- Copy from `references/` (strict clean-room, AGENTS R3).
- Add allocation, logging, or blocking calls to runtime data paths (codebase §3.1 R5).
