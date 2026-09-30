//! `same_channel` — one segment, two attachers: creator and consumer map the
//! same shared memory; bytes written through one are observed through the
//! other. Consumer attach never creates.
//! REQ: REQ-CORE-072, REQ-CORE-071.
#![cfg(unix)]
#![allow(clippy::cast_possible_truncation)] // byte index mod 256 is the payload

use drm_core::shm::{PosixShm, SegmentName, ShmBackend, ShmError};

use crate::child_util::{ok, unique_name};

/// REQ: REQ-CORE-072, REQ-CORE-071 — same-channel byte parity across two
/// mappings, then absence after unlink.
#[test]
fn same_channel() {
    const SIZE: usize = 64;
    let shm = PosixShm;
    let name = ok(SegmentName::new(&unique_name("same")), "name");

    let publisher = ok(shm.create_exclusive(&name, SIZE), "creator");
    let consumer = ok(shm.open_existing(&name, SIZE), "consumer");

    let pub_ptr = publisher.ptr();
    let con_ptr = consumer.ptr();
    let pub_addr = pub_ptr.as_ptr() as usize;
    let con_addr = con_ptr.as_ptr() as usize;
    assert_ne!(pub_addr, con_addr, "mappings are distinct views");

    // Publisher writes; consumer observes the same bytes (shared memory).
    for i in 0..SIZE {
        // SAFETY: within `publisher`'s mapped bounds (SIZE bytes).
        unsafe { pub_ptr.as_ptr().add(i).write_volatile((i as u8) ^ 0xA5) };
    }
    for i in 0..SIZE {
        // SAFETY: within `consumer`'s mapped bounds (SIZE bytes).
        let byte = unsafe { con_ptr.as_ptr().add(i).read_volatile() };
        assert_eq!(byte, (i as u8) ^ 0xA5, "byte {i}");
    }

    // Consumer never creates: unlink then attach ⇒ NotFound, not creation.
    ok(shm.unlink(&name), "unlink");
    shm.unmap(publisher);
    shm.unmap(consumer);
    assert!(matches!(
        shm.open_existing(&name, SIZE),
        Err(ShmError::NotFound)
    ));
}
