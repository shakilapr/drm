# Lanes & Shaping Algorithms

Scope: ordered-stream traffic classes — per-peer realtime and bulk lanes (never socket-per-topic), pre-stream per-class priority queues, unit-correct token-bucket shaping of DRM's own traffic. Implements architecture.md §Ordered-Stream Traffic Classes, §Priority Scheduling, §Traffic Shaping and Bandwidth Budgeting. Owner: E3 (crate `drm-router::{lane, shaping}`). Phase 30 (design from drafts; implementation ). Depends on [router-core.md](router-core.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-lanes-per-peer` | realtime lane + bulk lane per peer; never a socket per topic | — |
| `REQ-ROUTE-lane-no-cross-class` | latency-critical reliable traffic never queues behind bulk on the same ordered lane | — |
| `REQ-ROUTE-priority-queues-only` | priority applied in pre-stream class queues, before stream entry | — |
| `REQ-ROUTE-priority-order` | safety → control → status dequeue order into the realtime stream | — |
| `REQ-ROUTE-token-bucket` | configured token-bucket limits bound DRM's contribution to link load | — |
| `REQ-ROUTE-bucket-units` | bits/bytes units explicit; fractional credit preserved | — |
| `REQ-ROUTE-shaping-drm-only` | traffic from other applications is outside DRM's control (documented) | — |

## Data Structures

```rust
pub struct PeerLanes {
 // Pre-stream priority queues feeding the realtime ordered stream (index = class order).
 realtime_classes: [VecDeque<WireFrame>; 3], // 0 safety, 1 control, 2 status
 bulk: VecDeque<WireFrame>, // bulk ordered stream
 // best-effort (UDP/serial) bypasses lanes — single datagram path in router-core.md
}
pub struct LaneCaps { realtime: [usize; 3], bulk: usize } // static, from deployment generation

pub struct TokenBucket {
 rate_bits_per_sec: u64,
 burst_bytes: u64,
 credit_bit_ns: u128, // fixed-point bits·ns accumulator (no fractional loss)
 tokens_bytes: u64,
 last_ns: u64, // monotonic clock only
}
```

Priority classes (dequeue order into the realtime ordered stream — applied **before** stream entry):

| Class | Topics | Order |
|---|---|---|
| 0 safety | estop, safety interlocks | first |
| 1 control | actuator/command, control/* | then |
| 2 status/telemetry | state, battery, imu | then |
| 3 diagnostics/bulk | traces, logs, metrics, maps, frames, files | bulk lane |

## Invariants

- **I-LS1 ClassIsolation**: bulk saturation cannot delay realtime-lane delivery (separate ordered streams; bulk bytes never queue ahead of critical reliable messages). Test: `priority_saturation`.
- **I-LS2 FifoWithinClass**: per class queue, FIFO order preserved (ordered-stream contract). Test: `lane_fifo`.
- **I-LS3 BucketBound**: over any window W, bytes admitted ≤ `rate·W + burst`; never exceeds configured limit. Test: `shaping_property` (10k deterministic traces).
- **I-LS4 DrmOnlyScope**: shaping applies to DRM-generated frames only; foreign traffic unmanaged (documented behavior, not a bug). Test: doc-lint + runbook note.
- **I-LS5 CapsBounded**: caps static; overflow is per reliability — best-effort bulk drops oldest + counter, reliable bulk applies ingress backpressure (never silent drop); the realtime classes never silently drop. Test: `lane_caps`.
- **I-LS6 PriorityExact**: the realtime stream drains all safety frames before control, and control before status; a queued status frame never precedes an arriving safety frame. Test: `lane_class_priority`.
- **I-LS7 UnitCorrect**: refill converts `rate_bits_per_sec × ns` to bytes via an exact fixed-point accumulator; no unit mismatch and no permanent fractional loss. Test: `token_bucket_fractional`.

## Functions

```rust
fn enqueue(lanes: &mut PeerLanes, caps: &LaneCaps, class: PriorityClass, frame: WireFrame) -> Result<(), Status> {
 match class {
 Safety => push_capped(&mut lanes.realtime_classes[0], caps.realtime[0], frame),
 Control => push_capped(&mut lanes.realtime_classes[1], caps.realtime[1], frame),
 Status => push_capped(&mut lanes.realtime_classes[2], caps.realtime[2], frame),
 Diagnostics | Bulk => push_bulk(&mut lanes.bulk, caps.bulk, frame), // reliability-aware
 }
}

// Drain realtime ordered stream: strict class priority, FIFO within class (I-LS2/I-LS6)
fn drain_realtime(lanes: &mut PeerLanes, out: &mut impl FnMut(WireFrame)) {
 for class in 0..3 {
 while let Some(f) = lanes.realtime_classes[class].pop_front() { out(f); }
 }
}

fn token_bucket_refill(b: &mut TokenBucket, now_ns: u64) {
 let dt = now_ns.saturating_sub(b.last_ns) as u128;
 b.credit_bit_ns += b.rate_bits_per_sec as u128 * dt; // bits·ns accumulated exactly

 let one_byte_bit_ns = 8u128 * 1_000_000_000u128; // I-LS7: exact bits→bytes
 let gained = b.credit_bit_ns / one_byte_bit_ns;
 b.credit_bit_ns %= one_byte_bit_ns; // preserve fractional credit

 b.tokens_bytes = b.burst_bytes.min(b.tokens_bytes.saturating_add(gained as u64));
 b.last_ns = now_ns;
}

fn try_consume(b: &mut TokenBucket, cost_bytes: u64, now_ns: u64) -> bool {
 token_bucket_refill(b, now_ns);
 if b.tokens_bytes >= cost_bytes { b.tokens_bytes -= cost_bytes; true } else { false }
}
```

## Protocols

*Egress pipeline (per peer, per tick — router-core §egress feeds this):*

1. Classify frame (static per-topic class from workspace model).
2. Enqueue into the peer's class queue (safety/control/status) or bulk queue — priority already encoded by queue choice; no dynamic reordering inside a class (ordered stream).
3. Real-time drain: `drain_realtime` emits safety, then control, then status into the realtime ordered stream.
4. Before socket write: token-bucket admission (`try_consume(frame.bytes)`); denied frames wait in their queue (bulk/realtime backpressure) rather than being reordered across classes.
5. Socket write drains realtime stream first, then bulk.

*Sizing guidance:* lane caps derived from `expected_rate × frame_size × slack` at deployment generation (verify warns if caps < 2× nominal burst).

*Budget:* lane queues hold router-owned buffers (router-core.md I-RT8), never DRM borrows; their worst-case capacity (`Σ caps × max_frame_bytes` per peer) is included in the compute's DRM memory budget ([memory-budget.md](../memory-budget.md)).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Bulk queue full, best-effort route | drop-oldest bulk frame + counter (sample semantics preserved via sequence gaps) |
| Bulk queue full, `reliable` route | ingress backpressure; topic overflow governs the publisher — never a silent drop in a reliable lane (I-LS5) |
| Realtime class queue full | loud failure (`QueueFull` surfaced) — never silent drop (I-LS5) |
| Status frames queued, safety frame arrives | safety drains first (I-LS6) |
| Bucket empty, realtime frame waiting | admission waits ≤ one refill interval; sustained starvation alarms via diag (I-LS1) |
| Low rate (e.g. 1 byte/s, refills every 100 ms) | fractional `credit_bit_ns` retained; tokens never permanently starve (I-LS7) |
| Shaping misconfigured (rate 0) | verify rejects at build (V-REL/V-RT family) |
| Clock non-monotonic (NTP step) | refill uses monotonic clock only |

## Test Matrix

| Case | Expected |
|---|---|
| `priority_saturation` (bulk flood + critical burst) | I-LS1: realtime latency bounded |
| `lane_class_priority` (status queued, safety arrives) | I-LS6: safety first |
| `lane_fifo` (property) | I-LS2 |
| `shaping_property` (10k traces) | I-LS3: 0 violations |
| `token_bucket_fractional` (low rate, coarse refills) | I-LS7: no permanent starvation |
| `lane_caps` (best-effort vs reliable bulk) | I-LS5 policies exact |
| two-host physical matrix (M11) | limits hold within tolerance on real links |
