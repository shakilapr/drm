# Overflow Policies Algorithms

Scope: topic-level history overflow — `overwrite` continuous rolling with exact gap accounting, `reject` required-sequential gating with explicit `QueueFull`. Implements architecture.md §History Overflow. Owner: E1 (crate `drm-core::channel::overflow`). Phase 12.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-overflow-overwrite` | ring rolls; oldest retired; lapped ⇒ exact GAP | `REQ-CORE-039` |
| `REQ-CORE-overflow-reject` | required sequential consumer gates; publish fails `QueueFull`, never blocks | `REQ-CORE-040` |
| `REQ-CORE-required-consumers` | required sequential consumers registry per topic | `REQ-CORE-060` |
| `REQ-CORE-required-ack-seq` | shared `ack_seq` watermark, init 0 — sequence 0 representable | `REQ-CORE-059` |
| `REQ-CORE-ack-wiring` | `copy_next` acks after copy; `borrow_next` acks after release; separate from cursor/hazard | `REQ-CORE-001` |
| `REQ-CORE-explicit-errors` | explicit error over secretly stalling | `REQ-CORE-024` |

## Data Structures

```rust
pub enum OverflowPolicy { Overwrite, Reject } // from ChannelDescriptor (channel-metadata.md)

// `ack_seq` lives in the fixed ConsumerRecord (consumer-registration.md).
// It is the first sequence not acknowledged. Registration initializes it to first_valid_seq.
// Shared acknowledgement watermark per required consumer. This is DISTINCT from the reader's
// process-local cursor (which sequence to read next) and from the hazard slot (which physical
// slot is protected): `ack_seq` is the only shared state the publisher needs to gate retirement.
// Registry: per-channel array declared at descriptor creation (size = count of required
// sequential consumers from the workspace model; static, like pools).
// No sentinel: init 0 means "has acknowledged nothing", and sequence 0 is representable because
// retirement is allowed only when ack_seq > retire_seq.
```

## Invariants

- **I-OV1 OverwriteGaps**: under `overwrite`, lapped sequential reader receives exact `missed` count (cursor-delivery I-C2). Test: `overflow_overwrite_gaps`.
- **I-OV2 RejectNoBlock**: a full `reject` ring checks at most the fixed required-consumer count and returns `QueueFull` without waits, syscalls, or retry loops. Test: `overflow_reject_noblock`.
- **I-OV3 GatingExact**: `publish()` succeeds exactly when every required sequential consumer has `ack_seq > retire_seq` (the sequence about to be retired). Test: `overflow_gating`, `overflow_next_needed_sentinel` (first retirement, consumers at 0).
- **I-OV4 LatestUnaffected**: `latest()` readers always proceed under both policies (rejection only gates sequential retention). Test: `overflow_latest_proceeds`.
- **I-OV5 ZeroRequiredInvalid**: a `reject` topic with zero required sequential consumers is rejected at verify (`V-CMP`); at runtime a nonzero required count is a channel invariant, and a violated (zero) count returns `InvalidConfig` rather than silently overwriting or permanently returning `QueueFull`. Test: `overflow_zero_required`.

## Functions

```rust
fn allow_commit(ch: &Channel, ws: u64) -> Result<(), Status> {
 let (policy, R) = (ch.policy(), ch.depth());
 let fv = ch.ccb.first_valid_seq.load(Acquire);
 if policy == Reject {
 let reqs = ch.required_sequentials();
 if reqs.is_empty() { return Err(InvalidConfig); } // I-OV5: verifier also rejects
 if ws.saturating_sub(fv) >= R {
 let retire_seq = ws - R; // sequence that would be retired
 for req in reqs {
 if !may_retire(req, retire_seq) { return Err(QueueFull); } // I-OV2/I-OV3: immediate
 }
 }
 }
 Ok(()) // channel-publish.md owns every later step
}

fn may_retire(req: &RequiredSequential, retire_seq: u64) -> bool {
 req.ack_seq.load(Acquire) > retire_seq // I-OV3: strictly past retirement
}

// Consumer side: acknowledge after the read is complete, then publish the watermark for gating.
// copy_next: copy succeeds → advance local cursor → ack(seq)
// borrow_next: acquire → process → release hazard → ack(seq) (ack only after the borrow is released)
fn ack_completed(req: &ConsumerRecord, seq: u64) -> Result<(), Status> {
 let next = seq.checked_add(1).ok_or(SequenceExhausted)?;
 req.ack_seq.fetch_max(next, Release); // monotonic; concurrent/stale completion cannot regress
 Ok(())
}
```

*Overwrite path needs no gate:* ring rolls unconditionally; cursor-delivery.md turns lapping into exact `Gap { missed }`.

## Protocols

*Reject gating decision (publisher, per publish when ring full):*

1. Load each required consumer's `ack_seq` (acquire).
2. If any `<= ws − R` (that consumer still needs the sequence that would be retired) ⇒ return `QueueFull` immediately.
3. Else proceed with the normal commit path.
4. Publisher decides drop/throttle/alarm on `QueueFull` — middleware never stalls the real-time thread (architecture rule).

*Closed or crashed required consumer:* its last watermark keeps gating. Reattachment resumes from that
watermark; removing the requirement needs a new deployment. `drm-diag` reports the owner state and
blocking sequence.

## Edge Cases & Errors

| Case | Result |
|---|---|
| No required consumers + `reject` | build-time `V-CMP` error; runtime `InvalidConfig` (I-OV5) — never silent overwrite |
| First retirement in a run | required consumer must acknowledge the run's first retained sequence |
| Restart with large continued sequence | fullness uses `write_seq - first_valid_seq`, not absolute sequence |
| Required consumer advances past retirement point | publish proceeds |
| Mixed latest + required sequential consumers | latest unaffected (I-OV4) |
| `publish` under `reject` while ring not full | normal commit |

## Test Matrix

| Case | Expected |
|---|---|
| `overflow_reject_noblock` | ≤ required-count loads; zero waits/syscalls/retries |
| `overflow_gating` | succeeds iff all required consumers advanced |
| `overflow_next_needed_sentinel` | sequence 0 handled correctly from init |
| `overflow_zero_required` | verify error + runtime `InvalidConfig` |
| `overflow_overwrite_gaps` | exact missed counts under multi-lap |
| `overflow_latest_proceeds` | latest() never blocked/rejected |
| verify: `reject` with zero required consumers | build-time error (verify-engine.md) |
