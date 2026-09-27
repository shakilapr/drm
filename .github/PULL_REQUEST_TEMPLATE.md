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
- [ ] Generated code untouched (regenerate instead, §3.1 R6)
- [ ] `references/` untouched
