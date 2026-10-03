# DRM Directives

1. **Architecture-First**: Update/align the normative docs (`docs/architecture.md`, `docs/spatial_architecture.md`, `docs/ui-architecture.md`) + `codebase.md` before touching `crates/`, `bindings/`, `packages/`, `apps/`, `tests/`, `xtask/`. Zero drift.
2. **Reference-Only**: `references/` read-only.
3. **Clean-Room**: No copying GPL/copyleft/proprietary references; implement from scratch.
4. **Brokerless & Single-Writer**: Local IPC over shared memory; no broker, master, or daemon. One publisher per canonical topic; multiple inputs need an explicit multiplexer node.
5. **Transient Borrow Lifecycle**: Shared-memory samples are transient borrowed views guarded by hazard slots. External I/O (disk, network, GPU, blocking third-party APIs) must never retain a borrowed sample; extended lifetime needs copy (`copy_latest()`/`copy_next()`). No cross-process refcounts; no lease timeouts.
6. **Zero Data-Path Allocation**: Steady-state publish/consume: zero per-message heap allocs, free-list reallocs, dynamic serialization. Channel storage fixed at activation.
7. **Terse Style**: Docs, agent replies, code comments: fewest words possible.
8. **Verification**: Pass 100% of tests + `drm verify`.
9. **Commits**: Conventional prefix (`feat|fix|docs|refactor|perf|test|chore`); state what was done; no week numbers.
10. **Traceability**: Every public function, error branch, and test cites ≥1 registry ID (`docs/req/registry.csv`); registry changes land in the same PR as the architecture statement or implementation. Enforced by `xtask audit-traceability`.
11. **Falsification & ADR**: Implementation may falsify architecture. Stop, capture failure evidence, compare ≥2 alternatives on pre-stated criteria, record an ADR (`docs/adr/NNNN-<slug>.md`), update architecture + algorithm doc in the same PR, then implement. No arch-affecting divergence without an ADR.
12. **Layering**: `drm-types` zero deps; `drm-workspace` never depends on `drm-core`; ops crates never depend on each other. Enforced by `xtask check-layering`.
13. **Generated Code**: Generated artifacts carry a `@generated` banner and are never hand-edited; regenerate and commit the diff.
14. **Regression Permanence**: A green phase suite becomes a permanent blocking gate; flaky tests are quarantined and repaired or deleted within one week.
15. **Standards**: External standards and cross-cutting conventions registry: `docs/standards.md`; documents point there instead of restating.
