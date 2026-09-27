# Cursor & Delivery Algorithms

Scope: reader cursors, the four access operations (`borrow_latest`, `copy_latest`, `borrow_next`, `copy_next`), independent reader positions, exact `GAP` reporting on lapped sequential readers, and epoch verification around every acquisition. Implements architecture.md §Reader Position, §Memory Ownership, §Orthogonal Sample Access, §Publisher Epoch (first_valid_seq window). Owner: E1 (crate `drm-core::channel`). Phase 11.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-cursor-latest` | `latest()` — newest retained sequence, no backlog | `REQ-CORE-012` |
| `REQ-CORE-cursor-next` | `next()` — advances to next unread in ring | `REQ-CORE-013` |
| `REQ-CORE-gap-exact` | lapped reader ⇒ `GAP { missed: N }` exact | `REQ-CORE-028` |
| `REQ-CORE-gap-resume-oldest` | after `Gap`, resume at oldest retained — never skip retained samples | `REQ-CORE-029` |
| `REQ-CORE-cursor-independence` | readers at different speeds never affect each other | `REQ-CORE-011` |
| `REQ-CORE-borrow-copy` | borrow vs copy is an API choice per call | `REQ-CORE-003` |
| `REQ-CORE-cursor-epoch-verified` | epoch re-read after acquisition; restart invalidates the read | `REQ-CORE-010` |

## Data Structures

```rust
// Reader cursor — LOCAL to each consumer process (never shared, never in SHM)
pub struct ReaderCursor { last_read: Option<u64> } // required reattach seeds this from ack_seq - 1

pub enum ReadOutcome<B> {
 Sample(B), // borrowed or copied payload view
 Gap { missed: u64 }, // exact count of sequences aged out since last_read
 Empty, // nothing published yet / fully caught up
}

pub enum ReadPos { Latest, Next }
pub enum MemOwn { Borrow, Copy }
// Signals from acquisition (hazard-protocol.md): Ok(Payload) | Err(AgedOut) | Err(Retry) | other Status.
// Native Copy receives caller-owned `&mut [u8]`; language bindings may allocate before entering core.
```

## Invariants

- **I-C1 Independent**: readers at different speeds never affect each other's cursor (cursors are process-local; no shared reader state). Test: `cursor_independence`.
- **I-C2 GapExact**: `next()` when overrun by k ⇒ `Gap { missed: k }` where k = number of sequences retired between `last_read` and the oldest retained sequence. Test: `gap_exact` (fixtures k ∈ 1..=32), `gap_exact_concurrent` (publisher advancing during lookup).
- **I-C3 LatestNewest**: `latest()` resolves the newest committed sequence (or `Empty`). Test: `latest_newest`.
- **I-C4 NextNoSkip**: every committed sequence is delivered exactly once per consumer unless it was already aged out when the cursor reached it (then one `Gap`). After a `Gap`, the cursor resumes at the oldest retained sequence — retained samples are never discarded. Test: `next_no_skip` (property), `next_resume_oldest`.
- **I-C5 BorrowSemantics**: borrow returns transient view guarded by hazard slot (hazard-protocol.md); copy returns owned bytes and releases immediately. Test: `borrow_copy_lifecycle`.
- **I-C6 EpochAtomic**: the cursor re-reads the publisher epoch after acquisition; a restart that crosses the read discards the acquired payload, resets the cursor, and retries. Test: `cursor_epoch_restart`.
- **I-C7 AckAfterCompletion**: on a `reject` topic, a required sequential consumer publishes `ack_seq` only after the sample is fully consumed (copy done, or borrow released) — never before. Test: `ack_after_completion`.
- **I-C8 SnapshotLinearization**: `Gap` and `Empty` are relative to one epoch-stable `write_seq` snapshot; a crossing restart causes retry. Test: `cursor_snapshot_restart`.
- **I-C9 CoreCopyNoAlloc**: copy operations write into caller-owned capacity and report required length without allocating. Test: `copy_zero_alloc`.

## Functions

```rust
fn oldest_retained(ch: &Channel, ws: u64) -> u64 {
 // Addressable window: last R sequences, never earlier than the current run's watermark.
 ws.saturating_sub(ch.depth() as u64).max(ch.first_valid_seq().load(Acquire))
}

fn next<B>(&mut self, ch: &Channel, own: MemOwn) -> Result<ReadOutcome<B>, Status> {
 loop {
 let epoch0 = ch.epoch().load(Acquire);
 let ws = ch.write_seq().load(Acquire);
 let fv = ch.first_valid_seq().load(Acquire);
 if ch.epoch().load(Acquire) != epoch0 { self.reset(); continue; }

 if ws <= fv { return Ok(Empty); }

 let oldest = ws.saturating_sub(ch.depth() as u64).max(fv);
 let wanted = match self.last_read {
 None => oldest, // fresh reader starts at oldest retained
 Some(last) => last.checked_add(1).ok_or(Status::InvalidData)?,
 };

 if wanted < oldest { // lapped: [wanted, oldest) aged out
 let missed = oldest - wanted; // I-C2: exact
 if ch.epoch().load(Acquire) != epoch0 { self.reset(); continue; }
 self.last_read = Some(oldest - 1); // I-C4: resume at oldest next call
 ch.overrun_count.fetch_add(missed, Relaxed); // aggregate observed gaps; diagnostics only
 return Ok(Gap { missed });
 }
 if wanted >= ws {
  if ch.epoch().load(Acquire) != epoch0 { self.reset(); continue; }
  return Ok(Empty);
 }

 match ch.try_acquire(wanted, own) { // hazard-protocol.md (borrow or copy)
 Ok(payload) => {
 if ch.epoch().load(Acquire) != epoch0 { // I-C6: restart crossed the read
 drop(payload);
 self.reset();
 continue;
 }
 self.last_read = Some(wanted);
 // I-C7: required consumer acks after completion — Copy now; Borrow on guard drop.
 if own == MemOwn::Copy && ch.is_required_consumer(self.consumer_id()) {
  ch.ack_completed(wanted)?;
 }
 if own == MemOwn::Borrow && ch.is_required_consumer(self.consumer_id()) {
  payload.arm_ack_on_drop(wanted);
 }
 return Ok(Sample(payload));
 }
 Err(AgedOut | Retry) => continue, // publisher advanced; recompute snapshot
 Err(e) => return Err(e),
 }
 }
}

fn latest<B>(&mut self, ch: &Channel, own: MemOwn) -> Result<ReadOutcome<B>, Status> {
 loop {
 let epoch0 = ch.epoch().load(Acquire);
 let (s, e) = match ch.newest() {
  Ok(Some(pair)) => pair,
  Ok(None) => {
   if ch.epoch().load(Acquire) != epoch0 { self.reset(); continue; }
   return Ok(Empty);
  }
  Err(Retry) => continue,
 };
 match ch.try_acquire_entry(s, e, own) {
 Ok(payload) => {
 if ch.epoch().load(Acquire) != epoch0 { drop(payload); self.reset(); continue; }
 Ok(Sample(payload))
 }
 Err(Retry) => continue, // rolled during acquisition
 Err(e) => return Err(e),
 }
 }
}
```

## Protocols

*Borrow scope (consumer, ties to hazard-protocol.md):*

1. `try_acquire` runs the hazard acquisition protocol → transient view (or immediate copy).
2. View valid only inside scope; drop ⇒ hazard cleared (`protected[me] = NONE`).
3. External I/O must `copy` first (architecture borrow-lifecycle rule; enforced by binding design + docs).

*Required-consumer acknowledgement (overflow-policies.md):* if this consumer is a required sequential
consumer, `next()` publishes the shared `ack_seq` watermark after the read completes — for `Copy`
immediately after the copy, for `Borrow` only after the hazard is released. The publisher therefore
never retires a sequence a required consumer has not finished with. This is separate from the local
cursor and the hazard slot. The borrow guard contains the acknowledgement record and performs hazard
release before monotonic acknowledgement on `Drop`. Acquisition creates the guard with
acknowledgement disarmed; the cursor arms it only after the post-acquisition epoch check accepts the
sample. Retry, restart discard, or failed copy therefore cannot acknowledge delivery.

*Epoch-crossed read (I-C6):* acquire → re-read epoch → mismatch ⇒ drop payload, `reset()`, retry. This makes restart handling intrinsic to the read path rather than a separate poll.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Fresh consumer, empty channel | `Empty` |
| Fresh non-required consumer, non-empty channel (`next`) | returns oldest retained |
| Reattached required consumer | resumes at persisted `ack_seq` |
| Fully caught-up (`next`) | `Empty` (not Gap — nothing aged out) |
| Lapped (`next`) | `Gap { missed: k }`, then next call continues at oldest retained |
| `ws < R` (fewer published than depth) | `oldest = 0`; no unsigned underflow |
| Publisher advances between `ws` load and acquisition | `Err(AgedOut)` ⇒ loop; recompute `ws`/`oldest` |
| Restart between epoch check and acquisition | epoch re-read differs ⇒ drop + reset + retry |
| `last_read` at `u64::MAX` | `checked_add` ⇒ `Status::InvalidData` (practically unreachable) |
| `latest` during publish commit | ring I3 guarantees newest-or-newer; no torn view |

## Test Matrix

| Case | Expected |
|---|---|
| `gap_exact` fixtures lap 1..32 | missed == lap count exactly |
| `gap_exact_concurrent` (publisher advancing during lookup) | exact missed under racing advance |
| `next_resume_oldest` | retained samples delivered after a Gap |
| `cursor_independence` (fast/slow/lapped concurrent) | no cross-influence |
| `latest_newest` | newest committed |
| `next_no_skip` property | exactly-once unless aged-out |
| `borrow_copy_lifecycle` | slot freed on scope exit; copy never pins |
| `cursor_epoch_restart` | pre-restart payload discarded; cursor reset |
| `ack_after_completion` (copy vs borrow release) | I-C7: ack only after completion |
| fresh-next | oldest retained delivered |
| `cursor_snapshot_restart` | I-C8 |
| `copy_zero_alloc` | caller buffer only; I-C9 |
