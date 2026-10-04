//! Real-POSIX lifecycle tests (Linux CI): authority release on process death
//! and cross-process single-winner exclusion over `flock(2)` + `shm_open`.
//! REQ: REQ-CORE-054, REQ-CORE-019, REQ-CORE-020, REQ-CORE-026.

use std::io::{Read, Write};
use std::mem::offset_of;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::{Publisher, expect, fixture, spec, unique};
use crate::channel::ccb::{ChannelControlBlock, Expectation};
use crate::channel::lifecycle::publisher_open;
use crate::shm::{FlockBackend, PosixShm, SegmentName, ShmBackend};
use drm_types::Status;

/// Unique backend segment name.
/// REQ: REQ-CORE-088.
fn segment_name(tag: &str) -> SegmentName {
    match SegmentName::new(&unique(tag)) {
        Ok(n) => n,
        Err(e) => unreachable!("test segment name: {e:?}"),
    }
}

/// Unique lock-file path under the temp dir.
/// REQ: REQ-CORE-073.
fn lock_path(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "drm-lifecycle-{}-{tag}-{n}.lock",
        std::process::id()
    ))
}

/// Test guard: unlink the segment and remove the lock file at scope end.
struct Cleanup(PosixShm, SegmentName, PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = ShmBackend::unlink(&self.0, &self.1);
        let _ = std::fs::remove_file(&self.2);
    }
}

/// Best-effort one-byte report from a forked child.
fn send_byte(s: &mut UnixStream, b: u8) {
    let _ = s.write_all(std::slice::from_ref(&b));
    let _ = s.flush();
}

/// Forks the process; the child never returns into the test harness (it
/// either `_exit`s or parks while holding its resources).
///
/// # Safety
/// Plain `fork(2)`; the caller guarantees the child body performs no
/// async-signal-unsafe teardown before diverging.
/// REQ: REQ-CORE-054.
unsafe fn fork_or_fail() -> libc::pid_t {
    // SAFETY: plain fork(2) call; failure is a negative return value.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
    pid
}

/// Kills the child with SIGKILL and reaps it (the kernel releases the flock
/// and closes every descriptor).
///
/// # Safety
/// `pid` must be a forked child of this process.
/// REQ: REQ-CORE-054.
unsafe fn kill_reap(pid: libc::pid_t) {
    // SAFETY: `pid` is our forked child; waitpid reaps it exactly once.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        let mut st: libc::c_int = 0;
        libc::waitpid(pid, std::ptr::addr_of_mut!(st), 0);
    }
}

/// Writes `v` into the CCB `write_seq` word (simulating the killed writer's
/// published run).
/// REQ: REQ-CORE-020.
fn plant_floor(p: &Publisher, v: u64) {
    let off = usize::try_from(p.view.ccb_offset).unwrap_or(0)
        + offset_of!(ChannelControlBlock, write_seq);
    let base = p.mapping().ptr().as_ptr();
    // SAFETY: validated CCB base plus a naturally aligned 8-byte field.
    let dst = unsafe { std::slice::from_raw_parts_mut(base.add(off), 8) };
    dst.copy_from_slice(&v.to_ne_bytes());
}

/// Reads the CCB `first_valid_seq` word.
/// REQ: REQ-CORE-020.
fn read_first_valid_seq(p: &Publisher) -> u64 {
    let off = usize::try_from(p.view.ccb_offset).unwrap_or(0)
        + offset_of!(ChannelControlBlock, first_valid_seq);
    let base = p.mapping().ptr().as_ptr();
    // SAFETY: validated CCB base plus a naturally aligned 8-byte field.
    let src = unsafe { std::slice::from_raw_parts(base.add(off), 8) };
    let mut b = [0_u8; 8];
    b.copy_from_slice(src);
    u64::from_ne_bytes(b)
}

/// Reads the child report byte with a timeout so a wedged child fails the
/// test instead of hanging it.
/// REQ: REQ-CORE-054.
fn read_child_byte(s: &mut UnixStream) -> u8 {
    match s.set_read_timeout(Some(Duration::from_secs(30))) {
        Ok(()) => {}
        Err(e) => unreachable!("set read timeout: {e:?}"),
    }
    let mut sig = [0_u8; 1];
    match s.read_exact(&mut sig) {
        Ok(()) => {}
        Err(e) => unreachable!("child report: {e:?}"),
    }
    sig[0]
}

/// Fork-child racer: one publisher attempt, then either park holding the
/// authority or exit after reporting (`1` won, `2` refused, `0` unexpected).
/// REQ: REQ-CORE-054.
fn child_racer(
    shm: PosixShm,
    path: &Path,
    n: &SegmentName,
    e: &Expectation,
    mut s: UnixStream,
) -> ! {
    match publisher_open(
        &shm,
        &FlockBackend,
        path,
        n,
        e.topic_name.as_bytes(),
        e,
        &spec(),
    ) {
        Ok(_p) => {
            send_byte(&mut s, 1);
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        Err(Status::Permission) => {
            send_byte(&mut s, 2);
            // SAFETY: raw exit skips test-harness teardown in the fork.
            unsafe { libc::_exit(0) };
        }
        Err(_) => {
            send_byte(&mut s, 0);
            // SAFETY: raw exit skips test-harness teardown in the fork.
            unsafe { libc::_exit(1) };
        }
    }
}

/// REQ: REQ-CORE-054, REQ-CORE-020, REQ-CORE-026 — SIGKILL death releases the
/// authority (kernel flock release) and the successor adopts the Ready image
/// with epoch + 1 and the preserved floor (I-L1/I-L3).
#[test]
fn publisher_kill_restart() {
    let shm = PosixShm;
    let n = segment_name("killrestart");
    let e = expect("pkg/topic-a");
    let path = lock_path("killrestart");
    let _g = Cleanup(shm, n.clone(), path.clone());
    let (mut parent, child) = match UnixStream::pair() {
        Ok(p) => p,
        Err(e) => unreachable!("socketpair: {e:?}"),
    };
    // SAFETY: the child body below never returns into the harness.
    let pid = unsafe { fork_or_fail() };
    if pid == 0 {
        drop(parent);
        let mut s = child;
        if let Ok(p) = publisher_open(
            &shm,
            &FlockBackend,
            &path,
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ) {
            plant_floor(&p, 7);
            send_byte(&mut s, 1);
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        } else {
            send_byte(&mut s, 0);
            // SAFETY: raw exit skips test-harness teardown in the fork.
            unsafe { libc::_exit(1) };
        }
    }
    drop(child);
    assert_eq!(read_child_byte(&mut parent), 1, "child publisher open");
    // SAFETY: `pid` is our live forked child.
    unsafe { kill_reap(pid) };
    let p2: Publisher = fixture(
        publisher_open(
            &shm,
            &FlockBackend,
            &path,
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "successor publisher",
    );
    assert_eq!(p2.epoch, 2, "kernel-released authority, epoch + 1");
    assert_eq!(read_first_valid_seq(&p2), 7, "floor = preserved write_seq");
}

/// REQ: REQ-CORE-054 — cross-process single-winner exclusion (I-L1): two
/// concurrent forks race for the authority; exactly one wins and the loser
/// reads `Permission`.
#[test]
fn publisher_race_processes() {
    let shm = PosixShm;
    let n = segment_name("race");
    let e = expect("pkg/topic-a");
    let path = lock_path("race");
    let _g = Cleanup(shm, n.clone(), path.clone());
    let mut parent_ends = Vec::new();
    let mut pids = Vec::new();
    for _ in 0..2 {
        let (parent, child) = match UnixStream::pair() {
            Ok(p) => p,
            Err(e) => unreachable!("socketpair: {e:?}"),
        };
        // SAFETY: the child body diverges; the loop continues in the parent only.
        let pid = unsafe { fork_or_fail() };
        if pid == 0 {
            child_racer(shm, &path, &n, &e, child);
        }
        drop(child);
        parent_ends.push(parent);
        pids.push(pid);
    }
    let mut counts = [0_usize; 3];
    for parent in &mut parent_ends {
        let b = usize::from(read_child_byte(parent));
        counts[b.min(2)] += 1;
    }
    assert_eq!(counts[1], 1, "exactly one cross-process winner");
    assert_eq!(counts[2], 1, "loser refused with Permission");
    for pid in pids {
        // SAFETY: each is a forked child of this process.
        unsafe { kill_reap(pid) };
    }
}
