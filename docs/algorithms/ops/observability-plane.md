# Observability Plane Algorithms

Scope: the observability plane — `drm-diag` (out-of-band aggregation: counters, observed_rate, deadline misses, link stats), `drm-trace` (non-blocking best-effort compiled event rings), and read-only introspection tools (`drm echo`, `drm hz`, `drm log`). Implements architecture.md §Two-Plane Architecture (Observability Plane), §What drm-diag Measures, §Tooling and Introspection. Owner: E1 (trace runtime), E4 (diag + tools). Phase 32. Depends on [channel-metadata.md](../runtime/channel-metadata.md) (counters), [cursor-delivery.md](../runtime/cursor-delivery.md) (momentary borrow rule).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-diag-out-of-band` | aggregation never imposes data-path overhead | — |
| `REQ-OPS-diag-measures` | observed_rate vs expected_rate; deadline misses; link stats | — |
| `REQ-OPS-trace-nonblocking` | trace rings never stall publishers; best-effort drop | — |
| `REQ-OPS-trace-compiled-ids` | event names/format strings compiled to metadata dict; runtime emits numeric IDs + args | — |
| `REQ-OPS-introspect-readonly` | tools read-only; momentary borrow+copy on bulk; never pin slots | — |

## Data Structures

```rust
// Trace (per-process ring; SHM-mapped for diag consumption)
pub struct TraceRing {
 events: RingBuf<TraceEvent>, // bounded, overwrite-oldest
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

- **I-OB1 TraceNeverBlocks**: `trace_emit` is wait-free; full ring ⇒ drop + counter, publisher never stalls. Test: `trace_never_blocks` (slow/no reader, publisher loop timing bound).
- **I-OB2 DiagNoPathOverhead**: data path contains no diag code beyond existing atomic counters (reads happen out-of-band from another process). Test: `diag_no_overhead` (with/without diag attached, latency unchanged within noise).
- **I-OB3 RateMeasurement**: `observed_rate` computed over sliding window; matches injected known rates within tolerance. Test: `diag_observed_rate`.
- **I-OB4 DeadlineDetection**: injected deadline misses detected (every architecture-defined condition). Test: `diag_deadline_matrix`.
- **I-OB5 IntrospectNoPin**: `drm echo/hz` on bulk topics: momentary borrow → copy → release within bounded time; long-running tool holds zero hazard slots. Test: `introspect_no_pin` (pool stays exhaustible-free while tool runs).
- **I-OB6 DropTransparency**: trace drops counted and reported; never silently lost. Test: `trace_drop_counter`.

## Functions

```rust
// Trace emit (publisher thread — wait-free)
fn trace_emit(ring: &TraceRing, id: u16, args: &[u8]) {
 if !ring.try_push(TraceEvent { id, args: args.try_into()?, ts_ns: mono_now() }) {
 ring.dropped.fetch_add(1, Relaxed); // I-OB1/I-OB6: drop, never block
 }
}

// Diag sampler (separate process)
fn diag_sample(channel: &ChannelMapping) -> ChannelStats {
 let s = ChannelStats {
 last_sequence: ccb.last_sequence.load(Relaxed),
 .. read all counters(Relaxed) // out-of-band reads; no data-path involvement
 };
 s.observed_rate_hz = sliding_window_rate(samples_seen);
 s
}

// Introspection (drm echo)
fn echo_once(topic: &TopicPath) -> Result<Owned, Status> {
 let entry = consumer_latest(topic)?;
 let t0 = mono_now();
 let copy = channel.copy_latest(entry)?; // momentary borrow internally
 debug_assert!(mono_now() - t0 < BORROW_BUDGET); // I-OB5 budget
 Ok(copy)
}
```

## Protocols

*Deadline monitoring:*

1. Consumer-side deadline declarations (`consume.deadline`) known to deployment config.
2. Diag observes inter-sample gaps + borrow latency vs declared deadline.
3. Violations counted per channel; surfaced in `drm hz`/diag report (architecture: observed vs declared).

*Compiled trace pipeline (staged per architecture):*

1. Build: `drm.log`/`trace!` call sites → metadata dictionary (name, format) → dictionary serialized into deployment artifacts.
2. Runtime: numeric IDs + fixed-size args only.
3. Diag/reader: dictionary lookup renders human text; unknown ID ⇒ raw hex (version skew visible, not silent).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Ring reader detached | overwrite-oldest continues; drop counter grows (I-OB6) |
| Diag reads during epoch change | counters epoch-scoped; stale epoch reads discarded |
| Tool attached to stale deployment namespace | NotFound (namespace scoping, deployment-identity.md) |
| Trace dict version mismatch | raw-ID mode + warning |
| echo on `reject` full topic | read-only observer — never publishes, never gates |

## Test Matrix

| Case | Expected |
|---|---|
| `trace_never_blocks` | I-OB1 timing bound |
| `diag_no_overhead` | I-OB2 within noise |
| `diag_observed_rate` (known rates) | I-OB3 tolerance |
| `diag_deadline_matrix` (all injected conditions) | I-OB4 |
| `introspect_no_pin` | I-OB5 |
| `trace_drop_counter` | I-OB6 |
