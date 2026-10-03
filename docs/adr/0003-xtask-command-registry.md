# ADR-0003: xtask is the only automation surface; command registry is authoritative

## Status

`accepted`
Date: 2026-09-29 · Deciders: E4 (CI/tooling), all engineers (W1 review)

## Context and Problem Statement

Automation was accreting as one-off scripts in prior projects; Windows is a first-class platform (Phase 31) and CI must run identical gates locally and on runners. codebase.md 5.1 defines core commands; extension commands accumulate over the program and need a single registration point.

## Decision Drivers

- Identical gates locally and on runners.
- Windows first-class: no bash-only tooling.
- Command growth must be reviewable at a single diff.

## Considered Options

- **xtask crate only** — chosen.
- **Shell scripts / makefiles** — rejected: platform-divergent, unreviewable sprawl.
- **Per-language task runners** — rejected: duplicates the surface, no single registry.

## Decision Outcome

Chosen: all repository automation lives in the `xtask` crate. Commands are governed by `xtask/README.md`; adding, renaming, or removing a command requires updating that registry and this ADR in the same PR. No shell scripts, makefiles, or per-language task runners. Secondary formatter/linter tools (clang-format, ruff, prettier, taplo, markdownlint, mypy, eslint, yamllint) degrade gracefully when absent locally and are installed by CI.

### Positive Consequences

- `cargo xtask` works identically on Linux and Windows.
- Missing tools cannot silently diverge gates: CI installs every tool; local skips print `skip <tool> (not installed)`.

### Negative Consequences

- Every new automation needs a registry diff — deliberate friction.

## Confirmation

`cargo xtask` runs the full gate set on both platforms (existing, permanent).

## More Information

- Accepted ADRs are immutable; supersession creates a new record (codebase.md 6.3).
- Registry: `xtask/README.md`.
