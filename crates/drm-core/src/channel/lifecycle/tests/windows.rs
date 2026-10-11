//! Real-Windows lifecycle tests (Windows CI): publisher authority over
//! `LockFileEx`, pagefile-backed sections via `CreateFileMappingW`, in-place
//! `Init` recovery, consumer open-only, epoch parity across publisher
//! restarts with a live consumer mapping holding the section open (I-W4b), and
//! the M13W4 crash matrix — a publisher process killed and restarted 50 times
//! (subset of the 100× Linux loop) with kernel lock release (I-W3) and
//! exactly-once epoch advance (I-L3). Mirrors the POSIX suite (`tests/posix.rs`)
//! and the in-memory protocol suite (`tests.rs`).
//! REQ: REQ-CORE-101, REQ-CORE-054, REQ-CORE-071, REQ-CORE-019, REQ-CORE-020,
//! REQ-CORE-057, REQ-CORE-074, REQ-CORE-075, REQ-CORE-098, REQ-CORE-018,
//! REQ-CORE-038.

use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

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

/// Waits (sleeping) until `path` exists or a bounded deadline elapses.
/// REQ: REQ-CORE-101.
fn wait_for_file(path: &Path, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Publisher open that tolerates the brief `LockFileEx` release lag after a
/// process death (windows-backends.md §Edge Cases: transiently busy is treated
/// as active, never force-destroyed).
/// REQ: REQ-CORE-054, REQ-CORE-075.
fn open_publisher_settled(
    shm: WinShm,
    locks: WinLockBackend,
    path: &Path,
    n: &SegmentName,
    e: &Expectation,
) -> Publisher {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match publisher_open(&shm, &locks, path, n, e.topic_name.as_bytes(), e, &spec()) {
            Ok(p) => return p,
            Err(Status::Permission) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(err) => unreachable!("publisher open after death: {err:?}"),
        }
    }
}

/// REQ: REQ-CORE-101, REQ-CORE-054, REQ-CORE-020 — child driver for the crash
/// matrix: adopt the section named by `DRM_WIN_CRASH_NAME` under the publisher
/// authority, plant `DRM_WIN_CRASH_FLOOR` as the published `write_seq`, signal
/// `DRM_WIN_CRASH_READY`, then block until the parent kills it (the kernel
/// releases the lock). A normal suite run (no env) returns immediately.
#[test]
fn win_lifecycle_crash_child() {
    let (Ok(raw_name), Ok(raw_lock), Ok(raw_ready)) = (
        std::env::var("DRM_WIN_CRASH_NAME"),
        std::env::var("DRM_WIN_CRASH_LOCK"),
        std::env::var("DRM_WIN_CRASH_READY"),
    ) else {
        return;
    };
    let floor = std::env::var("DRM_WIN_CRASH_FLOOR")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let n = fixture(SegmentName::new(&raw_name), "child section name");
    let e = expect("pkg/topic-a");
    let path = PathBuf::from(raw_lock);
    let p = open_publisher_settled(WinShm, WinLockBackend, &path, &n, &e);
    write_u64(
        p.mapping(),
        p.view.ccb_offset,
        offset_of!(ChannelControlBlock, write_seq),
        floor,
    );
    let _ = std::fs::write(&raw_ready, b"ready");
    // Hold authority across the kill: leak the publisher so no `Drop` closes it.
    std::mem::forget(p);
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// REQ: REQ-CORE-101, REQ-CORE-054, REQ-CORE-020, REQ-CORE-018, REQ-CORE-038 —
/// I-L2/I-L3/I-L4/I-L6 Windows crash matrix: a publisher process is killed and
/// restarted 50 times (subset of the 100× Linux loop) over real `LockFileEx`
/// authority and a pagefile section kept alive by a live consumer mapping. The
/// kernel releases the lock on death (I-W3), each restart adopts the Ready
/// image and advances the epoch exactly once (I-L3), the preserved `write_seq`
/// becomes the new window floor (I-L6), and the consumer mapping observes every
/// epoch (I-L4) — no prior-run payload is ever read as new (I-L5).
#[test]
fn win_lifecycle_crash_x50() {
    let _serial = crate::test_guard::acquire();
    let shm = WinShm;
    let locks = WinLockBackend;
    let n = section("crash50");
    let e = expect("pkg/topic-a");
    let path = lock_path("crash50");
    let ready = std::env::temp_dir().join(format!("drm-win-crash-{}.ready", std::process::id()));
    let exe = fixture(std::env::current_exe(), "current_exe");

    // Seed the section (epoch 1) and keep it alive with a consumer mapping so it
    // survives every publisher death (pagefile sections die with their last
    // handle, I-W4b).
    let seed = open_publisher(shm, locks, &path, &n, &e);
    assert_eq!(seed.epoch, 1);
    let consumer: Consumer = fixture(consumer_open(&shm, &n, &e, &spec(), None), "consumer open");
    drop(seed);

    let mut epoch = 1_u32;
    let mut floor = 0_u64;
    for cycle in 0..50_u32 {
        let _ = std::fs::remove_file(&ready);
        let mut child = fixture(
            Command::new(&exe)
                .args([
                    "--exact",
                    "channel::lifecycle::tests::windows::win_lifecycle_crash_child",
                    "--nocapture",
                ])
                .env("DRM_WIN_CRASH_NAME", n.as_str())
                .env("DRM_WIN_CRASH_LOCK", &path)
                .env("DRM_WIN_CRASH_FLOOR", floor.to_string())
                .env("DRM_WIN_CRASH_READY", &ready)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
            "spawn publisher child",
        );
        wait_for_file(&ready, "publisher child");
        fixture(child.kill(), "kill publisher child");
        fixture(child.wait(), "reap publisher child");
        // Kernel released the authority: the child consumed one epoch, the
        // restart consumes the next.
        let p = open_publisher_settled(shm, locks, &path, &n, &e);
        epoch += 2;
        assert_eq!(
            p.epoch, epoch,
            "epoch advances exactly once per acquisition"
        );
        assert_eq!(
            read_u64(
                p.mapping(),
                p.view.ccb_offset,
                offset_of!(ChannelControlBlock, first_valid_seq)
            ),
            floor,
            "cycle {cycle}: floor = preserved next sequence"
        );
        assert_eq!(
            read_u32(
                consumer.mapping(),
                consumer.view.ccb_offset,
                offset_of!(ChannelControlBlock, publisher_epoch)
            ),
            epoch,
            "cycle {cycle}: consumer mapping observes the new epoch"
        );
        floor = floor.saturating_add(3);
        write_u64(
            p.mapping(),
            p.view.ccb_offset,
            offset_of!(ChannelControlBlock, write_seq),
            floor,
        );
        drop(p);
    }
    assert_eq!(epoch, 101, "50 kills => 50 restart acquisitions");
    drop(consumer);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&ready);
}
