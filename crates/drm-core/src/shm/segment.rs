//! POSIX SHM backend bodies, verbatim from
//! `docs/algorithms/runtime/shm-backend.md` §Functions (`create_exclusive`,
//! `open_raw`, `open_existing`, `precommit_and_prove`) with the §Protocols
//! create-rollback guard (I7). Linux-only; Windows variants live in
//! `windows-backends.md`.
//! REQ: REQ-CORE-072, REQ-CORE-098, REQ-CORE-070, REQ-CORE-090.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr::{NonNull, null_mut};

use super::{Mapping, SegmentName, ShmError};
use crate::channel::ccb::SegmentPrefix;

/// `shm_open(3)`-based [`super::ShmBackend`] implementation.
/// REQ: REQ-CORE-072.
#[derive(Debug, Clone, Copy, Default)]
pub struct PosixShm;

/// Arms unlink-on-drop until the complete mapping exists (I7: every failure
/// after exclusive creation closes the fd and unlinks the incomplete name).
struct UnlinkOnDrop<'a> {
    name: &'a SegmentName,
    armed: bool,
}

impl Drop for UnlinkOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort: an incomplete name must not survive a failed create.
            let _ = shm_unlink_name(self.name);
        }
    }
}

impl UnlinkOnDrop<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

/// errno → [`ShmError`] for the documented special cases; everything else stays
/// a raw [`ShmError::Io`].
fn err_from_errno() -> ShmError {
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EEXIST) => ShmError::AlreadyExists,
        Some(libc::ENOENT) => ShmError::NotFound,
        Some(libc::EACCES) => ShmError::PermissionDenied,
        Some(libc::ENOSPC | libc::EDQUOT) => ShmError::QuotaExceeded,
        _ => ShmError::Io(e),
    }
}

fn invalid_name() -> ShmError {
    ShmError::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        "interior NUL in segment name",
    ))
}

fn shm_fd(name: &SegmentName, oflag: libc::c_int) -> Result<OwnedFd, ShmError> {
    let cname = CString::new(name.as_str()).map_err(|_| invalid_name())?;
    // SAFETY: `cname` is NUL-terminated for the duration of the call and
    // `shm_open` does not retain the pointer.
    let fd = unsafe { libc::shm_open(cname.as_ptr(), oflag, 0o600) };
    if fd < 0 {
        return Err(err_from_errno());
    }
    // SAFETY: `fd` is an owned descriptor just returned by `shm_open`.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn shm_unlink_name(name: &SegmentName) -> Result<(), ShmError> {
    let cname = CString::new(name.as_str()).map_err(|_| invalid_name())?;
    // SAFETY: NUL-terminated name for the duration of the call.
    if unsafe { libc::shm_unlink(cname.as_ptr()) } != 0 {
        return Err(err_from_errno());
    }
    Ok(())
}

fn map_fd(fd: RawFd, size: usize) -> Result<Mapping, ShmError> {
    // SAFETY: `size` bytes are established on `fd` (ftruncate + posix_fallocate)
    // before this call; `MAP_SHARED` keeps cross-process coherence; offset 0.
    let raw = unsafe {
        libc::mmap(
            null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if raw == libc::MAP_FAILED {
        return Err(err_from_errno());
    }
    NonNull::new(raw.cast::<u8>())
        .map(|p| Mapping::from_raw_parts(p, size))
        .ok_or_else(|| ShmError::Io(io::Error::other("mmap returned null")))
}

fn fallocate_err(rc: libc::c_int) -> ShmError {
    let e = io::Error::from_raw_os_error(rc);
    match e.raw_os_error() {
        Some(libc::ENOSPC | libc::EDQUOT) => ShmError::QuotaExceeded,
        _ => ShmError::Io(e),
    }
}

fn off_t_of(size: usize) -> Result<libc::off_t, ShmError> {
    libc::off_t::try_from(size).map_err(|_| {
        ShmError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "size exceeds off_t",
        ))
    })
}

impl super::ShmBackend for PosixShm {
    fn create_exclusive(&self, name: &SegmentName, size: usize) -> Result<Mapping, ShmError> {
        let fd = shm_fd(name, libc::O_CREAT | libc::O_EXCL | libc::O_RDWR)?;
        let mut rollback = UnlinkOnDrop { name, armed: true };
        let sz = off_t_of(size)?;
        // SAFETY: `fd` is an owned, valid descriptor.
        if unsafe { libc::ftruncate(fd.as_raw_fd(), sz) } != 0 {
            return Err(err_from_errno());
        }
        // Reserve the tmpfs extent up front: no later SIGBUS on our own mapping.
        // REQ: REQ-CORE-072.
        let rc = unsafe { libc::posix_fallocate(fd.as_raw_fd(), 0, sz) };
        if rc != 0 {
            return Err(fallocate_err(rc));
        }
        let mapping = map_fd(fd.as_raw_fd(), size)?;
        rollback.disarm();
        Ok(mapping)
    }

    fn open_raw(&self, name: &SegmentName) -> Result<(OwnedFd, u64), ShmError> {
        let fd = shm_fd(name, libc::O_RDWR)?;
        let file = File::from(fd);
        let size = file.metadata().map_err(ShmError::Io)?.len();
        Ok((OwnedFd::from(file), size))
    }

    fn open_existing(&self, name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError> {
        let (fd, size) = self.open_raw(name)?;
        // A POSIX SHM object is born zero-length; ftruncate(size) establishes the
        // full size in one atomic step, so a live object is either 0 (creation
        // never completed) or full. Anything below the fixed 16-byte
        // `SegmentPrefix` is unclassifiable ⇒ stale (shm-backend.md §Functions).
        // REQ: REQ-CORE-098.
        if size < u64::try_from(size_of::<SegmentPrefix>()).unwrap_or(u64::MAX) {
            return Err(ShmError::StaleIncomplete);
        }
        if size != u64::try_from(expect_size).unwrap_or(u64::MAX) {
            return Err(ShmError::SizeMismatch);
        }
        let mapping = map_fd(fd.as_raw_fd(), expect_size)?;
        Ok(mapping)
    }

    fn unmap(&self, m: Mapping) {
        drop(m);
    }

    fn unlink(&self, name: &SegmentName) -> Result<(), ShmError> {
        shm_unlink_name(name)
    }

    fn precommit_and_prove(&self, m: &Mapping) -> Result<(), ShmError> {
        if m.is_empty() {
            return Ok(());
        }
        // SAFETY: `ptr`/`len` describe a live `MAP_SHARED` mapping owned by `m`.
        if unsafe { libc::mlock(m.ptr().cast().as_ptr(), m.len()) } != 0 {
            // EPERM ⇒ RLIMIT_MEMLOCK too small: explicit quota-family failure,
            // never a silent pageable fallback (I9).
            let e = io::Error::last_os_error();
            return Err(match e.raw_os_error() {
                Some(libc::EPERM | libc::EAGAIN) => ShmError::QuotaExceeded,
                _ => ShmError::Io(e),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PosixShm;
    use crate::shm::{SegmentName, ShmBackend, ShmError, backend_segment_name};

    fn name(tag: &str) -> SegmentName {
        match SegmentName::new(&format!("/drm.test.{tag}")) {
            Ok(n) => n,
            Err(e) => unreachable!("test name: {e:?}"),
        }
    }

    /// REQ: REQ-CORE-072, REQ-CORE-098 — open semantics: exclusive create wins
    /// once, open never creates (I2), exact size enforced after stale
    /// classification (I3), undersized object classified stale.
    #[test]
    fn open_semantics() {
        let shm = PosixShm;
        let n = name("open-sem");
        let _g = scopeguard_unlink(shm, n.clone());

        // I2: open_existing on a nonexistent name ⇒ NotFound, no creation.
        assert!(matches!(shm.open_existing(&n, 64), Err(ShmError::NotFound)));
        assert!(matches!(shm.open_raw(&n), Err(ShmError::NotFound)));

        let m = match shm.create_exclusive(&n, 64) {
            Ok(m) => m,
            Err(e) => unreachable!("create: {e:?}"),
        };
        assert_eq!(m.len(), 64);
        // I1: second exclusive create loses explicitly.
        assert!(matches!(
            shm.create_exclusive(&n, 64),
            Err(ShmError::AlreadyExists)
        ));
        drop(m);

        // I3: exact-size mismatch ⇒ SizeMismatch.
        assert!(matches!(
            shm.open_existing(&n, 63),
            Err(ShmError::SizeMismatch)
        ));
        let m = match shm.open_existing(&n, 64) {
            Ok(m) => m,
            Err(e) => unreachable!("open: {e:?}"),
        };
        assert_eq!(m.len(), 64);
        shm.unmap(m);

        // Stale classification: a below-prefix object is recoverable, not a
        // compatibility failure.
        let stale = name("stale-sem");
        let _g = scopeguard_unlink(shm, stale.clone());
        match shm.create_exclusive(&stale, 8) {
            Ok(m) => shm.unmap(m),
            Err(e) => unreachable!("create stale: {e:?}"),
        }
        assert!(matches!(
            shm.open_existing(&stale, 64),
            Err(ShmError::StaleIncomplete)
        ));
    }

    /// REQ: REQ-CORE-072, REQ-CORE-088 — generated-name conformance against the
    /// real `shm_open` for a maximum-length deployment id (I0).
    #[test]
    fn name_conformance() {
        let shm = PosixShm;
        // NAME_MAX 255: "/drm." (5) + deploy id + "." (1) + 16 hex chars.
        let deploy = "d".repeat(255 - 5 - 1 - 16);
        let n = match backend_segment_name(&deploy, 0x1234) {
            Ok(n) => n,
            Err(e) => unreachable!("max name: {e:?}"),
        };
        assert_eq!(n.as_str().len(), 255);
        let _g = scopeguard_unlink(shm, n.clone());
        match shm.create_exclusive(&n, 32) {
            Ok(m) => shm.unmap(m),
            Err(e) => unreachable!("create max: {e:?}"),
        }
        match shm.open_existing(&n, 32) {
            Ok(m) => shm.unmap(m),
            Err(e) => unreachable!("open max: {e:?}"),
        }
    }

    /// REQ: REQ-CORE-090 — precommit locks the full region or refuses (I9);
    /// mapped writes are visible through a second mapping of the same object.
    #[test]
    fn precommit_and_visibility() {
        let shm = PosixShm;
        let n = name("precommit");
        let _g = scopeguard_unlink(shm, n.clone());
        let m = match shm.create_exclusive(&n, 64) {
            Ok(m) => m,
            Err(e) => unreachable!("create: {e:?}"),
        };
        match shm.precommit_and_prove(&m) {
            Ok(()) | Err(ShmError::QuotaExceeded) => {}
            Err(e) => unreachable!("precommit: {e:?}"),
        }
        let m2 = match shm.open_existing(&n, 64) {
            Ok(m) => m,
            Err(e) => unreachable!("reopen: {e:?}"),
        };
        // SAFETY: both views map the same 64-byte object; byte 0 lies within it.
        unsafe {
            m.ptr().as_ptr().write(0xAB_u8);
            assert_eq!(m2.ptr().as_ptr().read(), 0xAB_u8);
        }
        shm.unmap(m2);
        shm.unmap(m);
    }

    /// REQ: REQ-CORE-070 — unlink removes the name; a second unlink is `NotFound`.
    #[test]
    fn unlink_semantics() {
        let shm = PosixShm;
        let n = name("unlink-sem");
        match shm.create_exclusive(&n, 16) {
            Ok(m) => shm.unmap(m),
            Err(e) => unreachable!("create: {e:?}"),
        }
        assert!(shm.unlink(&n).is_ok());
        assert!(matches!(shm.unlink(&n), Err(ShmError::NotFound)));
    }

    /// Test guard: unlink the segment name when the scope ends.
    fn scopeguard_unlink(shm: PosixShm, n: SegmentName) -> impl Drop {
        struct G(PosixShm, SegmentName);
        impl Drop for G {
            fn drop(&mut self) {
                let _ = ShmBackend::unlink(&self.0, &self.1);
            }
        }
        G(shm, n)
    }
}
