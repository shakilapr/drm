//! FFI exchange matrix (`cargo test -p drm-ffi ffi_matrix`): every
//! publisher/consumer combination across the two access paths over the single
//! in-process backing channel — Rust-native (`drm_core::channel::Channel`) and
//! C-ABI-driven (`drm-ffi` exports) — must exchange identical bytes
//! (c-abi-surface.md §Test Matrix `ffi_parity`; I-A1 validate-once, I-A5
//! borrow discipline).
//!
//! | publisher → consumer | Rust-native consumer        | C-ABI consumer            |
//! |----------------------|-----------------------------|---------------------------|
//! | Rust-native          | `channel.publish`/`.latest` | `drm_read_latest`         |
//! | C-ABI                | `channel.latest`            | `drm_read_latest`/`_next` |
//!
//! # Documented gap: variable-size topics
//!
//! The in-process backing topic is **fixed-size** (`ENDPOINT_SAMPLE_MAX`): a
//! shorter source publish is zero-padded to the fixed sample and every read
//! delivers the full fixed sample, so a reader cannot recover the true
//! published length. True variable-size delivery (`*out_len` == published
//! length) is a later phase; `ffi_matrix_variable_size_topic_is_documented_gap`
//! pins the *current* fixed-size behaviour rather than faking variable support.
//!
//! REQ: REQ-BIND-002, REQ-BIND-003, REQ-BIND-004, REQ-BIND-005, REQ-BIND-006,
//! REQ-BIND-008.

use core::ptr;
use core::ptr::NonNull;

use drm_core::channel::ReadOutcome;
use drm_ffi::{
    ENDPOINT_SAMPLE_MAX, drm_borrow_latest, drm_borrow_release, drm_borrow_view, drm_consumer,
    drm_consumer_close, drm_consumer_open, drm_context, drm_context_channel, drm_context_close,
    drm_context_open, drm_publish_copy, drm_publisher, drm_publisher_close, drm_publisher_open,
    drm_read_latest, drm_read_next,
};
use drm_types::Status;

/// Native-consumer borrower-record index (0 is claimed by the C consumer).
const NATIVE_REC: usize = 1;

/// Non-null placeholder handle overwritten by a successful open.
fn dangling<T>() -> *mut T {
    NonNull::<T>::dangling().as_ptr()
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

/// REQ: REQ-BIND-004, REQ-BIND-008 — Rust-native publisher → Rust-native
/// consumer: the control cell of the matrix, both ends on the shared channel.
#[test]
fn ffi_matrix_rust_publish_rust_read() {
    let (ctx, publisher, consumer) = open_all();
    let channel = unsafe { &*drm_context_channel(ctx) };

    let pattern = [0x11_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(channel.publish(&pattern), Ok(0));

    let mut state = channel.reader();
    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        channel.latest(&mut state, NATIVE_REC, &mut buf),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(buf, pattern, "Rust publish -> Rust read bytes");

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-002, REQ-BIND-005 — C-ABI publisher → Rust-native consumer:
/// bytes published through `drm_publish_copy` are visible byte-exact to the
/// native `channel.latest` reader.
#[test]
fn ffi_matrix_c_publish_rust_read() {
    let (ctx, publisher, consumer) = open_all();
    let channel = unsafe { &*drm_context_channel(ctx) };

    let pattern = [0x22_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, pattern.as_ptr(), pattern.len()) },
        Status::Ok.code()
    );

    let mut state = channel.reader();
    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        channel.latest(&mut state, NATIVE_REC, &mut buf),
        Ok(ReadOutcome::Sample)
    );
    assert_eq!(buf, pattern, "C publish -> Rust read bytes");

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-002, REQ-BIND-005 — Rust-native publisher → C-ABI consumer:
/// a native `channel.publish` is delivered byte-exact to `drm_read_latest`.
#[test]
fn ffi_matrix_rust_publish_c_read() {
    let (ctx, publisher, consumer) = open_all();
    let channel = unsafe { &*drm_context_channel(ctx) };

    let pattern = [0x33_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(channel.publish(&pattern), Ok(0));

    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, buf.as_mut_ptr(), buf.len(), &raw mut out_len) },
        Status::Ok.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX);
    assert_eq!(buf, pattern, "Rust publish -> C read bytes");

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-002, REQ-BIND-005 — C-ABI publisher → C-ABI consumer: the
/// fully C-driven cell for both the `latest` and `next` copy reads.
#[test]
fn ffi_matrix_c_publish_c_read() {
    let (ctx, publisher, consumer) = open_all();
    let pattern = [0x44_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, pattern.as_ptr(), pattern.len()) },
        Status::Ok.code()
    );

    // latest
    let mut latest = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    assert_eq!(
        unsafe {
            drm_read_latest(
                consumer,
                latest.as_mut_ptr(),
                latest.len(),
                &raw mut out_len,
            )
        },
        Status::Ok.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX);
    assert_eq!(latest, pattern, "C publish -> C read (latest) bytes");

    // next (first read anchors the cursor at the retained sample).
    let mut next = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    let mut missed = 0_u64;
    assert_eq!(
        unsafe {
            drm_read_next(
                consumer,
                next.as_mut_ptr(),
                next.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        },
        Status::Ok.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX);
    assert_eq!(missed, 0);
    assert_eq!(next, pattern, "C publish -> C read (next) bytes");

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-001, REQ-BIND-006 — C-ABI publisher → C-ABI zero-copy borrow:
/// the borrowed bytes are byte-exact, the token is non-zero, the sequence
/// advances per publish, and release re-arms the endpoint.
#[test]
fn ffi_matrix_c_publish_c_borrow() {
    let (ctx, publisher, consumer) = open_all();

    let first = [0x55_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, first.as_ptr(), first.len()) },
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
    let borrowed = unsafe { core::slice::from_raw_parts(view.data.cast::<u8>(), view.len) };
    assert_eq!(borrowed, &first[..], "C publish -> C borrow bytes");
    let seq0 = view.sequence;
    assert_eq!(
        unsafe { drm_borrow_release(consumer, view.token) },
        Status::Ok.code()
    );

    let second = [0x66_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, second.as_ptr(), second.len()) },
        Status::Ok.code()
    );
    let mut view2 = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, &raw mut view2) },
        Status::Ok.code()
    );
    let borrowed2 = unsafe { core::slice::from_raw_parts(view2.data.cast::<u8>(), view2.len) };
    assert_eq!(borrowed2, &second[..], "second borrow is the newest sample");
    assert_eq!(view2.sequence, seq0 + 1, "sequence advances per publish");
    assert_eq!(
        unsafe { drm_borrow_release(consumer, view2.token) },
        Status::Ok.code()
    );

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-002, REQ-BIND-005 — cross-path ordering: distinct samples
/// published through the C ABI are delivered in publication order by the
/// C-ABI sequential reader (`drm_read_next`).
#[test]
fn ffi_matrix_read_next_orders_published_samples() {
    let (ctx, publisher, consumer) = open_all();
    let samples = [
        [0x10_u8; ENDPOINT_SAMPLE_MAX],
        [0x20_u8; ENDPOINT_SAMPLE_MAX],
        [0x30_u8; ENDPOINT_SAMPLE_MAX],
    ];
    for sample in &samples {
        assert_eq!(
            unsafe { drm_publish_copy(publisher, sample.as_ptr(), sample.len()) },
            Status::Ok.code()
        );
    }

    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    let mut missed = 0_u64;
    for expected in &samples {
        missed = 0;
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
        assert_eq!(missed, 0);
        assert_eq!(&buf, expected, "sequential delivery order");
    }
    // Caught up: the next sequential read finds nothing new.
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
        Status::NoSample.code()
    );

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-005, REQ-BIND-008 — DOCUMENTED GAP. The in-process backing is
/// a fixed-size topic: a shorter C-ABI publish is zero-padded to
/// `ENDPOINT_SAMPLE_MAX` and the read reports the *full* fixed sample length,
/// not the published length. This pins the current behaviour so the gap is
/// explicit; true variable-size delivery (`*out_len` == published length) is a
/// later phase and must not be faked.
#[test]
fn ffi_matrix_variable_size_topic_is_documented_gap() {
    let (ctx, publisher, consumer) = open_all();

    let short = [0x7A_u8; 16];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, short.as_ptr(), short.len()) },
        Status::Ok.code()
    );

    let mut buf = [0xEE_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, buf.as_mut_ptr(), buf.len(), &raw mut out_len) },
        Status::Ok.code()
    );
    // GAP (not variable-size support): the read always yields the fixed sample.
    assert_eq!(
        out_len, ENDPOINT_SAMPLE_MAX,
        "fixed-size topic: read length is the fixed sample, not the 16-byte publish"
    );
    assert_eq!(&buf[..16], &short[..], "published head is preserved");
    assert_eq!(
        &buf[16..],
        &[0_u8; ENDPOINT_SAMPLE_MAX - 16][..],
        "short publish zero-padded"
    );

    close_all(ctx, publisher, consumer);
}
