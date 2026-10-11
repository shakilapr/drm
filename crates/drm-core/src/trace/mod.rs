//! Wait-free per-producer SPSC trace lanes and the build-time metadata
//! dictionary (`docs/algorithms/ops/observability-plane.md` §Data Structures
//! `TraceRingSet`/`TraceLane`/`TraceEvent`, §Functions `trace_emit`,
//! `trace_register_thread`, §Protocols compiled trace pipeline).
//!
//! The unsafe lock-free ring lives here (runtime layer) rather than in the ops
//! crate `drm-trace`, because codebase.md §2.2 confines `unsafe` to
//! `drm-core`/`drm-ffi`; `drm-trace` composes this primitive safely.
//! REQ: REQ-OPS-014 (numeric IDs + build-time dict), REQ-OPS-015 (never stalls
//! producers, best-effort drop), REQ-OPS-017 (fixed per-producer SPSC lanes).

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use drm_types::{Status, blake3_128};

/// Fixed argument payload size of one [`TraceEvent`] (build-time property).
/// REQ: REQ-OPS-014.
pub const TRACE_ARG_BYTES: usize = 16;

/// Metadata-dictionary wire format version (append-only mapping contract).
/// REQ: REQ-OPS-014.
pub const TRACE_DICT_FORMAT_VERSION: u16 = 1;

/// One emitted diagnostic event: numeric id, fixed-size args, monotonic stamp.
/// No global order counter — diag merges by `(ts_ns, lane_id, lane_seq)`
/// (`observability-plane.md` §Data Structures).
/// REQ: REQ-OPS-014, REQ-OPS-017.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct TraceEvent {
    /// Numeric event id indexing the build-time metadata dictionary.
    pub id: u16,
    /// Reserved alignment padding to keep the payload naturally aligned.
    pub pad: u16,
    /// Fixed-size argument payload (size enforced at build; `REQ-OPS-014`).
    pub args: [u8; TRACE_ARG_BYTES],
    /// Monotonic emission stamp (nanoseconds).
    pub ts_ns: u64,
}

const _: () = assert!(std::mem::size_of::<TraceEvent>() == 32);

/// Single-producer/single-consumer bounded ring of `T`.
///
/// The producer executes exactly one bounded `try_push` — no CAS retry loop,
/// no shared producer state (`I-OB1`). Capacity is fixed at construction.
/// REQ: REQ-OPS-015.
pub struct FixedSpscRing<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    head: AtomicU64,
    tail: AtomicU64,
    capacity: u64,
}

// SAFETY: the ring is a classic SPSC queue: the producer writes `tail` and only
// reads `head`; the consumer writes `head` and only reads `tail`. The slot
// touched by the producer is never concurrently touched by the consumer
// because a full/empty check gates every access (Acquire/Release pairing).
unsafe impl<T: Send> Send for FixedSpscRing<T> {}
// SAFETY: see the `Send` impl — interior mutation is serialized by the SPSC
// head/tail protocol.
unsafe impl<T: Send> Sync for FixedSpscRing<T> {}

impl<T> FixedSpscRing<T> {
    /// Allocates a ring of `capacity` slots (activation-time allocation only).
    /// REQ: REQ-OPS-015.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `capacity` is zero.
    pub fn new(capacity: u64) -> Result<Self, Status> {
        if capacity == 0 {
            return Err(Status::InvalidConfig);
        }
        let n = usize::try_from(capacity).map_err(|_| Status::InvalidConfig)?;
        let slots = (0..n)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            slots,
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            capacity,
        })
    }

    /// Ring capacity.
    /// REQ: REQ-OPS-015.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Bounded, wait-free push. Returns the value back when the ring is full
    /// (caller counts the drop; never blocks, never retries).
    /// REQ: REQ-OPS-015.
    ///
    /// # Errors
    /// `Err(value)` when the ring is full.
    pub fn try_push(&self, value: T) -> Result<(), T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= self.capacity {
            return Err(value);
        }
        let idx = usize::try_from(tail % self.capacity).unwrap_or(0);
        // SAFETY: the SPSC protocol guarantees the consumer is not reading this
        // slot (the full check passed), so the write is race-free.
        unsafe { (*self.slots[idx].get()).write(value) };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Bounded, wait-free pop; `None` when empty.
    /// REQ: REQ-OPS-015.
    pub fn try_pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let idx = usize::try_from(head % self.capacity).unwrap_or(0);
        // SAFETY: the producer published this slot before advancing `tail`
        // (Release) and cannot touch it again until `head` advances.
        let value = unsafe { (*self.slots[idx].get()).assume_init_read() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

/// One producer lane: a bounded event ring plus its drop counter.
/// REQ: REQ-OPS-015, REQ-OPS-017.
pub struct TraceLane {
    ring: FixedSpscRing<TraceEvent>,
    dropped: AtomicU64,
}

impl TraceLane {
    /// Lane ring capacity.
    /// REQ: REQ-OPS-015.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.ring.capacity()
    }

    /// Best-effort emit: full ring ⇒ drop + `dropped` counter (never blocks).
    /// REQ: REQ-OPS-015.
    pub fn emit(&self, event: TraceEvent) {
        if self.ring.try_push(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Drop counter for this lane (`I-OB6`).
    /// REQ: REQ-OPS-015.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Drains up to `out.len()` retained events (diag/reader side).
    /// REQ: REQ-OPS-017.
    pub fn drain_into(&self, out: &mut Vec<TraceEvent>) {
        while let Some(ev) = self.ring.try_pop() {
            out.push(ev);
        }
    }
}

/// A fixed set of producer lanes claimed per thread, plus the set-level
/// no-lane drop counter. `N` is fixed at activation; no dynamic growth.
/// REQ: REQ-OPS-015, REQ-OPS-017.
pub struct TraceRingSet {
    lanes: Box<[TraceLane]>,
    claimed: Box<[AtomicBool]>,
    no_lane_drops: AtomicU64,
    dict_version: DictVersion,
}

// SAFETY: every mutable field is either an atomic or a lane guarded by the SPSC
// protocol; claiming uses an atomic CAS, so the set is shareable across threads.
unsafe impl Send for TraceRingSet {}
// SAFETY: see `Send`; concurrent `&TraceRingSet` use only touches atomics and
// one lane per producer.
unsafe impl Sync for TraceRingSet {}

impl TraceRingSet {
    /// Activates a set of `lanes` SPSC lanes of `lane_capacity` events each.
    /// REQ: REQ-OPS-015, REQ-OPS-017.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `lanes` is zero or `lane_capacity` is zero.
    pub fn new(
        lanes: usize,
        lane_capacity: u64,
        dict_version: DictVersion,
    ) -> Result<Self, Status> {
        if lanes == 0 || lane_capacity == 0 {
            return Err(Status::InvalidConfig);
        }
        let mut built = Vec::with_capacity(lanes);
        for _ in 0..lanes {
            built.push(TraceLane {
                ring: FixedSpscRing::new(lane_capacity)?,
                dropped: AtomicU64::new(0),
            });
        }
        let claimed = (0..lanes)
            .map(|_| AtomicBool::new(false))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            lanes: built.into_boxed_slice(),
            claimed,
            no_lane_drops: AtomicU64::new(0),
            dict_version,
        })
    }

    /// Number of lanes.
    /// REQ: REQ-OPS-017.
    #[must_use]
    pub fn lane_count(&self) -> usize {
        self.lanes.len()
    }

    /// Dictionary version bound at activation (`observability-plane.md`
    /// §Dictionary versioning).
    /// REQ: REQ-OPS-014.
    #[must_use]
    pub fn dict_version(&self) -> DictVersion {
        self.dict_version
    }

    /// Claims a free lane; `None` when every lane is taken (no growth).
    /// REQ: REQ-OPS-017.
    pub fn claim_free_lane(&self) -> Option<usize> {
        for (i, flag) in self.claimed.iter().enumerate() {
            if flag
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(i);
            }
        }
        None
    }

    /// Releases a claimed lane.
    /// REQ: REQ-OPS-017.
    pub fn release_lane(&self, lane: usize) {
        if let Some(flag) = self.claimed.get(lane) {
            flag.store(false, Ordering::Release);
        }
    }

    /// Lane view (diag reader side).
    /// REQ: REQ-OPS-017.
    #[must_use]
    pub fn lane(&self, lane: usize) -> Option<&TraceLane> {
        self.lanes.get(lane)
    }

    /// Best-effort emit directly on a lane (no TLS); unregistered lane ⇒
    /// `no_lane_drops` increment (`I-OB1`).
    /// REQ: REQ-OPS-015, REQ-OPS-017.
    pub fn emit_on(&self, lane: Option<usize>, event: TraceEvent) {
        match lane.and_then(|i| self.lanes.get(i)) {
            Some(l) => l.emit(event),
            None => {
                self.no_lane_drops.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Set-level no-lane drop counter (`I-OB6`).
    /// REQ: REQ-OPS-015.
    #[must_use]
    pub fn no_lane_drops(&self) -> u64 {
        self.no_lane_drops.load(Ordering::Relaxed)
    }

    /// Total drops across every lane plus the set-level counter (`I-OB6`).
    /// REQ: REQ-OPS-015.
    #[must_use]
    pub fn total_dropped(&self) -> u64 {
        self.lanes.iter().map(TraceLane::dropped).sum::<u64>() + self.no_lane_drops()
    }
}

/// Build-time metadata dictionary: numeric id → (event name, format string).
/// Append-only within a deployment (`observability-plane.md` §Dictionary
/// versioning). REQ: REQ-OPS-014.
#[derive(Debug, Clone, Default)]
pub struct TraceDict {
    entries: Vec<DictEntry>,
}

/// One dictionary entry.
/// REQ: REQ-OPS-014.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictEntry {
    /// Event name.
    pub name: String,
    /// Format string; `{}` placeholders consume successive `u32` args.
    pub format: String,
}

impl TraceDict {
    /// Empty dictionary.
    /// REQ: REQ-OPS-014.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an event (idempotent for an identical name+format pair); returns
    /// the numeric id. Append-only: existing ids never change.
    /// REQ: REQ-OPS-014.
    pub fn intern(&mut self, name: impl Into<String>, format: impl Into<String>) -> u16 {
        let entry = DictEntry {
            name: name.into(),
            format: format.into(),
        };
        if let Some(i) = self.entries.iter().position(|e| *e == entry) {
            return u16::try_from(i).unwrap_or(u16::MAX);
        }
        let id = u16::try_from(self.entries.len()).unwrap_or(u16::MAX);
        self.entries.push(entry);
        id
    }

    /// Entry for `id`.
    /// REQ: REQ-OPS-014.
    #[must_use]
    pub fn entry(&self, id: u16) -> Option<&DictEntry> {
        self.entries.get(usize::from(id))
    }

    /// Renders an event to text; an unknown id renders raw hex plus a
    /// version-mismatch marker (never a wrong name, `I-OB6`).
    /// REQ: REQ-OPS-014.
    #[must_use]
    pub fn render(&self, event: &TraceEvent) -> String {
        match self.entry(event.id) {
            Some(e) => {
                let mut out = String::with_capacity(e.name.len() + e.format.len() + 16);
                out.push_str(&e.name);
                out.push_str(": ");
                let mut rest = e.format.as_str();
                let mut arg = 0_usize;
                while let Some(pos) = rest.find("{}") {
                    out.push_str(&rest[..pos]);
                    out.push_str(&format_arg(&event.args, arg));
                    rest = &rest[pos + 2..];
                    arg += 1;
                }
                out.push_str(rest);
                out
            }
            None => format!(
                "0x{:04x} (dict version mismatch): {}",
                event.id,
                hex_args(&event.args)
            ),
        }
    }

    /// `dict_version = (format_version, BLAKE3(dict bytes)[..16])`.
    /// REQ: REQ-OPS-014.
    #[must_use]
    pub fn version(&self) -> DictVersion {
        let mut bytes = Vec::new();
        for e in &self.entries {
            bytes.extend_from_slice(e.name.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(e.format.as_bytes());
            bytes.push(0);
        }
        DictVersion {
            format_version: TRACE_DICT_FORMAT_VERSION,
            digest: blake3_128(&bytes),
        }
    }
}

/// Dictionary version: format version + truncated BLAKE3 digest of the bytes.
/// REQ: REQ-OPS-014.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DictVersion {
    /// Wire format version.
    pub format_version: u16,
    /// 128-bit truncated digest of the dictionary bytes.
    pub digest: [u8; 16],
}

impl DictVersion {
    /// Unknown/empty version sentinel.
    /// REQ: REQ-OPS-014.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            format_version: 0,
            digest: [0; 16],
        }
    }
}

/// Renders one 4-byte little-endian `u32` argument slot as decimal.
/// REQ: REQ-OPS-014.
fn format_arg(args: &[u8; TRACE_ARG_BYTES], index: usize) -> String {
    let off = index * 4;
    let mut word = [0_u8; 4];
    if let Some(slice) = args.get(off..off + 4) {
        word.copy_from_slice(slice);
    }
    u32::from_le_bytes(word).to_string()
}

/// Renders the 16-byte arg payload as spaced hex bytes.
/// REQ: REQ-OPS-014.
fn hex_args(args: &[u8; TRACE_ARG_BYTES]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(args.len() * 3);
    for (i, b) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{b:02x}");
    }
    out
}

thread_local! {
    /// Per-thread lane binding (lazy for normal threads; explicit for RT).
    static TRACE_LANE: Cell<Option<LaneBinding>> = const { Cell::new(None) };
}

/// Raw per-thread lane binding; valid only while the owning `TraceRingSet`
/// outlives the thread (runtime contract).
#[derive(Clone, Copy)]
struct LaneBinding {
    set: *const TraceRingSet,
    lane: usize,
}

/// Claims a lane for the calling thread and binds it in TLS. RT threads call
/// this before entering a hot loop (lane pre-faulted); ordinary threads claim
/// lazily via [`trace_emit`]. REQ: REQ-OPS-017.
///
/// # Errors
/// [`Status::Busy`] when every lane is claimed (no dynamic growth).
pub fn trace_register_thread(set: &TraceRingSet) -> Result<usize, Status> {
    let lane = set.claim_free_lane().ok_or(Status::Busy)?;
    TRACE_LANE.with(|slot| {
        slot.set(Some(LaneBinding {
            set: std::ptr::from_ref(set),
            lane,
        }));
    });
    Ok(lane)
}

/// Releases the calling thread's lane binding (and frees the lane).
/// REQ: REQ-OPS-017.
pub fn trace_unregister_thread() {
    TRACE_LANE.with(|slot| {
        if let Some(binding) = slot.take() {
            // SAFETY: the binding was installed by `trace_register_thread` for
            // this thread; the set is still alive per the runtime contract.
            let set = unsafe { &*binding.set };
            set.release_lane(binding.lane);
        }
    });
}

/// Wait-free producer emit (exactly one bounded `try_push`, `I-OB1`): uses the
/// calling thread's TLS lane; an unregistered thread drops and counts via
/// `no_lane_drops`; a full lane drops and counts via the lane counter.
/// REQ: REQ-OPS-015, REQ-OPS-017.
pub fn trace_emit(set: &TraceRingSet, id: u16, args: [u8; TRACE_ARG_BYTES], ts_ns: u64) {
    let event = TraceEvent {
        id,
        pad: 0,
        args,
        ts_ns,
    };
    let lane = TRACE_LANE.with(|slot| {
        slot.get().and_then(|binding| {
            // SAFETY: the binding points at a live set (runtime contract); emit
            // only reads the lane array.
            let bound = unsafe { &*binding.set };
            std::ptr::eq(bound, set).then_some(binding.lane)
        })
    });
    set.emit_on(lane, event);
}

/// Process-monotonic nanosecond clock for [`trace_emit_now`].
/// REQ: REQ-OPS-015.
#[must_use]
pub fn mono_now_ns() -> u64 {
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let base = BASE.get_or_init(Instant::now);
    u64::try_from(base.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Convenience emit stamping the event with [`mono_now_ns`].
/// REQ: REQ-OPS-015.
pub fn trace_emit_now(set: &TraceRingSet, id: u16, args: [u8; TRACE_ARG_BYTES]) {
    trace_emit(set, id, args, mono_now_ns());
}
