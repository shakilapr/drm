# DRM Algorithm Specifications

Canonical pseudocode for every DRM subsystem, grouped by module. Implementation PRs that diverge from pseudocode **must update the algorithm doc in the same PR** (codebase.md R1/R2 sync discipline, same as the REQ registry).

**Reference direction**: work plans reference algorithms; algorithms never reference work plans — each doc below is self-contained per subsystem, anchored to architecture sections. Architecture remains normative: [architecture.md](../architecture.md), [spatial_architecture.md](../spatial_architecture.md), [ui-architecture.md](../ui-architecture.md). Algorithm docs translate normative rules into implementable pseudocode; on conflict, architecture wins.

## Conventions

1. **Style**: Rust-flavored pseudocode (`fn`, `match`, `Option`, `Result`, `enum`) for functions; **numbered steps** for cross-process protocols (SHM, locks, recovery). Pseudocode is a reviewable contract, not compilable code — implementation details (error paths via `?`, interior mutability) are elided.
2. **REQ citations**: docs use provisional requirement slugs (`REQ-AREA-<slug>`, areas `CORE|TOOL|ROUTE|SPATIAL|BIND|STUDIO|OPS|QAL` per codebase.md R7). Numeric IDs (`REQ-TOOL-001…`) are bound in `docs/req/registry.csv` during requirement extraction; each doc carries a `## REQ Bindings` table updated in the same PR as extraction.
3. **Invariants**: `I1, I2, …` — each invariant names the test that proves it. Tests live in the locations fixed by codebase.md §4 (per-crate / `tests/{conformance,stress,soak,fuzz,fixtures}`).
4. **Error taxonomy**: every error path maps to `drm-types::Status` (codebase R4). Docs state the mapping; no panics on data paths.
5. **Templates**: every doc follows §Doc Template below.

## Index

All 36 algorithm documents are **written**. Grouped by module; three cross-cutting docs sit at the root.

### Shared (cross-cutting)

| Doc | Scope |
|---|---|
| [canon-codec.md](canon-codec.md) | Canonical binary layout: toolchain layout calc (`drm-workspace`) + runtime codec (`drm-core`); layout-fixture exchange gate |
| [memory-budget.md](memory-budget.md) | Deployment memory budgets: worst-case verifier math + runtime precommit protocol |
| [deployment-identity.md](deployment-identity.md) | Three identities: fingerprints, namespaces, startup guard, cleanup |

### runtime/ — `drm-core`

| Doc | Phase(s) | Scope |
|---|---|---|
| [runtime/shm-backend.md](runtime/shm-backend.md) | 8 | Shared-memory create/classify/map, locks, fork sanitization, mlock precommit (Linux) |
| [runtime/ring-history.md](runtime/ring-history.md) | 10 | Sequence-history ring, retention, commit/lookup ordering |
| [runtime/channel-metadata.md](runtime/channel-metadata.md) | 5, 17 | Channel descriptor, attach validation, epoch observe/reset |
| [runtime/consumer-registration.md](runtime/consumer-registration.md) | 14–16 | Consumer record claim, close, dead takeover, generation-safe reuse |
| [runtime/cursor-delivery.md](runtime/cursor-delivery.md) | 11 | Reader cursors, borrow/copy × latest/next, exact GAP algebra |
| [runtime/overflow-policies.md](runtime/overflow-policies.md) | 12 | Overwrite lapping, reject gating, required sequential consumers |
| [runtime/payload-pool.md](runtime/payload-pool.md) | 13 | Fixed slot pools, offsets, pool invariant, drop-free sizing |
| [runtime/hazard-protocol.md](runtime/hazard-protocol.md) | 14 | 6-step acquisition, publisher slot selection, ABA, race matrix R1–R6 |
| [runtime/channel-publish.md](runtime/channel-publish.md) | 12–15 | Integrated publish transaction, linearization, error precedence |
| [runtime/exhaustion-reclamation.md](runtime/exhaustion-reclamation.md) | 15, 16 | Drop accounting, registration-lock reclamation slow path, RT off-path |
| [runtime/publisher-lifecycle.md](runtime/publisher-lifecycle.md) | 17 | Writer authority, epoch exactly-once, restart protocol, freshness |
| [runtime/windows-backends.md](runtime/windows-backends.md) | 31 | Sections, `LockFileEx`, diagnostic identity, `VirtualLock` precommit, Init reinit |

### toolchain/ — `drm-workspace` + `drm-cli`

| Doc | Phase(s) | Scope |
|---|---|---|
| [toolchain/schema-parser.md](toolchain/schema-parser.md) | 2 | `.drm` lexing/parsing, diagnostics vocabulary, deterministic render |
| [toolchain/workspace-model.md](toolchain/workspace-model.md) | 23 | WorkspaceModel construction, contracts, fingerprints, determinism |
| [toolchain/handle-system.md](toolchain/handle-system.md) | 6 | Stable runtime handles, committed lockfile, merge reconciliation, explicit compaction |
| [toolchain/codegen-invalidation.md](toolchain/codegen-invalidation.md) | 24 | Generators, directed fingerprints, content-compared atomic writes, reconciliation |
| [toolchain/build-orchestration.md](toolchain/build-orchestration.md) | 25 | Orchestrator flow, native adapters, watch mode + event-loss recovery |
| [toolchain/verify-engine.md](toolchain/verify-engine.md) | 26 | Deterministic-fact check catalogue (V-*), facts/estimates boundary |
| [toolchain/route-derivation.md](toolchain/route-derivation.md) | 28 | Graph routing, ambiguity detection, prefer/deny/force/export_limit, external vertices |
| [toolchain/feasibility-estimation.md](toolchain/feasibility-estimation.md) | 27 | Link-utilization math, route shaping, adapter wire sizing, honest labels |

### router/ — `drm-router` + transports

| Doc | Phase(s) | Scope |
|---|---|---|
| [router/router-core.md](router/router-core.md) | 29 | Egress/ingress framing, scoped egress admission, fan-out branch isolation, two-way import, session lifecycle |
| [router/lanes-shaping.md](router/lanes-shaping.md) | 30 | Per-link scheduler (all transports), bounded priority + DRR, token buckets, ordered lanes for reliable streams |
| [router/serial-mcu.md](router/serial-mcu.md) | 32 | Bounded serial frames, CRC/escaping, blocking bound, MCU freestanding subset, zero-heap barriers |

### bindings/ — `drm-ffi` + first-party bindings

| Doc | Phase(s) | Scope |
|---|---|---|
| [bindings/c-abi-surface.md](bindings/c-abi-surface.md) | 7, 20–22 | Context/publisher/consumer handles, validate-once, copy + write-loan publish, borrow/copy reads, fixed int32 statuses, fail-stop, one-way ABI compat |
| [bindings/bindings-cpp-rust-python.md](bindings/bindings-cpp-rust-python.md) | 20–22 | C++ RAII owning accessor, Rust `!Send !Sync` guards, Python owned-reads-only + threading, two-layer codegen |

### ops/ — `drm-record` / `drm-diag` / `drm-trace` / `drm-rpc`

| Doc | Phase(s) | Scope |
|---|---|---|
| [ops/record-replay.md](ops/record-replay.md) | 32 | Borrow-copy-release pipeline, per-topic slab pools, loss-ledger gap accounting, MCAP + DRM metadata records, tiered replay fidelity with scheduler |
| [ops/observability-plane.md](ops/observability-plane.md) | 32 | diag aggregation, per-consumer deadline state, `timing=auto/exact/off`, per-producer SPSC trace lanes, epoch-consistent snapshots |
| [ops/rpc-actions.md](ops/rpc-actions.md) | 32 | Reactor/worker RPC over one ordered per-client event stream; admission, bounded outbox, sessions, relative deadlines, cooperative cancel |

### studio/ — Studio host, protocol, extensions

| Doc | Phase(s) | Scope |
|---|---|---|
| [studio/studio-protocol.md](studio/studio-protocol.md) | 32 | JSON-RPC engine bridge, engine-only mutations, capability model, extension budgets |

### spatial/ — `drm-spatial`

| Doc | Phase(s) | Scope |
|---|---|---|
| [spatial/spatial-core.md](spatial/spatial-core.md) | 33 | Typed quantities, compile/dynamic spaces, `[spatial]` contracts, four clock domains, geodetic, overlay handles, MCU constants |
| [spatial/spatial-queries.md](spatial/spatial-queries.md) | 33 | Compiled transform plans, latest/at/predict/delayed_view, interpolation guard, temporal reparenting, calibration slots |
| [spatial/spatial-index.md](spatial/spatial-index.md) | 33 | Incremental fixed-capacity hash grid (grid epoch), motion slack, dependency invalidation, frame-typed proximity, compiled BVH with live calibration slots, memory verification |

### program/ — qualification

| Doc | Phase(s) | Scope |
|---|---|---|
| [program/qualification.md](program/qualification.md) | 34 | Evidence-classed traceability + mutation audit, soak/aging/accelerated gates, per-platform reproducible builds, fuzz resolution gate, matrix closure, defect ledger |

## Doc Template

```markdown
# <Subsystem> Algorithms
Scope: <one paragraph> · Normative source: architecture.md §<section>
## REQ Bindings            <!-- provisional slugs → numeric IDs after registry extraction -->
## Data Structures         <!-- field / type / offset / atomic-ordering tables -->
## Invariants              <!-- I1..In, each bound to a named test -->
## Functions               <!-- Rust-flavored pseudocode blocks -->
## Protocols               <!-- numbered steps for cross-process sequences -->
## Edge Cases & Errors     <!-- mapping to drm-types::Status -->
## Test Matrix             <!-- named case → expected outcome -->
```
