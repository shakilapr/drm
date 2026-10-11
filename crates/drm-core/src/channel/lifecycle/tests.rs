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
use crate::channel::{Channel, OverflowPolicy, ReadOutcome, ReaderState};
use crate::ring::LookupErr;
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

/// Opens a publisher on the shared in-memory fixtures.
/// REQ: REQ-CORE-054.
fn open_publisher(
    shm: &MemShm,
    locks: &MemLock,
    path: &Path,
    n: &SegmentName,
    e: &Expectation,
) -> Publisher {
    fixture(
        publisher_open(shm, locks, path, n, e.topic_name.as_bytes(), e, &spec()),
        "publisher open",
    )
}

/// Segment-relative byte offset of a CCB field.
fn ccb_field(ccb_offset: u64, field: usize) -> usize {
    usize::try_from(ccb_offset).unwrap_or(0) + field
}

/// Writes a naturally aligned 8-byte CCB word (test instrumentation).
/// REQ: REQ-CORE-020.
fn write_u64(map: &Mapping, ccb_offset: u64, field: usize, v: u64) {
    let base = map.ptr().as_ptr();
    // SAFETY: `ccb_offset` is the validated CCB offset and `field` is a
    // naturally aligned 8-byte field within the mapped segment.
    let dst = unsafe { std::slice::from_raw_parts_mut(base.add(ccb_field(ccb_offset, field)), 8) };
    dst.copy_from_slice(&v.to_ne_bytes());
}

/// Reads a naturally aligned 8-byte CCB word (test instrumentation).
/// REQ: REQ-CORE-020.
fn read_u64(map: &Mapping, ccb_offset: u64, field: usize) -> u64 {
    let base = map.ptr().as_ptr();
    // SAFETY: see `write_u64`.
    let src = unsafe { std::slice::from_raw_parts(base.add(ccb_field(ccb_offset, field)), 8) };
    let mut b = [0_u8; 8];
    b.copy_from_slice(src);
    u64::from_ne_bytes(b)
}

/// Reads a naturally aligned 4-byte CCB word (test instrumentation).
/// REQ: REQ-CORE-019.
fn read_u32(map: &Mapping, ccb_offset: u64, field: usize) -> u32 {
    let base = map.ptr().as_ptr();
    // SAFETY: validated CCB base plus a naturally aligned 4-byte field.
    let src = unsafe { std::slice::from_raw_parts(base.add(ccb_field(ccb_offset, field)), 4) };
    let mut b = [0_u8; 4];
    b.copy_from_slice(src);
    u32::from_ne_bytes(b)
}

/// Writes a CCB `AtomicBool` flag (test instrumentation).
/// REQ: REQ-CORE-026.
fn write_flag(map: &Mapping, ccb_offset: u64, field: usize, v: bool) {
    let base = map.ptr().as_ptr();
    // SAFETY: validated CCB base plus a 1-byte flag.
    let dst = unsafe { std::slice::from_raw_parts_mut(base.add(ccb_field(ccb_offset, field)), 1) };
    dst[0] = u8::from(v);
}

/// Reads a CCB `AtomicBool` flag (test instrumentation).
/// REQ: REQ-CORE-026.
fn read_flag(map: &Mapping, ccb_offset: u64, field: usize) -> bool {
    let base = map.ptr().as_ptr();
    // SAFETY: validated CCB base plus a 1-byte flag.
    let src = unsafe { std::slice::from_raw_parts(base.add(ccb_field(ccb_offset, field)), 1) };
    src[0] != 0
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

/// REQ: REQ-CORE-019, REQ-CORE-020 — I-L3: the epoch is initialized to 1 on a
/// fresh segment and advances by exactly one per authority acquisition (init
/// exactly once, increment exactly once).
#[test]
fn publisher_epoch_init_and_increment_exactly_once() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("epochonce"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    for expected in 1..=5_u32 {
        let p = open_publisher(&shm, &locks, &path, &n, &e);
        assert_eq!(p.epoch, expected, "epoch advances exactly once per open");
        drop(p);
    }
}

/// REQ: REQ-CORE-019, REQ-CORE-020, REQ-CORE-026 — I-L2/I-L3/I-L6: a
/// deterministic 100-cycle restart loop over the in-memory backends (no fork
/// or OS churn): each acquisition advances the epoch by exactly one and
/// republishes `first_valid_seq = write_seq` from the prior run, so the
/// sequence continues at the preserved floor and the epoch disambiguates runs.
#[test]
fn publisher_restart_loop_100() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("restartloop"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    let mut floor = 0_u64;
    for cycle in 0..100_u32 {
        let p = open_publisher(&shm, &locks, &path, &n, &e);
        assert_eq!(p.epoch, cycle + 1, "epoch advances exactly once per cycle");
        assert_eq!(
            read_u64(
                p.mapping(),
                p.view.ccb_offset,
                offset_of!(ChannelControlBlock, first_valid_seq)
            ),
            floor,
            "watermark = preserved next sequence of the prior run"
        );
        // This run publishes up to `floor`, then dies before the next cycle.
        floor = u64::from(cycle) * 3 + 7;
        write_u64(
            p.mapping(),
            p.view.ccb_offset,
            offset_of!(ChannelControlBlock, write_seq),
            floor,
        );
        drop(p);
    }
}

/// REQ: REQ-CORE-020, REQ-CORE-026 — I-L6/I-L7: restart clears the prior run's
/// transient state (`publish_busy`, `reclaim_requested`), moves the watermark
/// to the preserved next sequence, and advances the epoch once.
#[test]
fn publisher_restart_resets_run_state() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("resetrun"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    {
        let p = open_publisher(&shm, &locks, &path, &n, &e);
        // Prior writer died mid-run: in-progress slot flagged, a stale
        // maintenance wakeup pending, sequence advanced to 11.
        write_flag(
            p.mapping(),
            p.view.ccb_offset,
            offset_of!(ChannelControlBlock, publish_busy),
            true,
        );
        write_flag(
            p.mapping(),
            p.view.ccb_offset,
            offset_of!(ChannelControlBlock, reclaim_requested),
            true,
        );
        write_u64(
            p.mapping(),
            p.view.ccb_offset,
            offset_of!(ChannelControlBlock, write_seq),
            11,
        );
    }
    let p2 = open_publisher(&shm, &locks, &path, &n, &e);
    assert_eq!(p2.epoch, 2, "one acquisition, one epoch advance");
    assert!(
        !read_flag(
            p2.mapping(),
            p2.view.ccb_offset,
            offset_of!(ChannelControlBlock, publish_busy)
        ),
        "in-progress flag cleared on restart"
    );
    assert!(
        !read_flag(
            p2.mapping(),
            p2.view.ccb_offset,
            offset_of!(ChannelControlBlock, reclaim_requested)
        ),
        "stale reclaim wakeup cleared on restart"
    );
    assert_eq!(
        read_u64(
            p2.mapping(),
            p2.view.ccb_offset,
            offset_of!(ChannelControlBlock, first_valid_seq)
        ),
        11,
        "watermark = preserved next sequence"
    );
}

/// REQ: REQ-CORE-018, REQ-CORE-038 — I-L4/I-L5: a consumer attached before a
/// restart observes the epoch change on the shared mapping and the new window
/// floor excludes every prior-run sequence (no cross-epoch confusion).
#[test]
fn consumer_observes_epoch_reset() {
    let shm = MemShm::default();
    let locks = MemLock::default();
    let n = name(&unique("epochreset"));
    let e = expect("pkg/topic-a");
    let path = PathBuf::from("/locks/t42");
    let p = open_publisher(&shm, &locks, &path, &n, &e);
    write_u64(
        p.mapping(),
        p.view.ccb_offset,
        offset_of!(ChannelControlBlock, write_seq),
        9,
    );
    let c: Consumer = fixture(consumer_open(&shm, &n, &e, &spec(), None), "consumer open");
    assert_eq!(
        read_u32(
            c.mapping(),
            c.view.ccb_offset,
            offset_of!(ChannelControlBlock, publisher_epoch)
        ),
        1,
        "fresh run observed as epoch 1"
    );
    drop(p); // simulated publisher death: authority released
    let p2 = open_publisher(&shm, &locks, &path, &n, &e);
    assert_eq!(p2.epoch, 2);
    assert_eq!(
        read_u32(
            c.mapping(),
            c.view.ccb_offset,
            offset_of!(ChannelControlBlock, publisher_epoch)
        ),
        2,
        "consumer mapping observes the epoch change"
    );
    assert_eq!(
        read_u64(
            c.mapping(),
            c.view.ccb_offset,
            offset_of!(ChannelControlBlock, first_valid_seq)
        ),
        9,
        "new window floor excludes prior-run sequences"
    );
}

/// Drains the bounded read loop (`Channel::next`) until `Empty`, asserting every
/// delivery is a `Sample` matching `expected` in order; returns the sample count.
/// REQ: REQ-CORE-013, REQ-CORE-018.
fn drain(c: &Channel, reader: &mut ReaderState, expected: &[[u8; 16]]) -> usize {
    let mut buf = [0_u8; 16];
    let mut n = 0_usize;
    loop {
        match c.next(reader, 0, &mut buf) {
            Ok(ReadOutcome::Sample) => match expected.get(n) {
                Some(want) => assert_eq!(&buf, want, "delivered payload {n}"),
                None => unreachable!("read loop delivered an unexpected extra sample {n}"),
            },
            Ok(ReadOutcome::Empty) => return n,
            other => unreachable!("unexpected read outcome: {other:?}"),
        }
        n += 1;
    }
}

/// REQ: REQ-CORE-018, REQ-CORE-038, REQ-CORE-010, REQ-CORE-019, REQ-CORE-026 —
/// publisher-lifecycle e2e over the in-memory channel read path (no fork, no
/// sleep): a consumer read loop spanning a publisher restart observes the epoch
/// change, resets its cursor to the new window floor, never reads a pre-restart
/// payload as new, and sees prior-run sequences excluded by the floor while the
/// diagnostic counters stay surfaced (I-L4/I-L5/I-L6, REQ-CORE-093).
#[test]
fn consumer_read_loop_observes_publisher_restart() {
    let c = fixture(
        Channel::activate(4, 5, 16, 8, OverflowPolicy::Overwrite, 0, 2, None),
        "activate",
    );
    let mut reader = c.reader();

    // Run 1: a clean read loop drains four ordered samples.
    for v in 0..4_u8 {
        assert_eq!(c.publish(&[v; 16]), Ok(u64::from(v)), "run-1 publish {v}");
    }
    let run1 = [[0_u8; 16], [1; 16], [2; 16], [3; 16]];
    assert_eq!(drain(&c, &mut reader, &run1), 4, "run-1 drained in order");

    // Surface the overrun counter through the read path: the publisher advances
    // past the retained window so the reader laps exactly once (REQ-CORE-093).
    for v in 4..9_u8 {
        assert!(c.publish(&[v; 16]).is_ok(), "run-1 publish {v}");
    }
    let mut buf = [0_u8; 16];
    assert_eq!(
        c.next(&mut reader, 0, &mut buf),
        Ok(ReadOutcome::Gap { missed: 1 }),
        "lapped reader reports an exact gap"
    );
    assert_eq!(
        c.overrun_count(),
        1,
        "read path surfaces the overrun counter"
    );

    // Publisher restart: the epoch advances exactly once and the window floor is
    // the preserved next sequence; prior-run sequences become unaddressable
    // (I-L3/I-L6/I-L7).
    let before = c.epoch();
    assert_eq!(c.restart_run(), before + 1, "epoch advances exactly once");
    assert_eq!(c.epoch(), before + 1, "read path observes the new epoch");
    assert_eq!(c.ccb().first_valid(), 9, "floor = preserved next sequence");
    assert_eq!(
        c.ring().lookup(c.ccb(), 8),
        Err(LookupErr::AgedOut),
        "prior-run sequence excluded by the floor"
    );

    // No run-2 sample yet: the reset cursor must not surface a prior-run payload
    // as new (I-L4/I-L5).
    assert_eq!(
        c.next(&mut reader, 0, &mut buf),
        Ok(ReadOutcome::Empty),
        "prior-run data is never read post-restart"
    );

    // Run 2 under the new epoch: the read loop resets to the floor and delivers
    // the first run-2 sample, never a pre-restart payload as new.
    for v in 40_u8..44_u8 {
        assert!(c.publish(&[v; 16]).is_ok(), "run-2 publish {v}");
    }
    let run2 = [[40_u8; 16], [41; 16], [42; 16], [43; 16]];
    assert_eq!(
        drain(&c, &mut reader, &run2),
        4,
        "run-2 drained from the floor"
    );

    // Diagnostic counters stay surfaced across the restart; the epoch and the
    // lifetime overrun counter are not cleared by restart (I-L7, REQ-CORE-093).
    assert_eq!(c.epoch(), before + 1, "epoch surfaced");
    assert_eq!(c.overrun_count(), 1, "lifetime overrun counter preserved");
    assert_eq!(c.drop_count(), 0, "no drops");
    assert_eq!(c.pool_exhausted(), 0, "no pool exhaustion");
}
