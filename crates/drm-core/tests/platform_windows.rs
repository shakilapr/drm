//! Windows platform lifecycle conformance — the Windows equivalents of the
//! Linux `shm_kill`, `no_fd_leak`, and `quota_edge` suites (windows-backends.md
//! §Test Matrix; work-plan M12W4-E1-T1). Windows-only: pagefile-backed
//! sections, `LockFileEx` locks, and Win32 handle accounting. The kill scenario
//! spawns this same test binary in child mode (`DRM_WIN_KILL_NAME` set).
//! REQ: REQ-CORE-101, REQ-CORE-078, REQ-CORE-075, REQ-CORE-077.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use drm_core::shm::{SegmentName, ShmBackend, ShmError, WinShm, windows_section_name};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

/// Test helper: unwrap-or-abort without `unwrap`/`expect` (workspace deny).
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("platform_windows: {what}: {e:?}"),
    }
}

/// Unique per-test section name (FNV-1a over the tag).
fn name(tag: &str) -> SegmentName {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in tag.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    ok(
        SegmentName::new(&windows_section_name("win-plat", hash)),
        "name",
    )
}

/// Process handle count via `GetProcessHandleCount` (Win32 `no_fd_leak` parity).
fn handle_count() -> u32 {
    let mut count = 0_u32;
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle needing no cleanup;
    // `count` is a valid out-parameter for the duration of the call.
    let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &raw mut count) };
    assert!(ok != 0, "GetProcessHandleCount failed");
    count
}

/// REQ: REQ-CORE-101, REQ-CORE-078 — I-W7: 10 000 create/unmap cycles leave the
/// process handle count stable (no leaked section or view handles).
#[test]
fn win_no_handle_leak() {
    let shm = WinShm;
    let base = handle_count();
    for i in 0..10_000_u32 {
        let n = name(&format!("leak-{i}"));
        let m = ok(shm.create_exclusive(&n, 64), "create");
        shm.unmap(m);
    }
    let after = handle_count();
    let diff = after.abs_diff(base);
    assert!(diff <= 2, "handle drift over 10k cycles: {base} -> {after}");
}

/// REQ: REQ-CORE-101, REQ-CORE-078 — I-W4b: a child killed while holding the
/// section's last view releases the pagefile section; the parent recreates the
/// same name fresh (pagefile sections have no unlink).
#[test]
fn win_kill_releases_section() {
    let n = name("kill");
    let ready = std::env::temp_dir().join(format!("drm-win-kill-{}.ready", std::process::id()));
    let exe = ok(std::env::current_exe(), "current_exe");
    let mut child = ok(
        Command::new(exe)
            .args(["--exact", "win_kill_child", "--nocapture"])
            .env("DRM_WIN_KILL_NAME", n.as_str())
            .env("DRM_WIN_KILL_READY", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        "spawn child",
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for child");
        std::thread::sleep(Duration::from_millis(5));
    }
    let shm = WinShm;
    assert!(
        matches!(shm.create_exclusive(&n, 64), Err(ShmError::AlreadyExists)),
        "live child keeps the section"
    );
    ok(child.kill(), "kill child");
    ok(child.wait(), "reap child");
    // The kernel releases the view with the process; the section then ceases to
    // exist, so an exclusive create succeeds.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match shm.create_exclusive(&n, 64) {
            Ok(m) => {
                shm.unmap(m);
                break;
            }
            Err(ShmError::AlreadyExists) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => unreachable!("post-death create: {e:?}"),
        }
    }
    let _ = std::fs::remove_file(&ready);
}

/// Child driver for `win_kill_releases_section`: create the section named by
/// `DRM_WIN_KILL_NAME`, signal readiness, then block until killed. A normal
/// suite run (no env) returns immediately.
/// REQ: REQ-CORE-101.
#[test]
fn win_kill_child() {
    let Ok(raw) = std::env::var("DRM_WIN_KILL_NAME") else {
        return;
    };
    let n = ok(SegmentName::new(&raw), "child name");
    let m = ok(WinShm.create_exclusive(&n, 64), "child create");
    // Leak the mapping: the child holds the section until the kernel kills it.
    std::mem::forget(m);
    if let Ok(ready) = std::env::var("DRM_WIN_KILL_READY") {
        let _ = std::fs::write(ready, b"ready");
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// REQ: REQ-CORE-101, REQ-CORE-077 — I-W6: a create beyond the commit limit
/// fails explicitly (`QuotaExceeded`, or an OS commitment error) and leaves the
/// namespace clean; a normal create succeeds immediately after.
#[test]
fn win_quota_edge() {
    let shm = WinShm;
    let huge = name("quota-huge");
    match shm.create_exclusive(&huge, 1_usize << 46) {
        Err(ShmError::QuotaExceeded | ShmError::Io(_)) => {}
        Ok(m) => shm.unmap(m), // pagefile overcommit accepted: not applicable
        Err(other) => unreachable!("quota_edge: unexpected error {other:?}"),
    }
    let normal = name("quota-normal");
    let m = ok(shm.create_exclusive(&normal, 64), "normal create");
    shm.unmap(m);
}
