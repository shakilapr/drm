//! Per-peer ordered lanes (lanes-shaping.md §Data Structures, work-plan
//! Phase 30 seed). Each peer owns one realtime lane and one bulk lane
//! (REQ-ROUTE-013); realtime traffic can never queue behind bulk
//! (REQ-ROUTE-012). Within the realtime lane the classes keep strict service
//! order with FIFO inside a class (REQ-ROUTE-020/021, I-LS2). Caps are static
//! (I-LS5): realtime classes never silently drop — overflow is an explicit
//! [`Status::QueueFull`] — and the reliable ordered bulk lane applies
//! publisher backpressure, never a silent drop.
//! REQ: REQ-ROUTE-013, REQ-ROUTE-012, REQ-ROUTE-020, REQ-ROUTE-021.

use std::collections::VecDeque;

use drm_types::Status;

use crate::priority::PriorityClass;
use crate::transport::Reliability;

/// Number of realtime classes sharing the realtime lane (safety, control,
/// status); diagnostics/bulk is the bulk lane.
pub const REALTIME_CLASSES: usize = 3;

/// Static per-lane capacities (lanes-shaping.md `LaneCaps`; from deployment
/// generation, never resized on the data path). `realtime[i]` is the cap for
/// the `i`-th realtime class in service order; `bulk` is the bulk-lane cap.
/// REQ: REQ-ROUTE-013.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneCaps {
    /// Per-realtime-class caps, indexed by [`PriorityClass::index`] (0..3).
    pub realtime: [usize; 3],
    /// Bulk-lane cap.
    pub bulk: usize,
}

impl LaneCaps {
    /// Constructs caps from a realtime triple and a bulk cap.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub const fn new(realtime: [usize; 3], bulk: usize) -> Self {
        Self { realtime, bulk }
    }

    /// Static cap for `class` (realtime triple or bulk).
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn cap_for(self, class: PriorityClass) -> usize {
        match realtime_index(class) {
            Some(idx) => self.realtime[idx],
            None => self.bulk,
        }
    }
}

/// Index of `class` within the realtime lane, or `None` for the bulk class.
/// REQ: REQ-ROUTE-012.
#[must_use]
pub fn realtime_index(class: PriorityClass) -> Option<usize> {
    match class {
        PriorityClass::Safety | PriorityClass::Control | PriorityClass::Status => {
            Some(class.index())
        }
        PriorityClass::Diagnostics => None,
    }
}

/// One peer's ordered lanes: a class-scheduled realtime lane plus a separate
/// bulk lane, so latency-critical reliable traffic never queues behind bulk
/// (REQ-ROUTE-012). FIFO is preserved within every class (I-LS2) and the
/// realtime classes are served in strict class order (REQ-ROUTE-020).
/// REQ: REQ-ROUTE-013, REQ-ROUTE-012.
#[derive(Debug, Clone)]
pub struct PeerLanes<T> {
    realtime: [VecDeque<T>; REALTIME_CLASSES],
    bulk: VecDeque<T>,
    caps: LaneCaps,
}

impl<T> PeerLanes<T> {
    /// Creates empty per-peer lanes with static caps.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn new(caps: LaneCaps) -> Self {
        Self {
            realtime: std::array::from_fn(|_| VecDeque::new()),
            bulk: VecDeque::new(),
            caps,
        }
    }

    /// Static caps for this peer's lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn caps(&self) -> LaneCaps {
        self.caps
    }

    /// Enqueues `item` at the tail of `class`'s FIFO (I-LS2).
    /// REQ: REQ-ROUTE-021, REQ-ROUTE-012.
    ///
    /// # Errors
    /// [`Status::QueueFull`] when the target lane is at its static cap — a
    /// realtime class never silently drops (I-LS5) and the reliable ordered
    /// bulk lane applies backpressure, never a silent drop.
    pub fn enqueue(&mut self, class: PriorityClass, item: T) -> Result<(), Status> {
        if let Some(idx) = realtime_index(class) {
            let lane = &mut self.realtime[idx];
            if lane.len() >= self.caps.realtime[idx] {
                return Err(Status::QueueFull);
            }
            lane.push_back(item);
        } else {
            if self.bulk.len() >= self.caps.bulk {
                return Err(Status::QueueFull);
            }
            self.bulk.push_back(item);
        }
        Ok(())
    }

    /// Dequeues the head of the highest non-empty realtime class (strict class
    /// order, then FIFO); bulk drains only when every realtime class is empty
    /// (I-LS1/I-LS6). REQ: REQ-ROUTE-020, REQ-ROUTE-012.
    pub fn dequeue(&mut self) -> Option<(PriorityClass, T)> {
        for class in [
            PriorityClass::Safety,
            PriorityClass::Control,
            PriorityClass::Status,
        ] {
            if let Some(item) = self.realtime[class.index()].pop_front() {
                return Some((class, item));
            }
        }
        self.bulk
            .pop_front()
            .map(|item| (PriorityClass::Diagnostics, item))
    }

    /// Items queued across both lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn len(&self) -> usize {
        self.realtime.iter().map(VecDeque::len).sum::<usize>() + self.bulk.len()
    }

    /// Whether both lanes hold nothing.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bulk.is_empty() && self.realtime.iter().all(VecDeque::is_empty)
    }

    /// Items queued in `class`'s lane (realtime class or bulk).
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn class_len(&self, class: PriorityClass) -> usize {
        match realtime_index(class) {
            Some(idx) => self.realtime[idx].len(),
            None => self.bulk.len(),
        }
    }
}

/// Per-peer lane socket state (lanes-shaping.md §Data Structures `PeerLanes`).
/// A peer is either connected or not; connecting and draining are explicit, so
/// no queue is created or dropped implicitly on a state flip.
/// REQ: REQ-ROUTE-013.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneState {
    /// Lanes accept and serve traffic.
    Connected,
    /// Lanes are quiescent; a disconnect has already reported queued items.
    Disconnected,
}

/// Disposition of the bulk lane's queued items when a peer disconnects
/// (I-LS5 reliability policy). Best-effort bulk drops and counts; reliable
/// bulk surfaces the items as ended, because a gone peer can no longer absorb
/// publisher backpressure.
/// REQ: REQ-ROUTE-013.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkDisposition<T> {
    /// Best-effort bulk: `n` items dropped and counted (I-LS5).
    Dropped {
        /// Number of items dropped.
        n: u64,
    },
    /// Reliable bulk: the items surfaced as ended (peer gone).
    Ended(Vec<T>),
}

/// What a disconnect did to a peer's queued items. Realtime classes never
/// silently drop (I-LS5); with the peer gone they are surfaced as ended in
/// class-then-FIFO service order, so the caller can observe the loss.
/// REQ: REQ-ROUTE-013, REQ-ROUTE-020.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectReport<T> {
    /// Realtime items surfaced as ended, in service order.
    pub realtime_ended: Vec<(PriorityClass, T)>,
    /// Bulk lane disposition per its reliability (see [`BulkDisposition`]).
    pub bulk: BulkDisposition<T>,
}

/// Per-peer lane socket: the existing [`PeerLanes`] plus an explicit
/// connect/disconnect lifecycle (REQ-ROUTE-013). Pure in-memory state — no
/// networking; only reliable ordered streams use lanes (REQ-ROUTE-034). FIFO
/// within a class (I-LS2) and the static caps (I-LS5) are unchanged while
/// connected, because every data-path call delegates to the same lanes.
/// REQ: REQ-ROUTE-013, REQ-ROUTE-012, REQ-ROUTE-034.
#[derive(Debug, Clone)]
pub struct LaneSocket<T> {
    lanes: PeerLanes<T>,
    state: LaneState,
    bulk_reliability: Reliability,
    bulk_dropped: u64,
}

impl<T> LaneSocket<T> {
    /// Creates a per-peer lane socket in the [`LaneState::Disconnected`] state
    /// with static caps and the bulk lane's reliability. Empty queues; nothing
    /// is created on connect.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn new(caps: LaneCaps, bulk_reliability: Reliability) -> Self {
        Self {
            lanes: PeerLanes::new(caps),
            state: LaneState::Disconnected,
            bulk_reliability,
            bulk_dropped: 0,
        }
    }

    /// Current connect state of this peer's lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn state(&self) -> LaneState {
        self.state
    }

    /// Static caps of the underlying lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn caps(&self) -> LaneCaps {
        self.lanes.caps()
    }

    /// Reliability class of the bulk lane (best-effort or reliable).
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn bulk_reliability(&self) -> Reliability {
        self.bulk_reliability
    }

    /// Bulk items dropped by the best-effort disconnect policy so far (I-LS5).
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn bulk_dropped(&self) -> u64 {
        self.bulk_dropped
    }

    /// Connects the peer: flips the state to [`LaneState::Connected`].
    /// Creating and draining are explicit — queues are untouched, so connecting
    /// neither allocates nor discards a queued item (idempotent).
    /// REQ: REQ-ROUTE-013.
    pub fn connect(&mut self) {
        self.state = LaneState::Connected;
    }

    /// Disconnects the peer and reports the disposition of every queued item.
    /// Realtime items are surfaced as ended in service order (never silently
    /// dropped, I-LS5); the bulk lane is dropped-and-counted (best-effort) or
    /// surfaced as ended (reliable) per its reliability. Idempotent: a second
    /// disconnect reports empty.
    /// REQ: REQ-ROUTE-013, REQ-ROUTE-020.
    pub fn disconnect(&mut self) -> DisconnectReport<T> {
        self.state = LaneState::Disconnected;
        let mut realtime_ended = Vec::new();
        let mut bulk_items = Vec::new();
        while let Some((class, item)) = self.lanes.dequeue() {
            if realtime_index(class).is_some() {
                realtime_ended.push((class, item));
            } else {
                bulk_items.push(item);
            }
        }
        let bulk = match self.bulk_reliability {
            Reliability::BestEffort => {
                let n = u64::try_from(bulk_items.len()).unwrap_or(u64::MAX);
                self.bulk_dropped = self.bulk_dropped.saturating_add(n);
                BulkDisposition::Dropped { n }
            }
            Reliability::Reliable => BulkDisposition::Ended(bulk_items),
        };
        DisconnectReport {
            realtime_ended,
            bulk,
        }
    }

    /// Enqueues `item` at the tail of `class`'s FIFO while connected
    /// (I-LS2). Enqueue is refused while disconnected, so a queued item is
    /// never silently accepted for a gone peer.
    /// REQ: REQ-ROUTE-021, REQ-ROUTE-012.
    ///
    /// # Errors
    /// [`Status::NotReady`] when the peer is disconnected;
    /// [`Status::QueueFull`] when the target lane is at its static cap — a
    /// realtime class never silently drops (I-LS5) and the reliable ordered
    /// bulk lane applies backpressure.
    pub fn enqueue(&mut self, class: PriorityClass, item: T) -> Result<(), Status> {
        if self.state == LaneState::Disconnected {
            return Err(Status::NotReady);
        }
        self.lanes.enqueue(class, item)
    }

    /// Dequeues the head of the highest non-empty realtime class (strict class
    /// order, then FIFO); bulk drains only when realtime is empty. Nothing is
    /// served while disconnected.
    /// REQ: REQ-ROUTE-020, REQ-ROUTE-012.
    pub fn dequeue(&mut self) -> Option<(PriorityClass, T)> {
        if self.state == LaneState::Disconnected {
            return None;
        }
        self.lanes.dequeue()
    }

    /// Items queued across both lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lanes.len()
    }

    /// Whether both lanes hold nothing.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// Items queued in `class`'s lane (realtime class or bulk).
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn class_len(&self, class: PriorityClass) -> usize {
        self.lanes.class_len(class)
    }
}
