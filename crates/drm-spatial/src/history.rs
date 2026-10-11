//! Time-based transform history rings
//! ([spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Data Structures `TimeHistory`, §Invariants `I-Q4`/`I-Q13`;
//! [spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
//! §Protocols history allocation). A [`TimeHistory`] is allocated only on an
//! upstream dynamic edge with declared consumer requirements. It is ordered by
//! **sample time**, bounded by `max_samples = N`, guarantees a sample-time
//! queryability window `history = H`, and enforces the producer-density contract
//! `min_sample_period = P`: two accepted samples are never closer than `P` in
//! sample time, so `N >= floor(H/P) + 2` always holds (`I-Q13`). In-window late
//! arrivals insert in order; older-than-floor samples are dropped and counted;
//! duplicate sample times keep the newer receive time.
//!
//! `at(t)` selects the exact sample or interpolates linearly in translation and
//! via SLERP in rotation between the bracketing samples; it **never silently
//! extrapolates** beyond the newest sample (`I-Q4`). Interpolation never spans a
//! discontinuity: a bracketing pair whose newer sample is flagged discontinuous
//! refuses interpolation and surfaces [`SpatialError::DiscontinuityNearby`] with
//! no cross-jump value (`I-Q7`).
//!
//! REQ: REQ-SPATIAL-033, REQ-SPATIAL-037, REQ-SPATIAL-022, REQ-SPATIAL-016.

use crate::cache::FLAG_DISCONTINUOUS;
use crate::clock::TimePoint;
use crate::types::{SpatialError, Transform3, UnitQuat, Vector3};

/// One time-history entry: a transform stamped with sample/receive
/// [`TimePoint`]s and per-sample flags (the discontinuous boundary marker used
/// by the interpolation guard, `I-Q7`).
/// REQ: REQ-SPATIAL-033, REQ-SPATIAL-016.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistorySample {
    /// The transform.
    pub value: Transform3,
    /// The instant the pose was true (`I-S6`).
    pub sample: TimePoint,
    /// The instant DRM received the sample (`I-S6`).
    pub receive: TimePoint,
    /// Per-sample flag bits ([`FLAG_DISCONTINUOUS`]).
    pub flags: u32,
}

impl HistorySample {
    /// Builds a flagged history sample.
    /// REQ: REQ-SPATIAL-033.
    #[must_use]
    pub const fn new(value: Transform3, sample: TimePoint, receive: TimePoint, flags: u32) -> Self {
        Self {
            value,
            sample,
            receive,
            flags,
        }
    }
}

/// Outcome of ingesting one sample into a [`TimeHistory`] (`REQ-SPATIAL-037`).
/// REQ: REQ-SPATIAL-037.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingest {
    /// The sample was appended at the newest end.
    Appended,
    /// The sample arrived in-window and was inserted in sample-time order.
    Inserted,
    /// The sample duplicates a retained sample time; the newer receive time won.
    Duplicate,
    /// The sample was closer than `P` to the previously accepted sample; rejected
    /// and counted (`DensityViolation`, `I-Q13`).
    DensityViolation,
    /// The sample was older than the retained floor; rejected and counted
    /// (`LateSampleDropped`, `REQ-SPATIAL-037`).
    LateDropped,
}

/// A sample-time-ordered transform history with a guaranteed window `H`, a fixed
/// capacity `N`, and a producer-density contract `P` (`REQ-SPATIAL-037`).
/// REQ: REQ-SPATIAL-033, REQ-SPATIAL-037.
#[derive(Debug, Clone)]
pub struct TimeHistory {
    entries: Vec<HistorySample>,
    capacity: usize,
    window_ns: u64,
    min_period_ns: u64,
    density_violations: u64,
    late_dropped: u64,
}

impl TimeHistory {
    /// Allocates a history with `capacity = N` samples, a guaranteed window
    /// `window_ns = H`, and a producer-density contract `min_period_ns = P`.
    ///
    /// # Errors
    /// [`SpatialError::InvalidConfig`] when `capacity < floor(H/P) + 2` (an
    /// unsatisfiable declaration is rejected, never silently shortened, `I-Q13`),
    /// when `P == 0`, or when `H`/`P` is otherwise unusable.
    /// REQ: REQ-SPATIAL-037.
    pub fn new(capacity: u32, window_ns: u64, min_period_ns: u64) -> Result<Self, SpatialError> {
        if min_period_ns == 0 {
            return Err(SpatialError::InvalidConfig);
        }
        let required = window_ns / min_period_ns + 2;
        if u64::from(capacity) < required {
            return Err(SpatialError::InvalidConfig);
        }
        let capacity = usize::try_from(capacity).map_err(|_| SpatialError::InvalidConfig)?;
        Ok(Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            window_ns,
            min_period_ns,
            density_violations: 0,
            late_dropped: 0,
        })
    }

    /// Number of retained samples.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no samples are retained.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The fixed capacity `N`.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The guaranteed sample-time window `H`, nanoseconds.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn window_ns(&self) -> u64 {
        self.window_ns
    }

    /// The producer-density contract `P`, nanoseconds.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn min_period_ns(&self) -> u64 {
        self.min_period_ns
    }

    /// The `DensityViolation` counter (`I-Q13`).
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn density_violations(&self) -> u64 {
        self.density_violations
    }

    /// The `LateSampleDropped` counter (`REQ-SPATIAL-037`).
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn late_dropped(&self) -> u64 {
        self.late_dropped
    }

    /// The retained samples in sample-time order.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn samples(&self) -> &[HistorySample] {
        &self.entries
    }

    /// The newest retained sample time, if any.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn newest_ns(&self) -> Option<u64> {
        self.entries.last().map(|s| s.sample.value_ns)
    }

    /// The oldest retained sample time, if any.
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn oldest_ns(&self) -> Option<u64> {
        self.entries.first().map(|s| s.sample.value_ns)
    }

    /// The earliest sample time still guaranteed queryable against the newest
    /// sample: `newest - H` (the window floor, `REQ-SPATIAL-037`).
    /// REQ: REQ-SPATIAL-037.
    #[must_use]
    pub fn window_floor_ns(&self) -> Option<u64> {
        self.newest_ns().map(|n| n.saturating_sub(self.window_ns))
    }

    /// Ingests a sample per the history-ingestion protocol (`REQ-SPATIAL-037`).
    ///
    /// Returns [`Ingest::DensityViolation`] for a sample closer than `P` to the
    /// previously accepted sample, [`Ingest::LateDropped`] for a sample older
    /// than the retained floor, [`Ingest::Duplicate`] when the sample time is
    /// already retained (newer receive time wins), and
    /// [`Ingest::Appended`]/[`Ingest::Inserted`] otherwise.
    /// REQ: REQ-SPATIAL-037, REQ-SPATIAL-033.
    pub fn push(&mut self, sample: HistorySample) -> Ingest {
        let t = sample.sample.value_ns;
        // Density contract: two accepted samples are never closer than P. The
        // comparison is against the time-neighbour below the new sample.
        let too_close_below = self
            .entries
            .iter()
            .map(|s| s.sample.value_ns)
            .filter(|&v| v < t)
            .max()
            .is_some_and(|prev| t - prev < self.min_period_ns);
        let too_close_above = self
            .entries
            .iter()
            .map(|s| s.sample.value_ns)
            .filter(|&v| v > t)
            .min()
            .is_some_and(|next| next - t < self.min_period_ns);
        if too_close_below || too_close_above {
            self.density_violations += 1;
            return Ingest::DensityViolation;
        }
        // Duplicate sample time: keep the newer receive time.
        if let Some(existing) = self.entries.iter_mut().find(|s| s.sample.value_ns == t) {
            if sample.receive.value_ns > existing.receive.value_ns {
                *existing = sample;
            }
            return Ingest::Duplicate;
        }
        let outcome = if self.entries.is_empty()
            || t > self.entries[self.entries.len() - 1].sample.value_ns
        {
            self.entries.push(sample);
            Ingest::Appended
        } else {
            // Older than the retained floor ⇒ rejected and counted.
            let before_floor = self
                .newest_ns()
                .is_some_and(|newest| t < newest.saturating_sub(self.window_ns));
            if before_floor {
                self.late_dropped += 1;
                return Ingest::LateDropped;
            }
            let pos = self.entries.partition_point(|s| s.sample.value_ns < t);
            self.entries.insert(pos, sample);
            Ingest::Inserted
        };
        self.evict();
        outcome
    }

    /// Evicts the oldest samples until the ring fits capacity `N` and every
    /// retained sample lies within `[newest − H, newest]` (`REQ-SPATIAL-037`).
    /// REQ: REQ-SPATIAL-037.
    fn evict(&mut self) {
        if let Some(floor) = self.newest_ns().map(|n| n.saturating_sub(self.window_ns)) {
            while self
                .entries
                .first()
                .is_some_and(|s| s.sample.value_ns < floor)
            {
                self.entries.remove(0);
            }
        }
        while self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
    }

    /// The retained sample exactly at `t_ns`, if any (`I-Q4`).
    /// REQ: REQ-SPATIAL-022.
    #[must_use]
    pub fn exact_at(&self, t_ns: u64) -> Option<&HistorySample> {
        self.entries.iter().find(|s| s.sample.value_ns == t_ns)
    }

    /// Interpolates the transform at `t_ns` (linear translation + SLERP rotation)
    /// between the bracketing samples of the sample-time-ordered ring (`I-Q4`).
    ///
    /// # Errors
    /// [`SpatialError::OutOfHistory`] when `t_ns` is before the oldest sample or
    /// beyond the newest sample (never a silent extrapolation — `predict` is the
    /// only extrapolation path), or when `t_ns` falls inside a discontinuity gap.
    /// [`SpatialError::DiscontinuityNearby`] when the bracketing pair spans a
    /// flagged discontinuity boundary: interpolation is refused with no
    /// cross-jump value (`I-Q7`).
    /// REQ: REQ-SPATIAL-022, REQ-SPATIAL-016.
    pub fn interpolate(&self, t_ns: u64) -> Result<Transform3, SpatialError> {
        if self.entries.is_empty() {
            return Err(SpatialError::OutOfHistory);
        }
        if let Some(exact) = self.exact_at(t_ns) {
            return Ok(exact.value);
        }
        let oldest = self.entries[0].sample.value_ns;
        let newest = self.entries[self.entries.len() - 1].sample.value_ns;
        if t_ns < oldest || t_ns > newest {
            return Err(SpatialError::OutOfHistory);
        }
        // Bracket: entries[lo].time < t_ns < entries[hi].time.
        let hi = self.entries.partition_point(|s| s.sample.value_ns < t_ns);
        let lo = hi - 1;
        // The jumped local edge is flagged on the newer sample; interpolating
        // across its boundary would produce an invalid intermediate path.
        if self.entries[hi].flags & FLAG_DISCONTINUOUS != 0 {
            return Err(SpatialError::DiscontinuityNearby);
        }
        let a = &self.entries[lo];
        let b = &self.entries[hi];
        let span = b.sample.value_ns - a.sample.value_ns;
        let frac = (t_ns - a.sample.value_ns) as f64 / span as f64;
        Ok(interpolate_transform(&a.value, &b.value, frac))
    }

    /// The transform of the newest sample at or before `t_ns` (a step lookup, no
    /// interpolation) — used to resolve calibration values effective at an
    /// instant; returns [`SpatialError::OutOfHistory`] when `t_ns` predates the
    /// oldest retained sample.
    ///
    /// # Errors
    /// [`SpatialError::OutOfHistory`] when no sample is retained or `t_ns`
    /// predates the oldest retained sample.
    /// REQ: REQ-SPATIAL-003, REQ-SPATIAL-022.
    pub fn value_at_or_before(&self, t_ns: u64) -> Result<HistorySample, SpatialError> {
        if self.entries.is_empty() || t_ns < self.entries[0].sample.value_ns {
            return Err(SpatialError::OutOfHistory);
        }
        let idx = self.entries.partition_point(|s| s.sample.value_ns <= t_ns);
        Ok(self.entries[idx - 1])
    }
}

/// Interpolates between transforms `a` and `b` at fraction `frac ∈ [0, 1]`:
/// linear translation + SLERP rotation (`I-Q4`).
/// REQ: REQ-SPATIAL-022.
#[must_use]
pub fn interpolate_transform(a: &Transform3, b: &Transform3, frac: f64) -> Transform3 {
    let ta = Vector3 {
        x: a.translation[0],
        y: a.translation[1],
        z: a.translation[2],
    };
    let tb = Vector3 {
        x: b.translation[0],
        y: b.translation[1],
        z: b.translation[2],
    };
    let t = ta + (tb - ta).scale(frac);
    let q = a.quat().slerp(b.quat(), frac);
    Transform3 {
        translation: [t.x, t.y, t.z],
        rotation: q.into_components(),
    }
}

/// Builds a [`HistorySample`] with no discontinuity flag, under caller-supplied
/// sample/receive [`TimePoint`]s.
/// REQ: REQ-SPATIAL-035, REQ-SPATIAL-033.
#[must_use]
pub fn stamp(transform: Transform3, sample: TimePoint, receive: TimePoint) -> HistorySample {
    HistorySample::new(transform, sample, receive, 0)
}

/// Re-exported for callers that build samples with an explicit [`UnitQuat`].
/// REQ: REQ-SPATIAL-036.
#[must_use]
pub fn transform_from_quat(translation: [f64; 3], rotation: UnitQuat) -> Transform3 {
    Transform3 {
        translation,
        rotation: rotation.into_components(),
    }
}

#[cfg(test)]
mod tests {
    use super::{HistorySample, Ingest, TimeHistory, interpolate_transform};
    use crate::clock::{ClockDomain, ClockIdentity, TimePoint};
    use crate::types::{SpatialError, Transform3, UnitQuat};

    fn host(ns: u64) -> TimePoint {
        TimePoint::new(ns, ClockIdentity::new(ClockDomain::HostMonotonic))
    }

    fn t(x: f64, y: f64, z: f64) -> Transform3 {
        Transform3 {
            translation: [x, y, z],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn sample(t_ns: u64, x: f64) -> HistorySample {
        HistorySample {
            value: t(x, 0.0, 0.0),
            sample: host(t_ns),
            receive: host(t_ns),
            flags: 0,
        }
    }

    /// REQ: REQ-SPATIAL-037 — a declaration below `N >= floor(H/P) + 2` is
    /// rejected, never silently shortened (`I-Q13`).
    #[test]
    fn history_sizing_rejects_unsatisfiable() {
        // floor(100/10) + 2 = 12 required.
        assert_eq!(
            TimeHistory::new(10, 100, 10).err(),
            Some(SpatialError::InvalidConfig)
        );
        assert!(TimeHistory::new(12, 100, 10).is_ok());
        assert_eq!(
            TimeHistory::new(5, 100, 0).err(),
            Some(SpatialError::InvalidConfig)
        );
    }

    /// REQ: REQ-SPATIAL-037 — in-window late arrivals insert in sample-time
    /// order; too-close samples are rejected and counted (`I-Q13`).
    #[test]
    fn history_ordered_insert_and_density() {
        let mut h = TimeHistory::new(7, 50, 10).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(h.push(sample(100, 1.0)), Ingest::Appended);
        assert_eq!(h.push(sample(120, 3.0)), Ingest::Appended);
        // In-window late arrival (110) inserts between 100 and 120.
        assert_eq!(h.push(sample(110, 2.0)), Ingest::Inserted);
        assert_eq!(
            h.samples()
                .iter()
                .map(|s| s.sample.value_ns)
                .collect::<Vec<_>>(),
            vec![100, 110, 120]
        );
        // 5 ns after the neighbour violates P=10.
        assert_eq!(h.push(sample(115, 9.0)), Ingest::DensityViolation);
        assert_eq!(h.density_violations(), 1);
    }

    /// REQ: REQ-SPATIAL-037 — a duplicate sample time keeps the newer receive
    /// time; an ancient sample is dropped and counted.
    #[test]
    fn history_duplicate_and_late() {
        let mut h = TimeHistory::new(8, 50, 10).unwrap_or_else(|e| unreachable!("{e}"));
        h.push(sample(200, 1.0));
        let mut dup = sample(200, 5.0);
        dup.receive = host(999);
        assert_eq!(h.push(dup), Ingest::Duplicate);
        assert_eq!(h.exact_at(200).map(|s| s.value.translation[0]), Some(5.0));
        // Older than the retained floor (newest 200 − H 50 = 150).
        assert_eq!(h.push(sample(50, 7.0)), Ingest::LateDropped);
        assert_eq!(h.late_dropped(), 1);
    }

    /// REQ: REQ-SPATIAL-022 — exact samples return unchanged; between samples
    /// translation is linear and rotation `SLERP`ed (`I-Q4`).
    #[test]
    fn history_interpolates() {
        let mut h = TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}"));
        h.push(sample(0, 0.0));
        let mut b = sample(10, 10.0);
        b.value.rotation =
            UnitQuat::from_axis_angle(crate::types::Direction3::Z, core::f64::consts::FRAC_PI_2)
                .unwrap_or(UnitQuat::IDENTITY)
                .into_components();
        h.push(b);
        // Exact endpoints.
        assert!((h.interpolate(0).unwrap_or_default().translation[0]).abs() < 1e-12);
        // Midpoint: linear translation 5.0; rotation halfway to +90°.
        let mid = h.interpolate(5).unwrap_or_else(|e| unreachable!("{e}"));
        assert!((mid.translation[0] - 5.0).abs() < 1e-12);
        assert!((mid.quat().norm() - 1.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-022 — never silently extrapolate beyond the newest
    /// sample (`I-Q4`).
    #[test]
    fn history_no_extrapolation() {
        let mut h = TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}"));
        h.push(sample(0, 0.0));
        h.push(sample(100, 1.0));
        assert_eq!(h.interpolate(101).err(), Some(SpatialError::OutOfHistory));
        assert!(h.interpolate(50).is_ok());
    }

    /// REQ: REQ-SPATIAL-016 — interpolation never spans a flagged discontinuity
    /// boundary (`I-Q7`).
    #[test]
    fn history_discontinuity_guard() {
        let mut h = TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}"));
        h.push(sample(0, 0.0));
        h.push(sample(10, 100.0));
        // Mark the newer sample discontinuous.
        if let Some(last) = h.entries.last_mut() {
            last.flags |= crate::cache::FLAG_DISCONTINUOUS;
        }
        assert_eq!(
            h.interpolate(5).err(),
            Some(SpatialError::DiscontinuityNearby)
        );
        // Exact endpoints remain valid.
        assert!((h.interpolate(0).unwrap_or_default().translation[0]).abs() < 1e-12);
        assert!((h.interpolate(10).unwrap_or_default().translation[0] - 100.0).abs() < 1e-12);
    }

    /// REQ: REQ-SPATIAL-022 — the interpolation helper blends translation
    /// linearly.
    #[test]
    fn interpolate_helper_linear() {
        let a = t(0.0, 0.0, 0.0);
        let b = t(4.0, 8.0, 12.0);
        let m = interpolate_transform(&a, &b, 0.25);
        assert!((m.translation[0] - 1.0).abs() < 1e-12);
        assert!((m.translation[1] - 2.0).abs() < 1e-12);
        assert!((m.translation[2] - 3.0).abs() < 1e-12);
    }
}
