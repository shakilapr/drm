# Zero-Copy Hazard Protocol Algorithms

Scope: lockless consumer acquisition with a sequentially consistent control handshake, one single-word hazard slot per consumer endpoint, 64-bit generation ABA protection, publisher slot selection/retirement, borrow scope release, and multi-reader sharing of one slot. Implements architecture.md §Lockless Consumer Acquisition Protocol, §Publisher Slot Selection, §Reader Cursor vs Hazard Slot. Owner: E1 (crate `drm-core::hazard`). Phase 14. Reclamation of abandoned slots: [exhaustion-reclamation.md](exhaustion-reclamation.md).

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
// In CCB (channel-metadata.md layout), sized = declared zero-copy consumer count H' (static):
pub struct HazardWord(AtomicU64); // packed {registration_generation:u32, slot:u32|NONE}
// One word per fixed consumer record. Registration generation prevents stale guard/reclaimer
// clears after record reuse; payload ABA is checked by the ring entry's u64 slot generation.

pub struct PoolState {
 gen: [AtomicU64; ..], // dynamic fixed count; bumped by publisher on reuse (64-bit)
 free_hint: AtomicU32, // publisher-private optimization only; correctness never depends on it
 busy: AtomicBool, // one in-progress write per topic (architecture §Single Publisher)
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
- **I-HZ7 SingleBorrowPerEndpoint**: a consumer endpoint with an outstanding borrow returns `Status::Busy` on a second acquire rather than overwriting its own hazard word. Test: `borrow_single_outstanding`.

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
 for s in 0..ch.pool_size() {
 if scratch.contains(s) { continue; }
 if hazard_protected_on_sc(s) { continue; }
 return Some(s);
 }
 None // caller drops + pool_exhausted++
}
```

The full publisher transaction, including preflight checks and commit ordering, is defined only in
[channel-publish.md](channel-publish.md). Hazard code supplies bounded slot selection; it does not
write payload or advance history.

`history_referenced_slots_sc` clears activation-owned scratch, snapshots `write_seq` and
`first_valid_seq`, and visits at most `R` versioned ring cells. Because the caller holds `busy`, no
publisher can change the ring during this scan. Every accepted entry in the addressable window marks
its slot. Pool indices are `< u32::MAX`; `u32::MAX` is the sole `NONE` encoding.

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

*Loom gate (`hazard_loom`, mandatory):* model the full publish / retire+scan / acquire / release / reclaim protocol with deliberately tiny constants (2 slots, 1–2 positions, 2-bit generation) and enumerate all interleavings; assert I-HZ1/I-HZ2/I-HZ5/I-HZ7 hold. Stress testing alone cannot exercise weak-memory interleavings.

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
| consumer crashes mid-borrow | generation-bearing hazard stays set until exact-incarnation reclamation |
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
| pool exhaustion with H borrowers at `pool_size = R+H+1` | payload-pool I-P5 |
