# Observability Plane Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: the observability plane — `drm-diag` (out-of-band aggregation: counters, observed_rate, per-consumer deadline state, link stats), `drm-trace` (wait-free per-producer SPSC event lanes), and non-publishing introspection tools (`drm echo`, `drm hz`, `drm log`). Implements architecture.md §Two-Plane Architecture (Observability Plane), §What drm-diag Measures, §Tooling and Introspection. Owner: E1 (trace runtime), E4 (diag + tools). Phase 32. Depends on [channel-metadata.md](../runtime/channel-metadata.md) (counters, timing variant), [cursor-delivery.md](../runtime/cursor-delivery.md) (momentary borrow rule).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-diag-out-of-band` | diagnostic aggregation never executes on the data path; timing instrumentation compiled in only when declared semantics require it | `REQ-OPS-002` |
| `REQ-OPS-diag-measures` | observed_rate vs expected_rate; per-consumer deadline state; link stats | `REQ-OPS-001` |
| `REQ-OPS-diag-timing-modes` | topic `timing = auto/exact/off`; `auto` resolves `exact` only with a consumer deadline or jitter diagnostics; `exact` guarantees latest-sample age + retained inter-arrival, not historical miss counts | `REQ-OPS-016` |
| `REQ-CORE-ccb-timing-stamps` | timing-enabled CCB variant: `last_commit_mono_ns` + per-ring-entry `commit_mono_ns` | `REQ-CORE-094` |
| `REQ-OPS-trace-nonblocking` | trace emission never stalls producers; best-effort drop | `REQ-OPS-015` |
| `REQ-OPS-trace-compiled-ids` | event names/format strings compiled to metadata dict; runtime emits numeric IDs + args; arg size enforced at build | `REQ-OPS-014` |
| `REQ-OPS-trace-lanes` | fixed per-producer SPSC lanes claimed per thread; no shared producer state; diag merges by `(ts_ns, lane_id, lane_seq)` | `REQ-OPS-017` |
| `REQ-OPS-introspect-readonly` | tools never publish; each owns its diagnostic control record and releases borrows immediately | `REQ-OPS-003` |
| `REQ-CORE-runtime-counters` | channels expose drop/overrun/pool-exhaustion counters directly | `REQ-CORE-093` |

## Data Structures

```rust
// Trace (per-process lane set; SHM-mapped for diag consumption)
pub struct TraceRingSet { lanes: [TraceLane; N_LANES], no_lane_drops: AtomicU64 } // N fixed at activation
pub struct TraceLane { ring: FixedSpscRing<TraceEvent>, dropped: AtomicU64 } // single producer = owning thread
pub struct TraceEvent { id: u16, args: [u8; 16], ts_ns: u64 } // no global order counter; diag merges (ts_ns, lane_id, lane_seq)
// id indexes a build-time metadata dictionary: id → (event name, format string) — architecture §Staged Implementation.
// Arg size is a build-time property: codegen rejects >16-byte encodings (or selects a larger record class);
// no runtime size branch exists.

// Diag aggregation targets (read from CCB + trace lanes)
pub struct ChannelStats { last_sequence, drop_count, overrun_count, publisher_epoch,
 pool_exhausted, observed_rate_hz, last_commit_mono_ns } // channel facts only — no deadline fields
pub struct ConsumerStats { // per (topic, consumer endpoint); from deployment config
 consumer_endpoint_id, topic_id, deadline_ns,
 last_seen_seq: Option<u64>, last_commit_mono_ns: Option<u64>,
 miss_count: u64, state: DeadlineState }
pub enum DeadlineState { WaitingFirst, Current, Late }
pub struct LinkStats { nominal_bps, observed_bps, bucket_drops, lane_realtime_starve_events }
```

## Invariants

- **I-OB1 TraceNeverBlocks**: `trace_emit` on a registered thread executes exactly one bounded SPSC `try_push` on its TLS lane — wait-free: fixed step bound, no CAS retry loops, no shared producer state. Full lane ⇒ drop + lane counter; unregistered thread or exhausted lane set ⇒ drop + `no_lane_drops`; no dynamic growth. Test: `trace_never_blocks` (1/2/4/16 producers, stopped reader; step-count + tail-cost assertion), `trace_lane_exhaustion`.
- **I-OB2 DiagNoPathOverhead**: diagnostic aggregation never executes on the data path. Timing instrumentation is compiled into channels only when declared timing semantics require it (`timing = auto` ⇒ `exact` only if a consumer deadline or jitter/inter-arrival diagnostics exist on the topic; else `off`; `expected_rate` alone never enables it). Binary/call-graph comparison per resolved variant. Test: `diag_no_overhead`.
- **I-OB3 RateMeasurement**: `observed_rate = Δwrite_seq / Δdiag_time` over a sliding window; matches injected rates within tolerance; requires no commit timestamps. Test: `diag_observed_rate`.
- **I-OB4 DeadlineDetection**: per-consumer deadline state machine (`WaitingFirst → Current ⇄ Late`) detects declared deadline violations on `exact` channels. `miss_count` increments exactly once per `Current → Late` transition. Honest scope: latest-sample age and inter-arrival gaps among retained commits are exact; historical miss counts are observational — a violation that starts and recovers between diag polls is not counted (Protocols). Non-`exact` channels report no deadline state. Never-published topics stay `WaitingFirst`; publisher death after a sample keeps age growing (`Late` persists) with publisher state reported separately. Test: `diag_deadline_matrix` (burst vs even traffic at equal mean rate, multi-deadline consumers, startup silence, publisher death, recover-between-polls).
- **I-OB5 IntrospectNoPin**: `drm echo/hz` on bulk topics: momentary borrow → copy → release within bounded time; long-running tool holds zero hazard slots. Test: `introspect_no_pin` (pool stays exhaustible-free while tool runs).
- **I-OB6 DropTransparency**: per-lane `dropped` and set-level `no_lane_drops` counted and reported; never silently lost. Test: `trace_drop_counter`.
- **I-OB7 SnapshotEpochConsistent**: diag sampler reads `epoch → counters → epoch`; mismatch ⇒ discard + retry. Counters are cumulative lifetime; epoch-local deltas are computed by diag, never by resetting shared counters. Test: `diag_epoch_sandwich`.

## Functions

```rust
// Trace emit (producer thread — wait-free, I-OB1)
fn trace_emit(set: &TraceRingSet, id: u16, args: [u8; 16]) { // size enforced at build (I-OB2)
 let Some(lane) = TLS_TRACE_LANE.get() else { set.no_lane_drops.fetch_add(1, Relaxed); return; };
 let ev = TraceEvent { id, args, ts_ns: mono_now() };
 if !lane.ring.try_push(ev) { lane.dropped.fetch_add(1, Relaxed); } // I-OB6: drop, never block
}

// Lane registration — lazy for normal threads; explicit + pre-faulted before entering RT loops
fn trace_register_thread(set: &TraceRingSet) -> Result<(), Status> {
 let lane = set.claim_free_lane()?; // all lanes taken ⇒ Err(Busy); no dynamic growth
 TLS_TRACE_LANE.set(lane);
 Ok(())
}

// Diag sampler (separate process) — epoch sandwich (I-OB7)
fn diag_sample(ch: &ChannelView) -> ChannelStats {
 loop {
  let e1 = ch.epoch.load(Acquire);
  let s = ChannelStats { .. read all counters(Relaxed) .. }; // out-of-band; no data-path involvement
  if ch.epoch.load(Acquire) == e1 { return s; } // epoch changed mid-read ⇒ discard + retry
 }
}

// Consumer deadline tick (diag poll loop; exact channels only)
fn deadline_tick(c: &mut ConsumerStats, ch: &ChannelView, now_mono: u64) {
 c.last_commit_mono_ns = ch.last_commit_mono_ns.load(Relaxed); // None until first commit
 match c.state {
  WaitingFirst => if c.last_commit_mono_ns.is_some() { c.state = Current; },
  Current => if let Some(t) = c.last_commit_mono_ns {
   if now_mono.saturating_sub(t) >= c.deadline_ns { c.state = Late; c.miss_count += 1; } },
  Late => if let Some(t) = c.last_commit_mono_ns {
   if now_mono.saturating_sub(t) < c.deadline_ns { c.state = Current; } },
 }
}
// Exact inter-arrival gaps among retained commits: diag walks ring entries' `commit_mono_ns`
// (REQ-CORE-094) between polls. Entries overwritten before a poll leave no trace — honest
// scope, I-OB4.
```

## Protocols

*Timing-mode resolution (build time):*

1. Per topic, `timing` declared `auto` (default) | `exact` | `off` in the model (workspace-model.md).
2. `auto` resolves to `exact` iff any consumer of the topic declares `deadline` or jitter/inter-arrival diagnostics are requested; otherwise `off`. `expected_rate` alone never enables `exact`.
3. Resolved variant is fixed in the descriptor flags and layout hash (channel-metadata.md); attachment requires the exact variant.

*Deadline monitoring (runtime, `exact` channels):*

1. Diag builds `ConsumerStats` per (topic, consumer) from the deployment config — deadlines are consumer properties, never channel properties.
2. Poll loop: refresh `last_commit_mono_ns`, run `deadline_tick`, compute retained-gap statistics from entry stamps.
3. `miss_count` = one increment per `Current → Late` transition. Violations that start and recover between polls are not counted — diagnostics observe; they do not warranty history. Controllers needing hard staleness guarantees evaluate `now − latest_commit_time` in their own safety logic; diag reports the same fact out-of-band.
4. Publisher state (epoch liveness, ONLINE/OFFLINE) is reported separately from deadline state and never conflated with it.

*Compiled trace pipeline (staged per architecture):*

1. Build: `drm.log`/`trace!` call sites → metadata dictionary (name, format) → arg encodings checked against the record class (oversize ⇒ build error or generated larger class) → dictionary serialized into deployment artifacts.
2. Runtime: numeric IDs + fixed-size args only; `trace_register_thread` for RT threads before hot loops (lane pre-faulted); lazy claim otherwise.
3. Diag/reader: k-way merge by `(ts_ns, lane_id, lane_seq)`; dictionary lookup renders human text; unknown ID ⇒ raw hex (version skew visible, not silent). Cross-thread causality uses explicit correlation IDs, not a global counter.

*Dictionary versioning (pinned M14W3):*

1. The metadata dictionary is a build artifact carrying a `dict_version` = `(dictionary_format_version, BLAKE3(dict bytes) truncated to 16 bytes)`; the version is emitted into the deployment artifact set and the `TraceRingSet` header at activation.
2. The numeric `id → (name, format)` mapping is **append-only within a deployment**: compaction (`drm ids compact`) may renumber and emits a new `dict_version`; a reader holding an older version renders unknown IDs as raw hex plus a version-mismatch warning — never a wrong name (I-OB6, Edge Cases: trace dict version mismatch).
3. The runtime stores no strings; arg encodings are checked against the record class at build (oversize ⇒ build error or a generated larger record class). `dict_version` participates in the deployment identity so a mismatched reader is detectable before attach.

## Design Confirmation (M14W3)

Design reviewed against architecture §Two-Plane Architecture (Observability Plane),
§What drm-diag Measures, §Tooling and Introspection; no invariant changed.

- [x] Trace ring: wait-free per-producer SPSC lanes (`TraceRingSet`/`TraceLane`), fixed `N_LANES` at activation, no shared producer state (I-OB1, `REQ-OPS-015`/`REQ-OPS-017`).
- [x] `TraceEvent { id, args: [u8; 16], ts_ns }`; numeric IDs + build-time metadata dictionary (I-OB2, `REQ-OPS-014`).
- [x] Dictionary versioning pinned (above) — append-only per deployment; compaction renumbers under a new `dict_version`; version skew renders raw IDs, never wrong text.
- [x] Diag sampler epoch sandwich `epoch → counters → epoch`; counters cumulative; deltas computed by diag (I-OB7).
- [x] Deadline state machine `WaitingFirst → Current ⇄ Late`; one `miss_count` increment per `Current → Late`; non-`exact` channels report no deadline state (I-OB4, `REQ-OPS-016`).
- [x] Introspection tools never publish and release borrows immediately (I-OB5, `REQ-OPS-003`).

## Implementation Status (M15W1)

Phase 32 wave A landed the trace/diag slice. The lock-free SPSC primitive lives
in `drm-core::trace` (runtime layer) because codebase.md §2.2 confines `unsafe`
to `drm-core`/`drm-ffi`; the `drm-trace` crate composes it safely and lands with
the trace reader/merge. `drm-diag` (ops) aggregates counters, observed rate,
per-consumer deadline state, and the epoch sandwich, and provides the read-only
`Observer` used by `drm echo`/`hz`/`log`.

| Element | Location | Test |
|---|---|---|
| `TraceRingSet`/`TraceLane`/`TraceEvent`, `trace_emit`, `trace_register_thread` | `crates/drm-core/src/trace/mod.rs` | `trace_never_blocks`, `trace_lane_exhaustion`, `trace_drop_counter` |
| `TraceDict` + `dict_version` | `crates/drm-core/src/trace/mod.rs` | `trace_dict_roundtrip` |
| `diag_sample` (epoch sandwich), `deadline_tick`, `RateWindow`, `Observer` | `crates/drm-diag/src/lib.rs` | `diag_no_overhead`, `diag_epoch_sandwich`, `diag_observed_rate`, `diag_deadline_matrix`, `introspect_no_pin` |
| `drm echo`/`hz`/`log` | `crates/drm-cli/src/{echo,hz,log}.rs` | `echo_copies_and_reports_empty`, `hz_tolerance`, `render_and_filter` |

Live SHM attach for the tools is deferred to the diag session; the tools resolve
deployment metadata and report `NotFound` without a running publisher
(namespace scoping, Edge Cases). No invariant changed.

REQ: REQ-OPS-001, REQ-OPS-002, REQ-OPS-003, REQ-OPS-014, REQ-OPS-015,
REQ-OPS-016, REQ-OPS-017, REQ-CORE-093, REQ-CORE-094.

### Trace gate evidence (M16W2)

`cargo test -p drm-core trace` re-runs the never-blocks and drop-counter suites
as gate M17W2 evidence: `trace_never_blocks` (1/2/4/16 producers, stopped
reader, bounded tail cost; every excess emit counted once) and
`trace_drop_counter` (full lane retains exactly capacity, counts each excess
once) plus `trace_lane_exhaustion` (`no_lane_drops`, no growth). No invariant
changed.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Lane reader detached | new events drop; lane `dropped` grows (I-OB6) |
| All lanes claimed / unregistered thread | drop + `no_lane_drops`; no dynamic growth (I-OB1) |
| Diag reads during epoch change | epoch-sandwich discard + retry (I-OB7) |
| Violation starts and recovers between polls | not in `miss_count`; documented observational scope (I-OB4) |
| Topic never published | `WaitingFirst`; no deadline inference from silence alone |
| Publisher dies after last sample | age keeps growing ⇒ `Late` persists; publisher state reported separately |
| `timing=off` channel with a declared consumer deadline | build error (resolution conflict; workspace-model.md) |
| Tool attached to stale deployment namespace | NotFound (namespace scoping, deployment-identity.md) |
| Trace dict version mismatch | raw-ID mode + warning |
| echo on `reject` full topic | non-publishing diagnostic observer; its reserved control record never gates required acknowledgements |

## Test Matrix

| Case | Expected |
|---|---|
| `trace_never_blocks` (1/2/4/16 producers, stopped reader) | I-OB1 bounded steps, no wait path, tail cost reported |
| `trace_lane_exhaustion` | drops counted via `no_lane_drops`; no growth |
| `diag_no_overhead` (per timing variant) | I-OB2 within noise |
| `diag_observed_rate` (known rates; even vs burst) | I-OB3 tolerance |
| `diag_deadline_matrix` (equal-mean burst/even × deadlines × consumers; silence; publisher death; recover-between-polls) | I-OB4 exact-scope behavior |
| `introspect_no_pin` | I-OB5 |
| `trace_drop_counter` | I-OB6 |
| `diag_epoch_sandwich` (epoch change mid-snapshot) | I-OB7 |
