//! Deployment identity guard, namespace isolation, and guarded cleanup
//! (work-plan M05W4-E1-T1/T2/T3; `deployment-identity.md` I-DP1/I-DP2/I-DP5).
//! REQ: REQ-CORE-092, REQ-CORE-025, REQ-CORE-088, REQ-CORE-089, REQ-CORE-080,
//! REQ-CORE-069, REQ-CORE-070.
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

// `clean` is aliased: the test function below shares the name.
use drm_core::deploy::{
    ContractFingerprint, DeploymentId, clean as namespace_clean, guard_fingerprint, node_startup,
};
use drm_core::shm::{
    CleanReport, FlockBackend, LockBackend, LockPaths, PosixShm, ShmBackend, ShmError,
};
use drm_types::Status;

/// Test-suite oracle: the precondition must hold; any `Err` here is a suite bug.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("deployment identity suite: {what}: {e:?}"),
    }
}

/// Fresh unique lock directory under the system temp dir.
fn lock_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("drm-identity-{tag}-{pid}-{n}"))
}

/// I-DP1 + I-DP2 (REQ: REQ-CORE-092, REQ-CORE-025, REQ-CORE-088, REQ-CORE-089):
/// a mismatched compiled fingerprint is refused before any SHM touch, and two
/// deployment namespaces cannot cross-open.
#[test]
fn identity() {
    let shm = PosixShm;
    let locks = FlockBackend;

    // --- I-DP1: wrong identity refuses before SHM touch ---
    let dir = lock_dir("guard");
    ok(fs::create_dir_all(&dir), "guard lock dir");
    let paths = LockPaths::new(&dir);

    let dep_a = DeploymentId::new(0x1111_2222_3333_4444_5555_6666_7777_8888);
    let slot = 1;
    let name_a = ok(dep_a.segment_name(slot), "name a");

    let compiled = ContractFingerprint([0xAA; 16]);
    let wrong = Some(ContractFingerprint([0xBB; 16]));
    // Pure guard: mismatch ⇒ WrongAbi, unknown node ⇒ NotFound, match ⇒ Ok.
    assert_eq!(guard_fingerprint(compiled, wrong), Err(Status::WrongAbi));
    assert_eq!(guard_fingerprint(compiled, None), Err(Status::NotFound));
    assert_eq!(guard_fingerprint(compiled, Some(compiled)), Ok(()));

    // node_startup runs the guard FIRST: a refusal leaves the namespace
    // untouched (no segment created) and the active lock unacquired.
    match node_startup(&locks, &paths, compiled, wrong) {
        Err(Status::WrongAbi) => {}
        other => unreachable!("guard must refuse before SHM touch: {other:?}"),
    }
    assert!(
        matches!(shm.open_existing(&name_a, 64), Err(ShmError::NotFound)),
        "refused startup must not create a segment"
    );
    let free = ok(
        locks.try_exclusive(&paths.active()),
        "active lock free after refusal",
    );
    locks.release(free);

    // Matching fingerprint acquires the shared active hold for the node.
    let hold = ok(
        node_startup(&locks, &paths, compiled, Some(compiled)),
        "startup with matching fingerprint",
    );

    // --- I-DP2: two deployments, same topic slot, cannot cross-open ---
    let dep_b = DeploymentId::new(0x9999_AAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0000);
    let name_b = ok(dep_b.segment_name(slot), "name b");
    assert_ne!(
        name_a.as_str(),
        name_b.as_str(),
        "distinct deployments must derive distinct namespaces"
    );

    let m = ok(shm.create_exclusive(&name_a, 64), "create under A");
    // Cross-open under B's name fails NotFound — never foreign memory.
    assert!(
        matches!(shm.open_existing(&name_b, 64), Err(ShmError::NotFound)),
        "cross-deployment open must not attach"
    );
    // Own-namespace open succeeds.
    let own = ok(shm.open_existing(&name_a, 64), "open under A");
    shm.unmap(own);

    shm.unmap(m);
    ok(shm.unlink(&name_a), "unlink a");
    locks.release(hold);
    ok(fs::remove_dir_all(&dir), "rm guard dir");
}

/// I-DP5 (REQ: REQ-CORE-080, REQ-CORE-070, REQ-CORE-069): an active shared hold
/// blocks cleanup; an inactive namespace is cleaned/unlinked and recreatable.
#[test]
fn clean() {
    let shm = PosixShm;
    let locks = FlockBackend;

    let dir = lock_dir("clean");
    ok(fs::create_dir_all(&dir), "clean lock dir");
    let paths = LockPaths::new(&dir);

    let dep = DeploymentId::new(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10);
    let n1 = ok(dep.segment_name(1), "n1");
    let n2 = ok(dep.segment_name(2), "n2");
    let names = [n1.clone(), n2.clone()];

    let m1 = ok(shm.create_exclusive(&n1, 64), "create n1");
    let m2 = ok(shm.create_exclusive(&n2, 64), "create n2");
    shm.unmap(m1);
    shm.unmap(m2);

    // A live node holds the shared active lock ⇒ cleanup is refused and nothing
    // is unlinked (ActiveUntouched).
    let node_hold = ok(locks.try_shared(&paths.active()), "node active hold");
    assert_eq!(
        ok(
            namespace_clean(&shm, &locks, &paths, &names),
            "clean under hold"
        ),
        CleanReport::ActiveUntouched
    );
    assert!(shm.open_existing(&n1, 64).is_ok(), "n1 intact during hold");
    assert!(shm.open_existing(&n2, 64).is_ok(), "n2 intact during hold");

    // Node exits; the namespace is now unowned ⇒ clean unlinks every segment.
    locks.release(node_hold);
    assert_eq!(
        ok(
            namespace_clean(&shm, &locks, &paths, &names),
            "clean inactive"
        ),
        CleanReport::Cleaned
    );
    assert!(matches!(
        shm.open_existing(&n1, 64),
        Err(ShmError::NotFound)
    ));
    assert!(matches!(
        shm.open_existing(&n2, 64),
        Err(ShmError::NotFound)
    ));

    // Recreatable after cleanup (fresh deployment generation, new namespace).
    let m = ok(shm.create_exclusive(&n1, 64), "recreate n1");
    shm.unmap(m);
    ok(shm.unlink(&n1), "unlink recreated n1");
    ok(fs::remove_dir_all(&dir), "rm clean dir");
}
