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
