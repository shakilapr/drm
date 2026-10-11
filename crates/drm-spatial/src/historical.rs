//! Historical (time-scoped) forest evaluation
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §The Five
//! Golden Rules rule 5, §Invariants `I-S9`;
//! [spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Functions `reparent`). Every conversion resolves the space graph **at the
//! query's sample timestamp**, never at "now": a topology change (reparent,
//! overlay frame create/delete, bridge registration) takes effect at its
//! effective timestamp `t₁` and leaves history before `t₁` traversing the
//! original hierarchy. History is never rewritten.
//!
//! A [`HistoricalForest`] wraps a base [`SpaceForest`] plus a bounded,
//! sample-time-ordered list of topology events. Validation resolves the graph as
//! of a requested timestamp by replaying only the events at or before it.
//!
//! REQ: REQ-SPATIAL-029, REQ-SPATIAL-010, REQ-SPATIAL-039.

use crate::forest::{PathEdge, SpaceForest};
use crate::types::{SpatialError, Transform3};

/// One topology mutation with its effective timestamp (nanoseconds on the
/// query's clock). Time-scoped plans select the hierarchy as of a query instant
/// (`I-S9`).
/// REQ: REQ-SPATIAL-029.
#[derive(Debug, Clone, PartialEq)]
pub enum TopologyEvent {
    /// A frame gained a static relationship to `parent` at `at_ns`.
    AddStatic {
        /// Effective sample time, nanoseconds.
        at_ns: u64,
        /// Frame identity.
        frame: String,
        /// New parent frame.
        parent: String,
        /// Nominal offset.
        transform: Transform3,
    },
    /// A frame became a dynamic edge at `at_ns`.
    AddDynamic {
        /// Effective sample time, nanoseconds.
        at_ns: u64,
        /// Frame identity.
        frame: String,
        /// Parent frame.
        parent: String,
        /// Numeric dynamic-edge id.
        edge: u32,
    },
    /// A frame was deleted at `at_ns` (`space.delete`, `I-S12`).
    Delete {
        /// Effective sample time, nanoseconds.
        at_ns: u64,
        /// Frame identity.
        frame: String,
    },
    /// A bridge became available at `at_ns` (`REQ-SPATIAL-039`).
    AddBridge {
        /// Effective sample time, nanoseconds.
        at_ns: u64,
        /// Bridge endpoint.
        from: String,
        /// Bridge endpoint.
        to: String,
        /// The `from → to` offset.
        transform: Transform3,
    },
}

impl TopologyEvent {
    /// The event's effective timestamp (`I-S9`).
    /// REQ: REQ-SPATIAL-029.
    #[must_use]
    pub fn at_ns(&self) -> u64 {
        match self {
            Self::AddStatic { at_ns, .. }
            | Self::AddDynamic { at_ns, .. }
            | Self::Delete { at_ns, .. }
            | Self::AddBridge { at_ns, .. } => *at_ns,
        }
    }
}

/// A forest plus a bounded, sample-time-ordered topology-event log. Queries name
/// a timestamp; the graph is reconstructed as of that instant (`I-S9`).
/// REQ: REQ-SPATIAL-029, REQ-SPATIAL-010.
#[derive(Debug, Clone)]
pub struct HistoricalForest {
    base: SpaceForest,
    events: Vec<TopologyEvent>,
}

impl HistoricalForest {
    /// Wraps a base forest with no topology history.
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn new(base: SpaceForest) -> Self {
        Self {
            base,
            events: Vec::new(),
        }
    }

    /// The current (base) forest.
    /// REQ: REQ-SPATIAL-010.
    #[must_use]
    pub fn current(&self) -> &SpaceForest {
        &self.base
    }

    /// Records a topology event. Events need not arrive in order; the log is
    /// kept sample-time-ordered (stable for equal timestamps).
    /// REQ: REQ-SPATIAL-029.
    pub fn record(&mut self, event: TopologyEvent) {
        self.events.push(event);
        self.events.sort_by_key(TopologyEvent::at_ns);
    }

    /// Number of recorded topology events.
    /// REQ: REQ-SPATIAL-029.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Resolves the graph as of `t_ns` by replaying only events with
    /// `at_ns <= t_ns` on top of the base forest. The returned forest is the
    /// historical hierarchy, never the current one (`I-S9`).
    /// REQ: REQ-SPATIAL-029.
    #[must_use]
    pub fn at(&self, t_ns: u64) -> SpaceForest {
        let mut forest = self.base.clone();
        for event in &self.events {
            if event.at_ns() > t_ns {
                break; // events are sample-time-ordered
            }
            match event {
                TopologyEvent::AddStatic {
                    frame,
                    parent,
                    transform,
                    ..
                } => {
                    let _ = forest.add_static(frame.clone(), parent.clone(), *transform);
                }
                TopologyEvent::AddDynamic {
                    frame,
                    parent,
                    edge,
                    ..
                } => forest.add_dynamic(frame.clone(), parent.clone(), *edge),
                TopologyEvent::Delete { frame, .. } => forest.remove(frame),
                TopologyEvent::AddBridge {
                    from,
                    to,
                    transform,
                    ..
                } => forest.add_bridge(from.clone(), to.clone(), *transform),
            }
        }
        forest
    }

    /// Resolves a path as of `t_ns` (`I-S9`): the graph in effect at the
    /// historical sample instant, not the current graph.
    ///
    /// # Errors
    /// [`SpatialError::PathNotFound`] for an unknown/disconnected pair at `t_ns`.
    /// REQ: REQ-SPATIAL-029, REQ-SPATIAL-010, REQ-SPATIAL-039.
    pub fn path_between_at(
        &self,
        reference: &str,
        target: &str,
        t_ns: u64,
    ) -> Result<Vec<PathEdge>, SpatialError> {
        self.at(t_ns).path_between(reference, target)
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoricalForest, TopologyEvent};
    use crate::forest::SpaceForest;
    use crate::types::{SpatialError, Transform3};

    fn t(x: f64, y: f64, z: f64) -> Transform3 {
        Transform3 {
            translation: [x, y, z],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn tree() -> SpaceForest {
        let mut f = SpaceForest::new();
        f.add_root("world".to_string());
        f.add_static("base".to_string(), "world".to_string(), t(1.0, 0.0, 0.0))
            .unwrap_or_else(|e| unreachable!("{e}"));
        f
    }

    /// REQ: REQ-SPATIAL-029 — a frame added at `t₁` is invisible to queries
    /// before `t₁` and visible at/after it (`I-S9`).
    #[test]
    fn historical_topology() {
        let mut h = HistoricalForest::new(tree());
        h.record(TopologyEvent::AddStatic {
            at_ns: 100,
            frame: "camera".to_string(),
            parent: "base".to_string(),
            transform: t(0.0, 0.0, 1.0),
        });
        assert_eq!(
            h.path_between_at("camera", "world", 50),
            Err(SpatialError::PathNotFound)
        );
        assert!(h.path_between_at("camera", "world", 100).is_ok());
    }

    /// REQ: REQ-SPATIAL-029 — a deleted frame still resolves in history before
    /// its deletion instant; deletion is never retroactive (`I-S9`).
    #[test]
    fn delete_preserves_history() {
        let mut base = tree();
        base.add_static("camera".to_string(), "base".to_string(), t(0.0, 0.0, 1.0))
            .unwrap_or_else(|e| unreachable!("{e}"));
        let mut h = HistoricalForest::new(base);
        h.record(TopologyEvent::Delete {
            at_ns: 200,
            frame: "camera".to_string(),
        });
        assert!(h.path_between_at("camera", "world", 199).is_ok());
        assert_eq!(
            h.path_between_at("camera", "world", 200),
            Err(SpatialError::PathNotFound)
        );
    }

    /// REQ: REQ-SPATIAL-029 — one event recorded out of order replays in
    /// sample-time order.
    #[test]
    fn events_replay_in_order() {
        let mut h = HistoricalForest::new(tree());
        h.record(TopologyEvent::Delete {
            at_ns: 300,
            frame: "camera".to_string(),
        });
        h.record(TopologyEvent::AddStatic {
            at_ns: 100,
            frame: "camera".to_string(),
            parent: "base".to_string(),
            transform: t(0.0, 0.0, 1.0),
        });
        assert_eq!(h.event_count(), 2);
        assert!(h.path_between_at("camera", "world", 150).is_ok());
        assert_eq!(
            h.path_between_at("camera", "world", 350),
            Err(SpatialError::PathNotFound)
        );
    }
}
