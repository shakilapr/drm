# Record & Replay Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: `drm-record` — borrow-copy-release logging pipeline, recorder-owned bounded per-topic slab buffers, loss-ledger gap accounting, disk threads fully decoupled from DRM memory, `on_full` policies, MCAP framing with DRM metadata records, replay with explicit scheduler and tiered fidelity. Implements architecture.md §Memory Ownership (record lifecycle), §drm-record. Owner: E4 (crate `drm-record`). Phase 32. Depends on [cursor-delivery.md](../runtime/cursor-delivery.md) (borrow_next), [hazard-protocol.md](../runtime/hazard-protocol.md), [exhaustion-reclamation.md](../runtime/exhaustion-reclamation.md), [overflow-policies.md](../runtime/overflow-policies.md) (reject gating).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-record-borrow-copy-release` | ingress borrows, copies into recorder-owned buffer, releases immediately | `REQ-OPS-005` |
| `REQ-OPS-record-no-disk-on-borrow` | disk threads never touch DRM slots | `REQ-OPS-007` |
| `REQ-OPS-record-on-full` | drop / stop / backpressure policies; drops land in the loss ledger; recorder never an implicit required consumer | `REQ-OPS-008` |
| `REQ-OPS-record-loss-ledger` | per-topic preallocated gap ledger; coalesced ranges; `gap_detail_lost` on overflow | `REQ-OPS-018` |
| `REQ-OPS-record-topic-slabs` | per-topic slab pools sized to each topic's payload bound; byte-budget stays honest | `REQ-OPS-019` |
| `REQ-OPS-record-mcap` | MCAP framing + DRM metadata records (u64 seq, epoch, ingress order); gaps recorded | `REQ-OPS-006` |
| `REQ-OPS-replay-faithful` | tiered fidelity (payload / per-topic order / recorder-observed global order) + explicit replay scheduler | `REQ-OPS-009` |

## Data Structures

```rust
pub struct RecorderConfig {
 topics: SortedSet<TopicPath>, // sequential reader positions (cursor-delivery next())
 buffer_bytes: u64, // recorder-owned budget = Σ per-topic slab pools + descriptor ring + gap ledgers
 on_full: OnFull, // Drop | Stop | Backpressure
 chunk_target: u64, // MCAP chunk size
}
pub enum OnFull { Drop, Stop, Backpressure }

// Per-topic slab pool: slots sized to THAT topic's schema payload bound. Preallocated at session
// activation. A 128-byte IMU sample never pays an 8 MiB camera slot.
pub struct TopicSlab { slots: FixedFreeList<TopicSlot>, slot_bytes: usize } // slot_bytes = topic bound
pub struct TopicSlot { len: u32, payload: [u8; TOPIC_BOUND] } // TOPIC_BOUND = per-topic schema max

// Small fixed descriptor ring (global ingress order across topics)
pub struct DescEntry { topic_id: u32, slab_index: u32, len: u32, seq: u64,
 recv_mono_ns: u64, recv_wall_ns: u64, ingress_order: u64 }
// recv_mono_ns: local monotonic at copy completion — drives replay scheduling and global order.
// recv_wall_ns: wall-clock pair member — human indexing only. Source timestamps are never here;
// they remain topic-owned and travel verbatim inside the payload or the topic's declared fields.

// Loss ledger — per topic, tiny, fixed, SEPARATE exhaustion domain from payload slabs (I-RC5)
pub struct GapLedger { entries: FixedRing<PendingGap> }
pub struct PendingGap { valid: bool, first_seq: u64, last_seq: u64, reason: GapReason }
pub enum GapReason { HistoryLap, PolicyDrop, PublisherEpoch }
```

## Invariants

- **I-RC1 NoDiskOnBorrow**: a DRM borrow is held only for the copy into a per-topic slab slot; disk threads observe recorder memory exclusively. Test: `record_borrow_scope` (instrumented borrow-window assertion).
- **I-RC2 BufferBounded**: recorder memory never exceeds `buffer_bytes`; per-topic slabs make the accounting exact (8 MiB camera + 128 B IMU ⇒ IMU slots cost IMU-bound bytes). Test: `record_bounded` (mixed topic sizes, sustained overload).
- **I-RC3 PolicyExact**: on slab pool full — `Drop` discards the sample and coalesces its sequence into that topic's `GapLedger` (never into payload storage, never via disk I/O from ingress); `Stop` aborts with `RECORDING_FAILED reason: BUFFER_EXHAUSTED`; `Backpressure` pauses that topic's ingress (owned slot retained; no loss). The recorder is never an implicit required sequential consumer — publisher impact is governed by the topic's own overflow policy (Protocols). Test: `record_policies` (3 fixtures × topic-policy combinations).
- **I-RC4 ReplayFaithful**: three independent tiers — (a) payload bytes byte-identical; (b) per-topic causal order preserved as `(publisher_epoch, seq)`; (c) recorder-observed global order reproducible as `(recv_mono_ns, ingress_order)`. `recv_mono_ns` drives replay scheduling; `recv_wall_ns` is for indexing; source timestamps are replayed exactly as the original topic declared them, never rewritten. Test: `replay_roundtrip` (record → wipe → replay → byte/epoch/seq/order/timestamp compare).
- **I-RC5 GapAccounted**: every gapped or dropped sequence lands in its topic's `GapLedger` as a coalesced range, regardless of slab exhaustion; the ledger never shares an exhaustion domain with payload slabs; ledger overflow ⇒ conservative range merge + `gap_detail_lost = true`. Test: `record_gap_ledger` (slabs full + history lap + publisher-epoch crash sources; forced ledger overflow).
- **I-RC6 McapFidelity**: DRM's `(publisher_epoch, seq: u64, ingress_order)` is recorded in DRM metadata records; standard MCAP message fields remain interoperable (`publish_time = log_time` when no source time exists — the 32-bit MCAP `sequence` is advisory). Test: `mcap_drm_meta`, `mcap_interop`.

## Functions

```rust
// Ingress thread (per topic)
fn ingress_loop(rec: &Recorder, topic: TopicId) {
 loop {
  match rec.copy_next_into_free_slot(topic) { // borrow → copy into per-topic slab slot → release
  Ok(mut desc) => {
   desc.ingress_order = rec.global_order.fetch_add(1, Relaxed); // recorder-observed global order
   loop {
   match rec.descriptors.try_push(desc) {
    Ok(()) => break,
    Err(Full) => match rec.cfg.on_full { // descriptor ring full ⇒ policy (I-RC3)
     Drop => { rec.ledger(topic).coalesce(desc.seq, PolicyDrop); rec.slab(topic).free(desc); break; }
     Stop => { rec.abort(RECORDING_FAILED, BUFFER_EXHAUSTED); return; }
     Backpressure => { rec.wait_desc_space(topic); continue; } // slot retained; no loss (I-RC1: borrow already released)
    },
   }}
  },
  Err(Gap { missed }) => rec.ledger(topic).coalesce_range(missed, HistoryLap), // tiny ledger, not slabs (I-RC5)
  Err(Empty) => wait_for_sequence(),
  } // DRM borrow was released before try_push/wait/disk work (I-RC1)
 }
}

// Epoch observation (cursor-delivery I-C6 discipline): publisher restart between reads ⇒
// ledger.coalesce_range(since_last_recorded, PublisherEpoch); recording continues (or stops per policy).

// Disk thread: drains descriptors + ledgers; exclusively recorder memory (I-RC1); no DRM handle exists here
fn disk_thread(rec: &Recorder) {
 while let Some(batch) = rec.descriptors.take_batch(rec.cfg.chunk_target) {
  for d in batch {
   mcap::write_message(d, rec.slab(d.topic).payload(d))?; // standard MCAP message + DRM metadata record
   rec.ledger(d.topic).drain_into(&mut mcap); // DrmGap records, seq-ordered (I-RC5)
   rec.slab(d.topic).free(d);
  }
 }
}
```

## Protocols

*Record session:*

1. `drm record --topics a,b --on-full drop --out session/` — opens sequential readers (cursor next), preallocates per-topic slabs, descriptor ring, and gap ledgers; spawns ingress + disk threads.
2. Ingress: borrow → copy → release (single atomic window per sample); ingress order assigned at copy completion.
3. Disk: MCAP chunks; DRM metadata records per message; `DrmGap` records drained from ledgers — exact coalesced `{topic, publisher_epoch, first_missing_seq, missed, reason}` markers.
4. Stop: flush chunks; footer; session summary (counts, gaps, drops, `gap_detail_lost`).

*Backpressure semantics (recorder is never an implicit required consumer):*

| Topic policy | Recorder declared required sequential? | Effect when recorder pauses |
|---|---|---|
| `overwrite` | n/a | none — ring rolls; recorder later observes `Gap` (ledger: `HistoryLap`) |
| `reject` | no | none — gating applies only to declared required consumers (overflow-policies.md); recorder later observes `Gap` |
| `reject` | yes (declared) | `publish()` returns `QueueFull` per overflow-policies.md |

`drm verify` (`V-REC`) rejects `on_full = backpressure` on a `reject` topic without a declared required recorder.

*Replay session:*

1. Read MCAP; reconstruct per-topic streams from DRM metadata records ordered by `(publisher_epoch, seq)`; insert gap boundaries from `DrmGap` records.
2. Scheduler modes: `realtime` (scale 1.0) | `rate(x)` | `as_fast` (no timing waits). Target time: `replay_start + (recv_mono_ns − record_start_mono_ns) / rate`; ties in `recv_mono_ns` broken by `ingress_order`.
3. Republish under a replay deployment namespace (deployment-identity.md); source timestamps replayed exactly as recorded; receive time = replay time.
4. Consumers observe tiered fidelity (I-RC4): payload bytes, per-topic order, and recorder-observed global order.

## Design Confirmation (M14W3)

Recorder pipeline reviewed against architecture §Memory Ownership (record lifecycle),
§drm-record and this doc's I-RC1..I-RC6; implementation plan confirmed, no invariant changed.

- [x] Borrow-copy-release: ingress holds a DRM borrow only for the copy into a per-topic slab slot; disk threads observe recorder memory exclusively (I-RC1, `REQ-OPS-005`/`REQ-OPS-007`).
- [x] Bounded buffer: recorder memory never exceeds `buffer_bytes`; per-topic slabs make accounting exact — a 128-byte IMU sample never pays an 8 MiB camera slot (I-RC2, `REQ-OPS-019`).
- [x] `on_full` policies exact: `Drop` coalesces the sequence into the topic `GapLedger`; `Stop` aborts `RECORDING_FAILED reason: BUFFER_EXHAUSTED`; `Backpressure` pauses ingress with the slot retained (I-RC3, `REQ-OPS-008`). The recorder is never an implicit required sequential consumer.
- [x] Gap ledger is a separate exhaustion domain from payload slabs; overflow ⇒ conservative range merge + `gap_detail_lost = true` (I-RC5, `REQ-OPS-018`).
- [x] MCAP chunking (`chunk_target`) with DRM metadata records carrying full u64 `(publisher_epoch, seq)` + `ingress_order`; standard MCAP fields stay interoperable (I-RC6, `REQ-OPS-006`).
- [x] Replay under a replay deployment namespace ([deployment-identity.md](../deployment-identity.md)); source timestamps replayed exactly, receive time = replay time; three fidelity tiers + explicit scheduler (I-RC4, `REQ-OPS-009`).

Deltas: none — the doc already states these; this section records the review. The
implementation lands in Phase 32 (M15W1).

## Implementation Status (M16W2)

Record gate evidence re-run: `record_borrow_scope` (I-RC1 borrow window),
`record_bounded` + `record_slab_full_boundary_exact` (I-RC2 exact budget, no
off-by-one at the slab boundary), `record_policies` + `record_slow_disk_policy`
(I-RC3 exact policy behavior at the descriptor/slab boundary), and
`record_mcap_drm_meta` (I-RC6 DRM metadata + third-party MCAP parse).

| Element | Location | Test |
|---|---|---|
| slab/decriptor boundary exactness | `crates/drm-record/src/lib.rs` | `record_slab_full_boundary_exact` |
| slow-disk policy boundary | `crates/drm-record/src/tests.rs` | `record_slow_disk_policy` |
| MCAP DRM metadata interop | `crates/drm-record/src/mcap.rs` | `record_mcap_drm_meta` |

REQ: REQ-OPS-005, REQ-OPS-006, REQ-OPS-007, REQ-OPS-008, REQ-OPS-019.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Disk slower than ingress (sustained) | slabs fill → policy (I-RC3); gap metadata still lands in the ledger (I-RC5) |
| Publisher crash mid-record | epoch observation → ledger range (`PublisherEpoch`); recording continues (drop/backpressure) or stops per policy |
| Small topic on mixed-schema deployment | per-topic slabs sized to that topic's bound; no cross-topic amplification (I-RC2) |
| Gap ledger overflow | conservative range merge + `gap_detail_lost = true` (I-RC5) |
| Timestamp clock source | `recv_mono_ns` + `recv_wall_ns` pair stored per sample; source timestamps untouched (I-RC4) |
| Backpressure + `reject` topic | publisher sees `QueueFull` only if the recorder is a declared required consumer; otherwise gaps (`V-REC` at build) |
| Sequence crosses 2³² | DRM metadata records carry full u64 `(publisher_epoch, seq)`; MCAP 32-bit `sequence` advisory (I-RC6) |
| Equal `recv_mono_ns` across topics | `ingress_order` breaks ties in global replay order (I-RC4c) |
| MCAP chunk corruption (disk full) | `RECORDING_FAILED reason: IO_ERROR`; no silent truncation |

## Test Matrix

| Case | Expected |
|---|---|
| `record_borrow_scope` | I-RC1 (0 borrows held during disk writes) |
| `record_bounded` (8 MiB camera + 128 B IMU mix, sustained overload) | I-RC2 exact per-topic accounting |
| `record_policies` ×3 × topic-policy combinations | I-RC3 exact messages; no disk I/O from ingress |
| `record_gap_ledger` (slab-full + history-lap + epoch-crash sources; ledger overflow) | I-RC5 coalesced ranges; `gap_detail_lost` |
| `replay_roundtrip` (payload, epoch, u64 seq across 2³², gaps, both time domains, tie order) | I-RC4 all tiers |
| `mcap_drm_meta` / `mcap_interop` | I-RC6; third-party reader parses |
| replay scheduler modes (`realtime`/`rate(x)`/`as_fast`) | temporal replay matches mode definition |
