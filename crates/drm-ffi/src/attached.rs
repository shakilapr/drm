//! Cross-process attached-channel endpoints over the C ABI
//! ([attached-data-path.md](../../../docs/algorithms/runtime/attached-data-path.md),
//! ADR [0007](../../../docs/adr/0007-cross-process-channel-data-operations.md)).
//!
//! These are the first C ABI exports backed by a *real shared segment* rather
//! than an in-process channel: `drm_attached_open` acquires publisher authority or
//! attaches read-only, and the publish/read exports operate over the segment's
//! reserved regions through [`AttachedChannel`]. Append-only addition to the
//! frozen v1.0 surface (`REQ-CORE-107`): no existing symbol changes shape.
//!
//! Slot alignment is fixed at 8 for these exports (the value `drm build` lays
//! segments out with by default), so a C-side attach agrees with a Rust-side
//! publisher on the same layout without extra negotiation.
//!
//! REQ: REQ-CORE-107, REQ-CORE-102, REQ-CORE-103.

use core::ffi::c_char;
use core::ptr;
use std::time::Duration;

use drm_core::channel::attached::AttachedChannel;
use drm_core::channel::ccb::Expectation;
use drm_core::channel::lifecycle::{
    Consumer, LayoutSpec, Publisher, consumer_open, publisher_open,
};
use drm_core::channel::{ReadOutcome, ReaderState};
use drm_core::shm::{FlockBackend, LockPaths, PosixShm, SegmentName};
use drm_types::Status;

use crate::{drm_status, zero_out};

/// Slot alignment these exports lay out and expect.
const SLOT_ALIGN: u64 = 8;
/// `mode` value: attach read-only as a consumer.
pub const MODE_CONSUMER: u8 = 0;
/// `mode` value: acquire publisher authority (single writer).
pub const MODE_PUBLISHER: u8 = 1;

/// Opaque attached-endpoint handle (opaque by design — REQ-BIND-004).
pub enum drm_attached {}

/// Live endpoint behind a `drm_attached` handle.
enum Endpoint {
    /// Holds segment authority for its lifetime.
    Publisher(Publisher),
    /// Read-only attachment plus the caller's cursor.
    Consumer {
        /// The validated attachment.
        consumer: Consumer,
        /// Copy-form cursor state for `drm_attached_read_next`, created on first
        /// use from the attachment's epoch.
        state: Option<ReaderState>,
    },
}

struct AttachedBox {
    endpoint: Endpoint,
}

/// Reads a NUL-terminated C string as UTF-8.
///
/// # Safety
/// `s` is null or NUL-terminated.
unsafe fn c_str(s: *const c_char) -> Result<&'static str, Status> {
    if s.is_null() {
        return Err(Status::InvalidArg);
    }
    // SAFETY: caller guarantees NUL termination.
    let bytes = unsafe { core::ffi::CStr::from_ptr(s) }.to_bytes();
    core::str::from_utf8(bytes).map_err(|_| Status::InvalidArg)
}

/// Copies a caller-provided 16-byte hash, or zeros when the pointer is null.
fn hash_arg(p: *const u8) -> [u8; 16] {
    let mut out = [0_u8; 16];
    if !p.is_null() {
        // SAFETY: contract — `p` addresses at least 16 readable bytes.
        let src = unsafe { core::slice::from_raw_parts(p, 16) };
        out.copy_from_slice(src);
    }
    out
}

/// The expectation both sides must agree on for a segment.
#[allow(clippy::too_many_arguments)]
fn expectation(
    topic: &str,
    schema_hash: [u8; 16],
    layout_hash: [u8; 16],
    topic_handle: u32,
    sample_max: u64,
    depth: u64,
    pool: u64,
    overflow_policy: u8,
) -> Expectation {
    Expectation {
        schema_hash,
        layout_hash,
        topic_handle,
        topic_name: topic.to_string(),
        payload_capacity: sample_max,
        history_depth: depth,
        pool_size: pool,
        overflow_policy,
        consumer_count: 1,
        required_count: 0,
        diagnostic_count: 0,
    }
}

/// Opens a segment as its publisher or as a read-only consumer.
///
/// `mode` selects `MODE_PUBLISHER` (acquires the segment's single-writer
/// authority; a second live publisher is refused with `DRM_ERR_PERMISSION`) or
/// `MODE_CONSUMER` (never creates; `timeout_ms` bounds the wait for the segment).
/// Both sides must pass the same `segment_name`, `topic_name`, hashes, handle, and
/// sizes, and the same `lock_dir` the publisher uses.
/// REQ: REQ-CORE-107, REQ-CORE-102, REQ-CORE-104.
///
/// # Safety
/// Every pointer is null or valid for its documented length; `out` is writable.
#[must_use]
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn drm_attached_open(
    segment_name: *const c_char,
    topic_name: *const c_char,
    lock_dir: *const c_char,
    schema_hash: *const u8,
    layout_hash: *const u8,
    topic_handle: u32,
    sample_max: u64,
    depth: u64,
    pool: u64,
    overflow_policy: u8,
    mode: u8,
    timeout_ms: u64,
    out: *mut *mut drm_attached,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    let opened = (|| -> Result<AttachedBox, Status> {
        // SAFETY: caller contract on the C strings.
        let segment = unsafe { c_str(segment_name) }?;
        let topic = unsafe { c_str(topic_name) }?;
        let lock_dir = unsafe { c_str(lock_dir) }?;
        let segment = SegmentName::new(segment).map_err(|_| Status::InvalidArg)?;
        let expected = expectation(
            topic,
            hash_arg(schema_hash),
            hash_arg(layout_hash),
            topic_handle,
            sample_max,
            depth,
            pool,
            overflow_policy,
        );
        let spec = LayoutSpec {
            flags: 0,
            slot_align: SLOT_ALIGN,
        };
        match mode {
            MODE_PUBLISHER => {
                let locks = LockPaths::new(lock_dir);
                let publisher = publisher_open(
                    &PosixShm,
                    &FlockBackend,
                    &locks.publisher(topic_handle),
                    &segment,
                    topic.as_bytes(),
                    &expected,
                    &spec,
                )?;
                Ok(AttachedBox {
                    endpoint: Endpoint::Publisher(publisher),
                })
            }
            MODE_CONSUMER => {
                let wait = if timeout_ms == 0 {
                    None
                } else {
                    Some(Duration::from_millis(timeout_ms))
                };
                let consumer = consumer_open(&PosixShm, &segment, &expected, &spec, wait)?;
                Ok(AttachedBox {
                    endpoint: Endpoint::Consumer {
                        consumer,
                        state: None,
                    },
                })
            }
            _ => Err(Status::InvalidArg),
        }
    })();
    match opened {
        Ok(handle) => {
            // SAFETY: `out` is non-null (checked by `zero_out`).
            unsafe { *out = Box::into_raw(Box::new(handle)).cast::<drm_attached>() };
            Status::Ok.code()
        }
        Err(status) => status.code(),
    }
}

/// Closes an attached endpoint. A double close is caller UB (I-A2).
/// REQ: REQ-CORE-107.
///
/// # Safety
/// `handle` is null or a handle from [`drm_attached_open`].
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_attached_close(handle: *mut drm_attached) -> drm_status {
    if handle.is_null() {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — handle from `drm_attached_open`.
    drop(unsafe { Box::from_raw(handle.cast::<AttachedBox>()) });
    Status::Ok.code()
}

/// Publishes through an attached publisher (copy form).
/// REQ: REQ-CORE-107, REQ-CORE-102.
///
/// `out_seq` is optional and receives the sequence the sample took.
///
/// # Safety
/// `handle` is live; `data` is null or points to `len` readable bytes.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_attached_publish(
    handle: *mut drm_attached,
    data: *const u8,
    len: usize,
    out_seq: *mut u64,
) -> drm_status {
    if data.is_null() && len != 0 {
        return Status::InvalidArg.code();
    }
    if !out_seq.is_null() {
        // SAFETY: caller contract — writable.
        unsafe { ptr::write(out_seq, 0) };
    }
    // SAFETY: caller contract — `data` readable for `len`.
    let payload: &[u8] = if len == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(data, len) }
    };
    if handle.is_null() {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — live handle.
    let boxed = unsafe { &*handle.cast::<AttachedBox>() };
    let Endpoint::Publisher(publisher) = &boxed.endpoint else {
        return Status::Permission.code();
    };
    let Ok(channel) = AttachedChannel::new(publisher.mapping(), &publisher.view, SLOT_ALIGN) else {
        return Status::InvalidData.code();
    };
    match channel.publish(payload) {
        Ok(seq) => {
            if !out_seq.is_null() {
                // SAFETY: caller contract — writable.
                unsafe { ptr::write(out_seq, seq) };
            }
            Status::Ok.code()
        }
        Err(status) => status.code(),
    }
}

/// Reads the newest committed sample into `buf` (copy form).
/// REQ: REQ-CORE-107, REQ-CORE-103.
///
/// # Safety
/// `handle` is live; `buf` is null or writable for `cap`; `out_len` is writable.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_attached_read_latest(
    handle: *mut drm_attached,
    buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out_len) } {
        return status.code();
    }
    if handle.is_null() || (buf.is_null() && cap != 0) {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — writable for `cap`.
    let dst: &mut [u8] = if cap == 0 {
        &mut []
    } else {
        unsafe { core::slice::from_raw_parts_mut(buf, cap) }
    };
    // SAFETY: caller contract — live handle.
    let boxed = unsafe { &mut *handle.cast::<AttachedBox>() };
    let Endpoint::Consumer { consumer, state } = &mut boxed.endpoint else {
        return Status::Permission.code();
    };
    let Ok(channel) = AttachedChannel::new(consumer.mapping(), &consumer.view, SLOT_ALIGN) else {
        return Status::InvalidData.code();
    };
    let state = state.get_or_insert_with(|| channel.reader());
    match channel.read_latest(state, dst) {
        Ok(ReadOutcome::Sample) => {
            // SAFETY: caller contract — writable.
            unsafe { ptr::write(out_len, dst.len()) };
            Status::Ok.code()
        }
        Ok(ReadOutcome::Empty) => Status::NoSample.code(),
        Ok(ReadOutcome::Gap { .. }) => Status::Gap.code(),
        Ok(ReadOutcome::WouldRetry) => Status::Retry.code(),
        Err(status) => status.code(),
    }
}

/// Reads the next sample after the consumer's cursor (copy form).
/// REQ: REQ-CORE-107, REQ-CORE-103.
///
/// `out_missed` is optional and receives the `Gap` count when the reader fell
/// behind the retained window.
///
/// # Safety
/// As [`drm_attached_read_latest`]; `out_missed` is null or writable.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_attached_read_next(
    handle: *mut drm_attached,
    buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
    out_missed: *mut u64,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out_len) } {
        return status.code();
    }
    if !out_missed.is_null() {
        // SAFETY: caller contract — writable.
        unsafe { ptr::write(out_missed, 0) };
    }
    if handle.is_null() || (buf.is_null() && cap != 0) {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — writable for `cap`.
    let dst: &mut [u8] = if cap == 0 {
        &mut []
    } else {
        unsafe { core::slice::from_raw_parts_mut(buf, cap) }
    };
    // SAFETY: caller contract — live handle.
    let boxed = unsafe { &mut *handle.cast::<AttachedBox>() };
    let Endpoint::Consumer { consumer, state } = &mut boxed.endpoint else {
        return Status::Permission.code();
    };
    let Ok(channel) = AttachedChannel::new(consumer.mapping(), &consumer.view, SLOT_ALIGN) else {
        return Status::InvalidData.code();
    };
    let state = state.get_or_insert_with(|| channel.reader());
    match channel.read_next(state, dst) {
        Ok(ReadOutcome::Sample) => {
            // SAFETY: caller contract — writable.
            unsafe { ptr::write(out_len, dst.len()) };
            Status::Ok.code()
        }
        Ok(ReadOutcome::Empty) => Status::NoSample.code(),
        Ok(ReadOutcome::Gap { missed }) => {
            if !out_missed.is_null() {
                // SAFETY: caller contract — writable.
                unsafe { ptr::write(out_missed, missed) };
            }
            Status::Gap.code()
        }
        Ok(ReadOutcome::WouldRetry) => Status::Retry.code(),
        Err(status) => status.code(),
    }
}
