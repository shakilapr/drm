# Metrics — Month 17 (M17)

Normative source: [work-plan.md](../../work-plan.md) W6 operational tooling →
Studio transition. Phase 32 close-out (introspection/trace/RPC gates) opens the
month; the Phase 32 gate and the Studio protocol doc land at M17W2. One section
per weekly tranche.

## M17W1 — Phase 32 Close-Out: Introspection + Trace + RPC Gates

| Variable | Target | Measured |
|---|---|---|
| `INTROSPECT_GATE` | echo/hz/log read-only, no-pin (I-OB5), rate tolerance (I-OB3) | PASS — `drm-diag` 10/10: `introspect_no_pin` (a continuous `copy_latest` observer holds zero hazard slots — `rec0.hazard == HAZARD_NONE` — and the pool stays exhaustible: holding the sole free slot makes `publish` return `PoolExhausted`, proving the observer consumed no capacity), `diag_observed_rate` (10/100/1000 Hz within 1%; a 100-sample burst reports its own high rate and decays to idle), `diag_no_overhead` (sampling leaves every counter untouched), `observer_copies_owned_bytes`/`log_level_filter` (read-only copy + filter) |
| `TRACE_GATE` | never-blocks, drop transparency (I-OB1/I-OB6) | PASS — `drm-core --test trace` 4/4: `trace_never_blocks` (1/2/4/16 producers, stopped reader, tail cost bounded < 60 s; `dropped = producers × (N − CAP)` exactly), `trace_drop_counter` (full lane retains exactly capacity 4 of 10, `total_dropped = 6`), `trace_lane_exhaustion` (`no_lane_drops`, no dynamic growth), `trace_dict_roundtrip` |
| `RPC_GATE` | correlation, cancel, timeout (I-RPC1..I-RPC3) | PASS — `drm-rpc` 12/12: `dispatch_concurrent_no_cross_correlation` (`C ∈ 1..=16`, every client gets its own response, zero stale), `cancel_at_each_phase_boundary` (`k ∈ 0..4`, exactly one `CancelAccepted`, no phase after the token, `Terminal(Cancelled)`), `dispatch_timeout_expires_and_late_terminal_stale` (`RpcTimeout` + best-effort `Cancel`; late terminal dropped stale), NEW `rpc_capacity_busy_gap_forward` (client table full ⇒ `RpcError::Busy`; server table full consumes nothing; retained request forwarded once a slot frees — I-RPC6/I-RPC7) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **RPC capacity + gap-forwarding** (`crates/drm-rpc/src/tests.rs`
  `rpc_capacity_busy_gap_forward`): the `rpc_capacity` cell of the rpc-actions
  test matrix — a full client correlation table returns `RpcError::Busy`; a full
  server operation table leaves the request published; after the admitted
  operation completes, the retained request is admitted on the next reactor pass
  and served (`publish ≠ accepted`, `I-RPC6`/`I-RPC7`).
- **MCU regression wiring** (`.github/workflows/ci.yml` `mcu` job;
  `.github/workflows/mcu-cross.yml` `push: [main]`): the host parity self-test
  and the zero-forbidden-heap symbol audit are now permanent blocking gates on
  every PR/push (regression permanence, AGENTS #14).
- **Gate evidence** ([m17w1.md](../gates/m17w1.md)): I-OB1/I-OB3/I-OB5/I-OB6 and
  I-RPC1..I-RPC3 re-run as close-out evidence; Phase 32 tracker
  [phase-32.md](../gates/phase-32.md) finalized with the deferred post-1.0 Studio
  UI scope list; [CHANGELOG.md](../../CHANGELOG.md) M17W1 tranche.

Notes:

- No architecture divergence: pinned invariants only (observability-plane
  I-OB1/I-OB3/I-OB5/I-OB6, rpc-actions I-RPC1/I-RPC2/I-RPC3/I-RPC6/I-RPC7); no
  ADR.
- The 2 h MCU target soak remains deferred to the `mcu-cross` hardware lane; host
  parity and the zero-forbidden-heap audit are green in-session and now required
  in CI.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `INTROSPECT_GATE = PASS`; `TRACE_GATE = PASS`; `RPC_GATE = PASS`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M17W1) = PASS`**

### How to reproduce

```text
cargo test -p drm-diag
cargo test -p drm-core --test trace
cargo test -p drm-rpc
make -C bindings/c/mcu host-test host-audit
cargo xtask audit-traceability
cargo xtask check-layering
```
