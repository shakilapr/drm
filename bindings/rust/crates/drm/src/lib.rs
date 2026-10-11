//! Safe Rust SDK over the `drm-ffi` C ABI (`bindings/rust/crates/drm`).
//!
//! Layer `bindings` (codebase.md 2.1): depends on `drm-ffi` and `drm-types`
//! only — never on toolchain, ops, or facade crates. `unsafe` is confined to
//! the thin `drm-ffi` call boundary; every public item is a safe abstraction.
//!
//! # Safety model
//!
//! [`BorrowGuard`] is a transient, hazard-protected view of a committed sample:
//!
//! * it borrows its [`Consumer`] for the guard's lifetime, so the endpoint
//!   cannot be closed while a borrow is outstanding;
//! * [`BorrowGuard::payload`] returns a slice tied to `&self`, so the bytes can
//!   never outlive the guard;
//! * it releases the hazard slot in [`Drop`] (RAII); and
//! * it is `!Send + !Sync` by construction (`PhantomData<*mut ()>`): the
//!   borrower thread owns its hazard slot (bindings-cpp-rust-python.md I-B8).
//!
//! Together these make reading payload bytes after release unrepresentable in
//! safe code. REQ: REQ-BIND-016, REQ-BIND-001, REQ-BIND-006, REQ-BIND-021,
//! REQ-BIND-002, REQ-BIND-004, REQ-BIND-017.
//!
//! # Borrow lifecycle
//!
//! A [`BorrowGuard`] pins one hazard slot for the duration of a scoped read
//! (see runtime `hazard-protocol.md`, invariant `I-HZ7` one outstanding borrow,
//! and bindings-cpp-rust-python.md I-B4). External I/O must never retain the
//! borrowed bytes; take an owned read instead:
//!
//! * [`Consumer::read_latest`] copies the newest sample into a caller buffer;
//! * [`Consumer::read_next`] copies the next in-order sample and reports a
//!   lapped cursor as [`ReadOutcome::Gap`].
//!
//! Owned reads return application memory and may be retained freely.
//!
//! # Example
//!
//! ```
//! use drm::Context;
//!
//! # fn main() -> Result<(), drm::Error> {
//! let ctx = Context::open(None)?;
//! let publisher = ctx.open_publisher(0)?;
//! let consumer = ctx.open_consumer(0)?;
//! publisher.publish(b"hello")?;
//! let guard = consumer.borrow_latest()?;
//! assert_eq!(&guard.payload()[..5], b"hello"); // tied to `guard`; released on drop
//! # Ok(())
//! # }
//! ```

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::ptr;

use drm_ffi::{
    drm_borrow_latest, drm_borrow_release, drm_borrow_view, drm_consumer, drm_consumer_close,
    drm_consumer_open, drm_context, drm_context_close, drm_context_open, drm_publish_copy,
    drm_publisher, drm_publisher_close, drm_publisher_open, drm_read_latest, drm_read_next,
};
use drm_types::Status;

/// Encodes `!Send + !Sync`: every SDK handle is owned by one thread, which is
/// the thread that holds the associated hazard slot / borrower record
/// (bindings-cpp-rust-python.md I-B8). Without this marker the auto-traits would
/// derive the opposite of the contract.
/// REQ: REQ-BIND-016.
type NotSendSync = PhantomData<*mut ()>;

/// Fallible SDK result returned by every operation that can fail.
/// REQ: REQ-BIND-007.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A DRM status other than success, mirrored 1:1 from the C code.
    /// REQ: REQ-BIND-007.
    Status(Status),
    /// A status code not known to this SDK build. Surfaced verbatim so an older
    /// binding never misreads a future code as a `repr(C)` enum (I-A9).
    /// REQ: REQ-BIND-017.
    UnknownStatus(i32),
}

impl Error {
    /// The fixed `int32_t` status code behind this error.
    /// REQ: REQ-BIND-017.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Status(status) => status.code(),
            Self::UnknownStatus(code) => code,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => write!(f, "drm: {status:?}"),
            Self::UnknownStatus(code) => write!(f, "drm: unknown status {code}"),
        }
    }
}

impl core::error::Error for Error {}

/// Outcome of an owned sequential read ([`Consumer::read_next`]).
///
/// A caught-up/empty channel is not an error: it surfaces as `Ok(None)` so the
/// caller can poll. A lapped cursor is explicit — `missed` is the exact number
/// of samples never delivered (cursor-delivery I-C2). REQ: REQ-BIND-002.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// A sample was copied into the caller buffer.
    Sample {
        /// Number of bytes written into the destination buffer.
        len: usize,
    },
    /// The cursor was lapped; `missed` samples were never delivered (I-C2).
    Gap {
        /// Exact count of missed samples.
        missed: u64,
    },
}

/// Maps a C status code to `Ok(())` or [`Error`], preserving unknown codes.
/// REQ: REQ-BIND-017.
fn check(code: i32) -> Result<(), Error> {
    match Status::from_code(code) {
        Some(Status::Ok) => Ok(()),
        Some(status) => Err(Error::Status(status)),
        None => Err(Error::UnknownStatus(code)),
    }
}

/// RAII owner of a deployment context (the opaque `drm_context` handle).
///
/// The context owns the backing channel; every [`Publisher`] and [`Consumer`]
/// borrows it for its lifetime, so endpoints are always closed before the
/// context (I-A7). REQ: REQ-BIND-021, REQ-BIND-002, REQ-BIND-004.
//
// The SDK defines no C-layout (`repr(C)`) struct: all ABI-layout types live in
// `drm-ffi` (`drm_borrow_view`). The golden-layout check (work-plan
// M07W3-E4-T2) therefore does not apply here and is asserted in `mod sdk`.
pub struct Context {
    /// Live opaque context handle (non-null after a successful open).
    handle: *mut drm_context,
    /// Encodes `!Send + !Sync` (REQ: REQ-BIND-016).
    _not_send_sync: NotSendSync,
}

impl Context {
    /// Opens the deployment context. In this phase `path` is accepted and
    /// ignored by the ABI (in-process backing).
    ///
    /// # Errors
    /// Returns [`Error`] when the ABI reports a non-`Ok` status.
    /// REQ: REQ-BIND-021.
    pub fn open(path: Option<&CStr>) -> Result<Self, Error> {
        let mut raw: *mut drm_context = ptr::null_mut();
        let c_path = path.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: `raw` is a valid out-parameter and `c_path` is null or a valid
        // NUL-terminated C string; `drm_context_open` checks both.
        let code = unsafe { drm_context_open(c_path, &raw mut raw) };
        check(code)?;
        // Success guarantees a live, non-null handle (drm-ffi contract, I-A2).
        Ok(Self {
            handle: raw,
            _not_send_sync: PhantomData,
        })
    }

    /// Opens a publisher endpoint on `topic_id`.
    ///
    /// # Errors
    /// Returns [`Error`] when the ABI reports a non-`Ok` status (for example
    /// `Permission` for an undeclared topic).
    /// REQ: REQ-BIND-002, REQ-BIND-008, REQ-BIND-004.
    pub fn open_publisher(&self, topic_id: u32) -> Result<Publisher<'_>, Error> {
        let mut raw: *mut drm_publisher = ptr::null_mut();
        // SAFETY: `self.handle` is a live context; `raw` is a valid out-parameter.
        let code = unsafe { drm_publisher_open(self.handle, topic_id, &raw mut raw) };
        check(code)?;
        Ok(Publisher {
            handle: raw,
            _ctx: PhantomData,
            _not_send_sync: PhantomData,
        })
    }

    /// Opens a consumer endpoint on `topic_id`.
    ///
    /// # Errors
    /// Returns [`Error`] when the ABI reports a non-`Ok` status (for example
    /// `Permission` for an undeclared topic, or `Busy` when the borrower-record
    /// lease is exhausted).
    /// REQ: REQ-BIND-002, REQ-BIND-008, REQ-BIND-010, REQ-BIND-004.
    pub fn open_consumer(&self, topic_id: u32) -> Result<Consumer<'_>, Error> {
        let mut raw: *mut drm_consumer = ptr::null_mut();
        // SAFETY: `self.handle` is a live context; `raw` is a valid out-parameter.
        let code = unsafe { drm_consumer_open(self.handle, topic_id, &raw mut raw) };
        check(code)?;
        Ok(Consumer {
            handle: raw,
            _ctx: PhantomData,
            _not_send_sync: PhantomData,
        })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a live handle from `drm_context_open`; every
        // endpoint derived from it has already been dropped (it borrows `&self`).
        let _ = unsafe { drm_context_close(self.handle) };
    }
}

/// RAII owner of a publisher endpoint (the opaque `drm_publisher` handle).
/// REQ: REQ-BIND-002, REQ-BIND-004.
pub struct Publisher<'ctx> {
    /// Live opaque publisher handle.
    handle: *mut drm_publisher,
    /// Ties the endpoint to the context borrow `'ctx` (I-A7).
    _ctx: PhantomData<&'ctx Context>,
    /// Encodes `!Send + !Sync` (REQ: REQ-BIND-016).
    _not_send_sync: NotSendSync,
}

impl Publisher<'_> {
    /// Copy-publishes `data` on the endpoint's topic. The ABI zero-pads a
    /// shorter source to the fixed sample size.
    ///
    /// # Errors
    /// Returns [`Error`] when the ABI reports a non-`Ok` status (for example
    /// `QueueFull`, `PoolExhausted`, or `InvalidArg` when `data` exceeds the
    /// fixed sample size).
    /// REQ: REQ-BIND-002, REQ-BIND-008.
    pub fn publish(&self, data: &[u8]) -> Result<(), Error> {
        // SAFETY: `self.handle` is live; `data` is valid for `data.len()` reads.
        let code = unsafe { drm_publish_copy(self.handle, data.as_ptr(), data.len()) };
        check(code)
    }
}

impl Drop for Publisher<'_> {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a live handle from `drm_publisher_open`.
        let _ = unsafe { drm_publisher_close(self.handle) };
    }
}

/// RAII owner of a consumer endpoint (the opaque `drm_consumer` handle).
///
/// At most one [`BorrowGuard`] may be outstanding at a time (single-borrower
/// endpoint, I-A5). REQ: REQ-BIND-002, REQ-BIND-006, REQ-BIND-004.
pub struct Consumer<'ctx> {
    /// Live opaque consumer handle.
    handle: *mut drm_consumer,
    /// Ties the endpoint to the context borrow `'ctx` (I-A7).
    _ctx: PhantomData<&'ctx Context>,
    /// Encodes `!Send + !Sync` (REQ: REQ-BIND-016).
    _not_send_sync: NotSendSync,
}

impl Consumer<'_> {
    /// Takes the newest committed sample as a transient [`BorrowGuard`].
    ///
    /// # Errors
    /// Returns [`Error`] when the ABI reports a non-`Ok` status: `Busy` while a
    /// borrow is already outstanding on this endpoint, `NoSample` on an empty
    /// channel, and any other code verbatim.
    /// REQ: REQ-BIND-001, REQ-BIND-006.
    pub fn borrow_latest(&self) -> Result<BorrowGuard<'_>, Error> {
        let mut view = drm_borrow_view {
            data: ptr::null(),
            len: 0,
            sequence: 0,
            token: 0,
        };
        // SAFETY: `self.handle` is live; `view` is a valid out-parameter that
        // `drm_borrow_latest` zeroes before any fallible step.
        let code = unsafe { drm_borrow_latest(self.handle, &raw mut view) };
        check(code)?;
        Ok(BorrowGuard {
            consumer: self.handle,
            data: view.data.cast::<u8>(),
            len: view.len,
            sequence: view.sequence,
            token: view.token,
            _chan: PhantomData,
            _not_send_sync: PhantomData,
        })
    }

    /// Copy-reads the newest committed sample into `dst` (owned read).
    ///
    /// # Errors
    /// Returns [`Error`] when the ABI reports a non-`Ok` status (for example
    /// `InvalidArg` when `dst` is smaller than the fixed sample size, or
    /// `NoSample` on an empty channel).
    /// REQ: REQ-BIND-002, REQ-BIND-008.
    ///
    /// ```
    /// use drm::Context;
    ///
    /// # fn main() -> Result<(), drm::Error> {
    /// let ctx = Context::open(None)?;
    /// let publisher = ctx.open_publisher(0)?;
    /// let consumer = ctx.open_consumer(0)?;
    /// publisher.publish(b"hi")?;
    /// let mut buf = [0_u8; 64];
    /// let len = consumer.read_latest(&mut buf)?;
    /// assert_eq!(&buf[..2], b"hi");
    /// assert_eq!(len, 64); // fixed-size topic, zero-padded
    /// # Ok(())
    /// # }
    /// ```
    pub fn read_latest(&self, dst: &mut [u8]) -> Result<usize, Error> {
        let mut out_len: usize = 0;
        // SAFETY: `self.handle` is live; `dst` is valid for `dst.len()` writes;
        // `out_len` is a valid out-parameter.
        let code =
            unsafe { drm_read_latest(self.handle, dst.as_mut_ptr(), dst.len(), &raw mut out_len) };
        check(code)?;
        Ok(out_len)
    }

    /// Copy-reads the next in-order committed sample into `dst` (owned read).
    ///
    /// Returns `Ok(None)` when the consumer is caught up, and
    /// `Ok(Some(ReadOutcome::Gap { missed }))` when the cursor was lapped
    /// (I-C2). The destination is only written on `Ok(Some(..))`.
    ///
    /// # Errors
    /// Returns [`Error`] for any other non-`Ok` status (for example `InvalidArg`
    /// when `dst` is smaller than the fixed sample size).
    /// REQ: REQ-BIND-002, REQ-BIND-008.
    ///
    /// ```
    /// use drm::{Context, ReadOutcome};
    ///
    /// # fn main() -> Result<(), drm::Error> {
    /// let ctx = Context::open(None)?;
    /// let publisher = ctx.open_publisher(0)?;
    /// let consumer = ctx.open_consumer(0)?;
    /// assert!(consumer.read_next(&mut [0_u8; 64])?.is_none()); // caught up
    /// publisher.publish(b"go")?;
    /// match consumer.read_next(&mut [0_u8; 64])? {
    ///     Some(ReadOutcome::Sample { len }) => assert_eq!(len, 64),
    ///     other => panic!("unexpected outcome: {other:?}"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn read_next(&self, dst: &mut [u8]) -> Result<Option<ReadOutcome>, Error> {
        let mut out_len: usize = 0;
        let mut missed: u64 = 0;
        // SAFETY: `self.handle` is live; `dst` is valid for `dst.len()` writes;
        // `out_len`/`missed` are valid out-parameters.
        let code = unsafe {
            drm_read_next(
                self.handle,
                dst.as_mut_ptr(),
                dst.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        };
        match Status::from_code(code) {
            Some(Status::Ok) => Ok(Some(ReadOutcome::Sample { len: out_len })),
            Some(Status::NoSample) => Ok(None),
            Some(Status::Gap) => Ok(Some(ReadOutcome::Gap { missed })),
            Some(status) => Err(Error::Status(status)),
            None => Err(Error::UnknownStatus(code)),
        }
    }
}

impl Drop for Consumer<'_> {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a live handle from `drm_consumer_open`; no
        // borrow is outstanding (any `BorrowGuard` borrows `&self`).
        let _ = unsafe { drm_consumer_close(self.handle) };
    }
}

/// Transient, hazard-protected view of one committed sample.
///
/// Created by [`Consumer::borrow_latest`] and released in [`Drop`]. The guard is
/// `!Send + !Sync`, and [`BorrowGuard::payload`] returns bytes tied to `&self`,
/// so a safe program can neither use the payload after release nor move/share
/// the guard across threads (I-B4). REQ: REQ-BIND-016, REQ-BIND-001.
pub struct BorrowGuard<'chan> {
    /// Live consumer handle borrowed for `'chan`.
    consumer: *mut drm_consumer,
    /// Base of the committed sample (non-null whenever `len > 0`).
    data: *const u8,
    /// Committed sample length in bytes.
    len: usize,
    /// Sequence of the borrowed sample.
    sequence: u64,
    /// Single-outstanding borrow token passed to `drm_borrow_release`.
    token: u32,
    /// Ties the guard to the consumer borrow `'chan`.
    _chan: PhantomData<&'chan ()>,
    /// Encodes `!Send + !Sync` (REQ: REQ-BIND-016).
    _not_send_sync: NotSendSync,
}

impl BorrowGuard<'_> {
    /// The committed sample bytes.
    ///
    /// The returned slice borrows `self`, so it cannot outlive the guard: the
    /// borrow checker rejects any attempt to read it after `Drop` (I-B4).
    /// REQ: REQ-BIND-016, REQ-BIND-001.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: on success the ABI returns a non-null pointer to `len`
        // committed bytes valid until release; the guard (and its borrow of the
        // consumer) is still alive, so the bytes are not yet reclaimed.
        unsafe { core::slice::from_raw_parts(self.data, self.len) }
    }

    /// Committed sample length in bytes.
    /// REQ: REQ-BIND-001.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the borrowed sample is empty.
    /// REQ: REQ-BIND-001.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Sequence number of the borrowed sample.
    /// REQ: REQ-BIND-001.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Opaque single-outstanding borrow token (non-zero on success).
    /// REQ: REQ-BIND-001.
    #[must_use]
    pub const fn token(&self) -> u32 {
        self.token
    }
}

impl Drop for BorrowGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: `self.consumer` is a live handle borrowed for `'chan`, and
        // `self.token` identifies the one outstanding borrow on this endpoint.
        let _ = unsafe { drm_borrow_release(self.consumer, self.token) };
    }
}

/// Unit tests for the safe SDK: `cargo test -p drm sdk`.
/// REQ: REQ-BIND-016, REQ-BIND-001, REQ-BIND-006.
#[cfg(test)]
mod sdk {
    use drm_ffi::ENDPOINT_SAMPLE_MAX;
    use drm_types::Status;

    use super::{Context, Error, ReadOutcome};

    /// The one topic the in-process backing declares. REQ: REQ-BIND-008.
    const TOPIC: u32 = 0;

    /// REQ: REQ-BIND-016, REQ-BIND-001 — `payload(&self)` exposes exactly the
    /// committed sample and nothing outlives the guard.
    #[test]
    fn sdk_borrow_guard_payload_access() {
        let ctx = Context::open(None).unwrap_or_else(|e| unreachable!("open: {e:?}"));
        let publisher = ctx
            .open_publisher(TOPIC)
            .unwrap_or_else(|e| unreachable!("publisher: {e:?}"));
        let consumer = ctx
            .open_consumer(TOPIC)
            .unwrap_or_else(|e| unreachable!("consumer: {e:?}"));

        let sample = [0xA5_u8; 16];
        publisher
            .publish(&sample)
            .unwrap_or_else(|e| unreachable!("publish: {e:?}"));

        // A shorter source publish is zero-padded to the fixed sample size.
        let mut expected = [0_u8; ENDPOINT_SAMPLE_MAX];
        expected[..sample.len()].copy_from_slice(&sample);

        let guard = consumer
            .borrow_latest()
            .unwrap_or_else(|e| unreachable!("borrow: {e:?}"));
        assert_eq!(guard.payload(), &expected[..]);
        assert_eq!(guard.len(), ENDPOINT_SAMPLE_MAX);
        assert!(!guard.is_empty());
        assert_ne!(guard.token(), 0, "a successful borrow carries a live token");
        let _ = guard.sequence();
    }

    /// REQ: REQ-BIND-016, REQ-BIND-006 — `Drop` releases the hazard slot: a
    /// fresh borrow then succeeds where a still-pinned slot would return `Busy`.
    #[test]
    fn sdk_release_on_drop() {
        let ctx = Context::open(None).unwrap_or_else(|e| unreachable!("open: {e:?}"));
        let publisher = ctx
            .open_publisher(TOPIC)
            .unwrap_or_else(|e| unreachable!("publisher: {e:?}"));
        let consumer = ctx
            .open_consumer(TOPIC)
            .unwrap_or_else(|e| unreachable!("consumer: {e:?}"));
        publisher
            .publish(b"abc")
            .unwrap_or_else(|e| unreachable!("publish: {e:?}"));

        let first_token = {
            let guard = consumer
                .borrow_latest()
                .unwrap_or_else(|e| unreachable!("borrow: {e:?}"));
            guard.token()
        }; // ~BorrowGuard → drm_borrow_release

        // If `Drop` had not released, this borrow would be refused with `Busy`.
        let again = consumer
            .borrow_latest()
            .unwrap_or_else(|e| unreachable!("re-borrow after drop: {e:?}"));
        assert_ne!(
            again.token(),
            first_token,
            "each borrow issues a fresh token"
        );
    }

    /// REQ: REQ-BIND-016, REQ-BIND-006 — a second borrow while one is held is
    /// refused at runtime (one outstanding borrow per consumer endpoint, I-A5).
    #[test]
    fn sdk_second_borrow_while_held_is_refused() {
        let ctx = Context::open(None).unwrap_or_else(|e| unreachable!("open: {e:?}"));
        let publisher = ctx
            .open_publisher(TOPIC)
            .unwrap_or_else(|e| unreachable!("publisher: {e:?}"));
        let consumer = ctx
            .open_consumer(TOPIC)
            .unwrap_or_else(|e| unreachable!("consumer: {e:?}"));
        publisher
            .publish(b"xyz")
            .unwrap_or_else(|e| unreachable!("publish: {e:?}"));

        let held = consumer
            .borrow_latest()
            .unwrap_or_else(|e| unreachable!("first borrow: {e:?}"));
        match consumer.borrow_latest() {
            Err(Error::Status(Status::Busy)) => {}
            Err(other) => unreachable!("second borrow refused with unexpected error: {other:?}"),
            Ok(_) => unreachable!("second borrow must be refused while one is held"),
        }
        // Releasing the held guard frees the endpoint again.
        drop(held);
        let _ = consumer
            .borrow_latest()
            .unwrap_or_else(|e| unreachable!("borrow after release: {e:?}"));
    }

    /// REQ: REQ-BIND-002 — owned `read_latest` copies the newest sample into
    /// caller memory (retained after the call) and reports the fixed length.
    #[test]
    fn sdk_owned_read_latest() {
        let ctx = Context::open(None).unwrap_or_else(|e| unreachable!("open: {e:?}"));
        let publisher = ctx
            .open_publisher(TOPIC)
            .unwrap_or_else(|e| unreachable!("publisher: {e:?}"));
        let consumer = ctx
            .open_consumer(TOPIC)
            .unwrap_or_else(|e| unreachable!("consumer: {e:?}"));
        publisher
            .publish(b"owned")
            .unwrap_or_else(|e| unreachable!("publish: {e:?}"));

        let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
        let len = consumer
            .read_latest(&mut buf)
            .unwrap_or_else(|e| unreachable!("read: {e:?}"));
        assert_eq!(len, ENDPOINT_SAMPLE_MAX);
        assert_eq!(&buf[..5], b"owned");
    }

    /// REQ: REQ-BIND-002 — sequential owned read returns `None` when caught up
    /// and `Some(Sample)` once a sample is published.
    #[test]
    fn sdk_owned_read_next() {
        let ctx = Context::open(None).unwrap_or_else(|e| unreachable!("open: {e:?}"));
        let publisher = ctx
            .open_publisher(TOPIC)
            .unwrap_or_else(|e| unreachable!("publisher: {e:?}"));
        let consumer = ctx
            .open_consumer(TOPIC)
            .unwrap_or_else(|e| unreachable!("consumer: {e:?}"));

        let mut buf = [0_u8; ENDPOINT_SAMPLE_MAX];
        assert!(
            consumer
                .read_next(&mut buf)
                .unwrap_or_else(|e| unreachable!("read: {e:?}"))
                .is_none(),
            "a caught-up consumer reports no sample"
        );

        publisher
            .publish(b"seq")
            .unwrap_or_else(|e| unreachable!("publish: {e:?}"));
        match consumer
            .read_next(&mut buf)
            .unwrap_or_else(|e| unreachable!("read: {e:?}"))
        {
            Some(ReadOutcome::Sample { len }) => {
                assert_eq!(len, ENDPOINT_SAMPLE_MAX);
                assert_eq!(&buf[..3], b"seq");
            }
            other => unreachable!("expected a delivered sample, got {other:?}"),
        }
    }

    /// REQ: REQ-BIND-016 — the SDK defines no C-layout (`repr(C)`) struct, so
    /// the golden-layout check (work-plan M07W3-E4-T2) does not apply; this
    /// guards against a future layout type landing without a golden assertion.
    #[test]
    fn sdk_defines_no_repr_c_structs() {
        let needle = concat!("#[repr", "(C)]");
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let mut offenders = Vec::new();
        while let Some(dir) = stack.pop() {
            let entries =
                std::fs::read_dir(&dir).unwrap_or_else(|e| unreachable!("read {dir:?}: {e:?}"));
            for entry in entries {
                let path = entry
                    .unwrap_or_else(|e| unreachable!("entry: {e:?}"))
                    .path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| unreachable!("read {path:?}: {e:?}"));
                    if text.contains(needle) {
                        offenders.push(path);
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "SDK must define no C-repr structs (none expected); add a golden \
             layout check for: {offenders:?}"
        );
    }
}
