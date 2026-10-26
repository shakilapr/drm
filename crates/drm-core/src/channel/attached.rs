//! Attached-channel data operations
//! ([attached-data-path.md](../../../docs/algorithms/runtime/attached-data-path.md),
//! ADR [0007](../../../docs/adr/0007-cross-process-channel-data-operations.md)).
//!
//! Publishing into, and reading from, a segment created by **another process**.
//! The reserved CCB, ring, slot-generation, and payload regions are addressed
//! through the attachment's validated offsets ([`AttachedView`]), and the
//! algorithm is the one the in-process [`Channel`](super::Channel) already runs:
//! the same `select_slot`/`allow_commit`/`preflight_commit`/`commit` helpers and
//! the same ring protocol, not a second implementation (I-AT6).
//!
//! Scope of this slice (recorded in the algorithm doc):
//! - **Copy form only.** No zero-copy borrow is handed out across the
//!   attachment; readers copy into caller memory. External I/O must copy anyway
//!   (AGENTS.md #5), and a language bridge is exactly that caller.
//! - **No consumer hazard registration.** A copying reader is guarded by the
//!   ring's `SeqCst` version stamps: the bank is re-read after the copy and a
//!   torn read is retried (`I-AT3` covers the borrow form, tracked separately).
//! - **Single writer** is enforced by the publisher authority the segment's
//!   lock already carries ([`publisher_open`](super::lifecycle::publisher_open));
//!   this type never acquires authority by itself (I-AT2).
//!
//! REQ: REQ-CORE-102, REQ-CORE-103, REQ-CORE-104, REQ-CORE-105, REQ-CORE-106.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::ptr::NonNull;

use super::{ReadOutcome, ReaderState};
use crate::channel::ccb::{AttachedView, ChannelControlBlock};
use crate::channel::overflow::allow_commit;
use crate::channel::pool::{PoolLayout, slot_offset, write_checked};
use crate::hazard::{bump_generation, select_slot};
use crate::ring::{HistoryRing, LookupErr, RingCell, RingEntry};
use crate::shm::Mapping;
use drm_types::Status;

/// Bounded retry budget for one read attempt (matches the in-process path).
const RETRY_BUDGET: u32 = 8;

/// A data-operations view over an attached segment.
///
/// Holds no ownership of the segment: the mapping stays alive in the caller (the
/// [`Publisher`](super::lifecycle::Publisher) or
/// [`Consumer`](super::lifecycle::Consumer) it came from).
pub struct AttachedChannel<'m> {
    /// Mapping base (page-aligned; region offsets are 8-aligned).
    base: NonNull<u8>,
    /// Mapped length, for the one-time region bounds check.
    len: usize,
    /// Validated region offsets.
    view: AttachedView,
    /// Slot stride (aligned payload capacity).
    stride: u64,
    /// Slot alignment the segment was laid out with.
    slot_align: u64,
    /// Keeps the mapping borrowed for the lifetime of this view.
    _mapping: &'m Mapping,
}

/// One-shot region check: the mapping must cover the reserved payload pool.
fn region_end(view: &AttachedView, stride: u64) -> Option<u64> {
    let pool_size = view.descriptor.pool_size;
    let payload_offset = view.payload_offset;
    payload_offset.checked_add(pool_size.checked_mul(stride)?)
}

impl<'m> AttachedChannel<'m> {
    /// Binds the data operations to an attachment.
    ///
    /// `slot_align` must be the alignment the segment was laid out with (the
    /// `LayoutSpec` passed to `publisher_open`/`consumer_open`); it is not part of
    /// the descriptor, so it is *proved* here rather than trusted: the stride it
    /// implies must reproduce the descriptor's `segment_size` exactly. A wrong
    /// alignment changes the stride, which shifts every payload slot after the
    /// first — a reader would then hand back another sample's bytes as a valid
    /// sample instead of failing (I-AT1).
    ///
    /// # Errors
    /// [`Status::InvalidData`] when the mapping is smaller than the layout the
    /// descriptor describes, or when `slot_align` does not reproduce the
    /// descriptor's segment size.
    /// REQ: REQ-CORE-102, REQ-CORE-103.
    pub fn new(mapping: &'m Mapping, view: &AttachedView, slot_align: u64) -> Result<Self, Status> {
        let slot_align = slot_align.max(1);
        let stride = stride_of(view.descriptor.payload_capacity, slot_align);
        let end = region_end(view, stride).ok_or(Status::InvalidConfig)?;
        if end != view.descriptor.segment_size {
            return Err(Status::InvalidData);
        }
        if u64::try_from(mapping.len()).unwrap_or(u64::MAX) < end {
            return Err(Status::InvalidData);
        }
        Ok(Self {
            base: mapping.ptr(),
            len: mapping.len(),
            view: view.clone(),
            stride,
            slot_align,
            _mapping: mapping,
        })
    }

    /// Binds the data operations to an attachment without being told the
    /// alignment: the stride is recovered from the descriptor's own
    /// `segment_size` (`(segment_size - payload_offset) / pool_size`), so a
    /// caller that does not know the deployment's `slot_align` — every
    /// non-Rust binding — still addresses the right slots.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when the descriptor's sizes do not divide
    /// evenly or the recovered stride is not an alignment of the payload
    /// capacity, and [`Status::InvalidData`] when the mapping is smaller than
    /// the layout.
    /// REQ: REQ-CORE-102, REQ-CORE-103.
    pub fn derive(mapping: &'m Mapping, view: &AttachedView) -> Result<Self, Status> {
        let descriptor = &view.descriptor;
        let pool_bytes = descriptor
            .segment_size
            .checked_sub(view.payload_offset)
            .ok_or(Status::InvalidConfig)?;
        if descriptor.pool_size == 0 || pool_bytes % descriptor.pool_size != 0 {
            return Err(Status::InvalidConfig);
        }
        let stride = pool_bytes / descriptor.pool_size;
        // The layout aligns the capacity up to `slot_align`; recover the
        // alignment the stride implies rather than assuming one.
        let capacity = descriptor.payload_capacity.max(1);
        let slot_align = (1..=8)
            .find(|align| stride_of(capacity, *align) == stride)
            .ok_or(Status::InvalidConfig)?;
        let end = region_end(view, stride).ok_or(Status::InvalidConfig)?;
        if end != descriptor.segment_size {
            return Err(Status::InvalidData);
        }
        if u64::try_from(mapping.len()).unwrap_or(u64::MAX) < end {
            return Err(Status::InvalidData);
        }
        Ok(Self {
            base: mapping.ptr(),
            len: mapping.len(),
            view: view.clone(),
            stride,
            slot_align,
            _mapping: mapping,
        })
    }

    /// Attached CCB: shared atomics over the mapped reservation.
    /// REQ: REQ-CORE-102.
    fn ccb(&self) -> &ChannelControlBlock {
        // SAFETY: `new` established that the mapping covers every region of the
        // descriptor's layout; `ccb_offset` is 8-aligned by `segment_layout` and
        // `ChannelControlBlock` is `repr(C, align(8))` with compile-time offset
        // asserts, so this cast is valid for the reservation's lifetime.
        unsafe { &*self.typed_at::<ChannelControlBlock>(self.view.ccb_offset) }
    }

    /// Typed pointer at a validated region offset.
    ///
    /// Alignment is a segment-layout invariant, not a runtime check:
    /// `segment_layout` aligns every region offset to 8, and every region pointee
    /// (`ChannelControlBlock`, `RingCell`, `AtomicU64`) has alignment ≤ 8.
    #[allow(
        clippy::cast_ptr_alignment,
        reason = "mapping base is page-aligned and every region offset is 8-aligned by segment_layout; pointees are 8-aligned or less"
    )]
    fn typed_at<T>(&self, offset: u64) -> *mut T {
        self.at(offset).cast::<T>()
    }

    /// Attached history ring (a view over the mapped cells).
    /// REQ: REQ-CORE-102, REQ-CORE-103.
    fn ring(&self) -> Result<HistoryRing, Status> {
        let depth = usize::try_from(self.view.descriptor.history_depth)
            .map_err(|_| Status::InvalidConfig)?;
        let cells = self.typed_at::<RingCell>(self.view.ring_offset);
        // SAFETY: `cells` addresses the ring reservation whose size
        // `segment_layout` derived as `history_depth * size_of::<RingCell>()`.
        HistoryRing::from_mapped(unsafe { NonNull::new_unchecked(cells) }, depth)
    }

    /// Slot generation counters.
    /// REQ: REQ-CORE-102.
    fn generations(&self) -> *mut AtomicU64 {
        self.typed_at::<AtomicU64>(self.view.slot_generation_offset)
    }

    /// Pool layout over the mapped payload region.
    /// REQ: REQ-CORE-102, REQ-CORE-103.
    fn pool_layout(&self) -> PoolLayout {
        PoolLayout {
            base: 0,
            stride: self.stride,
            count: self.view.descriptor.pool_size,
            header_size: 0,
            sample_max: self.view.descriptor.payload_capacity,
            slot_align: self.slot_align,
        }
    }

    /// Payload bytes for `slot` (read side).
    fn payload(&self, entry: &RingEntry) -> Result<&[u8], Status> {
        let layout = self.pool_layout();
        let off = slot_offset(&layout, u64::from(entry.slot))?;
        let len = usize::try_from(layout.sample_max).map_err(|_| Status::InvalidConfig)?;
        let start = usize::try_from(
            self.view
                .payload_offset
                .checked_add(u64::try_from(off).map_err(|_| Status::InvalidConfig)?)
                .ok_or(Status::InvalidConfig)?,
        )
        .map_err(|_| Status::InvalidConfig)?;
        let end = start.checked_add(len).ok_or(Status::InvalidConfig)?;
        if end > self.len {
            return Err(Status::InvalidData);
        }
        // SAFETY: `start..end` lies inside the mapping (`new` checked the pool
        // end and `off` is slot-bounded by `slot_offset`).
        Ok(unsafe { core::slice::from_raw_parts(self.base.as_ptr().add(start), len) })
    }

    /// Raw pointer at a validated region offset.
    fn at(&self, offset: u64) -> *mut u8 {
        // `offset` is a validated attachment offset; `new` checked the mapping
        // covers the whole layout, so the sum stays inside the mapping.
        let base = self.base.as_ptr();
        // SAFETY: wrapping arithmetic on a mapped allocation; the result is only
        // dereferenced through the typed accessors above after their own bounds
        // arguments.
        unsafe { base.add(usize::try_from(offset).unwrap_or(0)) }
    }

    /// Sequence number the next publish will take.
    /// REQ: REQ-CORE-102.
    #[must_use]
    pub fn write_seq(&self) -> u64 {
        self.ccb().write_seq.load(Ordering::Acquire)
    }

    /// Published epoch (bumped on publisher restart; I-AT5).
    /// REQ: REQ-CORE-106.
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.ccb().publisher_epoch.load(Ordering::Acquire)
    }

    /// History depth R.
    /// REQ: REQ-CORE-103.
    #[must_use]
    pub fn depth(&self) -> u64 {
        self.view.descriptor.history_depth
    }

    /// Fresh reader state bound to the attachment's current epoch.
    ///
    /// The copy-form read API takes caller-held cursor state (there is no
    /// per-consumer record registration in this slice), so a reader keeps one of
    /// these per logical stream.
    /// REQ: REQ-CORE-103.
    #[must_use]
    pub fn reader(&self) -> ReaderState {
        ReaderState {
            last_read: None,
            observed_epoch: self.epoch(),
        }
    }

    /// Publishes `src` (copy form), returning the sequence it took.
    ///
    /// # Errors
    /// Same precedence table as [`Channel::publish`](super::Channel::publish):
    /// `InvalidArg` → `Busy` → `SequenceExhausted` → `InvalidConfig` →
    /// `PoolExhausted` → counter exhaustion.
    /// REQ: REQ-CORE-102, REQ-CORE-104.
    pub fn publish(&self, src: &[u8]) -> Result<u64, Status> {
        let ccb = self.ccb();
        if u64::try_from(src.len()).unwrap_or(u64::MAX) > self.view.descriptor.payload_capacity {
            return Err(Status::InvalidArg);
        }
        let _guard = BusyGuard::acquire(&ccb.publish_busy)?;
        let seq = ccb.write_seq.load(Ordering::Acquire);
        let next = seq.checked_add(1).ok_or(Status::SequenceExhausted)?;
        // Attached segments carry no required-acknowledgement floors yet, so a
        // `reject` channel refuses to publish rather than silently overwriting.
        allow_commit(self.policy(), ccb, &[], self.depth())?;
        let ring = self.ring()?;
        let mut scratch: Vec<u32> =
            Vec::with_capacity(usize::try_from(self.view.descriptor.pool_size).unwrap_or(0));
        let Some(slot) = select_slot(
            &ring,
            ccb,
            &[],
            &mut scratch,
            self.view.descriptor.pool_size,
        ) else {
            ccb.pool_exhausted.fetch_add(1, Ordering::Relaxed);
            ccb.drop_count.fetch_add(1, Ordering::Relaxed);
            ccb.reclaim_requested.store(true, Ordering::Release);
            return Err(Status::PoolExhausted);
        };
        // SAFETY: `slot < pool_size` (ring selection) and the generation region
        // holds exactly `pool_size` counters.
        let generation = bump_generation(unsafe {
            (*self.generations().add(usize::try_from(slot).unwrap_or(0))).load(Ordering::Relaxed)
        })?;
        let ring_stamp = ring.preflight_commit(seq)?;
        let layout = self.pool_layout();
        let pool_len = usize::try_from(
            layout
                .count
                .checked_mul(layout.stride)
                .ok_or(Status::InvalidConfig)?,
        )
        .map_err(|_| Status::InvalidConfig)?;
        let start = usize::try_from(self.view.payload_offset).map_err(|_| Status::InvalidConfig)?;
        let end = start.checked_add(pool_len).ok_or(Status::InvalidConfig)?;
        if end > self.len {
            return Err(Status::InvalidData);
        }
        // SAFETY: the pool region is inside the mapping and the publish busy
        // guard makes this the single in-progress write (I-AT2).
        let pool: &mut [u8] =
            unsafe { core::slice::from_raw_parts_mut(self.base.as_ptr().add(start), pool_len) };
        write_checked(pool, &layout, u64::from(slot), src)?;
        // SAFETY: same slot bound as above.
        unsafe {
            (*self.generations().add(usize::try_from(slot).unwrap_or(0)))
                .store(generation, Ordering::Relaxed);
        }
        ring.commit(seq, slot, generation, ring_stamp);
        ccb.write_seq.store(next, Ordering::Release);
        Ok(seq)
    }

    /// Reads the newest committed sample into `buf` (copy form).
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `buf` is smaller than the payload capacity.
    /// REQ: REQ-CORE-103, REQ-CORE-106.
    pub fn read_latest(
        &self,
        state: &mut ReaderState,
        buf: &mut [u8],
    ) -> Result<ReadOutcome, Status> {
        let ccb = self.ccb();
        observe_epoch(state, ccb);
        let ws = ccb.write_seq.load(Ordering::Acquire);
        let fv = ccb.first_valid_seq.load(Ordering::Acquire);
        if ws <= fv {
            return Ok(ReadOutcome::Empty);
        }
        self.try_sequence(state, ws - 1, buf)
    }

    /// Reads the next sequence after the reader's cursor (copy form).
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `buf` is smaller than the payload capacity.
    /// REQ: REQ-CORE-103.
    pub fn read_next(
        &self,
        state: &mut ReaderState,
        buf: &mut [u8],
    ) -> Result<ReadOutcome, Status> {
        let ccb = self.ccb();
        observe_epoch(state, ccb);
        let ws = ccb.write_seq.load(Ordering::Acquire);
        let fv = ccb.first_valid_seq.load(Ordering::Acquire);
        let wanted = match state.last_read {
            // A fresh attached reader has no registration watermark, so it starts
            // at the oldest sample still addressable rather than reporting the
            // history it can never reach as a gap.
            None => fv.max(ws.saturating_sub(self.depth())),
            Some(last) => last.checked_add(1).ok_or(Status::InvalidData)?,
        };
        if wanted < fv {
            let missed = fv - wanted;
            state.last_read = Some(fv - 1);
            ccb.overrun_count.fetch_add(missed, Ordering::Relaxed);
            return Ok(ReadOutcome::Gap { missed });
        }
        if wanted >= ws {
            return Ok(ReadOutcome::Empty);
        }
        self.try_sequence(state, wanted, buf)
    }

    /// Copy-form acquisition of one sequence, guarded by the ring's version
    /// stamps: the bank is re-read after the copy, and a torn read retries.
    /// REQ: REQ-CORE-103.
    fn try_sequence(
        &self,
        state: &mut ReaderState,
        wanted: u64,
        buf: &mut [u8],
    ) -> Result<ReadOutcome, Status> {
        let ccb = self.ccb();
        let ring = self.ring()?;
        let mut attempts = 0;
        while attempts < RETRY_BUDGET {
            attempts += 1;
            let epoch0 = ccb.publisher_epoch.load(Ordering::Acquire);
            let first = match ring.lookup_sc(ccb, wanted) {
                Ok(entry) => entry,
                Err(LookupErr::Future) => return Ok(ReadOutcome::Empty),
                Err(LookupErr::AgedOut) => {
                    let ws = ccb.write_seq.load(Ordering::Acquire);
                    let fv = ccb.first_valid_seq.load(Ordering::Acquire);
                    let missed = ws.saturating_sub(fv);
                    state.last_read = Some(ws.saturating_sub(1));
                    return Ok(ReadOutcome::Gap { missed });
                }
            };
            let payload = self.payload(&first)?;
            if buf.len() < payload.len() {
                return Err(Status::InvalidArg);
            }
            buf[..payload.len()].copy_from_slice(payload);
            // Re-read the bank: unchanged ⇒ the copy is consistent (the stamp is
            // `SeqCst`, so no write can be in flight unobserved).
            let second = ring.lookup_sc(ccb, wanted);
            if ccb.publisher_epoch.load(Ordering::Acquire) != epoch0 {
                // Publisher restarted mid-read: the cursor belongs to the prior
                // run (I-AT5).
                state.last_read = None;
                continue;
            }
            match second {
                Ok(entry) if entry == first => {
                    state.last_read = Some(wanted);
                    return Ok(ReadOutcome::Sample);
                }
                // Torn or recycled: retry the same sequence.
                Ok(_) | Err(_) => {}
            }
        }
        Ok(ReadOutcome::WouldRetry)
    }

    /// Overflow policy from the attached descriptor.
    fn policy(&self) -> super::OverflowPolicy {
        match self.view.descriptor.overflow_policy {
            1 => super::OverflowPolicy::Reject,
            _ => super::OverflowPolicy::Overwrite,
        }
    }
}

/// Aligned slot stride for a payload capacity (mirrors `SegmentLayout`).
fn stride_of(payload_capacity: u64, slot_align: u64) -> u64 {
    let align = slot_align.max(1);
    let cap = payload_capacity.max(1);
    match cap.checked_add(align - 1) {
        Some(x) => x - (x % align),
        None => cap,
    }
}

/// Observer-side epoch change: a new epoch invalidates the reader's cursor.
fn observe_epoch(state: &mut ReaderState, ccb: &ChannelControlBlock) {
    let epoch = ccb.publisher_epoch.load(Ordering::Acquire);
    if epoch != state.observed_epoch {
        state.observed_epoch = epoch;
        state.last_read = None;
    }
}

/// Sole-writer guard over the mapped `publish_busy` flag; releases on every path
/// including panics.
struct BusyGuard<'a>(&'a AtomicBool);

impl<'a> BusyGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Result<Self, Status> {
        flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self(flag))
            .map_err(|_| Status::Busy)
    }
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
