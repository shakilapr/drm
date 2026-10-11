//! Canonical C ABI crate (codebase.md 2.1 layer `runtime`): `extern "C"` exports
//! over drm-core; `bindings/c/include/drm/drm.h` is the single source of truth.
//! Fail-stop: a Rust panic here terminates the node process (no `Internal`
//! status). Exports use the plain `extern "C"` ABI, which aborts rather than
//! unwinding across the FFI boundary (I-A6; no `catch_unwind`).
//! REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes), REQ-BIND-002
//! (explicit close; per-handle thread-safety), REQ-BIND-020 (fail-stop FFI).
//!
//! Stub surface (work-plan M06W1-E4): opaque handles, the fixed status mirror,
//! and the endpoint-lifecycle exports. The core hooks
//! (`drm_core::{context_open, publisher_open, consumer_open}` and their close
//! counterparts) are pending, so each export validates its arguments (null
//! checks and out-param zeroing, I-A2) and then returns `Status::NotImplemented`.
//! No handle or sample is ever fabricated. Unsafe is permitted here
//! (codebase.md 3.2).

// ABI identifiers mirror `drm.h` verbatim (`drm_status`, `drm_context`, ...) and
// are intentionally not camel-cased. REQ: REQ-BIND-004, REQ-BIND-007.
#![allow(non_camel_case_types)]

use core::ffi::c_char;

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
    unsafe { core::ptr::write_bytes(out, 0, 1) };
    Ok(())
}

/// ABI version probe; mirrors `DRM_ABI_VERSION` in `drm.h` (bump on layout
/// change only). REQ: REQ-BIND-018.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn drm_abi_version() -> u32 {
    drm_types::ABI_VERSION
}

/// Open the deployment context (opaque handle). REQ: REQ-BIND-021, REQ-BIND-005.
///
/// Stub: `out` is zeroed first (null `out` ⇒ `InvalidArg`); the pending
/// `drm_core::context_open` hook then yields `Status::NotImplemented`. `path`
/// semantics (NULL ⇒ `DRM_DEPLOYMENT`, else a NUL-terminated deployment path)
/// apply once the hook lands.
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
    Status::NotImplemented.code()
}

/// Close a deployment context. REQ: REQ-BIND-002, REQ-BIND-021.
///
/// Stub: null `ctx` ⇒ `InvalidArg`; otherwise the pending
/// `drm_core::context_close` hook ⇒ `Status::NotImplemented`. Only the null case
/// is validated — a non-null dangling handle is caller UB (I-A2).
///
/// # Safety
/// `ctx` is null or a handle returned by `drm_context_open`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_context_close(ctx: *mut drm_context) -> drm_status {
    if ctx.is_null() {
        return Status::InvalidArg.code();
    }
    Status::NotImplemented.code()
}

/// Open a publisher endpoint on `topic_id`. REQ: REQ-BIND-002, REQ-BIND-008.
///
/// Stub: `out` is zeroed first (null `out` ⇒ `InvalidArg`); a null `ctx` then
/// yields `InvalidArg`; otherwise the pending `drm_core::publisher_open` hook
/// yields `Status::NotImplemented`. Validation-once (I-A1) runs inside the hook.
///
/// # Safety
/// `ctx` is null or a live context handle; `out` is null or points to a
/// writable `*mut drm_publisher`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_publisher_open(
    ctx: *mut drm_context,
    _topic_id: u32,
    out: *mut *mut drm_publisher,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    if ctx.is_null() {
        return Status::InvalidArg.code();
    }
    Status::NotImplemented.code()
}

/// Close a publisher endpoint. REQ: REQ-BIND-002.
///
/// Stub: null `publisher` ⇒ `InvalidArg`; otherwise the pending
/// `drm_core::publisher_close` hook ⇒ `Status::NotImplemented`.
///
/// # Safety
/// `publisher` is null or a handle returned by `drm_publisher_open`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_publisher_close(publisher: *mut drm_publisher) -> drm_status {
    if publisher.is_null() {
        return Status::InvalidArg.code();
    }
    Status::NotImplemented.code()
}

/// Open a consumer endpoint on `topic_id`. REQ: REQ-BIND-002, REQ-BIND-010.
///
/// Stub: `out` is zeroed first (null `out` ⇒ `InvalidArg`); a null `ctx` then
/// yields `InvalidArg`; otherwise the pending `drm_core::consumer_open` hook
/// (which claims a declared borrower record) yields `Status::NotImplemented`.
///
/// # Safety
/// `ctx` is null or a live context handle; `out` is null or points to a
/// writable `*mut drm_consumer`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_consumer_open(
    ctx: *mut drm_context,
    _topic_id: u32,
    out: *mut *mut drm_consumer,
) -> drm_status {
    if let Err(status) = unsafe { zero_out(out) } {
        return status.code();
    }
    if ctx.is_null() {
        return Status::InvalidArg.code();
    }
    Status::NotImplemented.code()
}

/// Close a consumer endpoint. REQ: REQ-BIND-002.
///
/// Stub: null `consumer` ⇒ `InvalidArg`; otherwise the pending
/// `drm_core::consumer_close` hook ⇒ `Status::NotImplemented`.
///
/// # Safety
/// `consumer` is null or a handle returned by `drm_consumer_open`.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn drm_consumer_close(consumer: *mut drm_consumer) -> drm_status {
    if consumer.is_null() {
        return Status::InvalidArg.code();
    }
    Status::NotImplemented.code()
}

#[cfg(test)]
mod tests {
    use drm_types::Status;

    /// REQ: REQ-BIND-007 — compile smoke for the stub surface.
    #[test]
    fn skeleton_compiles() {
        assert_eq!(Status::Busy.code(), -11);
        assert_eq!(Status::NotImplemented.code(), -22);
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
    /// and every stub-lifecycle export; a dropped declaration is a contract
    /// drift the C compile fixture alone cannot detect.
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
