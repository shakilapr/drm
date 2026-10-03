//! Channel control block, segment prefix, and channel descriptor
//! (docs/algorithms/runtime/channel-metadata.md §Data Structures).
//! REQ: REQ-CORE-015 (descriptor stored once), REQ-CORE-014 (naturally aligned, no
//! packed atomics), REQ-CORE-093 (diagnostic counters), REQ-CORE-094 (timing variant).

use std::mem::{align_of, offset_of, size_of};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use drm_types::Status;

/// Segment layout state constant for `SegmentPrefix`.
/// REQ: REQ-CORE-015.
pub const SEGMENT_ZERO: u32 = 0;
/// Segment layout state constant for `SegmentPrefix`.
/// REQ: REQ-CORE-015.
pub const SEGMENT_INIT: u32 = 1;
/// Segment layout state constant for `SegmentPrefix`.
/// REQ: REQ-CORE-015.
pub const SEGMENT_READY: u32 = 2;

/// Descriptor flag bit 0: timing-instrumented CCB + ring-entry variant.
/// REQ: REQ-CORE-094.
pub const DESC_FLAG_TIMING: u32 = 1 << 0;

/// Descriptor flag bit 1: variable-length payload (bounded tail).
/// REQ: REQ-CORE-016.
pub const DESC_FLAG_VARIABLE: u32 = 1 << 1;

/// Layout versions accepted by [`validate_on_attach`] (exact-match rule).
/// REQ: REQ-CORE-015.
pub const SUPPORTED_LAYOUT_VERSIONS: [u32; 1] = [0];

/// Fixed 16-byte bootstrap at offset zero.
/// REQ: REQ-CORE-015, REQ-CORE-014.
#[repr(C, align(8))]
pub struct SegmentPrefix {
    /// Magic `b"DRMC"`.
    pub magic: [u8; 4],
    /// Layout version (exact-match attach rule).
    pub layout_version: u32,
    /// Zero/Init/Ready lifecycle state.
    pub segment_state: AtomicU32,
    /// Size of the descriptor block that follows.
    pub descriptor_size: u32,
}
const _: () = assert!(size_of::<SegmentPrefix>() == 16);
const _: () = assert!(align_of::<SegmentPrefix>() == 8);

/// Immutable descriptor following the prefix; dynamic regions use checked offsets.
/// REQ: REQ-CORE-015, REQ-CORE-017, REQ-CORE-016.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(8))]
pub struct ChannelDescriptor {
    /// TIMING bit selects the instrumented CCB/ring-entry variant.
    pub flags: u32,
    /// Reserved padding to keep 8-byte alignment of the next field.
    pub pad_header: u32,
    /// 128-bit canonical schema content hash.
    pub schema_hash: [u8; 16],
    /// Exact descriptor + CCB + canonical payload layout hash.
    pub layout_hash: [u8; 16],
    /// Compact runtime topic handle (handle-system.md).
    pub topic_handle: u32,
    /// Length of the topic name stored contiguously after the header.
    pub topic_name_len: u16,
    /// Overflow policy: 0 = overwrite, 1 = reject.
    pub overflow_policy: u8,
    /// Reserved alignment byte.
    pub pad0: u8,
    /// Fixed payload size or max bound (bytes).
    pub payload_capacity: u64,
    /// History depth R (1 for `@history: latest`).
    pub history_depth: u64,
    /// Payload slots (payload-pool.md; `pool_size >= R + 1`).
    pub pool_size: u64,
    /// Declared consumer records.
    pub consumer_count: u32,
    /// Required-sequential consumer records.
    pub required_count: u32,
    /// Reserved diagnostic-consumer records.
    pub diagnostic_count: u32,
    /// Reserved alignment padding.
    pub pad_counts: u32,
    /// Checked offset of the CCB within the segment.
    pub ccb_offset: u64,
    /// Checked offset of the ring bank array.
    pub ring_offset: u64,
    /// Checked offset of consumer records.
    pub consumer_offset: u64,
    /// Checked offset of slot generation counters.
    pub slot_generation_offset: u64,
    /// Checked offset of the payload pool.
    pub payload_offset: u64,
    /// Exact segment size (validated against the mapping length).
    pub segment_size: u64,
    /// Zeroed forward-compatibility reserve.
    pub reserved: [u8; 16],
}
const _: () = assert!(align_of::<ChannelDescriptor>() == 8);

/// Fixed bootstrap/control block; ring and consumer records follow at descriptor
/// offsets. REQ: REQ-CORE-014, REQ-CORE-019, REQ-CORE-026, REQ-CORE-093.
#[repr(C, align(8))]
pub struct ChannelControlBlock {
    /// Publisher epoch; restart increments exactly once (publisher-lifecycle.md).
    pub publisher_epoch: AtomicU32,
    /// Reserved alignment padding.
    pub pad_epoch: u32,
    /// Addressability watermark; sole physical owner (ring-history.md I5).
    pub first_valid_seq: AtomicU64,
    /// Next sequence to publish.
    pub write_seq: AtomicU64,
    /// Samples dropped inside DRM (pool exhaustion).
    pub drop_count: AtomicU64,
    /// Sequences reported missed by consumer Gap outcomes.
    pub overrun_count: AtomicU64,
    /// Pool-exhaustion event counter.
    pub pool_exhausted: AtomicU64,
    /// Set by the RT publisher on exhaustion; consumed by maintenance reclaim.
    pub reclaim_requested: AtomicBool,
    /// Sole physical in-progress-write flag.
    pub publish_busy: AtomicBool,
    /// Reserved alignment padding.
    pub pad: [u8; 6],
}
const _: () = assert!(align_of::<ChannelControlBlock>() == 8);
const _: () = assert!(offset_of!(ChannelControlBlock, first_valid_seq) % 8 == 0);
const _: () = assert!(offset_of!(ChannelControlBlock, write_seq) % 8 == 0);

impl ChannelControlBlock {
    /// Zero-valued control block (activation-time initialization).
    /// REQ: REQ-CORE-014.
    #[must_use]
    pub fn zeroed() -> Self {
        Self {
            publisher_epoch: AtomicU32::new(0),
            pad_epoch: 0,
            first_valid_seq: AtomicU64::new(0),
            write_seq: AtomicU64::new(0),
            drop_count: AtomicU64::new(0),
            overrun_count: AtomicU64::new(0),
            pool_exhausted: AtomicU64::new(0),
            reclaim_requested: AtomicBool::new(false),
            publish_busy: AtomicBool::new(false),
            pad: [0; 6],
        }
    }

    /// REQ: REQ-CORE-002 — epoch observation compare (in-call reread gate).
    #[must_use]
    pub fn epoch_changed(&self, observed_epoch: u32) -> bool {
        self.publisher_epoch.load(Ordering::Acquire) != observed_epoch
    }

    /// REQ: REQ-CORE-026 — addressability watermark read (Acquire).
    #[must_use]
    pub fn first_valid(&self) -> u64 {
        self.first_valid_seq.load(Ordering::Acquire)
    }

    /// REQ: REQ-CORE-061 — sequence-space exhaustion guard before any payload write.
    /// # Errors
    /// [`Status::SequenceExhausted`] when `next` has no successor.
    pub fn check_sequence_free(next: u64) -> Result<(), Status> {
        if next == u64::MAX {
            return Err(Status::SequenceExhausted);
        }
        Ok(())
    }
}

/// What the attaching side expects the segment to contain; built once from
/// deployment data. REQ: REQ-CORE-002, REQ-CORE-017.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expectation {
    /// Canonical schema content hash (secondary per-segment check).
    pub schema_hash: [u8; 16],
    /// Exact descriptor + CCB + canonical payload layout hash.
    pub layout_hash: [u8; 16],
    /// Compact runtime topic handle.
    pub topic_handle: u32,
    /// Canonical topic name stored after the descriptor.
    pub topic_name: String,
    /// Fixed payload size or max bound (bytes).
    pub payload_capacity: u64,
    /// History depth R (1 for `@history: latest`).
    pub history_depth: u64,
    /// Payload slots (`pool_size >= R + 1`).
    pub pool_size: u64,
    /// Overflow policy: 0 = overwrite, 1 = reject.
    pub overflow_policy: u8,
    /// Declared consumer records.
    pub consumer_count: u32,
    /// Required-sequential consumer records.
    pub required_count: u32,
    /// Reserved diagnostic-consumer records.
    pub diagnostic_count: u32,
}

/// Successfully validated attachment: descriptor copy + checked offsets. The
/// caller never re-validates immutable fields afterwards (compare-once,
/// I-M2). REQ: REQ-CORE-002, REQ-CORE-016.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedView {
    /// Validated descriptor (copy; the mapping bytes stay authoritative).
    pub descriptor: ChannelDescriptor,
    /// Checked CCB offset.
    pub ccb_offset: u64,
    /// Checked ring-bank offset.
    pub ring_offset: u64,
    /// Checked consumer-record offset.
    pub consumer_offset: u64,
    /// Checked slot-generation-counter offset.
    pub slot_generation_offset: u64,
    /// Checked payload-pool offset.
    pub payload_offset: u64,
}

/// LE `u32` read at `off` (bounds-checked caller).
fn read_u32(b: &[u8], off: usize) -> u32 {
    let mut a = [0_u8; 4];
    if let Some(s) = b.get(off..off + 4) {
        a.copy_from_slice(s);
    }
    u32::from_le_bytes(a)
}

/// LE `u64` read at `off` (bounds-checked caller).
fn read_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0_u8; 8];
    if let Some(s) = b.get(off..off + 8) {
        a.copy_from_slice(s);
    }
    u64::from_le_bytes(a)
}

/// Parses a descriptor from exactly `size_of::<ChannelDescriptor>()` bytes
/// (caller verifies the length); field offsets match the compile-time asserts.
/// REQ: REQ-CORE-015.
#[must_use]
pub fn read_descriptor(b: &[u8]) -> ChannelDescriptor {
    let byte = |off: usize| -> u8 { b.get(off).map_or(0, |v| *v) };
    let mut schema_hash = [0_u8; 16];
    let mut layout_hash = [0_u8; 16];
    let mut reserved = [0_u8; 16];
    for i in 0..16 {
        schema_hash[i] = byte(8 + i);
        layout_hash[i] = byte(24 + i);
        reserved[i] = byte(136 + i);
    }
    ChannelDescriptor {
        flags: read_u32(b, 0),
        pad_header: read_u32(b, 4),
        schema_hash,
        layout_hash,
        topic_handle: read_u32(b, 40),
        topic_name_len: u16::try_from(read_u32(b, 44) & 0xFFFF).unwrap_or(0),
        overflow_policy: byte(46),
        pad0: byte(47),
        payload_capacity: read_u64(b, 48),
        history_depth: read_u64(b, 56),
        pool_size: read_u64(b, 64),
        consumer_count: read_u32(b, 72),
        required_count: read_u32(b, 76),
        diagnostic_count: read_u32(b, 80),
        pad_counts: read_u32(b, 84),
        ccb_offset: read_u64(b, 88),
        ring_offset: read_u64(b, 96),
        consumer_offset: read_u64(b, 104),
        slot_generation_offset: read_u64(b, 112),
        payload_offset: read_u64(b, 120),
        segment_size: read_u64(b, 128),
        reserved,
    }
}

/// Writes a descriptor as LE bytes at the compile-time-asserted offsets into
/// `buf` (which must be exactly the descriptor size). Used by the publisher
/// init path and the byte-exact golden test. REQ: REQ-CORE-015.
///
/// # Errors
/// [`Status::InvalidArg`] when `buf` is not exactly the descriptor size.
pub fn write_descriptor(buf: &mut [u8], d: &ChannelDescriptor) -> Result<(), Status> {
    if buf.len() != size_of::<ChannelDescriptor>() {
        return Err(Status::InvalidArg);
    }
    let put = |dst: &mut [u8], src: &[u8]| dst.copy_from_slice(src);
    put(&mut buf[0..4], &d.flags.to_le_bytes());
    put(&mut buf[4..8], &d.pad_header.to_le_bytes());
    buf[8..24].copy_from_slice(&d.schema_hash);
    buf[24..40].copy_from_slice(&d.layout_hash);
    put(&mut buf[40..44], &d.topic_handle.to_le_bytes());
    put(&mut buf[44..46], &d.topic_name_len.to_le_bytes());
    buf[46] = d.overflow_policy;
    buf[47] = d.pad0;
    put(&mut buf[48..56], &d.payload_capacity.to_le_bytes());
    put(&mut buf[56..64], &d.history_depth.to_le_bytes());
    put(&mut buf[64..72], &d.pool_size.to_le_bytes());
    put(&mut buf[72..76], &d.consumer_count.to_le_bytes());
    put(&mut buf[76..80], &d.required_count.to_le_bytes());
    put(&mut buf[80..84], &d.diagnostic_count.to_le_bytes());
    put(&mut buf[84..88], &d.pad_counts.to_le_bytes());
    put(&mut buf[88..96], &d.ccb_offset.to_le_bytes());
    put(&mut buf[96..104], &d.ring_offset.to_le_bytes());
    put(&mut buf[104..112], &d.consumer_offset.to_le_bytes());
    put(&mut buf[112..120], &d.slot_generation_offset.to_le_bytes());
    put(&mut buf[120..128], &d.payload_offset.to_le_bytes());
    put(&mut buf[128..136], &d.segment_size.to_le_bytes());
    buf[136..152].copy_from_slice(&d.reserved);
    Ok(())
}

/// Writes the fixed 16-byte segment prefix. REQ: REQ-CORE-015.
///
/// # Errors
/// [`Status::InvalidArg`] when `buf` is shorter than the prefix.
pub fn write_prefix(buf: &mut [u8], state: u32) -> Result<(), Status> {
    if buf.len() < size_of::<SegmentPrefix>() {
        return Err(Status::InvalidArg);
    }
    buf[0..4].copy_from_slice(b"DRMC");
    buf[4..8].copy_from_slice(&0_u32.to_le_bytes());
    buf[8..12].copy_from_slice(&state.to_le_bytes());
    buf[12..16].copy_from_slice(
        &u32::try_from(size_of::<ChannelDescriptor>())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    Ok(())
}

/// Attach validation — runs exactly once per open (publisher or consumer);
/// steady state never re-validates (I-M2). Checks the fixed prefix (state
/// must be `Ready`, I-M6; nothing else is interpreted otherwise), then every
/// immutable descriptor field against `expect`, the structural invariants
/// (history depth, pool sizing, record counts, tail bound), zero reserved
/// bytes, and the aligned non-overlapping region layout (I-M7). Lock-free
/// shared atomics are a target platform precondition (all supported targets
/// provide them; enforced by CI, not at runtime).
/// REQ: REQ-CORE-015, REQ-CORE-002, REQ-CORE-016, REQ-CORE-017, REQ-CORE-068.
///
/// # Errors
/// [`Status::NotReady`] while the prefix state is not `Ready`;
/// [`Status::InvalidData`] for identity/content mismatches (magic, schema
/// hash, topic handle/name); [`Status::WrongAbi`] for layout mismatches
/// (version, descriptor size, layout hash, capacity set, reserved bytes);
/// [`Status::InvalidConfig`] for structural violations.
pub fn validate_on_attach(segment: &[u8], expect: &Expectation) -> Result<AttachedView, Status> {
    validate_prefix(segment)?;
    let dstart = size_of::<SegmentPrefix>();
    let dend = dstart + size_of::<ChannelDescriptor>();
    let db = segment.get(dstart..dend).ok_or(Status::InvalidData)?;
    let d = read_descriptor(db);
    validate_descriptor(&d, segment, expect)?;
    let name_end = dend + usize::from(d.topic_name_len);
    let name = segment.get(dend..name_end).ok_or(Status::InvalidData)?;
    if name != expect.topic_name.as_bytes() {
        return Err(Status::InvalidData);
    }
    validate_ranges(&d, segment.len())?;
    Ok(AttachedView {
        descriptor: d,
        ccb_offset: read_u64(db, 88),
        ring_offset: read_u64(db, 96),
        consumer_offset: read_u64(db, 104),
        slot_generation_offset: read_u64(db, 112),
        payload_offset: read_u64(db, 120),
    })
}

/// Prefix gate: lifecycle state FIRST (I-M6 — a non-`Ready` prefix is
/// `NotReady` and nothing else is interpreted; a zero-filled in-progress
/// segment is a retryable startup artifact, not a compatibility failure),
/// then magic, layout version, and descriptor size.
/// REQ: REQ-CORE-015, REQ-CORE-068.
fn validate_prefix(segment: &[u8]) -> Result<(), Status> {
    if segment.len() < size_of::<SegmentPrefix>() {
        return Err(Status::InvalidData);
    }
    if read_u32(segment, 8) != SEGMENT_READY {
        return Err(Status::NotReady);
    }
    if segment[0..4] != *b"DRMC" {
        return Err(Status::InvalidData);
    }
    if !SUPPORTED_LAYOUT_VERSIONS.contains(&read_u32(segment, 4)) {
        return Err(Status::WrongAbi);
    }
    let dsize = usize::try_from(read_u32(segment, 12)).unwrap_or(usize::MAX);
    if dsize != size_of::<ChannelDescriptor>() {
        return Err(Status::WrongAbi);
    }
    Ok(())
}

/// Immutable-field and structural checks (compare-once). REQ: REQ-CORE-016,
/// REQ-CORE-017, REQ-CORE-068.
fn validate_descriptor(
    d: &ChannelDescriptor,
    segment: &[u8],
    expect: &Expectation,
) -> Result<(), Status> {
    if d.schema_hash != expect.schema_hash {
        return Err(Status::InvalidData);
    }
    if d.layout_hash != expect.layout_hash
        || d.payload_capacity != expect.payload_capacity
        || d.history_depth != expect.history_depth
        || d.pool_size != expect.pool_size
        || d.overflow_policy != expect.overflow_policy
        || d.consumer_count != expect.consumer_count
        || d.required_count != expect.required_count
        || d.diagnostic_count != expect.diagnostic_count
    {
        return Err(Status::WrongAbi);
    }
    if d.topic_handle != expect.topic_handle {
        return Err(Status::InvalidData);
    }
    if u64::try_from(segment.len()).map_err(|_| Status::InvalidConfig)? != d.segment_size {
        return Err(Status::WrongAbi);
    }
    let reserved = u64::from(d.required_count).checked_add(u64::from(d.diagnostic_count));
    let Some(reserved) = reserved else {
        return Err(Status::InvalidConfig);
    };
    if d.history_depth == 0 || reserved > u64::from(d.consumer_count) {
        return Err(Status::InvalidConfig);
    }
    let Some(min_pool) = d.history_depth.checked_add(1) else {
        return Err(Status::InvalidConfig);
    };
    if d.pool_size < min_pool || d.pool_size >= u64::from(u32::MAX) {
        return Err(Status::InvalidConfig);
    }
    if (d.flags & DESC_FLAG_VARIABLE) != 0 && d.payload_capacity > u64::from(u32::MAX) {
        return Err(Status::InvalidConfig);
    }
    if d.reserved.iter().any(|b| *b != 0) {
        return Err(Status::WrongAbi);
    }
    Ok(())
}

/// Aligned, strictly ordered, in-bounds region layout (checked arithmetic
/// only). Ring/consumer record sizes arrive with those subsystems; this slice
/// enforces ordering, alignment, prefix/CCB bounds, and pool capacity bounds.
/// REQ: REQ-CORE-016, REQ-CORE-014.
fn validate_ranges(d: &ChannelDescriptor, segment_len: usize) -> Result<(), Status> {
    let len = u64::try_from(segment_len).map_err(|_| Status::InvalidConfig)?;
    for off in [
        d.ccb_offset,
        d.ring_offset,
        d.consumer_offset,
        d.slot_generation_offset,
        d.payload_offset,
    ] {
        if off % 8 != 0 || off >= len {
            return Err(Status::InvalidConfig);
        }
    }
    if !(d.ccb_offset < d.ring_offset
        && d.ring_offset < d.consumer_offset
        && d.consumer_offset < d.slot_generation_offset
        && d.slot_generation_offset < d.payload_offset)
    {
        return Err(Status::InvalidConfig);
    }
    let ccb_size = u64::try_from(size_of::<ChannelControlBlock>()).unwrap_or(0)
        + u64::from(u32::from(d.flags & DESC_FLAG_TIMING != 0)) * 8;
    let ccb_end = d
        .ccb_offset
        .checked_add(ccb_size)
        .ok_or(Status::InvalidConfig)?;
    if ccb_end > d.ring_offset {
        return Err(Status::InvalidConfig);
    }
    let desc_end =
        size_of::<SegmentPrefix>() + size_of::<ChannelDescriptor>() + usize::from(d.topic_name_len);
    let desc_end = u64::try_from(desc_end).map_err(|_| Status::InvalidConfig)?;
    if desc_end > d.ccb_offset {
        return Err(Status::InvalidConfig);
    }
    let pool_bytes = d
        .pool_size
        .checked_mul(d.payload_capacity)
        .ok_or(Status::InvalidConfig)?;
    let payload_end = d
        .payload_offset
        .checked_add(pool_bytes)
        .ok_or(Status::InvalidConfig)?;
    if payload_end > len {
        return Err(Status::InvalidConfig);
    }
    Ok(())
}
