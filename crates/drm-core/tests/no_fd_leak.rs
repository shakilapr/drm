//! fd-leak guard over SHM create/open/unlink cycles (work-plan M02W1-E1-T2;
//! shm-backend.md stress matrix). 10 000 create/unmap cycles must leave the
//! process descriptor count stable. `#[ignore]`d: `cargo xtask test --stress`.
//! REQ: REQ-CORE-072, REQ-CORE-098.

#![cfg(unix)]

use drm_core::shm::{PosixShm, SegmentName, ShmBackend};

fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("no_fd_leak: {what}: {e:?}"),
    }
}

/// Descriptor count via `/proc/self/fd` (Linux CI runners).
fn fd_count() -> usize {
    match std::fs::read_dir("/proc/self/fd") {
        Ok(entries) => entries.count(),
        Err(_) => 0, // non-Linux unix (e.g. macOS): count unavailable, skip
    }
}

/// REQ: REQ-CORE-072 — 10k create/unmap cycles: fd count stable (±2).
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_no_fd_leak() {
    if !std::path::Path::new("/proc/self/fd").exists() {
        eprintln!("no_fd_leak: /proc/self/fd unavailable; skipping count check");
        return;
    }
    let shm = PosixShm;
    let base = fd_count();
    for i in 0..10_000_u32 {
        let name = ok(
            SegmentName::new(&format!("/drm.fdl.{i}.{}", u64::from(i))),
            "name",
        );
        let m = ok(shm.create_exclusive(&name, 64), "create");
        drop(m);
        let _ = shm.unlink(&name);
    }
    let after = fd_count();
    let diff = after.abs_diff(base);
    assert!(diff <= 2, "fd drift over 10k cycles: {base} -> {after}");
}

/// REQ: REQ-CORE-098 — 10k `open_existing` cycles on one live segment: fd stable.
#[test]
#[ignore = "stress suite: cargo xtask test --stress"]
fn stress_no_fd_leak_open() {
    if !std::path::Path::new("/proc/self/fd").exists() {
        eprintln!("no_fd_leak_open: /proc/self/fd unavailable; skipping");
        return;
    }
    let shm = PosixShm;
    let name = ok(SegmentName::new("/drm.fdl.open"), "name");
    let guard = ok(shm.create_exclusive(&name, 64), "guard create");
    drop(guard);
    let base = fd_count();
    for _ in 0..10_000_u32 {
        let m = ok(shm.open_existing(&name, 64), "open");
        drop(m);
    }
    let after = fd_count();
    let diff = after.abs_diff(base);
    assert!(diff <= 2, "fd drift over 10k opens: {base} -> {after}");
    let _ = shm.unlink(&name);
}
