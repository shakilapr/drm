//! Lifecycle protocol tests. The in-memory backends model the platform
//! contracts (zero-filled exclusive create, name registry, process-scoped
//! locks) so the open protocol runs on every host.

#[cfg(unix)]
mod posix;

use std::collections::{HashMap, HashSet};
use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{
    Consumer, Expectation, LayoutSpec, LockBackend, Publisher, ShmBackend, ShmError, consumer_open,
    publisher_open,
};
use crate::channel::ccb::ChannelControlBlock;
use crate::shm::{LockHandle, Mapping, SegmentName};
use drm_types::Status;

/// Unique test name generator (per-process; parallel-safe).
fn unique(tag: &str) -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("/drm.test.{}.{}.{n}", std::process::id(), tag)
}

/// Lease-backed heap segment registry; memory is intentionally leaked for
/// the process lifetime (tests are short-lived; open mappings must survive
/// `unlink`, mirroring POSIX).
#[derive(Default)]
struct MemShm {
    slots: Mutex<HashMap<String, (usize, usize)>>,
}

impl MemShm {
    fn lookup(&self, name: &SegmentName) -> Option<(usize, usize)> {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name.as_str())
            .copied()
    }
}

impl ShmBackend for MemShm {
    fn create_exclusive(&self, name: &SegmentName, size: usize) -> Result<Mapping, ShmError> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slots.contains_key(name.as_str()) {
            return Err(ShmError::AlreadyExists);
        }
        let mut v = vec![0_u8; size];
        let ptr = v.as_mut_ptr();
        let addr = ptr as usize;
        std::mem::forget(v);
        slots.insert(name.as_str().to_string(), (addr, size));
        // `ptr` was just allocated with `size` writable bytes and is
        // intentionally leaked (registry-owned for the process lifetime).
        Ok(Mapping::from_raw_parts(
            std::ptr::NonNull::new(ptr)
                .unwrap_or_else(|| unreachable!("non-null fresh allocation")),
            size,
        ))
    }

    fn open_existing(&self, name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError> {
        let Some((addr, size)) = self.lookup(name) else {
            return Err(ShmError::NotFound);
        };
        if size < 16 {
            return Err(ShmError::StaleIncomplete);
        }
        if size != expect_size {
            return Err(ShmError::SizeMismatch);
        }
        // Registry addresses are valid leaked allocations of `size` bytes;
        // aliasing mirrors POSIX shared mappings.
        Ok(Mapping::from_raw_parts(
            std::ptr::NonNull::new(addr as *mut u8)
                .unwrap_or_else(|| unreachable!("registry stores non-null")),
            size,
        ))
    }

    fn unmap(&self, _m: Mapping) {}

    fn unlink(&self, name: &SegmentName) -> Result<(), ShmError> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots
            .remove(name.as_str())
            .map(|_| ())
            .ok_or(ShmError::NotFound)
    }

    fn precommit_and_prove(&self, _m: &Mapping) -> Result<(), ShmError> {
        Ok(())
    }

    /// `open_raw` is a unix-only trait member (classification is the caller's
    /// job); the lifecycle protocol uses `open_existing` exclusively, so the
    /// in-memory backend does not model it.
    /// REQ: REQ-CORE-098.
    #[cfg(unix)]
    fn open_raw(&self, _name: &SegmentName) -> Result<(std::os::fd::OwnedFd, u64), ShmError> {
        Err(ShmError::NotFound)
    }
}

/// Process-scoped lock table; the release hook runs on `LockHandle` drop.
#[derive(Clone, Default)]
struct MemLock {
    held: Arc<Mutex<HashSet<String>>>,
}

impl LockBackend for MemLock {
    fn try_exclusive(&self, path: &Path) -> Result<LockHandle, ShmError> {
        let key = path.to_string_lossy().to_string();
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.contains(&key) {
            return Err(ShmError::AlreadyExists);
        }
        held.insert(key.clone());
        let registry = Arc::clone(&self.held);
        Ok(LockHandle::from_hook(Box::new(move || {
            registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        })))
    }

    fn try_shared(&self, path: &Path) -> Result<LockHandle, ShmError> {
        self.try_exclusive(path)
    }

    fn release(self, h: LockHandle) {
        drop(h);
    }
}

/// Fixed single-slot channel expectation for the test images.
fn expect(name: &str) -> Expectation {
    Expectation {
        schema_hash: [7; 16],
        layout_hash: [9; 16],
        topic_handle: 42,
        topic_name: name.to_string(),
        payload_capacity: 64,
        history_depth: 2,
        pool_size: 3,
        overflow_policy: 0,
        consumer_count: 1,
        required_count: 0,
        diagnostic_count: 0,
    }
}

fn spec() -> LayoutSpec {
    LayoutSpec {
        flags: 0,
        slot_align: 8,
    }
}

fn name(text: &str) -> SegmentName {
    SegmentName::new(text).unwrap_or_else(|e| unreachable!("valid test name: {text}: {e:?}"))
}

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("{what}: {e:?}"),
    }
}

/// REQ: REQ-CORE-054, REQ-CORE-071 — a fresh publisher creates, publishes
/// readiness, and a consumer opens the same segment afterwards.
#[test]
fn publisher_fresh_then_consumer_opens() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("fresh"));
    let e = expect("pkg/topic-a");
    let p: Publisher = fixture(
        publisher_open(
            &shm,
            &locks,
            Path::new("/locks/t42"),
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "publisher open",
    );
    assert_eq!(p.epoch, 1);
    assert_eq!(p.view.descriptor.topic_handle, 42);
    let c: Consumer = fixture(consumer_open(&shm, &n, &e, &spec(), None), "consumer open");
    assert_eq!(
        c.view.descriptor.segment_size,
        p.view.descriptor.segment_size
    );
}

/// REQ: REQ-CORE-054 — I-L1/I-O1: a second publisher is refused while the
/// first holds the authority; release re-enables acquisition.
#[test]
fn publisher_race_second_refused() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("race"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    let p: Publisher = fixture(
        publisher_open(
            &shm,
            &locks,
            &path,
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "first publisher",
    );
    let second = publisher_open(
        &shm,
        &locks,
        &path,
        &n,
        e.topic_name.as_bytes(),
        &e,
        &spec(),
    );
    assert!(
        matches!(second, Err(Status::Permission)),
        "authority is exclusive: {second:?}"
    );
    drop(p);
    let third: Publisher = fixture(
        publisher_open(
            &shm,
            &locks,
            &path,
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "publisher after release",
    );
    assert_eq!(third.epoch, 2, "authority acquisition advances the epoch");
}

/// REQ: REQ-CORE-054, REQ-CORE-071 — concurrent opens: exactly one wins the
/// authority; the loser is refused (barrier keeps both `Publisher`s live
/// across the attempts, so the second attempt cannot follow a release).
#[test]
fn publisher_race_threads() {
    let shm = Arc::new(MemShm::default());
    let locks = MemLock::default();
    let n = name(&unique("tracerace"));
    let e = expect("pkg/topic-a");
    let path = Arc::new(PathBuf::from("/locks/t42"));
    let wins = Arc::new(AtomicU32::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let shm = Arc::clone(&shm);
        let locks = locks.clone();
        let n = n.clone();
        let e = e.clone();
        let path = Arc::clone(&path);
        let wins = Arc::clone(&wins);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let result = publisher_open(
                &*shm,
                &locks,
                &path,
                &n,
                e.topic_name.as_bytes(),
                &e,
                &spec(),
            );
            if result.is_ok() {
                wins.fetch_add(1, Ordering::Relaxed);
            }
            barrier.wait();
            drop(result);
        }));
    }
    for h in handles {
        h.join().unwrap_or_else(|_| unreachable!("join"));
    }
    assert_eq!(wins.load(Ordering::Relaxed), 1, "exactly one authority");
}

/// REQ: REQ-CORE-071 — I-O2: the consumer never creates; a missing segment
/// yields `NotFound` and leaves the namespace untouched.
#[test]
fn consumer_open_never_creates() {
    let shm = MemShm::default();
    let n = name(&unique("missing"));
    let e = expect("pkg/topic-a");
    let res = consumer_open(&shm, &n, &e, &spec(), None);
    assert!(matches!(res, Err(Status::NotFound)), "res: {res:?}");
    assert!(shm.lookup(&n).is_none(), "no creation side effect");
}

/// REQ: REQ-CORE-071 — a segment stuck in `Init` yields `NotReady` within the
/// bounded wait; the consumer never repairs or interprets `Init` bytes.
#[test]
fn consumer_bounded_wait_init() {
    let shm = MemShm::default();
    let n = name(&unique("init"));
    let e = expect("pkg/topic-a");
    let size = fixture(
        super::segment_layout(&e, &spec()).map(|l| l.segment_size),
        "layout",
    );
    let size = usize::try_from(size).unwrap_or(0);
    fixture(shm.create_exclusive(&n, size), "raw create");
    let res = consumer_open(&shm, &n, &e, &spec(), Some(Duration::from_millis(20)));
    assert!(matches!(res, Err(Status::NotReady)), "res: {res:?}");
}

/// REQ: REQ-CORE-071 — a wrong-size segment is a terminal `InvalidData`
/// classification, never retried into the wait budget.
#[test]
fn consumer_size_mismatch_terminal() {
    let shm = MemShm::default();
    let n = name(&unique("sizemismatch"));
    let e = expect("pkg/topic-a");
    let size = fixture(
        super::segment_layout(&e, &spec()).map(|l| l.segment_size),
        "layout",
    );
    let size = usize::try_from(size).unwrap_or(0);
    fixture(shm.create_exclusive(&n, size + 8), "raw create wrong size");
    let res = consumer_open(&shm, &n, &e, &spec(), Some(Duration::from_millis(20)));
    assert!(
        matches!(res, Err(Status::InvalidData)),
        "SizeMismatch maps to InvalidData: {res:?}"
    );
}

/// REQ: REQ-CORE-020, REQ-CORE-026 — restart publication: the successor
/// adopts the Ready image, publishes `first_valid_seq = write_seq` and
/// advances the epoch exactly once (I-L3/I-L6/I-L7).
#[test]
fn publisher_reopen_floor_and_epoch() {
    let shm = Arc::new(MemShm::default());
    let locks = MemLock::default();
    let n = name(&unique("reopen"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    {
        let p: Publisher = fixture(
            publisher_open(
                &*shm,
                &locks,
                &path,
                &n,
                e.topic_name.as_bytes(),
                &e,
                &spec(),
            ),
            "first publisher",
        );
        // Simulate a published run: sequence advanced to 5 by the "crashed" writer.
        let base = p.mapping().ptr().as_ptr();
        let off = usize::try_from(p.view.ccb_offset).unwrap_or(0)
            + offset_of!(ChannelControlBlock, write_seq);
        let dst = unsafe { std::slice::from_raw_parts_mut(base.add(off), 8) };
        dst.copy_from_slice(&5_u64.to_ne_bytes());
    }
    let p2: Publisher = fixture(
        publisher_open(
            &*shm,
            &locks,
            &path,
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "successor publisher",
    );
    assert_eq!(p2.epoch, 2, "exactly-once epoch advance");
    let base = p2.mapping().ptr().as_ptr();
    let off = usize::try_from(p2.view.ccb_offset).unwrap_or(0)
        + offset_of!(ChannelControlBlock, first_valid_seq);
    let src = unsafe { std::slice::from_raw_parts(base.add(off), 8) };
    let mut b = [0_u8; 8];
    b.copy_from_slice(src);
    assert_eq!(u64::from_ne_bytes(b), 5, "floor = preserved write_seq");
}

/// REQ: REQ-CORE-098, I-L8 — an `Init`-stuck segment is recovered by the
/// authority holder (unlink + recreate) and re-published fresh.
#[test]
fn publisher_recovers_init_segment() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("initrecover"));
    let e = expect("pkg/topic-a");
    let size = fixture(
        super::segment_layout(&e, &spec()).map(|l| l.segment_size),
        "layout",
    );
    let size = usize::try_from(size).unwrap_or(0);
    fixture(shm.create_exclusive(&n, size), "raw create");
    let p: Publisher = fixture(
        publisher_open(
            &shm,
            &locks,
            Path::new("/locks/t42"),
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "recovering publisher",
    );
    assert_eq!(p.epoch, 1, "fresh image after recovery");
}

/// REQ: REQ-CORE-098 — an undersized stale object (`StaleIncomplete`) is
/// unlinked and recreated under the held authority.
#[test]
fn publisher_recovers_undersized() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("undersized"));
    let e = expect("pkg/topic-a");
    fixture(shm.create_exclusive(&n, 8), "raw undersized create");
    let p: Publisher = fixture(
        publisher_open(
            &shm,
            &locks,
            Path::new("/locks/t42"),
            &n,
            e.topic_name.as_bytes(),
            &e,
            &spec(),
        ),
        "recovering publisher",
    );
    assert_eq!(p.epoch, 1);
}

/// REQ: REQ-CORE-019 — epoch space exhaustion is explicit (`EpochExhausted`),
/// never a wrap.
#[test]
fn publisher_epoch_exhaustion() {
    let shm = Arc::new(MemShm::default());
    let locks = MemLock::default();
    let n = name(&unique("epochmax"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    {
        let p: Publisher = fixture(
            publisher_open(
                &*shm,
                &locks,
                &path,
                &n,
                e.topic_name.as_bytes(),
                &e,
                &spec(),
            ),
            "first publisher",
        );
        let base = p.mapping().ptr().as_ptr();
        let off = usize::try_from(p.view.ccb_offset).unwrap_or(0)
            + offset_of!(ChannelControlBlock, publisher_epoch);
        let dst = unsafe { std::slice::from_raw_parts_mut(base.add(off), 4) };
        dst.copy_from_slice(&u32::MAX.to_ne_bytes());
    }
    let res = publisher_open(
        &*shm,
        &locks,
        &path,
        &n,
        e.topic_name.as_bytes(),
        &e,
        &spec(),
    );
    assert!(matches!(res, Err(Status::EpochExhausted)), "res: {res:?}");
}
