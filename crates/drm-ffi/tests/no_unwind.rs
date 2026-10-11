//! No-unwind policy (`cargo test -p drm-ffi no_unwind`): a **fail-stop policy
//! test, not a shield**.
//!
//! The normative rule is I-A6 `FailStopNoUnwind` (c-abi-surface.md): `extern "C"`
//! exports never unwind; expected conditions return a `drm_status`; a Rust panic
//! is an internal invariant violation that terminates the node process, and the
//! release profile sets `panic = "abort"`. There is **no** `catch_unwind` and no
//! `Internal` status — anything safe to return has a precise code.
//!
//! This file asserts two things and nothing more:
//!   1. the workspace release profile really is `panic = "abort"` (the fail-stop
//!      mechanism), and
//!   2. every export returns a status — never panics — on a bad-but-non-null
//!      argument, i.e. the exports are structurally panic-free (every fallible
//!      path returns `Status`).
//!
//! It is explicitly **not** a `catch_unwind` shield. A panic would fail this test
//! (in the test profile) or abort the process (release); either is the intended
//! fail-stop behaviour, not something to swallow.
//!
//! # Conflict with the week plan (the doc wins)
//!
//! `docs/work-plan/work-plan-m07-w1.md` words M07W1-E4-T2 as a `catch_unwind`
//! shield "on every exported fn" returning an "internal error code", and cites
//! I-A5. That conflicts with the normative doc: I-A6 forbids `catch_unwind`, the
//! release profile is `panic = "abort"`, and there is no `Internal` status (the
//! citation is also wrong — I-A5 is `BorrowDiscipline`). Per the doc, this test
//! verifies the fail-stop policy instead of wrapping exports in a shield.
//!
//! REQ: REQ-BIND-020 (fail-stop FFI), REQ-BIND-005 (null/length checks precede
//! any reference/slice).

use core::ffi::c_char;
use core::ptr;
use core::ptr::NonNull;

use drm_ffi::{
    ENDPOINT_SAMPLE_MAX, ENDPOINT_TOPIC_ID, drm_abi_version, drm_borrow_latest, drm_borrow_release,
    drm_borrow_view, drm_consumer, drm_consumer_close, drm_consumer_open, drm_context,
    drm_context_close, drm_context_open, drm_publish_copy, drm_publisher, drm_publisher_close,
    drm_publisher_open, drm_read_latest, drm_read_next,
};
use drm_types::Status;

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

/// Open one context plus a publisher and a consumer on the declared topic.
fn open_all() -> (*mut drm_context, *mut drm_publisher, *mut drm_consumer) {
    let mut ctx: *mut drm_context = dangling();
    assert_eq!(
        unsafe { drm_context_open(ptr::null(), &raw mut ctx) },
        Status::Ok.code()
    );
    let mut publisher: *mut drm_publisher = dangling();
    assert_eq!(
        unsafe { drm_publisher_open(ctx, ENDPOINT_TOPIC_ID, &raw mut publisher) },
        Status::Ok.code()
    );
    let mut consumer: *mut drm_consumer = dangling();
    assert_eq!(
        unsafe { drm_consumer_open(ctx, ENDPOINT_TOPIC_ID, &raw mut consumer) },
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

/// REQ: REQ-BIND-020 — the fail-stop mechanism is the release profile: `extern
/// "C"` exports cannot unwind when the crate is built `panic = "abort"`. Parse
/// the workspace manifest and assert `[profile.release] panic = "abort"`.
#[test]
fn no_unwind_release_profile_is_panic_abort() {
    const MANIFEST: &str = include_str!("../../../Cargo.toml");
    let mut in_release = false;
    let mut release_panic_abort = false;
    for raw in MANIFEST.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_release = line == "[profile.release]";
            continue;
        }
        if !in_release {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "panic" && value.trim() == "\"abort\"" {
            release_panic_abort = true;
        }
    }
    assert!(
        release_panic_abort,
        "workspace [profile.release] must set panic = \"abort\" (I-A6 fail-stop)"
    );
}

/// REQ: REQ-BIND-005, REQ-BIND-020 — every export returns a `drm_status` (never
/// panics) on a bad-but-non-null argument. Bad *lengths* and bad *out-params*
/// are rejected before any Rust reference/slice forms, so a non-null pointer
/// with a bogus bound is validated, not dereferenced (I-A2) — the structural
/// reason the exports cannot panic. This is the fail-stop policy in action, not
/// a `catch_unwind` shield.
#[test]
fn no_unwind_exports_return_status_on_bad_non_null_args() {
    let (ctx, publisher, consumer) = open_all();
    let data = [0xAB_u8; ENDPOINT_SAMPLE_MAX];

    // A non-null data pointer with a bad (oversize) length is rejected before
    // any slice forms — the pointer is validated, never dereferenced.
    assert_eq!(
        unsafe { drm_publish_copy(publisher, data.as_ptr(), ENDPOINT_SAMPLE_MAX + 1) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_publish_copy(publisher, data.as_ptr(), usize::MAX) },
        Status::InvalidArg.code()
    );
    // A valid empty publish still succeeds (no panic on the happy path either).
    assert_eq!(
        unsafe { drm_publish_copy(publisher, data.as_ptr(), 0) },
        Status::Ok.code()
    );

    // A non-null destination buffer with a too-small cap is rejected before the
    // destination slice forms; nothing is copied.
    let mut buf = [0xEE_u8; 4];
    let mut out_len = 123_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, buf.as_mut_ptr(), buf.len(), &raw mut out_len) },
        Status::InvalidArg.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX, "required size reported");
    let mut missed = 5_u64;
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
        Status::InvalidArg.code()
    );
    assert_eq!(buf, [0xEE_u8; 4], "no partial copy on a too-small buffer");

    // A non-null out-param with a null handle: the out-param is zeroed and
    // `InvalidArg` is returned (no deref of the null handle).
    let mut view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(ptr::null_mut(), &raw mut view) },
        Status::InvalidArg.code()
    );
    assert!(
        view.data.is_null() && view.token == 0,
        "out view zeroed before the null-handle failure"
    );
    // A non-null (non-zero) token on a null handle is `InvalidArg`, no panic.
    assert_eq!(
        unsafe { drm_borrow_release(ptr::null_mut(), 1) },
        Status::InvalidArg.code()
    );

    // A non-null but bogus topic id at open is `Permission` (validated, no panic).
    let mut bad_pub: *mut drm_publisher = dangling();
    assert_eq!(
        unsafe { drm_publisher_open(ctx, ENDPOINT_TOPIC_ID.wrapping_add(1), &raw mut bad_pub) },
        Status::Permission.code()
    );
    assert!(bad_pub.is_null());

    // A non-null out-param with a null out-length is `InvalidArg` (no panic).
    assert_eq!(
        unsafe {
            drm_read_latest(
                consumer,
                buf.as_mut_ptr(),
                ENDPOINT_SAMPLE_MAX,
                ptr::null_mut(),
            )
        },
        Status::InvalidArg.code()
    );

    // The ABI probe has no panic surface.
    assert_eq!(drm_abi_version(), drm_types::ABI_VERSION);

    close_all(ctx, publisher, consumer);

    // A non-null but bogus deployment path is accepted (ignored this phase):
    // never dereferenced, so it cannot panic.
    let bogus_path: *const c_char = NonNull::<c_char>::dangling().as_ptr();
    let mut ctx2: *mut drm_context = dangling();
    assert_eq!(
        unsafe { drm_context_open(bogus_path, &raw mut ctx2) },
        Status::Ok.code()
    );
    assert!(!ctx2.is_null());
    assert_eq!(unsafe { drm_context_close(ctx2) }, Status::Ok.code());
}
