# Requirement Registry

Machine-readable traceability registry required by work-plan Phase 1 and consumed by `xtask audit-traceability` (codebase.md §3.1 R2, §6.2).

## Layout

- `registry.csv` — one row per requirement ID.
- `REQ-<AREA>.md` — normative statements grouped per area.

## ID Scheme

`REQ-<AREA>-<NNN>`, areas: `CORE | TOOL | ROUTE | SPATIAL | BIND | STUDIO | OPS | QAL`.
Examples: `REQ-CORE-042`, `REQ-TOOL-011`.

## registry.csv Columns

| Column | Meaning |
|---|---|
| `id` | Unique requirement ID |
| `area` | CORE, TOOL, ROUTE, SPATIAL, BIND, STUDIO |
| `statement` | One-line normative statement |
| `source_ref` | Architecture doc + anchor (e.g. `architecture.md#zero-copy-memory-lifecycle`) |
| `phase` | Work-plan phase that implements it (1–34) |
| `test` | Test path or suite owning the verification |
| `status` | `open \| specified \| implemented \| verified` |

## Sync Rule

The registry changes in the same PR as its architecture statement or its implementation (work-plan risk register: prevents false 100% coverage). `xtask audit-traceability` fails when a cited ID is missing from the registry or a registry row has no implementation/test mapping.

## Schema Normalization (separate mechanical migration)

Registry schema is normalized by one idempotent transform, kept **out of** semantic/algorithm changesets:

- `area` ∈ `CORE | TOOL | ROUTE | SPATIAL | BIND | STUDIO | OPS | QAL` (codebase.md R7).
- `phase` ∈ integers `1..34` (work-plan). A legacy area-name in `phase` is resolved from the owning document's declared `Phase N`; a row resolvable to more than one/or no phase is **flagged for manual resolution, never guessed**.
- `status` ∈ `open | specified | implemented | verified`.
- `id` unique; every `REQ-*` cited anywhere under `docs/` exists in the registry (no dangling citations).

`migrate-registry.ps1` performs the transform; `-Check` validates only and writes nothing. The migration is accepted only when the validator reports **0 duplicate IDs, 0 dangling citations, and all `area`/`phase`/`status` valid**; otherwise it refuses to write and lists the offending rows. Land it as a standalone commit so routing/algorithm semantics are verifiably unchanged before and after.
