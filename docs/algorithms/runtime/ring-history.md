# Ring & History Algorithms

Scope: circular sequence-history ring — `sequence → {slot_index, generation, sequence}` handles at depth `R`; retention semantics; publisher write path; reader position resolution; per-run addressability watermark. Implements architecture.md §The Dual-Layer Architecture, §Shared History, §Publisher Slot Selection (storage side), §Publisher Epoch (first_valid_seq). Owner: E2 (crate `drm-core::ring`), E1 (integration). Phases 9–10.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-ring-handle-entry` | §Sequence History Ring — `sequence → {slot_index, generation}` | — |
| `REQ-CORE-ring-retention-R` | `@history: N` retains last R; exactly R addressable | — |
| `REQ-CORE-ring-wrap` | wraparound preserves semantics | — |
| `REQ-CORE-ring-release-order` | versioned entry committed before sequence advance | — |
| `REQ-CORE-ring-first-valid` | addressable window never predates `first_valid_seq` (restart watermark) | — |
| `REQ-CORE-ring-portable-atomic` | fixed-width lock-free atomics; no aggregate atomic | — |
| `REQ-CORE-ring-counter-exhaustion` | sequence never wraps | — |

## Data Structures

```rust
pub struct RingEntry { // process-local snapshot
 pub sequence: u64, // the sequence this entry describes (self-describing; enables exact re-read compare)
 pub slot: u32, // payload pool slot index
 pub generation: u64, // slot generation at commit (ABA guard)
}

#[repr(C, align(8))]
pub struct RingCell {
 stamp: AtomicU64,      // even = stable; odd = writer active
 sequence: AtomicU64,
 generation: AtomicU64,
 slot: AtomicU32,
 _pad: u32,
}

pub struct HistoryRing {
 entries: [RingCell; ..], // dynamic count R at fixed CCB offsets
 write_seq: AtomicU64, // next sequence to publish (release-advanced)
}
// Entry for sequence s is stored at index s % R; overwrite of an aged-out entry is
// permitted only under the overflow policy (see overflow-policies.md).
// `first_valid_seq` has one physical owner: ChannelControlBlock (channel-metadata.md).
// Activation requires lock-free AtomicU32 and AtomicU64. No aggregate atomic is used.
```

## Invariants

- **I1 RetentionExact**: with next sequence `ws` and run floor `fv`, the ring holds exactly `min(ws - fv, R)` addressable sequences ending at `ws - 1`. Test: `ring_retention_exact`.
- **I2 WrapStable**: wrapping (any number of laps) never changes retained-history semantics. Test: `ring_wraparound` (≥ 1e6 publishes across laps).
- **I3 EntryCoherence**: an entry read after the publisher's release-store of `write_seq = s` is either `{seq(s), slot(s), gen(s)}` or a *newer* entry — never torn or stale-older. Test: `ring_entry_coherence` (interleaving smoke + property).
- **I4 ReleaseOrdered**: sequence advance is release-ordered after ring entry write. Test: `ring_release_order` (reader observes entry before/with sequence).
- **I5 WatermarkExcludesPriorRun**: no `lookup` returns an entry with `sequence < first_valid_seq`. Test: `ring_first_valid`.
- **I6 PortableCell**: coherent entries use the versioned fixed-width fields above; activation refuses non-lock-free shared atomics. Test: `ring_layout_golden`, `ring_atomic_capability`.
- **I7 CounterNoWrap**: publishing at `write_seq == u64::MAX` returns `SequenceExhausted` before a payload write. Test: `ring_sequence_exhaustion`.

## Functions

```rust
impl HistoryRing {
 // Single-publisher side. SeqCst stamp transitions participate in the hazard handshake.
 fn commit(&self, s: u64, slot: u32, generation: u64, stable: u64) {
 let i = (s % R) as usize;
 let cell = &self.entries[i];
 cell.stamp.store(stable + 1, SeqCst);   // odd: invalid for readers; retires prior entry
 cell.sequence.store(s, Relaxed);
 cell.slot.store(slot, Relaxed);
 cell.generation.store(generation, Relaxed);
 cell.stamp.store(stable + 2, SeqCst);   // even: complete entry is visible
 }

 fn preflight_commit(&self, s: u64) -> Result<u64, Status> {
  let stable = self.entries[(s % R) as usize].stamp.load(Relaxed);
  if stable & 1 != 0 { return Err(InvalidData); }
  stable.checked_add(2).ok_or(RingStampExhausted)?;
  Ok(stable)
 }

 // Publisher restart: exclude every prior-run sequence before the first new publish.
 fn reset_run(&self, ccb: &ChannelControlBlock, first_valid_seq: u64) {
 ccb.first_valid_seq.store(first_valid_seq, Release); // sole field owner; I5
 self.write_seq.store(first_valid_seq, Release);
 }

 fn read_cell(cell: &RingCell, order: ReadOrder) -> Result<RingEntry, Retry> {
  let a = cell.stamp.load(order.stamp());
  if a & 1 != 0 { return Err(Retry); }
  let entry = RingEntry {
   sequence: cell.sequence.load(order.fields()),
   slot: cell.slot.load(order.fields()),
   generation: cell.generation.load(order.fields()),
  };
  let b = cell.stamp.load(order.stamp());
  if a != b || b & 1 != 0 { return Err(Retry); }
  Ok(entry)
 }

 // Reader side (cursor-delivery.md consumes)
 fn lookup(&self, ccb: &ChannelControlBlock, s: u64) -> Result<RingEntry, LookupErr> {
 let ws = self.write_seq.load(Acquire);
 let fv = ccb.first_valid_seq.load(Acquire);
 if s < fv { return Err(AgedOut); } // prior-run sample (I5)
 if s >= ws { return Err(Future); } // not yet published
 if s < ws.saturating_sub(R as u64) { return Err(AgedOut); }
 let e = read_cell(&self.entries[(s % R) as usize], AcquireRead)?;
 if e.sequence != s { return Err(AgedOut); } // self-describing guard against lap during load
 Ok(e)
 }

 fn newest(&self, ccb: &ChannelControlBlock) -> Result<Option<(u64, RingEntry)>, Retry> {
 let ws = self.write_seq.load(Acquire);
 let fv = ccb.first_valid_seq.load(Acquire);
 if ws <= fv { return Ok(None); }
 let s = ws - 1;
 self.lookup(ccb, s).map(|entry| Some((s, entry))).map_err(|_| Retry)
 }

 fn oldest_retained(&self, ccb: &ChannelControlBlock, ws: u64) -> u64 {
 ws.saturating_sub(R as u64).max(ccb.first_valid_seq.load(Acquire)) // I5
 }
}
```

## Protocols

*Publisher commit (integrated with channel publish — pool supplies slot, hazard-protocol.md governs selection when borrows exist):*

1. Write payload into selected slot (payload-pool.md).
2. Preflight the ring stamp and bump slot generation (`gen' = gen + 1`).
3. `ring.commit(s, slot, gen', stamp)` publishes one stable versioned cell.
4. The integrated publish path release-stores `write_seq = s + 1` after checking overflow.

*Publisher restart (publisher-lifecycle.md owns the epoch; ring owns the watermark):*

1. Adopt `first_valid_seq = last_sequence + 1` from the preserved CCB counter.
2. `ring.reset_run(first_valid_seq)` — excludes all prior-run sequences (I5).
3. Publish first new sample at `first_valid_seq`.

*Consumer resolve (cursor-delivery.md owns policy):*

1. `lookup(target_seq)` → `Ok(entry)` / `Err(Future)` / `Err(AgedOut)`; `AgedOut` causes cursor to loop and recompute the window.

## Edge Cases & Errors

| Case | Result |
|---|---|
| R = 1 (`@history: latest`) | single-entry ring; lookup of anything older ⇒ `AgedOut` |
| lookup of future sequence | `Err(Future)` (not yet published) |
| lookup of aged-out sequence | `Err(AgedOut)` → cursor-delivery loops / converts to `Gap { missed }` |
| lookup below `first_valid_seq` | `Err(AgedOut)` — prior-run sample never addressable (I5) |
| entry slot overwritten after `ws` load but before entry load | `e.sequence != s` ⇒ `AgedOut` (no torn handle) |
| concurrent laps during lookup | I3 guarantees entry is current-or-newer; aged-out window handled by lookup branches |
| cell stamp odd or changes during read | `Retry`; never expose a mixed entry |
| sequence reaches `u64::MAX` | `SequenceExhausted` before payload write |
| target cell stamp cannot advance by two | `RingStampExhausted` before payload write |

## Test Matrix

| Case | Expected |
|---|---|
| `ring_retention_exact` (R ∈ 1..=64 fixtures + sampled property) | exactly R addressable |
| `ring_wraparound` (≥ 1e6 publishes, multiple laps) | I2 |
| `ring_entry_coherence` (interleaving + property) | I3 |
| `ring_release_order` | I4 |
| `ring_first_valid` (restart watermark) | I5 |
| `ring_lookup_bounds` | future ⇒ `Future`; aged-out ⇒ `AgedOut` |
| `ring_layout_golden` / `ring_atomic_capability` | I6 |
| `ring_sequence_exhaustion` | I7 |
