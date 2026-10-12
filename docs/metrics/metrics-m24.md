# Metrics — Month 24 (M24)

Normative source: [work-plan.md](../../work-plan.md) release wave and the M24
month plan. Month 24 closes the program: coverage closure + API freeze prep
(M24W1), API freeze + pilot (M24W2), Phase 34 gate (M24W3), v1.0 (M24W4). One
section per weekly tranche.

## M24W1 — Coverage Closure + API Freeze Prep

| Variable | Target | Measured |
|---|---|---|
| `CRIT_COVERAGE` | critical paths ≥ target | 6/6 — `cargo xtask qualify`; [m24w1.md](../gates/m24w1.md) |
| `API_INVENTORY` | full public API inventory + freeze candidate list | 1 — [inventory.rs](../../xtask/src/inventory.rs), [api-inventory.md](../spec/api-inventory.md) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Critical-path coverage** ([coverage.rs](../../xtask/src/coverage.rs)):
  `critical_paths` resolves the six declared critical paths — ring history,
  overflow policies, hazard protocol, exhaustion/reclamation, UDP, serial — to
  live linked tests and folds the leg into the qualification packet
  (`CRIT_COVERAGE`, `I-QAL8`, `REQ-QAL-020`). Policy and path table:
  [coverage-policy.md](../spec/coverage-policy.md).
- **Runtime uncovered-branch sweep** (E1): `channel::overflow::tests` (descriptor
  decode, ack watermark and monotonicity, `allow_commit` gate paths),
  `channel::pool::tests` (offset overflow, out-of-bounds destination, default
  pool sizes), `ring::tests::stamp_exhaustion_paths` (writer-active stamp and
  both stamp-exhaustion paths), `hazard::tests` (generation no-wrap, idempotent
  borrow release).
- **Transport uncovered-branch sweep** (E3):
  `transport::tests::recv_queue_zero_capacity_and_empty_pop` (drop-newest
  accounting at capacity 0, empty pop); UDP and serial critical paths resolved.
- **Mutation sample EXP** ([coverage.rs](../../xtask/src/coverage.rs)
  `mutation_sample`): a fixed-seed seed-and-detect sample over the hazard/ring/
  queue predicates; 4 seeded defect classes (`ring-boundary-flip`,
  `ring-future-inclusive`, `hazard-epoch-invert`, `queue-full-removed`), each
  detected by the linked oracle over hardcoded probe expectations
  (`I-QAL9`, `REQ-QAL-022`). Sample rate: 4 classes/run, documented for W8.
- **API inventory + freeze list** ([inventory.rs](../../xtask/src/inventory.rs)):
  the canonical C ABI (12 exports, 3 handles, 23 status codes), 7 public Rust
  items, and the 7 deferred post-v1.0 exports; `verify` re-resolves each entry
  against its owner, so a renamed/removed symbol fails `cargo xtask qualify`
  (`REQ-QAL-021`). Reviewable table:
  [api-inventory.md](../spec/api-inventory.md).
- **Qualification packet** ([qualify.rs](../../xtask/src/qualify.rs)): the packet
  gains the coverage, mutation, and inventory legs; `is_complete` gates on them
  and on the existing trace/conformance/fault/soak/evidence legs
  (`REQ-QAL-018`).

### Weekly gate checklist (chaired by E4, Friday)

- [x] `CRIT_COVERAGE = met`; `API_INVENTORY = 1`; mutation sample clean
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M24W1) = PASS`**

### How to reproduce

```text
cargo xtask qualify --check --json target/qualification-packet.json
cargo xtask test
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask check-layering && cargo xtask audit-traceability
```

REQ: REQ-QAL-020, REQ-QAL-021, REQ-QAL-022, REQ-QAL-006, REQ-QAL-018,
REQ-CORE-040, REQ-CORE-066, REQ-CORE-058, REQ-CORE-030, REQ-ROUTE-032,
REQ-ROUTE-024.
