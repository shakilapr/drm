//! In-process C ABI endpoint behaviour (c-abi-surface.md test matrix):
//! validate-once accounting, borrow discipline, copy reads (incl. `Gap`), and a
//! C-driven vs Rust-native byte-parity check over one shared topic. The backing
//! is in-process (a single `drm_core::channel::Channel`); cross-process SHM
//! endpoints are a later phase.
//! REQ: REQ-BIND-001, REQ-BIND-002, REQ-BIND-005, REQ-BIND-006, REQ-BIND-008.

use core::ptr;

use drm_core::channel::ReadOutcome;
use drm_ffi::{
    ENDPOINT_SAMPLE_MAX, drm_borrow_latest, drm_borrow_release, drm_borrow_view, drm_consumer,
    drm_consumer_close, drm_consumer_open, drm_context, drm_context_channel, drm_context_close,
    drm_context_open, drm_context_per_message_checks, drm_context_validation_checks,
    drm_publish_copy, drm_publisher, drm_publisher_close, drm_publisher_open, drm_read_latest,
    drm_read_next,
};
use drm_types::Status;

/// Non-null placeholder handle overwritten by a successful open.
fn dangling<T>() -> *mut T {
    ptr::NonNull::<T>::dangling().as_ptr()
}

/// Zeroed borrow view for an out-parameter slot.
fn empty_view() -> drm_borrow_view {
    drm_borrow_view {
        data: ptr::null(),
        len: 0,
        sequence: 0,
        token: 0,
    }
}

/// Open one context plus a publisher and a consumer on it.
fn open_all() -> (*mut drm_context, *mut drm_publisher, *mut drm_consumer) {
    let mut ctx: *mut drm_context = dangling();
    assert_eq!(
        unsafe { drm_context_open(ptr::null(), &raw mut ctx) },
        Status::Ok.code()
    );
    let mut publisher: *mut drm_publisher = dangling();
    assert_eq!(
        unsafe { drm_publisher_open(ctx, 0, &raw mut publisher) },
        Status::Ok.code()
    );
    let mut consumer: *mut drm_consumer = dangling();
    assert_eq!(
        unsafe { drm_consumer_open(ctx, 0, &raw mut consumer) },
        Status::Ok.code()
    );
    (ctx, publisher, consumer)
}

/// Close the three handles in contract order (endpoints before context).
fn close_all(ctx: *mut drm_context, publisher: *mut drm_publisher, consumer: *mut drm_consumer) {
    assert_eq!(unsafe { drm_publisher_close(publisher) }, Status::Ok.code());
    assert_eq!(unsafe { drm_consumer_close(consumer) }, Status::Ok.code());
    assert_eq!(unsafe { drm_context_close(ctx) }, Status::Ok.code());
}

/// REQ: REQ-BIND-008 — validation runs exactly once per endpoint open; 10k
/// steady-state publishes add zero validation checks and zero per-message
/// checks (I-A1).
#[test]
fn validate_once_at_open_across_many_publishes() {
    let (ctx, publisher, consumer) = open_all();
    // Two opens (publisher + consumer) ⇒ two validate-once checks.
    assert_eq!(
        unsafe { drm_context_validation_checks(ctx) },
        2,
        "exactly one check per endpoint open"
    );

    let payload = [0x5A_u8; ENDPOINT_SAMPLE_MAX];
    for _ in 0..10_000 {
        assert_eq!(
            unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
            Status::Ok.code()
        );
    }

    assert_eq!(
        unsafe { drm_context_validation_checks(ctx) },
        2,
        "the hot path performs no authorization/descriptor checks"
    );
    assert_eq!(
        unsafe { drm_context_per_message_checks(ctx) },
        0,
        "zero per-message checks"
    );
    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-001, REQ-BIND-006 — borrow discipline: one outstanding borrow
/// per endpoint, explicit release, double release ⇒ `InvalidArg`, and a close
/// with an outstanding borrow counts one debug leak.
#[test]
fn borrow_double_release_is_invalid_arg() {
    let (ctx, publisher, consumer) = open_all();
    let payload = [0x11_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
        Status::Ok.code()
    );

    let mut view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, &raw mut view) },
        Status::Ok.code()
    );
    assert!(!view.data.is_null());
    assert_eq!(view.len, ENDPOINT_SAMPLE_MAX);
    assert_ne!(view.token, 0);
    let token = view.token;
    // The borrowed bytes are exactly what was published.
    let borrowed = unsafe { core::slice::from_raw_parts(view.data.cast::<u8>(), view.len) };
    assert_eq!(borrowed, &payload[..]);

    // A second borrow before release is Busy (one outstanding per endpoint).
    let mut second = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, &raw mut second) },
        Status::Busy.code()
    );

    // Explicit release, then the double release is InvalidArg.
    assert_eq!(
        unsafe { drm_borrow_release(consumer, token) },
        Status::Ok.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(consumer, token) },
        Status::InvalidArg.code()
    );
    // Token 0 and a null handle are InvalidArg.
    assert_eq!(
        unsafe { drm_borrow_release(consumer, 0) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(ptr::null_mut(), token) },
        Status::InvalidArg.code()
    );

    // Re-borrow succeeds once the previous borrow is released.
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, &raw mut second) },
        Status::Ok.code()
    );

    // Close with an outstanding borrow is refused (I-CR4): the endpoint stays
    // claimed; release first, then close succeeds.
    assert_eq!(unsafe { drm_consumer_close(consumer) }, Status::Busy.code());
    assert_eq!(
        unsafe { drm_borrow_release(consumer, second.token) },
        Status::Ok.code()
    );
    assert_eq!(unsafe { drm_consumer_close(consumer) }, Status::Ok.code());

    assert_eq!(unsafe { drm_publisher_close(publisher) }, Status::Ok.code());
    assert_eq!(unsafe { drm_context_close(ctx) }, Status::Ok.code());
}

/// REQ: REQ-BIND-005, REQ-BIND-008 — C-driven vs Rust-native byte parity over
/// one shared topic: C publish ⇒ Rust read, Rust publish ⇒ C read, identical
/// bytes both directions.
#[test]
fn c_rust_byte_parity() {
    let (ctx, publisher, consumer) = open_all();
    let channel_ptr = unsafe { drm_context_channel(ctx) };
    assert!(!channel_ptr.is_null());
    // SAFETY: the context (and its channel) outlives this test body.
    let channel = unsafe { &*channel_ptr };

    // Direction 1: publish through the C ABI, read through the Rust-native API.
    let c_pattern = [0xA1_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, c_pattern.as_ptr(), c_pattern.len()) },
        Status::Ok.code()
    );
    let mut native = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut state = channel.reader();
    assert_eq!(
        channel.latest(&mut state, 1, &mut native),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(native, c_pattern, "C publish -> Rust read bytes");

    // Direction 2: publish through the Rust-native API, read through the C ABI.
    let r_pattern = [0xB2_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(channel.publish(&r_pattern), Ok(1));
    let mut c_read = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    assert_eq!(
        unsafe {
            drm_read_latest(
                consumer,
                c_read.as_mut_ptr(),
                c_read.len(),
                &raw mut out_len,
            )
        },
        Status::Ok.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX);
    assert_eq!(c_read, r_pattern, "Rust publish -> C read bytes");

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-002, REQ-BIND-007 — a lapped sequential read reports `Gap`
/// with a positive missed count (exact accounting) and copies nothing; the
/// cursor then resumes at the oldest retained sample.
#[test]
fn read_next_reports_gap() {
    let (ctx, publisher, consumer) = open_all();
    let payload = [0x33_u8; ENDPOINT_SAMPLE_MAX];
    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    let mut missed = 0_u64;

    // Fill and lap the history window several times over.
    for _ in 0..40 {
        assert_eq!(
            unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
            Status::Ok.code()
        );
    }
    // First read anchors the cursor at the oldest retained sample.
    assert_eq!(
        unsafe {
            drm_read_next(
                consumer,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        },
        Status::Ok.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX);

    // Advance the publisher past the window so the cursor is lapped.
    for _ in 0..40 {
        assert_eq!(
            unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
            Status::Ok.code()
        );
    }
    let mut missed2 = 0_u64;
    assert_eq!(
        unsafe {
            drm_read_next(
                consumer,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
                &raw mut missed2,
            )
        },
        Status::Gap.code()
    );
    assert!(
        missed2 > 0,
        "lapped cursor must report a positive miss count"
    );
    assert_eq!(out_len, 0, "the Gap path performs no copy");

    // The cursor resumes at the oldest retained sample.
    assert_eq!(
        unsafe {
            drm_read_next(
                consumer,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        },
        Status::Ok.code()
    );
    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-005 — null-argument negatives and out-param zeroing for the
/// copy/borrow exports (I-A2).
#[test]
fn null_args_are_invalid_arg_and_zeroed() {
    // Null out-params.
    assert_eq!(
        unsafe { drm_read_latest(ptr::null_mut(), ptr::null_mut(), 0, ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe {
            drm_read_next(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        },
        Status::InvalidArg.code()
    );
    let mut view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(ptr::null_mut(), &raw mut view) },
        Status::InvalidArg.code()
    );
    assert!(
        view.data.is_null() && view.token == 0,
        "borrow view zeroed before the failure"
    );
    assert_eq!(
        unsafe { drm_publish_copy(ptr::null_mut(), ptr::null(), 0) },
        Status::InvalidArg.code()
    );

    let (ctx, publisher, consumer) = open_all();
    let payload = [0x44_u8; ENDPOINT_SAMPLE_MAX];

    // Out lengths are zeroed before a null-handle failure.
    let mut out_len = 7_usize;
    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe {
            drm_read_latest(
                ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
            )
        },
        Status::InvalidArg.code()
    );
    assert_eq!(out_len, 0, "out_len zeroed before the failure");

    // `data == NULL && len != 0` is InvalidArg before any slice forms.
    assert_eq!(
        unsafe { drm_publish_copy(publisher, ptr::null(), ENDPOINT_SAMPLE_MAX) },
        Status::InvalidArg.code()
    );
    // Oversize length is InvalidArg.
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), ENDPOINT_SAMPLE_MAX + 1) },
        Status::InvalidArg.code()
    );
    // `data == NULL && len == 0` is a valid empty publish (zero-padded).
    assert_eq!(
        unsafe { drm_publish_copy(publisher, ptr::null(), 0) },
        Status::Ok.code()
    );

    // A zero-cap read is a size query: InvalidArg with the required length.
    let mut required = 0_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, ptr::null_mut(), 0, &raw mut required) },
        Status::InvalidArg.code()
    );
    assert_eq!(required, ENDPOINT_SAMPLE_MAX);

    close_all(ctx, publisher, consumer);
}
