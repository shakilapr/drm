# ADR-0003: xtask is the only automation surface; command registry is authoritative

- **Status**: accepted
- **Date**: 2026-09-29
- **Deciders**: E4 (CI/tooling), all engineers (W1 review)

## Context

Automation was accreting as one-off scripts in prior projects; Windows is a
first-class platform (Phase 31) and CI must run identical gates locally and on
runners. codebase.md §5.1 defines core commands; extension commands accumulate
over the program and need a single registration point.

## Decision

All repository automation lives in the `xtask` crate. Commands are governed by
`xtask/README.md`: adding, renaming, or removing a command requires updating
that registry and this ADR in the same PR. No shell scripts, makefiles, or
per-language task runners are introduced. Secondary formatter/linter tools
(clang-format, ruff, prettier, taplo, markdownlint, mypy, eslint, yamllint)
degrade gracefully when absent locally and are installed by CI.

## Consequences

- `cargo xtask` works identically on Linux and Windows (no bash-only tooling).
- Missing tools cannot silently diverge gates: CI installs every tool; local
  skips print `skip <tool> (not installed)`.
- Command count growth is reviewable at the registry diff.
