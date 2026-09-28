# ADR-0001: Ops-plane execution model — single RPC event stream, per-producer trace lanes, resolved timing modes

- **Status**: accepted
- **Date**: 2026-09-28
- **Deciders**: E1 (trace/RPC runtime), E4 (diag + record)

## Context

A pre-implementation audit of the Phase-32 ops algorithms falsified several guarantees:
RPC cancellation could not be serviced while user code ran, requests were consumed before
response capacity was reserved, action states bypassed the pending-output machinery, wire
deadlines were absolute clock readings, and server restarts left stale client requests
executable. Observability counted deadline misses per channel although deadlines are
consumer properties, claimed exact deadline history it could not observe, and specified a
process-wide MPSC trace ring without a wait-free argument. Recording had nowhere to store
a gap marker when its bounded buffer was full and sized every slot by the largest topic.

## Decision

Three choices:

1. **Single ordered `event/<client>` stream.** RPC replaces generated `response/<client>` +
   `state/<client>` topics with one server-owned ordered stream (`request`/`cancel` stay
   separate). The topic sequence is the total operation order; no extra `event_seq`.
   Execution is reactor/worker: a reactor polls requests, cancels, and session health and
   flushes bounded outboxes; user code runs only on a bounded worker pool with cooperative
   cancellation tokens. Admission reserves operation + terminal output capacity before a
   request is consumed. The wire carries relative `remaining_budget_ns`, a random
   `server_session` rejects stale requests across restarts, and RPC routes must be reliable.
2. **Fixed per-producer SPSC trace lanes.** `TraceRingSet` of preallocated SPSC lanes, one
   claimed per thread (TLS); emit is one bounded `try_push`; overflow counts per lane plus
   `no_lane_drops`. Diag merges by `(ts_ns, lane_id, lane_seq)`; no global order counter.
3. **`timing = auto` resolved at build.** Default `auto` compiles commit timestamps
   (`last_commit_mono_ns`, per-entry `commit_mono_ns`) only when a consumer declares a
   deadline or jitter/inter-arrival diagnostics; otherwise `off`. `expected_rate` alone
   never enables instrumentation. Guarantees are scoped to latest-sample age and retained
   inter-arrival; historical miss counts are observational only.

## Consequences

- Simpler RPC client: one sequential cursor and one validity state machine; impossible
  states (post-`Terminal`, `Progress` before `Started`) are rejectable.
- Cancellation is real: reactor-observed tokens bound acknowledgement to one loop; phase
  `expected_max_duration_ns` becomes SLO metadata, not an unenforceable preemption bound.
- Trace emit has a defensible bounded-step operation; lane memory scales with threads and
  is declared in the budget.
- Hard-RT topics without deadline/jitter needs pay zero timing cost; `auto` keeps deadline
  channels instrumented without explicit opt-in.
- Tradeoffs: per-producer lanes multiply trace capacity by thread count (bounded); a single
  event stream means a stalled client retains lifecycle events in its outbox (progress is
  coalesced); commit timestamps add one relaxed store per publish only on `exact` channels.

## Notes

- Accepted ADRs are immutable; supersession creates a new record (codebase.md §6.3).
- ADRs record rationale only — the architecture docs remain normative and win conflicts.
- Normative homes: `architecture.md` §Two-Plane Architecture, §drm-rpc, §drm-record;
  `docs/algorithms/ops/{observability-plane,record-replay,rpc-actions}.md`;
  `channel-metadata.md` (TIMING CCB variant, REQ-CORE-094).
