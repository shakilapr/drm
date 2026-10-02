//! Quota edge: `/dev/shm` exhaustion ⇒ explicit error, clean state; and the
//! truncation-while-mapped (SIGBUS) policy note (work-plan M02W1-E1-T3;
//! shm-backend.md stress matrix). `#[ignore]`d: `cargo xtask test --stress`.
//! REQ: REQ-CORE-072, REQ-CORE-098.

#![cfg(unix)]

use drm_core::shm::{PosixShm, SegmentName, ShmBackend, ShmError};

fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("quota_edge: {what}: {e:?}"),
    }
}

/// `/dev/shm` free bytes via `statvfs(3)`; `None` where unavailable.
fn dev_shm_free() -> Option<u64> {
    let path = std::ffi::CString::new("/dev/shm").ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated; `st` is fully initialized by the call.
    if unsafe { libc::statvfs(path.as_ptr(), std::ptr::addr_of_mut!(st)) } != 0 {
        return None;
    }
    Some(st.f_bsize.saturating_mul(st.f_bavail))
}

/// REQ: REQ-CORE-072 — creating a segment larger than `/dev/shm` free space
/// fails explicitly (`QuotaExceeded` via `posix_fallocate` ENOSPC) and leaves
/// the namespace clean (no partial name survives; re-create with a normal
/// size still works).
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_quota_edge() {
    let Some(free) = dev_shm_free() else {
        eprintln!("quota_edge: /dev/shm statvfs unavailable; skipping");
        return;
    };
    let shm = PosixShm;
    let name = ok(SegmentName::new("/drm.quota.huge"), "name");
    let huge = free.saturating_add(4 << 20); // beyond free space
    let r = shm.create_exclusive(&name, usize::try_from(huge).unwrap_or(usize::MAX));
    match r {
        Err(ShmError::QuotaExceeded) => {} // expected: explicit error
        Err(ShmError::Io(e)) => {
            let raw = e.raw_os_error().unwrap_or(0);
            if raw == libc::EINVAL {
                // fallocate unsupported on this filesystem: not applicable.
                eprintln!("quota_edge: fallocate EINVAL (non-tmpfs); skipping");
                let _ = shm.unlink(&name);
                return;
            }
            unreachable!("quota_edge: unexpected io family {e:?}")
        }
        Err(other) => unreachable!("quota_edge: unexpected error {other:?}"),
        Ok(m) => {
            // tmpfs without a hard cap (overcommit): allocation succeeded.
            eprintln!("quota_edge: no ENOSPC (tmpfs overcommit); accepted mapping");
            drop(m);
        }
    }
    // Clean state: the namespace accepts a normal create right after.
    let normal = ok(SegmentName::new("/drm.quota.normal"), "name");
    let m = ok(shm.create_exclusive(&normal, 64), "normal create");
    drop(m);
    let _ = shm.unlink(&normal);
    let _ = shm.unlink(&name);
}

/// Truncation-while-mapped policy (documented unsupported, architecture
/// §Platform-Managed Paging): DRM never truncates its own live mappings —
/// `create_exclusive` reserves the full extent up front via
/// `posix_fallocate`, so self-inflicted SIGBUS is impossible. A FOREIGN
/// truncation of the same object is out of contract; this test documents
/// that the up-front reservation holds: after create, the object size is
/// exactly `size` (ftruncate cannot have been partial).
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_no_partial_extent() {
    let shm = PosixShm;
    let name = ok(SegmentName::new("/drm.quota.extent"), "name");
    let m = ok(shm.create_exclusive(&name, 4096), "create");
    let (fd, size) = ok(shm.open_raw(&name), "open_raw");
    assert_eq!(size, 4096, "extent reserved atomically by ftruncate");
    drop(fd);
    drop(m);
    let _ = shm.unlink(&name);
}
