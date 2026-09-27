# Record & Replay Algorithms

Scope: `drm-record` — borrow-copy-release logging pipeline, recorder-owned bounded buffers, disk threads fully decoupled from DRM memory, `on_full` policies, MCAP framing, replay with order/timestamp/gap preservation. Implements architecture.md §Memory Ownership (record lifecycle), §drm-record. Owner: E4 (crate `drm-record`). Phase 32. Depends on [cursor-delivery.md](../runtime/cursor-delivery.md) (borrow_next), [hazard-protocol.md](../runtime/hazard-protocol.md), [exhaustion-reclamation.md](../runtime/exhaustion-reclamation.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-OPS-record-borrow-copy-release` | ingress borrows, copies into recorder-owned buffer, releases immediately | `REQ-OPS-005` |
| `REQ-OPS-record-no-disk-on-borrow` | disk threads never touch DRM slots | `REQ-OPS-007` |
| `REQ-OPS-record-on-full` | drop / stop / backpressure policies | `REQ-OPS-008` |
| `REQ-OPS-record-mcap` | MCAP framing; gaps recorded | `REQ-OPS-006` |
| `REQ-OPS-replay-faithful` | replay preserves samples/order/timestamps/gaps | `REQ-OPS-009` |

## Data Structures

```rust
pub struct RecorderConfig {
 topics: SortedSet<TopicPath>, // sequential reader positions (cursor-delivery next())
 buffer_bytes: u64, // recorder-owned bounded buffer
 on_full: OnFull, // Drop | Stop | Backpressure
 chunk_target: u64, // MCAP chunk size
}
pub enum OnFull { Drop, Stop, Backpressure }

pub struct RecorderBuffer { slots: FixedRing<OwnedSampleSlot>, free: FixedFreeList } // preallocated at session activation
pub struct OwnedSampleSlot { topic_id: u32, seq: u64, recv_ns: u64, src_ns: Option<u64>, len: u32, payload: [u8; MAX_SAMPLE] }
```

## Invariants

- **I-RC1 NoDiskOnBorrow**: a DRM borrow is held only for the copy into `RecorderBuffer`; disk threads observe recorder memory exclusively. Test: `record_borrow_scope` (instrumented borrow-window assertion).
- **I-RC2 BufferBounded**: recorder memory never exceeds `buffer_bytes`. Test: `record_bounded`.
- **I-RC3 PolicyExact**: on full buffer — `Drop` discards + gap counts; `Stop` aborts with `RECORDING_FAILED reason: BUFFER_EXHAUSTED`; `Backpressure` pauses ingress (publisher then governed by topic overflow policy — never hazard-pinned). Test: `record_policies` (3 fixtures).
- **I-RC4 ReplayFaithful**: replay restores samples, order, both timestamps, and gap boundaries exactly. Test: `replay_roundtrip` (record → wipe → replay → byte/sequence compare).

## Functions

```rust
// Ingress thread
fn ingress_loop(rec: &Recorder, topic: TopicId) {
 loop {
 match rec.copy_next_into_free_slot(topic) { // borrow → copy into preallocated slot → release
 Ok(slot) => {
 let mut pending = slot;
 loop {
 match rec.buffer.try_push(pending) {
 Ok(()) => break,
 Err(Full(slot)) => {
 pending = slot;
 match rec.cfg.on_full {
 Drop => { rec.write_policy_gap(topic, pending.seq, 1); rec.release_slot(pending); break; }
 Stop => { rec.abort(RECORDING_FAILED, BUFFER_EXHAUSTED); return; }
 Backpressure => { rec.wait_for_space(); continue; } // owned slot retained; no sample loss
 }
 },
 }
 },
 },
 Err(Gap { missed }) => rec.record_gap_marker(missed), // MCAP gap record
 Err(Empty) => wait_for_sequence(),
 }
 // DRM borrow was released before try_push/wait/disk work (I-RC1)
 }
}

// Disk threads: chunking, compression, MCAP write — exclusively from RecorderBuffer (I-RC1)
fn disk_thread(rec: &Recorder) {
 while let Some(chunk) = rec.buffer.take_chunk(rec.cfg.chunk_target) {
 mcap::write_chunk(chunk)?; // no DRM handle exists on this thread
 }
}
```

## Protocols

*Record session:*

1. `drm record --topics a,b --on-full drop --out session/` — opens sequential readers (cursor next), spawns ingress + disk threads.
2. Ingress: borrow → copy → release (single atomic window per sample).
3. Disk: MCAP chunks; exact `{topic, first_seq, missed}` markers for history gaps and policy drops.
4. Stop: flush chunks; footer; explicit session summary (counts, gaps, drops).

*Replay session:*

1. Read MCAP; reconstruct per-topic sequence streams; insert gap boundaries.
2. Republish under a replay deployment namespace (deployment-identity.md) with recorded timestamps as source timestamps; receive time = replay time.
3. Consumers observe faithful order and gaps (I-RC4).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Disk slower than ingress (sustained) | buffer fills → policy (I-RC3); never extends borrows |
| Publisher crash mid-record | epoch observation → gap marker; recording continues (drop/backpressure) or stops per policy |
| Clock source for recv_ns | monotonic + wall pair; replay uses recorded source timestamps |
| Backpressure + `reject` topic | publisher sees `QueueFull` (overflow-policies.md) — correct chain, no hazard pin |
| MCAP chunk corruption (disk full) | `RECORDING_FAILED reason: IO_ERROR`; no silent truncation |

## Test Matrix

| Case | Expected |
|---|---|
| `record_borrow_scope` | I-RC1 (0 borrows held during disk writes) |
| `record_bounded` (sustained overload) | I-RC2 |
| `record_policies` ×3 | I-RC3 exact messages |
| `replay_roundtrip` | I-RC4 |
| MCAP reader interop | third-party reader parses |
