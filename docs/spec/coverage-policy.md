# Coverage Policy — Critical Paths and Mutation Sample

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
Governs the M24W1 coverage closure (`REQ-QAL-020`, `REQ-QAL-022`; invariants
`I-QAL8`/`I-QAL9`). Owner: E4 (test lead), with E1 (runtime) and E3
(transports).

## Measurement

Coverage here is **critical-path coverage**, not line count. A declared critical
path is *resolved* when its implementation owner exists, cites its `REQ` anchor,
and every linked test file exists and carries at least one Rust test marker
(`#[test]` / `#[cfg(test)]`). `cargo xtask qualify` re-resolves each path and
fails `--check` while any path is a gap (`CRIT_COVERAGE = resolved/total`). This
is structural and toolchain-free; it complements (never replaces) the
traceability matrix (`I-QAL1`).

The gate job is the CI `coverage` job, which runs `cargo xtask qualify --check`
and the workspace tests.

## Declared critical paths

| Path | Owner | REQ anchor | Linked tests |
|---|---|---|---|
| ring-history | [ring/mod.rs](../../crates/drm-core/src/ring/mod.rs) | `REQ-CORE-066` | inline `ring::tests` (retention, wrap, release order, stamp exhaustion) |
| overflow-policies | [overflow.rs](../../crates/drm-core/src/channel/overflow.rs) | `REQ-CORE-040` | inline `channel::overflow::tests` (byte decode, ack watermark, commit gate) |
| hazard-protocol | [hazard/mod.rs](../../crates/drm-core/src/hazard/mod.rs) | `REQ-CORE-030` | inline `hazard::tests` + race/select/aba forcing modules |
| exhaustion-reclamation | [reclaim.rs](../../crates/drm-core/src/channel/reclaim.rs) | `REQ-CORE-058` | inline `channel::reclaim::tests` + `tests/stress/chaos_kill_matrix.rs` |
| transport-udp | [udp.rs](../../crates/drm-router/src/transport/udp.rs) | `REQ-ROUTE-032` | inline `transport::udp::tests` |
| transport-serial | [serial.rs](../../crates/drm-router/src/transport/serial.rs) | `REQ-ROUTE-024` | inline `transport::serial::tests` |

Target (`CRIT_COVERAGE`): every declared path resolves (6/6).

## Mutation sample (`I-QAL9`)

Line/branch coverage can be satisfied by assertions that would not catch a real
defect. The mutation sample validates assertion strength: a fixed set of seeded
defects on the hazard/ring/queue critical predicates must each be detected by the
linked oracle over a fixed probe set with **hardcoded expected outputs** (so the
oracle cannot pass vacuously).

| Class | Seeded defect | Oracle surface |
|---|---|---|
| `ring-boundary-flip` | window floor `>=` weakened to `>` | exact retention window boundaries |
| `ring-future-inclusive` | future bound `s < ws` widened to `s <= ws` | future-sequence rejection |
| `hazard-epoch-invert` | registration-generation equality inverted | stale-generation protection rule |
| `queue-full-removed` | required-consumer queue-full handling removed | `Reject` retirement gating |

**Sample rate**: 4 defect classes per run (fixed seed, fixed probe set); this is
a targeted sample, not an exhaustive mutation campaign. W8 (final gate) re-runs
the sample; an undetected seeded defect blocks the gate and the linked test is
strengthened before close. The machine form is
[xtask/src/coverage.rs](../../xtask/src/coverage.rs) (`mutation_sample()`).

## M24W1 closure additions

The runtime and transport uncovered-branch sweep added, with no refactors
(work-plan risk cap):

- `channel::overflow::tests` — descriptor-byte decode, ack watermark init and
  monotonicity, sequence exhaustion, and the `allow_commit` gate paths
  (overwrite pass-through, zero-required `InvalidConfig`, within-depth pass,
  `QueueFull` retirement gate).
- `channel::pool::tests` — checked offset overflow (`InvalidConfig`), out-of-bounds
  destination rejection, and the declared default pool sizes.
- `ring::tests::stamp_exhaustion_paths` — writer-active stamp (`InvalidData`) and
  both stamp-exhaustion paths (`preflight` `+2`, `repair` `+1`).
- `hazard::tests` — generation-bump no-wrap and idempotent borrow release.
- `transport::tests::recv_queue_zero_capacity_and_empty_pop` — zero-capacity
  drop-newest accounting and empty pop.

REQ: REQ-QAL-020, REQ-QAL-022.
