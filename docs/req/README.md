# Requirement Registry

Machine-readable traceability registry required by work-plan Phase 1 and consumed by `xtask audit-traceability` (codebase.md §3.1 R2, §6.2). Requirement engineering conventions follow `docs/standards.md` (adopted as concepts, no external standard named).

## Requirement Quality

- Every statement is a single normative sentence with a unique ID; IDs are append-only and never reused.
- Statements are testable: an implementation or test must be able to verify or refute them.
- Traceability is bidirectional and enforced: architecture statement ⇄ requirement ID ⇄ implementation ⇄ verification test. A change to any one leg lands in the same PR as the others.
- Test management chain: requirement → test condition → test case → procedure/environment → execution → actual result → pass/fail → traceability record. The `test` column names the chain's terminal anchor.

## Layout

- `registry.csv` — one row per requirement ID (single source of truth; no separate per-area files).
- `migrate-registry.ps1` — schema-normalization/validation helper (see below).

## ID Scheme

`REQ-<AREA>-<NNN>`, areas: `CORE | TOOL | ROUTE | SPATIAL | BIND | STUDIO | OPS | QAL`.
Examples: `REQ-CORE-042`, `REQ-TOOL-011`. IDs are append-only; a retired ID is never reused to name a different requirement.

## registry.csv Columns

| Column | Meaning |
|---|---|
| `id` | Unique requirement ID |
| `area` | `CORE \| TOOL \| ROUTE \| SPATIAL \| BIND \| STUDIO \| OPS \| QAL` |
| `statement` | One-line normative statement |
| `source_ref` | Architecture anchor + owning algorithm doc, e.g. `docs/architecture.md; docs/algorithms/runtime/hazard-protocol.md` |
| `phase` | Work-plan phase (1–34) whose gate verifies it; resolved during Phase-1 extraction, never guessed |
| `test` | Planned verification owner: algorithm Test Matrix anchor (`…/doc.md#test-matrix`) until the test exists, then the crate/`tests` path |
| `status` | `open \| specified \| implemented \| verified` |

## Sync Rule

The registry changes in the same PR as its architecture statement or its implementation (work-plan risk register: prevents false 100% coverage). `xtask audit-traceability` fails when a cited ID is missing from the registry or a registry row has no implementation/test mapping at the terminal gate (codebase.md §6.2).

## Phase Resolution (Phase-1 extraction)

`phase` is per requirement, not per document: an algorithm doc may declare several phases (`Phases 5, 17, 18`), and different rows from the same doc belong to different phases. Resolution rule:

- A row resolves only when its owning doc(s) yield exactly one candidate phase.
- Multi-phase or phase-less rows are **flagged for manual resolution, never guessed**; the flag list is reviewed in the Phase-1 extraction pass.

## Schema Normalization (separate mechanical migration)

Registry schema is normalized by one idempotent transform, kept **out of** semantic/algorithm changesets:

- `area` ∈ `CORE | TOOL | ROUTE | SPATIAL | BIND | STUDIO | OPS | QAL` (codebase.md R7).
- `phase` ∈ integers `1..34` (work-plan), resolved per the rule above; a legacy area-name in `phase` is resolved from the owning doc's declared `Phase N`/`Phases N, M`; an unresolvable row is **flagged, never guessed**.
- `status` ∈ `open | specified | implemented | verified`.
- `id` unique; every `REQ-*` cited anywhere under `docs/` exists in the registry (no dangling citations).

`migrate-registry.ps1` performs the transform; `-Check` validates only and writes nothing. The migration is accepted only when the validator reports **0 duplicate IDs, 0 dangling citations, and all `area`/`phase`/`status` valid**; otherwise it refuses to write and lists the offending rows. Land it as a standalone commit so routing/algorithm semantics are verifiably unchanged before and after.
