//! `cleanup` — shm-backend.md §Protocols cleanup probe + I8 `ActiveHold`:
//! a held shared deployment lock blocks the exclusive probe (`ActiveUntouched`);
//! without holders the exclusive probe succeeds and the namespace is unlinked
//! (`Cleaned`).
//! REQ: REQ-CORE-069, REQ-CORE-070.
#![cfg(unix)]

use std::fs;

use drm_core::shm::{
    FlockBackend, LockBackend, LockPaths, PosixShm, SegmentName, ShmBackend, ShmError,
};

use crate::child_util::{ok, spawn_guard, tag};

/// REQ: REQ-CORE-069, REQ-CORE-070 — exclusive probe wins on a free namespace;
/// a held shared hold ⇒ probe busy; segments unlinked only after the probe wins.
#[test]
fn cleanup() {
    // Serialize against the forking tests: a concurrent `Command::spawn` child
    // would inherit this test's lock descriptors and keep a released lock alive
    // until `exec`, making the re-probe read busy.
    let _serial = spawn_guard();
    let shm = PosixShm;
    let flock = FlockBackend;

    let tag = tag();
    let lock_dir =
        std::env::temp_dir().join(format!("drm-cleanup-test-{}-{tag:x}", std::process::id()));
    ok(fs::create_dir_all(&lock_dir), "lock dir");
    let paths = LockPaths::new(&lock_dir);

    let n1 = ok(
        SegmentName::new(&format!("/drm.plat.clean1.{tag}")),
        "name1",
    );
    let n2 = ok(
        SegmentName::new(&format!("/drm.plat.clean2.{tag}")),
        "name2",
    );
    let m1 = ok(shm.create_exclusive(&n1, 64), "create1");
    let m2 = ok(shm.create_exclusive(&n2, 64), "create2");

    // Probe without any holder: exclusive active lock succeeds.
    let excl = ok(flock.try_exclusive(&paths.active()), "exclusive free");

    // Release, take shared (a node's active hold), then the exclusive probe
    // (`drm clean`) must be refused.
    flock.release(excl);
    let shared = ok(flock.try_shared(&paths.active()), "shared hold");
    assert!(
        matches!(
            flock.try_exclusive(&paths.active()),
            Err(ShmError::AlreadyExists)
        ),
        "exclusive probe must be busy while nodes hold shared"
    );

    // Shared hold present ⇒ nothing is unlinked (ActiveHold, I8).
    assert!(
        shm.open_existing(&n1, 64).is_ok(),
        "segment intact during hold"
    );

    // Holder releases; `drm clean` re-probes and wins, then unlinks.
    flock.release(shared);
    let clean = ok(
        flock.try_exclusive(&paths.active()),
        "exclusive after release",
    );
    ok(shm.unlink(&n1), "unlink1");
    ok(shm.unlink(&n2), "unlink2");
    flock.release(clean);

    assert!(matches!(
        shm.open_existing(&n1, 64),
        Err(ShmError::NotFound)
    ));
    assert!(matches!(
        shm.open_existing(&n2, 64),
        Err(ShmError::NotFound)
    ));
    shm.unmap(m1);
    shm.unmap(m2);
    ok(fs::remove_dir_all(&lock_dir), "rm dir");
}
