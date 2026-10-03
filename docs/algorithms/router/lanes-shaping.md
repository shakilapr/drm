# Lanes & Shaping Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: per-link scheduler spanning all transports — admission, class queues, a dedicated safety budget isolated from non-safety token state, control/status deficit round robin, a non-safety shaper, route export limits; ordered realtime/bulk lanes only for reliable ordered streams (never socket-per-topic). Unit-correct shaping of DRM traffic with a declared charge; safety latency includes non-preemptible transmission as an explicit verifier term. Implements architecture.md §The Boundary Router (Ordered-Stream Traffic Classes, Priority Scheduling, Traffic Shaping), §Transport Metadata. Owner: E3 (crate `drm-router::{lane, shaping}`). Phase 30. Depends on [router-core.md](router-core.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-link-scheduler` | per-link scheduler shapes all transports; only reliable streams use ordered lanes | `REQ-ROUTE-034` |
| `REQ-ROUTE-lanes-per-peer` | realtime lane + bulk lane per peer; never a socket per topic | `REQ-ROUTE-013` |
| `REQ-ROUTE-lane-no-cross-class` | latency-critical reliable traffic never queues behind bulk on the same ordered lane | `REQ-ROUTE-012` |
| `REQ-ROUTE-priority-queues-only` | priority applied in pre-stream class queues, before stream entry | `REQ-ROUTE-021` |
| `REQ-ROUTE-priority-order` | safety strict-first on a dedicated budget; control/status DRR; no starvation | `REQ-ROUTE-020` |
| `REQ-ROUTE-safety-budget` | safety token state isolated from non-safety; budgets carved from link capacity | `REQ-ROUTE-045` |
| `REQ-ROUTE-safety-latency` | safety worst-case latency bound includes non-preemptible transmission | `REQ-ROUTE-046` |
| `REQ-ROUTE-token-bucket` | token-bucket limits bound DRM's contribution to link load; burst ≥ max charged frame | `REQ-ROUTE-030` |
| `REQ-ROUTE-shaping-charge` | charge = declared per-link wire-frame bytes | `REQ-ROUTE-039` |
| `REQ-ROUTE-export-limit` | route-level `export_limit = "10Hz"` admits at most one sample per generated period | `REQ-ROUTE-008` |
| `REQ-ROUTE-bucket-units` | bits/bytes units explicit; fractional credit preserved | `REQ-ROUTE-002` |
| `REQ-ROUTE-shaping-drm-only` | traffic from other applications is outside DRM's control (documented) | `REQ-ROUTE-028` |

## Data Structures

```rust
pub struct LinkScheduler { // per link — spans ALL transports (REQ-ROUTE-034)
 classes: [FixedRing<OwnedFrameSlot>; 3], // 0 safety, 1 control, 2 status (router-owned, pre-transport)
 bulk: FixedRing<OwnedFrameSlot>, // best-effort bulk
 safety_budget: TokenBucket, // DEDICATED to class 0 — non-safety traffic never charges it (REQ-ROUTE-045)
 non_safety_shaper: TokenBucket, // control/status/bulk only
 caps: LaneCaps, // static, from deployment generation
 drr_deficit: [u32; 2], // control/status DRR state (quanta 4:1), reset on link reset
}
pub struct LaneCaps { realtime: [usize; 3], bulk: usize } // static, from deployment generation
pub struct PeerLanes { // only reliable ordered streams: ordered lanes per peer (REQ-ROUTE-013)
 realtime: OrderedStream, // class-scheduled frames enter here in dequeue order
 bulk: OrderedStream, // separate ordered stream (REQ-ROUTE-012)
}
pub struct TokenBucket {
 rate_bits_per_sec: u64,
 burst_bytes: u64, // verify: burst ≥ max charge of the traffic that charges this bucket (I-LS9)
 credit_bit_ns: u128, // 0..=burst_bytes·8e9
 last_ns: u64, // monotonic clock only
}
pub struct ExportLimiter { period_ns: u64, next_allowed_ns: u64 }
```

*Charge definition (I-LS9):* `charge_bytes` = the full encoded wire frame as submitted to the transport — header + payload + trailers + this link's framing overhead (UDP/IP headers on datagram links, COBS expansion on serial). OS/lower-layer overhead is outside DRM's charge and is measured at runtime (I-LS4).

Priority classes (scheduler service order — applied before stream entry or datagram/serial egress):

| Class | Topics | Service |
|---|---|---|
| 0 safety | estop, safety interlocks | strict first on its own dedicated budget; non-safety tokens never consumed |
| 1 control | actuator/command, control/* | DRR quantum 4 |
| 2 status/telemetry | state, battery, imu | DRR quantum 1 |
| 3 diagnostics/bulk | traces, logs, metrics, maps, frames, files | bulk; drained after realtime |

## Invariants

- **I-LS1 DrmBoundaryIsolation**: bulk bytes never queue ahead of realtime bytes in DRM's own queues and ordered streams. Downstream shared resources (kernel socket buffers, qdisc, NIC, switches, remote kernel) can still add latency; end-to-end isolation is a deployment QoS concern and is measured, not guaranteed by lane separation. Test: `priority_saturation` (bulk flood + critical burst; DRM-queue latency bounded; physical matrix reports shared-link reality).
- **I-LS2 FifoWithinClass**: per class queue, FIFO order preserved (ordered-stream contract). Test: `lane_fifo`.
- **I-LS3 BucketBound**: over any window W seconds, bytes admitted ≤ `rate_bits_per_sec·W/8 + burst_bytes`; never exceeds the configured limit. Test: `shaping_property` (10k deterministic traces, byte accounting).
- **I-LS4 DrmOnlyScope**: shaping applies to DRM-generated frames only; foreign traffic unmanaged (documented behavior, not a bug). Test: doc-lint + runbook note.
- **I-LS5 CapsBounded**: caps static; overflow is per reliability — best-effort bulk drops oldest + counter, reliable bulk applies publisher backpressure (never silent drop); realtime classes never silently drop; a stalled reliable receiver fails its branch session, never grows memory ([router-core.md](router-core.md) I-RT17). `stall_window` is generated (reliable lane capacity ÷ declared minimum drain rate), never a magic constant. Test: `lane_caps`.
- **I-LS6 BoundedPriority**: safety has its own `safety_budget`; non-safety token consumption can never delay safety admission. When the physical transmitter becomes eligible, safety is selected strictly before control/status/bulk. Ordinary transports do not preempt transmission, so a safety frame may still wait for at most one already-committed lower-priority transmission plus adapter/OS buffering — that is the `T_inflight_lower_priority` term of I-LS11, not a token-bucket effect. Control/status get DRR quanta 4:1; no class waits indefinitely. Test: `lane_class_priority`, `lane_no_starvation`, `safety_budget_isolated`.
- **I-LS7 UnitCorrect**: refill converts `rate_bits_per_sec × ns` to bytes via an exact fixed-point accumulator; no unit mismatch and no permanent fractional loss. Test: `token_bucket_fractional`.
- **I-LS8 ExportLimit**: per `(topic, link)` limiter admits at most one frame per declared period; rejected frames are counted and never queued. Test: `export_limit`.
- **I-LS9 ChargeAndBurst**: every shaped route charges the declared `charge_bytes` (charge definition above). Verify requires `safety_burst_bytes ≥ max safety charge` and `non_safety_burst_bytes ≥ max non-safety charge`; a frame larger than its bucket's burst could never send (no fragmentation). Admission eligibility: `t ≈ 8·(charge_bytes − credit_bytes)/rate_bits_per_sec` + scheduler jitter. Test: `token_bucket_burst_bound`, `shaping_wait_model`.
- **I-LS10 AllTransportsScheduled**: datagram and serial egress pass through the same admission, class scheduling, and budget/shaper as reliable streams; only ordered-lane semantics are stream-specific. Test: `scheduler_transports` (instrument reserve counts > 0 for UDP/serial routes).
- **I-LS11 SafetyLatencyBound**: `W_safety ≤ T_scheduler + T_inflight_lower_priority + T_adapter_queue + T_transport + T_safety_budget_wait`. `T_inflight_lower_priority` is zero only where the transport provides enabled preemption (then bounded by the maximum non-preemptible fragment); otherwise it is one maximum lower-priority wire transmission — serial: one max encoded frame (I-SR7); CAN: one active frame plus arbitration effects; Ethernet/UDP without TSN: OS/NIC queue effects. Verify computes this bound including `max_nonpreemptible_wire_time` and reports it against declared deadlines. Test: `safety_latency_bound`.
- **I-LS12 CapacityFeasible**: the two budgets are independent in token state but carved from one physical link — `safety_budget.rate ≤ link_sustainable_rate` and `non_safety_shaper.rate ≤ link_sustainable_rate − safety_budget.rate`. Over-commit is a verify error. Test: `shaping_capacity_feasible`.

## Functions

```rust
fn enqueue(s: &mut LinkScheduler, class: PriorityClass, frame: WireFrame) -> Result<(), Status> {
 // Route-scoped (router-core I-RT15): caller counts and continues the scan on Err.
 match class {
  Safety => push_capped(&mut s.classes[0], s.caps.realtime[0], frame),
  Control => push_capped(&mut s.classes[1], s.caps.realtime[1], frame),
  Status => push_capped(&mut s.classes[2], s.caps.realtime[2], frame),
  Diagnostics | Bulk => push_bulk(&mut s.bulk, s.caps.bulk, frame), // reliability-aware
 }
}

// One scheduler pass (per tick, per link) — I-LS6/I-LS9/I-LS12:
// Safety charges ONLY safety_budget; control/status/bulk charge ONLY non_safety_shaper.
// Emission order is the strict transmission select order: safety first, always.
fn schedule_pass(s: &mut LinkScheduler, out: &mut impl FnMut(WireFrame, StreamKind)) {
 // 1. safety: strict first, on its dedicated budget (non-safety cannot deplete it)
 while let Some(f) = s.classes[0].peek_front() {
  if !try_consume(&mut s.safety_budget, charge_bytes(&f), now()) { break; } // denied ⇒ stays queued
  s.classes[0].pop_front();
  out(f, Safety);
 }
 // 2. control/status: deficit round robin, quanta 4:1, through the non-safety shaper.
 // A queue the shaper blocks is skipped this pass; the other continues.
 drr_pass(&mut s.classes[1], &mut s.classes[2], &mut s.drr_deficit, &[4, 1], &mut s.non_safety_shaper, out);
 // 3. bulk: after realtime, same non-safety shaper
 if let Some(f) = s.bulk.peek_front() {
  if try_consume(&mut s.non_safety_shaper, charge_bytes(&f), now()) { s.bulk.pop_front(); out(f, Bulk); }
 }
}
// charge_bytes(frame) = this link's adapter max_wire_size (feasibility-estimation I-F7) — one definition of charged bytes.

fn token_bucket_refill(b: &mut TokenBucket, now_ns: u64) {
 let dt = now_ns.saturating_sub(b.last_ns) as u128;
 let capacity = (b.burst_bytes as u128).saturating_mul(8_000_000_000);
 let added = (b.rate_bits_per_sec as u128).saturating_mul(dt).min(capacity);
 b.credit_bit_ns = b.credit_bit_ns.saturating_add(added).min(capacity);
 b.last_ns = now_ns;
}

fn try_consume(b: &mut TokenBucket, charge_bytes: u64, now_ns: u64) -> bool {
 token_bucket_refill(b, now_ns);
 let cost = (charge_bytes as u128).saturating_mul(8_000_000_000); // bytes → bit·ns fixed point
 if b.credit_bit_ns >= cost { b.credit_bit_ns -= cost; true } else { false }
 // verify guarantees charge_bytes ≤ this bucket's burst_bytes (I-LS9): every frame is eventually admissible
}

fn export_admit(l: &mut ExportLimiter, now_ns: u64) -> bool {
 if now_ns < l.next_allowed_ns { return false; }
 l.next_allowed_ns = now_ns.saturating_add(l.period_ns);
 true
}
```

## Protocols

*Egress pipeline (per link, per tick — router-core §egress feeds this):*

1. Apply the generated per-route export limiter; a denied frame increments `export_limited` (the router-core peek applies it — I-RT13/I-LS8).
2. Classify and enqueue into the link scheduler's class or bulk queue (route-scoped failures, I-RT15).
3. `schedule_pass`: safety on its dedicated budget → DRR control/status → bulk on the non-safety shaper. Admission happens inside the pass (I-LS9/I-LS12) before a frame leaves its queue; a denied frame stays queued in FIFO order — never reordered across classes. Emission order is the strict transmission select order (safety first), subject to the physical link's non-preemptibility (I-LS11).
4. Transport egress: frames enter the peer's ordered stream in dequeue order (reliable) or go to UDP datagram send / serial frame write (realtime stream first, then bulk).

*Sizing guidance:* lane caps derived from `expected_rate × frame_size × slack`. Verify hard-errors on `safety_burst < max safety charge` or `non_safety_burst < max non-safety charge` (I-LS9), on `safety_budget.rate > link_sustainable_rate`, and on `safety_budget.rate + non_safety_shaper.rate > link_sustainable_rate` (I-LS12, capacity carving). It warns if caps < 2× nominal burst and reports the I-LS11 safety latency bound against declared deadlines. Where the transport lacks preemption, that bound includes one max lower-priority frame — make safety worst-case acceptable via a smaller max frame, a faster link, or a preemption-capable link.

*Budget:* scheduler queues hold router-owned buffers (router-core.md I-RT8), never DRM borrows; worst-case capacity (`Σ caps × max_frame_bytes` per link) is included in the compute's DRM memory budget ([memory-budget.md](../memory-budget.md)).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Bulk queue full, best-effort route | drop-oldest bulk frame + counter (sample semantics preserved via sequence gaps) |
| Bulk queue full, `reliable` route | publisher backpressure per topic overflow contract; stalled receiver past the window ⇒ branch session fails (router-core I-RT17) — never silent drop, never unbounded growth (I-LS5) |
| Realtime class queue full | loud, route-scoped failure (`QueueFull` surfaced for that route) — never silent drop (I-LS5/I-RT15) |
| Safety publisher exceeds its dedicated budget | excess safety frames wait queued + counter; safety budget never bypassed, non-safety classes unaffected (I-LS6) |
| Control/status/bulk deplete non-safety tokens | safety admission unaffected — dedicated budget (I-LS6/I-LS12) |
| Status frames queued, safety frame arrives | safety selected first (I-LS6) |
| Lower-priority frame already transmitting when safety arrives | safety waits ≤ `T_inflight_lower_priority` (one frame, or max preemptible fragment where supported); included in the I-LS11 verify bound |
| Bucket empty, frame waiting | admission waits `t ≈ 8·(charge_bytes − credit_bytes)/rate` + jitter; each bucket's `burst ≥ max charge` guarantees eventual admission (I-LS9); sustained starvation alarms via diag |
| Low rate (e.g. 1 byte/s, refills every 100 ms) | fractional `credit_bit_ns` retained; tokens never permanently starve (I-LS7) |
| `charge_bytes` > its bucket's burst | verify build error (I-LS9) — never a permanently unsendable frame |
| Class budgets exceed link capacity | verify build error (I-LS12) — carrying capacity, not token state |
| Shaping misconfigured (rate 0) | verify rejects at build (V-REL/V-RT family) |
| Clock non-monotonic (NTP step) | refill uses monotonic clock only |

## Test Matrix

| Case | Expected |
|---|---|
| `scheduler_transports` (reserve instrumentation: UDP/serial/stream routes) | I-LS10: all transports scheduled; only streams use ordered lanes |
| `priority_saturation` (bulk flood + critical burst) | I-LS1: DRM-queue latency bounded; physical matrix measured |
| `lane_class_priority` (status queued, safety arrives) | I-LS6: safety first |
| `lane_no_starvation` (2× capacity safety flood, 1k ticks) | I-LS6: control/status serviced every window; safety capped by its budget |
| `safety_budget_isolated` (bulk flood drains non-safety tokens; safety arrives) | I-LS6: safety admitted from its own budget |
| `safety_latency_bound` (verify report incl. non-preemptible term) | I-LS11: bound reported vs declared deadlines |
| `shaping_capacity_feasible` (over-committed class budgets) | I-LS12: verify error |
| `lane_fifo` (property) | I-LS2 |
| `shaping_property` (10k traces, byte accounting) | I-LS3: `bytes ≤ rate·W/8 + burst`, 0 violations |
| `token_bucket_fractional` (low rate, coarse refills) | I-LS7: no permanent starvation |
| `token_bucket_burst_bound` (declared frame > its bucket's burst) | I-LS9: verify error; never unsendable |
| `shaping_wait_model` (empty bucket, known frame) | I-LS9: measured wait within model + jitter |
| `export_limit` (clock boundary matrix) | I-LS8 exact period |
| `lane_caps` (best-effort vs reliable bulk) | I-LS5 policies exact |
| two-host physical matrix (M11) | limits hold within tolerance on real links; shared-link latency reported |
