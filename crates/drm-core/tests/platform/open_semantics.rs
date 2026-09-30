//! `open_semantics` — shm-backend.md I2 + I3: open never creates; size is
//! validated only after stale classification.
//! REQ: REQ-CORE-098, REQ-CORE-071.
#![cfg(unix)]

use std::mem::size_of;

use drm_core::channel::ccb::SegmentPrefix;
use drm_core::shm::{PosixShm, SegmentName, ShmBackend, ShmError};

use crate::child_util::{ok, unique_name};

/// REQ: REQ-CORE-098 — I2 open never creates; I3 size mismatch after
/// classification; undersized object ⇒ `StaleIncomplete`; unlink restores absence.
#[test]
fn open_semantics() {
    let shm = PosixShm;
    let name = ok(SegmentName::new(&unique_name("os")), "name");

    // I2: open_existing on a nonexistent name never creates.
    assert!(matches!(
        shm.open_existing(&name, 64),
        Err(ShmError::NotFound)
    ));
    // Absence is still absence (no creation side effect).
    assert!(matches!(
        shm.open_existing(&name, 64),
        Err(ShmError::NotFound)
    ));

    // Exact create, exact-size reopen succeeds.
    let m = ok(shm.create_exclusive(&name, 64), "create");
    assert_eq!(m.len(), 64);
    let m2 = ok(shm.open_existing(&name, 64), "open exact");
    assert_eq!(m2.len(), 64);

    // I3: wrong expected size ⇒ SizeMismatch, no partial mapping.
    assert!(matches!(
        shm.open_existing(&name, 65),
        Err(ShmError::SizeMismatch)
    ));

    // Undersized live object (below the fixed SegmentPrefix) ⇒ StaleIncomplete.
    let small = ok(SegmentName::new(&unique_name("os-small")), "small name");
    let small_m = ok(
        shm.create_exclusive(&small, size_of::<SegmentPrefix>() / 2),
        "create small",
    );
    assert!(matches!(
        shm.open_existing(&small, 64),
        Err(ShmError::StaleIncomplete)
    ));

    ok(shm.unlink(&small), "unlink small");
    shm.unmap(small_m);
    assert!(matches!(
        shm.open_existing(&small, 64),
        Err(ShmError::NotFound)
    ));

    // Reopen after unlink ⇒ NotFound (consumer-never-create, REQ-CORE-071).
    ok(shm.unlink(&name), "unlink");
    shm.unmap(m);
    shm.unmap(m2);
    assert!(matches!(
        shm.open_existing(&name, 64),
        Err(ShmError::NotFound)
    ));
}
