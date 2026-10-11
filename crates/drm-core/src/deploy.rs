//! Deployment identity guard, namespace scoping, and guarded cleanup
//! (`docs/algorithms/deployment-identity.md` §Functions `node_startup`,
//! `segment_open`, `clean`; `docs/algorithms/runtime/shm-backend.md` §Functions
//! `remove_namespace`).
//!
//! Three-identity model: the compiled contract fingerprint (this module), the
//! deployment instance id (this module), and publisher epochs
//! (`crate::channel::lifecycle`). Namespace scoping reuses
//! [`crate::shm::backend_segment_name`]; locks reuse [`crate::shm::LockPaths`]
//! and [`crate::shm::LockBackend`].
//! REQ: REQ-CORE-084, REQ-CORE-085, REQ-CORE-088, REQ-CORE-089, REQ-CORE-092,
//! REQ-CORE-080, REQ-CORE-069, REQ-CORE-070.

use crate::shm::{
    CleanReport, LockBackend, LockHandle, LockPaths, SegmentName, ShmBackend, ShmError,
    backend_segment_name,
};
use drm_types::Status;

/// Compiled contract fingerprint: hash over the node's own interface inputs
/// (ABI version, topic schemas/layouts, declared interfaces), truncated to 128
/// bits. Width parity with `drm-workspace::ContractFingerprint`; kept local so
/// `drm-core` needs no toolchain dependency (codebase.md 2.2 R2).
/// REQ: REQ-CORE-085.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractFingerprint(
    /// 128-bit truncated digest.
    pub [u8; 16],
);

/// 128-bit random deployment instance id (runtime config, never compiled into
/// binaries). Encoded as a backend-safe token for namespace derivation.
/// REQ: REQ-CORE-084.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeploymentId(u128);

impl DeploymentId {
    /// Wraps a raw 128-bit id.
    /// REQ: REQ-CORE-084.
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    /// Backend-safe namespace token: 32 lowercase hex digits (fixed width, no
    /// `/`), derived solely from the id — a hand-edited `deployment.json`
    /// cannot redirect a node or `drm clean` at a foreign namespace (I-DP5).
    /// REQ: REQ-CORE-084, REQ-CORE-088.
    #[must_use]
    pub fn encoded(self) -> String {
        let raw = self.0;
        format!("{raw:032x}")
    }

    /// Deployment-scoped segment name `/drm.<encoded-id>.<slot>`: two
    /// deployments never share a name and the canonical topic never reaches the
    /// kernel namespace (I-DP2).
    /// REQ: REQ-CORE-088, REQ-CORE-089.
    ///
    /// # Errors
    /// [`ShmError`] when the generated name violates name conformance.
    pub fn segment_name(self, slot: u64) -> Result<SegmentName, ShmError> {
        backend_segment_name(&self.encoded(), slot)
    }
}

/// Startup fingerprint guard (I-DP1): the compiled fingerprint must equal the
/// deployment's expectation for this node. Pure comparison — no lock, no SHM
/// touch — so a refusal can never create or open a segment.
/// REQ: REQ-CORE-092, REQ-CORE-025.
///
/// # Errors
/// [`Status::NotFound`] when the deployment records no expectation for this
/// node; [`Status::WrongAbi`] on mismatch.
pub fn guard_fingerprint(
    compiled: ContractFingerprint,
    expected: Option<ContractFingerprint>,
) -> Result<(), Status> {
    match expected {
        Some(e) if e == compiled => Ok(()),
        Some(_) => Err(Status::WrongAbi),
        None => Err(Status::NotFound),
    }
}

/// Node startup sequence (deployment-identity.md §Protocols): the fingerprint
/// guard runs FIRST, then the shared deployment active hold is acquired for the
/// node's whole lifetime. A guard refusal returns before any lock or SHM object
/// is touched.
/// REQ: REQ-CORE-092, REQ-CORE-025, REQ-CORE-069.
///
/// # Errors
/// [`Status::NotFound`]/[`Status::WrongAbi`] from [`guard_fingerprint`];
/// [`Status::AlreadyExists`] when a `drm clean` is in flight.
pub fn node_startup<L: LockBackend>(
    locks: &L,
    paths: &LockPaths,
    compiled: ContractFingerprint,
    expected: Option<ContractFingerprint>,
) -> Result<LockHandle, Status> {
    guard_fingerprint(compiled, expected)?;
    locks.try_shared(&paths.active()).map_err(Status::from)
}

/// Guarded namespace teardown (`drm clean`; shm-backend.md §Functions
/// `remove_namespace`): the exclusive deployment active lock proves no node
/// holds a shared hold. Success ⇒ every segment unlinked (`Cleaned`); a live
/// shared hold ⇒ nothing touched (`ActiveUntouched`). The exclusive handle is
/// held until all unlinking completes, so no node can acquire the namespace
/// mid-clean. Missing segments are tolerated (idempotent clean).
/// REQ: REQ-CORE-080, REQ-CORE-070, REQ-CORE-069.
///
/// # Errors
/// [`ShmError`] when the active lock cannot be probed or a segment unlink fails
/// for a reason other than absence.
pub fn clean<S: ShmBackend, L: LockBackend>(
    shm: &S,
    locks: &L,
    paths: &LockPaths,
    names: &[SegmentName],
) -> Result<CleanReport, ShmError> {
    let _exclusive = match locks.try_exclusive(&paths.active()) {
        Ok(h) => h,
        Err(ShmError::AlreadyExists) => return Ok(CleanReport::ActiveUntouched),
        Err(e) => return Err(e),
    };
    for name in names {
        match shm.unlink(name) {
            Ok(()) | Err(ShmError::NotFound) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(CleanReport::Cleaned)
}
