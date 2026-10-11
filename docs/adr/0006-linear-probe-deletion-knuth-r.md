# ADR-0006: Linear-probe deletion — Knuth Algorithm R, not early-terminating backward shift

## Status

`accepted`
Date: 2026-10-11 · Deciders: E1 (runtime core), E2 (toolchain)

## Context and Problem Statement

The fixed-capacity incremental hash grid
(`crates/drm-spatial/src/grid.rs`, [spatial-index.md](../algorithms/spatial/spatial-index.md)
`I-X2`) mandates backward-shift deletion with no tombstones, so cell-hash
occupancy stays `≤ α_max` and probe bounds hold. The M21W3 probe-occupancy
campaign (`spatial_hash_occupancy`, 50–95% occupancy with colliding keys and
interleaved deletes) falsified the initial implementation: the backward-shift
scan **stopped at the first key that did not need to move** instead of
continuing past it. Keys stranded behind the new hole became unreachable, their
cells stayed `used`, and the freed-node count fell short, so re-insertion at the
declared bound returned `IndexFull` (`index full`).

Failure evidence (reproduced, pre-fix): filling `M = 512`, `S = 1024` and
removing every object left 32 phantom `used` cells and recovered none of the
stranded memberships; a subsequent re-insert of the same 512 objects failed with
`IndexFull` after 480 insertions.

## Decision Drivers

- `I-X2`: occupancy `≤ α_max`; deletion leaves no tombstone; probe bounds hold at
  worst-case occupancy.
- Zero-heap steady state (`I-X1`) and `O(k)` incremental updates (`I-X12`); the
  fix must not allocate on the update path.
- Deterministic, host↔MCU-reproducible behavior.

## Considered Options

- **Knuth Algorithm R backward shift** — chosen. Single in-place pass; skip a
  key while its home lies in the cyclic interval `(d, i]`, shift only a key whose
  home lies outside it, and continue the scan to the next occupied slot. No
  scratch, no allocation.
- **Cluster re-insertion** (collect the occupied run after the hole, clear it,
  re-insert each key) — correct but needs a scratch buffer sized to the cluster;
  rejected for the update path on the zero-heap/no-realloc grounds.
- **Tombstones** — rejected: violates `I-X2` (probe paths grow without bound).

Selection criterion was correctness of the probe invariant first, then
allocation-freedom; the two candidates were compared on those pre-stated terms
before implementation.

## Decision Outcome

Chosen: **Knuth Algorithm R** backward-shift deletion.

- The scan advances from the emptied slot `d`; for each occupied `i`, compute
  `r = home(key[i])`; if `r` is in the cyclic interval `(d, i]` the key stays,
  otherwise it is moved back into `d` and `d := i`. The scan terminates only at
  an unoccupied slot.
- No heap, no realloc, no tombstones; occupancy and probe bounds are restored to
  the declared invariants.

### Positive Consequences

- `spatial_hash_occupancy` is green from 50% to 95% occupancy with colliding keys
  and interleaved deletes; every live membership stays reachable.
- The declared membership pool is fully reclaimed by deletion, so re-insertion at
  the declared bound never spuriously returns `IndexFull`.
- No change to the zero-heap data path (`I-X1`) or to `O(k)` update cost
  (`I-X12`).

### Negative Consequences

- The deletion scan may traverse the whole probe cluster; still `O(cluster)`,
  bounded by the declared table size.

## Confirmation

`cargo test -p drm-spatial --test grid spatial_hash_occupancy` (50–95%
occupancy, colliding keys, interleaved deletes; every live membership reachable;
occupancy `≤ α_max`). Covered by `REQ-SPATIAL-013`, `REQ-SPATIAL-042`.

## More Information

- [spatial-index.md](../algorithms/spatial/spatial-index.md) `I-X2`, `I-X12`.
- ADRs are immutable; supersession creates a new record (codebase.md 6.3).
