# RPC & Actions Algorithms

Scope: `drm-rpc` — request-response services and long-running, cancellable action workflows built as a clean library layer over correlated topic pairs; no changes to core ring buffers. Implements architecture.md §drm-rpc. Owner: E1 (crate `drm-rpc`). Phase 32. Depends on [cursor-delivery.md](../runtime/cursor-delivery.md), [channel-metadata.md](../runtime/channel-metadata.md), [handle-system.md](../toolchain/handle-system.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-rpc-correlated-pairs` | services over correlated topic pairs; core untouched | `REQ-OPS-012` |
| `REQ-OPS-rpc-no-cross-correlation` | concurrent responses never cross-correlate | `REQ-OPS-013` |
| `REQ-OPS-rpc-cancel-deterministic` | cancellation is deterministic | `REQ-OPS-011` |
| `REQ-OPS-rpc-action-workflows` | long-running, cancellable actions | `REQ-OPS-010` |

## Data Structures

```rust
pub struct CorrelationId { client_id: u32, session_nonce: u64, counter: u64 }
pub struct Request { correlation_id: CorrelationId, payload: BoundedBytes, deadline_ns: Option<u64> }
pub struct Cancel { correlation_id: CorrelationId }
pub struct Response { correlation_id: CorrelationId, status: Status, payload: BoundedBytes }
pub struct ActionState { correlation_id: CorrelationId, phase: ActionPhase, progress_pct: u8 }
pub struct Phase { max_duration_ns: u64, execute: fn(&mut OwnedActionState) -> Status }
// Maps and payloads have generated fixed capacities allocated at activation.
```

Topic conventions (static, generated from service declarations in manifests):
- Each client owns `request/<client_id>` and `cancel/<client_id>` topics. This preserves one publisher per canonical topic.
- The server consumes those topics with `next()` through a deterministic round-robin multiplexer.
- The server alone publishes generated `response/<client_id>` and `state/<client_id>` topics. Each client reads only its pair and dispatches concurrent calls by full `CorrelationId`.
- Request and cancel topics use `reject` with the server as a required sequential consumer. Response and state topics use `reject` with their client as the required sequential consumer. Capacity and pending-output slots are verified from declared concurrency; saturation returns `RpcBusy`, never silently loses an accepted request, cancel, response, or terminal state.

## Invariants

- **I-R1 NoCrossCorrelation**: with C concurrent in-flight requests, every response is delivered to exactly the requesting client context; `{client_id, session_nonce, counter}` is unique until session close and echoed verbatim. Test: `rpc_no_cross_correlation` (concurrent fixture).
- **I-R2 DeterministicCancel**: cancel marks are observed in bounded order — server acknowledges cancel at the next phase boundary; after ack, no further phase executes. Test: `rpc_cancel_deterministic`.
- **I-R3 DeadlineSemantics**: `deadline` expiry ⇒ client raises `RpcTimeout` without removing the server-side request (server may still complete; late responses are dropped as stale by cid+deadline check). Test: `rpc_timeout`.
- **I-R4 CoreUntouched**: rpc adds zero new channel semantics — only the declared topic pairs. Test: code-audit + verify catalogue unchanged.
- **I-R5 SingleWriter**: every generated RPC topic has exactly one publisher; server fan-in is an explicit deterministic multiplexer. Test: `rpc_single_writer`.
- **I-R6 BoundedState**: correlation, cancellation, and workflow tables have declared fixed capacities; full returns `RpcBusy`. Test: `rpc_capacity`.
- **I-R7 AcceptedCompletes**: once a request is accepted, its response/terminal state occupies a fixed pending slot until the client topic accepts it or the session fails. Test: `rpc_response_pressure`.
- **I-R8 CancelLatencyBounded**: every action phase declares a finite maximum duration; cancellation acknowledgement latency is bounded by the current phase plus one server poll interval. Test: `rpc_cancel_bound`.

## Functions

```rust
// Client (per service)
fn call(&mut self, req: BoundedBytes, deadline_ns: Option<u64>) -> Result<BoundedBytes, RpcError> {
 let cid = self.next_cid()?; // counter exhaustion closes the session; never wraps
 let waiter = self.correlation.try_reserve(cid, deadline_ns).map_err(|Full| RpcBusy)?;
 self.request_topic.publish(Request { correlation_id: cid, payload: req, deadline_ns })
 .map_err(|e| { self.correlation.release(waiter); map_publish(e) })?;
 self.wait_bounded(waiter) // response pump fills this fixed slot; expiry returns RpcTimeout
}

fn poll_responses(&mut self) -> Result<(), RpcError> {
 while let Some(r) = self.response_topic.try_copy_next()? {
 match self.correlation.complete(r.correlation_id, r.status, r.payload) {
 Ok(()) => {}
 Err(UnknownOrExpired) => self.late_response_count += 1,
 }
 }
 Ok(())
}

// Server (per service)
fn serve(&self, handler: impl Fn(BoundedBytes) -> Result<BoundedBytes, Status>) {
 loop {
 self.poll_cancel_inputs_round_robin()?;
 let req = self.request_mux.copy_next_round_robin()?; // explicit fan-in, one input per client
 if self.cancelled.contains(&req.correlation_id) {
 self.state_topic.publish(ActionState { phase: Cancelled, .. }); continue; // I-R2 ack
 }
 let pending = self.pending_outputs.try_reserve(req.correlation_id).map_err(|Full| RpcBusy)?;
 let (status, payload) = split_result(handler(req.payload)); // request payload is owned
 pending.store(Response { correlation_id: req.correlation_id, status, payload });
 self.flush_pending_round_robin()?; // QueueFull retains the slot; retry occurs outside core
 }
}

// Action workflow: phase machine — handler checks cancellation between phases (deterministic boundary)
fn run_action(&self, cid: CorrelationId, phases: &[Phase; MAX_PHASES], phase_count: usize) {
 for ph in &phases[..phase_count] {
 if self.cancelled.contains(&cid) { self.ack_cancel(cid); return; } // I-R2
 ph.execute(&mut self.owned_action_state(cid)); // bounded by declared max_duration_ns (I-R8)
 self.state_topic.publish(ActionState { correlation_id: cid, phase: ph.done_marker(), .. });
 }
}
```

## Protocols

*Request/response cycle:*

1. Client `call` → publish Request on its generated input topic (full cid unique per client session).
2. Server multiplexes client inputs round-robin, reserves a pending-output slot, executes from owned request bytes, and retains the response until its client topic accepts it.
3. Client consumes responses, dispatches by cid; deadline bookkeeping per I-R3.

*Action workflow (cancellable):*

1. Client submits action request (cid, phases hint).
2. Server publishes `ActionState` per phase (progress).
3. Cancel: client publishes on its lossless bounded cancel topic; server acknowledges at the next declared-bounded phase boundary (I-R2/I-R8); state → `Cancelled`.
4. Client observes terminal state; correlation removed.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two concurrent calls, out-of-order responses | cid dispatch — no cross-correlation (I-R1) |
| Response after client timeout | dropped as stale (cid unknown post-timeout) + counter |
| Server crash mid-action | epoch change on topics (publisher-lifecycle); client sees state stop; restart ⇒ cid space resets (per-session) — pending actions fail with `RpcPeerOffline` |
| Request storm beyond server rate | request publish returns `RpcBusy`; accepted requests are never overwritten |
| Response/state topic full | pending output retained; server applies RPC-layer backpressure, core thread never blocks |
| Cancel topic full | client receives `RpcBusy`; no false claim that cancellation was accepted |
| Duplicate cid (bug) | server drops second + counter; never mis-delivers (I-R1) |
| Correlation counter exhausted | session closes with `SequenceExhausted`; counter never wraps |

## Test Matrix

| Case | Expected |
|---|---|
| `rpc_no_cross_correlation` (C ∈ 1..=16 concurrent) | I-R1 |
| `rpc_cancel_deterministic` (cancel per phase) | I-R2 |
| `rpc_timeout` (deadline expiry + late response) | I-R3 |
| `rpc_capacity` | full input/table returns `RpcBusy`; no accepted request lost |
| `rpc_single_writer` | generated graph passes single-writer verification |
| `rpc_response_pressure` | accepted result retained until delivery/session failure |
| `rpc_cancel_bound` | acknowledgement within declared phase bound |
| server-crash-restart fixture | clean failure semantics |
