//! C ABI stub-surface behaviour (work-plan M06W1-E4-T2): null-argument
//! negatives and status propagation for exports whose core hooks are pending.
//! REQ: REQ-BIND-004, REQ-BIND-005, REQ-BIND-007, REQ-BIND-021.

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

/// REQ: REQ-BIND-021 — the pending context hook reports `NotImplemented` and
/// zeroes `*out`, never fabricating a handle.
#[test]
fn context_open_reports_not_implemented_and_zeroes_out() {
    let mut out: *mut drm_context = dangling();
    let status = unsafe { drm_context_open(ptr::null(), &raw mut out) };
    assert_eq!(status, Status::NotImplemented.code());
    assert_eq!(status, -22);
    assert!(
        out.is_null(),
        "out must be zeroed on the NotImplemented path"
    );
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

/// REQ: REQ-BIND-007, REQ-BIND-020 — a non-null handle reaches the pending hook
/// path and propagates `NotImplemented` verbatim; the stub only null-checks, so
/// the (dangling) pointer is never dereferenced.
#[test]
fn non_null_handle_propagates_not_implemented() {
    assert_eq!(
        unsafe { drm_context_close(dangling::<drm_context>()) },
        Status::NotImplemented.code()
    );
    assert_eq!(
        unsafe { drm_publisher_close(dangling::<drm_publisher>()) },
        Status::NotImplemented.code()
    );
    assert_eq!(
        unsafe { drm_consumer_close(dangling::<drm_consumer>()) },
        Status::NotImplemented.code()
    );
}

/// REQ: REQ-BIND-018 — the ABI probe mirrors `drm_types::ABI_VERSION`.
#[test]
fn abi_version_matches_crate_constant() {
    assert_eq!(drm_abi_version(), drm_types::ABI_VERSION);
}
