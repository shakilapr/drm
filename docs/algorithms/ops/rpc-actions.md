# RPC & Actions Algorithms

Scope: `drm-rpc` — request-response services and long-running, cancellable action workflows built as a clean library layer over correlated topic pairs; no changes to core ring buffers. Implements architecture.md §drm-rpc. Owner: E1 (crate `drm-rpc`). Phase 32. Depends on [cursor-delivery.md](../runtime/cursor-delivery.md), [channel-metadata.md](../runtime/channel-metadata.md), [handle-system.md](../toolchain/handle-system.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-rpc-correlated-pairs` | services over correlated topic pairs; core untouched | — |
| `REQ-OPS-rpc-no-cross-correlation` | concurrent responses never cross-correlate | — |
| `REQ-OPS-rpc-cancel-deterministic` | cancellation is deterministic | — |
| `REQ-OPS-rpc-action-workflows` | long-running, cancellable actions | — |

## Data Structures

```rust
// Service `calc/solve` → topics `calc/solve/request` (@history: N), `calc/solve/response` (@history: N)
pub struct Request { correlation_id: u64, payload: Bytes, deadline: Option<Duration> }
pub struct Response { correlation_id: u64, status: Status, payload: Bytes }
pub struct ActionState { correlation_id: u64, phase: ActionPhase, progress_pct: u8, cancel_requested: bool }
// Client keeps local correlation map: cid → (sent_seq, deadline); server routes by cid.
```

Topic conventions (static, generated from service declarations in manifests):
- `request` topic: client publishes; server consumes with `next()` (sequential — no lost requests).
- `response` topic: server publishes; clients consume `next()` and filter by `correlation_id`.
- `action/state` topic (actions only): server publishes progress; cancel = client publishes `ActionState{cancel_requested: true}` on the request topic.

## Invariants

- **I-R1 NoCrossCorrelation**: with C concurrent in-flight requests, every response is delivered to exactly the requesting client context; no cid reuse confusion (ids unique per client session; server echoes cid verbatim). Test: `rpc_no_cross_correlation` (concurrent fixture).
- **I-R2 DeterministicCancel**: cancel marks are observed in bounded order — server acknowledges cancel at the next phase boundary; after ack, no further phase executes. Test: `rpc_cancel_deterministic`.
- **I-R3 DeadlineSemantics**: `deadline` expiry ⇒ client raises `RpcTimeout` without removing the server-side request (server may still complete; late responses are dropped as stale by cid+deadline check). Test: `rpc_timeout`.
- **I-R4 CoreUntouched**: rpc adds zero new channel semantics — only the declared topic pairs. Test: code-audit + verify catalogue unchanged.

## Functions

```rust
// Client (per service)
fn call(&mut self, req: Bytes, deadline: Option<Duration>) -> Result<Bytes, RpcError> {
 let cid = self.next_cid(); // u64, never reused per session
 self.request_topic.publish(Request { cid, payload: req, deadline })?;
 self.correlation.insert(cid, deadline);
 loop {
 match self.response_topic.copy_next()? { // owned reads — client simplicity (borrow optional)
 r if r.correlation_id == cid => return Ok(r),
 r => match self.correlation.get(&r.correlation_id) {
 Some(_) => self.correlation.remove(&r.correlation_id), // stale/other response → dispatch
 None => drop, // unknown cid → observability counter
 },
 }
 if deadline_elapsed(cid)? { return Err(RpcTimeout); } // I-R3
 }
}

// Server (per service)
fn serve(&self, handler: impl Fn(Bytes) -> Result<Bytes, Status>) {
 loop {
 let req = self.request_topic.copy_next()?; // sequential: none lost
 if let Some(cancel) = req_is_cancel(&req) { self.cancelled.insert(req.correlation_id); continue; }
 if self.cancelled.contains(&req.correlation_id) {
 self.state_topic.publish(ActionState { phase: Cancelled, .. }); continue; // I-R2 ack
 }
 let out = handler(req.payload);
 self.response_topic.publish(Response { correlation_id: req.correlation_id, .. });
 }
}

// Action workflow: phase machine — handler checks cancellation between phases (deterministic boundary)
fn run_action(&self, cid: u64, phases: Vec<Phase>) {
 for ph in phases {
 if self.cancelled.contains(&cid) { self.ack_cancel(cid); return; } // I-R2
 ph.execute();
 self.state_topic.publish(ActionState { correlation_id: cid, phase: ph.done_marker(), .. });
 }
}
```

## Protocols

*Request/response cycle:*

1. Client `call` → publish Request (cid unique per client session).
2. Server consumes sequentially (`next()`), executes handler, publishes Response (cid echo).
3. Client consumes responses, dispatches by cid; deadline bookkeeping per I-R3.

*Action workflow (cancellable):*

1. Client submits action request (cid, phases hint).
2. Server publishes `ActionState` per phase (progress).
3. Cancel: client publishes cancel marker on request topic; server acknowledges at next phase boundary (I-R2); state → `Cancelled`.
4. Client observes terminal state; correlation removed.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two concurrent calls, out-of-order responses | cid dispatch — no cross-correlation (I-R1) |
| Response after client timeout | dropped as stale (cid unknown post-timeout) + counter |
| Server crash mid-action | epoch change on topics (publisher-lifecycle); client sees state stop; restart ⇒ cid space resets (per-session) — pending actions fail with `RpcPeerOffline` |
| Request storm beyond server rate | server's sequential consumer laps ⇒ `GAP { missed }` → skipped requests surfaced to client as `RpcBusy` (gap marker forwarded) |
| Duplicate cid (bug) | server drops second + counter; never mis-delivers (I-R1) |

## Test Matrix

| Case | Expected |
|---|---|
| `rpc_no_cross_correlation` (C ∈ 1..=16 concurrent) | I-R1 |
| `rpc_cancel_deterministic` (cancel per phase) | I-R2 |
| `rpc_timeout` (deadline expiry + late response) | I-R3 |
| `rpc_gap_forwarding` | skipped requests surfaced as `RpcBusy` |
| server-crash-restart fixture | clean failure semantics |
