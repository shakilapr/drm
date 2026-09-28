# Channel Publish Algorithms

Scope: one authoritative publish transaction joining validation, overflow gating, slot selection,
reclamation, payload write, generation advance, ring commit, and diagnostics — in two forms sharing
one ordering: copy publish (`src` slice) and loan publish (`write_acquire`/`commit`/`abort`, the
architecture's bulk direct-write path). Implements architecture.md §Publisher Slot Selection,
§History Overflow, §Zero Data-Path Allocation, §Bulk Data (direct write into a pre-allocated slot).
Owner: E1 (`drm-core::channel`). Depends on all runtime storage protocols.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-publish-transaction` | publisher slot selection and commit | `REQ-CORE-052` |
| `REQ-CORE-publish-linearization` | release commit makes one sample visible | `REQ-CORE-051` |
| `REQ-CORE-publish-error-effects` | failed publish never changes visible history | `REQ-CORE-050` |
| `REQ-CORE-publish-zero-alloc` | steady-state data path reuses fixed storage | `REQ-CORE-053` |
| `REQ-CORE-publish-counter-bounds` | sequence and generation never wrap | `REQ-CORE-049` |
| `REQ-CORE-publish-loan-form` | loan acquire/commit/abort; abort and failed commit invisible | `REQ-CORE-052` |

## Data Structures

```rust
pub struct PublishScratch {
 referenced: FixedBitSet, // capacity = pool_size; allocated at activation
}

pub struct PublishGuard<'a> {
 busy: &'a AtomicBool,
 scratch: &'a mut FixedBitSet, // exclusive because `busy` is held
} // Drop release-stores `busy = false`

pub struct WriteLoan<'a> { // returned by write_acquire; consumed by commit/abort
 guard: PublishGuard<'a>,
 slot: usize,
 seq: u64,
 next: u64,
 generation: u64, // reserved (current + 1); stored only at commit
 capacity: usize, // acquired sample bound <= codec sample_max (canon-codec two-region)
} // `bytes()` = the writable canonical sample buffer at the slot; the caller writes fixed fields
  // and variable bodies through generated codec accessors (no fixed-offset length header)
```

## Invariants

- **I-PB1 AtomicVisibility**: readers observe the old history or the complete new sample, never a
  partial payload or handle. Test: `publish_visibility_loom`.
- **I-PB2 ErrorNoCommit**: `InvalidArg`, `Busy`, `QueueFull`, `PoolExhausted`, counter exhaustion,
  a loan `write_abort`, and a failed `write_commit` leave `write_seq`, ring entries, and the current
  sample unchanged. Test: `publish_error_no_commit` (incl. loan abort/fail paths).
- **I-PB3 FixedWork**: publish allocates zero bytes and performs loops bounded by declared ring,
  pool, and consumer counts — both forms. Test: `publish_zero_alloc`.
- **I-PB4 ErrorPrecedence**: validation precedes authority and capacity decisions; the ordering below
  is stable ABI behavior — identical for copy and loan forms. Test: `publish_error_precedence`.
- **I-PB5 NoWrap**: sequence and generation exhaustion return explicit status before payload write.
  Test: `publish_counter_exhaustion`.

## Functions

```rust
fn publish(ch: &Channel, src: &[u8]) -> Result<u64, Status> {
 validate_payload_shape(ch, src)?;                 // InvalidArg; no shared write
 let mut guard = ch.try_publish_guard()?;          // Busy; non-blocking
 let seq = ch.write_seq.load(Acquire);
 let next = seq.checked_add(1).ok_or(SequenceExhausted)?;
 overflow::allow_commit(ch, seq)?;                 // QueueFull / InvalidConfig

 let Some(slot) = hazard::select_slot(ch, &mut guard.scratch) else {
  reclaim::publish_exhausted(ch); // counters + RT flag / opportunistic scan; see exhaustion-reclamation.md
  return Err(PoolExhausted);        // incoming sample is always dropped
 };
 let generation = ch.slot_generation(slot)
  .load(Relaxed).checked_add(1).ok_or(GenerationExhausted)?;
 let ring_stamp = ch.ring.preflight_commit(seq)?;   // RingStampExhausted

 pool::write_checked(ch, slot, src)?;               // fixed destination; no allocation
 ch.slot_generation(slot).store(generation, Relaxed);
 ring::commit(ch, seq, slot, generation, ring_stamp); // prepare stable cell
 ch.write_seq.store(next, Release);                 // visibility linearization point
 drop(guard);
 Ok(seq)
}

// ---- Loan form: the publisher writes directly into the shared slot (bulk topics) ----
// Acquire = steps 1–5 of `publish` MINUS payload write and ring preflight.
fn write_acquire(ch: &Channel, requested: usize) -> Result<WriteLoan<'_>, Status> {
 if requested > ch.sample_max() { return Err(InvalidArg); }        // I-PB4: before guard (codec sample bound)
 let mut guard = ch.try_publish_guard()?;                          // Busy; non-blocking
 let seq = ch.write_seq.load(Acquire);
 let next = seq.checked_add(1).ok_or(SequenceExhausted)?;
 overflow::allow_commit(ch, seq)?;                                 // QueueFull / InvalidConfig
 let Some(slot) = hazard::select_slot(ch, &mut guard.scratch) else {
  reclaim::publish_exhausted(ch); // counters + RT flag / opportunistic scan (exhaustion-reclamation.md)
  return Err(PoolExhausted);
 };
 let generation = ch.slot_generation(slot)
  .load(Relaxed).checked_add(1).ok_or(GenerationExhausted)?;
 // Fixed-size topics acquire the exact sample size; variable topics acquire the request (I-A10).
 let capacity = if ch.is_fixed_size() { ch.fixed_len() } else { requested };
 Ok(WriteLoan { guard, slot, seq, next, generation, capacity })
}

// Commit = the ONLY step that touches ring/generation/write_seq. Ring preflight lives here:
// a failed stamp check discards only this loan's payload, never visible history.
fn write_commit(ch: &Channel, loan: WriteLoan<'_>, sample_len: usize) -> Result<u64, Status> {
 if sample_len > loan.capacity { return Err(InvalidArg); }         // never committed (I-PB2)
 if ch.is_fixed_size() && sample_len != ch.fixed_len() { return Err(InvalidArg); }
 // The canonical header (fixed fields + descriptor block) and tail bodies were already written
 // into the loan buffer through the generated codec (canon-codec.md two-region). Commit authors
 // no header, but the raw ABI path can bypass the encoder — validate the written sample before it
 // becomes visible (canon-codec I5: value or error, never UB); failure ⇒ nothing committed.
 canon::validate_sample(&loan.bytes()[..sample_len])?;
 let ring_stamp = ch.ring.preflight_commit(loan.seq)?;             // RingStampExhausted
 ch.slot_generation(loan.slot).store(loan.generation, Relaxed);
 ring::commit(ch, loan.seq, loan.slot, loan.generation, ring_stamp);
 ch.write_seq.store(loan.next, Release);                           // linearization point
 drop(loan.guard);
 Ok(loan.seq)
}

// Abort = guard drop only. The slot holds junk no sequence references; the next
// select_slot may reuse it. Crash before commit is the same state (I-L7 restart order).
fn write_abort(ch: &Channel, loan: WriteLoan<'_>) { drop(loan.guard); }
```

`PoolExhausted` increments once for the dropped incoming sample. `publish_exhausted`
(exhaustion-reclamation.md) increments both counters and either sets `reclaim_requested` (hard-RT
profile: zero inspection syscalls on this thread, I-E7) or runs one bounded maintenance pass
(general-purpose). It never retries that sample; the next publish sees freed capacity immediately. No
payload byte is written before a slot and both counters are known valid.

## Protocols

1. Validate the encoded sample against the codec structural bound (canon-codec.md I3; no fixed-offset header assumption).
2. Acquire `busy` with one compare-exchange; failure returns `Busy`.
3. Check sequence capacity and `reject` retirement gates.
4. Build the referenced-slot bitmap in activation-owned scratch and scan hazards with SeqCst loads.
5. On no slot, call `reclaim::publish_exhausted` — increments both counters, then sets `reclaim_requested` (hard-RT, I-E7) or runs one bounded maintenance pass (general-purpose) — and drop this sample.
6. Check generation capacity, then write the canonical sample (codec-authored fixed header + bounded tail).
7. Publish the versioned ring cell, then release-store `write_seq`; the latter linearizes visibility.
8. Release `busy` on every exit.

*Loan form (bulk direct-write):* steps 1–5 run in `write_acquire` (sample write and ring preflight
deferred); the caller writes fixed fields and variable bodies into the acquired slot through generated
codec accessors (canon-codec.md two-region); `write_commit` validates the final `sample_len`, runs ring
preflight, generation store, ring commit, and `write_seq` release (steps 6–7); `write_abort` runs step
8 only. Both forms share one ordering, so I-PB4 precedence and the crash states are identical: an
in-progress loan slot is unreachable until commit, exactly like the copy form's not-yet-committed write.

## Edge Cases & Errors

| Order | Case | Status | Visible side effect |
|---:|---|---|---|
| 1 | invalid sample shape (or `requested`/`sample_len` beyond structural max) | `InvalidArg` | none |
| 2 | another publish active (either form) | `Busy` | none |
| 3 | sequence cannot advance | `SequenceExhausted` | none |
| 4 | required consumer gates retirement | `QueueFull` | none |
| 5 | no reusable slot; `publish_exhausted` after the drop decision (RT: flag only) | `PoolExhausted` | both drop counters +1 only |
| 6 | selected slot generation/ring stamp exhausted | counter-exhaustion status | none |
| 7 | loan commit with malformed canonical sample | `InvalidData` | none (I-PB2) |
| 8 | loan `write_abort` or failed `write_commit` | per case | none (I-PB2) |
| 9 | success | — | one committed sample |

## Test Matrix

| Case | Expected |
|---|---|
| `publish_visibility_loom` | I-PB1 |
| `publish_error_no_commit` | I-PB2 for every status, both forms (incl. loan abort/commit-fail) |
| `publish_zero_alloc` | zero allocations; bounded scan counts; loan path copy-free |
| `publish_error_precedence` | table order is stable, identical for copy and loan |
| `publish_counter_exhaustion` | no wrap or payload write |
| crash injection before/after each step (both forms) | only a stable old or complete new sample is readable |
