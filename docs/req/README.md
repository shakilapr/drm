# Requirement Registry

Machine-readable traceability registry required by work-plan Phase 1 and consumed by `xtask audit-traceability` (codebase.md §3.1 R2, §6.2).

## Layout

- `registry.csv` — one row per requirement ID.
- `REQ-<AREA>.md` — normative statements grouped per area.

## ID Scheme

`REQ-<AREA>-<NNN>`, areas: `CORE | TOOL | ROUTE | SPATIAL | BIND | STUDIO`.
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
| `status` | `open \| implemented \| verified` |

## Sync Rule

The registry changes in the same PR as its architecture statement or its implementation (work-plan risk register: prevents false 100% coverage). `xtask audit-traceability` fails when a cited ID is missing from the registry or a registry row has no implementation/test mapping.
