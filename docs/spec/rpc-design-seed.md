# RPC Design Seed (M14W4)

Non-normative derived spec. Source of truth: [rpc-actions.md](../algorithms/ops/rpc-actions.md)
(§Invariants I-RPC1..I-RPC11, §Functions, §Protocols), [architecture.md](../architecture.md)
§drm-rpc, [ui-architecture.md](../ui-architecture.md) (engine-only mutations). On any conflict
the architecture docs win; on any pseudocode conflict the algorithm docs win (codebase.md §2.4).
Owner: E1. Phase 32 prep. Written M14W4.

Seeds the `drm-rpc` implementation (M15W1) by locking the correlated-topic layout, the cid
bookkeeping rule, deterministic cancellation at phase boundaries, deadline semantics, and the
core-untouched rule. The doc adds no invariant; it fixes the concrete choices the algorithm doc
leaves to the wave.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-rpc-correlated-pairs` | services over correlated topics; one ordered `event/<client>` stream; core untouched | `REQ-OPS-012` |
| `REQ-OPS-rpc-no-cross-correlation` | concurrent responses never cross-correlate | `REQ-OPS-013` |
| `REQ-OPS-rpc-cancel-deterministic` | cancel token observed by the reactor; no new phase after observation; `Terminal(Cancelled)` follows | `REQ-OPS-011` |
| `REQ-OPS-rpc-action-workflows` | long-running, cancellable actions with cooperative checkpoints | `REQ-OPS-010` |
| `REQ-OPS-rpc-server-session` | random `server_session` per start; stale-session requests dropped, never executed | `REQ-OPS-020` |
| `REQ-OPS-rpc-admission` | operation + terminal capacity reserved before a request is consumed | `REQ-OPS-021` |
| `REQ-OPS-rpc-outbox` | bounded outbox: lifecycle/terminal guaranteed, progress advisory | `REQ-OPS-022` |
| `REQ-OPS-rpc-deadline-budget` | wire carries relative `remaining_budget_ns` | `REQ-OPS-023` |
| `REQ-OPS-rpc-reliable-route` | codegen injects `reliability = reliable`; verify rejects best-effort-only routes (`V-RPC`) | `REQ-OPS-024` |
| `REQ-OPS-rpc-event-order` | per-cid event state machine; event topic sequence is the total operation order | `REQ-OPS-025` |

## 1. Correlated topic pairs (locked)

Each client owns `request/<client_id>` and `cancel/<client_id>` (client → server) and consumes
`event/<client_id>` (server → client). Topics are generated from service declarations in
manifests; the canonical topic set is static.

| Topic | Publisher | Consumer | Overflow |
|---|---|---|---|
| `request/<client_id>` | the client | server (required sequential) | `reject` |
| `cancel/<client_id>` | the client | server (required sequential) | `reject` |
| `event/<client_id>` | the server | the client (required sequential) | `reject` |

- Cancel keeps an independent topic so a request backlog can never delay cancellation.
- Exactly one publisher per canonical topic (I-RPC5); the server fan-in over `request/*` is an
  explicit deterministic round-robin multiplexer, never a multi-writer topic.
- The server alone publishes `event/<client_id>` — one ordered stream whose topic sequence is the
  total operation order; no separate `event_seq` (I-RPC10, `REQ-OPS-025`).
- Codegen injects `requires: reliability = reliable` for all three families; `drm verify`
  (`V-RPC`) rejects RPC over best-effort-only routes (I-RPC11, `REQ-OPS-024`).
- Capacities are verified from declared concurrency; saturation returns `RpcBusy` at admission
  (I-RPC6/I-RPC7, `REQ-OPS-021`).

## 2. cid bookkeeping (locked)

```rust
pub struct CorrelationId { client_id: u32, session_nonce: u64, counter: u64 }
```

- `counter` is a `u64` per client session, assigned by `next_cid()`, **never reused** and
  **never wrapped**; exhaustion closes the session with `SequenceExhausted` (I-RPC1).
- `{client_id, session_nonce, counter}` is unique until session close and is echoed verbatim on
  every event; the client dispatches by the full cid, so concurrent responses never
  cross-correlate (I-RPC1, `REQ-OPS-012`/`REQ-OPS-013`).
- A duplicate cid (a bug) is dropped server-side + counted; never mis-delivered (I-RPC1).
- A late terminal after the client timed out is dropped stale by cid (I-RPC3, `REQ-OPS-023`).

## 3. Deterministic cancel at phase boundaries (locked)

- The reactor polls cancel topics every loop iteration and applies cancels **before** dispatching
  new work (I-RPC2).
- A cancel for a non-terminal operation sets `cancel_requested` and commits `CancelAccepted`
  (guaranteed class); no new phase starts after the token is observed; the terminal outcome is
  then `Cancelled` (I-RPC2, `REQ-OPS-010`/`REQ-OPS-011`).
- A cancel during a non-cancellable phase commits `CancelRejected`; the operation runs to its
  natural terminal.
- `Phase.expected_max_duration_ns` is SLO metadata — declared, not enforced; middleware cannot
  preempt user code. Guaranteed: `CancelAccepted` within one reactor loop; intra-phase
  responsiveness requires cooperative checkpoints (I-RPC8).
- A cancel-table entry created before its request is session-scoped, bounded, and
  garbage-collected; oldest-entry eviction + counter — never unbounded growth (I-RPC6).

## 4. Deadline semantics (locked)

- The wire carries relative `remaining_budget_ns`; an absolute client deadline never crosses a
  machine (I-RPC3, `REQ-OPS-023`).
- The server derives `server_deadline = now + budget` in its own clock domain; the budget is SLO
  metadata and may drive injected cancellation on expiry.
- A client timeout emits `RpcTimeout` plus a best-effort `Cancel` publish (default; documented
  opt-out).
- Late terminals are dropped as stale by cid; clock skew between machines changes nothing.

## 5. Core-untouched rule (locked)

RPC adds zero new channel semantics — only the declared generated topics (architecture I-R4 /
I-RPC4). The library never creates a multi-writer topic and never changes core ring behavior;
the code-audit + verify catalogue stay unchanged.

## 6. Function seeds

Seeds [rpc-actions.md](../algorithms/ops/rpc-actions.md) §Functions: `call` (reserve waiter →
publish `Request` → pump events until `Terminal`), `pump_events` (per-cid state machine,
stale/order counting), `serve` (session identity → poll cancels → apply cancels → poll requests →
poll client health → flush outboxes), `poll_requests` (reserve-then-consume admission + session
check), `run_action` (cooperative checkpoints, phase-boundary cancel), and the outbox
`commit`/`flush` (guaranteed queue + replaceable progress).

## 7. Acceptance (M14W4)

- [x] Correlated topic pairs, one publisher per canonical topic, deterministic multiplexer.
- [x] cid bookkeeping: `u64` counter per client session, never reused, never wrapped.
- [x] Deterministic cancel at phase boundaries with `CancelAccepted`/`CancelRejected`.
- [x] Relative-budget deadline semantics; late responses dropped stale.
- [x] Core-untouched rule (I-R4 / I-RPC4) confirmed.

Deltas: none to rpc-actions.md; this seed fixes the concrete choices. Implementation lands in
Phase 32 (M15W1).

REQ: REQ-OPS-010, REQ-OPS-011, REQ-OPS-012, REQ-OPS-013, REQ-OPS-020, REQ-OPS-021,
REQ-OPS-022, REQ-OPS-023, REQ-OPS-024, REQ-OPS-025.
