//! Deployment-identity end-to-end suite: fingerprint isolation (I-DP4),
//! own-contract refusal before any SHM touch (I-DP1), and guarded cleanup of
//! two simultaneous deployments (I-DP5) with zero cross-namespace attachment.
//! Exercises the real `flock(2)` + `shm_open(3)` backends across forked node
//! processes (`deployment-identity.md` §Invariants, §Test Matrix).
//! REQ: REQ-CORE-085, REQ-CORE-092, REQ-CORE-025, REQ-CORE-088, REQ-CORE-089,
//! REQ-CORE-080, REQ-CORE-070, REQ-CORE-069.
#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::{
    ContractFingerprint, DeploymentId, clean as namespace_clean, guard_fingerprint, node_startup,
};
use crate::shm::{
    CleanReport, FlockBackend, LockBackend, LockPaths, PosixShm, SegmentName, ShmBackend, ShmError,
};
use crate::test_guard;
use drm_types::Status;

/// Segment size for the e2e fixtures (a single sentinel byte is exercised).
const SEG_SIZE: usize = 64;

/// Sentinel byte offset, past any fixed segment header.
const SENTINEL_OFF: usize = 32;

/// Test-suite oracle: the precondition must hold; any `Err` here is a suite bug.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("identity_e2e: {what}: {e:?}"),
    }
}

/// Fresh unique lock directory under the system temp dir.
fn lock_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("drm-identity-e2e-{tag}-{pid}-{n}"))
}

/// FNV-1a fold over `bytes`; the test's stand-in for the compiler's digest.
fn fold(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Simulates the compiled contract fingerprint over a node's OWN inputs — ABI
/// version, its topic schemas/layouts, its declared interfaces (`REQ-CORE-085`
/// input set). Deterministic and input-sensitive: an edit to any input changes
/// the digest, and an untouched contract reproduces it exactly.
fn compile_fingerprint(abi: u32, topics: &[&[u8]], interfaces: &[&[u8]]) -> ContractFingerprint {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    h = fold(h, &abi.to_le_bytes());
    h = fold(h, b"|topics|");
    for topic in topics {
        h = fold(h, topic);
        h = fold(h, b"|");
    }
    h = fold(h, b"|interfaces|");
    for iface in interfaces {
        h = fold(h, iface);
        h = fold(h, b"|");
    }
    let mut out = [0_u8; 16];
    out[..8].copy_from_slice(&h.to_le_bytes());
    out[8..].copy_from_slice(&h.rotate_left(31).to_le_bytes());
    ContractFingerprint(out)
}

/// Writes a sentinel byte into the deployment's segment (creating it if absent).
fn write_sentinel<S: ShmBackend>(shm: &S, name: &SegmentName, byte: u8) {
    let m = match shm.open_existing(name, SEG_SIZE) {
        Ok(m) => m,
        Err(ShmError::NotFound) => ok(shm.create_exclusive(name, SEG_SIZE), "create segment"),
        Err(e) => unreachable!("open for write: {e:?}"),
    };
    // SAFETY: `m` maps exactly `SEG_SIZE` (>= `SENTINEL_OFF + 1`) writable bytes;
    // the offset is in range.
    unsafe { m.ptr().as_ptr().add(SENTINEL_OFF).write(byte) };
    shm.unmap(m);
}

/// Reads the sentinel byte from the deployment's segment.
fn read_sentinel<S: ShmBackend>(shm: &S, name: &SegmentName) -> u8 {
    let m = ok(shm.open_existing(name, SEG_SIZE), "open for read");
    // SAFETY: `m` maps exactly `SEG_SIZE` (>= `SENTINEL_OFF + 1`) readable bytes;
    // the offset is in range.
    let byte = unsafe { m.ptr().as_ptr().add(SENTINEL_OFF).read() };
    shm.unmap(m);
    byte
}

/// Forks the process; the child never returns into the test harness.
///
/// # Safety
/// Plain `fork(2)`; the caller guarantees the child body diverges without
/// async-signal-unsafe teardown.
unsafe fn fork_or_fail() -> libc::pid_t {
    // SAFETY: plain fork(2); failure is a negative return value.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
    pid
}

/// Kills the child with SIGKILL and reaps it (the kernel releases the flock and
/// closes every descriptor).
///
/// # Safety
/// `pid` must be a forked child of this process.
unsafe fn kill_reap(pid: libc::pid_t) {
    // SAFETY: `pid` is our forked child; waitpid reaps it exactly once.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        let mut status: libc::c_int = 0;
        libc::waitpid(pid, std::ptr::addr_of_mut!(status), 0);
    }
}

/// Parks the current (forked) child until the parent kills it — no sleeps.
fn park_until_killed() -> ! {
    loop {
        // SAFETY: `pause` blocks until a signal; the parent sends SIGKILL.
        unsafe { libc::pause() };
    }
}

/// Best-effort one-byte report from a forked child.
fn send_byte(s: &mut UnixStream, byte: u8) {
    let _ = s.write_all(std::slice::from_ref(&byte));
    let _ = s.flush();
}

/// Reads the child report byte with a timeout so a wedged child fails the test
/// instead of hanging it.
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

/// Fork-child node: run the real startup sequence (fingerprint guard, then the
/// shared deployment active hold), report readiness, and park holding the guard
/// for the node's whole lifetime (`REQ-CORE-069`).
fn child_node_startup<L: LockBackend>(
    locks: &L,
    paths: &LockPaths,
    fp: ContractFingerprint,
    mut s: UnixStream,
) -> ! {
    if let Ok(_hold) = node_startup(locks, paths, fp, Some(fp)) {
        send_byte(&mut s, 1);
        park_until_killed();
    } else {
        send_byte(&mut s, 2);
        // SAFETY: raw exit skips test-harness teardown in the fork.
        unsafe { libc::_exit(1) };
    }
}

/// Tracks forked children so every path — including a failed assertion — reaps
/// them; an unreaped parked child would leak its `flock` hold.
struct ChildGuard(Vec<libc::pid_t>);

impl ChildGuard {
    /// Registers a freshly forked child.
    fn add(&mut self, pid: libc::pid_t) {
        self.0.push(pid);
    }

    /// Kills and reaps one tracked child (removing it from the guard).
    fn kill(&mut self, pid: libc::pid_t) {
        if let Some(pos) = self.0.iter().position(|&p| p == pid) {
            self.0.swap_remove(pos);
            // SAFETY: `pid` was forked by this process.
            unsafe { kill_reap(pid) };
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        for &pid in &self.0 {
            // SAFETY: every tracked pid was forked by this process.
            unsafe { kill_reap(pid) };
        }
    }
}

/// Forks a node process that holds the deployment's shared active guard and
/// returns its pid; the parent blocks on the readiness byte (no sleeps).
fn spawn_node<L: LockBackend>(
    locks: &L,
    paths: &LockPaths,
    fp: ContractFingerprint,
    children: &mut ChildGuard,
) -> libc::pid_t {
    let (mut parent, child) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(e) => unreachable!("socketpair: {e:?}"),
    };
    // SAFETY: the child body below diverges and never returns into the harness.
    let pid = unsafe { fork_or_fail() };
    if pid == 0 {
        drop(parent);
        child_node_startup(locks, paths, fp, child);
    }
    drop(child);
    children.add(pid);
    assert_eq!(read_child_byte(&mut parent), 1, "forked node startup");
    pid
}

/// I-DP4 (REQ: REQ-CORE-085, REQ-CORE-092): an unrelated fingerprint change (a
/// different node/topic) leaves the unaffected node's compiled fingerprint
/// identical, so the startup guard still admits it; the changed node is refused.
#[test]
fn identity_e2e_fingerprint_isolation() {
    // Node A declares topic T1; node B declares topic T2 (different node/topic).
    let t1: &[u8] = b"topic/t1:schema-v1";
    let t2_v1: &[u8] = b"topic/t2:schema-v1";
    let iface_a: &[u8] = b"iface/alpha";
    let iface_b: &[u8] = b"iface/beta";

    let a_before = compile_fingerprint(1, &[t1], &[iface_a]);
    let b_before = compile_fingerprint(1, &[t2_v1], &[iface_b]);
    assert_eq!(
        guard_fingerprint(a_before, Some(a_before)),
        Ok(()),
        "node A starts"
    );
    assert_eq!(
        guard_fingerprint(b_before, Some(b_before)),
        Ok(()),
        "node B starts"
    );

    // Unrelated change: node B's own topic T2 schema is edited.
    let t2_v2: &[u8] = b"topic/t2:schema-v2";
    let b_after = compile_fingerprint(1, &[t2_v2], &[iface_b]);
    assert_ne!(b_before, b_after, "node B fingerprint tracks its own topic");
    // Node A's inputs are untouched; recompiling reproduces its fingerprint.
    let a_after = compile_fingerprint(1, &[t1], &[iface_a]);

    // Snapshot before/after identical for the unaffected node.
    assert_eq!(
        a_before, a_after,
        "I-DP4: unrelated edit leaves node A unchanged"
    );
    // The guard still admits node A against its (unchanged) expectation.
    assert_eq!(
        guard_fingerprint(a_after, Some(a_before)),
        Ok(()),
        "node A unaffected"
    );
    // Node B's stale binary no longer matches its changed contract.
    assert_eq!(
        guard_fingerprint(b_before, Some(b_after)),
        Err(Status::WrongAbi),
        "node B refused"
    );
}

/// I-DP1 (REQ: REQ-CORE-092, REQ-CORE-025): a node whose OWN contract changed
/// is refused before any lock or SHM object is touched — the active lock is not
/// created and the deployment namespace is left intact.
#[test]
fn identity_e2e_own_fingerprint_refused_before_shm() {
    let _serial = test_guard::acquire();
    let shm = PosixShm;
    let locks = FlockBackend;

    let dir = lock_dir("e2e-guard");
    ok(fs::create_dir_all(&dir), "guard lock dir");
    let paths = LockPaths::new(&dir);
    let dep = DeploymentId::new(0x0BAD_F00D_1234_5678_9ABC_DEF0_1122_3344);
    let name = ok(dep.segment_name(7), "guard name");
    // The deployment namespace is already instantiated (a live segment exists).
    write_sentinel(&shm, &name, 0x42);

    // The node's OWN contract changed: compiled != deployment expectation.
    let own_v1: &[u8] = b"topic/own:schema-v1";
    let own_v2: &[u8] = b"topic/own:schema-v2";
    let compiled = compile_fingerprint(1, &[own_v1], &[]);
    let expected = compile_fingerprint(1, &[own_v2], &[]);
    assert_ne!(compiled, expected, "the node's own contract changed");

    assert_eq!(
        guard_fingerprint(compiled, Some(expected)),
        Err(Status::WrongAbi)
    );
    match node_startup(&locks, &paths, compiled, Some(expected)) {
        Err(Status::WrongAbi) => {}
        other => unreachable!("own-fingerprint refusal expected: {other:?}"),
    }
    assert!(
        !paths.active().exists(),
        "refusal must not create the active lock"
    );
    assert_eq!(
        read_sentinel(&shm, &name),
        0x42,
        "refusal leaves the namespace untouched"
    );

    // Control: the matching fingerprint acquires the shared active hold.
    let hold = ok(
        node_startup(&locks, &paths, expected, Some(expected)),
        "matching startup",
    );
    assert!(
        paths.active().exists(),
        "matching startup acquires the active lock"
    );
    locks.release(hold);

    ok(shm.unlink(&name), "unlink guard name");
    ok(fs::remove_dir_all(&dir), "rm guard dir");
}

/// Asserts cleanup of a live deployment is refused (`ActiveUntouched`) and its
/// segment is intact.
fn assert_clean_refused<S: ShmBackend, L: LockBackend>(
    shm: &S,
    locks: &L,
    paths: &LockPaths,
    name: &SegmentName,
) {
    assert_eq!(
        ok(
            namespace_clean(shm, locks, paths, std::slice::from_ref(name)),
            "clean live"
        ),
        CleanReport::ActiveUntouched
    );
    assert!(
        shm.open_existing(name, SEG_SIZE).is_ok(),
        "segment intact while live"
    );
}

/// Asserts cleanup of an inactive deployment succeeds and unlinks its segment.
fn assert_clean_unlinks<S: ShmBackend, L: LockBackend>(
    shm: &S,
    locks: &L,
    paths: &LockPaths,
    name: &SegmentName,
) {
    assert_eq!(
        ok(
            namespace_clean(shm, locks, paths, std::slice::from_ref(name)),
            "clean inactive"
        ),
        CleanReport::Cleaned
    );
    assert!(
        matches!(shm.open_existing(name, SEG_SIZE), Err(ShmError::NotFound)),
        "segment unlinked"
    );
}

/// I-DP5 + I-DP2 (REQ: REQ-CORE-080, REQ-CORE-070, REQ-CORE-069, REQ-CORE-088,
/// REQ-CORE-089): two deployments run live at once in separate node processes
/// with zero cross-namespace attachment; an active deployment is refused
/// cleanup (`ActiveUntouched`, segments intact) while an inactive one is
/// cleaned completely and is recreatable.
#[test]
fn identity_e2e_two_deployments_clean_live() {
    let _serial = test_guard::acquire();
    let shm = PosixShm;
    let locks = FlockBackend;

    let dir_a = lock_dir("e2e-a");
    let dir_b = lock_dir("e2e-b");
    ok(fs::create_dir_all(&dir_a), "lock dir a");
    ok(fs::create_dir_all(&dir_b), "lock dir b");
    let paths_a = LockPaths::new(&dir_a);
    let paths_b = LockPaths::new(&dir_b);

    let dep_a = DeploymentId::new(0x1111_2222_3333_4444_5555_6666_7777_8888);
    let dep_b = DeploymentId::new(0x9999_AAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0000);
    let name_a = ok(dep_a.segment_name(1), "name a");
    let name_b = ok(dep_b.segment_name(1), "name b");
    assert_ne!(
        name_a.as_str(),
        name_b.as_str(),
        "distinct deployments ⇒ distinct namespaces"
    );

    // Both deployment namespaces are instantiated at once with distinct
    // contents — a name from one deployment never resolves to the other.
    write_sentinel(&shm, &name_a, 0xA5);
    write_sentinel(&shm, &name_b, 0x5A);
    assert_eq!(
        read_sentinel(&shm, &name_a),
        0xA5,
        "zero cross-namespace attachment (a)"
    );
    assert_eq!(
        read_sentinel(&shm, &name_b),
        0x5A,
        "zero cross-namespace attachment (b)"
    );

    // Two live nodes, each holding its deployment's shared active guard.
    let fp = ContractFingerprint([0xAA; 16]);
    let mut children = ChildGuard(Vec::new());
    let pid_a = spawn_node(&locks, &paths_a, fp, &mut children);
    let pid_b = spawn_node(&locks, &paths_b, fp, &mut children);

    // Active deployments: cleanup is refused, nothing is unlinked.
    assert_clean_refused(&shm, &locks, &paths_a, &name_a);
    assert_clean_refused(&shm, &locks, &paths_b, &name_b);

    // Node B exits; its namespace is now unowned ⇒ cleaned completely, and A's
    // namespace is untouched (no cross-namespace effect).
    children.kill(pid_b);
    assert_clean_unlinks(&shm, &locks, &paths_b, &name_b);
    assert_eq!(
        read_sentinel(&shm, &name_a),
        0xA5,
        "b cleanup leaves a untouched"
    );

    // The cleaned namespace is recreatable (fresh generation, same id family).
    write_sentinel(&shm, &name_b, 0x3C);
    assert_eq!(read_sentinel(&shm, &name_b), 0x3C, "recreated namespace");
    assert_clean_unlinks(&shm, &locks, &paths_b, &name_b);

    // Node A is still live: cleanup stays refused until it exits.
    assert_clean_refused(&shm, &locks, &paths_a, &name_a);
    children.kill(pid_a);
    assert_clean_unlinks(&shm, &locks, &paths_a, &name_a);

    ok(fs::remove_dir_all(&dir_a), "rm dir a");
    ok(fs::remove_dir_all(&dir_b), "rm dir b");
}
