//! Shared child-mode helpers for the multiprocess platform scenarios.
//! REQ: REQ-CORE-072.

#![cfg(unix)]

use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

/// True when this process was spawned as a platform-suite child.
#[must_use]
pub fn is_child() -> bool {
    std::env::var_os("DRM_PLATFORM_CHILD").is_some()
}

/// Test-suite oracle: the precondition must hold; any Err here is a suite bug,
/// never an acceptable value. REQ: REQ-CORE-072.
pub fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("platform suite: {what}: {e:?}"),
    }
}

/// Spawns this test binary in child mode running `$test` exactly once.
/// REQ: REQ-CORE-072.
pub fn spawn_child(test: &str, name_env: &str, name: &str) -> Output {
    let exe = ok(std::env::current_exe(), "current_exe");
    ok(
        std::process::Command::new(exe)
            .arg("--exact")
            .arg(test)
            .arg("--test-threads=1")
            .arg("--nocapture")
            .env("DRM_PLATFORM_CHILD", "1")
            .env(name_env, name)
            .output(),
        "spawn child",
    )
}

/// Fresh POSIX-shm-legal name: leading slash, no further slashes, unique per
/// pid + clock. REQ: REQ-CORE-088.
#[must_use]
pub fn unique_name(prefix: &str) -> String {
    let nanos = ok(SystemTime::now().duration_since(UNIX_EPOCH), "clock").as_nanos();
    format!("/drm.plat.{prefix}.{}_{:x}", std::process::id(), nanos)
}

/// Fresh unique nanos tag (unique file/dir suffixes).
/// REQ: REQ-CORE-069.
#[must_use]
pub fn tag() -> u128 {
    ok(SystemTime::now().duration_since(UNIX_EPOCH), "clock").as_nanos()
}

/// Reads a required child-mode environment variable.
/// REQ: REQ-CORE-072.
#[must_use]
pub fn env_var(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) => v,
        Err(e) => unreachable!("platform suite: env {name}: {e:?}"),
    }
}
