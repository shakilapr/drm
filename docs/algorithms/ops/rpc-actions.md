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
// Maps and payloads have generated fixed capacities allocated at activation.
```

Topic conventions (static, generated from service declarations in manifests):
- Each client owns `request/<client_id>` and `cancel/<client_id>` topics. This preserves one publisher per canonical topic.
- The server consumes those topics with `next()` through a deterministic round-robin multiplexer.
- The server alone publishes generated `response/<client_id>` and `state/<client_id>` topics. Each client reads only its pair and dispatches concurrent calls by full `CorrelationId`.
- Request topics use `reject` with the server as a required sequential consumer. Capacity is verified from declared concurrency; saturation returns `RpcBusy`, never loses a request.

## Invariants

- **I-R1 NoCrossCorrelation**: with C concurrent in-flight requests, every response is delivered to exactly the requesting client context; `{client_id, session_nonce, counter}` is unique until session close and echoed verbatim. Test: `rpc_no_cross_correlation` (concurrent fixture).
- **I-R2 DeterministicCancel**: cancel marks are observed in bounded order — server acknowledges cancel at the next phase boundary; after ack, no further phase executes. Test: `rpc_cancel_deterministic`.
- **I-R3 DeadlineSemantics**: `deadline` expiry ⇒ client raises `RpcTimeout` without removing the server-side request (server may still complete; late responses are dropped as stale by cid+deadline check). Test: `rpc_timeout`.
- **I-R4 CoreUntouched**: rpc adds zero new channel semantics — only the declared topic pairs. Test: code-audit + verify catalogue unchanged.
- **I-R5 SingleWriter**: every generated RPC topic has exactly one publisher; server fan-in is an explicit deterministic multiplexer. Test: `rpc_single_writer`.
- **I-R6 BoundedState**: correlation, cancellation, and workflow tables have declared fixed capacities; full returns `RpcBusy`. Test: `rpc_capacity`.

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
 let (status, payload) = split_result(handler(req.payload));
 self.response_topic(req.correlation_id.client_id)?.publish(Response {
 correlation_id: req.correlation_id, status, payload
 })?;
 }
}

// Action workflow: phase machine — handler checks cancellation between phases (deterministic boundary)
fn run_action(&self, cid: CorrelationId, phases: &[Phase; MAX_PHASES], phase_count: usize) {
 for ph in &phases[..phase_count] {
 if self.cancelled.contains(&cid) { self.ack_cancel(cid); return; } // I-R2
 ph.execute();
 self.state_topic.publish(ActionState { correlation_id: cid, phase: ph.done_marker(), .. });
 }
}
```

## Protocols

*Request/response cycle:*

1. Client `call` → publish Request on its generated input topic (full cid unique per client session).
2. Server multiplexes client inputs round-robin, executes handler, publishes Response (cid echo).
3. Client consumes responses, dispatches by cid; deadline bookkeeping per I-R3.

*Action workflow (cancellable):*

1. Client submits action request (cid, phases hint).
2. Server publishes `ActionState` per phase (progress).
3. Cancel: client publishes on its cancel topic; server acknowledges at next phase boundary (I-R2); state → `Cancelled`.
4. Client observes terminal state; correlation removed.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two concurrent calls, out-of-order responses | cid dispatch — no cross-correlation (I-R1) |
| Response after client timeout | dropped as stale (cid unknown post-timeout) + counter |
| Server crash mid-action | epoch change on topics (publisher-lifecycle); client sees state stop; restart ⇒ cid space resets (per-session) — pending actions fail with `RpcPeerOffline` |
| Request storm beyond server rate | request publish returns `RpcBusy`; accepted requests are never overwritten |
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
| server-crash-restart fixture | clean failure semantics |
