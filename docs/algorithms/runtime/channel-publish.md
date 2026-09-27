# Channel Publish Algorithms

Scope: one authoritative publish transaction joining validation, overflow gating, slot selection,
reclamation, payload write, generation advance, ring commit, and diagnostics. Implements
architecture.md §Publisher Slot Selection, §History Overflow, and §Zero Data-Path Allocation.
Owner: E1 (`drm-core::channel`). Depends on all runtime storage protocols.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-publish-transaction` | publisher slot selection and commit | `REQ-CORE-052` |
| `REQ-CORE-publish-linearization` | release commit makes one sample visible | `REQ-CORE-051` |
| `REQ-CORE-publish-error-effects` | failed publish never changes visible history | `REQ-CORE-050` |
| `REQ-CORE-publish-zero-alloc` | steady-state data path reuses fixed storage | `REQ-CORE-053` |
| `REQ-CORE-publish-counter-bounds` | sequence and generation never wrap | `REQ-CORE-049` |

## Data Structures

```rust
pub struct PublishScratch {
 referenced: FixedBitSet, // capacity = pool_size; allocated at activation
}

pub struct PublishGuard<'a> {
 busy: &'a AtomicBool,
 scratch: &'a mut FixedBitSet, // exclusive because `busy` is held
} // Drop release-stores `busy = false`
```

## Invariants

- **I-PB1 AtomicVisibility**: readers observe the old history or the complete new sample, never a
  partial payload or handle. Test: `publish_visibility_loom`.
- **I-PB2 ErrorNoCommit**: `InvalidArg`, `Busy`, `QueueFull`, `PoolExhausted`, and counter exhaustion
  leave `write_seq`, ring entries, and current sample unchanged. Test: `publish_error_no_commit`.
- **I-PB3 FixedWork**: publish allocates zero bytes and performs loops bounded by declared ring,
  pool, and consumer counts. Test: `publish_zero_alloc`.
- **I-PB4 ErrorPrecedence**: validation precedes authority and capacity decisions; the ordering below
  is stable ABI behavior. Test: `publish_error_precedence`.
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
  ch.pool_exhausted.fetch_add(1, Relaxed);
  reclaim::scan_exhausted(ch);                     // bounded; no wait or retry loop
  return Err(PoolExhausted);                        // incoming sample is always dropped
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
```

`PoolExhausted` increments once for the dropped incoming sample. Its bounded reclamation scan may
free capacity, but never retries that sample; the next publish sees the capacity immediately. No
payload byte is written before a slot and both counters are known valid.

## Protocols

1. Validate source length and variable-length header arithmetic.
2. Acquire `busy` with one compare-exchange; failure returns `Busy`.
3. Check sequence capacity and `reject` retirement gates.
4. Build the referenced-slot bitmap in activation-owned scratch and scan hazards with SeqCst loads.
5. On no slot, increment exhaustion once, perform one bounded dead-owner scan, and drop this sample.
6. Check generation capacity, then write payload and length.
7. Publish the versioned ring cell, then release-store `write_seq`; the latter linearizes visibility.
8. Release `busy` on every exit.

## Edge Cases & Errors

| Order | Case | Status | Visible side effect |
|---:|---|---|---|
| 1 | invalid payload shape/size | `InvalidArg` | none |
| 2 | another publish active | `Busy` | none |
| 3 | sequence cannot advance | `SequenceExhausted` | none |
| 4 | required consumer gates retirement | `QueueFull` | none |
| 5 | no reusable slot; bounded reclaim runs after the drop decision | `PoolExhausted` | counter +1 only |
| 6 | selected slot generation/ring stamp exhausted | counter-exhaustion status | none |
| 7 | success | — | one committed sample |

## Test Matrix

| Case | Expected |
|---|---|
| `publish_visibility_loom` | I-PB1 |
| `publish_error_no_commit` | I-PB2 for every status |
| `publish_zero_alloc` | zero allocations; bounded scan counts |
| `publish_error_precedence` | table order is stable |
| `publish_counter_exhaustion` | no wrap or payload write |
| crash injection before/after each step | only a stable old or complete new sample is readable |
