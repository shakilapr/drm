//! Canonical C ABI crate (codebase.md 2.1 layer `runtime`): `extern "C"` exports
//! over drm-core; `bindings/c/include/drm/drm.h` is the single source of truth.
//! Fail-stop: a Rust panic here terminates the node process (no `Internal`
//! status). Exports use the plain `extern "C"` ABI, which aborts rather than
//! unwinding across the FFI boundary (I-A6; no `catch_unwind`).
//! REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes), REQ-BIND-002
//! (explicit close; per-handle thread-safety), REQ-BIND-020 (fail-stop FFI).
//!
//! # In-process endpoint backing (current phase)
//!
//! The endpoint exports are backed by a **single in-process**
//! `drm_core::channel::Channel` owned by the deployment context created by
//! `drm_context_open`. `drm_publisher_open` / `drm_consumer_open` hand out
//! opaque handles that reference that one shared channel; `drm_publish_copy`
//! and `drm_read_latest` / `drm_read_next` copy through it, and
//! `drm_borrow_latest` / `drm_borrow_release` take a transient,
//! hazard-protected view.
//!
//! This is deliberately NOT the cross-process shared-memory endpoint: there is
//! no segment mapping, no deployment manifest, no fingerprint guard, and the
//! `path` argument of `drm_context_open` is ignored. Cross-process SHM
//! endpoints are a later phase (c-abi-surface.md, section *Protocols*); nothing
//! here should be read as claiming cross-process behaviour. The backing topic
//! is fixed-size (`ENDPOINT_SAMPLE_MAX` bytes): a shorter source publish is
//! zero-padded, and every read delivers the full fixed sample. The backing
//! declares exactly one topic (`ENDPOINT_TOPIC_ID`); any other `topic_id` is
//! undeclared ⇒ `Permission` at open (no deployment manifest exists in this
//! phase, so the declared set is that single topic).
//!
//! Validation runs **once** at endpoint open (I-A1, REQ-BIND-008) and is
//! accounted per context; the steady-state copy operations run zero per-message
//! permission/schema checks. Unsafe is permitted here (codebase.md 3.2).
//!
//! # Close semantics (REQ-BIND-002)
//!
//! Every open has a matching close, and the three closes are symmetric: a null
//! handle ⇒ `InvalidArg`, a live handle ⇒ `Ok` (drop). A second close of the
//! same handle — a *double close* — is caller UB, exactly like any other
//! non-null dangling handle: only NULL is checked, there is **no**
//! handle-generation table, and it is never claimed detectable (drm.h §Handle
//! validity; c-abi-surface.md §Edge Cases "Non-null invalid/dangling pointer").

// ABI identifiers mirror `drm.h`/c-abi-surface.md verbatim (`drm_status`,
// `drm_context`, `drm_borrow_view`, ...) and are intentionally not camel-cased.
// REQ: REQ-BIND-004, REQ-BIND-007.
#![allow(non_camel_case_types)]

use core::cell::UnsafeCell;
use core::ffi::{c_char, c_void};
use core::ptr;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome, ReaderState};
use drm_core::hazard::{Borrow, acquire};
use drm_types::Status;

/// Fixed `int32_t` status mirror of `drm_types::Status` (I-A4/I-A9): explicit
/// values, no `repr(C)` fieldless enum, unknown codes stay representable.
/// REQ: REQ-BIND-007, REQ-BIND-017.
pub type drm_status = i32;

/// Opaque deployment context handle; the C mirror is `struct drm_context`.
/// REQ: REQ-BIND-004, REQ-BIND-021.
pub enum drm_context {}

/// Opaque publisher endpoint handle. REQ: REQ-BIND-004.
pub enum drm_publisher {}

/// Opaque consumer endpoint handle. REQ: REQ-BIND-004.
pub enum drm_consumer {}

/// Caller-owned borrow view; mirrors `drm_borrow_view` in `drm.h` layout
/// exactly. `data` points into the in-process backing's local storage and is
/// valid until the matching `drm_borrow_release`.
/// REQ: REQ-BIND-001, REQ-BIND-004.
#[repr(C)]
pub struct drm_borrow_view {
    /// Pointer to the committed sample; valid until release (`0` if none).
    pub data: *const c_void,
    /// Committed canonical sample length in bytes (zero on failure).
    pub len: usize,
    /// Sequence of the borrowed sample.
    pub sequence: u64,
    /// Opaque single-outstanding token; `0` means invalid.
    pub token: u32,
}

/// Fixed canonical sample size of the in-process backing topic, in bytes.
/// REQ: REQ-BIND-005, REQ-BIND-019.
pub const ENDPOINT_SAMPLE_MAX: usize = 64;

/// The one topic the in-process backing declares. Any other `topic_id` is
/// undeclared ⇒ `Permission` at endpoint open (c-abi-surface.md §Edge Cases:
/// "Undeclared topic at open ⇒ Permission"). The backing serves a single shared
/// channel, so exactly one topic is declared. REQ: REQ-BIND-008.
pub const ENDPOINT_TOPIC_ID: u32 = 0;

/// Maximum number of concurrently open consumer endpoints (borrower records)
/// the in-process context can lease. REQ: REQ-BIND-010.
pub const ENDPOINT_BORROWERS: usize = 8;

/// In-process channel history depth R. REQ: REQ-BIND-004.
const ENDPOINT_DEPTH: u64 = 8;

/// In-process channel pool size (slots). REQ: REQ-BIND-004.
const ENDPOINT_POOL: u64 = 16;

/// In-process slot alignment. REQ: REQ-BIND-004.
const ENDPOINT_SLOT_ALIGN: u64 = 8;

/// Bounded read budget per copy call (cursor-delivery I-C10; matches the core
/// ergonomic wrappers). REQ: REQ-BIND-003.
const RETRY_BUDGET: u32 = 4;

/// Deployment context backing store: owns the single in-process channel and the
/// per-context validate-once accounting. Boxed so the channel address handed to
/// endpoints stays stable.
/// REQ: REQ-BIND-008, REQ-BIND-021.
struct ContextBox {
    channel: Channel,
    validation_checks: AtomicU64,
    per_message_checks: AtomicU64,
    next_borrower: AtomicUsize,
}

/// Publisher endpoint backing store: a non-owning reference to the context's
/// channel (the context outlives every derived endpoint, I-A7).
/// REQ: REQ-BIND-002, REQ-BIND-004.
struct PublisherBox {
    channel: *const Channel,
}

/// Consumer endpoint backing store: a non-owning channel reference, a claimed
/// borrower record, a process-local cursor, and at most one outstanding borrow.
/// REQ: REQ-BIND-006, REQ-BIND-010.
struct ConsumerBox {
    channel: *const Channel,
    rec_index: usize,
    reader: UnsafeCell<ReaderState>,
    outstanding: UnsafeCell<Option<(u32, Borrow)>>,
    next_token: AtomicU32,
}

/// Zero an out-parameter before any fallible step (I-A2): a null out-param is
/// `InvalidArg`; otherwise the pointee is overwritten with zeros so an ignored
/// status can never observe a stale pointer or token.
/// REQ: REQ-BIND-005.
///
/// # Safety
/// `out` must be null or point to a valid, writable `T`.
unsafe fn zero_out<T>(out: *mut T) -> Result<(), Status> {
    if out.is_null() {
        return Err(Status::InvalidArg);
    }
    // SAFETY: caller contract — `out` is valid for a single `T` write.
    unsafe { ptr::write_bytes(out, 0, 1) };
    Ok(())
}

/// ABI version probe; mirrors `DRM_ABI_VERSION` in `drm.h` (bump on layout
/// change only). REQ: REQ-BIND-018.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn drm_abi_version() -> u32 {
    drm_types::ABI_VERSION
}

/// Open the deployment context and activate the in-process backing channel.
/// REQ: REQ-BIND-021, REQ-BIND-005, REQ-BIND-008.
///
/// `out` is zeroed first (null `out` ⇒ `InvalidArg`). `path` is accepted but
/// ignored in this phase (no manifest/fingerprint load; in-process backing).
///
/// # Safety
/// `_path` is null or a valid NUL-terminated C string; `out` is null or points
/// to a writable `*mut drm_context`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_context_open(
    _path: *const c_char,
    out: *mut *mut drm_context,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    // Fixed-size, overwrite-policy topic: no descriptor/schema validation and
    // no manifest authorization yet — storage is fixed at activation.
    let sample_max = u64::try_from(ENDPOINT_SAMPLE_MAX).unwrap_or(0);
    match Channel::activate(
        ENDPOINT_DEPTH,
        ENDPOINT_POOL,
        sample_max,
        ENDPOINT_SLOT_ALIGN,
        OverflowPolicy::Overwrite,
        0,
        ENDPOINT_BORROWERS,
        None,
    ) {
        Ok(channel) => {
            let ctx = Box::new(ContextBox {
                channel,
                validation_checks: AtomicU64::new(0),
                per_message_checks: AtomicU64::new(0),
                next_borrower: AtomicUsize::new(0),
            });
            // SAFETY: `out` is non-null (checked by `zero_out`).
            unsafe { *out = Box::into_raw(ctx).cast::<drm_context>() };
            Status::Ok.code()
        }
        Err(status) => status.code(),
    }
}

/// Close a deployment context. REQ: REQ-BIND-002, REQ-BIND-021.
///
/// Null `ctx` ⇒ `InvalidArg`. The caller must have closed every endpoint
/// derived from the context first (I-A7); a non-null dangling handle is caller
/// UB (I-A2) — this includes a second close of the same handle (double close);
/// no handle-generation table, never claimed detectable.
///
/// # Safety
/// `ctx` is null or a live handle returned by `drm_context_open`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_context_close(ctx: *mut drm_context) -> drm_status {
    if ctx.is_null() {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — `ctx` is a handle from `drm_context_open`.
    drop(unsafe { Box::from_raw(ctx.cast::<ContextBox>()) });
    Status::Ok.code()
}

/// Open a publisher endpoint on `topic_id`. REQ: REQ-BIND-002, REQ-BIND-008.
///
/// `out` is zeroed first (null `out` ⇒ `InvalidArg`); a null `ctx` ⇒
/// `InvalidArg`. An undeclared `topic_id` ⇒ `Permission` (c-abi-surface.md
/// §Edge Cases); validation runs exactly once here (I-A1); the in-process
/// backing declares only `ENDPOINT_TOPIC_ID`.
///
/// # Safety
/// `ctx` is null or a live context handle; `out` is null or points to a
/// writable `*mut drm_publisher`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_publisher_open(
    ctx: *mut drm_context,
    topic_id: u32,
    out: *mut *mut drm_publisher,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    if ctx.is_null() {
        return Status::InvalidArg.code();
    }
    // Undeclared topic ⇒ Permission, before the validate-once accounting
    // (c-abi-surface.md §Protocols "Undeclared topic at open ⇒ Permission").
    if topic_id != ENDPOINT_TOPIC_ID {
        return Status::Permission.code();
    }
    // SAFETY: caller contract — `ctx` is a live handle.
    let ctx = unsafe { &*ctx.cast::<ContextBox>() };
    // Validate-once (I-A1 / REQ-BIND-008): authorization + descriptor checks
    // execute here, once per open; steady-state publishes run none.
    ctx.validation_checks.fetch_add(1, Ordering::Relaxed);
    let publisher = Box::new(PublisherBox {
        channel: ptr::from_ref(&ctx.channel),
    });
    // SAFETY: `out` is non-null (checked by `zero_out`).
    unsafe { *out = Box::into_raw(publisher).cast::<drm_publisher>() };
    Status::Ok.code()
}

/// Close a publisher endpoint. REQ: REQ-BIND-002.
///
/// Null `publisher` ⇒ `InvalidArg`; otherwise the handle is reclaimed. A second
/// close of the same handle (double close) is caller UB (I-A2); no
/// handle-generation table.
///
/// # Safety
/// `publisher` is null or a handle returned by `drm_publisher_open`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_publisher_close(publisher: *mut drm_publisher) -> drm_status {
    if publisher.is_null() {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — handle from `drm_publisher_open`.
    drop(unsafe { Box::from_raw(publisher.cast::<PublisherBox>()) });
    Status::Ok.code()
}

/// Copy publish on `publisher` (channel-publish.md copy form).
/// REQ: REQ-BIND-002, REQ-BIND-008.
///
/// A null `publisher`, or `data == NULL && len != 0`, is `InvalidArg`; a `len`
/// larger than the fixed sample size is `InvalidArg` before any slice exists
/// (I-A2). A shorter source is zero-padded to the fixed sample size. Every core
/// `Status` (e.g. `QueueFull`, `PoolExhausted`) propagates verbatim, with zero
/// per-message permission/schema checks.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_publish_copy(
    publisher: *mut drm_publisher,
    data: *const u8,
    len: usize,
) -> drm_status {
    if publisher.is_null() || (data.is_null() && len != 0) {
        return Status::InvalidArg.code();
    }
    if len > ENDPOINT_SAMPLE_MAX {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — `data` is valid for `len` bytes.
    let payload: &[u8] = if len == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(data, len) }
    };
    let mut padded = [0_u8; ENDPOINT_SAMPLE_MAX];
    let Some(head) = padded.get_mut(..len) else {
        return Status::InvalidArg.code();
    };
    head.copy_from_slice(payload);
    // SAFETY: caller contract — `publisher` from `drm_publisher_open`.
    let publisher = unsafe { &*publisher.cast::<PublisherBox>() };
    // SAFETY: the channel is owned by the live context, which outlives it.
    let channel = unsafe { &*publisher.channel };
    // Zero per-message permission/schema checks (I-A1); the core publish runs
    // only its own structural/sample checks.
    match channel.publish(&padded) {
        Ok(_) => Status::Ok.code(),
        Err(status) => status.code(),
    }
}

/// Open a consumer endpoint on `topic_id`. REQ: REQ-BIND-002, REQ-BIND-010.
///
/// `out` is zeroed first (null `out` ⇒ `InvalidArg`); a null `ctx` ⇒
/// `InvalidArg`. An undeclared `topic_id` ⇒ `Permission` (c-abi-surface.md
/// §Edge Cases). The open claims one borrower record; when the
/// `ENDPOINT_BORROWERS` lease is exhausted it returns `Busy` (I-A7). Validation
/// runs exactly once here (I-A1).
///
/// # Safety
/// `ctx` is null or a live context handle; `out` is null or points to a
/// writable `*mut drm_consumer`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_consumer_open(
    ctx: *mut drm_context,
    topic_id: u32,
    out: *mut *mut drm_consumer,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    if ctx.is_null() {
        return Status::InvalidArg.code();
    }
    // Undeclared topic ⇒ Permission (c-abi-surface.md §Edge Cases), before the
    // borrower-record lease is claimed.
    if topic_id != ENDPOINT_TOPIC_ID {
        return Status::Permission.code();
    }
    // SAFETY: caller contract — `ctx` is a live handle.
    let ctx = unsafe { &*ctx.cast::<ContextBox>() };
    let rec_index = ctx.next_borrower.fetch_add(1, Ordering::AcqRel);
    if rec_index >= ENDPOINT_BORROWERS {
        // No vacant borrower record ⇒ Busy (REQ-BIND-010). Leases are not
        // reclaimed within a context in this phase.
        return Status::Busy.code();
    }
    ctx.validation_checks.fetch_add(1, Ordering::Relaxed);
    let consumer = Box::new(ConsumerBox {
        channel: ptr::from_ref(&ctx.channel),
        rec_index,
        reader: UnsafeCell::new(ctx.channel.reader()),
        outstanding: UnsafeCell::new(None),
        next_token: AtomicU32::new(1),
    });
    // SAFETY: `out` is non-null (checked by `zero_out`).
    unsafe { *out = Box::into_raw(consumer).cast::<drm_consumer>() };
    Status::Ok.code()
}

/// Close a consumer endpoint. REQ: REQ-BIND-002, REQ-BIND-001.
///
/// Null `consumer` ⇒ `InvalidArg`. An outstanding borrow at close ⇒ `Busy`
/// (consumer-registration I-CR4): the endpoint stays claimed and the handle is
/// returned intact so the caller can `drm_borrow_release`, then close. A second
/// close of the same handle (double close) is caller UB (I-A2); no
/// handle-generation table.
///
/// # Safety
/// `consumer` is null or a handle returned by `drm_consumer_open`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_consumer_close(consumer: *mut drm_consumer) -> drm_status {
    if consumer.is_null() {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — handle from `drm_consumer_open`.
    let mut boxed = unsafe { Box::from_raw(consumer.cast::<ConsumerBox>()) };
    if boxed.outstanding.get_mut().is_some() {
        // Outstanding borrow: refuse (I-CR4) and hand the handle back intact so
        // the caller can release, then close. Never drop a live borrow here.
        let _ = Box::into_raw(boxed);
        return Status::Busy.code();
    }
    drop(boxed);
    Status::Ok.code()
}

/// Take a transient, hazard-protected view of the newest committed sample.
/// REQ: REQ-BIND-001, REQ-BIND-006.
///
/// `out` is zeroed first (null `out` ⇒ `InvalidArg`); a null `consumer` ⇒
/// `InvalidArg`. A second call before release ⇒ `Busy` (one outstanding borrow
/// per endpoint, I-A5). Empty channel ⇒ `NoSample`. On success the view carries
/// a non-zero token that must be passed to [`drm_borrow_release`].
///
/// # Safety
/// `consumer` is null or a live consumer handle; `out` is null or points to a
/// writable `drm_borrow_view`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_borrow_latest(
    consumer: *mut drm_consumer,
    out: *mut drm_borrow_view,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    if consumer.is_null() {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — `consumer` is a live handle.
    let consumer = unsafe { &*consumer.cast::<ConsumerBox>() };
    // SAFETY: the channel is owned by the live context, which outlives it.
    let channel = unsafe { &*consumer.channel };
    // SAFETY: single-borrower endpoint; the cell is touched only here/release.
    let outstanding = unsafe { &mut *consumer.outstanding.get() };
    if outstanding.is_some() {
        return Status::Busy.code();
    }
    let ws = channel.write_seq();
    let fv = channel.ccb().first_valid_seq.load(Ordering::Acquire);
    if ws <= fv {
        return Status::NoSample.code();
    }
    let Some(rec) = channel.rec(consumer.rec_index) else {
        return Status::InvalidArg.code();
    };
    let borrow = match acquire(channel.ring(), channel.ccb(), ws - 1, rec) {
        Ok(borrow) => borrow,
        Err(err) => return Status::from(err).code(),
    };
    let payload = channel.payload(&borrow.entry);
    let token = consumer.next_token.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `out` is non-null and valid for one `drm_borrow_view` write.
    unsafe {
        (*out).data = payload.as_ptr().cast::<c_void>();
        (*out).len = payload.len();
        (*out).sequence = borrow.entry.sequence;
        (*out).token = token;
    }
    *outstanding = Some((token, borrow));
    Status::Ok.code()
}

/// Release a borrow taken by [`drm_borrow_latest`].
/// REQ: REQ-BIND-001, REQ-BIND-006.
///
/// Null `consumer` or `token == 0` ⇒ `InvalidArg`. A release with no
/// outstanding borrow (double release) or a token mismatch ⇒ `InvalidArg`.
///
/// # Safety
/// `consumer` is null or a live consumer handle.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_borrow_release(consumer: *mut drm_consumer, token: u32) -> drm_status {
    if consumer.is_null() || token == 0 {
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — `consumer` is a live handle.
    let consumer = unsafe { &*consumer.cast::<ConsumerBox>() };
    // SAFETY: single-borrower endpoint; the cell is touched only here/borrow.
    let outstanding = unsafe { &mut *consumer.outstanding.get() };
    match outstanding {
        Some((held, _)) if *held == token => {
            // Dropping the guard clears the hazard slot.
            *outstanding = None;
            Status::Ok.code()
        }
        // No outstanding borrow (double release) or token mismatch.
        _ => Status::InvalidArg.code(),
    }
}

/// Token of the borrow currently outstanding on `consumer`, or `0` when none.
/// Rust-native diagnostic for the Python forced-GC hazard-free probe
/// (bindings-cpp-rust-python.md I-B5/I-B6); not part of the C ABI. A non-zero
/// result means a hazard slot is pinned and must be released with
/// [`drm_borrow_release`]. REQ: REQ-BIND-006, REQ-BIND-015.
///
/// # Safety
/// `consumer` is null or a live consumer handle.
#[must_use]
pub unsafe fn drm_borrow_outstanding_token(consumer: *const drm_consumer) -> u32 {
    if consumer.is_null() {
        return 0;
    }
    // SAFETY: caller contract — `consumer` is a live handle.
    let consumer = unsafe { &*consumer.cast::<ConsumerBox>() };
    // SAFETY: single-borrower endpoint; the cell is read under the same
    // single-thread discipline as borrow/release.
    unsafe { &*consumer.outstanding.get() }
        .as_ref()
        .map_or(0, |(token, _)| *token)
}

/// Copy the newest committed sample into a caller buffer.
/// REQ: REQ-BIND-002, REQ-BIND-008.
///
/// `out_len` is zeroed first (null ⇒ `InvalidArg`); a null `consumer`, or
/// `buf == NULL && cap != 0`, is `InvalidArg`. When `cap` is smaller than the
/// fixed sample size the call returns `InvalidArg` with `*out_len` set to the
/// required size and copies nothing (no partial copy, I-C9); a `cap == 0` call
/// is therefore a size query. Empty channel ⇒ `NoSample`. The read is bounded
/// (I-C10): budget exhaustion ⇒ `Retry`.
///
/// # Safety
/// `consumer` is null or a live consumer handle; `buf` is null or points to
/// `cap` writable bytes; `out_len` is null or points to a writable `usize`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_read_latest(
    consumer: *mut drm_consumer,
    buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out_len) } {
        return status.code();
    }
    if consumer.is_null() || (buf.is_null() && cap != 0) {
        return Status::InvalidArg.code();
    }
    if cap < ENDPOINT_SAMPLE_MAX {
        // Informative size query: report the required size, copy nothing.
        // SAFETY: `out_len` is non-null (checked by `zero_out`).
        unsafe { *out_len = ENDPOINT_SAMPLE_MAX };
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — `consumer` is a live handle.
    let consumer = unsafe { &*consumer.cast::<ConsumerBox>() };
    // SAFETY: the channel is owned by the live context, which outlives it.
    let channel = unsafe { &*consumer.channel };
    // SAFETY: `buf` is non-null (cap >= 64) and valid for `cap >= required`.
    let dst = unsafe { core::slice::from_raw_parts_mut(buf, ENDPOINT_SAMPLE_MAX) };
    // SAFETY: single-borrower endpoint; the cursor cell is touched only here.
    let reader = unsafe { &mut *consumer.reader.get() };
    match channel.try_latest(reader, consumer.rec_index, dst, RETRY_BUDGET) {
        Ok(ReadOutcome::Sample) => {
            // SAFETY: `out_len` is non-null (checked by `zero_out`).
            unsafe { *out_len = ENDPOINT_SAMPLE_MAX };
            Status::Ok.code()
        }
        Ok(ReadOutcome::Empty) => Status::NoSample.code(),
        Ok(ReadOutcome::Gap { .. }) => Status::Gap.code(),
        Ok(ReadOutcome::WouldRetry) => Status::Retry.code(),
        Err(status) => status.code(),
    }
}

/// Copy the next in-order committed sample into a caller buffer.
/// REQ: REQ-BIND-002, REQ-BIND-008.
///
/// Same rules as [`drm_read_latest`], plus lapped-sequential accounting: a
/// lapped cursor returns `Gap` with the exact `*out_missed` count and copies
/// nothing (I-C2). Both `out_len` and `out_missed` are zeroed first (a null
/// ⇒ `InvalidArg`).
///
/// # Safety
/// `consumer` is null or a live consumer handle; `buf` is null or points to
/// `cap` writable bytes; `out_len`/`out_missed` are null or point to writable
/// `usize`/`u64`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_read_next(
    consumer: *mut drm_consumer,
    buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
    out_missed: *mut u64,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out_len) } {
        return status.code();
    }
    if let Err(status) = unsafe { zero_out(out_missed) } {
        return status.code();
    }
    if consumer.is_null() || (buf.is_null() && cap != 0) {
        return Status::InvalidArg.code();
    }
    if cap < ENDPOINT_SAMPLE_MAX {
        // SAFETY: `out_len` is non-null (checked by `zero_out`).
        unsafe { *out_len = ENDPOINT_SAMPLE_MAX };
        return Status::InvalidArg.code();
    }
    // SAFETY: caller contract — `consumer` is a live handle.
    let consumer = unsafe { &*consumer.cast::<ConsumerBox>() };
    // SAFETY: the channel is owned by the live context, which outlives it.
    let channel = unsafe { &*consumer.channel };
    // SAFETY: `buf` is non-null (cap >= 64) and valid for `cap >= required`.
    let dst = unsafe { core::slice::from_raw_parts_mut(buf, ENDPOINT_SAMPLE_MAX) };
    // SAFETY: single-borrower endpoint; the cursor cell is touched only here.
    let reader = unsafe { &mut *consumer.reader.get() };
    match channel.try_next(reader, consumer.rec_index, dst, RETRY_BUDGET) {
        Ok(ReadOutcome::Sample) => {
            // SAFETY: `out_len` is non-null (checked by `zero_out`).
            unsafe { *out_len = ENDPOINT_SAMPLE_MAX };
            Status::Ok.code()
        }
        Ok(ReadOutcome::Empty) => Status::NoSample.code(),
        Ok(ReadOutcome::Gap { missed }) => {
            // SAFETY: `out_missed` is non-null (checked by `zero_out`).
            unsafe { *out_missed = missed };
            Status::Gap.code()
        }
        Ok(ReadOutcome::WouldRetry) => Status::Retry.code(),
        Err(status) => status.code(),
    }
}

/// Validate-once accounting for `ctx`: number of authorization/descriptor
/// checks recorded since open. It advances once per successful endpoint open
/// and never on the publish/read hot path (I-A1). Diagnostic only.
/// REQ: REQ-BIND-008.
///
/// # Safety
/// `ctx` is null or a live handle returned by `drm_context_open`.
#[must_use]
pub unsafe fn drm_context_validation_checks(ctx: *const drm_context) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    // SAFETY: caller contract — `ctx` is a live handle.
    unsafe {
        (*ctx.cast::<ContextBox>())
            .validation_checks
            .load(Ordering::Relaxed)
    }
}

/// Steady-state per-message permission/schema check counter for `ctx`; it stays
/// zero because validation runs only at open (I-A1). Diagnostic only.
/// REQ: REQ-BIND-008.
///
/// # Safety
/// `ctx` is null or a live handle returned by `drm_context_open`.
#[must_use]
pub unsafe fn drm_context_per_message_checks(ctx: *const drm_context) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    // SAFETY: caller contract — `ctx` is a live handle.
    unsafe {
        (*ctx.cast::<ContextBox>())
            .per_message_checks
            .load(Ordering::Relaxed)
    }
}

/// Rust-native view of the in-process backing channel for `ctx`. Diagnostic /
/// parity-harness accessor (not part of the C ABI).
/// REQ: REQ-BIND-004, REQ-BIND-021.
///
/// # Safety
/// `ctx` is null or a live handle; the returned pointer is valid until that
/// handle is passed to `drm_context_close`.
#[must_use]
pub unsafe fn drm_context_channel(ctx: *const drm_context) -> *const Channel {
    if ctx.is_null() {
        return ptr::null();
    }
    // SAFETY: caller contract — `ctx` is a live handle.
    unsafe { ptr::from_ref(&(*ctx.cast::<ContextBox>()).channel) }
}

#[cfg(test)]
mod tests {
    use super::drm_borrow_view;
    use drm_types::Status;

    /// REQ: REQ-BIND-004, REQ-BIND-001 — the exported `drm_borrow_view` mirrors
    /// the `drm.h` layout `{const void *, size_t, uint64_t, uint32_t}` with
    /// natural alignment padding (64-bit target).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn borrow_view_layout_matches_header() {
        use core::mem::{align_of, offset_of, size_of};
        assert_eq!(size_of::<drm_borrow_view>(), 32);
        assert_eq!(align_of::<drm_borrow_view>(), 8);
        assert_eq!(offset_of!(drm_borrow_view, data), 0);
        assert_eq!(offset_of!(drm_borrow_view, len), 8);
        assert_eq!(offset_of!(drm_borrow_view, sequence), 16);
        assert_eq!(offset_of!(drm_borrow_view, token), 24);
    }

    /// REQ: REQ-BIND-007 — compile smoke for the surface.
    #[test]
    fn skeleton_compiles() {
        assert_eq!(Status::Busy.code(), -11);
        assert_eq!(Status::NotImplemented.code(), -22);
    }

    /// REQ: REQ-BIND-006, REQ-BIND-015 — the Rust-native diagnostic reports a
    /// pinned borrow token and clears to zero after release.
    #[test]
    fn borrow_outstanding_token_reports_pin() {
        use super::{
            drm_borrow_latest, drm_borrow_outstanding_token, drm_borrow_release, drm_borrow_view,
            drm_consumer, drm_consumer_close, drm_consumer_open, drm_context, drm_context_close,
            drm_context_open, drm_publish_copy, drm_publisher, drm_publisher_close,
            drm_publisher_open,
        };
        let mut ctx: *mut drm_context = std::ptr::null_mut();
        assert_eq!(
            unsafe { drm_context_open(std::ptr::null(), &raw mut ctx) },
            0
        );
        let mut publisher: *mut drm_publisher = std::ptr::null_mut();
        assert_eq!(unsafe { drm_publisher_open(ctx, 0, &raw mut publisher) }, 0);
        assert_eq!(unsafe { drm_publish_copy(publisher, b"x".as_ptr(), 1) }, 0);
        let mut consumer: *mut drm_consumer = std::ptr::null_mut();
        assert_eq!(unsafe { drm_consumer_open(ctx, 0, &raw mut consumer) }, 0);
        assert_eq!(unsafe { drm_borrow_outstanding_token(consumer) }, 0);
        let mut view = drm_borrow_view {
            data: std::ptr::null(),
            len: 0,
            sequence: 0,
            token: 0,
        };
        assert_eq!(unsafe { drm_borrow_latest(consumer, &raw mut view) }, 0);
        assert_eq!(
            unsafe { drm_borrow_outstanding_token(consumer) },
            view.token
        );
        assert_ne!(view.token, 0);
        assert_eq!(unsafe { drm_borrow_release(consumer, view.token) }, 0);
        assert_eq!(unsafe { drm_borrow_outstanding_token(consumer) }, 0);
        assert_eq!(unsafe { drm_consumer_close(consumer) }, 0);
        assert_eq!(unsafe { drm_publisher_close(publisher) }, 0);
        assert_eq!(unsafe { drm_context_close(ctx) }, 0);
    }

    /// REQ: REQ-BIND-007 — I-A4 exhaustive 1:1 map: the `DRM_OK`/`DRM_ERR_*`
    /// codes parsed from `drm.h` equal the `Status::code()` set exactly, in
    /// both directions. `status_codes()` is an exhaustive `match` (no
    /// wildcard), so a new `Status` variant without a header row breaks this
    /// test at compile time; a header-only addition breaks the set compare.
    #[test]
    fn drm_h_status_map_exhaustive() {
        const HEADER: &str = include_str!("../../../bindings/c/include/drm/drm.h");
        let mut c_codes: Vec<i32> = Vec::new();
        for line in HEADER.lines() {
            let Some((name, value)) = line.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            if !(name.starts_with("DRM_OK") || name.starts_with("DRM_ERR_")) {
                continue;
            }
            let code: i32 = value
                .trim()
                .trim_end_matches(',')
                .parse()
                .unwrap_or_else(|e| unreachable!("drm.h `{name}`: {e:?}"));
            assert!(
                !c_codes.contains(&code),
                "duplicate drm.h code {code} ({name})"
            );
            c_codes.push(code);
        }

        let rust_codes = status_codes();
        assert_eq!(c_codes.len(), rust_codes.len(), "drm.h vs Status count");
        for c in &rust_codes {
            assert!(c_codes.contains(c), "Status code {c} missing from drm.h");
        }
        for c in &c_codes {
            assert!(rust_codes.contains(c), "drm.h code {c} missing from Status");
        }
    }

    /// REQ: REQ-BIND-004, REQ-BIND-021 — the header declares the opaque handles
    /// and every lifecycle export; a dropped declaration is a contract drift
    /// the C compile fixture alone cannot detect.
    #[test]
    fn drm_h_declares_stub_surface() {
        const HEADER: &str = include_str!("../../../bindings/c/include/drm/drm.h");
        for needle in [
            "typedef struct drm_context drm_context;",
            "typedef struct drm_publisher drm_publisher;",
            "typedef struct drm_consumer drm_consumer;",
            "int32_t drm_abi_version(void);",
            "drm_status drm_context_open(",
            "drm_status drm_context_close(",
            "drm_status drm_publisher_open(",
            "drm_status drm_publisher_close(",
            "drm_status drm_consumer_open(",
            "drm_status drm_consumer_close(",
        ] {
            assert!(HEADER.contains(needle), "drm.h missing `{needle}`");
        }
    }

    /// Exhaustive (no wildcard) so adding a `Status` variant without a header
    /// row fails compilation of this test.
    fn status_codes() -> Vec<i32> {
        use drm_types::Status as S;
        let all = [
            S::Ok,
            S::InvalidArg,
            S::NotFound,
            S::AlreadyExists,
            S::Permission,
            S::QueueFull,
            S::Gap,
            S::Stale,
            S::WrongAbi,
            S::PoolExhausted,
            S::Retry,
            S::Busy,
            S::NoSample,
            S::InvalidBorrow,
            S::GenerationExhausted,
            S::InvalidData,
            S::InvalidConfig,
            S::NotReady,
            S::LockFailed,
            S::SequenceExhausted,
            S::EpochExhausted,
            S::RingStampExhausted,
            S::NotImplemented,
        ];
        let mut codes = Vec::new();
        for s in all {
            // Or-pattern without wildcard: exhaustive against future variants.
            codes.push(match s {
                S::Ok
                | S::InvalidArg
                | S::NotFound
                | S::AlreadyExists
                | S::Permission
                | S::QueueFull
                | S::Gap
                | S::Stale
                | S::WrongAbi
                | S::PoolExhausted
                | S::Retry
                | S::Busy
                | S::NoSample
                | S::InvalidBorrow
                | S::GenerationExhausted
                | S::InvalidData
                | S::InvalidConfig
                | S::NotReady
                | S::LockFailed
                | S::SequenceExhausted
                | S::EpochExhausted
                | S::RingStampExhausted
                | S::NotImplemented => s.code(),
            });
        }
        codes
    }
}
