# DRM Directives

1. **Architecture-First**: Update/align `docs/architecture.md` + `codebase.md` before touching `crates/`, `bindings/`, `packages/`, `apps/`, `tests/`, `xtask/`. Zero drift.
2. **Reference-Only**: `references/` read-only.
3. **Clean-Room**: No copying GPL/copyleft/proprietary references; implement from scratch.
4. **Brokerless & Single-Writer**: Local IPC over shared memory; no broker, master, or daemon. One publisher per canonical topic; multiple inputs need an explicit multiplexer node.
5. **Transient Borrow Lifecycle**: Shared-memory samples are transient borrowed views guarded by hazard slots. External I/O (disk, network, GPU, blocking third-party APIs) must never retain a borrowed sample; extended lifetime needs copy (`copy_latest()`/`copy_next()`). No cross-process refcounts; no lease timeouts.
6. **Zero Data-Path Allocation**: Steady-state publish/consume: zero per-message heap allocs, free-list reallocs, dynamic serialization. Channel storage fixed at activation.
7. **Terse Style**: Docs, agent replies, code comments: fewest words possible.
8. **Verification**: Pass 100% of tests + `drm verify`.
9. **Commits**: Conventional prefix (`feat|fix|docs|refactor|perf|test|chore`); state what was done; no week numbers.
