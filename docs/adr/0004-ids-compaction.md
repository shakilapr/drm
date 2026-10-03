# ADR-0004: Handle compaction is explicit-only

## Status

`accepted`
Date: 2026-09-28 (decision origin; recorded 2026-10-03) · Deciders: E1 (runtime), E2 (toolchain)

## Context and Problem Statement

Topic handles are u32 identities used by routing tables, deployment artifacts, and the CCB. Deleted topics would either leave permanent tombstones (unbounded exhaustion) or allow silent reuse (identity instability for live consumers and persisted artifacts). The architecture requires a policy that keeps handles stable across normal operation while bounding exhaustion.

## Decision Drivers

- Handle stability: no silent reuse (I-H3).
- Exhaustion must be bounded and escape hatch must be deliberate.
- Deployment identity must change whenever handle numbering changes.

## Considered Options

- **Automatic periodic compaction** — rejected: renumbers live IDs, races with running consumers, silently invalidates persisted routing tables.
- **Tombstones forever** — rejected: unbounded exhaustion of the u32 space on churn-heavy deployments.
- **Explicit command only** — chosen.

## Decision Outcome

Chosen: only an explicit maintenance command (`drm ids compact`) recalculates and packs numeric IDs. Compaction regenerates all handle-dependent deployment artifacts (routing/runtime tables), creates a new deployment identity, and requires coordinated redeployment. It does not regenerate schema bindings (they never embed handles; codegen I-G9); only binaries/firmware that compile deployment tables (e.g. MCU topic tables) rebuild. `drm ids reconcile` applies the same deployment-identity rule when a committed `ids.json` merge changes assignments. Compaction requires an acknowledgement CLI flag (I-H4).

### Positive Consequences

- IDs stable across all normal operation; delete/readd reserves tombstones (no silent reuse).
- Exhaustion is bounded and its escape hatch is explicit, auditable, and requires operator acknowledgement.

### Negative Consequences

- Churn-heavy deployments can exhaust the u32 space until compaction is run.

## Confirmation

Tests: `handle_delete_readd`, `handle_compact_gate`, `handle_compact_scope` (handle-system.md test matrix).

## More Information

- Accepted ADRs are immutable; supersession creates a new record (codebase.md 6.3).
- Normative homes: `docs/architecture.md` (Explicit Handle Compaction, No Runtime Handles in Generated Bindings); `docs/algorithms/toolchain/handle-system.md` (I-H3, I-H4); `docs/algorithms/toolchain/build-orchestration.md`.
- Recorded retrospectively to close the CHANGELOG reference; the decision itself has been normative since the initial architecture.
