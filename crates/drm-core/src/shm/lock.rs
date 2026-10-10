//! OS-backed advisory locks, verbatim from
//! `docs/algorithms/runtime/shm-backend.md` §Data Structures (`LockBackend`) and
//! §Protocols (writer authority, cleanup probe). `flock(2)` semantics: locks
//! bind to the open file description; lock descriptors are `O_CLOEXEC`; release
//! happens by closing the descriptor — never an explicit `LOCK_UN`, so an
//! inherited duplicate cannot release the parent's authority (fork
//! sanitization, I5). NFS/flock semantics are out of scope (documented
//! exclusion). Shared acquisition is non-blocking like the exclusive path: a
//! `drm clean` in flight makes node activation fail explicitly rather than
//! block.
//! REQ: REQ-CORE-073, REQ-CORE-069.

use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::{LockHandle, ShmError};

/// `flock(2)`-based [`super::LockBackend`] implementation.
/// REQ: REQ-CORE-073.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlockBackend;

/// Contention or failure while taking a lock: the `AlreadyExists`-family busy
/// failure reads as "another live holder exists" at every call site.
fn lock_busy() -> ShmError {
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        // Linux: `EWOULDBLOCK` == `EAGAIN`; flock(2) reports one value.
        Some(libc::EWOULDBLOCK) => ShmError::AlreadyExists,
        _ => ShmError::Io(e),
    }
}

fn lock_fd(path: &Path, op: libc::c_int) -> Result<LockHandle, ShmError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
        .map_err(ShmError::Io)?;
    // SAFETY: `fd` is an owned, valid descriptor for the lock file; a
    // non-blocking `flock` only reports `EWOULDBLOCK`/`EAGAIN` on contention.
    let rc = unsafe { libc::flock(file.as_raw_fd(), op) };
    if rc != 0 {
        return Err(lock_busy());
    }
    Ok(LockHandle::from_fd(file.into()))
}

impl super::LockBackend for FlockBackend {
    fn try_exclusive(&self, path: &Path) -> Result<LockHandle, ShmError> {
        lock_fd(path, libc::LOCK_EX | libc::LOCK_NB)
    }

    fn try_shared(&self, path: &Path) -> Result<LockHandle, ShmError> {
        lock_fd(path, libc::LOCK_SH | libc::LOCK_NB)
    }

    fn release(self, h: LockHandle) {
        drop(h);
    }
}

#[cfg(test)]
mod tests {
    use super::FlockBackend;
    use crate::shm::{LockBackend, ShmError};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn lock_path(tag: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("drm-lock-{}-{tag}-{n}.lock", std::process::id()))
    }

    /// REQ: REQ-CORE-073 — writer authority: a second exclusive acquisition
    /// fails busy; releasing the hold re-enables acquisition (I4 in-process).
    #[test]
    fn exclusive_is_exclusive() {
        let _serial = crate::test_guard::acquire();
        let b = FlockBackend;
        let p = lock_path("ex-ex");
        let h = match b.try_exclusive(&p) {
            Ok(h) => h,
            Err(e) => unreachable!("exclusive lock: {e:?}"),
        };
        assert!(matches!(b.try_exclusive(&p), Err(ShmError::AlreadyExists)));
        b.release(h);
        let h2 = match b.try_exclusive(&p) {
            Ok(h) => h,
            Err(e) => unreachable!("re-lock: {e:?}"),
        };
        b.release(h2);
        let _ = std::fs::remove_file(&p);
    }

    /// REQ: REQ-CORE-069 — shared holds coexist (nodes), exclusive is blocked
    /// while any shared hold is live, and exclusive wins once all shared holds
    /// drop (active-lock / cleanup-probe model).
    #[test]
    fn shared_blocks_exclusive() {
        let _serial = crate::test_guard::acquire();
        let b = FlockBackend;
        let p = lock_path("sh-ex");
        let s1 = match b.try_shared(&p) {
            Ok(h) => h,
            Err(e) => unreachable!("shared 1: {e:?}"),
        };
        let s2 = match b.try_shared(&p) {
            Ok(h) => h,
            Err(e) => unreachable!("shared 2: {e:?}"),
        };
        assert!(matches!(b.try_exclusive(&p), Err(ShmError::AlreadyExists)));
        b.release(s1);
        assert!(matches!(b.try_exclusive(&p), Err(ShmError::AlreadyExists)));
        b.release(s2);
        let x = match b.try_exclusive(&p) {
            Ok(h) => h,
            Err(e) => unreachable!("exclusive after shared drop: {e:?}"),
        };
        assert!(matches!(b.try_shared(&p), Err(ShmError::AlreadyExists)));
        b.release(x);
        let _ = std::fs::remove_file(&p);
    }
}
