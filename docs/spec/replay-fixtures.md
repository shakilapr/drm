# Replay Round-Trip Fixture Spec

Non-normative component spec (codebase.md §2.4). Normative source:
[record-replay.md](../algorithms/ops/record-replay.md) §Invariants I-RC4/I-RC5/I-RC6,
§Protocols replay session, §Test Matrix `replay_roundtrip`. This spec pins the
round-trip fixture the `drm-record` suite executes; architecture wins conflicts.

Scope: a deterministic record → wipe → replay fixture with injected gaps that
proves tiered replay fidelity (`I-RC4`) end to end, using only recorded bytes
after the wipe. Owner: E2 (fixtures) + E4 (publisher). Phase 32. Depends on
[deployment-identity.md](../algorithms/deployment-identity.md) (replay
namespace), [serial-mcu.md](../algorithms/router/serial-mcu.md) (MCU interop,
E3).

## REQ Bindings

| Slug | Anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-record-mcap` | MCAP framing + DRM metadata records (u64 seq, epoch, ingress order); gaps recorded | `REQ-OPS-006` |
| `REQ-OPS-replay-faithful` | tiered fidelity + explicit replay scheduler | `REQ-OPS-009` |
| `REQ-OPS-record-loss-ledger` | per-topic gap ledger; coalesced ranges | `REQ-OPS-018` |
| `REQ-CORE-namespace-scoping` | deployment-scoped opaque names; replay isolation | `REQ-CORE-088` |

## Fixture shape

One helper, `record_known_session` (`crates/drm-record/src/tests.rs`), records a
three-topic session through the real ingress pipeline and MCAP disk stage, then
returns the MCAP bytes and the session summary.

| Topic | Channel | Slabs | Injected defect | Expected record |
|---|---|---|---|---|
| 1 | depth 8 | 8 slots | none | 4 samples, seq `0..=3`, payloads `0x10..=0x13` |
| 2 | depth 1 | 8 slots | history lap (overwrite) | 2 samples, seq `{0,4}`; `HistoryLap` gap `1..=3` |
| 3 | depth 8 | 1 slot | `Drop` policy (slow drain) | 1 sample, seq `0`; `PolicyDrop` gap `1..=2` |

`recv_mono_ns` is assigned monotonically across every ingress step so the
recorder-observed global order equals the record order; the MCAP writer uses it
as `log_time` (source time when no separate source time exists, I-RC6).

## Steps

1. **Record**: publish/interleave the table above through `Recorder::ingress_step`
   with `OnFull::Drop`; drain with `disk::Stage` at `Compression::None`.
2. **Wipe**: drop the recorder, channels, and stage; keep only the MCAP bytes.
3. **Read**: `Session::decode(&bytes)` reconstructs channels, per-topic streams
   ordered by `(publisher_epoch, seq)` (I-RC4b), and gap boundaries from the
   `drm.gap` markers (I-RC5).
4. **Replay**: derive a `ReplayNamespace` from a base `DeploymentId` and a run
   discriminator, open a `ReplayPublisher`, and run `replay(session, publisher,
   mode)`; consumers read each topic channel back.
5. **Compare**: payload bytes (I-RC4a), per-topic order (I-RC4b),
   recorder-observed global order `(recv_mono_ns, ingress_order)` (I-RC4c),
   source timestamps verbatim, and the gap boundaries.

## Fidelity criteria (pre-stated)

| Tier | Criterion | Evidence |
|---|---|---|
| a | payload bytes byte-identical per topic | `replay_publisher_faithful` read-back |
| b | per-topic `(publisher_epoch, seq)` order preserved | `topic_stream`, `replay_reader_epoch` |
| c | recorder-observed `(recv_mono_ns, ingress_order)` reproducible | `global_order` monotone key |
| gaps | every dropped/lapped range present as a coalesced boundary | `session.gaps()` reasons/counts |
| time | source timestamps replayed exactly; replay receive = schedule | `replay_summary_metrics`, `ReplayStep.source_time` |
| namespace | replay never adopts the live deployment namespace | `replay_namespace_isolated` |

## Scheduler matrix

| Mode | Target offset | Evidence |
|---|---|---|
| `realtime` | recorded `recv_mono_ns` delta | `replay_scheduler_modes` |
| `rate(x)` (`num/den`) | delta scaled by `den/num` | `rate(2/1)` halves the wall duration |
| `as_fast` | `0` (no waits) | duration `0`, order unchanged |

`recv_mono_ns` ties are broken by `ingress_order`; mode changes timing, never
order.

## Test anchors

`cargo test -p drm-record replay` runs `replay_roundtrip`,
`replay_publisher_faithful`, `replay_scheduler_modes`,
`replay_namespace_isolated`, `replay_reader_epoch`, `replay_summary_metrics`.
