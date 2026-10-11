# Tooling Design Lock (M14W4)

Non-normative derived spec. Consolidates the Phase 32 sub-designs into one lock record; each
sub-owner signs the row. Source of truth per sub-design as listed below. On any conflict the
architecture docs win; on any pseudocode conflict the algorithm docs win (codebase.md §2.4).
Owner: E4 (consolidation). Phase 32 prep. Written M14W4.

This record locks all Phase 32 tooling sub-designs (`echo`/`hz`/`log`, `diag`, `record`/replay,
`trace`, RPC/actions, serial/MCU) plus the Studio bridge before implementation starts at M15W1.
It restates pinned invariants only; no invariant changed and no new REQ ID was minted.

## 1. Sub-design lock table

| Sub-design | Owner | Locked design | Doc | Invariants | Sign |
|---|---|---|---|---|---|
| `echo`/`hz`/`log` | E4 | non-publishing introspection: momentary borrow → copy → release, zero held hazard slots; each owns a reserved diagnostic control record | [observability-plane.md](../algorithms/ops/observability-plane.md); [architecture.md](../architecture.md) §Tooling and Introspection | I-OB5, `REQ-OPS-003` | E4 ✓ |
| `diag` | E4 | out-of-band aggregation: `observed_rate`, per-consumer deadline state, link stats; epoch sandwich; timing compiled only when declared | [observability-plane.md](../algorithms/ops/observability-plane.md) | I-OB1..I-OB7, `REQ-OPS-001`/`REQ-OPS-002`/`REQ-OPS-016`, `REQ-CORE-093`/`REQ-CORE-094` | E4 ✓ |
| `record`/replay | E4 | borrow-copy-release, recorder-owned per-topic slabs, loss ledger, `on_full` {Drop,Stop,Backpressure}, MCAP framing, tiered replay | [record-replay.md](../algorithms/ops/record-replay.md) | I-RC1..I-RC6, `REQ-OPS-005`..`REQ-OPS-009`, `REQ-OPS-018`/`REQ-OPS-019` | E4 ✓ |
| `trace` | E1 | wait-free per-producer SPSC lanes, numeric IDs + build-time metadata dictionary, append-only `dict_version` | [observability-plane.md](../algorithms/ops/observability-plane.md) | I-OB1/I-OB2/I-OB6/I-OB7, `REQ-OPS-014`/`REQ-OPS-015`/`REQ-OPS-017` | E1 ✓ |
| RPC/actions | E1 | correlated topic pairs, `u64` cid per client session never reused, deterministic phase-boundary cancel, relative-budget deadlines, core untouched | [rpc-actions.md](../algorithms/ops/rpc-actions.md); [rpc-design-seed.md](rpc-design-seed.md) | I-RPC1..I-RPC11, `REQ-OPS-010`..`REQ-OPS-025` | E1 ✓ |
| serial/MCU | E3 | COBS bounded frames, CRC32C trailer, byte-for-byte host↔MCU interop, zero-forbidden-heap steady state, frozen bring-up/CI | [serial-mcu.md](../algorithms/router/serial-mcu.md); [mcu-bring-up.md](mcu-bring-up.md) | I-SR1..I-SR7, `REQ-ROUTE-024`..`REQ-ROUTE-026`/`REQ-ROUTE-040`, `REQ-CORE-086`/`REQ-CORE-087`, `REQ-OPS-004` | E3 ✓ |
| Studio bridge | E2 | `drm studio --stdio` JSON-RPC host, generated TS types pipeline, engine-only mutations, capability model | [studio-protocol.md](../algorithms/studio/studio-protocol.md); [studio-prep-survey.md](studio-prep-survey.md) | I-ST1..I-ST13, `REQ-STUDIO-001`..`REQ-STUDIO-014` | E2 ✓ |

## 2. Cross-cutting locks

- The observability plane never executes on the data path; introspection tools are non-publishing
  observers (I-OB2/I-OB5, `REQ-OPS-002`/`REQ-OPS-003`).
- The recorder is never an implicit required sequential consumer (I-RC3, `REQ-OPS-008`).
- Trace emission never stalls producers; drops are counted, never silent (I-OB1/I-OB6).
- RPC adds no core channel semantics and never creates a multi-writer topic (I-RPC4/I-RPC5).
- Serial is best-effort; `reliable` over serial is rejected at verify (I-SR2, `REQ-ROUTE-024`).
- Studio mutations run only in the Rust engine; the renderer holds no DRM logic (I-ST1/I-ST4).

## 3. Acceptance (M14W4)

- [x] All Phase 32 sub-designs consolidated into one lock record.
- [x] Each sub-owner signed its row.
- [x] Cross-cutting locks restated; no invariant changed.

Deltas: none — this record consolidates already-merged confirmations. Implementation of the
tooling wave lands from M15W1.

REQ: REQ-OPS-001, REQ-OPS-002, REQ-OPS-003, REQ-OPS-004, REQ-OPS-005, REQ-OPS-006,
REQ-OPS-007, REQ-OPS-008, REQ-OPS-009, REQ-OPS-010, REQ-OPS-011, REQ-OPS-012,
REQ-OPS-013, REQ-OPS-014, REQ-OPS-015, REQ-OPS-016, REQ-OPS-017, REQ-OPS-018,
REQ-OPS-019, REQ-OPS-020, REQ-OPS-021, REQ-OPS-022, REQ-OPS-023, REQ-OPS-024,
REQ-OPS-025, REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026, REQ-ROUTE-040,
REQ-CORE-086, REQ-CORE-087, REQ-STUDIO-001, REQ-STUDIO-002, REQ-STUDIO-003,
REQ-STUDIO-004, REQ-STUDIO-005, REQ-STUDIO-006, REQ-STUDIO-007, REQ-STUDIO-008,
REQ-STUDIO-009, REQ-STUDIO-010, REQ-STUDIO-011, REQ-STUDIO-012, REQ-STUDIO-013,
REQ-STUDIO-014.
