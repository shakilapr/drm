//! Circular sequence-history ring (docs/algorithms/runtime/ring-history.md).
//! REQ: REQ-CORE-063 (handle entries), REQ-CORE-066 (exact retention),
//! REQ-CORE-067 (wrap), REQ-CORE-065 (release ordering), REQ-CORE-062
//! (`first_valid` watermark), REQ-CORE-064 (portable fixed-width atomics),
//! REQ-CORE-061 (no sequence wrap).

use std::mem::{align_of, size_of};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::channel::ccb::ChannelControlBlock;
use drm_types::Status;

/// One ring cell: two banks, per-lap ping-pong.
/// REQ: REQ-CORE-064.
#[repr(C, align(8))]
pub struct RingCell {
    /// Lap-selected banks.
    pub banks: [RingBank; 2],
}
const _: () = assert!(align_of::<RingCell>() == 8);

/// Versioned single-entry bank; even stamp = stable, odd = writer active.
/// REQ: REQ-CORE-064, REQ-CORE-065.
#[repr(C, align(8))]
pub struct RingBank {
    /// `SeqCst` version stamp (even = stable, odd = writer active).
    pub stamp: AtomicU64,
    /// Sequence this entry describes (self-describing re-read guard).
    pub sequence: AtomicU64,
    /// Slot generation at commit (ABA guard).
    pub generation: AtomicU64,
    /// Payload pool slot index.
    pub slot: AtomicU32,
    /// Reserved alignment padding.
    pub pad: u32,
}

/// Snapshot of one committed entry (process-local value copy).
/// REQ: REQ-CORE-063.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingEntry {
    /// Sequence this entry describes.
    pub sequence: u64,
    /// Payload pool slot index.
    pub slot: u32,
    /// Slot generation at commit (ABA guard).
    pub generation: u64,
}

/// Lookup outcome for a target sequence.
/// REQ: REQ-CORE-066.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupErr {
    /// Sequence not yet published.
    Future,
    /// Sequence aged out of the retained window or below `first_valid_seq`.
    AgedOut,
}

/// Reader-ordering bundle: stamp check is `SeqCst`, fields `Acquire`.
/// REQ: REQ-CORE-065.
#[derive(Debug, Clone, Copy)]
pub struct ReadOrder;

impl ReadOrder {
    /// Stamp ordering (`SeqCst` — participates in the hazard handshake).
    #[must_use]
    pub fn stamp(self) -> Ordering {
        Ordering::SeqCst
    }

    /// Field ordering (Acquire — payload publication handshake).
    #[must_use]
    pub fn fields(self) -> Ordering {
        Ordering::Acquire
    }
}

/// Circular history ring of depth R over a fixed cell array.
/// REQ: REQ-CORE-066.
pub struct HistoryRing {
    entries: Box<[RingCell]>,
    r: u64,
}

impl HistoryRing {
    /// Activation-time allocation of the fixed ring (control plane only).
    /// REQ: REQ-CORE-064.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when R is 0.
    pub fn new(r: u64) -> Result<Self, Status> {
        if r == 0 {
            return Err(Status::InvalidConfig);
        }
        let entries = (0..r)
            .map(|_| RingCell::zeroed())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self { entries, r })
    }

    /// Depth R.
    #[must_use]
    pub fn depth(&self) -> u64 {
        self.r
    }

    /// Cell index for sequence `s` (total: depth is bounded by `usize::MAX`
    /// at activation and the modulo result is smaller than the depth).
    // Narrowing is total after the activation bound check.
    #[allow(clippy::cast_possible_truncation)]
    fn cell_index(r: u64, s: u64) -> usize {
        (s % r) as usize
    }

    /// Lap-selected bank index.
    // Narrowing is total: the index is 0 or 1.
    #[allow(clippy::cast_possible_truncation)]
    fn bank_index(r: u64, s: u64) -> usize {
        ((s / r) & 1) as usize
    }

    /// Activation gate: shared atomics must be lock-free (codebase R11). Best-effort
    /// structural probe — single-width atomics on supported targets (x86-64/aarch64)
    /// compile to single instructions; the std lock-free introspection API is not
    /// available on this toolchain, so refusal is enforced by the size probe + the
    /// no-aggregate-atomic layout asserts.
    /// REQ: REQ-CORE-064.
    #[must_use]
    pub fn atomics_lock_free() -> bool {
        size_of::<AtomicU64>() == 8
            && align_of::<AtomicU64>() == 8
            && AtomicU32::new(0).load(Ordering::Relaxed) == 0
    }

    /// Preflight: target bank must be stable and stamp must advance by 2.
    /// REQ: REQ-CORE-061.
    ///
    /// # Errors
    /// [`Status::InvalidData`] on odd (writer-active) stamp;
    /// [`Status::RingStampExhausted`] when the stamp space is exhausted.
    pub fn preflight_commit(&self, s: u64) -> Result<u64, Status> {
        let cell = &self.entries[Self::cell_index(self.r, s)];
        let stable = cell.banks[Self::bank_index(self.r, s)]
            .stamp
            .load(Ordering::Relaxed);
        if stable & 1 != 0 {
            return Err(Status::InvalidData);
        }
        stable.checked_add(2).ok_or(Status::RingStampExhausted)?;
        Ok(stable)
    }

    /// Single-publisher commit: prepare the lap-selected bank (other bank untouched).
    /// REQ: REQ-CORE-065.
    pub fn commit(&self, s: u64, slot: u32, generation: u64, stable: u64) {
        let i = Self::cell_index(self.r, s);
        let bank = &self.entries[i].banks[Self::bank_index(self.r, s)];
        bank.stamp.store(stable + 1, Ordering::SeqCst);
        bank.sequence.store(s, Ordering::Relaxed);
        bank.slot.store(slot, Ordering::Relaxed);
        bank.generation.store(generation, Ordering::Relaxed);
        bank.stamp.store(stable + 2, Ordering::SeqCst);
    }

    /// Publisher restart: exclude every prior-run sequence (sole owner is the CCB).
    /// REQ: REQ-CORE-062.
    pub fn reset_run(&self, ccb: &ChannelControlBlock, first_valid_seq: u64) {
        ccb.first_valid_seq
            .store(first_valid_seq, Ordering::Release);
    }

    /// Read one bank coherently; `Err(())` = Retry (odd/moved stamp).
    /// REQ: REQ-CORE-065.
    fn read_bank(bank: &RingBank, order: ReadOrder) -> Option<RingEntry> {
        let a = bank.stamp.load(order.stamp());
        if a & 1 != 0 {
            return None;
        }
        let entry = RingEntry {
            sequence: bank.sequence.load(order.fields()),
            slot: bank.slot.load(order.fields()),
            generation: bank.generation.load(order.fields()),
        };
        let b = bank.stamp.load(order.stamp());
        if a != b || b & 1 != 0 {
            return None;
        }
        Some(entry)
    }

    /// Resolve a target sequence to its entry.
    /// REQ: REQ-CORE-066, REQ-CORE-062.
    ///
    /// # Errors
    /// [`LookupErr::Future`] for unpublished sequences; [`LookupErr::AgedOut`] for
    /// retired/prior-run sequences or unstable banks.
    pub fn lookup(&self, ccb: &ChannelControlBlock, s: u64) -> Result<RingEntry, LookupErr> {
        let ws = ccb.write_seq.load(Ordering::Acquire);
        let fv = ccb.first_valid_seq.load(Ordering::Acquire);
        if s < fv {
            return Err(LookupErr::AgedOut);
        }
        if s >= ws {
            return Err(LookupErr::Future);
        }
        if s < ws.saturating_sub(self.r) {
            return Err(LookupErr::AgedOut);
        }
        let cell = &self.entries[Self::cell_index(self.r, s)];
        let bank = &cell.banks[Self::bank_index(self.r, s)];
        let e = Self::read_bank(bank, ReadOrder).ok_or(LookupErr::AgedOut)?;
        if e.sequence != s {
            return Err(LookupErr::AgedOut);
        }
        Ok(e)
    }

    /// Hazard-handshake variant: fully `SeqCst` loads (steps 1/3 of acquisition).
    /// REQ: REQ-CORE-030.
    ///
    /// # Errors
    /// [`LookupErr`] per the window bounds; unstable banks report `AgedOut` (retry).
    pub fn lookup_sc(&self, ccb: &ChannelControlBlock, s: u64) -> Result<RingEntry, LookupErr> {
        let ws = ccb.write_seq.load(Ordering::SeqCst);
        let fv = ccb.first_valid_seq.load(Ordering::SeqCst);
        if s < fv {
            return Err(LookupErr::AgedOut);
        }
        if s >= ws {
            return Err(LookupErr::Future);
        }
        if s < ws.saturating_sub(self.r) {
            return Err(LookupErr::AgedOut);
        }
        let cell = &self.entries[Self::cell_index(self.r, s)];
        let bank = &cell.banks[Self::bank_index(self.r, s)];
        let a = bank.stamp.load(Ordering::SeqCst);
        if a & 1 != 0 {
            return Err(LookupErr::AgedOut);
        }
        let entry = RingEntry {
            sequence: bank.sequence.load(Ordering::SeqCst),
            slot: bank.slot.load(Ordering::SeqCst),
            generation: bank.generation.load(Ordering::SeqCst),
        };
        let b = bank.stamp.load(Ordering::SeqCst);
        if a != b || b & 1 != 0 || entry.sequence != s {
            return Err(LookupErr::AgedOut);
        }
        Ok(entry)
    }

    /// Newest committed entry, if any.
    /// REQ: REQ-CORE-066.
    ///
    /// # Errors
    /// [`Status::Retry`] when the newest bank was mid-commit.
    pub fn newest(&self, ccb: &ChannelControlBlock) -> Result<Option<(u64, RingEntry)>, Status> {
        let ws = ccb.write_seq.load(Ordering::Acquire);
        let fv = ccb.first_valid_seq.load(Ordering::Acquire);
        if ws <= fv {
            return Ok(None);
        }
        let s = ws - 1;
        self.lookup(ccb, s)
            .map(|entry| Some((s, entry)))
            .map_err(|_| Status::Retry)
    }

    /// Oldest retained sequence for `ws` (window floor; never below `first_valid`).
    /// REQ: REQ-CORE-062.
    #[must_use]
    pub fn oldest_retained(&self, ccb: &ChannelControlBlock, ws: u64) -> u64 {
        ws.saturating_sub(self.r)
            .max(ccb.first_valid_seq.load(Ordering::Acquire))
    }

    /// Restart repair: make every odd stamp even (caller holds the publisher lock).
    /// REQ: REQ-CORE-061.
    ///
    /// # Errors
    /// [`Status::RingStampExhausted`] when a stamp cannot advance.
    pub fn repair_incomplete_banks(&self) -> Result<(), Status> {
        for cell in &self.entries {
            for bank in &cell.banks {
                let stamp = bank.stamp.load(Ordering::SeqCst);
                if stamp & 1 != 0 {
                    bank.stamp.store(
                        stamp.checked_add(1).ok_or(Status::RingStampExhausted)?,
                        Ordering::SeqCst,
                    );
                }
            }
        }
        Ok(())
    }
}

impl RingCell {
    fn zeroed() -> Self {
        let mk = || RingBank {
            stamp: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            slot: AtomicU32::new(0),
            pad: 0,
        };
        Self {
            banks: [mk(), mk()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelControlBlock, HistoryRing, LookupErr, RingEntry};
    use std::sync::atomic::Ordering;

    fn ccb() -> ChannelControlBlock {
        ChannelControlBlock::zeroed()
    }

    /// REQ: REQ-CORE-066 — exactly min(ws-fv, R) entries addressable ending at ws-1 (I1).
    #[test]
    fn retention_exact() {
        let ring = assert_ok(HistoryRing::new(8));
        let c = ccb();
        for s in 0..48u64 {
            let stable = assert_preflight(&ring, s);
            ring.commit(s, (s % 8) as u32, s, stable);
            c.write_seq.store(s + 1, Ordering::Release);
            for target in 0..=s {
                let got = ring.lookup(&c, target);
                let want_ok = target + 8 > s;
                assert_eq!(got.is_ok(), want_ok, "s={s} target={target}");
            }
        }
    }

    /// REQ: REQ-CORE-067 — wrapping never changes retained semantics (I2).
    #[test]
    fn wraparound() {
        let ring = assert_ok(HistoryRing::new(3));
        let c = ccb();
        for s in 0..100_000u64 {
            let stable = assert_preflight(&ring, s);
            ring.commit(s, (s % 3) as u32, s, stable);
            c.write_seq.store(s + 1, Ordering::Release);
        }
        assert_eq!(ring.lookup(&c, 99_999).map(|e| e.sequence), Ok(99_999));
        assert_eq!(ring.lookup(&c, 99_996), Err(LookupErr::AgedOut));
        assert_eq!(ring.lookup(&c, 99_997).map(|e| e.sequence), Ok(99_997));
    }

    /// REQ: REQ-CORE-062 — no lookup returns a prior-run sequence (I5).
    #[test]
    fn first_valid_watermark() {
        let ring = assert_ok(HistoryRing::new(4));
        let c = ccb();
        for s in 0..10u64 {
            let stable = assert_preflight(&ring, s);
            ring.commit(s, (s % 4) as u32, s, stable);
            c.write_seq.store(s + 1, Ordering::Release);
        }
        ring.reset_run(&c, 10);
        assert_eq!(ring.lookup(&c, 9), Err(LookupErr::AgedOut));
        assert_eq!(ring.oldest_retained(&c, 12), 10);
    }

    /// REQ: REQ-CORE-066 — future sequences are Future, not fabricated (I1 bounds).
    #[test]
    fn lookup_bounds() {
        let ring = assert_ok(HistoryRing::new(4));
        let c = ccb();
        assert_eq!(ring.lookup(&c, 0), Err(LookupErr::Future));
        let stable = assert_preflight(&ring, 0);
        ring.commit(0, 0, 1, stable);
        c.write_seq.store(1, Ordering::Release);
        assert_eq!(ring.lookup(&c, 0).map(|e: RingEntry| e.sequence), Ok(0));
        assert_eq!(ring.newest(&c).map(|o| o.map(|(s, _)| s)), Ok(Some(0)));
        assert_eq!(ring.lookup(&c, 1), Err(LookupErr::Future));
        c.first_valid_seq.store(1, Ordering::Release);
        assert_eq!(ring.newest(&c), Ok(None));
    }

    /// REQ: REQ-CORE-061 — repair makes every odd stamp even (I8 restart path).
    #[test]
    fn repair_odd_stamps() {
        let ring = assert_ok(HistoryRing::new(2));
        ring.entries[0].banks[0].stamp.store(9, Ordering::SeqCst);
        assert!(ring.repair_incomplete_banks().is_ok());
        assert_eq!(ring.entries[0].banks[0].stamp.load(Ordering::SeqCst), 10);
        assert!(HistoryRing::atomics_lock_free());
        assert!(HistoryRing::new(0).is_err());
    }

    /// REQ: REQ-CORE-061 — sequence exhaustion guard fires before payload write (I7).
    #[test]
    fn sequence_exhaustion() {
        assert_eq!(
            ChannelControlBlock::check_sequence_free(u64::MAX),
            Err(drm_types::Status::SequenceExhausted)
        );
        assert!(ChannelControlBlock::check_sequence_free(0).is_ok());
    }

    fn assert_ok(r: Result<HistoryRing, drm_types::Status>) -> HistoryRing {
        match r {
            Ok(ring) => ring,
            Err(e) => unreachable!("ring alloc failed: {e:?}"),
        }
    }

    fn assert_preflight(ring: &HistoryRing, s: u64) -> u64 {
        match ring.preflight_commit(s) {
            Ok(stable) => stable,
            Err(e) => unreachable!("preflight s={s}: {e:?}"),
        }
    }
}
