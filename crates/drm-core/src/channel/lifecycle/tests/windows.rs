//! Real-Windows lifecycle tests (Windows CI): publisher authority over
//! `LockFileEx`, pagefile-backed sections via `CreateFileMappingW`, in-place
//! `Init` recovery, consumer open-only, and epoch parity across publisher
//! restarts with a live consumer mapping holding the section open (I-W4b).
//! Mirrors the POSIX suite (`tests/posix.rs`) and the in-memory protocol suite
//! (`tests.rs`); the full crash matrix runs at the Phase 31 gate (M14W2).
//! REQ: REQ-CORE-101, REQ-CORE-054, REQ-CORE-071, REQ-CORE-019, REQ-CORE-020,
//! REQ-CORE-057, REQ-CORE-074, REQ-CORE-075, REQ-CORE-098.

use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{expect, fixture, read_u32, read_u64, spec, write_u64};
use crate::channel::ccb::{ChannelControlBlock, Expectation};
use crate::channel::lifecycle::{
    Consumer, Publisher, consumer_open, publisher_open, segment_layout,
};
use crate::shm::{
    LockBackend, SegmentName, ShmBackend, ShmError, WinLockBackend, WinShm, current_identity,
    identity_alive, is_alive, windows_section_name,
};
use drm_types::Status;

/// Unique per-test section name (FNV-1a over the tag so distinct tags never
/// collide across parallel tests).
/// REQ: REQ-CORE-101.
fn section(tag: &str) -> SegmentName {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in tag.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    fixture(
        SegmentName::new(&windows_section_name("win-lifecycle", hash)),
        "section name",
    )
}

/// Unique per-test lock-file path under the temp dir.
/// REQ: REQ-CORE-075.
fn lock_path(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "drm-win-lifecycle-{}-{tag}-{n}.lock",
        std::process::id()
    ))
}

/// Opens a publisher on the real Windows backends.
/// REQ: REQ-CORE-054.
fn open_publisher(
    shm: WinShm,
    locks: WinLockBackend,
    path: &Path,
    n: &SegmentName,
    e: &Expectation,
) -> Publisher {
    fixture(
        publisher_open(&shm, &locks, path, n, e.topic_name.as_bytes(), e, &spec()),
        "publisher open",
    )
}

/// REQ: REQ-CORE-101, REQ-CORE-054, REQ-CORE-071 — a fresh publisher creates a
/// pagefile section, publishes readiness, and a consumer opens the same section
/// with an identical descriptor (I-L8/I-O2).
#[test]
fn win_lifecycle_win_roundtrip() {
    let shm = WinShm;
    let locks = WinLockBackend;
    let n = section("fresh");
    let e = expect("pkg/topic-a");
    let path = lock_path("fresh");
    let p = open_publisher(shm, locks, &path, &n, &e);
    assert_eq!(p.epoch, 1);
    assert_eq!(p.view.descriptor.topic_handle, 42);
    let c: Consumer = fixture(consumer_open(&shm, &n, &e, &spec(), None), "consumer open");
    assert_eq!(
        c.view.descriptor.segment_size,
        p.view.descriptor.segment_size
    );
    drop(c);
    drop(p);
    let _ = std::fs::remove_file(&path);
}

/// REQ: REQ-CORE-101, REQ-CORE-054 — I-W1/I-L1: eight threads race for the
/// publisher authority over `LockFileEx`; exactly one wins and the losers read
/// `Permission` (never a partial init).
#[test]
fn win_race_create() {
    let shm = WinShm;
    let locks = WinLockBackend;
    let n = section("race");
    let e = expect("pkg/topic-a");
    let path = Arc::new(lock_path("race"));
    let wins = Arc::new(AtomicU32::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let n = n.clone();
        let e = e.clone();
        let path = Arc::clone(&path);
        let wins = Arc::clone(&wins);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let result = publisher_open(
                &shm,
                &locks,
                path.as_path(),
                &n,
                e.topic_name.as_bytes(),
                &e,
                &spec(),
            );
            if result.is_ok() {
                wins.fetch_add(1, Ordering::Relaxed);
            }
            // Hold the winning authority across the barrier so every loser
            // observes a live publisher.
            barrier.wait();
            drop(result);
        }));
    }
    for h in handles {
        fixture(h.join(), "join");
    }
    assert_eq!(
        wins.load(Ordering::Relaxed),
        1,
        "exactly one publisher authority"
    );
    let _ = std::fs::remove_file(path.as_path());
}

/// REQ: REQ-CORE-101, REQ-CORE-098 — I-W1/I-L8: an exact-size section left in
/// `Init` by a crashed creator is reinitialized in place under the held
/// authority (no unlink exists for pagefile sections) and re-published fresh.
#[test]
fn win_init_reinit() {
    let shm = WinShm;
    let locks = WinLockBackend;
    let n = section("init-reinit");
    let e = expect("pkg/topic-a");
    let path = lock_path("init-reinit");
    let size =
        usize::try_from(fixture(segment_layout(&e, &spec()), "layout").segment_size).unwrap_or(0);
    // Raw zero-filled section: the `Init` artifact of a killed creator.
    let raw = fixture(shm.create_exclusive(&n, size), "raw create");
    let p = open_publisher(shm, locks, &path, &n, &e);
    assert_eq!(p.epoch, 1, "in-place reinit publishes a fresh epoch");
    assert_eq!(p.view.descriptor.topic_handle, 42);
    let c: Consumer = fixture(consumer_open(&shm, &n, &e, &spec(), None), "consumer open");
    assert_eq!(
        c.view.descriptor.segment_size,
        p.view.descriptor.segment_size
    );
    drop(c);
    drop(p);
    drop(raw);
    let _ = std::fs::remove_file(&path);
}

/// REQ: REQ-CORE-101, REQ-CORE-071 — I-O2: the consumer never creates; a missing
/// section yields `NotFound` and leaves the namespace untouched.
#[test]
fn win_consumer_never_creates() {
    let shm = WinShm;
    let n = section("missing");
    let e = expect("pkg/topic-a");
    let res = consumer_open(&shm, &n, &e, &spec(), None);
    assert!(matches!(res, Err(Status::NotFound)), "res: {res:?}");
    assert!(matches!(shm.open_existing(&n, 1), Err(ShmError::NotFound)));
}

/// REQ: REQ-CORE-101, REQ-CORE-019, REQ-CORE-020 — I-L3/I-L4/I-L6: with a live
/// consumer mapping holding the section open across restarts (I-W4b), the epoch
/// advances exactly once per authority acquisition and the window floor is the
/// preserved next sequence; the consumer mapping observes the new epoch.
#[test]
fn win_epoch_restart_parity() {
    let shm = WinShm;
    let locks = WinLockBackend;
    let n = section("restart");
    let e = expect("pkg/topic-a");
    let path = lock_path("restart");
    let p = open_publisher(shm, locks, &path, &n, &e);
    assert_eq!(p.epoch, 1);
    let c: Consumer = fixture(consumer_open(&shm, &n, &e, &spec(), None), "consumer open");
    write_u64(
        p.mapping(),
        p.view.ccb_offset,
        offset_of!(ChannelControlBlock, write_seq),
        5,
    );
    drop(p);
    let mut floor = 5_u64;
    for expected in 2..=20_u32 {
        let p = open_publisher(shm, locks, &path, &n, &e);
        assert_eq!(p.epoch, expected, "epoch advances exactly once per restart");
        assert_eq!(
            read_u64(
                p.mapping(),
                p.view.ccb_offset,
                offset_of!(ChannelControlBlock, first_valid_seq)
            ),
            floor,
            "floor = preserved next sequence"
        );
        floor = u64::from(expected) * 3 + 1;
        write_u64(
            p.mapping(),
            p.view.ccb_offset,
            offset_of!(ChannelControlBlock, write_seq),
            floor,
        );
        drop(p);
    }
    assert_eq!(
        read_u32(
            c.mapping(),
            c.view.ccb_offset,
            offset_of!(ChannelControlBlock, publisher_epoch)
        ),
        20,
        "consumer mapping observes the latest epoch"
    );
    drop(c);
    let _ = std::fs::remove_file(&path);
}

/// REQ: REQ-CORE-057, REQ-CORE-074, REQ-CORE-075 — I-E5 subset: the registration
/// lock is the reclamation authority. A live shared hold blocks the exclusive
/// reclaim probe; once every shared hold releases the exclusive probe succeeds.
/// Identity is diagnostic only and never gates the decision.
#[test]
fn win_reclaim_authority_subset() {
    let locks = WinLockBackend;
    let path = lock_path("reclaim");
    let node_a = fixture(locks.try_shared(&path), "node a shared");
    let node_b = fixture(locks.try_shared(&path), "node b shared");
    assert!(
        matches!(locks.try_exclusive(&path), Err(ShmError::AlreadyExists)),
        "live owner blocks reclaim"
    );
    locks.release(node_b);
    assert!(
        matches!(locks.try_exclusive(&path), Err(ShmError::AlreadyExists)),
        "one live owner still blocks reclaim"
    );
    locks.release(node_a);
    let reclaimed = fixture(locks.try_exclusive(&path), "reclaim exclusive");
    locks.release(reclaimed);
    let _ = std::fs::remove_file(&path);
}

/// REQ: REQ-CORE-074 — I-W5 subset: identity is a live/diagnostic signal — the
/// current process is alive and incarnation-safe, an invalid PID is dead, and a
/// perturbed creation time is a different incarnation.
#[test]
fn win_identity_diagnostic_subset() {
    let me = current_identity();
    assert!(me.pid > 0);
    assert!(is_alive(me.pid));
    assert!(identity_alive(me.pid, me.start_ticks));
    assert!(!identity_alive(me.pid, me.start_ticks.wrapping_add(1)));
    assert!(!is_alive(0));
}
