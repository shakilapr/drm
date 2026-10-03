# ADR-0001: Ops-plane execution model — single RPC event stream, per-producer trace lanes, resolved timing modes

## Status

`accepted`
Date: 2026-09-28 · Deciders: E1 (trace/RPC runtime), E4 (diag + record)

## Context and Problem Statement

A pre-implementation audit of the Phase-32 ops algorithms falsified several guarantees: RPC cancellation could not be serviced while user code ran, requests were consumed before response capacity was reserved, action states bypassed the pending-output machinery, wire deadlines were absolute clock readings, and server restarts left stale client requests executable. Observability counted deadline misses per channel although deadlines are consumer properties, claimed exact deadline history it could not observe, and specified a process-wide MPSC trace ring without a wait-free argument. Recording had nowhere to store a gap marker when its bounded buffer was full and sized every slot by the largest topic.

## Decision Drivers

- Cancellation must be real while user code runs.
- No unbounded or wait-time-unbounded structures in the ops plane.
- Deadlines are consumer properties; instrumentation cost must not be charged to channels that need none.

## Considered Options

- **Single ordered `event/<client>` stream** — chosen; replaces generated `response/` + `state/` topics.
- **Separate `response/<client>` + `state/<client>` topics** — rejected: two cursors, two validity states, impossible-state windows.
- **Process-wide MPSC trace ring** — rejected: no wait-free argument; unbounded contention.
- **Fixed per-producer SPSC trace lanes** — chosen; one claimed lane per thread.
- **Always-on timing instrumentation** — rejected: charges cost to channels with no deadline need.
- **`timing = auto` resolved at build** — chosen.

## Decision Outcome

Chosen: single ordered `event/<client>` stream; reactor/worker execution with cooperative cancellation and admission that reserves operation + terminal capacity before a request is consumed; fixed per-producer SP trace lanes (`TraceRingSet`, one TLS lane per thread, emit = one bounded `try_push`, overflow counted per lane plus `no_lane_drops`, diag merges by `(ts_ns, lane_id, lane_seq)`); `timing = auto` compiles commit timestamps only when a consumer declares a deadline or jitter/inter-arrival diagnostics, otherwise off; guarantees scoped to latest-sample age and retained inter-arrival.

### Positive Consequences

- Simpler RPC client: one sequential cursor, one validity state machine; impossible states rejectable.
- Cancellation is real: reactor-observed tokens bound acknowledgement to one loop.
- Trace emit has a defensible bounded-step operation; lane memory scales with threads and is declared in the budget.
- Hard-RT topics without deadline/jitter needs pay zero timing cost.

### Negative Consequences

- Per-producer lanes multiply trace capacity by thread count (bounded).
- A stalled client retains lifecycle events in its outbox (progress is coalesced).
- Commit timestamps add one relaxed store per publish on `exact` channels only.

## Confirmation

Phase gate for the ops plane (tests: RPC stream ordering, lane overflow, timing-mode resolution). Not yet executed; becomes a permanent gate on green.

## More Information

- Accepted ADRs are immutable; supersession creates a new record (codebase.md 6.3).
- Normative homes: `architecture.md` (Two-Plane Architecture, drm-rpc, drm-record); `docs/algorithms/ops/{observability-plane,record-replay,rpc-actions}.md`; `channel-metadata.md` (TIMING CCB variant).
