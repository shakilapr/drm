//! Per-peer egress: classify each item into its [`PriorityClass`] and enqueue
//! it into the peer's [`LaneSocket`], then service dequeues in strict class
//! order with FIFO inside a class (I-LS2, I-LS6). This adds only classification
//! on top of the existing [`LaneSocket`] — caps, lifecycle, and the
//! realtime-never-behind-bulk separation are unchanged (REQ-ROUTE-012), and no
//! item is reordered across classes (REQ-ROUTE-020).
//! REQ: REQ-ROUTE-013, REQ-ROUTE-020, REQ-ROUTE-021.

use drm_types::Status;

use crate::lane::{DisconnectReport, LaneCaps, LaneSocket, LaneState};
use crate::priority::PriorityClass;
use crate::transport::Reliability;

/// An item that names its own service [`PriorityClass`], so the per-peer egress
/// classifies it without a separate lookup table. The underlying
/// [`LaneSocket`] then enforces FIFO inside a class (I-LS2) and strict class
/// service order (REQ-ROUTE-020).
/// REQ: REQ-ROUTE-021.
pub trait Classified {
    /// Service class of this item (safety → control → status → diagnostics).
    /// REQ: REQ-ROUTE-020.
    fn class(&self) -> PriorityClass;
}

/// One peer's egress endpoint: a [`LaneSocket`] plus classification of each
/// item by its [`Classified::class`] before enqueue. Dequeues are served in
/// strict class order (REQ-ROUTE-020) with FIFO inside a class (I-LS2); the
/// separate bulk lane keeps realtime traffic from queuing behind bulk
/// (REQ-ROUTE-012). The [`LaneSocket`] is reused unchanged, so its caps and
/// connect/disconnect lifecycle behave exactly as before (REQ-ROUTE-013).
/// REQ: REQ-ROUTE-013, REQ-ROUTE-020, REQ-ROUTE-021.
#[derive(Debug, Clone)]
pub struct PeerEgress<T> {
    socket: LaneSocket<T>,
}

impl<T> PeerEgress<T> {
    /// Creates a disconnected peer egress with static caps and the bulk lane's
    /// reliability. Empty queues; nothing is created on connect.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn new(caps: LaneCaps, bulk_reliability: Reliability) -> Self {
        Self {
            socket: LaneSocket::new(caps, bulk_reliability),
        }
    }

    /// Current connect state of this peer's lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn state(&self) -> LaneState {
        self.socket.state()
    }

    /// Connects the peer: flips the state to [`LaneState::Connected`].
    /// Idempotent; queued items are untouched.
    /// REQ: REQ-ROUTE-013.
    pub fn connect(&mut self) {
        self.socket.connect();
    }

    /// Disconnects the peer and reports the disposition of every queued item
    /// (realtime surfaced as ended in service order, bulk per its reliability).
    /// Idempotent: a second disconnect reports empty.
    /// REQ: REQ-ROUTE-013.
    pub fn disconnect(&mut self) -> DisconnectReport<T> {
        self.socket.disconnect()
    }

    /// Items queued across both lanes.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn len(&self) -> usize {
        self.socket.len()
    }

    /// Whether both lanes hold nothing.
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.socket.is_empty()
    }

    /// Items queued in `class`'s lane (realtime class or bulk).
    /// REQ: REQ-ROUTE-013.
    #[must_use]
    pub fn class_len(&self, class: PriorityClass) -> usize {
        self.socket.class_len(class)
    }
}

impl<T: Classified> PeerEgress<T> {
    /// Classifies `item` by [`Classified::class`] and enqueues it at the tail
    /// of that class's FIFO, so priority is applied before entry into the
    /// peer's lanes (REQ-ROUTE-021).
    /// REQ: REQ-ROUTE-021, REQ-ROUTE-012.
    ///
    /// # Errors
    /// [`Status::NotReady`] when the peer is disconnected;
    /// [`Status::QueueFull`] when the target lane is at its static cap — a
    /// realtime class never silently drops (I-LS5) and the reliable bulk lane
    /// applies backpressure.
    pub fn send(&mut self, item: T) -> Result<(), Status> {
        let class = item.class();
        self.socket.enqueue(class, item)
    }

    /// Services the next item: the head of the highest non-empty realtime
    /// class in strict class order, or the bulk lane when every realtime class
    /// is empty (FIFO inside each class); nothing is served while disconnected.
    /// REQ: REQ-ROUTE-020, REQ-ROUTE-012.
    pub fn service(&mut self) -> Option<(PriorityClass, T)> {
        self.socket.dequeue()
    }
}
