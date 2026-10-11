//! Windows SHM backend bodies, verbatim from
//! `docs/algorithms/runtime/windows-backends.md` §Functions (`create_exclusive`,
//! `open_existing`, `precommit_and_prove`) and §Protocols (cleanup, process
//! identity, registration-lock reclamation). Pagefile-backed sections via
//! `CreateFileMappingW`/`MapViewOfFile`; advisory `LockFileEx` locks; identity is
//! diagnostic-only (the registration lock is reclamation authority).
//! REQ: REQ-CORE-074, REQ-CORE-075, REQ-CORE-076, REQ-CORE-077, REQ-CORE-078,
//! REQ-CORE-098.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{NonNull, null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_COMMITMENT_LIMIT,
    ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, ERROR_LOCK_VIOLATION, ERROR_NOT_ENOUGH_MEMORY,
    ERROR_NOT_ENOUGH_QUOTA, FILETIME, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, LOCK_FILE_FLAGS,
    LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, OPEN_ALWAYS,
};
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_BASIC_INFORMATION, MapViewOfFile,
    OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile, VirtualLock, VirtualQuery,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};

use super::{LockBackend, LockHandle, Mapping, ProcessIdentity, SegmentName, ShmBackend, ShmError};

/// Windows pagefile-backed [`ShmBackend`] implementation: fixed virtual layouts,
/// the OS owns reservation/commit/residency (no per-page commit tracking or
/// decommit — windows-backends.md §Platform divergence rules).
/// REQ: REQ-CORE-076, REQ-CORE-078.
#[derive(Debug, Clone, Copy, Default)]
pub struct WinShm;

/// Windows [`LockBackend`] over `LockFileEx`; the kernel releases the locked
/// range when the last handle closes, so a killed holder leaves no stale
/// authority (I-W3).
/// REQ: REQ-CORE-075.
#[derive(Debug, Clone, Copy, Default)]
pub struct WinLockBackend;

/// RAII close-on-drop for a raw Win32 handle; disarmed when ownership transfers
/// to a [`LockHandle`]. Used so every create/open failure path closes its handle
/// (I-W7).
struct HandleGuard(HANDLE);

impl HandleGuard {
    /// Relinquishes ownership without closing (the caller now owns the handle).
    fn into_handle(mut self) -> HANDLE {
        let handle = self.0;
        self.0 = null_mut();
        handle
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: `self.0` is an owned handle produced by `CreateFileMappingW`,
            // `OpenFileMappingW`, or `CreateFileW` and closed exactly once here.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// NUL-terminated UTF-16 form of a mapping name for the `...W` Win32 calls.
/// REQ: REQ-CORE-078.
fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Invalid-size constructor (`CreateFileMappingW` size split overflow).
/// REQ: REQ-CORE-078.
fn invalid_size() -> ShmError {
    ShmError::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        "section size overflow",
    ))
}

/// Splits a byte size into the high/low `u32` pair `CreateFileMappingW` takes.
/// REQ: REQ-CORE-078.
fn split_size(size: usize) -> Result<(u32, u32), ShmError> {
    let size = u64::try_from(size).map_err(|_| invalid_size())?;
    let low = u32::try_from(size & 0xFFFF_FFFF).map_err(|_| invalid_size())?;
    let high = u32::try_from(size >> 32).map_err(|_| invalid_size())?;
    Ok((high, low))
}

/// Maps `len` bytes of `handle` (`0` maps the whole section); the returned
/// [`Mapping`] owns the view and unmaps it on drop.
/// REQ: REQ-CORE-078.
fn map_view(handle: HANDLE, len: usize) -> Result<Mapping, ShmError> {
    // SAFETY: `handle` is a live section handle; `FILE_MAP_ALL_ACCESS` matches
    // `PAGE_READWRITE`; offset 0.
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len) };
    let ptr = NonNull::new(view.Value.cast::<u8>())
        .ok_or_else(|| ShmError::Io(io::Error::other("MapViewOfFile returned null")))?;
    Ok(Mapping::from_raw_parts(ptr, len))
}

/// Region size (bytes, allocation-granularity rounded) of the view at `base`, or
/// `None` when the query fails. Used only for the undersized (`StaleIncomplete`)
/// classification; exact-size/layout compatibility is validated at attach from
/// the descriptor (`channel-metadata.md`).
/// REQ: REQ-CORE-098.
fn view_region_size(base: *const core::ffi::c_void) -> Option<usize> {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: `info` is a valid out-parameter and `base` points into a live
    // mapping owned by the caller.
    let written = unsafe {
        VirtualQuery(
            base,
            &raw mut info,
            core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if written == 0 {
        None
    } else {
        Some(info.RegionSize)
    }
}

/// Maps a `GetLastError` code to the platform-agnostic [`ShmError`].
/// REQ: REQ-CORE-076, REQ-CORE-078.
fn err_from(code: u32) -> ShmError {
    match code {
        ERROR_FILE_NOT_FOUND => ShmError::NotFound,
        ERROR_ALREADY_EXISTS => ShmError::AlreadyExists,
        ERROR_ACCESS_DENIED => ShmError::PermissionDenied,
        ERROR_NOT_ENOUGH_MEMORY | ERROR_COMMITMENT_LIMIT | ERROR_NOT_ENOUGH_QUOTA => {
            ShmError::QuotaExceeded
        }
        other => {
            let code = i32::try_from(other).unwrap_or(i32::MAX);
            ShmError::Io(io::Error::from_raw_os_error(code))
        }
    }
}

/// Maps the current thread's last-error code.
/// REQ: REQ-CORE-078.
fn err_from_last() -> ShmError {
    // SAFETY: `GetLastError` has no preconditions.
    err_from(unsafe { GetLastError() })
}

impl ShmBackend for WinShm {
    fn create_exclusive(&self, name: &SegmentName, size: usize) -> Result<Mapping, ShmError> {
        let wname = wide(name.as_str());
        let (high, low) = split_size(size)?;
        // SAFETY: `wname` is NUL-terminated for the call; a null
        // `SECURITY_ATTRIBUTES` uses the default descriptor; `INVALID_HANDLE_VALUE`
        // selects a pagefile-backed section (windows-backends.md §Functions).
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                null(),
                PAGE_READWRITE,
                high,
                low,
                wname.as_ptr(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(err_from_last());
        }
        // `GetLastError` must be read immediately after `CreateFileMappingW`,
        // before any other Win32 call. SAFETY: no intervening Win32 call.
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let guard = HandleGuard(handle);
        if exists {
            // I-W1: parity with `O_EXCL` — the loser observes `AlreadyExists`.
            return Err(ShmError::AlreadyExists);
        }
        let mapping = map_view(handle, size)?;
        // Section handle closes here; the view keeps the section alive (I-W4b).
        drop(guard);
        Ok(mapping)
    }

    fn open_existing(&self, name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError> {
        let wname = wide(name.as_str());
        // SAFETY: `wname` is NUL-terminated for the call; `bInheritHandle = 0`.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, wname.as_ptr()) };
        if handle.is_null() {
            return Err(err_from_last()); // ERROR_FILE_NOT_FOUND ⇒ NotFound (I-W2)
        }
        let guard = HandleGuard(handle);
        // Whole-section probe classifies an undersized section (I parity with
        // shm-backend `StaleIncomplete`); larger/incompatible sections are
        // rejected at attach from the descriptor + layout hash.
        // SAFETY: `handle` is live; `0` maps the whole section; the view is
        // unmapped below.
        let probe = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, 0) };
        if probe.Value.is_null() {
            return Err(err_from_last());
        }
        let region = view_region_size(probe.Value);
        // SAFETY: `probe` is the live view just created and is unmapped once.
        unsafe { UnmapViewOfFile(probe) };
        if region.is_some_and(|r| r < expect_size) {
            return Err(ShmError::StaleIncomplete);
        }
        let mapping = map_view(handle, expect_size)?;
        drop(guard);
        Ok(mapping)
    }

    fn unmap(&self, m: Mapping) {
        drop(m);
    }

    fn unlink(&self, _name: &SegmentName) -> Result<(), ShmError> {
        // I-W4: pagefile sections have no unlink operation; they cease to exist
        // with their last handle. `drm clean` therefore deletes only the
        // deployment lock files (windows-backends.md §Protocols).
        Ok(())
    }

    fn precommit_and_prove(&self, m: &Mapping) -> Result<(), ShmError> {
        if m.is_empty() {
            return Ok(());
        }
        // SAFETY: `ptr`/`len` describe a live section view owned by `m`; the
        // section was created fully committed (`PAGE_READWRITE`), so
        // `VirtualLock`'s committed-pages precondition holds by construction.
        let ok = unsafe { VirtualLock(m.ptr().cast::<core::ffi::c_void>().as_ptr(), m.len()) };
        if ok == 0 {
            // Working-set limit ⇒ explicit refusal; activation never continues
            // pageable (I-W6).
            return Err(ShmError::QuotaExceeded);
        }
        Ok(())
    }
}

impl LockBackend for WinLockBackend {
    fn try_exclusive(&self, path: &Path) -> Result<LockHandle, ShmError> {
        lock_path(path, LOCKFILE_EXCLUSIVE_LOCK)
    }

    fn try_shared(&self, path: &Path) -> Result<LockHandle, ShmError> {
        lock_path(path, 0)
    }

    fn release(self, h: LockHandle) {
        drop(h);
    }
}

/// Opens (creating if needed) the lock file at `path` and acquires a
/// non-blocking `flags`-mode lock over byte range `[0, 1)`. Lock handles open
/// non-inheritable (`bInheritHandle = 0`): a child never shares or releases the
/// parent's authority (POSIX `O_CLOEXEC` parity).
/// REQ: REQ-CORE-075.
fn lock_path(path: &Path, flags: LOCK_FILE_FLAGS) -> Result<LockHandle, ShmError> {
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide_path` is NUL-terminated for the call; a null
    // `SECURITY_ATTRIBUTES` uses the default descriptor.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(err_from_last());
    }
    let guard = HandleGuard(handle);
    let mut overlapped = OVERLAPPED::default();
    // SAFETY: `guard.0` is a live lock-file handle and `overlapped` outlives the
    // call; a one-byte range at offset 0 is the fixed DRM lock range.
    let ok = unsafe {
        LockFileEx(
            guard.0,
            flags | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &raw mut overlapped,
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(if code == ERROR_LOCK_VIOLATION {
            // Another live holder owns the range ⇒ busy (`AlreadyExists` family).
            ShmError::AlreadyExists
        } else {
            err_from(code)
        });
    }
    Ok(LockHandle::from_windows_handle(guard.into_handle()))
}

/// Reads a process's creation time as 100 ns ticks since 1601-01-01, or `None`
/// when the handle cannot be queried.
/// REQ: REQ-CORE-074.
fn process_start_time(handle: HANDLE) -> Option<u64> {
    let mut create = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: `handle` has query access and all four out-params are valid for
    // the duration of the call.
    let ok = unsafe {
        GetProcessTimes(
            handle,
            &raw mut create,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if ok == 0 {
        return None;
    }
    Some((u64::from(create.dwHighDateTime) << 32) | u64::from(create.dwLowDateTime))
}

/// Opens `pid` with query access, or `None` when it cannot be opened.
/// REQ: REQ-CORE-074.
fn open_query(pid: i32) -> Option<HANDLE> {
    let pid = u32::try_from(pid).ok()?;
    // SAFETY: `OpenProcess` has no preconditions; a null return is handled.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() { None } else { Some(handle) }
}

/// This process's `(pid, creation time)` identity. Diagnostic only; never
/// consulted by reclamation (registration-lock authority).
/// REQ: REQ-CORE-074.
#[must_use]
pub fn current_identity() -> ProcessIdentity {
    // SAFETY: `GetCurrentProcessId`/`GetCurrentProcess` have no preconditions.
    let pid = i32::try_from(unsafe { GetCurrentProcessId() }).unwrap_or(0);
    let start_ticks = process_start_time(unsafe { GetCurrentProcess() }).unwrap_or(0);
    ProcessIdentity { pid, start_ticks }
}

/// Existence probe: true when `pid` is a live process. A query-access failure
/// other than "no such PID" proves nothing and reports alive.
/// REQ: REQ-CORE-074.
#[must_use]
pub fn is_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let Some(handle) = open_query(pid) else {
        // ERROR_INVALID_PARAMETER ⇒ no such PID ⇒ dead; anything else (access
        // denied) is not proof of death.
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    };
    // SAFETY: `handle` is a query handle owned here; a signaled process object
    // means the process has terminated.
    let signaled = unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0;
    // SAFETY: `handle` is owned by this function and closed exactly once.
    unsafe { CloseHandle(handle) };
    !signaled
}

/// Incarnation-safe liveness: true iff `pid` is alive AND its creation time
/// equals the recorded `start_ticks`. A reused PID fails the guard; a query
/// failure is treated as alive so a live owner is never mis-reclaimed (I-E5
/// parity). Diagnostic only.
/// REQ: REQ-CORE-074, REQ-CORE-008, REQ-CORE-057.
#[must_use]
pub fn identity_alive(pid: i32, start_ticks: u64) -> bool {
    if pid <= 0 {
        return false;
    }
    let Some(handle) = open_query(pid) else {
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    };
    let alive = process_start_time(handle).is_none_or(|t| t == start_ticks)
        // SAFETY: `handle` is a query handle owned here.
        && unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0;
    // SAFETY: `handle` is owned by this function and closed exactly once.
    unsafe { CloseHandle(handle) };
    alive
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{WinLockBackend, WinShm, current_identity, identity_alive, is_alive};
    use crate::shm::{
        LockBackend, LockPaths, SegmentName, ShmBackend, ShmError, windows_section_name,
    };

    /// Test helper: unwrap-or-abort without `unwrap`/`expect` (workspace deny).
    fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
        match r {
            Ok(v) => v,
            Err(e) => unreachable!("{what}: {e:?}"),
        }
    }

    /// Unique per-test section name (FNV-1a over the tag so distinct tags never
    /// collide across parallel tests).
    fn name(tag: &str) -> SegmentName {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in tag.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        ok(
            SegmentName::new(&windows_section_name("win-test", hash)),
            "name",
        )
    }

    /// REQ: REQ-CORE-078 — section create/map round-trip: exactly one creator
    /// (I-W1), a second exclusive create loses, and writes are visible through a
    /// second mapping (Windows smoke, E1-T1).
    #[test]
    fn win_section_roundtrip() {
        let shm = WinShm;
        let n = name("roundtrip");
        let m = ok(shm.create_exclusive(&n, 64), "create");
        assert!(matches!(
            shm.create_exclusive(&n, 64),
            Err(ShmError::AlreadyExists)
        ));
        let m2 = ok(shm.open_existing(&n, 64), "open");
        // SAFETY: both views map the same 64-byte section; byte 0 lies within it.
        unsafe {
            m.ptr().as_ptr().write(0xA5);
            assert_eq!(m2.ptr().as_ptr().read(), 0xA5);
        }
        shm.unmap(m2);
        shm.unmap(m);
    }

    /// REQ: REQ-CORE-098 — open semantics: a miss never creates (I-W2).
    #[test]
    fn win_open_semantics() {
        let shm = WinShm;
        let n = name("missing");
        assert!(matches!(shm.open_existing(&n, 64), Err(ShmError::NotFound)));
    }

    /// REQ: REQ-CORE-077 — precommit locks the region or refuses (I-W6).
    #[test]
    fn win_precommit_roundtrip() {
        let shm = WinShm;
        let n = name("precommit");
        let m = ok(shm.create_exclusive(&n, 64), "create");
        match shm.precommit_and_prove(&m) {
            Ok(()) | Err(ShmError::QuotaExceeded) => {}
            Err(e) => unreachable!("precommit: {e:?}"),
        }
        shm.unmap(m);
    }

    /// REQ: REQ-CORE-075 — `LockFileEx` exclusive vs shared across handles in one
    /// process; release is explicit (I-W8 active-hold primitive).
    #[test]
    fn win_lock_roundtrip() {
        let locks = WinLockBackend;
        let dir = std::env::temp_dir().join(format!("drm-win-lock-{}", std::process::id()));
        ok(std::fs::create_dir_all(&dir), "lock dir");
        let paths = LockPaths::new(&dir);
        let excl = ok(locks.try_exclusive(&paths.active()), "exclusive");
        assert!(matches!(
            locks.try_exclusive(&paths.active()),
            Err(ShmError::AlreadyExists)
        ));
        locks.release(excl);
        let shared = ok(locks.try_shared(&paths.active()), "shared");
        locks.release(shared);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REQ: REQ-CORE-074 — identity is stable across reads and the live self is
    /// reported alive; a mismatched creation time is a different incarnation.
    #[test]
    fn win_process_identity() {
        let me = current_identity();
        assert!(me.pid > 0);
        assert!(is_alive(me.pid));
        assert!(identity_alive(me.pid, me.start_ticks));
        assert!(!identity_alive(me.pid, me.start_ticks.wrapping_add(1)));
        assert!(!is_alive(0));
    }

    /// Unique per-test lock directory (temp dir + process id + tag).
    fn lock_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("drm-win-{tag}-{}", std::process::id()))
    }

    /// Spins (sleeping) until `path` exists or a bounded deadline elapses.
    fn wait_for(path: &Path, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// REQ: REQ-CORE-078 — I-W7: a conflicting create closes its own handle and
    /// leaves the winner mapping intact.
    #[test]
    fn win_create_failure_matrix() {
        let shm = WinShm;
        let n = name("create-matrix");
        let m = ok(shm.create_exclusive(&n, 64), "create");
        assert!(matches!(
            shm.create_exclusive(&n, 64),
            Err(ShmError::AlreadyExists)
        ));
        let m2 = ok(shm.open_existing(&n, 64), "winner intact");
        shm.unmap(m2);
        shm.unmap(m);
    }

    /// REQ: REQ-CORE-078, REQ-CORE-098 — I-W4b: a pagefile-backed section dies
    /// with its last mapping, so a restart recreates fresh (there is no unlink).
    #[test]
    fn win_close_last_handle() {
        let shm = WinShm;
        let n = name("close-last");
        let m = ok(shm.create_exclusive(&n, 64), "create");
        let m2 = ok(shm.open_existing(&n, 64), "reopen");
        shm.unmap(m2);
        assert!(
            shm.open_existing(&n, 64).is_ok(),
            "one mapping keeps the section alive"
        );
        shm.unmap(m);
        assert!(
            matches!(shm.open_existing(&n, 64), Err(ShmError::NotFound)),
            "last handle closed ⇒ section gone"
        );
    }

    /// REQ: REQ-CORE-075 — I-W8: every running node holds one shared active
    /// lock; the `drm clean` exclusive probe is refused while any shared hold is
    /// live and succeeds once all shared holds release.
    #[test]
    fn win_active_hold_lifecycle() {
        let _serial = crate::test_guard::acquire();
        let locks = WinLockBackend;
        let dir = lock_dir("active-hold");
        ok(std::fs::create_dir_all(&dir), "dir");
        let active = LockPaths::new(&dir).active();
        let node_a = ok(locks.try_shared(&active), "node a shared");
        let node_b = ok(locks.try_shared(&active), "node b shared");
        assert!(matches!(
            locks.try_exclusive(&active),
            Err(ShmError::AlreadyExists)
        ));
        locks.release(node_b);
        assert!(matches!(
            locks.try_exclusive(&active),
            Err(ShmError::AlreadyExists)
        ));
        locks.release(node_a);
        let clean = ok(locks.try_exclusive(&active), "clean exclusive");
        locks.release(clean);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REQ: REQ-CORE-075 — I-W3: a `LockFileEx` range held by a test child is
    /// released by the kernel when the child is killed, so the next acquirer
    /// succeeds (no stale authority survives death).
    #[test]
    fn win_lock_release_on_death() {
        let _serial = crate::test_guard::acquire();
        let locks = WinLockBackend;
        let dir = lock_dir("kill");
        ok(std::fs::create_dir_all(&dir), "lock dir");
        let lock_path = LockPaths::new(&dir).active();
        let ready = dir.join("ready");
        let exe = ok(std::env::current_exe(), "current_exe");
        let mut child = ok(
            Command::new(exe)
                .args([
                    "--exact",
                    "shm::windows::tests::win_lock_holder_child",
                    "--nocapture",
                ])
                .env("DRM_TEST_LOCK_PATH", &lock_path)
                .env("DRM_TEST_LOCK_READY", &ready)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
            "spawn holder",
        );
        wait_for(&ready, "holder acquired");
        assert!(
            matches!(
                locks.try_exclusive(&lock_path),
                Err(ShmError::AlreadyExists)
            ),
            "live holder keeps the range"
        );
        ok(child.kill(), "kill holder");
        ok(child.wait(), "reap holder");
        // The OS may release the range with a short lag; poll, never force.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match locks.try_exclusive(&lock_path) {
                Ok(hold) => {
                    locks.release(hold);
                    break;
                }
                Err(ShmError::AlreadyExists) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => unreachable!("post-death acquire: {e:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Child driver for `win_lock_release_on_death`: acquire the lock named by
    /// `DRM_TEST_LOCK_PATH`, signal `DRM_TEST_LOCK_READY`, then block until
    /// killed. A normal suite run (no env) returns immediately.
    /// REQ: REQ-CORE-075.
    #[test]
    fn win_lock_holder_child() {
        let (Ok(path), Ok(ready)) = (
            std::env::var("DRM_TEST_LOCK_PATH"),
            std::env::var("DRM_TEST_LOCK_READY"),
        ) else {
            return;
        };
        let hold = ok(
            WinLockBackend.try_exclusive(Path::new(&path)),
            "child exclusive",
        );
        // Leak the handle: the child holds the range until the kernel kills it.
        std::mem::forget(hold);
        let _ = std::fs::write(ready, b"ready");
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    /// REQ: REQ-CORE-074 — I-W5: incarnation-safe diagnostic identity. A
    /// recorded child identity is dead once the child exits (a reused PID also
    /// mismatches its creation time); a perturbed creation time on a live PID is
    /// a different incarnation.
    #[test]
    fn win_pid_reuse() {
        let _serial = crate::test_guard::acquire();
        let dir = lock_dir("reuse");
        ok(std::fs::create_dir_all(&dir), "dir");
        let out = dir.join("id");
        let exe = ok(std::env::current_exe(), "current_exe");
        let mut child = ok(
            Command::new(exe)
                .args([
                    "--exact",
                    "shm::windows::tests::win_identity_child",
                    "--nocapture",
                ])
                .env("DRM_TEST_IDENTITY_OUT", &out)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
            "spawn identity child",
        );
        ok(child.wait(), "reap identity child");
        let text = ok(std::fs::read_to_string(&out), "identity file");
        let mut parts = text.split_ascii_whitespace();
        let pid = parts
            .next()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or_else(|| unreachable!("child pid in `{text}`"));
        let start = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| unreachable!("child start in `{text}`"));
        assert!(!identity_alive(pid, start), "exited incarnation is dead");
        let me = current_identity();
        assert!(identity_alive(me.pid, me.start_ticks));
        assert!(!identity_alive(me.pid, me.start_ticks.wrapping_add(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Child driver for `win_pid_reuse`: write `pid start_ticks` to
    /// `DRM_TEST_IDENTITY_OUT`, then exit. A normal suite run (no env) returns.
    /// REQ: REQ-CORE-074.
    #[test]
    fn win_identity_child() {
        let Ok(out) = std::env::var("DRM_TEST_IDENTITY_OUT") else {
            return;
        };
        let me = current_identity();
        let _ = std::fs::write(out, format!("{} {}", me.pid, me.start_ticks));
    }
}
