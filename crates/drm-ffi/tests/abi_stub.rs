//! C ABI endpoint-surface behaviour: null-argument negatives, status
//! propagation, and the live in-process endpoint lifecycle. The backing is
//! in-process (see the crate module docs); cross-process SHM endpoints are a
//! later phase. REQ: REQ-BIND-002, REQ-BIND-004, REQ-BIND-005, REQ-BIND-007,
//! REQ-BIND-021.

use core::ptr;
use core::ptr::NonNull;

use drm_ffi::{
    drm_abi_version, drm_consumer, drm_consumer_close, drm_consumer_open, drm_context,
    drm_context_close, drm_context_open, drm_publisher, drm_publisher_close, drm_publisher_open,
};
use drm_types::Status;

/// A non-null but never-dereferenced handle value: the stub exports only
/// null-check their handle argument, so this exercises the non-null path.
fn dangling<T>() -> *mut T {
    NonNull::<T>::dangling().as_ptr()
}

/// REQ: REQ-BIND-005 — a null out-param is `InvalidArg` for every open.
#[test]
fn null_out_param_is_invalid_arg() {
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
}

/// REQ: REQ-BIND-021 — a null path opens the in-process backing, writes a
/// non-null opaque handle, and closes cleanly (never a fabricated handle).
#[test]
fn context_open_activates_and_close_reclaims() {
    let mut out: *mut drm_context = dangling();
    assert_eq!(
        unsafe { drm_context_open(ptr::null(), &raw mut out) },
        Status::Ok.code()
    );
    assert!(!out.is_null(), "context_open must write a live handle");
    assert_eq!(unsafe { drm_context_close(out) }, Status::Ok.code());
}

/// REQ: REQ-BIND-004, REQ-BIND-005 — a null context at endpoint open is
/// `InvalidArg`, checked after the out-param is zeroed (no deref).
#[test]
fn null_context_open_is_invalid_arg_and_zeroes_out() {
    let mut pub_out: *mut drm_publisher = dangling();
    assert_eq!(
        unsafe { drm_publisher_open(ptr::null_mut(), 7, &raw mut pub_out) },
        Status::InvalidArg.code()
    );
    assert!(pub_out.is_null());

    let mut con_out: *mut drm_consumer = dangling();
    assert_eq!(
        unsafe { drm_consumer_open(ptr::null_mut(), 7, &raw mut con_out) },
        Status::InvalidArg.code()
    );
    assert!(con_out.is_null());
}

/// REQ: REQ-BIND-005 — a null handle is `InvalidArg` for every close.
#[test]
fn null_handle_close_is_invalid_arg() {
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
}

/// REQ: REQ-BIND-002, REQ-BIND-021 — a full open/close lifecycle through the
/// real in-process backing returns `OK` at every step (no fabricated handle,
/// no `NotImplemented`).
#[test]
fn endpoint_lifecycle_open_close_succeeds() {
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
    assert!(!publisher.is_null() && !consumer.is_null());
    assert_eq!(unsafe { drm_publisher_close(publisher) }, Status::Ok.code());
    assert_eq!(unsafe { drm_consumer_close(consumer) }, Status::Ok.code());
    assert_eq!(unsafe { drm_context_close(ctx) }, Status::Ok.code());
}

/// REQ: REQ-BIND-018 — the ABI probe mirrors `drm_types::ABI_VERSION`.
#[test]
fn abi_version_matches_crate_constant() {
    assert_eq!(drm_abi_version(), drm_types::ABI_VERSION);
}
