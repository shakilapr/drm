//! Channel runtime: publish transaction (channel-publish.md), overflow gating
//! (overflow-policies.md), cursors/delivery (cursor-delivery.md), pool
//! (payload-pool.md). REQ: REQ-CORE-050/051/052/053/049/100 (publish),
//! REQ-CORE-039/040/060/059 (overflow), REQ-CORE-011/012/013/028/029/096/097/003
//! (delivery).

pub mod ccb;
pub mod lifecycle;
pub mod overflow;
pub mod pattern;
pub mod pool;

pub use overflow::OverflowPolicy;

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::canon::{CanonLayout, validate_sample};
use crate::channel::ccb::ChannelControlBlock;
use crate::channel::overflow::{RequiredSequential, allow_commit};
use crate::channel::pool::{PoolLayout, slot_offset, write_checked};
use crate::hazard::{AcquireErr, ConsumerRec, acquire, bump_generation, select_slot};
use crate::ring::{HistoryRing, RingEntry};
use drm_types::Status;

/// Bounded retry budget for the ergonomic wrappers (cursor-delivery I-C10).
const RETRY_BUDGET: u32 = 4;

/// Activated channel: control block, history ring, payload pool, hazard words,
/// required-consumer registry. Storage is fixed at activation (zero data-path
/// allocation). REQ: REQ-CORE-045 (pool invariant), REQ-CORE-041 (latest default).
pub struct Channel {
    ccb: ChannelControlBlock,
    ring: HistoryRing,
    arena: Arena,
    gens: Box<[AtomicU64]>,
    policy: OverflowPolicy,
    required: Box<[RequiredSequential]>,
    recs: Box<[ConsumerRec]>,
    sample_max: u64,
    layout: Option<CanonLayout>,
    scratch: UnsafeCell<Vec<u32>>,
}

/// Payload pool byte arena + layout (in-process mirror of the mapped region).
struct Arena {
    bytes: UnsafeCell<Box<[u8]>>,
    layout: PoolLayout,
}

// SAFETY: slot bytes are written only while the publish busy guard is held
// (single writer), and read through hazard-protected transient views; the
// UnsafeCell is the shared-mapping mirror.
unsafe impl Sync for Arena {}

// SAFETY: every field is either an atomic, immutable after activation, or
// interior-mutable only under the `publish_busy` guard (`scratch`, `arena`
// bytes). The single-writer guard makes `&Channel` safe to share: multiple
// reader threads only run the copy/borrow read path, and at most one publisher
// mutates the guarded state at a time (I-HZ6).
unsafe impl Sync for Channel {}

/// Consumer cursor: process-local reader position.
/// REQ: REQ-CORE-011.
#[derive(Debug, Clone)]
pub struct ReaderState {
    observed_epoch: u32,
    last_read: Option<u64>,
}

/// Read outcome (cursor-delivery §Data Structures).
#[derive(Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// Owned copy; hazard released before return.
    Sample,
    /// Lapped: exact count of retired sequences since `last_read`.
    Gap {
        /// Sequences aged out between `last_read` and the oldest retained.
        missed: u64,
    },
    /// Nothing published yet / fully caught up.
    Empty,
    /// Bounded budget exhausted without a terminal outcome.
    WouldRetry,
}

impl Channel {
    /// Activation-time construction (control plane; storage fixed thereafter).
    /// REQ: REQ-CORE-045.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when the pool invariant `pool_size >= R + 1` fails
    /// or sizes exceed the `u32::MAX` slot index space.
    // Activation mirrors the immutable descriptor field list; narrowing the
    // signature below 8 parameters would obscure that 1:1 mapping.
    #[allow(clippy::too_many_arguments)]
    pub fn activate(
        depth: u64,
        pool_size: u64,
        sample_max: u64,
        slot_align: u64,
        policy: OverflowPolicy,
        required_count: usize,
        borrowers: usize,
        layout: Option<CanonLayout>,
    ) -> Result<Self, Status> {
        if pool_size < depth.checked_add(1).ok_or(Status::InvalidConfig)?
            || pool_size >= u64::from(u32::MAX)
            || sample_max == 0
            || depth == 0
        {
            return Err(Status::InvalidConfig);
        }
        let stride = align_up(sample_max, slot_align.max(1));
        let arena_bytes = stride.checked_mul(pool_size).ok_or(Status::InvalidConfig)?;
        let pool_layout = PoolLayout {
            base: 0,
            stride,
            count: pool_size,
            header_size: 0,
            sample_max,
            slot_align: slot_align.max(1),
        };
        Ok(Self {
            ccb: ChannelControlBlock::zeroed(),
            ring: HistoryRing::new(depth)?,
            arena: Arena {
                bytes: UnsafeCell::new(
                    vec![0_u8; usize::try_from(arena_bytes).map_err(|_| Status::InvalidConfig)?]
                        .into_boxed_slice(),
                ),
                layout: pool_layout,
            },
            gens: (0..pool_size).map(|_| AtomicU64::new(1)).collect(),
            policy,
            required: (0..required_count)
                .map(|_| RequiredSequential::new())
                .collect(),
            recs: (0..borrowers)
                .map(|i| ConsumerRec::new(u32::try_from(i + 1).unwrap_or(1)))
                .collect(),
            sample_max,
            layout,
            scratch: UnsafeCell::new(Vec::with_capacity(
                usize::try_from(pool_size).map_err(|_| Status::InvalidConfig)?,
            )),
        })
    }

    /// History depth R.
    #[must_use]
    pub fn depth(&self) -> u64 {
        self.ring.depth()
    }

    /// Ring view (hazard protocol tests and diagnostics).
    #[must_use]
    pub fn ring(&self) -> &HistoryRing {
        &self.ring
    }

    /// Control-block view (hazard protocol tests and diagnostics).
    #[must_use]
    pub fn ccb(&self) -> &ChannelControlBlock {
        &self.ccb
    }

    /// Slot generation counter (introspection accessor). Doc lint: `slot` is a `u32`
    /// index into the generation array.
    #[must_use]
    pub fn slot_generation(&self, slot: u32) -> u64 {
        self.gen_at(slot).load(Ordering::Relaxed)
    }

    /// Generation cell for `slot` (slot < `pool_size` is validated by selection).
    // Slot indices are u32 on the wire (HAZARD_NONE reserves u32::MAX); the index
    // narrowing is total for every validated pool size.
    #[allow(clippy::cast_possible_truncation)]
    fn gen_at(&self, slot: u32) -> &AtomicU64 {
        &self.gens[slot as usize]
    }

    /// Next sequence to publish.
    #[must_use]
    pub fn write_seq(&self) -> u64 {
        self.ccb.write_seq.load(Ordering::Acquire)
    }

    /// Publisher epoch.
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.ccb.publisher_epoch.load(Ordering::Acquire)
    }

    /// Pool size.
    #[must_use]
    pub fn pool_size(&self) -> u64 {
        self.arena.layout.count
    }

    /// Required-consumer record by index (records `0..required_count`).
    #[must_use]
    pub fn required(&self, i: usize) -> Option<&RequiredSequential> {
        self.required.get(i)
    }

    /// Consumer record by index.
    #[must_use]
    pub fn rec(&self, i: usize) -> Option<&ConsumerRec> {
        self.recs.get(i)
    }

    /// Publish busy flag view (single in-progress write; I-HZ6).
    #[must_use]
    pub fn publish_busy(&self) -> bool {
        self.ccb.publish_busy.load(Ordering::Acquire)
    }

    /// Pool-exhaustion drop counter (diagnostics; REQ-CORE-093).
    #[must_use]
    pub fn pool_exhausted(&self) -> u64 {
        self.ccb.pool_exhausted.load(Ordering::Relaxed)
    }

    /// Drop counter (diagnostics; REQ-CORE-093).
    #[must_use]
    pub fn drop_count(&self) -> u64 {
        self.ccb.drop_count.load(Ordering::Relaxed)
    }

    /// Overrun counter (diagnostics; REQ-CORE-093).
    #[must_use]
    pub fn overrun_count(&self) -> u64 {
        self.ccb.overrun_count.load(Ordering::Relaxed)
    }

    /// Oldest retained sequence for a snapshot `ws`.
    #[must_use]
    pub fn oldest_retained(&self, ws: u64) -> u64 {
        self.ring.oldest_retained(&self.ccb, ws)
    }

    /// Copy-form publish (channel-publish.md §Functions `publish`).
    /// REQ: REQ-CORE-052, REQ-CORE-051.
    ///
    /// # Errors
    /// Per the stable precedence table (I-PB4): `InvalidArg` → `Busy` →
    /// `SequenceExhausted` → `QueueFull`/`InvalidConfig` → `PoolExhausted` →
    /// counter exhaustion.
    pub fn publish(&self, src: &[u8]) -> Result<u64, Status> {
        match self.layout.as_ref() {
            Some(l) => validate_sample(src, l, src.len() as u64)?,
            None if src.len() as u64 > self.sample_max => return Err(Status::InvalidArg),
            None => {}
        }
        let _guard = self.try_publish_guard()?;
        let seq = self.ccb.write_seq.load(Ordering::Acquire);
        let next = seq.checked_add(1).ok_or(Status::SequenceExhausted)?;
        allow_commit(self.policy, &self.ccb, &self.required, self.depth())?;
        let Some(slot) = self.select() else {
            self.publish_exhausted();
            return Err(Status::PoolExhausted);
        };
        let generation = bump_generation(self.gen_at(slot).load(Ordering::Relaxed))?;
        let ring_stamp = self.ring.preflight_commit(seq)?;
        // SAFETY: busy guard held — this is the single in-progress write.
        write_checked(
            unsafe { &mut *self.arena.bytes.get() },
            &self.arena.layout,
            u64::from(slot),
            src,
        )?;
        self.gen_at(slot).store(generation, Ordering::Relaxed);
        self.ring.commit(seq, slot, generation, ring_stamp);
        self.ccb.write_seq.store(next, Ordering::Release);
        Ok(seq)
    }

    /// Loan-form acquire (steps 1-5 minus payload write; channel-publish.md).
    /// REQ: REQ-CORE-100.
    ///
    /// # Errors
    /// Same precedence as [`Channel::publish`].
    pub fn write_acquire(&self, requested: u64) -> Result<WriteLoan<'_>, Status> {
        if requested > self.sample_max {
            return Err(Status::InvalidArg);
        }
        let guard = self.try_publish_guard()?;
        let seq = self.ccb.write_seq.load(Ordering::Acquire);
        let next = seq.checked_add(1).ok_or(Status::SequenceExhausted)?;
        allow_commit(self.policy, &self.ccb, &self.required, self.depth())?;
        let Some(slot) = self.select() else {
            self.publish_exhausted();
            return Err(Status::PoolExhausted);
        };
        let generation = bump_generation(self.gen_at(slot).load(Ordering::Relaxed))?;
        Ok(WriteLoan {
            ch: self,
            guard,
            slot,
            seq,
            next,
            generation,
            capacity: requested,
        })
    }

    /// Loan commit (the only step touching `ring`/generation/`write_seq`); consumes the
    /// loan so the busy guard releases on every path.
    /// REQ: REQ-CORE-100, I-PB2.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] beyond the acquired capacity; [`Status::InvalidData`]
    /// on malformed canonical samples; [`Status::RingStampExhausted`].
    // Consuming the loan IS the release protocol; fields are read then dropped.
    #[allow(clippy::needless_pass_by_value)]
    pub fn write_commit(&self, loan: WriteLoan<'_>, sample_len: u64) -> Result<u64, Status> {
        if sample_len > loan.capacity {
            return Err(Status::InvalidArg);
        }
        if let Some(l) = self.layout.as_ref() {
            validate_sample(self.slot_view(loan.slot, l.sample_max), l, sample_len)?;
        }
        let ring_stamp = self.ring.preflight_commit(loan.seq)?;
        self.gen_at(loan.slot)
            .store(loan.generation, Ordering::Relaxed);
        self.ring
            .commit(loan.seq, loan.slot, loan.generation, ring_stamp);
        self.ccb.write_seq.store(loan.next, Ordering::Release);
        Ok(loan.seq)
    }

    /// Loan abort: releases the busy guard; the slot holds junk no sequence
    /// references. REQ: REQ-CORE-100. Consuming the loan IS the release protocol.
    #[allow(clippy::needless_pass_by_value)]
    pub fn write_abort(&self, loan: WriteLoan<'_>) {
        drop(loan);
    }

    /// Writable canonical sample buffer for a loan (fixed destination).
    /// REQ: REQ-CORE-100.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the slot offset or length leaves the arena.
    // SAFETY-backed shared-mapping mirror: the slice aliases no other live borrow
    // (single in-progress write, guarded above).
    #[allow(clippy::mut_from_ref)]
    pub fn loan_bytes(&self, loan: &WriteLoan<'_>) -> Result<&mut [u8], Status> {
        let off = slot_offset(&self.arena.layout, u64::from(loan.slot))?;
        let len = usize::try_from(loan.capacity).map_err(|_| Status::InvalidArg)?;
        // SAFETY: busy guard held exclusively by this loan; the slice aliases no
        // other live borrow (single in-progress write).
        let bytes = unsafe { &mut *self.arena.bytes.get() };
        let end = off.checked_add(len).ok_or(Status::InvalidArg)?;
        bytes.get_mut(off..end).ok_or(Status::InvalidArg)
    }

    /// Payload view of a committed entry's slot (transient borrow scope).
    /// REQ: REQ-CORE-003.
    #[must_use]
    pub fn payload(&self, entry: &RingEntry) -> &[u8] {
        self.slot_view(entry.slot, self.arena.layout.sample_max)
    }

    /// Publisher restart: epoch + 1, watermark advances, prior run unaddressable.
    /// REQ: REQ-CORE-019, REQ-CORE-026.
    pub fn restart_run(&self) -> u32 {
        let epoch = self.ccb.publisher_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let first_valid = self.ccb.write_seq.load(Ordering::Acquire);
        self.ring.reset_run(&self.ccb, first_valid);
        epoch
    }

    /// Consumer cursor: fresh reader state.
    /// REQ: REQ-CORE-011.
    #[must_use]
    pub fn reader(&self) -> ReaderState {
        ReaderState {
            observed_epoch: self.epoch(),
            last_read: None,
        }
    }

    /// Bounded `next()` (cursor-delivery `try_next`): copy-form delivery with exact
    /// gap accounting, epoch gates, and required-consumer acknowledgement after
    /// completion.
    /// REQ: REQ-CORE-013, REQ-CORE-028, REQ-CORE-097, REQ-CORE-001.
    ///
    /// # Errors
    /// [`Status::Retry`] family mapping per [`AcquireErr`]; [`Status::Busy`] when the
    /// endpoint record is closed.
    pub fn try_next(
        &self,
        state: &mut ReaderState,
        rec_index: usize,
        buf: &mut [u8],
        budget: u32,
    ) -> Result<ReadOutcome, Status> {
        self.observe_epoch_at_entry(state);
        let mut attempts = 0;
        while attempts < budget {
            attempts += 1;
            let epoch0 = self.epoch();
            let ws = self.write_seq();
            let fv = self.ccb.first_valid_seq.load(Ordering::Acquire);
            if self.epoch() != epoch0 {
                self.reset(state);
            } else if ws <= fv {
                return Ok(ReadOutcome::Empty);
            } else {
                let oldest = ws.saturating_sub(self.depth()).max(fv);
                let wanted = match state.last_read {
                    None => oldest,
                    Some(last) => last.checked_add(1).ok_or(Status::InvalidData)?,
                };
                if wanted < oldest {
                    let missed = oldest - wanted;
                    if self.epoch() == epoch0 {
                        state.last_read = Some(oldest - 1);
                        self.ccb.overrun_count.fetch_add(missed, Ordering::Relaxed);
                        return Ok(ReadOutcome::Gap { missed });
                    }
                    self.reset(state);
                } else if wanted >= ws {
                    if self.epoch() == epoch0 {
                        return Ok(ReadOutcome::Empty);
                    }
                    self.reset(state);
                } else {
                    match self.acquire_payload(wanted, rec_index, buf) {
                        Ok(()) => {
                            if self.epoch() == epoch0 {
                                state.last_read = Some(wanted);
                                if rec_index < self.required.len() {
                                    self.required[rec_index].ack_completed(wanted)?;
                                }
                                return Ok(ReadOutcome::Sample);
                            }
                            self.reset(state);
                        }
                        Err(AcquireErr::Lookup(_) | AcquireErr::Retry) => {}
                        Err(AcquireErr::Status(e)) => return Err(e),
                    }
                }
            }
        }
        Ok(ReadOutcome::WouldRetry)
    }

    /// Bounded `latest()` (cursor-delivery `try_latest`): newest committed sequence
    /// through the same acquisition primitive (no entry-passing side path).
    /// REQ: REQ-CORE-012.
    ///
    /// # Errors
    /// As [`Channel::try_next`].
    pub fn try_latest(
        &self,
        state: &mut ReaderState,
        rec_index: usize,
        buf: &mut [u8],
        budget: u32,
    ) -> Result<ReadOutcome, Status> {
        self.observe_epoch_at_entry(state);
        let mut attempts = 0;
        while attempts < budget {
            attempts += 1;
            let epoch0 = self.epoch();
            let ws = self.write_seq();
            let fv = self.ccb.first_valid_seq.load(Ordering::Acquire);
            if ws <= fv {
                if self.epoch() == epoch0 {
                    return Ok(ReadOutcome::Empty);
                }
                self.reset(state);
            } else {
                match self.acquire_payload(ws - 1, rec_index, buf) {
                    Ok(()) => {
                        if self.epoch() == epoch0 {
                            return Ok(ReadOutcome::Sample);
                        }
                        self.reset(state);
                    }
                    Err(AcquireErr::Lookup(_) | AcquireErr::Retry) => {}
                    Err(AcquireErr::Status(e)) => return Err(e),
                }
            }
        }
        Ok(ReadOutcome::WouldRetry)
    }

    /// Ergonomic wrapper: loops the bounded primitive (not the WCET surface).
    ///
    /// # Errors
    /// As [`Channel::try_next`].
    pub fn next(
        &self,
        state: &mut ReaderState,
        rec_index: usize,
        buf: &mut [u8],
    ) -> Result<ReadOutcome, Status> {
        loop {
            match self.try_next(state, rec_index, buf, RETRY_BUDGET)? {
                ReadOutcome::WouldRetry => {}
                done => return Ok(done),
            }
        }
    }

    /// Ergonomic `latest` wrapper.
    ///
    /// # Errors
    /// As [`Channel::try_latest`].
    pub fn latest(
        &self,
        state: &mut ReaderState,
        rec_index: usize,
        buf: &mut [u8],
    ) -> Result<ReadOutcome, Status> {
        loop {
            match self.try_latest(state, rec_index, buf, RETRY_BUDGET)? {
                ReadOutcome::WouldRetry => {}
                done => return Ok(done),
            }
        }
    }

    fn observe_epoch_at_entry(&self, state: &mut ReaderState) {
        let e = self.epoch();
        if e != state.observed_epoch {
            state.observed_epoch = e;
            state.last_read = None;
        }
    }

    fn reset(&self, state: &mut ReaderState) {
        state.last_read = None;
        state.observed_epoch = self.epoch();
    }

    /// Unified acquisition: hazard protocol + copy into caller memory (hazard
    /// released immediately for the copy form).
    fn acquire_payload(
        &self,
        wanted: u64,
        rec_index: usize,
        buf: &mut [u8],
    ) -> Result<(), AcquireErr> {
        let rec = self
            .recs
            .get(rec_index)
            .ok_or(AcquireErr::Status(Status::InvalidArg))?;
        let borrow = acquire(&self.ring, &self.ccb, wanted, rec)?;
        let payload = self.payload(&borrow.entry);
        if buf.len() < payload.len() {
            return Err(AcquireErr::Status(Status::InvalidArg));
        }
        buf[..payload.len()].copy_from_slice(payload);
        drop(borrow);
        Ok(())
    }

    fn select(&self) -> Option<u32> {
        // SAFETY: scratch is accessed only under the publish busy guard.
        let scratch = unsafe { &mut *self.scratch.get() };
        select_slot(&self.ring, &self.ccb, &self.recs, scratch, self.pool_size())
    }

    fn publish_exhausted(&self) {
        self.ccb.pool_exhausted.fetch_add(1, Ordering::Relaxed);
        self.ccb.drop_count.fetch_add(1, Ordering::Relaxed);
        self.ccb.reclaim_requested.store(true, Ordering::Release);
    }

    fn try_publish_guard(&self) -> Result<PublishGuard<'_>, Status> {
        self.ccb
            .publish_busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| PublishGuard {
                busy: &self.ccb.publish_busy,
            })
            .map_err(|_| Status::Busy)
    }

    fn slot_view(&self, slot: u32, len: u64) -> &[u8] {
        let off = slot_offset(&self.arena.layout, u64::from(slot)).unwrap_or(0);
        let len = usize::try_from(len).unwrap_or(0);
        // SAFETY: immutable view over the shared mapping mirror; the hazard protocol
        // guarantees the bytes are stable while a borrow is held.
        let bytes = unsafe { &*self.arena.bytes.get() };
        bytes.get(off..off + len).unwrap_or(&[])
    }
}

/// Publish busy guard: releases on drop (step 8).
struct PublishGuard<'a> {
    busy: &'a AtomicBool,
}

impl Drop for PublishGuard<'_> {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

/// Loan-form write reservation (channel-publish.md §Data Structures). Holding the
/// busy guard in the loan keeps the flag set until commit/abort drops it.
pub struct WriteLoan<'a> {
    ch: &'a Channel,
    #[expect(dead_code, reason = "guard lifetime carries the busy release on drop")]
    guard: PublishGuard<'a>,
    slot: u32,
    seq: u64,
    next: u64,
    generation: u64,
    capacity: u64,
}

impl<'a> WriteLoan<'a> {
    /// Reserved sequence.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Acquired capacity.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Owning channel view (commit/abort call sites).
    #[must_use]
    pub fn channel(&self) -> &'a Channel {
        self.ch
    }
}

/// Checked alignment round-up (I7 canonical arithmetic).
fn align_up(v: u64, align: u64) -> u64 {
    if align <= 1 {
        return v;
    }
    match v.checked_add(align - 1) {
        Some(x) => x - (x % align),
        None => v,
    }
}

#[cfg(test)]
mod tests {
    use super::{Channel, OverflowPolicy as Pol, ReadOutcome, Status};

    fn ch(policy: Pol, depth: u64, pool: u64, required: usize) -> Channel {
        Channel::activate(depth, pool, 16, 8, policy, required, 2, None)
            .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
    }

    fn pat(v: u8) -> [u8; 16] {
        [v; 16]
    }

    /// REQ: REQ-CORE-051 — copy publish round-trip; monotonic sequence.
    #[test]
    fn publish_read_roundtrip() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        assert_eq!(c.publish(&pat(1)), Ok(0));
        assert_eq!(c.publish(&pat(2)), Ok(1));
        let mut st = c.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(1));
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(2));
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
        assert_eq!(c.latest(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(2));
    }

    /// REQ: REQ-CORE-050 — failed publishes leave history unchanged (I-PB2).
    #[test]
    fn error_no_commit() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        assert!(c.publish(&pat(1)).is_ok());
        assert_eq!(c.publish(&[0_u8; 17]), Err(Status::InvalidArg));
        assert_eq!(c.write_seq(), 1);
        assert_eq!(c.publish(&pat(2)), Ok(1));
    }

    /// REQ: REQ-CORE-047 — concurrent publish guard rejects with Busy (I-HZ6).
    #[test]
    fn publish_busy_guard() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        let loan = c
            .write_acquire(16)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        assert_eq!(c.publish(&pat(1)), Err(Status::Busy));
        c.write_abort(loan);
        assert!(c.publish(&pat(1)).is_ok());
    }

    /// REQ: REQ-CORE-100 — loan commit publishes; abort and failed commit invisible.
    #[test]
    fn loan_commit_abort() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        let loan = c
            .write_acquire(16)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        assert_eq!(loan.seq(), 0);
        match c.loan_bytes(&loan) {
            Ok(b) => b.copy_from_slice(&pat(7)),
            Err(e) => unreachable!("loan: {e:?}"),
        }
        assert_eq!(c.write_commit(loan, 16), Ok(0));
        let mut st = c.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(c.latest(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(7));
        let loan2 = c
            .write_acquire(16)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        match c.loan_bytes(&loan2) {
            Ok(b) => b.copy_from_slice(&pat(9)),
            Err(e) => unreachable!("loan: {e:?}"),
        }
        c.write_abort(loan2);
        assert_eq!(c.write_seq(), 1);
        let loan3 = c
            .write_acquire(16)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        assert_eq!(c.write_commit(loan3, 17), Err(Status::InvalidArg));
        assert_eq!(c.write_seq(), 1);
    }

    /// REQ: REQ-CORE-046, REQ-CORE-044 — offsets static across remap; per-topic pools
    /// are separate instances (I-P1, I-P2).
    #[test]
    fn pool_offsets_static() {
        let a = ch(Pol::Overwrite, 2, 3, 0);
        let b = ch(Pol::Overwrite, 2, 3, 0);
        assert!(a.publish(&pat(1)).is_ok());
        assert!(a.publish(&pat(2)).is_ok());
        assert_eq!(b.pool_exhausted(), 0);
        let mut st = b.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(b.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
        assert!(a.publish(&pat(3)).is_ok());
    }

    /// REQ: REQ-CORE-045 — `pool_size < R + 1` refused at activation (I-P4).
    #[test]
    fn pool_invariant_min() {
        assert!(Channel::activate(4, 4, 16, 8, Pol::Overwrite, 0, 1, None).is_err());
        assert!(Channel::activate(4, 5, 16, 8, Pol::Overwrite, 0, 1, None).is_ok());
    }

    /// REQ: REQ-CORE-048 — oversize publish rejected before any write (I-P3).
    #[test]
    fn pool_size_reject_sentinel() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        assert!(c.publish(&pat(0xEE)).is_ok());
        assert_eq!(c.publish(&[0x11; 17]), Err(Status::InvalidArg));
        let mut st = c.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(c.latest(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(0xEE));
    }

    /// REQ: REQ-CORE-059, REQ-CORE-060 — reject gating: publish succeeds iff every
    /// required consumer has acknowledged past the retiring sequence (I-OV3).
    #[test]
    fn reject_gating() {
        let c = ch(Pol::Reject, 2, 3, 1);
        assert!(c.publish(&pat(1)).is_ok());
        assert!(c.publish(&pat(2)).is_ok());
        assert_eq!(c.publish(&pat(3)), Err(Status::QueueFull));
        let req = c.required(0).unwrap_or_else(|| unreachable!("req"));
        assert!(req.ack_completed(0).is_ok());
        assert!(c.publish(&pat(3)).is_ok());
        assert_eq!(c.publish(&pat(4)), Err(Status::QueueFull));
        assert!(req.ack_completed(1).is_ok());
        assert!(c.publish(&pat(4)).is_ok());
    }

    /// REQ: REQ-CORE-040 — `QueueFull` returns immediately, no blocking (I-OV2).
    #[test]
    fn reject_never_blocks() {
        let c = ch(Pol::Reject, 2, 3, 1);
        assert!(c.publish(&pat(1)).is_ok());
        assert!(c.publish(&pat(2)).is_ok());
        let t0 = std::time::Instant::now();
        let res = c.publish(&pat(3));
        assert!(t0.elapsed().as_millis() < 1, "took {:?}", t0.elapsed());
        assert_eq!(res, Err(Status::QueueFull));
    }

    /// REQ: REQ-CORE-040 — reject with zero required consumers is `InvalidConfig` on
    /// every publish (I-OV5; verifier rejects at build).
    #[test]
    fn reject_zero_required() {
        let c = ch(Pol::Reject, 2, 3, 0);
        assert_eq!(c.publish(&pat(1)), Err(Status::InvalidConfig));
    }

    /// REQ: REQ-CORE-028, REQ-CORE-029 — lap k (k ∈ 1..=32) yields Gap { missed: k }
    /// exactly, then the cursor resumes at the oldest retained sample (I-C2, I-C4).
    #[test]
    fn gap_exact_laps() {
        let mut buf = [0_u8; 16];
        for k in 1..=32_u64 {
            let c = ch(Pol::Overwrite, 4, 5, 0);
            for v in 0..6_u8 {
                assert!(c.publish(&pat(v)).is_ok(), "k={k} v={v}");
            }
            let mut st = c.reader();
            for v in 2..6_u8 {
                match c.next(&mut st, 0, &mut buf) {
                    Ok(ReadOutcome::Sample) => assert_eq!(buf, pat(v)),
                    other => unreachable!("k={k} want sample {v}, got {other:?}"),
                }
            }
            for v in 6_u8..(10_u8 + u8::try_from(k).unwrap_or(0)) {
                assert!(c.publish(&pat(v)).is_ok(), "k={k} v={v}");
            }
            assert_eq!(
                c.next(&mut st, 0, &mut buf),
                Ok(ReadOutcome::Gap { missed: k }),
                "k={k}"
            );
            assert_eq!(c.overrun_count(), k, "k={k}");
            match c.next(&mut st, 0, &mut buf) {
                Ok(ReadOutcome::Sample) => {
                    assert_eq!(buf, pat(6_u8 + u8::try_from(k).unwrap_or(0)));
                }
                other => unreachable!("k={k} want oldest, got {other:?}"),
            }
        }
    }

    /// REQ: REQ-CORE-011 — readers at different speeds never interact (I-C1).
    #[test]
    fn cursor_independence() {
        let c = ch(Pol::Overwrite, 8, 9, 0);
        for v in 0..6_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        let mut fast = c.reader();
        let mut slow = c.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(c.next(&mut fast, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(c.next(&mut slow, 1, &mut buf), Ok(ReadOutcome::Sample));
        for _ in 0..3 {
            assert_eq!(c.next(&mut fast, 0, &mut buf), Ok(ReadOutcome::Sample));
        }
        match c.next(&mut slow, 1, &mut buf) {
            Ok(ReadOutcome::Sample) => assert_eq!(buf, pat(1)),
            other => unreachable!("slow wants seq 1, got {other:?}"),
        }
    }

    /// REQ: REQ-CORE-011 — a lapped reader's `Gap` never perturbs a fast peer's
    /// cursor (I-C1): reader positions are process-local; no reader shares state.
    #[test]
    fn cursor_independence_lapped() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        for v in 0..4_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        let mut fast = c.reader();
        let mut held = c.reader();
        let mut buf = [0_u8; 16];
        for v in 0..4_u8 {
            assert_eq!(c.next(&mut fast, 0, &mut buf), Ok(ReadOutcome::Sample));
            assert_eq!(buf, pat(v));
        }
        assert_eq!(c.next(&mut held, 1, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(0));
        // Publisher advances 8 samples; `fast` keeps pace, `held` stalls and laps.
        for v in 4..12_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
            assert_eq!(c.next(&mut fast, 0, &mut buf), Ok(ReadOutcome::Sample));
            assert_eq!(buf, pat(v));
        }
        assert_eq!(
            c.next(&mut held, 1, &mut buf),
            Ok(ReadOutcome::Gap { missed: 7 }),
            "held laps seq 1..8 (oldest retained 8)"
        );
        // `held`'s Gap left `fast` untouched: it is simply caught up.
        assert_eq!(c.next(&mut fast, 0, &mut buf), Ok(ReadOutcome::Empty));
    }

    /// REQ: REQ-CORE-012 — `latest()` resolves the newest committed sequence
    /// regardless of backlog, independent of the `next()` cursor (I-C3).
    #[test]
    fn latest_newest() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        let mut buf = [0_u8; 16];
        let mut st = c.reader();
        assert_eq!(c.latest(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
        for v in 0..6_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        // `next` starts at the oldest retained backlog (seq 2 of 0..6)...
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(2));
        // ...while `latest` jumps to the newest committed sample (seq 5).
        assert_eq!(c.latest(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(5));
    }

    /// REQ: REQ-CORE-029 — after a `Gap`, `next()` resumes at the oldest retained
    /// sequence and continues in order; retained samples are never skipped (I-C4).
    #[test]
    fn next_resume_oldest() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        let mut buf = [0_u8; 16];
        for v in 0..2_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        let mut st = c.reader();
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(0));
        // Sequences 0..=3 age out (depth 2): oldest retained becomes 4.
        for v in 2..6_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        assert_eq!(
            c.next(&mut st, 0, &mut buf),
            Ok(ReadOutcome::Gap { missed: 3 })
        );
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(4));
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(5));
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
    }

    /// REQ: REQ-CORE-039 — overwrite lapping yields exact gaps across multiple
    /// laps (I-O1): for depth R, a cursor overrun by `lap` reports
    /// `Gap { missed: lap }`, exercised at laps 1, 2, R and R+1.
    #[test]
    fn overflow_laps() {
        const R: u64 = 3;
        let mut buf = [0_u8; 16];
        for lap in [1_u64, 2, R, R + 1] {
            let c = ch(Pol::Overwrite, R, R + 1, 0);
            assert!(c.publish(&pat(0)).is_ok(), "lap={lap}");
            let mut st = c.reader();
            assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
            assert_eq!(buf, pat(0));
            // Advance the publisher by R + lap: oldest retained moves `lap` ahead.
            for v in 1..=(R + lap) {
                let byte = u8::try_from(v).unwrap_or(0);
                assert!(c.publish(&pat(byte)).is_ok(), "lap={lap} v={v}");
            }
            assert_eq!(
                c.next(&mut st, 0, &mut buf),
                Ok(ReadOutcome::Gap { missed: lap }),
                "lap={lap}"
            );
            assert_eq!(c.overrun_count(), lap, "lap={lap}");
        }
    }

    /// REQ: REQ-CORE-039, REQ-CORE-040 — overflow combination matrix:
    /// {`Overwrite`, `Reject`} × {empty, not-full, full, required-behind,
    /// required-advanced}. Every cell asserts its exact outcome and is counted;
    /// the pinned total fails the test if a case is dropped.
    #[test]
    fn overflow_matrix() {
        let mut buf = [0_u8; 16];
        let mut cells = 0_u32;

        // 1. Overwrite, empty: no history.
        let c = ch(Pol::Overwrite, 2, 3, 0);
        let mut st = c.reader();
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
        assert_eq!(c.latest(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
        cells += 1;

        // 2. Overwrite, not full: ordered delivery.
        assert!(c.publish(&pat(1)).is_ok());
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(1));
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
        cells += 1;

        // 3. Overwrite, full and lapped: exact gap.
        let mut st2 = c.reader();
        assert_eq!(c.next(&mut st2, 0, &mut buf), Ok(ReadOutcome::Sample));
        for v in 2..6_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        assert_eq!(
            c.next(&mut st2, 0, &mut buf),
            Ok(ReadOutcome::Gap { missed: 2 })
        );
        cells += 1;

        // 4. Reject, zero required consumers: runtime invariant violation.
        let z = ch(Pol::Reject, 2, 3, 0);
        assert_eq!(z.publish(&pat(1)), Err(Status::InvalidConfig));
        cells += 1;

        // 5. Reject, full with a required consumer behind: refused, no block.
        let r = ch(Pol::Reject, 2, 3, 1);
        assert!(r.publish(&pat(1)).is_ok());
        assert!(r.publish(&pat(2)).is_ok());
        assert_eq!(r.publish(&pat(3)), Err(Status::QueueFull));
        cells += 1;

        // 6. Reject, required consumer advanced past retirement: publish proceeds.
        let req = r.required(0).unwrap_or_else(|| unreachable!("req"));
        assert!(req.ack_completed(0).is_ok());
        assert!(r.publish(&pat(3)).is_ok());
        cells += 1;

        assert_eq!(cells, 6, "overflow matrix cell count");
    }

    /// REQ: REQ-CORE-010 — restart invalidates acquired payloads; epoch observed at
    /// entry resets cursors (I-C6/I-C6b).
    #[test]
    fn epoch_restart() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        assert!(c.publish(&pat(1)).is_ok());
        assert!(c.publish(&pat(2)).is_ok());
        let mut st = c.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        c.restart_run();
        assert!(c.publish(&pat(9)).is_ok());
        assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
        assert_eq!(buf, pat(9));
    }

    /// REQ: REQ-CORE-004 — an actively borrowed slot is never overwritten (I-HZ1);
    /// with `pool_size = R + H + 1` the publisher still progresses (I-P5 drop-free).
    #[test]
    fn borrowed_slot_never_overwritten() {
        let c = ch(Pol::Overwrite, 2, 4, 0);
        assert!(c.publish(&pat(0xAA)).is_ok());
        let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
        let mut borrow = crate::hazard::acquire(c.ring(), c.ccb(), 0, rec)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        for v in 1..10_u8 {
            let res = c.publish(&pat(v));
            assert!(res.is_ok(), "v={v}: {res:?}");
        }
        assert_eq!(c.payload(&borrow.entry), pat(0xAA));
        borrow.release();
    }

    /// REQ: REQ-CORE-035 — second acquire on a busy endpoint returns Busy (I-HZ7).
    #[test]
    fn borrow_single_outstanding() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        assert!(c.publish(&pat(1)).is_ok());
        let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
        let first = crate::hazard::acquire(c.ring(), c.ccb(), 0, rec);
        assert!(first.is_ok());
        let second = crate::hazard::acquire(c.ring(), c.ccb(), 0, rec);
        assert!(matches!(
            second,
            Err(crate::hazard::AcquireErr::Status(Status::Busy))
        ));
        drop(first);
    }

    /// REQ: REQ-CORE-032 — multiple consumers sharing one latest sample are safe (I-HZ4);
    /// pool = R + H + 1 keeps publishes drop-free with both borrows held (I-P5).
    #[test]
    fn hazard_multi_reader() {
        let c = ch(Pol::Overwrite, 4, 7, 0);
        assert!(c.publish(&pat(0x42)).is_ok());
        let r0 = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
        let r1 = c.rec(1).unwrap_or_else(|| unreachable!("rec"));
        let b0 = crate::hazard::acquire(c.ring(), c.ccb(), 0, r0);
        let b1 = crate::hazard::acquire(c.ring(), c.ccb(), 0, r1);
        assert!(b0.is_ok() && b1.is_ok());
        for v in 1..12_u8 {
            assert!(c.publish(&pat(v)).is_ok(), "v={v}");
        }
        let e0 = b0.unwrap_or_else(|_| unreachable!("b0")).entry;
        let e1 = b1.unwrap_or_else(|_| unreachable!("b1")).entry;
        assert_eq!(c.payload(&e0), pat(0x42));
        assert_eq!(c.payload(&e1), pat(0x42));
    }

    /// REQ: REQ-CORE-002 — close invalidates future acquisitions (I-HZ8).
    #[test]
    fn close_invalidates() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        assert!(c.publish(&pat(1)).is_ok());
        let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
        rec.close();
        let res = crate::hazard::acquire(c.ring(), c.ccb(), 0, rec);
        assert!(matches!(
            res,
            Err(crate::hazard::AcquireErr::Status(Status::Busy))
        ));
    }

    /// REQ: REQ-CORE-031 — generation advance on slot reuse invalidates stale pairs
    /// (ABA guard; exhaustion-reclamation.md classifies the recovery path).
    #[test]
    fn generation_aba() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        assert!(c.publish(&pat(1)).is_ok());
        let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
        let mut b = crate::hazard::acquire(c.ring(), c.ccb(), 0, rec)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        let gen0 = b.entry.generation;
        b.release();
        for v in 2..9_u8 {
            assert!(c.publish(&pat(v)).is_ok());
        }
        let latest_entry = crate::hazard::acquire(c.ring(), c.ccb(), c.write_seq() - 1, rec)
            .unwrap_or_else(|e| unreachable!("acquire: {e:?}"));
        assert!(
            latest_entry.entry.generation > gen0,
            "generations must advance on reuse: {gen0} -> {}",
            latest_entry.entry.generation
        );
    }

    /// REQ: REQ-CORE-097 — bounded budget: adversarial advance yields `WouldRetry` (I-C10).
    #[test]
    fn read_bounded_retries() {
        let c = ch(Pol::Overwrite, 4, 5, 0);
        assert!(c.publish(&pat(1)).is_ok());
        let mut st = c.reader();
        let mut buf = [0_u8; 16];
        assert_eq!(
            c.try_next(&mut st, 0, &mut buf, 0),
            Ok(ReadOutcome::WouldRetry)
        );
        assert_eq!(c.try_next(&mut st, 0, &mut buf, 1), Ok(ReadOutcome::Sample));
    }

    /// REQ: REQ-CORE-049 — sequence exhaustion returns explicit status (I-PB5).
    #[test]
    fn publish_counter_exhaustion() {
        let c = ch(Pol::Overwrite, 2, 3, 0);
        c.ccb()
            .write_seq
            .store(u64::MAX, std::sync::atomic::Ordering::Release);
        assert_eq!(c.publish(&pat(1)), Err(Status::SequenceExhausted));
        assert_eq!(c.write_seq(), u64::MAX);
    }
}
