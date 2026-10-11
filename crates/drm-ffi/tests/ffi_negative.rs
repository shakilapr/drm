//! FFI negative sweep (`cargo test -p drm-ffi ffi_negative`): every rejected
//! input must return an explicit `drm_status` and never crash. Covers null
//! args, oversize publish length, too-small read buffer, double release,
//! foreign/stale borrow tokens, the defined close paths, and borrower-lease
//! exhaustion (c-abi-surface.md §Edge Cases & Errors; I-A2 `PointerChecked`,
//! I-A5 `BorrowDiscipline`).
//!
//! Handle contract (drm.h §Handle validity): only NULL is checked. A non-null
//! freed/closed/foreign/type-confused handle is a caller contract violation —
//! undefined by the C API and never claimed detectable. This suite therefore
//! does NOT dereference a closed handle (doing so would itself be UB); the
//! undetectable cases are pinned to the header contract in
//! `ffi_negative_use_after_close_is_documented_ub` instead of faked.
//!
//! REQ: REQ-BIND-001, REQ-BIND-002, REQ-BIND-004, REQ-BIND-005, REQ-BIND-006,
//! REQ-BIND-010, REQ-BIND-017.

use core::ptr;
use core::ptr::NonNull;

use drm_ffi::{
    ENDPOINT_BORROWERS, ENDPOINT_SAMPLE_MAX, drm_borrow_latest, drm_borrow_release,
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

/// REQ: REQ-BIND-005, REQ-BIND-017 — null where a non-null pointer is required
/// is `InvalidArg` for every export; out-params are zeroed before the failure
/// (I-A2) so an ignored status never observes a stale pointer/token.
#[test]
fn ffi_negative_null_args() {
    // Null out-params on the opens.
    assert_eq!(
        unsafe { drm_context_open(ptr::null(), ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_publisher_open(ptr::null_mut(), 0, ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_consumer_open(ptr::null_mut(), 0, ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    // Null handle on every close.
    assert_eq!(
        unsafe { drm_context_close(ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_publisher_close(ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_consumer_close(ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    // Null publisher and null out-params on the data path.
    assert_eq!(
        unsafe { drm_publish_copy(ptr::null_mut(), ptr::null(), 0) },
        Status::InvalidArg.code()
    );
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
    assert_eq!(
        unsafe { drm_borrow_latest(ptr::null_mut(), ptr::null_mut()) },
        Status::InvalidArg.code()
    );
}

/// REQ: REQ-BIND-005, REQ-BIND-017 — with live handles, a null handle or null
/// out-param is `InvalidArg` and every out-param is zeroed before the failure
/// (I-A2), so an ignored status never observes a stale pointer/token.
#[test]
fn ffi_negative_null_args_live_handles() {
    let (ctx, publisher, consumer) = open_all();
    let payload = [0x55_u8; ENDPOINT_SAMPLE_MAX];
    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];

    // Null consumer: out-param zeroed before the handle is examined.
    let mut out_len = 7_usize;
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
    assert_eq!(out_len, 0, "out_len zeroed before the null-handle failure");

    let mut out_len = 7_usize;
    let mut missed = 9_u64;
    assert_eq!(
        unsafe {
            drm_read_next(
                ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        },
        Status::InvalidArg.code()
    );
    assert_eq!(out_len, 0, "out_len zeroed before the failure");
    assert_eq!(missed, 0, "out_missed zeroed before the failure");

    // Null borrow view with a live consumer: the null out-param wins.
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    // Null borrow handle and null view.
    assert_eq!(
        unsafe { drm_borrow_release(ptr::null_mut(), 1) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(consumer, 0) },
        Status::InvalidArg.code()
    );
    // `data == NULL && len != 0` is InvalidArg before any slice forms.
    assert_eq!(
        unsafe { drm_publish_copy(publisher, ptr::null(), ENDPOINT_SAMPLE_MAX) },
        Status::InvalidArg.code()
    );
    // A borrow view zeroed before a failure never leaks a stale token.
    let mut view = drm_borrow_view {
        data: payload.as_ptr().cast(),
        len: payload.len(),
        sequence: 1,
        token: 0xDEAD_BEEF,
    };
    assert_eq!(
        unsafe { drm_borrow_latest(ptr::null_mut(), &raw mut view) },
        Status::InvalidArg.code()
    );
    assert!(
        view.data.is_null() && view.len == 0 && view.sequence == 0 && view.token == 0,
        "view zeroed before the failure"
    );

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-005 — a publish length above the fixed structural maximum is
/// `InvalidArg` *before* any Rust slice forms (I-A2); the bound is checked
/// first, so a non-null pointer with a bogus length is never dereferenced.
#[test]
fn ffi_negative_oversize_len() {
    let (ctx, publisher, consumer) = open_all();
    let payload = [0x66_u8; ENDPOINT_SAMPLE_MAX];

    // One past the structural maximum, and the extreme lengths: all rejected
    // without touching the (unreadable-by-contract) pointer.
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), ENDPOINT_SAMPLE_MAX + 1) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), isize::MAX as usize) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), usize::MAX) },
        Status::InvalidArg.code()
    );

    // Boundaries that are accepted: max-1 (zero-padded) and exactly max.
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), ENDPOINT_SAMPLE_MAX - 1) },
        Status::Ok.code()
    );
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), ENDPOINT_SAMPLE_MAX) },
        Status::Ok.code()
    );
    // The oversized calls above must not have published anything observable.
    let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, buf.as_mut_ptr(), buf.len(), &raw mut out_len) },
        Status::Ok.code()
    );
    assert_eq!(buf, payload, "the last accepted publish is delivered");

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-005, REQ-BIND-019 — a read buffer smaller than the required
/// sample is `InvalidArg` with `*out_len` set to the required size and *no*
/// partial copy (I-C9); a zero-cap call is a pure size query. `drm_read_next`
/// additionally leaves `*out_missed` zeroed on this path.
#[test]
fn ffi_negative_too_small_read_buffer() {
    let (ctx, publisher, consumer) = open_all();
    let payload = [0x77_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
        Status::Ok.code()
    );

    // Too-small non-null buffer: required size reported, destination untouched.
    let mut small = [0xEE_u8; 8];
    let mut out_len = 999_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, small.as_mut_ptr(), small.len(), &raw mut out_len) },
        Status::InvalidArg.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX, "required size reported");
    assert_eq!(small, [0xEE_u8; 8], "no partial copy on a too-small buffer");

    // Zero-cap size query (null buffer is legal when cap == 0).
    let mut required = 0_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, ptr::null_mut(), 0, &raw mut required) },
        Status::InvalidArg.code()
    );
    assert_eq!(required, ENDPOINT_SAMPLE_MAX);

    // drm_read_next: same rule, and out_missed stays zeroed.
    let mut small_next = [0xEE_u8; 16];
    let mut out_len = 3_usize;
    let mut missed = 5_u64;
    assert_eq!(
        unsafe {
            drm_read_next(
                consumer,
                small_next.as_mut_ptr(),
                small_next.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        },
        Status::InvalidArg.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX, "required size reported");
    assert_eq!(missed, 0, "out_missed zeroed on the too-small path");
    assert_eq!(
        small_next, [0xEE_u8; 16],
        "no partial copy on a too-small buffer"
    );

    // A correctly sized read then delivers the exact bytes.
    let mut ok = [0_u8; ENDPOINT_SAMPLE_MAX];
    let mut out_len = 0_usize;
    assert_eq!(
        unsafe { drm_read_latest(consumer, ok.as_mut_ptr(), ok.len(), &raw mut out_len) },
        Status::Ok.code()
    );
    assert_eq!(out_len, ENDPOINT_SAMPLE_MAX);
    assert_eq!(ok, payload);

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-001, REQ-BIND-006, REQ-BIND-017 — an explicit release followed
/// by a second release of the same token is `InvalidArg` (no outstanding borrow);
/// the endpoint stays usable afterwards.
#[test]
fn ffi_negative_double_release() {
    let (ctx, publisher, consumer) = open_all();
    let payload = [0x88_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
        Status::Ok.code()
    );

    let mut view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, &raw mut view) },
        Status::Ok.code()
    );
    assert_ne!(view.token, 0);
    let token = view.token;
    assert_eq!(
        unsafe { drm_borrow_release(consumer, token) },
        Status::Ok.code()
    );
    // Double release of the same (now stale) token.
    assert_eq!(
        unsafe { drm_borrow_release(consumer, token) },
        Status::InvalidArg.code()
    );
    // Token 0 is always InvalidArg, even with an outstanding borrow elsewhere.
    let mut again = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer, &raw mut again) },
        Status::Ok.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(consumer, 0) },
        Status::InvalidArg.code()
    );
    // The endpoint is still usable: a real release succeeds.
    assert_eq!(
        unsafe { drm_borrow_release(consumer, again.token) },
        Status::Ok.code()
    );

    close_all(ctx, publisher, consumer);
}

/// REQ: REQ-BIND-001, REQ-BIND-004, REQ-BIND-006 — wrong/foreign borrow tokens
/// are `InvalidArg`: a token issued to another consumer, a forged token, and a
/// stale token from an already-released borrow. This is the one *detectable*
/// cross-handle misuse: the token is validated against the endpoint's single
/// outstanding borrow. (A foreign/closed non-null *handle* is caller UB, see
/// `ffi_negative_use_after_close_is_documented_ub`.)
#[test]
fn ffi_negative_foreign_and_stale_tokens() {
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
    let mut consumer_a: *mut drm_consumer = dangling();
    let mut consumer_b: *mut drm_consumer = dangling();
    assert_eq!(
        unsafe { drm_consumer_open(ctx, 0, &raw mut consumer_a) },
        Status::Ok.code()
    );
    assert_eq!(
        unsafe { drm_consumer_open(ctx, 0, &raw mut consumer_b) },
        Status::Ok.code()
    );

    let payload = [0x99_u8; ENDPOINT_SAMPLE_MAX];
    assert_eq!(
        unsafe { drm_publish_copy(publisher, payload.as_ptr(), payload.len()) },
        Status::Ok.code()
    );

    // A burns its first token; its second token then differs from B's first,
    // making the foreign case a guaranteed mismatch.
    let mut first_view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer_a, &raw mut first_view) },
        Status::Ok.code()
    );
    let first_token = first_view.token;
    assert_eq!(
        unsafe { drm_borrow_release(consumer_a, first_token) },
        Status::Ok.code()
    );
    let mut held_view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer_a, &raw mut held_view) },
        Status::Ok.code()
    );
    let held_token = held_view.token;
    assert_ne!(held_token, first_token, "token increments per borrow");

    let mut peer_view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer_b, &raw mut peer_view) },
        Status::Ok.code()
    );
    let peer_token = peer_view.token;
    assert_ne!(
        held_token, peer_token,
        "foreign-token case needs distinct token namespaces"
    );

    // Foreign: A's live token presented to B (which holds its own token).
    assert_eq!(
        unsafe { drm_borrow_release(consumer_b, held_token) },
        Status::InvalidArg.code()
    );
    // Forged: a token A never issued.
    assert_eq!(
        unsafe { drm_borrow_release(consumer_a, held_token.wrapping_add(1)) },
        Status::InvalidArg.code()
    );
    // Genuine releases still succeed.
    assert_eq!(
        unsafe { drm_borrow_release(consumer_a, held_token) },
        Status::Ok.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(consumer_b, peer_token) },
        Status::Ok.code()
    );

    // Stale: A re-borrows; its previous (already-released) token is rejected.
    let mut final_view = empty_view();
    assert_eq!(
        unsafe { drm_borrow_latest(consumer_a, &raw mut final_view) },
        Status::Ok.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(consumer_a, held_token) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_borrow_release(consumer_a, final_view.token) },
        Status::Ok.code()
    );

    assert_eq!(unsafe { drm_publisher_close(publisher) }, Status::Ok.code());
    assert_eq!(unsafe { drm_consumer_close(consumer_a) }, Status::Ok.code());
    assert_eq!(unsafe { drm_consumer_close(consumer_b) }, Status::Ok.code());
    assert_eq!(unsafe { drm_context_close(ctx) }, Status::Ok.code());
}

/// REQ: REQ-BIND-004, REQ-BIND-002 — documented UB: reusing a handle after close
/// (or any non-null freed/foreign/type-confused handle) is a caller contract
/// violation. `drm.h` checks only NULL and never claims these detectable, so a
/// test that dereferenced a closed handle would itself invoke UB. We pin the
/// header contract (drift guard) and exercise the *defined* close paths instead
/// of faking a status for the undefined ones.
#[test]
fn ffi_negative_use_after_close_is_documented_ub() {
    const HEADER: &str = include_str!("../../../bindings/c/include/drm/drm.h");
    assert!(
        HEADER.contains("freed/foreign/NULL"),
        "drm.h must keep the closed/foreign handle contract"
    );
    assert!(
        HEADER.contains("never claimed detectable"),
        "drm.h must keep the undetectable-dangling contract"
    );
    assert!(
        HEADER.contains("A handle is valid from a successful open until its matching close"),
        "drm.h must keep the handle-lifetime statement"
    );

    // Defined close semantics: explicit close of live handles returns OK, and
    // NULL is InvalidArg for every close (never a crash).
    let (ctx, publisher, consumer) = open_all();
    assert_eq!(unsafe { drm_publisher_close(publisher) }, Status::Ok.code());
    assert_eq!(unsafe { drm_consumer_close(consumer) }, Status::Ok.code());
    assert_eq!(unsafe { drm_context_close(ctx) }, Status::Ok.code());
    assert_eq!(
        unsafe { drm_publisher_close(ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_consumer_close(ptr::null_mut()) },
        Status::InvalidArg.code()
    );
    assert_eq!(
        unsafe { drm_context_close(ptr::null_mut()) },
        Status::InvalidArg.code()
    );
}

/// REQ: REQ-BIND-010 — the borrower-record lease is a defined negative: after
/// `ENDPOINT_BORROWERS` consumers are opened, the next `drm_consumer_open`
/// returns `Busy` (never a crash) and writes no handle.
#[test]
fn ffi_negative_consumer_lease_exhausted() {
    let mut ctx: *mut drm_context = dangling();
    assert_eq!(
        unsafe { drm_context_open(ptr::null(), &raw mut ctx) },
        Status::Ok.code()
    );

    let mut consumers: [*mut drm_consumer; ENDPOINT_BORROWERS] =
        [ptr::null_mut(); ENDPOINT_BORROWERS];
    for slot in &mut consumers {
        let mut out: *mut drm_consumer = dangling();
        assert_eq!(
            unsafe { drm_consumer_open(ctx, 0, &raw mut out) },
            Status::Ok.code()
        );
        assert!(!out.is_null());
        *slot = out;
    }

    // The (N+1)th open finds no vacant record ⇒ Busy, and writes no handle.
    let mut overflow: *mut drm_consumer = dangling();
    assert_eq!(
        unsafe { drm_consumer_open(ctx, 0, &raw mut overflow) },
        Status::Busy.code()
    );
    assert!(overflow.is_null(), "failed open must not write a handle");

    for slot in consumers {
        assert_eq!(unsafe { drm_consumer_close(slot) }, Status::Ok.code());
    }
    assert_eq!(unsafe { drm_context_close(ctx) }, Status::Ok.code());
}
