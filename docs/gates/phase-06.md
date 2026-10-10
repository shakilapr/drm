# Phase 6 Gate — Topic Identity & Numeric Handle System

Evidence mapping, [work-plan.md](../../work-plan.md) #6 criteria ⇄ tests.
Implementation: `drm-workspace::model::handles` + `drm-cli::ids`
([handle-system.md](../algorithms/toolchain/handle-system.md)). Compaction
semantics locked by [ADR 0004](../adr/0004-ids-compaction.md).
Verdict: **PASS** (local; harness is cross-platform).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Same workspace rebuild preserves every assigned handle | `handle_stability::handle_determinism` (×100 identical `.drm/ids.json`), `handle_fresh_deterministic` (empty history ⇒ identical initial table) | PASS |
| Adding unrelated topics cannot renumber existing topics | `handle_stability::handle_add_stability` (append-only allocation; existing handles unchanged) | PASS |
| Deleting/readding behaves per defined policy | `handle_stability::handle_delete_readd` (tombstone reserved, not reused, until compaction), `handle_history_semantics` | PASS |
| Only `drm ids compact` changes established assignments | `handle_stability::handle_compact_repack` (dense re-pack, reservations dropped); CLI `compact_refuses_without_yes` (`drm-cli`) — `drm ids compact` without `--yes` refuses and writes nothing | PASS |

## Invariant coverage (handle-system.md I-H1–I-H7)

| Invariant | Test |
|---|---|
| I-H1 HistoryDeterministic | `handle_determinism` ×100, `handle_fresh_deterministic` |
| I-H2 AddNoRenumber | `handle_add_stability` |
| I-H3 DeleteReaddPolicy | `handle_delete_readd` |
| I-H4 CompactExplicit | `handle_compact_repack`, CLI `compact_refuses_without_yes` (no `--yes` ⇒ refuse, no write) |
| I-H5 NoSilentReuse | `handle_delete_readd`, `handle_history_semantics` (reservations never reissued pre-compaction) |
| I-H6 ExhaustionSafe | `handle_exhaustion` (typed error at the `u32` boundary, no overflow) |
| I-H7 BindingsHandleFree | handles never enter generated bindings (codegen-invalidation contract; no `ids.json` input to codegen) |

## Variables (M03W4)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 6)` | PASS | PASS |
| `HANDLE_E2E` | rebuild/add/delete/compact policy green (I-H1..I-H4) | `handle_stability` 7/7 + CLI compact gate |
| `HANDLE_DETERMINISM_X100` | byte-identical `.drm/ids.json` | `handle_determinism` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Requirements covered

REQ-TOOL-035 (add never renumbers), REQ-TOOL-036 (build stability),
REQ-TOOL-037 (canonical identity is the topic path, handle is representation),
REQ-TOOL-038 (only `drm ids compact` recalculates assignments),
REQ-TOOL-039 (compact integer for dispatch, never transmitted as identity),
REQ-TOOL-080 (committed assignment lockfile, no nondeterministic metadata),
REQ-TOOL-082 (assignment-time checked increment).

## Notes

- `drm assign <topic> --consume/--publish <node>` mutates the node manifest
  (the single source of truth) and re-verifies; it never writes handles into
  manifests. Round-trip: assign → manifest diff → `drm verify` green.
- Reconcile (REQ-TOOL-081) and lineage artefacts beyond compaction remain
  Phase 6 follow-ons; this gate covers the assignment/compaction core.
