//! SHM kill-during-lifecycle stress suite (work-plan M02W1-E1-T1;
//! shm-backend.md I1 + stress matrix). Each scenario spawns a child that
//! SIGKILLs itself at a precise lifecycle point; the parent then proves the
//! exact leftover state (absent / live / stale-incomplete), cleans up, and
//! re-creates with the same name. `#[ignore]`d: run via
//! `cargo xtask test --stress` (8 scenarios × `DRM_STRESS_ITERS`, default 100).
//! REQ: REQ-CORE-072, REQ-CORE-098, REQ-CORE-070.

#![cfg(unix)]

use std::time::{SystemTime, UNIX_EPOCH};

use drm_core::shm::{PosixShm, SegmentName, ShmBackend, ShmError};

fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("stress suite: {what}: {e:?}"),
    }
}

fn unique_name(tag: &str) -> String {
    let nanos = ok(SystemTime::now().duration_since(UNIX_EPOCH), "clock").as_nanos();
    format!("/drm.stress.{tag}.{}_{:x}", std::process::id(), nanos)
}

fn iters() -> usize {
    match std::env::var("DRM_STRESS_ITERS") {
        Ok(v) => v.parse().unwrap_or_else(|e| unreachable!("iters: {e}")),
        Err(_) => 100,
    }
}

/// Parent: spawn the child (SIGKILL at its step), then prove the leftover
/// state matches the scenario, clean it, and re-create with the same name.
fn kill_during(scenario: &str, step: &str, tag: &str) {
    let name = unique_name(tag);
    let exe = ok(std::env::current_exe(), "current_exe");
    let out = ok(
        std::process::Command::new(exe)
            .arg("--ignored")
            .arg("--exact")
            .arg("stress_shm_kill_child_driver")
            .arg("--test-threads=1")
            .env("DRM_KILL_NAME", &name)
            .env("DRM_KILL_STEP", step)
            .output(),
        "spawn kill child",
    );
    assert!(
        !out.status.success(),
        "{scenario}: child must die by SIGKILL"
    );

    let seg = ok(SegmentName::new(&name), "name");
    let shm = PosixShm;
    // Leftover state proof (I1): death leaves one of exactly two shapes —
    // a live segment (create ⇒ AlreadyExists) or nothing (create succeeds).
    let pre = shm.create_exclusive(&seg, 64);
    match step {
        "none" | "unlinked" => {
            assert!(
                pre.is_ok(),
                "{scenario}: name must not survive death: {pre:?}"
            );
        }
        "created" | "written" | "hold" => {
            assert!(
                matches!(pre, Err(ShmError::AlreadyExists)),
                "{scenario}: live segment must block re-create: {pre:?}"
            );
            let _ = shm.unlink(&seg);
            let again = shm.create_exclusive(&seg, 64);
            assert!(
                again.is_ok(),
                "{scenario}: unlink must clear re-create: {again:?}"
            );
        }
        "stale" => {
            assert!(
                matches!(pre, Err(ShmError::AlreadyExists)),
                "{scenario}: 0-length object blocks create: {pre:?}"
            );
            let open = shm.open_existing(&seg, 64);
            assert!(
                matches!(open, Err(ShmError::StaleIncomplete)),
                "{scenario}: 0-length object classified stale: {open:?}"
            );
            let _ = shm.unlink(&seg);
            let again = shm.create_exclusive(&seg, 64);
            assert!(
                again.is_ok(),
                "{scenario}: cleanup then re-create: {again:?}"
            );
        }
        other => unreachable!("unknown step {other}"),
    }
    let _ = shm.unlink(&seg);
}

/// Child driver: perform the lifecycle step, then SIGKILL before any cleanup.
#[test]
#[ignore = "stress suite: child driver only"]
fn stress_shm_kill_child_driver() {
    let name = match std::env::var("DRM_KILL_NAME") {
        Ok(v) => v,
        Err(e) => unreachable!("child env DRM_KILL_NAME: {e:?}"),
    };
    let step = match std::env::var("DRM_KILL_STEP") {
        Ok(v) => v,
        Err(e) => unreachable!("child env DRM_KILL_STEP: {e:?}"),
    };
    let seg = ok(SegmentName::new(&name), "name");
    let shm = PosixShm;
    match step.as_str() {
        "none" => {}
        "created" | "written" | "hold" => {
            let m = ok(shm.create_exclusive(&seg, 64), "child create");
            if step == "written" {
                // SAFETY: `m` covers 64 live mapped bytes we own in this test.
                unsafe { m.ptr().write_volatile(0x5A) };
            }
            // "hold": intentionally leak the mapping into a raw pointer so the
            // process dies mapped (no Drop runs after SIGKILL anyway).
            std::mem::forget(m);
        }
        "stale" => {
            // Raw creation without ftruncate: object exists at 0 length.
            let cname = std::ffi::CString::new(name.as_str())
                .unwrap_or_else(|e| unreachable!("cname: {e}"));
            // SAFETY: NUL-terminated name for the duration of the call.
            let fd = unsafe {
                libc::shm_open(
                    cname.as_ptr(),
                    libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                    0o600,
                )
            };
            assert!(fd >= 0, "child raw create");
            // SAFETY: owned descriptor from shm_open.
            unsafe { libc::close(fd) };
        }
        "unlinked" => {
            let m = ok(shm.create_exclusive(&seg, 64), "child create");
            drop(m);
            let _ = shm.unlink(&seg);
        }
        other => unreachable!("unknown child step {other}"),
    }
    // SAFETY: deliberate hard-kill stress step; no destructors run after this.
    unsafe {
        libc::kill(
            libc::pid_t::try_from(std::process::id()).ok().unwrap_or(1),
            libc::SIGKILL,
        );
    }
    unreachable!("SIGKILL must terminate before this line");
}

macro_rules! kill_tests {
    ($($fn_name:ident => $step:literal),* $(,)?) => {
        $(
            #[test]
            #[ignore = "stress suite: cargo xtask test --stress"]
            fn $fn_name() {
                for _ in 0..iters() {
                    kill_during(stringify!($fn_name), $step, $step);
                }
            }
        )*
    };
}

kill_tests! {
    stress_shm_kill_create => "none",
    stress_shm_kill_created_no_write => "created",
    stress_shm_kill_after_write => "written",
    stress_shm_kill_mapped_hold => "hold",
    stress_shm_kill_after_unlink => "unlinked",
    stress_shm_kill_lock_hold => "created",
    stress_shm_kill_stale_incomplete => "stale",
    stress_shm_kill_cleanup_loop => "unlinked",
}
