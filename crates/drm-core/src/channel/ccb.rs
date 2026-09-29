//! Channel control block, segment prefix, and channel descriptor
//! (docs/algorithms/runtime/channel-metadata.md §Data Structures).
//! REQ: REQ-CORE-015 (descriptor stored once), REQ-CORE-014 (naturally aligned, no
//! packed atomics), REQ-CORE-093 (diagnostic counters), REQ-CORE-094 (timing variant).

use std::mem::{align_of, offset_of, size_of};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use drm_types::Status;

/// Segment layout state (SegmentPrefix).
/// REQ: REQ-CORE-015.
pub const SEGMENT_ZERO: u32 = 0;
/// Segment layout state (SegmentPrefix).
/// REQ: REQ-CORE-015.
pub const SEGMENT_INIT: u32 = 1;
/// Segment layout state (SegmentPrefix).
/// REQ: REQ-CORE-015.
pub const SEGMENT_READY: u32 = 2;

/// Descriptor flag bit 0: timing-instrumented CCB + ring-entry variant.
/// REQ: REQ-CORE-094.
pub const DESC_FLAG_TIMING: u32 = 1 << 0;

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
