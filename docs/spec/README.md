# Derived Component Specs

Non-normative derived specifications: extraction of details that live in the architecture docs
and algorithm pseudocode but need a stable, standalone home (grammar, diagnostics vocabulary,
ABI policy, capability tables, routing derivation drafts). On any conflict the architecture docs
win; on any pseudocode conflict the algorithm docs win (codebase.md §2.4).

Owned by E2 unless noted. Written by the weekly plan named in each file's header.

| Spec | Phase | Source of truth |
|---|---|---|
| `schema-grammar.md` | 2 | `docs/algorithms/toolchain/schema-parser.md` §Grammar |
| `DIAGNOSTICS.md` | 2 | `docs/algorithms/toolchain/schema-parser.md` §Diagnostics Vocabulary |
| `routing-derivation.md` | 28 (pointer only; absorbed M09W3) | `docs/algorithms/toolchain/route-derivation.md` |
| `transport-capabilities.md` | 29 (draft M01) | `docs/algorithms/router/router-core.md`, `lanes-shaping.md`, `serial-mcu.md` |
| `metadata-layout.md` | 5 | `docs/architecture.md` §No Universal Envelope, `docs/algorithms/canon-codec.md` |
| `abi-policy.md` | 7 | `docs/algorithms/bindings/c-abi-surface.md`; ADR 0002 |
| `spatial-types-design.md` | 33 prep (M14W3) | `docs/spatial_architecture.md`; `docs/algorithms/spatial/spatial-core.md`; `docs/algorithms/toolchain/workspace-model.md` |
| `mcu-bring-up.md` | 32 prep (M14W3) | `docs/algorithms/router/serial-mcu.md`; `codebase.md` §3.3 |
| `rpc-design-seed.md` | 32 prep (M14W4) | `docs/algorithms/ops/rpc-actions.md`; `docs/architecture.md` §drm-rpc |
| `studio-prep-survey.md` | 32 prep (M14W4) | `docs/algorithms/studio/studio-protocol.md`; `docs/ui-architecture.md` |
| `tooling-design-lock.md` | 32 prep (M14W4) | `docs/algorithms/ops/*`, `docs/algorithms/router/serial-mcu.md`, `docs/algorithms/studio/studio-protocol.md` |
| `replay-fixtures.md` | 32 (M15W4) | `docs/algorithms/ops/record-replay.md` §§Invariants, Protocols, Test Matrix |

Specs here may not introduce requirements beyond the architecture; each lands with a `source_ref`
back to the owning architecture/algorithm section.
