//! Per-channel precommit activation (docs/algorithms/memory-budget.md
//! §Functions `activate_precommitted`, §Protocols *Precommit activation*;
//! docs/architecture.md §Hard Real-Time Precommitment). Step 1 pre-faults every
//! declared page by write-touching it; step 2 memory-locks the range. Either
//! step's failure returns an explicit status and no active-state value — the
//! channel is never left active or pageable (I-MB5). The lock step reuses the
//! platform proof [`crate::shm::ShmBackend::precommit_and_prove`] (Linux
//! `mlock`), so the residency proof has a single source of truth.
//! REQ: REQ-CORE-090.

use crate::shm::{Mapping, ShmError};
use drm_types::Status;

/// Platform lockability seam for precommit activation (memory-budget.md
/// §Functions `activate_precommitted`). The real Linux backend is
/// [`crate::shm::PosixShm`]; a test double implements this trait to force the
/// low-lockable-budget refusal deterministically.
/// REQ: REQ-CORE-090.
pub trait PrecommitProver {
    /// Pre-fault every declared page of `m` by write-touching it (step 1).
    /// REQ: REQ-CORE-090.
    ///
    /// # Errors
    /// [`ShmError`] when a declared page cannot be faulted in.
    fn prefault(&self, m: &Mapping) -> Result<(), ShmError>;

    /// Memory-lock `m`'s declared range (step 2); success is the residency
    /// proof (all intersecting pages resident until unmapped). REQ: REQ-CORE-090.
    ///
    /// # Errors
    /// [`ShmError::QuotaExceeded`] when the lockable budget is too small
    /// (`EPERM` ⇒ `RLIMIT_MEMLOCK`); another [`ShmError`] on OS failure.
    fn lock_range(&self, m: &Mapping) -> Result<(), ShmError>;
}

/// OS page size for the pre-fault stride (`sysconf(_SC_PAGESIZE)`; 4096
/// fallback when the query is unavailable).
#[cfg(unix)]
fn page_size() -> usize {
    // SAFETY: `sysconf` with a valid name is a pure query with no side effects.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(raw).ok().filter(|n| *n > 0).unwrap_or(4096)
}

/// Pre-faults every declared page of `m` by write-touching one byte per page:
/// reads the byte back and writes the same value, so the page becomes resident
/// (dirtied) without changing content. Infallible on a live mapping; the
/// fallible seam contract lives on [`PrecommitProver::prefault`].
/// REQ: REQ-CORE-090.
#[cfg(unix)]
fn prefault_pages(m: &Mapping) {
    let base = m.ptr().as_ptr();
    let page = page_size();
    let mut off = 0_usize;
    while off < m.len() {
        // SAFETY: `off < m.len()` and the mapping covers `m.len()` writable
        // bytes at `base`; reading then writing the same byte preserves the
        // value while faulting the page in for write.
        unsafe {
            let p = base.add(off);
            p.write_volatile(p.read_volatile());
        }
        off = match off.checked_add(page) {
            Some(next) => next,
            None => break,
        };
    }
}

/// Linux precommit prover: write-touch pre-fault plus the shared `mlock` proof.
/// REQ: REQ-CORE-090.
#[cfg(unix)]
impl PrecommitProver for crate::shm::PosixShm {
    fn prefault(&self, m: &Mapping) -> Result<(), ShmError> {
        prefault_pages(m);
        Ok(())
    }

    fn lock_range(&self, m: &Mapping) -> Result<(), ShmError> {
        crate::shm::ShmBackend::precommit_and_prove(self, m)
    }
}

/// Proof that a channel's declared region was pre-faulted and memory-locked. It
/// is the only value that marks a channel active; a failed activation returns
/// no value, so there is no pageable continuation (I-MB5). The field is
/// private — only [`activate_precommitted`] constructs it.
/// REQ: REQ-CORE-090.
///
/// The proof cannot be forged; only a successful activation yields one:
///
/// ```compile_fail
/// use drm_core::channel::precommit::PrecommitProof;
/// let _ = PrecommitProof { len: 1 };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecommitProof {
    len: usize,
}

impl PrecommitProof {
    /// Proven (pre-faulted and locked) byte length.
    #[must_use]
    pub fn len(self) -> usize {
        self.len
    }

    /// Empty-region predicate (`clippy::len_without_is_empty`).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Per-channel precommit activation (memory-budget.md §Functions
/// `activate_precommitted`; architecture.md §Hard Real-Time Precommitment).
/// Step 1 pre-faults every declared page by write-touching it; step 2
/// memory-locks the range. On failure of either step the call returns an
/// explicit status and no [`PrecommitProof`], so the caller can never mark the
/// channel active — continuing pageable is forbidden (I-MB5).
/// REQ: REQ-CORE-090.
///
/// # Errors
/// [`Status::InvalidConfig`] when the pre-fault step fails or the lock step
/// reports a lockable-budget refusal; [`Status::LockFailed`] when the lock step
/// fails for any other OS reason.
pub fn activate_precommitted<P: PrecommitProver>(
    prover: &P,
    m: &Mapping,
) -> Result<PrecommitProof, Status> {
    prover.prefault(m).map_err(|_| Status::InvalidConfig)?;
    prover.lock_range(m).map_err(|e| match e {
        ShmError::QuotaExceeded => Status::InvalidConfig,
        _ => Status::LockFailed,
    })?;
    Ok(PrecommitProof { len: m.len() })
}

/// An activated precommit channel: the sole handle for a channel that must not
/// run pageable. Constructed only by [`PrecommittedChannel::activate`], which
/// proves the declared region before building the channel; any failure yields
/// no handle (I-MB5).
/// REQ: REQ-CORE-090.
pub struct PrecommittedChannel {
    channel: super::Channel,
    proof: PrecommitProof,
}

impl PrecommittedChannel {
    /// Pre-faults and locks `region` first, then builds the channel via
    /// `build`. If either proof step or `build` fails the call returns `Err`
    /// and never leaves a channel active (I-MB5).
    /// REQ: REQ-CORE-090.
    ///
    /// # Errors
    /// As [`activate_precommitted`] for the proof steps; the channel
    /// constructor's own [`Status`] otherwise.
    pub fn activate<P, F>(prover: &P, region: &Mapping, build: F) -> Result<Self, Status>
    where
        P: PrecommitProver,
        F: FnOnce() -> Result<super::Channel, Status>,
    {
        let proof = activate_precommitted(prover, region)?;
        let channel = build()?;
        Ok(Self { channel, proof })
    }

    /// The active channel.
    #[must_use]
    pub fn channel(&self) -> &super::Channel {
        &self.channel
    }

    /// The completed precommit proof.
    #[must_use]
    pub fn proof(&self) -> PrecommitProof {
        self.proof
    }
}

impl std::fmt::Debug for PrecommittedChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrecommittedChannel")
            .field("proof", &self.proof)
            .finish_non_exhaustive()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{PrecommitProver, PrecommittedChannel, activate_precommitted};
    use crate::channel::{Channel, OverflowPolicy};
    use crate::shm::{Mapping, PosixShm, SegmentName, ShmBackend, ShmError};
    use drm_types::Status;
    use std::cell::Cell;
    use std::io;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Which activation step the stub refuses (the injectable failure seam).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Refuse {
        /// Both steps succeed (deterministic success path).
        Nothing,
        /// Step 1 refuses.
        Prefault,
        /// Step 2 refuses with the low-lockable-budget (`EPERM`) family error.
        LockQuota,
        /// Step 2 refuses with a non-quota OS error.
        LockIo,
    }

    /// Stub prover: records call order and refuses the selected step, so the
    /// low-lockable-budget refusal is deterministic without kernel pressure.
    struct StubProver {
        refuse: Refuse,
        prefaulted: Cell<bool>,
        locked: Cell<bool>,
    }

    impl StubProver {
        fn new(refuse: Refuse) -> Self {
            Self {
                refuse,
                prefaulted: Cell::new(false),
                locked: Cell::new(false),
            }
        }
    }

    impl PrecommitProver for StubProver {
        fn prefault(&self, _m: &Mapping) -> Result<(), ShmError> {
            if self.refuse == Refuse::Prefault {
                return Err(ShmError::Io(io::Error::other("prefault refused")));
            }
            self.prefaulted.set(true);
            Ok(())
        }

        fn lock_range(&self, _m: &Mapping) -> Result<(), ShmError> {
            self.locked.set(true);
            match self.refuse {
                Refuse::LockQuota => Err(ShmError::QuotaExceeded),
                Refuse::LockIo => Err(ShmError::Io(io::Error::other("lock refused"))),
                _ => Ok(()),
            }
        }
    }

    /// A real, uniquely named shared mapping standing in for a channel's
    /// declared region; the name is unlinked immediately (the object survives
    /// through the mapping) so reruns start clean.
    fn real_mapping(tag: &str) -> Mapping {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let shm = PosixShm;
        let name = match SegmentName::new(&format!(
            "/drm.precommit.{}.{}.{tag}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )) {
            Ok(n) => n,
            Err(e) => unreachable!("test name: {e:?}"),
        };
        let _ = shm.unlink(&name);
        let m = match shm.create_exclusive(&name, 4096) {
            Ok(m) => m,
            Err(e) => unreachable!("create mapping: {e:?}"),
        };
        if let Err(e) = shm.unlink(&name) {
            unreachable!("unlink mapping name: {e:?}");
        }
        m
    }

    fn build_channel() -> Result<Channel, Status> {
        Channel::activate(4, 5, 16, 8, OverflowPolicy::Overwrite, 0, 2, None)
    }

    /// REQ: REQ-CORE-090 — success path: a real backend pre-faults and locks the
    /// declared region, yielding a proof. If this environment refuses `mlock`
    /// (`RLIMIT_MEMLOCK` too small) the refusal path is asserted instead and the
    /// limitation is reported; a success is never faked.
    #[test]
    fn precommit_activation_success() {
        let m = real_mapping("ok");
        match activate_precommitted(&PosixShm, &m) {
            Ok(proof) => assert_eq!(proof.len(), 4096),
            Err(Status::InvalidConfig) => {
                eprintln!(
                    "precommit_activation_success: mlock refused by this environment \
                     (RLIMIT_MEMLOCK); asserting the refusal path instead"
                );
                let stub = StubProver::new(Refuse::LockQuota);
                assert_eq!(activate_precommitted(&stub, &m), Err(Status::InvalidConfig));
            }
            Err(e) => unreachable!("unexpected precommit status: {e:?}"),
        }
    }

    /// REQ: REQ-CORE-090 — deterministic success: a prover that runs both steps
    /// yields an active channel whose proof matches the declared region.
    #[test]
    fn precommit_success_locked_is_active() {
        let m = real_mapping("active");
        let built = Cell::new(false);
        let ctor = || {
            built.set(true);
            build_channel()
        };
        match PrecommittedChannel::activate(&StubProver::new(Refuse::Nothing), &m, ctor) {
            Ok(ch) => {
                assert!(built.get(), "channel built after a successful proof");
                assert_eq!(ch.proof().len(), 4096);
                assert_eq!(ch.channel().pool_size(), 5);
            }
            Err(e) => unreachable!("stub success: {e:?}"),
        }
    }

    /// REQ: REQ-CORE-090 — low-lockable-budget refusal: the injected quota-family
    /// lock failure yields `InvalidConfig`; the pre-fault ran first.
    #[test]
    fn precommit_refusal_low_lockable_budget() {
        let m = real_mapping("budget");
        let stub = StubProver::new(Refuse::LockQuota);
        assert_eq!(activate_precommitted(&stub, &m), Err(Status::InvalidConfig));
        assert!(stub.prefaulted.get(), "pre-fault runs before the lock step");
        assert!(stub.locked.get(), "the lock step was attempted and refused");
    }

    /// REQ: REQ-CORE-090 — a non-quota lock failure is `LockFailed`.
    #[test]
    fn precommit_refusal_lock_failure() {
        let m = real_mapping("lockfail");
        assert_eq!(
            activate_precommitted(&StubProver::new(Refuse::LockIo), &m),
            Err(Status::LockFailed)
        );
    }

    /// REQ: REQ-CORE-090 — pre-fault refusal yields `InvalidConfig` and never
    /// reaches the lock step (no partial proof).
    #[test]
    fn precommit_refusal_prefault_failure() {
        let m = real_mapping("prefault");
        let stub = StubProver::new(Refuse::Prefault);
        assert_eq!(activate_precommitted(&stub, &m), Err(Status::InvalidConfig));
        assert!(
            !stub.locked.get(),
            "a failed pre-fault must not reach the lock step"
        );
    }

    /// REQ: REQ-CORE-090 — per-channel refusal leaves no active handle: the
    /// channel is never even built when the proof fails (I-MB5).
    #[test]
    fn precommit_refusal_leaves_no_active_channel() {
        let m = real_mapping("nochan");
        let built = Cell::new(false);
        let ctor = || {
            built.set(true);
            build_channel()
        };
        let result = PrecommittedChannel::activate(&StubProver::new(Refuse::LockQuota), &m, ctor);
        assert!(result.is_err(), "no active channel on refusal");
        assert!(!built.get(), "the channel is never built without a proof");
    }

    /// REQ: REQ-CORE-090 — audit: no pageable-continuation path exists. The
    /// active-state token has exactly one construction site, reachable only
    /// after both proof steps, with no failure-swallowing default; and every
    /// injected step failure yields `Err`.
    #[test]
    fn precommit_no_pageable_continuation() {
        // Structural audit of the activation body (between the activation
        // function and the next free function, so the test source is excluded).
        let src = include_str!("precommit.rs");
        let start = src
            .find("pub fn activate_precommitted")
            .unwrap_or(usize::MAX);
        let end = src
            .find("pub struct PrecommittedChannel")
            .unwrap_or(usize::MAX);
        let body = if start < end {
            src.get(start..end).unwrap_or("")
        } else {
            ""
        };
        assert!(body.contains("prefault(m)"), "step 1 present: {body}");
        assert!(body.contains("lock_range(m)"), "step 2 present: {body}");
        assert_eq!(
            body.matches("Ok(PrecommitProof").count(),
            1,
            "exactly one active-state construction site"
        );
        assert!(
            !body.contains("unwrap") && !body.contains("Default"),
            "no failure-swallowing default in the activation path: {body}"
        );

        // Behavioural audit: either injected step failure refuses (never Ok).
        let m = real_mapping("audit");
        assert!(activate_precommitted(&StubProver::new(Refuse::Prefault), &m).is_err());
        assert!(activate_precommitted(&StubProver::new(Refuse::LockQuota), &m).is_err());
    }
}
