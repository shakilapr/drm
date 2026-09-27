# DRM Directives

1. **Architecture-First**: Never edit implementation files (`src/`, `packages/`, `crates/`, `tests/`) without first updating and aligning with `docs/architecture.md`. Zero architectural drift.
2. **Reference-Only**: Never edit or modify files in `references/`. Strictly read-only for structural and algorithmic reference.
3. **Clean-Room**: Never copy code from references with licensing/IP conflicts (GPL/copyleft/proprietary). All features must be clean-room implemented from scratch.
4. **Brokerless & Single-Writer**: Decentralized local IPC over shared memory; no central broker, master node, or local daemon. Exactly one publisher per canonical topic across the domain; multiple inputs require an explicit multiplexer node.
5. **Transient Borrow Lifecycle**: Zero-copy shared-memory samples are strictly transient borrowed views protected by hazard slots. External I/O (disk writes, network routing, GPU compute, blocking third-party APIs) must **never** retain a borrowed sample; consumers needing extended lifetime must copy (`copy_latest()` / `copy_next()`). No cross-process reference counts; no arbitrary lease timeouts.
6. **Zero Data-Path Allocation**: Steady-state publishing and consumption must execute zero per-message heap allocations, free-list reallocations, or dynamic serialization. All channel storage is fixed at activation.
7. **Terse Style**: All documentation, agent responses, and code comments must be ultra-concise, dense, and use the fewest words possible.
8. **Verification**: All changes must pass 100% of test suites and static system verification (`drm verify`).
