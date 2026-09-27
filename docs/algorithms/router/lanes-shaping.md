# Lanes & Shaping Algorithms

Scope: ordered-stream traffic classes — per-peer realtime and bulk lanes (never socket-per-topic), pre-stream per-class priority queues, route export limits, and unit-correct shaping of DRM traffic. Implements architecture.md §Ordered-Stream Traffic Classes, §Priority Scheduling, §Traffic Shaping and Bandwidth Budgeting. Owner: E3 (crate `drm-router::{lane, shaping}`). Phase 30. Depends on [router-core.md](router-core.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-lanes-per-peer` | realtime lane + bulk lane per peer; never a socket per topic | `REQ-ROUTE-013` |
| `REQ-ROUTE-lane-no-cross-class` | latency-critical reliable traffic never queues behind bulk on the same ordered lane | `REQ-ROUTE-012` |
| `REQ-ROUTE-priority-queues-only` | priority applied in pre-stream class queues, before stream entry | `REQ-ROUTE-021` |
| `REQ-ROUTE-priority-order` | safety → control → status dequeue order into the realtime stream | `REQ-ROUTE-020` |
| `REQ-ROUTE-token-bucket` | configured token-bucket limits bound DRM's contribution to link load | `REQ-ROUTE-030` |
| `REQ-ROUTE-export-limit` | route-level `export_limit = "10Hz"` admits at most one sample per generated period | `REQ-ROUTE-008` |
| `REQ-ROUTE-bucket-units` | bits/bytes units explicit; fractional credit preserved | `REQ-ROUTE-002` |
| `REQ-ROUTE-shaping-drm-only` | traffic from other applications is outside DRM's control (documented) | `REQ-ROUTE-028` |

## Data Structures

```rust
pub struct PeerLanes {
 // Pre-stream priority queues feeding the realtime ordered stream (index = class order).
 realtime_classes: [FixedRing<OwnedFrameSlot>; 3], // storage allocated at activation
 bulk: FixedRing<OwnedFrameSlot>,
 // best-effort (UDP/serial) bypasses lanes — single datagram path in router-core.md
}
pub struct LaneCaps { realtime: [usize; 3], bulk: usize } // static, from deployment generation

pub struct TokenBucket {
 rate_bits_per_sec: u64,
 burst_bytes: u64,
 credit_bit_ns: u128, // 0..=burst_bytes·8e9
 last_ns: u64, // monotonic clock only
}
pub struct ExportLimiter { period_ns: u64, next_allowed_ns: u64 }
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
- **I-LS8 ExportLimit**: per `(topic, link)` limiter admits at most one frame per declared period; rejected frames are counted and never queued. Test: `export_limit`.

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
 let capacity = (b.burst_bytes as u128).saturating_mul(8_000_000_000);
 let added = (b.rate_bits_per_sec as u128).saturating_mul(dt).min(capacity);
 b.credit_bit_ns = b.credit_bit_ns.saturating_add(added).min(capacity);
 b.last_ns = now_ns;
}

fn try_consume(b: &mut TokenBucket, cost_bytes: u64, now_ns: u64) -> bool {
 token_bucket_refill(b, now_ns);
 let cost = (cost_bytes as u128).saturating_mul(8_000_000_000);
 if b.credit_bit_ns >= cost { b.credit_bit_ns -= cost; true } else { false }
}

fn export_admit(l: &mut ExportLimiter, now_ns: u64) -> bool {
 if now_ns < l.next_allowed_ns { return false; }
 l.next_allowed_ns = now_ns.saturating_add(l.period_ns);
 true
}
```

## Protocols

*Egress pipeline (per peer, per tick — router-core §egress feeds this):*

1. Apply the generated per-route export limiter; a denied frame increments `export_limited`.
2. Classify and enqueue into the peer's fixed-capacity class or bulk queue.
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
| `export_limit` (clock boundary matrix) | I-LS8 exact period |
| `lane_caps` (best-effort vs reliable bulk) | I-LS5 policies exact |
| two-host physical matrix (M11) | limits hold within tolerance on real links |
