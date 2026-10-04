//! Pre-stream priority class queues (lanes-shaping.md §Data Structures
//! priority table; skeleton for the Phase 30 link scheduler, work-plan
//! M03W3-E3-T1). Service order is strict class order (safety → control →
//! status → diagnostics) with FIFO within a class. Priority applies only
//! inside router queues and never reorders an ordered stream.
//! REQ: REQ-ROUTE-020, REQ-ROUTE-021.

use std::collections::VecDeque;

use drm_types::Status;

/// Number of pre-stream classes.
pub const CLASS_COUNT: usize = 4;

/// Scheduler service class (0 = safety … 3 = diagnostics/bulk).
/// REQ: REQ-ROUTE-020.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriorityClass {
    /// Estop, safety interlocks — strict first.
    Safety,
    /// Actuator/command traffic.
    Control,
    /// State, battery, telemetry.
    Status,
    /// Traces, logs, metrics, bulk data.
    Diagnostics,
}

impl PriorityClass {
    /// All classes in service order.
    pub const ALL: [Self; CLASS_COUNT] =
        [Self::Safety, Self::Control, Self::Status, Self::Diagnostics];

    /// Class index in service order.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Safety => 0,
            Self::Control => 1,
            Self::Status => 2,
            Self::Diagnostics => 3,
        }
    }
}

/// Per-lane pre-stream queue: one bounded FIFO per class.
/// REQ: REQ-ROUTE-021.
#[derive(Debug, Clone)]
pub struct PriorityQueue<T> {
    buckets: [VecDeque<T>; CLASS_COUNT],
    cap_per_class: usize,
}

impl<T> PriorityQueue<T> {
    /// Creates a queue with a static per-class cap (activation-time sizing).
    #[must_use]
    pub fn new(cap_per_class: usize) -> Self {
        Self {
            buckets: std::array::from_fn(|_| VecDeque::new()),
            cap_per_class,
        }
    }

    /// Enqueues at the tail of `class`'s FIFO.
    /// REQ: REQ-ROUTE-021.
    ///
    /// # Errors
    /// [`Status::QueueFull`] when the class bucket is at its static cap.
    pub fn enqueue(&mut self, class: PriorityClass, item: T) -> Result<(), Status> {
        let bucket = &mut self.buckets[class.index()];
        if bucket.len() >= self.cap_per_class {
            return Err(Status::QueueFull);
        }
        bucket.push_back(item);
        Ok(())
    }

    /// Pops the head of the highest non-empty class (class order, then FIFO).
    /// REQ: REQ-ROUTE-020.
    pub fn pop(&mut self) -> Option<(PriorityClass, T)> {
        for class in PriorityClass::ALL {
            if let Some(item) = self.buckets[class.index()].pop_front() {
                return Some((class, item));
            }
        }
        None
    }

    /// Total queued items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.iter().map(VecDeque::len).sum()
    }

    /// Whether no class holds an item.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buckets.iter().all(VecDeque::is_empty)
    }

    /// Items queued in `class`.
    #[must_use]
    pub fn class_len(&self, class: PriorityClass) -> usize {
        self.buckets[class.index()].len()
    }
}

#[cfg(test)]
mod tests {
    use super::{PriorityClass, PriorityQueue};
    use drm_types::Status;

    /// REQ: REQ-ROUTE-020 — interleaved enqueue drains in exact class order,
    /// FIFO within each class.
    #[test]
    fn priority_order() {
        let mut q = PriorityQueue::new(8);
        let enq = |q: &mut PriorityQueue<&str>, c, v| {
            q.enqueue(c, v).unwrap_or_else(|e| unreachable!("{e:?}"));
        };
        enq(&mut q, PriorityClass::Diagnostics, "d1");
        enq(&mut q, PriorityClass::Control, "c1");
        enq(&mut q, PriorityClass::Safety, "s1");
        enq(&mut q, PriorityClass::Status, "st1");
        enq(&mut q, PriorityClass::Control, "c2");
        enq(&mut q, PriorityClass::Safety, "s2");
        enq(&mut q, PriorityClass::Diagnostics, "d2");

        let mut order = Vec::new();
        while let Some((class, v)) = q.pop() {
            order.push((class, v));
        }
        assert_eq!(
            order,
            vec![
                (PriorityClass::Safety, "s1"),
                (PriorityClass::Safety, "s2"),
                (PriorityClass::Control, "c1"),
                (PriorityClass::Control, "c2"),
                (PriorityClass::Status, "st1"),
                (PriorityClass::Diagnostics, "d1"),
                (PriorityClass::Diagnostics, "d2"),
            ]
        );
        assert!(q.is_empty());
    }

    /// REQ: REQ-ROUTE-021 — an empty queue pops nothing; each class keeps its
    /// own FIFO order.
    #[test]
    fn priority_fifo_and_empty() {
        let mut q: PriorityQueue<u32> = PriorityQueue::new(2);
        assert_eq!(q.pop(), None);
        assert!(q.is_empty());
        assert_eq!(q.enqueue(PriorityClass::Status, 1), Ok(()));
        assert_eq!(q.enqueue(PriorityClass::Status, 2), Ok(()));
        assert_eq!(q.enqueue(PriorityClass::Status, 3), Err(Status::QueueFull));
        assert_eq!(q.class_len(PriorityClass::Status), 2);
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some((PriorityClass::Status, 1)));
        assert_eq!(q.pop(), Some((PriorityClass::Status, 2)));
        assert_eq!(q.pop(), None);
    }
}
