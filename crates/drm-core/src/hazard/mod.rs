//! Zero-copy hazard protocol (`docs/algorithms/runtime/hazard-protocol.md`).
//! REQ: REQ-CORE-033 (one aligned atomic word per consumer), REQ-CORE-034 (`SeqCst`
//! control handshake), REQ-CORE-030 (acquisition steps 1-6), REQ-CORE-031 (64-bit
//! generation ABA), REQ-CORE-035 (single outstanding borrow), REQ-CORE-055
//! (retire + scan), REQ-CORE-032 (multi-reader share), REQ-CORE-004 (borrowed slot
//! never overwritten).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::channel::ccb::ChannelControlBlock;
use crate::ring::{HistoryRing, LookupErr, RingEntry};
use drm_types::Status;

/// ABA long-run forcing harness.
#[cfg(test)]
mod aba;
/// Deterministic acquisition race-matrix forcing tests (R1-R6).
#[cfg(test)]
mod race;
/// Selection-vs-borrows property tests.
#[cfg(test)]
mod select;
/// Shared test fixtures for the forcing suites.
#[cfg(test)]
mod support;

/// Sole `NONE` encoding for hazard slot indices (pool indices are `< u32::MAX`).
/// REQ: REQ-CORE-033.
pub const HAZARD_NONE: u32 = u32::MAX;

/// One hazard word per consumer record: packed `{registration_generation:u32, slot:u32}`.
/// REQ: REQ-CORE-033.
#[derive(Debug)]
pub struct HazardWord(AtomicU64);

impl HazardWord {
    /// Zero-valued word (generation 0, slot NONE).
    #[must_use]
    pub fn zeroed() -> Self {
        Self(AtomicU64::new(pack(0, HAZARD_NONE)))
    }

    /// Packed view of the current word.
    #[must_use]
    pub fn load(&self, order: Ordering) -> (u32, u32) {
        unpack(self.0.load(order))
    }

    /// REQ: REQ-CORE-030 — protect/install or clear (`SeqCst` CAS); `false` = raced.
    pub fn compare_exchange(&self, expected: u64, new: u64) -> bool {
        self.0
            .compare_exchange(expected, new, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

/// Pack `{generation, slot}` into one u64.
#[must_use]
pub fn pack(generation: u32, slot: u32) -> u64 {
    (u64::from(generation) << 32) | u64::from(slot)
}

/// Unpack a hazard word; the halves are u32 by construction (packing inverse).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn unpack(word: u64) -> (u32, u32) {
    ((word >> 32) as u32, word as u32)
}

/// Consumer record view used by acquisition and the selection scan
/// (consumer-registration.md owns the full record layout; the hazard protocol
/// consults generation, active state, and the hazard word only).
/// REQ: REQ-CORE-036 (registration authority).
pub struct ConsumerRec {
    /// Registration generation (bumped on record reuse; stale words match nothing).
    pub generation: AtomicU32,
    /// `false` after close; a live borrow still protects its slot (word-based rule).
    pub active: AtomicBool,
    /// This endpoint's single hazard word.
    pub hazard: HazardWord,
}

impl ConsumerRec {
    /// New record in the `Active` state; the hazard word starts at
    /// `{generation, NONE}` (the acquisition CAS's empty form).
    #[must_use]
    pub fn new(generation: u32) -> Self {
        Self {
            generation: AtomicU32::new(generation),
            active: AtomicBool::new(true),
            hazard: HazardWord(AtomicU64::new(pack(generation, HAZARD_NONE))),
        }
    }

    /// Close: new borrows refused; existing borrows keep protection until release.
    /// REQ: REQ-CORE-002, I-HZ1.
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);
    }
}

/// Acquisition failure taxonomy (cursor-delivery.md consumes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireErr {
    /// Target aged out / future / rolled (recompute the window).
    Lookup(LookupErr),
    /// Publisher rolled the entry during the handshake; recompute and retry.
    Retry,
    /// Terminal status.
    Status(Status),
}

impl From<AcquireErr> for Status {
    fn from(e: AcquireErr) -> Self {
        match e {
            AcquireErr::Lookup(_) | AcquireErr::Retry => Self::Retry,
            AcquireErr::Status(s) => s,
        }
    }
}

/// Acquired transient view; drop releases the hazard (I-HZ5).
/// REQ: REQ-CORE-003.
pub struct Borrow {
    /// The validated ring entry (self-describing; ABA-checked on the SC re-read).
    pub entry: RingEntry,
    hazard: *const HazardWord,
    installed: u64,
}

// SAFETY: the hazard word has CCB (segment) lifetime, outliving every borrow;
// the pointer is used only to clear this endpoint's own word.
unsafe impl Send for Borrow {}

impl Borrow {
    fn install(entry: RingEntry, hazard: &HazardWord, word: u64) -> Self {
        Self {
            entry,
            hazard: std::ptr::from_ref(hazard),
            installed: word,
        }
    }

    /// Slot held by this borrow.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.entry.slot
    }

    /// Explicit release (also runs on drop); idempotent.
    pub fn release(&mut self) {
        if self.installed != 0 {
            // SAFETY: hazard pointer has segment lifetime (see unsafe impl Send).
            let word = unsafe { &*self.hazard };
            let empty = (self.installed >> 32 << 32) | u64::from(HAZARD_NONE);
            word.compare_exchange(self.installed, empty);
            self.installed = 0;
        }
    }
}

impl Drop for Borrow {
    fn drop(&mut self) {
        self.release();
    }
}

/// Consumer acquisition steps 1-6 (hazard-protocol.md §Functions `acquire`).
/// REQ: REQ-CORE-030.
///
/// # Errors
/// [`AcquireErr`] per the race matrix: `Lookup` (Future/AgedOut), `Retry`, or
/// terminal [`Status::Busy`].
pub fn acquire(
    ring: &HistoryRing,
    ccb: &ChannelControlBlock,
    target_seq: u64,
    rec: &ConsumerRec,
) -> Result<Borrow, AcquireErr> {
    let my_generation = rec.generation.load(Ordering::Acquire);
    let empty = pack(my_generation, HAZARD_NONE);
    // 1. Versioned-cell SC snapshot.
    let e1 = ring
        .lookup_sc(ccb, target_seq)
        .map_err(AcquireErr::Lookup)?;
    // 2. Install protection.
    if !rec
        .hazard
        .compare_exchange(empty, pack(my_generation, e1.slot))
    {
        return Err(AcquireErr::Status(Status::Busy));
    }
    let installed = pack(my_generation, e1.slot);
    // 3. SC re-read.
    let e2 = match ring.lookup_sc(ccb, target_seq) {
        Ok(entry) => entry,
        Err(err) => {
            rec.hazard.compare_exchange(installed, empty);
            return Err(AcquireErr::Lookup(err));
        }
    };
    if e1 != e2 {
        // 4. Publisher rolled during acquisition.
        rec.hazard.compare_exchange(installed, empty);
        return Err(AcquireErr::Retry);
    }
    // 4b. Registration recheck (I-HZ8).
    if !rec.acquisition_valid(my_generation) {
        rec.hazard.compare_exchange(installed, empty);
        return Err(AcquireErr::Status(Status::Busy));
    }
    // 5. Locked: process the immutable payload in place.
    Ok(Borrow::install(e2, &rec.hazard, installed))
}

impl ConsumerRec {
    /// REQ: I-HZ8 — acquisition recheck: record still `Active` with our generation.
    fn acquisition_valid(&self, my_generation: u32) -> bool {
        self.active.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == my_generation
    }
}

/// Publisher-side slot selection: history-referenced + hazard-protected marks.
/// REQ: REQ-CORE-055. Caller holds the publish busy guard (stable history);
/// `scratch` is activation-owned and reused across calls.
pub fn select_slot(
    ring: &HistoryRing,
    ccb: &ChannelControlBlock,
    recs: &[ConsumerRec],
    scratch: &mut Vec<u32>,
    pool_size: u64,
) -> Option<u32> {
    scratch.clear();
    // History-referenced slots (at most R accepted entries).
    let ws = ccb.write_seq.load(Ordering::Acquire);
    let fv = ccb.first_valid_seq.load(Ordering::Acquire);
    let oldest = ws.saturating_sub(ring.depth()).max(fv);
    for s in oldest..ws {
        if let Ok(entry) = ring.lookup_sc(ccb, s) {
            scratch.push(entry.slot);
        }
    }
    // Hazard-protected: one SeqCst read per word; marks iff the word carries the
    // record's CURRENT registration generation and a non-NONE slot. Record state
    // is never consulted (hazard-validity rule).
    for rec in recs {
        let cur = rec.generation.load(Ordering::Relaxed);
        let (wgen, slot) = rec.hazard.load(Ordering::SeqCst);
        if slot != HAZARD_NONE && wgen == cur {
            scratch.push(slot);
        }
    }
    let pool = u32::try_from(pool_size).ok()?;
    (0..pool).find(|s| !scratch.contains(s))
}

/// Generation bump guard: 64-bit, no silent wrap.
/// REQ: REQ-CORE-031.
///
/// # Errors
/// [`Status::GenerationExhausted`] at `u64::MAX`.
pub fn bump_generation(current: u64) -> Result<u64, Status> {
    current.checked_add(1).ok_or(Status::GenerationExhausted)
}

#[cfg(test)]
mod tests {
    use super::{
        AcquireErr, Borrow, ConsumerRec, HAZARD_NONE, HazardWord, acquire, pack, select_slot,
        unpack,
    };
    use crate::channel::ccb::ChannelControlBlock;
    use crate::ring::{HistoryRing, LookupErr};
    use drm_types::Status;
    use std::sync::atomic::Ordering;

    fn ccb() -> ChannelControlBlock {
        ChannelControlBlock::zeroed()
    }

    fn ring(r: u64) -> HistoryRing {
        HistoryRing::new(r).unwrap_or_else(|e| unreachable!("ring: {e:?}"))
    }

    /// Commit `seq` to `slot` and advance the published watermark (single writer).
    fn commit(ring: &HistoryRing, ccb: &ChannelControlBlock, seq: u64, slot: u32, generation: u64) {
        let stable = ring
            .preflight_commit(seq)
            .unwrap_or_else(|e| unreachable!("preflight seq={seq}: {e:?}"));
        ring.commit(seq, slot, generation, stable);
        ccb.write_seq.store(seq + 1, Ordering::Release);
    }

    /// `acquire` steps 1-4 with an injected publisher roll between the two
    /// versioned snapshots, so the `Retry` branch is exercised deterministically
    /// on one thread. Mirrors the production comparison; test-only.
    fn acquire_with_roll(
        ring: &HistoryRing,
        ccb: &ChannelControlBlock,
        target_seq: u64,
        rec: &ConsumerRec,
        roll: impl FnOnce(),
    ) -> Result<Borrow, AcquireErr> {
        let g = rec.generation.load(Ordering::Acquire);
        let empty = pack(g, HAZARD_NONE);
        let e1 = ring
            .lookup_sc(ccb, target_seq)
            .map_err(AcquireErr::Lookup)?;
        if !rec.hazard.compare_exchange(empty, pack(g, e1.slot)) {
            return Err(AcquireErr::Status(Status::Busy));
        }
        let installed = pack(g, e1.slot);
        roll();
        let e2 = match ring.lookup_sc(ccb, target_seq) {
            Ok(entry) => entry,
            Err(err) => {
                rec.hazard.compare_exchange(installed, empty);
                return Err(AcquireErr::Lookup(err));
            }
        };
        if e1 != e2 {
            rec.hazard.compare_exchange(installed, empty);
            return Err(AcquireErr::Retry);
        }
        Ok(Borrow::install(e2, &rec.hazard, installed))
    }

    /// Protect `slot` for `rec` from its empty word.
    fn protect(rec: &ConsumerRec, g: u32, slot: u32) {
        assert!(
            rec.hazard
                .compare_exchange(pack(g, HAZARD_NONE), pack(g, slot)),
            "install failed"
        );
    }

    /// Clear `rec`'s word if it still names a slot at generation `gen`.
    fn unprotect(rec: &ConsumerRec, g: u32) {
        let (wgen, slot) = rec.hazard.load(Ordering::SeqCst);
        if wgen == g && slot != HAZARD_NONE {
            assert!(
                rec.hazard
                    .compare_exchange(pack(g, slot), pack(g, HAZARD_NONE)),
                "clear failed"
            );
        }
    }

    /// REQ: REQ-CORE-033 — the sole `NONE` encoding and packed `{generation, slot}`
    /// word round-trip; a zeroed word is `{0, NONE}`.
    #[test]
    fn hazard_slot_none_sentinel() {
        assert_eq!(HAZARD_NONE, u32::MAX);
        let word = pack(0, HAZARD_NONE);
        assert_eq!(unpack(word), (0, HAZARD_NONE));
        // pack/unpack are inverses for arbitrary halves.
        assert_eq!(unpack(pack(9, 3)), (9, 3));
        assert_eq!(pack(9, 3) & u64::from(u32::MAX), 3);
        assert_eq!(
            HazardWord::zeroed().load(Ordering::SeqCst),
            (0, HAZARD_NONE)
        );
    }

    /// REQ: REQ-CORE-033, REQ-CORE-035 — install/clear is one CAS; a second
    /// install from the empty form fails while a borrow is held (I-HZ7).
    #[test]
    fn hazard_slot_set_clear_compare() {
        let rec = ConsumerRec::new(5);
        let empty = pack(5, HAZARD_NONE);
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (5, HAZARD_NONE));
        assert!(rec.hazard.compare_exchange(empty, pack(5, 4)));
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (5, 4));
        // second acquire on the held word cannot claim the empty form
        assert!(!rec.hazard.compare_exchange(empty, pack(5, 7)));
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (5, 4));
        // release clears exactly the installed word; a stale key no longer matches
        assert!(rec.hazard.compare_exchange(pack(5, 4), empty));
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (5, HAZARD_NONE));
        assert!(!rec.hazard.compare_exchange(pack(5, 4), empty));
    }

    /// REQ: REQ-CORE-030 — steps 1-5: a normal acquire returns the newest eligible
    /// entry, installs this endpoint's protection, and drop releases it (I-HZ5).
    #[test]
    fn hazard_acquire_returns_newest() {
        let ring = ring(8);
        let ccb = ccb();
        commit(&ring, &ccb, 0, 2, 11);
        commit(&ring, &ccb, 1, 5, 22);
        let rec = ConsumerRec::new(3);
        let borrow =
            acquire(&ring, &ccb, 1, &rec).unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        assert_eq!(borrow.slot(), 5);
        assert_eq!(borrow.entry.sequence, 1);
        assert_eq!(borrow.entry.generation, 22);
        // protection installed while the borrow is live
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (3, 5));
        drop(borrow);
        // released on drop -> reusable empty form
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (3, HAZARD_NONE));
    }

    /// REQ: REQ-CORE-030, REQ-CORE-035 — future/aged targets map to `Lookup` and
    /// leave no protection installed; a live borrow refuses a second acquire.
    #[test]
    fn hazard_acquire_lookup_and_busy() {
        let ring = ring(4);
        let ccb = ccb();
        commit(&ring, &ccb, 0, 0, 1);
        let rec = ConsumerRec::new(1);
        assert_eq!(
            acquire(&ring, &ccb, 9, &rec).err(),
            Some(AcquireErr::Lookup(LookupErr::Future))
        );
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (1, HAZARD_NONE));
        // hold a borrow, then a second acquire must report Busy (I-HZ7)
        let borrow = acquire(&ring, &ccb, 0, &rec).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(
            acquire(&ring, &ccb, 0, &rec).err(),
            Some(AcquireErr::Status(Status::Busy))
        );
        drop(borrow);
        assert!(acquire(&ring, &ccb, 0, &rec).is_ok());
    }

    /// REQ: REQ-CORE-030, I-HZ8 — a record closed during the handshake fails the
    /// registration recheck with `Busy` and clears the installed word.
    #[test]
    fn hazard_close_vs_acquire() {
        let ring = ring(4);
        let ccb = ccb();
        commit(&ring, &ccb, 0, 0, 1);
        let rec = ConsumerRec::new(2);
        rec.close();
        assert_eq!(
            acquire(&ring, &ccb, 0, &rec).err(),
            Some(AcquireErr::Status(Status::Busy))
        );
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (2, HAZARD_NONE));
    }

    /// REQ: REQ-CORE-030, REQ-CORE-034 — intent-store then re-read mismatch (a roll
    /// between the snapshots) clears the exact installed word and yields `Retry`.
    #[test]
    fn hazard_acquire_retry_clears() {
        let ring = ring(4);
        let ccb = ccb();
        commit(&ring, &ccb, 0, 1, 7);
        let rec = ConsumerRec::new(9);
        let outcome = acquire_with_roll(&ring, &ccb, 0, &rec, || {
            // publisher rolls the entry to a new slot/generation mid-handshake
            let stable = ring
                .preflight_commit(0)
                .unwrap_or_else(|e| unreachable!("{e:?}"));
            ring.commit(0, 3, 8, stable);
        });
        assert_eq!(outcome.err(), Some(AcquireErr::Retry));
        assert_eq!(rec.hazard.load(Ordering::SeqCst), (9, HAZARD_NONE));
        // a fresh acquire now observes the rolled entry
        let borrow = acquire(&ring, &ccb, 0, &rec).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(borrow.slot(), 3);
    }

    /// REQ: REQ-CORE-055, REQ-CORE-004 — a protected slot is skipped; when every
    /// pool slot is referenced or protected, selection returns `None`.
    #[test]
    fn hazard_selection_skips_protected() {
        let ring = ring(2);
        let ccb = ccb();
        commit(&ring, &ccb, 0, 0, 1);
        commit(&ring, &ccb, 1, 1, 2);
        let rec = ConsumerRec::new(1);
        protect(&rec, 1, 2);
        let recs = [rec];
        let mut scratch = Vec::new();
        // pool = 3: history {0,1} + protected {2} -> nothing free
        assert_eq!(select_slot(&ring, &ccb, &recs, &mut scratch, 3), None);
        // pool = 4: slot 3 is free and chosen
        assert_eq!(select_slot(&ring, &ccb, &recs, &mut scratch, 4), Some(3));
    }

    /// REQ: REQ-CORE-033, I-HZ1 — a word whose generation predates the record's
    /// current registration generation protects nothing (hazard-validity rule).
    #[test]
    fn hazard_selection_stale_generation() {
        let ring = ring(2);
        let ccb = ccb();
        commit(&ring, &ccb, 0, 0, 1);
        commit(&ring, &ccb, 1, 1, 2);
        let rec = ConsumerRec::new(1);
        // overwrite with a stale-generation word naming slot 2
        assert!(
            rec.hazard
                .compare_exchange(pack(1, HAZARD_NONE), pack(0, 2))
        );
        let recs = [rec];
        let mut scratch = Vec::new();
        // stale word ignored -> slot 2 is the sole free slot
        assert_eq!(select_slot(&ring, &ccb, &recs, &mut scratch, 3), Some(2));
    }

    /// REQ: REQ-CORE-055, REQ-CORE-032 — selection property under H >= 2 held
    /// borrows: `select_slot` never returns a history-referenced or
    /// hazard-protected slot, and returns `None` only when every pool slot is
    /// forbidden. Driven over many iterations with no sleeps.
    #[test]
    fn hazard_selection_property() {
        const H: usize = 3;
        let depth = 8_u64;
        let pool = depth + u64::try_from(H).unwrap_or(0) + 2;
        let pool_u32 = u32::try_from(pool).unwrap_or(u32::MAX);
        let ring = ring(depth);
        let ccb = ccb();
        let recs: Vec<ConsumerRec> = (0..H)
            .map(|i| ConsumerRec::new(u32::try_from(i + 1).unwrap_or(1)))
            .collect();
        let gens: Vec<u32> = recs
            .iter()
            .map(|r| r.generation.load(Ordering::Acquire))
            .collect();
        let mut scratch: Vec<u32> = Vec::with_capacity(usize::try_from(pool).unwrap_or(0));
        for seq in 0..20_000_u64 {
            commit(
                &ring,
                &ccb,
                seq,
                u32::try_from(seq % pool).unwrap_or(0),
                seq + 1,
            );
            // each endpoint holds a fresh synthetic borrow of a distinct slot
            let mut held: Vec<u32> = Vec::with_capacity(H);
            for (i, rec) in recs.iter().enumerate() {
                unprotect(rec, gens[i]);
                let target =
                    u32::try_from((seq + u64::try_from(i).unwrap_or(0)) % pool).unwrap_or(0);
                protect(rec, gens[i], target);
                held.push(target);
            }
            // exact forbidden set: history-referenced slots + protected slots
            let ws = ccb.write_seq.load(Ordering::Acquire);
            let fv = ccb.first_valid_seq.load(Ordering::Acquire);
            let oldest = ws.saturating_sub(ring.depth()).max(fv);
            let mut forbidden: Vec<u32> = Vec::new();
            for t in oldest..ws {
                if let Ok(entry) = ring.lookup_sc(&ccb, t) {
                    forbidden.push(entry.slot);
                }
            }
            forbidden.extend(held.iter().copied());
            match select_slot(&ring, &ccb, &recs, &mut scratch, pool) {
                Some(chosen) => assert!(
                    !forbidden.contains(&chosen),
                    "seq={seq} chose forbidden slot {chosen}"
                ),
                None => {
                    for slot in 0..pool_u32 {
                        assert!(
                            forbidden.contains(&slot),
                            "seq={seq} returned None with free slot {slot}"
                        );
                    }
                }
            }
        }
    }
}
