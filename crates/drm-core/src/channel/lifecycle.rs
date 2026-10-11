//! Cross-process channel lifecycle over the platform SHM/lock backends
//! (docs/algorithms/runtime/publisher-l.md §Functions, shm-backend.md
//! §Protocols, channel-metadata.md §Functions): `publisher_open` (exclusive
//! create or classified adoption; I-L1, I-L8), `consumer_open` (open-only,
//! bounded startup policy; I-O2), and restart publication (floor + epoch;
//! I-L3, I-L6, I-L7). Ring repair and required-consumer floor publication
//! land with those subsystems on the shared mapping (Phase 17); this slice
//! covers the authority, validation, and publication path.
//! REQ: REQ-CORE-054, REQ-CORE-071, REQ-CORE-072, REQ-CORE-019, REQ-CORE-020,
//! REQ-CORE-026, REQ-CORE-015.

use std::mem::{offset_of, size_of};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::channel::ccb::{
    self, AttachedView, ChannelControlBlock, DESC_FLAG_TIMING, Expectation, SEGMENT_INIT,
    SEGMENT_READY, SegmentPrefix, validate_on_attach,
};
use crate::ring::RingCell;
use crate::shm::{LockBackend, LockHandle, Mapping, ShmBackend, ShmError};
use drm_types::Status;

/// Deployment-side layout inputs beyond the [`Expectation`]: descriptor flags
/// (TIMING/VARIANT bits) and the payload slot alignment.
/// REQ: REQ-CORE-015.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutSpec {
    /// Descriptor flags (`DESC_FLAG_*`).
    pub flags: u32,
    /// Payload slot alignment in bytes.
    pub slot_align: u64,
}

/// Wire size of one consumer record (consumer-registration.md §Data
/// Structures: state, registration generation, owner pid, padding, owner
/// start, hazard word, acknowledgement sequence).
/// REQ: REQ-CORE-009.
pub const CONSUMER_RECORD_SIZE: u64 = 40;

/// Computed segment image: descriptor plus the total segment size. Offsets
/// inside the descriptor; all arithmetic checked.
/// REQ: REQ-CORE-015, REQ-CORE-016.
#[derive(Debug, Clone)]
pub struct SegmentLayout {
    /// Descriptor image to write at offset `size_of::<SegmentPrefix>()`.
    pub descriptor: ccb::ChannelDescriptor,
    /// Exact total segment size in bytes.
    pub segment_size: u64,
}

/// Checked alignment round-up.
fn align_up(v: u64, align: u64) -> u64 {
    if align <= 1 {
        return v;
    }
    match v.checked_add(align - 1) {
        Some(x) => x - (x % align),
        None => v,
    }
}

/// Computes the full segment layout from the expectation and layout spec
/// (checked arithmetic only; regions in validation order: prefix, descriptor,
/// name, CCB, ring cells, consumer records, slot generations, payload pool).
/// REQ: REQ-CORE-015, REQ-CORE-016.
///
/// # Errors
/// [`Status::InvalidConfig`] on overflow or a name longer than `u16::MAX`.
pub fn segment_layout(expect: &Expectation, spec: &LayoutSpec) -> Result<SegmentLayout, Status> {
    let name_len = u64::try_from(expect.topic_name.len()).map_err(|_| Status::InvalidConfig)?;
    if name_len > u64::from(u16::MAX) {
        return Err(Status::InvalidConfig);
    }
    let timing = u64::from(spec.flags & DESC_FLAG_TIMING != 0);
    let ccb_size = u64::try_from(size_of::<ChannelControlBlock>()).unwrap_or(0) + timing * 8;
    let head = u64::try_from(size_of::<SegmentPrefix>() + size_of::<ccb::ChannelDescriptor>())
        .unwrap_or(0)
        .checked_add(name_len)
        .ok_or(Status::InvalidConfig)?;
    let ccb_offset = align_up(head, 8);
    let ring_offset = ccb_offset
        .checked_add(ccb_size)
        .ok_or(Status::InvalidConfig)?;
    let ring_bytes = expect
        .history_depth
        .checked_mul(u64::try_from(size_of::<RingCell>()).unwrap_or(0))
        .ok_or(Status::InvalidConfig)?;
    let consumer_offset = ring_offset
        .checked_add(ring_bytes)
        .ok_or(Status::InvalidConfig)?;
    let consumer_bytes = u64::from(expect.consumer_count)
        .checked_mul(CONSUMER_RECORD_SIZE)
        .ok_or(Status::InvalidConfig)?;
    let gen_offset = consumer_offset
        .checked_add(consumer_bytes)
        .ok_or(Status::InvalidConfig)?;
    let gen_bytes = expect
        .pool_size
        .checked_mul(8)
        .ok_or(Status::InvalidConfig)?;
    let payload_offset = gen_offset
        .checked_add(gen_bytes)
        .ok_or(Status::InvalidConfig)?;
    let stride = align_up(expect.payload_capacity.max(1), spec.slot_align.max(1));
    let pool_bytes = expect
        .pool_size
        .checked_mul(stride)
        .ok_or(Status::InvalidConfig)?;
    let segment_size = payload_offset
        .checked_add(pool_bytes)
        .ok_or(Status::InvalidConfig)?;
    let descriptor = ccb::ChannelDescriptor {
        flags: spec.flags,
        pad_header: 0,
        schema_hash: expect.schema_hash,
        layout_hash: expect.layout_hash,
        topic_handle: expect.topic_handle,
        topic_name_len: u16::try_from(expect.topic_name.len())
            .map_err(|_| Status::InvalidConfig)?,
        overflow_policy: expect.overflow_policy,
        pad0: 0,
        payload_capacity: expect.payload_capacity,
        history_depth: expect.history_depth,
        pool_size: expect.pool_size,
        consumer_count: expect.consumer_count,
        required_count: expect.required_count,
        diagnostic_count: expect.diagnostic_count,
        pad_counts: 0,
        ccb_offset,
        ring_offset,
        consumer_offset,
        slot_generation_offset: gen_offset,
        payload_offset,
        segment_size,
        reserved: [0; 16],
    };
    Ok(SegmentLayout {
        descriptor,
        segment_size,
    })
}

/// Byte view of the mapping.
///
/// # Safety
/// The mapping covers `len` readable bytes at `ptr` (platform mapper
/// contract); the slice aliases no live mutable view (single-writer
/// discipline).
unsafe fn as_bytes(m: &Mapping) -> &[u8] {
    // SAFETY: see function contract.
    unsafe { std::slice::from_raw_parts(m.ptr().as_ptr(), m.len()) }
}

/// Mutable byte view of the mapping.
///
/// # Safety
/// The mapping covers `len` writable bytes at `ptr`; the caller holds the
/// publisher authority (exclusive writer) for writes through this view.
#[allow(
    clippy::mut_from_ref,
    reason = "shared-memory aliasing: the mapping base is a shared region, the borrow protocol provides exclusivity"
)]
unsafe fn as_bytes_mut(m: &Mapping) -> &mut [u8] {
    // SAFETY: see function contract.
    unsafe { std::slice::from_raw_parts_mut(m.ptr().as_ptr(), m.len()) }
}

/// Reads the prefix lifecycle state as a plain aligned 4-byte load (the
/// publisher side publishes with an atomic Release store; aligned 4-byte
/// accesses are atomic on every supported target).
///
/// # Safety
/// `base` points at a mapping of at least `size_of::<SegmentPrefix>()` bytes.
unsafe fn prefix_state(base: *const u8) -> u32 {
    let off = offset_of!(SegmentPrefix, segment_state);
    // SAFETY: aligned, in-bounds 4-byte read (see function contract).
    unsafe {
        let mut a = [0_u8; 4];
        std::ptr::copy_nonoverlapping(base.add(off), a.as_mut_ptr(), 4);
        u32::from_ne_bytes(a)
    }
}

/// Publishes the segment image: atomic Release store to the prefix state
/// (I-L8 — the only point at which the segment becomes usable; everything
/// before is initialization).
///
/// # Safety
/// `base` points at a mapping of at least `size_of::<SegmentPrefix>()` bytes
/// with fully written descriptor, name, and CCB.
unsafe fn publish_ready(base: *mut u8) {
    let off = offset_of!(SegmentPrefix, segment_state);
    // SAFETY: offset 8 is 4-byte aligned within the mapping; the store
    // publishes every prior byte write to acquiring readers.
    #[allow(
        clippy::cast_ptr_alignment,
        reason = "mapping base is page-aligned and the offset is 4-byte aligned by construction"
    )]
    unsafe {
        let state = base.add(off).cast::<AtomicU32>();
        (*state).store(SEGMENT_READY, Ordering::Release);
    }
}

/// CCB field accessors over the mapping base (offsets compile-time asserted;
/// raw-pointer arithmetic needs no guard — dereference happens at the call
/// site inside the caller's `unsafe` block).
macro_rules! ccb_ptr {
    ($base:expr, $ccb_off:expr, $field:ident, $ty:ty) => {{
        let off = offset_of!(ChannelControlBlock, $field);
        #[allow(
            clippy::cast_ptr_alignment,
            reason = "mapping base is page-aligned, ccb_offset is 8-aligned and CCB fields are naturally aligned (I-M5)"
        )]
        {
            $base.add($ccb_off + off).cast::<$ty>()
        }
    }};
}

/// Restart publication (I-L3, I-L6, I-L7): floor = preserved next sequence;
/// busy/reclaim flags and the window start first, the epoch last. Required
/// acknowledgement floors land with the consumer subsystem (Phase 17).
/// Returns the published epoch.
/// REQ: REQ-CORE-020, REQ-CORE-026.
///
/// # Safety
/// `base` maps at least `ccb_offset + size_of::<ChannelControlBlock>()`
/// bytes; the caller holds publisher authority.
unsafe fn restart_publication(base: *mut u8, ccb_offset: u64) -> Result<u32, Status> {
    let off = usize::try_from(ccb_offset).map_err(|_| Status::InvalidConfig)?;
    // SAFETY: validated CCB base (see function contract).
    unsafe {
        let floor = (*ccb_ptr!(base, off, write_seq, AtomicU64)).load(Ordering::Acquire);
        (*ccb_ptr!(base, off, publish_busy, AtomicBool)).store(false, Ordering::Release);
        (*ccb_ptr!(base, off, reclaim_requested, AtomicBool)).store(false, Ordering::Release);
        (*ccb_ptr!(base, off, first_valid_seq, AtomicU64)).store(floor, Ordering::Release);
        let epoch = ccb_ptr!(base, off, publisher_epoch, AtomicU32);
        let old = (*epoch).load(Ordering::Acquire);
        let next = old
            .checked_add(1)
            .filter(|e| *e != 0)
            .ok_or(Status::EpochExhausted)?;
        (*epoch).store(next, Ordering::Release); // I-L7: published last
        Ok(next)
    }
}

/// Writes the fresh segment image with the prefix left `Init`: prefix,
/// descriptor, topic name, and zeroed CCB (the platform contract for a newly
/// created object is zero-filled storage; backends mirror it). Readiness is
/// published later by [`publish_ready`].
/// REQ: REQ-CORE-015, I-L8.
///
/// # Safety
/// `map` covers at least `layout.segment_size` bytes and is exclusively
/// writable by the caller (publisher authority held).
unsafe fn init_fresh(map: &Mapping, layout: &SegmentLayout, name: &[u8]) -> Result<(), Status> {
    let bytes = unsafe { as_bytes_mut(map) };
    ccb::write_prefix(bytes, SEGMENT_INIT)?;
    let dstart = size_of::<SegmentPrefix>();
    let dend = dstart + size_of::<ccb::ChannelDescriptor>();
    let db = bytes.get_mut(dstart..dend).ok_or(Status::InvalidArg)?;
    ccb::write_descriptor(db, &layout.descriptor)?;
    let name_end = dend + name.len();
    let name_dst = bytes.get_mut(dend..name_end).ok_or(Status::InvalidArg)?;
    name_dst.copy_from_slice(name);
    Ok(())
}

/// Outcome of the create-or-classify step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freshness {
    /// Freshly created image (including unlink+recreate recovery).
    Created,
    /// Existing segment adopted in `Ready` state.
    Adopted,
}

/// Create-or-classify step (publisher-l.md §Functions `publisher_open`):
/// exclusive create; on `AlreadyExists` classify before exact-size
/// validation — `Ready` adopts, `Init`/undersized recovers by unlink +
/// recreate under the held authority.
/// REQ: REQ-CORE-072, REQ-CORE-098, I-L8.
fn acquire_segment<S: ShmBackend>(
    shm: &S,
    name: &crate::shm::SegmentName,
    name_bytes: &[u8],
    size: usize,
    layout: &SegmentLayout,
) -> Result<(Mapping, Freshness), Status> {
    match shm.create_exclusive(name, size) {
        Ok(map) => {
            // SAFETY: fresh mapping of exactly `size` bytes, exclusive writer.
            unsafe { init_fresh(&map, layout, name_bytes)? };
            Ok((map, Freshness::Created))
        }
        Err(ShmError::AlreadyExists) => match shm.open_existing(name, size) {
            Ok(map) => {
                // SAFETY: mapping of at least the 16-byte prefix (I3).
                let state = unsafe { prefix_state(map.ptr().as_ptr()) };
                if state == SEGMENT_READY {
                    return Ok((map, Freshness::Adopted));
                }
                // Init image: the held authority proves no live writer
                // (publisher-lifecycle.md §Protocols "Init crash recovery").
                #[cfg(unix)]
                {
                    // POSIX: unlink + recreate (legal with the mapping still
                    // open; the stale view dies at arm end).
                    shm.unlink(name)?;
                    recreate(shm, name, name_bytes, size, layout)
                }
                #[cfg(windows)]
                {
                    // Windows: pagefile sections have no unlink, so an
                    // exact-size `Init` section is reinitialized in place.
                    // SAFETY: held authority; consumers never interpret `Init`.
                    unsafe { reinit_in_place(&map, layout, name_bytes)? };
                    Ok((map, Freshness::Created))
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = map;
                    Err(Status::NotReady)
                }
            }
            Err(ShmError::StaleIncomplete) => {
                // Creation never completed; recoverable artifact.
                #[cfg(unix)]
                {
                    shm.unlink(name)?;
                    recreate(shm, name, name_bytes, size, layout)
                }
                #[cfg(windows)]
                {
                    // No unlink for pagefile sections: an undersized section is
                    // recreated once its last handle closes; retryable meanwhile.
                    Err(ShmError::StaleIncomplete.into())
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err(Status::NotReady)
                }
            }
            Err(e) => Err(e.into()),
        },
        Err(e) => Err(e.into()),
    }
}

/// Recreates a fresh image after unlink (POSIX recovery path; Windows
/// reinitializes in place instead — [`reinit_in_place`]).
#[cfg(unix)]
fn recreate<S: ShmBackend>(
    shm: &S,
    name: &crate::shm::SegmentName,
    name_bytes: &[u8],
    size: usize,
    layout: &SegmentLayout,
) -> Result<(Mapping, Freshness), Status> {
    let map = shm.create_exclusive(name, size)?;
    // SAFETY: fresh mapping of exactly `size` bytes, exclusive writer.
    unsafe { init_fresh(&map, layout, name_bytes)? };
    Ok((map, Freshness::Created))
}

/// Reinitializes an exact-size `Init` section in place under the held publisher
/// authority (windows-backends.md §Protocols "Stale `Init` recovery"): zero the
/// CCB region, then rewrite the prefix, descriptor, and name; readiness is
/// published later by [`publish_ready`]. Pagefile sections have no unlink, and
/// consumers never interpret `Init` bytes, so no observer wait is required.
/// REQ: REQ-CORE-098.
///
/// # Safety
/// `map` covers `layout.segment_size` bytes and the caller holds the publisher
/// authority (no live writer).
#[cfg(windows)]
unsafe fn reinit_in_place(
    map: &Mapping,
    layout: &SegmentLayout,
    name: &[u8],
) -> Result<(), Status> {
    let ccb_off = usize::try_from(layout.descriptor.ccb_offset).map_err(|_| Status::InvalidArg)?;
    {
        // SAFETY: see function contract; the caller holds the publisher
        // authority, so this mutable view aliases no live writer.
        let bytes = unsafe { as_bytes_mut(map) };
        // A never-ready image carries no valid CCB state; zero the region from
        // the CCB onward so restart publication starts from a clean floor.
        if let Some(tail) = bytes.get_mut(ccb_off..) {
            tail.fill(0);
        }
    }
    // SAFETY: same mapping and authority contract as the caller.
    unsafe { init_fresh(map, layout, name) }
}

/// Held publisher authority: validated attachment, epoch after this
/// acquisition, the mapping, and the OS lock (Drop releases authority).
/// REQ: REQ-CORE-054.
#[derive(Debug)]
pub struct Publisher {
    /// Validated attachment view (compare-once; steady state never re-checks).
    pub view: AttachedView,
    /// Publisher epoch after this authority acquisition (I-L3).
    pub epoch: u32,
    map: Mapping,
    _lock: LockHandle,
}

impl Publisher {
    /// Mapping view (payload and control regions).
    #[must_use]
    pub fn mapping(&self) -> &Mapping {
        &self.map
    }
}

/// Acquires publisher authority and opens or creates the segment
/// (publisher-l.md §Functions `publisher_open`): the lock is held-or-refuse
/// (I-L1 — the loser observes `Permission`), the segment is created
/// exclusively or classified and adopted/recovered, attachment is validated
/// once, and restart publication runs (fresh: epoch 1; adopted: epoch + 1,
/// published last).
/// REQ: REQ-CORE-054, REQ-CORE-072, REQ-CORE-019, REQ-CORE-020, REQ-CORE-026.
///
/// # Errors
/// [`Status::Permission`] when another live publisher holds the authority;
/// [`Status::NotFound`]/`AlreadyExists`/`Permission` per the backend;
/// [`Status::NotReady`]/`WrongAbi`/`InvalidData`/`InvalidConfig` per
/// [`validate_on_attach`]; [`Status::EpochExhausted`] when the epoch space
/// is exhausted.
pub fn publisher_open<S: ShmBackend, L: LockBackend>(
    shm: &S,
    locks: &L,
    lock_path: &Path,
    name: &crate::shm::SegmentName,
    name_bytes: &[u8],
    expect: &Expectation,
    spec: &LayoutSpec,
) -> Result<Publisher, Status> {
    let layout = segment_layout(expect, spec)?;
    let size = usize::try_from(layout.segment_size).map_err(|_| Status::InvalidConfig)?;
    // I-L1: held-or-refuse; the loser reads as "another live publisher
    // exists" (publisher-l.md edge cases: `Permission`, never a partial init).
    let lock = locks.try_exclusive(lock_path).map_err(|e| match e {
        ShmError::AlreadyExists => Status::Permission,
        other => other.into(),
    })?;
    let (map, freshness) = acquire_segment(shm, name, name_bytes, size, &layout)?;
    if freshness == Freshness::Created {
        // Complete the CCB image, then publish readiness last (I-L8).
        let ccb_off = layout.descriptor.ccb_offset;
        // SAFETY: fresh mapping of `size` bytes with the image written.
        let epoch = unsafe { restart_publication(map.ptr().as_ptr(), ccb_off)? };
        // SAFETY: fully initialized image (see publish_ready contract).
        unsafe { publish_ready(map.ptr().as_ptr()) };
        let bytes = unsafe { as_bytes(&map) };
        let view = validate_on_attach(bytes, expect)?;
        return Ok(Publisher {
            view,
            epoch,
            map,
            _lock: lock,
        });
    }
    let bytes = unsafe { as_bytes(&map) };
    let view = validate_on_attach(bytes, expect)?;
    // SAFETY: validated CCB offset; authority held.
    let epoch = unsafe { restart_publication(map.ptr().as_ptr(), view.ccb_offset)? };
    Ok(Publisher {
        view,
        epoch,
        map,
        _lock: lock,
    })
}

/// Held consumer attachment: validated view plus the mapping. Never holds
/// publisher authority; the registration record claim lands with the
/// consumer subsystem (Phase 17).
/// REQ: REQ-CORE-071.
#[derive(Debug)]
pub struct Consumer {
    /// Validated attachment view.
    pub view: AttachedView,
    map: Mapping,
}

impl Consumer {
    /// Mapping view (read-only discipline is the caller's borrow protocol).
    #[must_use]
    pub fn mapping(&self) -> &Mapping {
        &self.map
    }
}

/// Opens an existing segment for consumption. Never creates (I-O2 —
/// `open_existing` only). The bounded wait applies to `NotFound` (the
/// publisher has not created the segment yet) and `NotReady` (segment still
/// `Init` or undersized) per the consumer startup policy; terminal errors
/// return immediately. `wait = None` performs a single attempt.
/// REQ: REQ-CORE-071, REQ-CORE-002.
///
/// # Errors
/// [`Status::NotFound`] when the segment does not exist (and the wait, if
/// any, elapsed); [`Status::NotReady`] while initialization has not
/// completed; terminal validation errors per [`validate_on_attach`].
pub fn consumer_open<S: ShmBackend>(
    shm: &S,
    name: &crate::shm::SegmentName,
    expect: &Expectation,
    spec: &LayoutSpec,
    wait: Option<Duration>,
) -> Result<Consumer, Status> {
    let layout = segment_layout(expect, spec)?;
    let size = usize::try_from(layout.segment_size).map_err(|_| Status::InvalidConfig)?;
    let deadline = wait.map(|w| Instant::now() + w);
    let mut last;
    loop {
        match shm.open_existing(name, size) {
            Ok(map) => {
                let bytes = unsafe { as_bytes(&map) };
                match validate_on_attach(bytes, expect) {
                    Ok(view) => return Ok(Consumer { view, map }),
                    Err(Status::NotReady) => {
                        last = Status::NotReady;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
            Err(ShmError::NotFound) => last = Status::NotFound,
            Err(ShmError::StaleIncomplete) => last = Status::NotReady,
            Err(e) => return Err(e.into()),
        }
        if deadline.is_none_or(|d| Instant::now() >= d) {
            return Err(last);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests;
