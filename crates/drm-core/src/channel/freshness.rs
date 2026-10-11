//! Consumer-local freshness observation (publisher-lifecycle.md §Data
//! Structures `FreshnessTracker`, §Functions `freshness`; I-L9): monotonic
//! arrival time drives `Active` → `Stale` → `Offline`, and an epoch change
//! immediately yields `NoCurrentSample` until a complete new-epoch read. Source
//! timestamps never drive freshness; clock regression saturates age at zero.
//! REQ: REQ-CORE-027, REQ-CORE-018.

use std::time::{Duration, Instant};

use drm_types::Status;

/// Observed stream state (publisher-lifecycle.md `StreamState`).
/// REQ: REQ-CORE-027.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    /// No sample observed for the current epoch (fresh segment, or an epoch
    /// change observed before any new-epoch read).
    NoCurrentSample,
    /// A sample arrived within `stale_after`.
    Active,
    /// Age at or beyond `stale_after` and below `offline_after`.
    Stale,
    /// Age at or beyond `offline_after`.
    Offline,
}

/// Consumer-local freshness tracker (publisher-lifecycle.md §Data Structures).
/// Freshness is process-local and uses the consumer's monotonic arrival clock;
/// a publisher restart (epoch change) clears the arrival time (I-L9).
/// REQ: REQ-CORE-027.
#[derive(Debug, Clone)]
pub struct FreshnessTracker {
    epoch: u32,
    state: StreamState,
    last_arrival: Option<Instant>,
    stale_after: Duration,
    offline_after: Duration,
}

impl FreshnessTracker {
    /// Creates a tracker for `epoch`, requiring `stale_after < offline_after`
    /// (publisher-lifecycle.md §Protocols *Freshness transitions* step 1).
    /// REQ: REQ-CORE-027.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `stale_after >= offline_after`.
    pub fn new(epoch: u32, stale_after: Duration, offline_after: Duration) -> Result<Self, Status> {
        if stale_after >= offline_after {
            return Err(Status::InvalidConfig);
        }
        Ok(Self {
            epoch,
            state: StreamState::NoCurrentSample,
            last_arrival: None,
            stale_after,
            offline_after,
        })
    }

    /// Current observed state. REQ: REQ-CORE-027.
    #[must_use]
    pub fn state(&self) -> StreamState {
        self.state
    }

    /// Epoch last observed by this tracker. REQ: REQ-CORE-027.
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Records a completed read at `now`: enters `Active` and stamps the
    /// consumer-local arrival time (publisher-lifecycle.md `observe_sample`).
    /// REQ: REQ-CORE-027.
    pub fn observe_sample(&mut self, epoch: u32, now: Instant) {
        self.epoch = epoch;
        self.last_arrival = Some(now);
        self.state = StreamState::Active;
    }

    /// Advances the state for `epoch` at `now` (publisher-lifecycle.md
    /// `freshness`). An epoch change clears the arrival time and yields
    /// `NoCurrentSample`; otherwise the age selects `Active`/`Stale`/`Offline`.
    /// REQ: REQ-CORE-027, REQ-CORE-018.
    pub fn freshness(&mut self, epoch: u32, now: Instant) -> StreamState {
        if epoch != self.epoch {
            self.epoch = epoch;
            self.last_arrival = None;
            self.state = StreamState::NoCurrentSample;
            return self.state;
        }
        let Some(last) = self.last_arrival else {
            return StreamState::NoCurrentSample;
        };
        let age = now.saturating_duration_since(last);
        self.state = if age >= self.offline_after {
            StreamState::Offline
        } else if age >= self.stale_after {
            StreamState::Stale
        } else {
            StreamState::Active
        };
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::{FreshnessTracker, StreamState};
    use std::time::{Duration, Instant};

    /// Tracker with `stale_after = 10 ms`, `offline_after = 20 ms`.
    /// REQ: REQ-CORE-027.
    fn tracker() -> FreshnessTracker {
        FreshnessTracker::new(1, Duration::from_millis(10), Duration::from_millis(20))
            .unwrap_or_else(|e| unreachable!("tracker: {e:?}"))
    }

    /// REQ: REQ-CORE-027 — I-L9: threshold−1 is `Active`, threshold is `Stale`,
    /// `offline_after` is `Offline`; a clock regression saturates age at zero.
    #[test]
    fn freshness_boundaries() {
        let t0 = Instant::now();
        let mut f = tracker();
        f.observe_sample(1, t0);
        assert_eq!(f.freshness(1, t0), StreamState::Active);
        assert_eq!(
            f.freshness(1, t0 + Duration::from_millis(9)),
            StreamState::Active
        );
        assert_eq!(
            f.freshness(1, t0 + Duration::from_millis(10)),
            StreamState::Stale
        );
        assert_eq!(
            f.freshness(1, t0 + Duration::from_millis(20)),
            StreamState::Offline
        );
        // Clock regression: `now` before the arrival saturates age at zero.
        assert_eq!(f.freshness(1, t0), StreamState::Active);
    }

    /// REQ: REQ-CORE-027, REQ-CORE-018 — an epoch change yields
    /// `NoCurrentSample` before any new-epoch read; the arrival time is cleared.
    #[test]
    fn epoch_clears_freshness() {
        let t0 = Instant::now();
        let mut f = tracker();
        f.observe_sample(1, t0);
        assert_eq!(f.freshness(2, t0), StreamState::NoCurrentSample);
        assert_eq!(
            f.freshness(2, t0 + Duration::from_secs(1)),
            StreamState::NoCurrentSample
        );
        f.observe_sample(2, t0 + Duration::from_secs(1));
        assert_eq!(
            f.freshness(2, t0 + Duration::from_secs(1)),
            StreamState::Active
        );
    }

    /// REQ: REQ-CORE-027 — `stale_after < offline_after` is validated at
    /// construction.
    #[test]
    fn rejects_inverted_thresholds() {
        assert!(
            FreshnessTracker::new(1, Duration::from_millis(20), Duration::from_millis(10)).is_err()
        );
        assert!(
            FreshnessTracker::new(1, Duration::from_millis(10), Duration::from_millis(10)).is_err()
        );
    }
}
