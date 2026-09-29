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
