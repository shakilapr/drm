# RPC & Actions Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: `drm-rpc` — request-response services and long-running, cancellable action workflows built as a clean library layer over correlated topics; single ordered event stream per client; reactor/worker execution with server sessions, admission control, and bounded outboxes; no changes to core ring buffers. Implements architecture.md §drm-rpc. Owner: E1 (crate `drm-rpc`). Phase 32. Depends on [cursor-delivery.md](../runtime/cursor-delivery.md), [channel-metadata.md](../runtime/channel-metadata.md), [handle-system.md](../toolchain/handle-system.md), [overflow-policies.md](../runtime/overflow-policies.md) (reject gating).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-rpc-correlated-pairs` | services over correlated topics; one ordered `event/<client>` stream; core untouched | `REQ-OPS-012` |
| `REQ-OPS-rpc-no-cross-correlation` | concurrent responses never cross-correlate | `REQ-OPS-013` |
| `REQ-OPS-rpc-cancel-deterministic` | cancel token observed by the reactor; no new phase after observation; `Terminal(Cancelled)` follows | `REQ-OPS-011` |
| `REQ-OPS-rpc-action-workflows` | long-running, cancellable actions with cooperative checkpoints | `REQ-OPS-010` |
| `REQ-OPS-rpc-server-session` | random `server_session` per start; stale-session requests dropped, never executed; no automatic re-execution | `REQ-OPS-020` |
| `REQ-OPS-rpc-admission` | operation + terminal capacity reserved before a request is consumed; `publish ≠ accepted` | `REQ-OPS-021` |
| `REQ-OPS-rpc-outbox` | bounded outbox: lifecycle/terminal guaranteed, progress advisory (replaceable) | `REQ-OPS-022` |
| `REQ-OPS-rpc-deadline-budget` | wire carries relative `remaining_budget_ns`; absolute client deadline never crosses a machine | `REQ-OPS-023` |
| `REQ-OPS-rpc-reliable-route` | codegen injects `reliability = reliable`; verify rejects best-effort-only routes (`V-RPC`) | `REQ-OPS-024` |
| `REQ-OPS-rpc-event-order` | per-cid event state machine; the event topic sequence is the total operation order | `REQ-OPS-025` |

## Data Structures

```rust
pub struct CorrelationId { client_id: u32, session_nonce: u64, counter: u64 }
pub struct Request { correlation_id: CorrelationId, target_server_session: u128, // 0 = unknown / first contact
 payload: BoundedBytes, remaining_budget_ns: Option<u64> } // relative budget, not an absolute clock reading
pub struct Cancel { correlation_id: CorrelationId }
pub struct RpcEvent { correlation_id: CorrelationId, kind: EventKind, payload: BoundedBytes }
pub enum EventKind { Accepted { server_session: u128 }, Started, Progress, CancelAccepted, CancelRejected,
 Terminal { outcome: Outcome, status: Status } } // Outcome: Succeeded | Aborted | Cancelled
pub struct Phase { expected_max_duration_ns: u64, // SLO metadata — middleware cannot preempt user code
 execute: fn(&mut OwnedActionState, &ActionContext) -> Status }
pub struct ActionContext<'a> { cancel_requested: &'a AtomicBool } // ctx.cancelled() / ctx.checkpoint()
// Fixed-capacity structures allocated at activation; sizes verified from declared concurrency (I-RPC6):
// operation_table (each reservation includes a guaranteed terminal slot), per-client outboxes,
// session-scoped cancel table.
```

Topic conventions (static, generated from service declarations in manifests):

- Each client owns `request/<client_id>` and `cancel/<client_id>` (client → server) and consumes `event/<client_id>` (server → client). Cancel keeps an independent topic so a request backlog can never delay cancellation. One publisher per canonical topic everywhere.
- The server consumes request/cancel through a deterministic round-robin multiplexer and alone publishes `event/<client_id>` — one ordered stream whose topic sequence is the total operation order (no separate `event_seq`).
- Request/cancel: `reject` with the server as required sequential consumer. Event: `reject` with its client as required sequential consumer.
- Codegen injects `requires: reliability = reliable` for all three families; `drm verify` (`V-RPC`) rejects RPC over best-effort-only routes.
- Capacities are verified from declared concurrency; saturation returns `RpcBusy` at admission. Accepted work is never silently lost.

## Invariants

- **I-RPC1 NoCrossCorrelation**: with C concurrent in-flight requests, every event is delivered to exactly the requesting client context; `{client_id, session_nonce, counter}` is unique until session close and echoed verbatim. Test: `rpc_no_cross_correlation` (concurrent fixture).
- **I-RPC2 DeterministicCancel**: the reactor polls cancel topics every loop iteration and applies cancels before dispatching new work; a cancel sets `cancel_requested` and commits `CancelAccepted` (guaranteed class) for non-terminal operations; no new phase starts after the token is observed; the terminal outcome is then `Cancelled`. Test: `rpc_cancel_deterministic` (cancel at every phase point, including mid-phase checkpoints).
- **I-RPC3 DeadlineSemantics**: the wire carries relative `remaining_budget_ns`; the server derives `server_deadline = now + budget` in its own clock domain. Client timeout ⇒ `RpcTimeout` plus a best-effort `Cancel` publish (default; documented opt-out); the server treats the budget as SLO metadata and may inject cancellation on expiry. Late terminals are dropped as stale by cid. Clock skew between machines changes nothing. Test: `rpc_timeout` (+ ±seconds clock-offset fixture).
- **I-RPC4 CoreUntouched**: rpc adds zero new channel semantics — only the declared topics. Test: code-audit + verify catalogue unchanged.
- **I-RPC5 SingleWriter**: every generated RPC topic has exactly one publisher; server fan-in is an explicit deterministic multiplexer. Test: `rpc_single_writer`.
- **I-RPC6 BoundedState**: operation table, per-client outboxes, and the cancel table have declared fixed capacities; full ⇒ `RpcBusy` at admission; the cancel table is session-scoped and garbage-collected — never unbounded growth. Test: `rpc_capacity`.
- **I-RPC7 AcceptedCompletes**: admission reserves the operation and its guaranteed terminal slot **before** a request is consumed; `publish ≠ accepted` — acceptance means admitted to the bounded operation table. Once accepted, lifecycle/terminal events occupy reserved outbox capacity until the client topic accepts them or the session fails. `Progress` is advisory: 0-or-1 pending, replaced by newer progress, discarded on terminal — it never displaces guaranteed capacity. Test: `rpc_response_pressure`, `rpc_outbox_classes`.
- **I-RPC8 CancelBoundHonest**: `Phase.expected_max_duration_ns` is SLO metadata — declared, not enforced; middleware cannot preempt user code. Guaranteed: `CancelAccepted` within one reactor loop; no new phase starts after token observation. Intra-phase responsiveness requires cooperative checkpoints. Test: `rpc_cancel_bound` (ack ≤ poll interval; no-phase-start proof), `rpc_handler_overrun` (handler exceeds declared duration: ack still ≤ poll interval; SLO violation reported).
- **I-RPC9 ServerSession**: a random `server_session: u128` is regenerated at every server start; requests carrying a non-zero stale session are dropped + counted, never executed; `Accepted` echoes the session. Clients observe the event-topic epoch change and fail open calls with `RpcPeerRestarted`; there is no automatic re-execution — retry is an explicit client decision. Test: `server_crash_restart` (crash before execution / during / after side effect before response / after response publish; restart with retained client request histories).
- **I-RPC10 EventOrder**: per-cid validity — `Started` only after `Accepted`, `Progress` only after `Started`, `CancelAccepted`/`CancelRejected` only before `Terminal`, nothing after `Terminal`. The client rejects invalid sequences + counts them (defensive; the single-writer server cannot produce them absent a bug). Test: `rpc_event_order` (injected invalid sequences).
- **I-RPC11 ReliableRoute**: request/cancel/event resolve only to reliable routes; `V-RPC` build error otherwise. Test: verify fixture (RPC service over best-effort-only link).

## Functions

```rust
// Client (per service). The async reactor is the primary model; sync call() is a thin wrapper
// over the same explicit pump.
fn call(&mut self, req: BoundedBytes, timeout: Option<Duration>) -> Result<BoundedBytes, RpcError> {
 let cid = self.next_cid()?; // counter exhaustion closes the session; never wraps
 let waiter = self.correlation.try_reserve(cid, timeout).map_err(|Full| RpcBusy)?;
 let request = Request { correlation_id: cid, target_server_session: self.observed_session,
  payload: req, remaining_budget_ns: timeout.map(dur_to_ns) };
 self.request_topic.publish(request).map_err(|e| { self.correlation.release(waiter); map_publish(e) })?;
 loop {
  self.pump_events()?;
  if let Some(r) = waiter.take_terminal() { return r; }
  if self.event_topic.epoch_changed()? { self.fail_open_calls(RpcPeerRestarted); return Err(RpcPeerRestarted); } // I-RPC9
  if waiter.expired() { self.publish_cancel_best_effort(cid); return Err(RpcTimeout); } // I-RPC3
  park_bounded(POLL_INTERVAL); // bounded, cancellable — never a bare sleep on an undefined pump
 }
}

fn pump_events(&mut self) -> Result<(), RpcError> {
 while let Some(ev) = self.event_topic.try_copy_next()? { // reject topic; client is required consumer
  self.observed_session = ev.accepted_session_or(self.observed_session); // session discovery via Accepted echo
  match self.correlation.dispatch(ev) { // per-cid state machine (I-RPC10); Terminal completes the waiter
  Err(Stale | InvalidOrder) => self.stale_event_count += 1,
  Ok(()) => {}
  }
 }
 Ok(())
}

// Server reactor (one per service; never executes user code)
fn serve(&mut self) -> Result<(), RpcError> {
 self.session = random_u128(); // I-RPC9: new identity per start
 loop {
  self.poll_cancels_round_robin()?; // every iteration — never behind request handling or handlers
  self.apply_cancels(); // non-terminal op: set token + outbox CancelAccepted (guaranteed class);
   // terminal/unknown cid: drop + counter (nothing after Terminal, I-RPC10); non-cancellable phase: CancelRejected
  self.poll_requests()?; // reserve-then-consume admission (I-RPC7)
  self.poll_client_health()?; // dead client epoch ⇒ release operation + counters
  self.flush_outboxes_round_robin()?; // QueueFull retains entries; retried next loop — core never blocks
 }
}

// Admission — capacity reserved BEFORE a request is consumed (I-RPC7); publish ≠ accepted
fn poll_requests(&mut self) -> Result<(), RpcError> {
 for client in self.clients_fair() { // deterministic round-robin, one input per client
  let Some(op) = self.operation_table.try_reserve() else { return Ok(()); }; // no free op ⇒ consume nothing
  let Some(req) = self.request_mux.copy_next(client)? else { self.operation_table.release(op); continue; };
  if req.target_server_session != 0 && req.target_server_session != self.session {
   self.operation_table.release(op); self.stale_request_count += 1; continue; } // I-RPC9: never executed
  op.accept(req); // reservation includes this operation's guaranteed terminal slot
  self.outbox(client).commit(Accepted { server_session: self.session, .. });
  self.workers.schedule(op); // user code runs only on the bounded worker pool
 }
 Ok(())
}

// Worker pool (bounded) — the reactor never blocks on handlers
fn run_unary(op: &mut Operation, ctx: &ActionContext) {
 let (status, payload) = split_result(handler(op.request.payload)); // owned request bytes
 op.outbox.commit(Terminal { outcome: from(status), status, payload }); // outbox only — never direct publish (I-RPC7)
}

fn run_action(op: &mut Operation, phases: &[Phase], ctx: &ActionContext) {
 op.outbox.commit(Started);
 for ph in phases {
  if ctx.cancelled() { op.outbox.commit(Terminal { outcome: Cancelled, .. }); return; } // I-RPC2
  ph.execute(&mut op.state, ctx); // cooperative checkpoints; expected_max_duration_ns = SLO metadata (I-RPC8)
  op.outbox.commit(Progress { .. }); // advisory — outbox may coalesce (I-RPC7)
 }
 op.outbox.commit(Terminal { outcome: from_status(op.state), .. });
}

// Outbox — fixed capacity per operation (I-RPC6/I-RPC7)
fn commit(&mut self, ev: Event) {
 if ev.is_progress() { self.pending_progress = Some(ev); return; } // replace-on-new, coalesced pre-publication
 if ev.is_terminal() { self.pending_progress = None; } // unsent progress is obsolete at terminal
 self.guaranteed.push(ev); // capacity reserved at admission — cannot fail
}
fn flush(&mut self, topic: &EventTopic) {
 if let Some(p) = self.pending_progress { if topic.try_publish(p).is_ok() { self.pending_progress = None; } }
 while let Some(ev) = self.guaranteed.peek() { if topic.try_publish(ev).is_err() { break; } self.guaranteed.pop(); }
}
```

## Protocols

*Request/response cycle:*

1. Client `call`: reserve correlation waiter → publish `Request` (session, relative budget) → pump the event stream until `Terminal`.
2. Reactor (every loop): poll cancels → apply tokens/acks → poll requests (reserve-then-consume, session check) → poll client health → flush outboxes.
3. Bounded workers execute handlers/actions from owned request bytes and commit events to the outbox; workers never publish to topics directly.
4. Client dispatches by full cid; the per-cid state machine validates order (I-RPC10); `Terminal` completes the waiter; late terminals drop stale (I-RPC3).

*Action workflow (cancellable):*

1. Submit → `Accepted` → `Started` → `Progress`* → `Terminal(Succeeded | Aborted | Cancelled)`.
2. Cancel path: client publishes on its cancel topic (independent of request backlog) → reactor sets `cancel_requested` + commits `CancelAccepted` → worker stops at the next checkpoint or phase boundary → `Terminal(Cancelled)`. No new phase starts after token observation (I-RPC2). Mid-phase responsiveness requires cooperative checkpoints; `expected_max_duration_ns` is SLO metadata only (I-RPC8).
3. Non-cancellable phase: reactor commits `CancelRejected`; the operation completes to its natural terminal.

*Server session (I-RPC9):*

1. `server_session: u128` regenerated at every server start; `Accepted` echoes it.
2. Client stamps subsequent requests with the observed session; `0` = unknown (first contact — admitted).
3. Non-zero stale session ⇒ request dropped + counter — never executed. No automatic re-execution; retry is an explicit client decision.
4. Event-topic epoch change ⇒ client fails open calls with `RpcPeerRestarted` and resets its observed session.

*Cancel-table lifecycle (I-RPC6):*

1. Entry created when a cancel arrives before its request (or while the request is queued); consumed when that request is admitted.
2. Evicted when its cid counter window passes or on session/epoch change; bounded capacity; oldest-entry eviction + counter — never unbounded growth.

## Design Confirmation (M14W4)

RPC design seeded against architecture.md §drm-rpc and this doc's I-RPC1..I-RPC11; the
seed is recorded in [rpc-design-seed.md](../../spec/rpc-design-seed.md). No invariant changed.

- [x] Correlated topic pairs: each client owns `request/<client_id>` + `cancel/<client_id>` and consumes `event/<client_id>`; one publisher per canonical topic; server fan-in is an explicit deterministic round-robin multiplexer (I-RPC1, I-RPC5, `REQ-OPS-012`/`REQ-OPS-013`).
- [x] `CorrelationId {client_id, session_nonce, counter}`: `counter` is a `u64` per client session, never reused and never wrapped; exhaustion closes the session with `SequenceExhausted` (I-RPC1, I-RPC10, `REQ-OPS-025`).
- [x] Deterministic cancel at phase boundaries: the reactor polls cancel topics every loop iteration, sets `cancel_requested` before dispatching new work, and commits `CancelAccepted` for non-terminal operations; no new phase starts after token observation, then `Terminal(Cancelled)` (I-RPC2, I-RPC8, `REQ-OPS-010`/`REQ-OPS-011`).
- [x] Deadline semantics: the wire carries relative `remaining_budget_ns`; the server derives `server_deadline = now + budget` in its own clock domain; late terminals are dropped stale by cid; a client timeout emits `RpcTimeout` plus a best-effort `Cancel` (I-RPC3, `REQ-OPS-023`).
- [x] Core-untouched (architecture I-R4 / I-RPC4): rpc adds zero new channel semantics — only the declared generated topics; codegen injects `reliability = reliable` (I-RPC11, `REQ-OPS-024`).

Seeds [§Functions](#functions): `call`, `pump_events`, `serve`, `poll_requests`, `run_action`,
and the outbox `commit`/`flush`. Deltas: none — this section records the review; the
implementation lands in Phase 32 (M15W1).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two concurrent calls, out-of-order events | cid dispatch — no cross-correlation (I-RPC1) |
| Terminal after client timeout | dropped stale by cid + counter (I-RPC3) |
| Server crash mid-action | event-topic epoch change ⇒ open calls fail `RpcPeerRestarted` (I-RPC9) |
| Restart with retained client requests | stale-session requests dropped + counter; session-0 requests admitted (I-RPC9) |
| Request storm beyond server rate | request topic fills (`reject`, server required) ⇒ publish `QueueFull` ⇒ `RpcBusy`; nothing consumed without a reserved operation slot |
| Client stalls; event topic full | outbox retains lifecycle/terminal (I-RPC7); pending progress replaced; retention ends at session failure |
| Cancel before request | cancel-table entry (session-scoped, bounded); applied if the request is admitted; GC per lifecycle |
| Cancel topic full | client publish fails — no false claim cancellation was accepted |
| Cancel for terminal/unknown cid | dropped + counter; no event after `Terminal` (I-RPC10) |
| Cancel during a non-cancellable phase | `CancelRejected` committed; operation runs to its natural terminal |
| Duplicate cid (bug) | server drops second + counter; never mis-delivers (I-RPC1) |
| Correlation counter exhausted | session closes with `SequenceExhausted`; counter never wraps |
| Handler exceeds `expected_max_duration_ns` | SLO violation reported; no preemption; cancel ack still ≤ one reactor loop (I-RPC8) |
| Cross-machine clock skew | wire carries relative budget only (I-RPC3) |
| RPC topic routed best-effort | build error `V-RPC` (I-RPC11) |

## Test Matrix

| Case | Expected |
|---|---|
| `rpc_no_cross_correlation` (C ∈ 1..=16 concurrent) | I-RPC1 |
| `rpc_cancel_deterministic` (cancel at every phase point incl. checkpoints) | I-RPC2 |
| `rpc_timeout` (expiry + late terminal + ±s clock offset) | I-RPC3 |
| `rpc_capacity` (operation/outbox/cancel-table full) | I-RPC6 `RpcBusy` at admission; nothing consumed |
| `rpc_single_writer` | generated graph passes single-writer verification (I-RPC5) |
| `rpc_response_pressure` (stalled client fills event history) | I-RPC7 terminal retained; progress coalesced |
| `rpc_outbox_classes` | I-RPC7 progress replaceable; lifecycle/terminal never dropped |
| `rpc_cancel_bound` + `rpc_handler_overrun` | I-RPC8 ack ≤ one reactor loop; SLO violation reported |
| `server_crash_restart` (4 crash points; retained request histories) | I-RPC9 explicit result — never silent, never re-executed |
| `rpc_event_order` (injected invalid sequences) | I-RPC10 rejected + counted |
| verify: RPC over best-effort-only route | I-RPC11 build error (`V-RPC`) |
