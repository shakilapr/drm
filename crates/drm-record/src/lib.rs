//! Record-core ingress (`docs/algorithms/ops/record-replay.md`): the
//! borrow-copy-release pipeline (`I-RC1`), recorder-owned bounded per-topic slab
//! buffers (`I-RC2`/`I-RC3`), and the per-topic loss ledger (`I-RC5`).
//!
//! Layer `ops` (codebase.md 2.1): depends on `drm-types` + `drm-core` only,
//! never on another ops crate. `#![forbid(unsafe_code)]`. The disk stage
//! ([`disk`]) drains the descriptor ring into an MCAP stream ([`mcap`]) that
//! observes recorder-owned memory exclusively (`I-RC1`); [`replay`] reads a
//! recorded session back, reconstructs per-topic streams and gap boundaries,
//! and republishes under an isolated replay namespace (`I-RC4`/`I-RC6`).
//! REQ: REQ-OPS-005 (borrow-copy-release), REQ-OPS-006 (MCAP framing),
//! REQ-OPS-007 (no disk on borrow), REQ-OPS-008 (`on_full` policies),
//! REQ-OPS-009 (replay fidelity), REQ-OPS-018 (gap ledger),
//! REQ-OPS-019 (per-topic slabs).

#![forbid(unsafe_code)]

pub mod disk;
pub mod mcap;
pub mod replay;

use std::collections::VecDeque;

use drm_core::channel::{Channel, ReadOutcome, ReaderState};
use drm_types::Status;

/// Policy applied when a topic's slab pool is exhausted
/// (`record-replay.md` §Data Structures `OnFull`).
/// REQ: REQ-OPS-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFull {
    /// Discard the sample; coalesce its sequence into the topic `GapLedger`.
    Drop,
    /// Abort the session with `RECORDING_FAILED reason: BUFFER_EXHAUSTED`.
    Stop,
    /// Pause that topic's ingress; the owned slot is retained, no loss.
    Backpressure,
}

/// Reason a sequence range is absent from a topic's recorded stream
/// (`record-replay.md` §Data Structures `GapReason`).
/// REQ: REQ-OPS-018.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapReason {
    /// The publisher overwrote the range before ingress read it.
    HistoryLap,
    /// The `Drop` policy discarded the range at a full slab pool.
    PolicyDrop,
    /// A publisher restart left a range unrecorded (epoch boundary).
    PublisherEpoch,
}

impl GapReason {
    /// Stable lowercase tag recorded in MCAP `DrmGap` markers.
    /// REQ: REQ-OPS-006.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HistoryLap => "history_lap",
            Self::PolicyDrop => "policy_drop",
            Self::PublisherEpoch => "publisher_epoch",
        }
    }
}

/// One coalesced missing-sequence range (`record-replay.md` §Data Structures
/// `PendingGap`).
/// REQ: REQ-OPS-018.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingGap {
    /// First missing sequence in the range.
    pub first_seq: u64,
    /// Last missing sequence in the range (inclusive).
    pub last_seq: u64,
    /// Why the range is missing.
    pub reason: GapReason,
}

/// Per-topic preallocated loss ledger (`record-replay.md` §Data Structures).
/// A separate exhaustion domain from payload slabs (`I-RC5`): ledger overflow
/// merges conservatively and reports `gap_detail_lost`; it never touches slab
/// storage or performs I/O.
/// REQ: REQ-OPS-018.
#[derive(Debug, Clone)]
pub struct GapLedger {
    entries: Vec<Option<PendingGap>>,
    detail_lost: bool,
}

impl GapLedger {
    /// Builds a fixed-capacity ledger.
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: vec![None; capacity],
            detail_lost: false,
        }
    }

    /// Coalesces one missing sequence into the ledger, extending the newest
    /// matching contiguous range or opening a new entry (`I-RC5`).
    /// REQ: REQ-OPS-018.
    pub fn coalesce(&mut self, seq: u64, reason: GapReason) {
        self.push_range(seq, seq, reason);
    }

    /// Coalesces an inclusive missing range `[first, last]` (`I-RC5`).
    /// REQ: REQ-OPS-018.
    pub fn coalesce_range(&mut self, first: u64, last: u64, reason: GapReason) {
        self.push_range(first, last, reason);
    }

    fn newest_index(&self) -> Option<usize> {
        (0..self.entries.len()).rfind(|&i| self.entries[i].is_some())
    }

    fn push_range(&mut self, first: u64, last: u64, reason: GapReason) {
        if let Some(i) = self.newest_index()
            && let Some(g) = self.entries[i].as_mut()
            && g.reason == reason
            && first <= g.last_seq.saturating_add(1)
        {
            g.last_seq = g.last_seq.max(last);
            return;
        }
        if let Some(i) = self.entries.iter().position(Option::is_none) {
            self.entries[i] = Some(PendingGap {
                first_seq: first,
                last_seq: last,
                reason,
            });
            return;
        }
        self.detail_lost = true;
        if let Some(i) = self.newest_index()
            && let Some(g) = self.entries[i].as_mut()
        {
            g.last_seq = g.last_seq.max(last);
        }
    }

    /// Number of live coalesced ranges.
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn live_ranges(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    /// Fixed entry capacity.
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Whether overflow forced a conservative merge (`gap_detail_lost`).
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn gap_detail_lost(&self) -> bool {
        self.detail_lost
    }

    /// Live ranges in coalescing order.
    pub fn iter(&self) -> impl Iterator<Item = &PendingGap> {
        self.entries.iter().flatten()
    }
}

/// One recorder-owned slab slot (`record-replay.md` §Data Structures
/// `TopicSlot`). `payload` is preallocated at construction; ingress copies into
/// it and never grows it (`I-RC2`).
/// REQ: REQ-OPS-019.
#[derive(Debug)]
struct Slot {
    in_use: bool,
    len: u32,
    payload: Vec<u8>,
}

/// Per-topic recorder-owned slab pool (`record-replay.md` §Data Structures
/// `TopicSlab`), sized to that topic's payload bound so a 128-byte IMU sample
/// never pays an 8 MiB camera slot (`I-RC2`). Storage is fixed at construction;
/// `alloc`/`free` touch only bookkeeping (`I-RC1`, zero data-path allocation).
/// REQ: REQ-OPS-019.
#[derive(Debug)]
pub struct TopicSlab {
    slot_bytes: usize,
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl TopicSlab {
    /// Preallocates `slot_count` slots of `slot_bytes` each.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn new(slot_bytes: usize, slot_count: usize) -> Self {
        let slots = (0..slot_count)
            .map(|_| Slot {
                in_use: false,
                len: 0,
                payload: vec![0_u8; slot_bytes],
            })
            .collect();
        let free = (0..slot_count)
            .rev()
            .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
            .collect();
        Self {
            slot_bytes,
            slots,
            free,
        }
    }

    fn alloc(&mut self) -> Option<u32> {
        let idx = self.free.pop()?;
        if let Some(slot) = self.slots.get_mut(usize::try_from(idx).ok()?) {
            slot.in_use = true;
            slot.len = 0;
        }
        Some(idx)
    }

    fn free_slot(&mut self, idx: u32) {
        let Ok(i) = usize::try_from(idx) else {
            return;
        };
        if let Some(slot) = self.slots.get_mut(i)
            && slot.in_use
        {
            slot.in_use = false;
            slot.len = 0;
            self.free.push(idx);
        }
    }

    fn payload_mut(&mut self, idx: u32) -> Option<&mut [u8]> {
        let i = usize::try_from(idx).ok()?;
        self.slots.get_mut(i).map(|s| s.payload.as_mut_slice())
    }

    fn set_len(&mut self, idx: u32, len: u32) {
        if let Ok(i) = usize::try_from(idx)
            && let Some(slot) = self.slots.get_mut(i)
        {
            slot.len = len;
        }
    }

    /// Configured per-slot payload bound, in bytes.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn slot_bytes(&self) -> usize {
        self.slot_bytes
    }

    /// Number of preallocated slots.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Number of free slots.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn free_count(&self) -> usize {
        self.free.len()
    }

    /// Recorded payload of slot `idx` (its declared length), or `None`.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn payload(&self, idx: u32) -> Option<&[u8]> {
        let i = usize::try_from(idx).ok()?;
        let slot = self.slots.get(i)?;
        if !slot.in_use {
            return None;
        }
        slot.payload.get(..usize::try_from(slot.len).ok()?)
    }
}

/// Per-topic recorder storage sizing (`record-replay.md` §Data Structures).
/// `slot_bytes` is that topic's schema payload bound.
/// REQ: REQ-OPS-019.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicSpec {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// Per-slot payload bound for this topic (bytes).
    pub slot_bytes: usize,
    /// Preallocated slot count.
    pub slot_count: usize,
    /// Gap-ledger entry capacity (separate exhaustion domain).
    pub ledger_capacity: usize,
}

/// Recorder session configuration (`record-replay.md` §Data Structures
/// `RecorderConfig`).
/// REQ: REQ-OPS-008, REQ-OPS-019.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecorderConfig {
    /// Recorder-owned budget: Σ per-topic slab pools + descriptor ring + gap
    /// ledgers, in bytes (`I-RC2`).
    pub buffer_bytes: u64,
    /// Policy applied when a topic's slab pool is exhausted.
    pub on_full: OnFull,
    /// MCAP chunk target in bytes; consumed by the M15W3 disk writer.
    pub chunk_target: u64,
    /// Fixed descriptor-ring capacity; must be ≥ the total slab slot count so
    /// every allocated slot has a guaranteed descriptor slot (`I-RC2`).
    pub desc_capacity: usize,
}

/// Fixed descriptor for one recorded sample, queued in global ingress order
/// (`record-replay.md` §Data Structures `DescEntry`).
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescEntry {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// Slab slot holding the copied payload.
    pub slab_index: u32,
    /// Copied payload length in bytes.
    pub len: u32,
    /// Publisher sequence at copy completion.
    pub seq: u64,
    /// Publisher epoch observed at copy completion (`I-RC6`).
    pub publisher_epoch: u32,
    /// Local monotonic receive time at copy completion.
    pub recv_mono_ns: u64,
    /// Wall-clock receive time (human indexing only).
    pub recv_wall_ns: u64,
    /// Recorder-observed global ingress order (`I-RC4c`).
    pub ingress_order: u64,
}

/// Why recording stopped (`record-replay.md` §Protocols).
/// REQ: REQ-OPS-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// `Stop` policy hit a full slab pool: `RECORDING_FAILED reason: BUFFER_EXHAUSTED`.
    BufferExhausted,
}

/// Outcome of one [`Recorder::ingress_step`].
/// REQ: REQ-OPS-005, REQ-OPS-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressStep {
    /// One sample copied into a slab slot and queued in the descriptor ring.
    Recorded,
    /// `Drop` policy discarded the sample; its sequence landed in the ledger.
    PolicyDropped,
    /// `Backpressure` policy paused ingress at a full slab pool; no loss.
    Paused,
    /// Nothing published, or the reader is fully caught up.
    Empty,
    /// A history lap was coalesced into the ledger (`I-RC5`).
    Gap,
    /// `Stop` policy aborted the session (`I-RC3`).
    Aborted,
}

struct TopicState {
    topic_id: u32,
    slab: TopicSlab,
    ledger: GapLedger,
    state: Option<ReaderState>,
    scratch: Vec<u8>,
    epoch: Option<u32>,
    next_seq: Option<u64>,
    last_seq: Option<u64>,
}

/// Recorder core: per-topic slab pools, one global descriptor ring, and
/// per-topic gap ledgers. All memory is fixed at construction (`I-RC2`);
/// steady-state ingress performs no heap allocation and holds a DRM borrow only
/// for the copy into a slab slot (`I-RC1`).
/// REQ: REQ-OPS-005, REQ-OPS-007, REQ-OPS-008, REQ-OPS-019.
pub struct Recorder {
    cfg: RecorderConfig,
    topics: Vec<TopicState>,
    descriptors: VecDeque<DescEntry>,
    global_order: u64,
    active_borrows: u64,
    max_active_borrows: u64,
    aborted: Option<StopReason>,
    disk_ops: u64,
}

impl Recorder {
    /// Exact minimum `buffer_bytes` for `specs` and `desc_capacity` — the
    /// recorder-owned budget (`I-RC2`).
    /// REQ: REQ-OPS-019.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] on arithmetic overflow.
    pub fn required_bytes(specs: &[TopicSpec], desc_capacity: usize) -> Result<u64, Status> {
        let mut total: u64 = 0;
        for spec in specs {
            let slab = u64::try_from(
                spec.slot_bytes
                    .checked_mul(spec.slot_count)
                    .ok_or(Status::InvalidConfig)?,
            )
            .map_err(|_| Status::InvalidConfig)?;
            let ledger = u64::try_from(
                spec.ledger_capacity
                    .checked_mul(size_of::<PendingGap>())
                    .ok_or(Status::InvalidConfig)?,
            )
            .map_err(|_| Status::InvalidConfig)?;
            total = total
                .checked_add(slab)
                .and_then(|t| t.checked_add(ledger))
                .ok_or(Status::InvalidConfig)?;
        }
        let descs = u64::try_from(
            desc_capacity
                .checked_mul(size_of::<DescEntry>())
                .ok_or(Status::InvalidConfig)?,
        )
        .map_err(|_| Status::InvalidConfig)?;
        total.checked_add(descs).ok_or(Status::InvalidConfig)
    }

    /// Builds a recorder: preallocates every slab and ledger and verifies the
    /// byte budget. Storage is fixed thereafter (`I-RC2`).
    /// REQ: REQ-OPS-008, REQ-OPS-019.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `specs` is empty, a slot/ledger count is
    /// zero, `desc_capacity < total slots`, or the budget is under
    /// [`Recorder::required_bytes`].
    pub fn new(cfg: RecorderConfig, specs: Vec<TopicSpec>) -> Result<Self, Status> {
        if specs.is_empty() || cfg.desc_capacity == 0 {
            return Err(Status::InvalidConfig);
        }
        let total_slots = specs.iter().try_fold(0usize, |acc, s| {
            if s.slot_count == 0 || s.ledger_capacity == 0 || s.slot_bytes == 0 {
                return None;
            }
            acc.checked_add(s.slot_count)
        });
        let total_slots = total_slots.ok_or(Status::InvalidConfig)?;
        if cfg.desc_capacity < total_slots {
            return Err(Status::InvalidConfig);
        }
        let need = Self::required_bytes(&specs, cfg.desc_capacity)?;
        if need > cfg.buffer_bytes {
            return Err(Status::InvalidConfig);
        }
        let topics = specs
            .into_iter()
            .map(|s| TopicState {
                topic_id: s.topic_id,
                slab: TopicSlab::new(s.slot_bytes, s.slot_count),
                ledger: GapLedger::new(s.ledger_capacity),
                state: None,
                scratch: vec![0_u8; s.slot_bytes],
                epoch: None,
                next_seq: None,
                last_seq: None,
            })
            .collect();
        Ok(Self {
            cfg,
            topics,
            descriptors: VecDeque::with_capacity(cfg.desc_capacity),
            global_order: 0,
            active_borrows: 0,
            max_active_borrows: 0,
            aborted: None,
            disk_ops: 0,
        })
    }

    /// One ingress step for `topic_index`: borrow → copy into a per-topic slab
    /// slot → release (`I-RC1`), then queue a descriptor in global ingress
    /// order. Full slab ⇒ the configured `on_full` policy (`I-RC3`).
    /// REQ: REQ-OPS-005, REQ-OPS-007, REQ-OPS-008, REQ-OPS-018.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] for an unknown topic or record index;
    /// [`Status::InvalidConfig`] on a slab/cursor invariant violation;
    /// channel read errors propagate from [`Channel::next`].
    pub fn ingress_step(
        &mut self,
        topic_index: usize,
        ch: &Channel,
        rec_index: usize,
        now_mono_ns: u64,
        now_wall_ns: u64,
    ) -> Result<IngressStep, Status> {
        if self.aborted.is_some() {
            return Ok(IngressStep::Aborted);
        }
        let Self {
            cfg,
            topics,
            descriptors,
            global_order,
            active_borrows,
            max_active_borrows,
            aborted,
            ..
        } = self;
        let on_full = cfg.on_full;
        let ts = topics.get_mut(topic_index).ok_or(Status::InvalidArg)?;

        // Epoch observe/reset (cursor-delivery I-C6): a publisher restart
        // between reads coalesces the unrecorded boundary and resets the cursor.
        let epoch = ch.epoch();
        if ts.epoch != Some(epoch) {
            if let Some(last) = ts.last_seq {
                ts.ledger
                    .coalesce(last.saturating_add(1), GapReason::PublisherEpoch);
            }
            ts.epoch = Some(epoch);
            ts.state = Some(ch.reader());
            ts.next_seq = None;
            ts.last_seq = None;
        }
        if ts.next_seq.is_none() {
            let ws = ch.write_seq();
            ts.next_seq = Some(ch.oldest_retained(ws).max(ch.ccb().first_valid()));
        }
        let state = ts.state.get_or_insert_with(|| ch.reader());

        let full = ts.slab.free_count() == 0;
        if full {
            match on_full {
                OnFull::Backpressure => return Ok(IngressStep::Paused),
                OnFull::Stop => {
                    *aborted = Some(StopReason::BufferExhausted);
                    return Ok(IngressStep::Aborted);
                }
                OnFull::Drop => {}
            }
        }
        let slot = if full { None } else { ts.slab.alloc() };
        let target: &mut [u8] = match slot {
            Some(idx) => ts.slab.payload_mut(idx).ok_or(Status::InvalidConfig)?,
            None => ts.scratch.as_mut_slice(),
        };

        // I-RC1 instrumented borrow window: the DRM borrow lives only across the
        // copy inside `Channel::next`; it is released before any policy/disk work.
        *active_borrows = 1;
        *max_active_borrows = (*max_active_borrows).max(1);
        let outcome = ch.next(state, rec_index, target);
        *active_borrows = 0;
        let outcome = outcome?;

        match outcome {
            ReadOutcome::Sample => {
                let seq = ts.next_seq.unwrap_or(0);
                ts.next_seq = Some(seq.saturating_add(1));
                ts.last_seq = Some(seq);
                let bound = ch
                    .sample_max()
                    .min(u64::try_from(target.len()).map_err(|_| Status::InvalidConfig)?);
                let len = u32::try_from(bound).map_err(|_| Status::InvalidConfig)?;
                if let Some(idx) = slot {
                    ts.slab.set_len(idx, len);
                    let entry = DescEntry {
                        topic_id: ts.topic_id,
                        slab_index: idx,
                        len,
                        seq,
                        publisher_epoch: ts.epoch.unwrap_or(0),
                        recv_mono_ns: now_mono_ns,
                        recv_wall_ns: now_wall_ns,
                        ingress_order: *global_order,
                    };
                    *global_order = global_order.saturating_add(1);
                    descriptors.push_back(entry);
                    Ok(IngressStep::Recorded)
                } else {
                    // I-RC3 Drop: the sample is discarded; only the ledger grows.
                    ts.ledger.coalesce(seq, GapReason::PolicyDrop);
                    Ok(IngressStep::PolicyDropped)
                }
            }
            ReadOutcome::Gap { missed } => {
                let first = ts.next_seq.unwrap_or(0);
                let last = first.saturating_add(missed.saturating_sub(1));
                ts.next_seq = Some(first.saturating_add(missed));
                ts.last_seq = Some(last);
                ts.ledger.coalesce_range(first, last, GapReason::HistoryLap);
                Ok(IngressStep::Gap)
            }
            ReadOutcome::Empty | ReadOutcome::WouldRetry => Ok(IngressStep::Empty),
        }
    }

    /// Drains one queued descriptor and frees its slab slot — recorder-owned
    /// memory only; no DRM handle is touched (`I-RC1`, `REQ-OPS-007`).
    /// REQ: REQ-OPS-007, REQ-OPS-019.
    #[must_use]
    pub fn drain_one(&mut self) -> Option<DescEntry> {
        let desc = self.pop_descriptor()?;
        self.free_descriptor(&desc);
        Some(desc)
    }

    /// Pops the oldest queued descriptor without freeing its slab slot, so the
    /// disk stage can copy the recorder-owned payload first (`I-RC1`).
    /// REQ: REQ-OPS-007.
    #[must_use]
    pub fn pop_descriptor(&mut self) -> Option<DescEntry> {
        self.descriptors.pop_front()
    }

    /// Borrows the recorder-owned payload of `desc` (never a DRM handle; `I-RC1`).
    /// REQ: REQ-OPS-007.
    #[must_use]
    pub fn slot_payload(&self, desc: &DescEntry) -> Option<&[u8]> {
        let ts = self.topics.iter().find(|t| t.topic_id == desc.topic_id)?;
        ts.slab.payload(desc.slab_index)
    }

    /// Frees the slab slot held by `desc` after the disk stage has copied it;
    /// touches recorder memory only (`I-RC1`).
    /// REQ: REQ-OPS-007.
    pub fn free_descriptor(&mut self, desc: &DescEntry) {
        if let Some(ts) = self.topics.iter_mut().find(|t| t.topic_id == desc.topic_id) {
            ts.slab.free_slot(desc.slab_index);
        }
        self.disk_ops = self.disk_ops.saturating_add(1);
    }

    /// Coalesced gaps across every topic, tagged with topic id and the observed
    /// publisher epoch (`I-RC5`, `I-RC6`).
    /// REQ: REQ-OPS-006, REQ-OPS-018.
    pub fn gaps(&self) -> impl Iterator<Item = (u32, u32, PendingGap)> + '_ {
        self.topics.iter().flat_map(|t| {
            let epoch = t.epoch.unwrap_or(0);
            t.ledger
                .iter()
                .copied()
                .map(move |g| (t.topic_id, epoch, g))
        })
    }

    /// Whether any topic's gap ledger overflowed (`gap_detail_lost`; `I-RC5`).
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn gap_detail_lost(&self) -> bool {
        self.topics.iter().any(|t| t.ledger.gap_detail_lost())
    }

    /// Exact recorder-owned bytes currently accounted (`I-RC2`).
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        let specs: Vec<TopicSpec> = self
            .topics
            .iter()
            .map(|t| TopicSpec {
                topic_id: t.topic_id,
                slot_bytes: t.slab.slot_bytes(),
                slot_count: t.slab.capacity(),
                ledger_capacity: t.ledger.capacity(),
            })
            .collect();
        Self::required_bytes(&specs, self.cfg.desc_capacity).unwrap_or(u64::MAX)
    }

    /// Number of configured topics.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn topic_count(&self) -> usize {
        self.topics.len()
    }

    /// Slab pool for `topic_index`.
    /// REQ: REQ-OPS-019.
    #[must_use]
    pub fn slab(&self, topic_index: usize) -> Option<&TopicSlab> {
        self.topics.get(topic_index).map(|t| &t.slab)
    }

    /// Gap ledger for `topic_index`.
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn ledger(&self, topic_index: usize) -> Option<&GapLedger> {
        self.topics.get(topic_index).map(|t| &t.ledger)
    }

    /// Descriptors currently queued for the disk stage.
    /// REQ: REQ-OPS-007.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.descriptors.len()
    }

    /// DRM borrows currently held (0 outside an ingress copy; `I-RC1`).
    /// REQ: REQ-OPS-005.
    #[must_use]
    pub fn active_borrows(&self) -> u64 {
        self.active_borrows
    }

    /// Peak concurrent DRM borrows (never above 1; `I-RC1`).
    /// REQ: REQ-OPS-005.
    #[must_use]
    pub fn max_active_borrows(&self) -> u64 {
        self.max_active_borrows
    }

    /// Completed disk-stage drains (descriptor pop + slot free).
    /// REQ: REQ-OPS-007.
    #[must_use]
    pub fn disk_ops(&self) -> u64 {
        self.disk_ops
    }

    /// Whether the `Stop` policy aborted the session.
    /// REQ: REQ-OPS-008.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.aborted.is_some()
    }

    /// Abort reason once `Stop` fires.
    /// REQ: REQ-OPS-008.
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.aborted
    }

    /// Configured policy.
    /// REQ: REQ-OPS-008.
    #[must_use]
    pub fn on_full(&self) -> OnFull {
        self.cfg.on_full
    }

    /// Configured MCAP chunk target, in bytes.
    /// REQ: REQ-OPS-006.
    #[must_use]
    pub fn chunk_target(&self) -> u64 {
        self.cfg.chunk_target
    }
}

#[cfg(test)]
mod tests;
