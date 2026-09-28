# Zero-Copy Hazard Protocol Algorithms

Scope: lockless consumer acquisition with a sequentially consistent control handshake, one
single-word hazard slot per consumer endpoint, 64-bit generation ABA protection, publisher slot
selection/retirement, borrow scope release, and multi-reader sharing of one slot. Implements
architecture.md §Lockless Consumer Acquisition Protocol, §Publisher Slot Selection, §Reader Cursor
vs Hazard Slot. Owner: E1 (crate `drm-core::hazard`). Phase 14. Reclamation of abandoned slots:
[exhaustion-reclamation.md](exhaustion-reclamation.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-hazard-protected-word` | `protected[consumer_id] = slot \| NONE`, one aligned atomic word per consumer | `REQ-CORE-033` |
| `REQ-CORE-hazard-sc-handshake` | protect / recheck / retire+scan / reclaim use SeqCst; payload stays Release/Acquire | `REQ-CORE-034` |
| `REQ-CORE-hazard-acquisition` | §Lockless Consumer Acquisition Protocol steps 1–6 | `REQ-CORE-030` |
| `REQ-CORE-hazard-generation-aba` | 64-bit generation in the ring entry prevents ABA on slot wrap | `REQ-CORE-031` |
| `REQ-CORE-hazard-single-borrow` | at most one outstanding borrow per consumer endpoint; second ⇒ `Busy` | `REQ-CORE-035` |
| `REQ-CORE-publisher-slot-selection` | publisher retires + scans under SeqCst; picks slot not history-referenced nor hazard-protected | `REQ-CORE-055` |
| `REQ-CORE-hazard-multi-reader` | multiple consumers sharing one latest-sample slot are safe | `REQ-CORE-032` |
| `REQ-CORE-borrow-never-overwritten` | actively borrowed slot never overwritten | `REQ-CORE-004` |

## Data Structures

```rust
// In CCB (channel-metadata.md layout), sized = declared borrower-record count
// (Σ manifest borrowers + diagnostic range — static, payload-pool.md proof step 0):
pub struct HazardWord(AtomicU64); // packed {registration_generation:u32, slot:u32|NONE}
// One word per fixed consumer record. Registration generation prevents stale guard/reclaimer
// clears after record reuse; payload ABA is checked by the ring entry's u64 slot generation.

// Single physical in-progress-write flag: ChannelControlBlock.publish_busy
// (channel-metadata.md). PoolState below holds a view/reference to that CCB field, never a
// second AtomicBool — one physical owner, one repair site (publisher restart clears exactly it).
pub struct PoolState<'ccb> {
 gen: [AtomicU64; ..], // dynamic fixed count; bumped by publisher on reuse (64-bit)
 free_hint: AtomicU32, // publisher-private optimization only; correctness never depends on it
 busy: &'ccb AtomicBool, // == ccb.publish_busy (hazard-protocol I-HZ6; architecture §Single Publisher)
}
// Generation lives in the atomically published ring entry, so ABA is decided against a 64-bit
// value, not a truncated hazard word.
```

## Invariants

- **I-HZ1 NeverOverwritten**: while a generation-bearing hazard word names slot `s`, publisher reuse of `s` is impossible. Test: `hazard_never_overwritten` (Loom + storm).
- **I-HZ2 NoABA**: a stale borrow can never validate as fresh; ABA rejection uses the **64-bit** generation carried in the self-describing ring entry, verified on the SC re-read. Test: `hazard_aba` (Loom with deliberately tiny generation width; plus long-run wrap).
- **I-HZ3 RacePointCoverage**: every race point of the acquisition protocol has a named forcing test. Test: `hazard_race_matrix` + `hazard_loom`.
- **I-HZ4 MultiReaderShare**: ≥ 2 consumer endpoints borrowing the same latest sample each protect that one slot via their own hazard word; publisher never corrupts it. Test: `hazard_multi_reader`.
- **I-HZ5 ReleaseFrees**: scope exit clears protection (SeqCst); slot becomes reusable once no history entry references it either. Test: `hazard_release_reusable`.
- **I-HZ6 SingleInProgress**: at most one publish in progress per topic; concurrent publish ⇒ `Status::Busy`. Test: `publish_busy_guard`.
- **I-HZ7 SingleBorrowPerEndpoint**: a consumer endpoint with an outstanding borrow returns
  `Status::Busy` on a second acquire rather than overwriting its own hazard word. Test:
  `borrow_single_outstanding`.
- **I-HZ8 AcquisitionRecheck**: after installing the hazard and before touching payload bytes, every
  acquisition rechecks `state == Active && registration_generation == my_generation`
  (consumer-registration.md I-CR6); on failure it clears the exact installed word and aborts. Test:
  `consumer_close_vs_acquire` (same test owns both invariants).

## Functions

```rust
// ---- Consumer acquisition (architecture steps 1–6) ----
fn acquire(ch: &Channel, target_seq: u64, reg: &Registration) -> Result<Borrow, Status> {
 let empty = pack(reg.generation, NONE);
 let e1 = ch.ring().lookup_sc(target_seq)?; // versioned-cell SC snapshot
 if reg.hazard.compare_exchange(empty, pack(reg.generation, e1.slot), SeqCst, SeqCst).is_err() {
  return Err(Status::Busy);
 }
 let e2 = match ch.ring().lookup_sc(target_seq) { // 3. SC re-read
  Ok(entry) => entry,
  Err(err) => {
   reg.hazard.compare_exchange(pack(reg.generation, e1.slot), empty, SeqCst, SeqCst)
    .map_err(|_| Status::InvalidData)?;
   return Err(err.into());
  }
 };
 if e1 != e2 { // 4. publisher rolled during acquisition
 reg.hazard.compare_exchange(pack(reg.generation, e1.slot), empty, SeqCst, SeqCst)
  .map_err(|_| Status::InvalidData)?;
 return Err(Retry);
 }
 // 4b. Registration recheck (I-HZ8 / consumer-registration.md I-CR6): close-vs-acquire race.
 if !acquisition_valid(reg.rec, reg.generation) {
  // Best-effort clear of the exact installed word; if the word was already overwritten
  // (record changed authority) there is nothing of ours left to clear.
  let _ = reg.hazard.compare_exchange(pack(reg.generation, e1.slot), empty, SeqCst, SeqCst);
  return Err(Status::Busy);
 }
 // 5. locked: process immutable payload in place
 Ok(Borrow { entry: e2, guard: HazardGuard { reg, slot: e2.slot } })
}

// try_acquire(seq, own) — cursor-delivery.md entry point:
// MemOwn::Borrow → acquire(seq) above (transient view, hazard held until Drop).
// MemOwn::Copy → acquire(seq), copy into caller memory, release immediately.
// Internal outcomes: AgedOut or Retry. Public failure: Busy or InvalidData.

// ---- Publisher retirement + slot selection (same SeqCst handshake) ----
fn select_slot(ch: &Channel, scratch: &mut FixedBitSet) -> Option<usize> {
 history_referenced_slots_sc(ch, scratch); // busy guard makes writer-side history stable
 mark_hazard_protected_sc(ch, scratch);    // each hazard word read exactly once
 (0..ch.pool_size()).find(|&s| !scratch.contains(s)) // rotating free_hint start is an optimization
}
```

The full publisher transaction, including preflight checks and commit ordering, is defined only in
[channel-publish.md](channel-publish.md). Hazard code supplies bounded slot selection; it does not
write payload or advance history.

`history_referenced_slots_sc` clears activation-owned scratch, snapshots `write_seq` and
`first_valid_seq`, and visits at most `R` versioned ring cells. Because the caller holds `busy`, no
publisher can change the ring during this scan. Every accepted entry in the addressable window marks
its slot. Pool indices are `< u32::MAX`; `u32::MAX` is the sole `NONE` encoding.

**Hazard-validity rule (`mark_hazard_protected_sc`, the sole specified form of
`hazard_protected_on_sc`):** one SeqCst pass over all `H'` hazard words; a word marks its slot iff
`word == {the record's CURRENT registration_generation, slot != NONE}` — **record state is not
consulted**. Rationale: only the current-generation owner can install a matching word, and a
generation match always implies protection is required — the owner is alive and reading (state
`Active` or `Closing` with a live borrow; a close that saw the borrow left it in `Closing`), or the
owner died and the slot is reusable only after lock-proven reclamation clears the word
(exhaustion-reclamation.md). `Closing` blocks **new** borrows; it never cancels an existing borrow's
protection. A record interrupted in `Claiming` carries a word whose generation predates the current
one, so it matches nothing, protects nothing, and pins nothing — the claimant or reclaimer
overwrites it under the registration lock (consumer-registration.md I-CR7). Non-matching words are
skipped without inspection. The combined selection scan costs `O(R + H' + pool/word)` with each
hazard word read exactly once — no per-slot rescans.

## Protocols

*Acquisition race matrix (I-HZ3 — each row is a named forcing test; Loom explores the interleavings systematically, scheduling storms complement it):*

| # | Race point | Forcing test |
|---|---|---|
| R1 | publisher rolls entry between read(1) and intent-store(2) | `race_r1_roll_before_intent` |
| R2 | rolls between intent-store(2) and re-read(3) | `race_r2_roll_after_intent` |
| R3 | generation wraps between intent and re-read | `race_r3_generation_wrap` |
| R4 | two consumers acquire same sequence concurrently | `race_r4_concurrent_acquire` |
| R5 | consumer releases while publisher scans hazard words | `race_r5_release_during_scan` |
| R6 | publisher reuses slot exactly at consumer re-read | `race_r6_reuse_during_reread` |

*Loom gate (`hazard_loom`, mandatory):* model the full publish / retire+scan / acquire / release /
reclaim protocol — driven through the **public** read APIs (`borrow_latest`, `copy_latest`,
`borrow_next`, `copy_next`), not through `acquire` alone, so the unified primitive is what is
proven — with deliberately tiny constants (2 slots, 1–2 positions, 2-bit generation) and enumerate
all interleavings; assert I-HZ1/I-HZ2/I-HZ5/I-HZ7/I-HZ8 hold. Stress testing alone cannot exercise
weak-memory interleavings.

*Retire/scan proof:* if a reader's first snapshot names slot `s`, either its protection CAS precedes
the publisher's SC scan and the publisher skips `s`, or the scan precedes protection. In the latter
case, the earlier commit that removed `s` from history is already SC-ordered before the scan, so the
reader's second versioned snapshot differs and it rejects before reading payload bytes.

*Storm suite (`hazard_storm`):* 1 publisher × H readers × random depths × hours-long randomized run with a shadow thread asserting I-HZ1 continuously, on x86-64 and ARM64.

## Edge Cases & Errors

| Case | Result |
|---|---|
| target sequence aged out during acquire | `Retry` (protection cleared; caller may borrow newest or copy) |
| second acquire on an endpoint holding a borrow | `Status::Busy` (I-HZ7) |
| borrow while pool exhausted | protected target still readable; new publishes drop (exhaustion-reclamation.md) |
| publisher finds no free slot | drop + `pool_exhausted` counter++ — never blocks (I-HZ1 preserved) |
| consumer crashes mid-borrow | generation-bearing hazard stays set until registration-lock reclamation (exhaustion-reclamation.md) |
| generation reaches `u64::MAX` | `Status::GenerationExhausted` (no silent wrap) |
| same slot re-borrowed after release | allowed; ring entry generation unchanged since slot not yet reused |

## Test Matrix

| Case | Expected |
|---|---|
| `hazard_loom` (tiny widths, all interleavings) | I-HZ1/I-HZ2/I-HZ5/I-HZ7 |
| `race_r1..r6` (six forcing tests) | I-HZ3 |
| `hazard_aba` (Loom + long-run wrap) | I-HZ2 |
| `hazard_never_overwritten` storm (x86-64 + ARM64) | I-HZ1 |
| `hazard_multi_reader` (4 consumers, 1 slot) | I-HZ4 |
| `hazard_release_reusable` | I-HZ5 |
| `publish_busy_guard` (two threads) | I-HZ6 |
| `borrow_single_outstanding` | I-HZ7 |
| `consumer_close_vs_acquire` (cross-doc with consumer-registration.md; close at every acquisition boundary) | I-HZ8 + I-CR6 |
| `hazard_closing_protects_live_borrow` (close returns Busy with a live borrow; publisher must skip that slot until the guard releases) | I-HZ1 + I-CR7: protection follows the word, not the state |
| pool exhaustion with H borrowers at `pool_size = R+H+1` | payload-pool I-P5 |
