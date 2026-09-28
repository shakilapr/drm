<!-- Conventional Commits title: feat|fix|docs|refactor|test|chore(scope) -->

## Summary

<One paragraph.>

## Requirement traceability

<REQ IDs touched (docs/req/registry.csv) or "n/a (docs only)"> — codebase.md §3.1 R2.

## Architecture alignment

<Architecture docs changed? Link. If implementation changed semantics without an arch edit, stop.>

## Verification

- [ ] `cargo xtask lint`
- [ ] `cargo xtask test`
- [ ] `cargo xtask check-layering` (new crates: layer + CODEOWNERS added, §2.2 R5)
- [ ] Algorithm doc updated in the same PR if pseudocode diverged (algorithms/README.md)
- [ ] Generated code untouched (regenerate instead, §3.1 R6)
- [ ] ADR added for any structure/layering/public-API/toolchain decision (AGENTS R11)
- [ ] `references/` untouched and not committed (local-only, AGENTS R2/R3)
