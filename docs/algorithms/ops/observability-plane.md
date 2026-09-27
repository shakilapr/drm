# Observability Plane Algorithms

Scope: the observability plane — `drm-diag` (out-of-band aggregation: counters, observed_rate, deadline misses, link stats), `drm-trace` (non-blocking best-effort compiled event rings), and non-publishing introspection tools (`drm echo`, `drm hz`, `drm log`). Implements architecture.md §Two-Plane Architecture (Observability Plane), §What drm-diag Measures, §Tooling and Introspection. Owner: E1 (trace runtime), E4 (diag + tools). Phase 32. Depends on [channel-metadata.md](../runtime/channel-metadata.md) (counters), [cursor-delivery.md](../runtime/cursor-delivery.md) (momentary borrow rule).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-diag-out-of-band` | aggregation never imposes data-path overhead | `REQ-OPS-002` |
| `REQ-OPS-diag-measures` | observed_rate vs expected_rate; deadline misses; link stats | `REQ-OPS-001` |
| `REQ-OPS-trace-nonblocking` | trace rings never stall publishers; best-effort drop | `REQ-OPS-015` |
| `REQ-OPS-trace-compiled-ids` | event names/format strings compiled to metadata dict; runtime emits numeric IDs + args | `REQ-OPS-014` |
| `REQ-OPS-introspect-readonly` | tools never publish; each owns its diagnostic control record and releases borrows immediately | `REQ-OPS-003` |
| `REQ-CORE-runtime-counters` | channels expose drop/overrun/pool-exhaustion counters directly | `REQ-CORE-093` |

## Data Structures

```rust
// Trace (per-process ring; SHM-mapped for diag consumption)
pub struct TraceRing {
 events: FixedMpscRing<TraceEvent>, // fixed capacity, multi-thread producers, drop-new
 dropped: AtomicU64, // drop counter surfaced to diag
}
pub struct TraceEvent { id: u16, args: [u8; 16], ts_ns: u64 }
// id indexes a build-time metadata dictionary: id → (event name, format string) — architecture §Staged Implementation

// Diag aggregation targets (read from CCB + trace rings):
pub struct ChannelStats { last_sequence, drop_count, overrun_count, publisher_epoch,
 pool_exhausted, observed_rate_hz, deadline_miss_count }
pub struct LinkStats { nominal_bps, observed_bps, bucket_drops, lane_realtime_starve_events }
```

## Invariants

- **I-OB1 TraceNeverBlocks**: `trace_emit` executes one bounded `try_push`; full ring ⇒ drop + counter, publisher never waits. Test: `trace_never_blocks` (slow/no reader, operation-count assertion).
- **I-OB2 DiagNoPathOverhead**: data path contains no diag code beyond existing atomic counters; attachment changes no publisher call graph or operation count. Test: `diag_no_overhead` (binary/call-graph comparison).
- **I-OB3 RateMeasurement**: `observed_rate` computed over sliding window; matches injected known rates within tolerance. Test: `diag_observed_rate`.
- **I-OB4 DeadlineDetection**: injected deadline misses detected (every architecture-defined condition). Test: `diag_deadline_matrix`.
- **I-OB5 IntrospectNoPin**: `drm echo/hz` on bulk topics: momentary borrow → copy → release within bounded time; long-running tool holds zero hazard slots. Test: `introspect_no_pin` (pool stays exhaustible-free while tool runs).
- **I-OB6 DropTransparency**: trace drops counted and reported; never silently lost. Test: `trace_drop_counter`.

## Functions

```rust
// Trace emit (publisher thread — wait-free)
fn trace_emit(ring: &TraceRing, id: u16, args: &[u8]) {
 let Ok(args) = <[u8; 16]>::try_from(args) else {
 ring.dropped.fetch_add(1, Relaxed);
 return;
 };
 if !ring.try_push(TraceEvent { id, args, ts_ns: mono_now() }) {
 ring.dropped.fetch_add(1, Relaxed); // I-OB1/I-OB6: drop, never block
 }
}

// Diag sampler (separate process)
fn diag_sample(channel: &ChannelMapping) -> ChannelStats {
 let s = ChannelStats {
 last_sequence: ccb.write_seq.load(Relaxed).checked_sub(1),
 .. read all counters(Relaxed) // out-of-band reads; no data-path involvement
 };
 s.observed_rate_hz = sliding_window_rate(samples_seen);
 s
}

// Introspection (drm echo)
fn echo_once(topic: &TopicPath) -> Result<Owned, Status> {
 let entry = consumer_latest(topic)?;
 let copy = channel.copy_latest(entry)?; // momentary borrow internally
 Ok(copy)
}
```

## Protocols

*Deadline monitoring:*

1. Consumer-side deadline declarations (`consume.deadline`) known to deployment config.
2. Diag observes inter-sample arrival gaps against the declared deadline; it does not infer application compute or borrow duration.
3. Violations counted per channel; surfaced in `drm hz`/diag report (architecture: observed vs declared).

*Compiled trace pipeline (staged per architecture):*

1. Build: `drm.log`/`trace!` call sites → metadata dictionary (name, format) → dictionary serialized into deployment artifacts.
2. Runtime: numeric IDs + fixed-size args only.
3. Diag/reader: dictionary lookup renders human text; unknown ID ⇒ raw hex (version skew visible, not silent).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Ring reader detached | new events drop; drop counter grows (I-OB6) |
| Diag reads during epoch change | counters epoch-scoped; stale epoch reads discarded |
| Tool attached to stale deployment namespace | NotFound (namespace scoping, deployment-identity.md) |
| Trace dict version mismatch | raw-ID mode + warning |
| echo on `reject` full topic | non-publishing diagnostic observer; its reserved control record never gates required acknowledgements |

## Test Matrix

| Case | Expected |
|---|---|
| `trace_never_blocks` | I-OB1 bounded operation count; no wait path |
| `diag_no_overhead` | I-OB2 within noise |
| `diag_observed_rate` (known rates) | I-OB3 tolerance |
| `diag_deadline_matrix` (all injected conditions) | I-OB4 |
| `introspect_no_pin` | I-OB5 |
| `trace_drop_counter` | I-OB6 |
