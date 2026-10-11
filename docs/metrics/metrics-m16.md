# Metrics — Month 16 (M16)

Normative source: [work-plan.md](../../work-plan.md) W6 operational tooling.
Phase 32 continues with the RPC implementation start and record/replay gate prep.
One section per weekly tranche.

## M16W1 — RPC Implementation Start, Record/Replay Gate Prep

| Variable | Target | Measured |
|---|---|---|
| `RPC_CALL_GREEN` | call/response round-trip green | 1 — `drm-rpc` `call_round_trip`: `call` submits, the reactor admits (`Accepted`, session echo) and the bounded worker commits `Terminal`; the client observes the server session. `call_remote_failure` maps a non-success handler to `Terminal(Aborted)` + `RpcError::Remote` |
| `RPC_CID_GREEN` | concurrent dispatch green (I-RPC1 subset) | 1 — `drm-rpc` `dispatch_concurrent_no_cross_correlation` (C = 4 clients, distinct responses, zero stale), `dispatch_admission_busy` (reserve-then-consume, `I-RPC7`), `dispatch_timeout_expires_and_late_terminal_stale` (`RpcTimeout` + late terminal stale, `I-RPC3`), `server_session_stale_dropped` (`I-RPC9`), `event_order_invalid_terminal_first` (`I-RPC10`), `cancel_accepted_terminal_cancelled` (`I-RPC2` subset) |
| `REPLAY_E2E` | replay round-trip fixture green (I-RC4) | 1 — `drm-record` `replay_roundtrip`: record (injected history lap + policy drops) → wipe → decode → payload bytes, per-topic `(publisher_epoch, seq)`, global `(recv_mono_ns, ingress_order)`, gaps, source timestamps |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **RPC crate** (`crates/drm-rpc`, ops): `#![forbid(unsafe_code)]`; depends on
  `drm-types` + `drm-core` only.
  - `codec` — bounded little-endian frames for `Request`/`Cancel`/`RpcEvent`;
    self-delimiting via `payload_len` (channel slots are fixed `sample_max`).
  - `client` — `RpcClient`: `next_cid` (`u64`, never wraps), correlation table,
    `submit`, `pump` (per-cid state machine `I-RPC10`, stale/invalid counters),
    `call` (submit → pump → timeout with best-effort `Cancel`), epoch-change
    `RpcPeerRestarted` (`I-RPC9`).
  - `server` — `RpcServer<S: UnaryService>`: `serve_step` (poll cancels → admit
    requests reserve-then-consume → deadline injection → flush outboxes),
    `run_workers` (bounded worker pool), random `server_session`, stale-session
    and duplicate-cid drop counters, bounded per-client `Outbox` (guaranteed
    lifecycle/terminal, replaceable progress).
- **Replay E2E + fixtures** (carried from M15W4): `replay_roundtrip` and
  [replay-fixtures.md](../spec/replay-fixtures.md); re-run this week as gate
  M16W4 evidence.
- Gate evidence [m16w1.md](../gates/m16w1.md); phase tracker
  [phase-32.md](../gates/phase-32.md); [CHANGELOG.md](../../CHANGELOG.md) M16W1
  tranche.

Notes:

- No architecture divergence: pinned invariants only (rpc-actions
  I-RPC1/3/4/6/7/9/10, record-replay I-RC4); no ADR.
- M16W1 is the unary request/response slice. The action phase machine, the
  deterministic cancel-at-phase-boundary protocol, and the C ∈ 1..=16 dispatch
  fixture carry to M16W2/M16W3.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `RPC_CALL_GREEN = 1`; `RPC_CID_GREEN = 1`; `REPLAY_E2E = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M16W1) = PASS`**

### How to reproduce

```text
cargo test -p drm-rpc
cargo test -p drm-record replay_roundtrip
cargo xtask audit-traceability
cargo xtask check-layering
```
