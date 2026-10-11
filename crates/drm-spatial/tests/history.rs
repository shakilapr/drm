//! Time-history sizing and ordering suite (`cargo test -p drm-spatial history`):
//! the guaranteed window `history = H`, structural capacity `max_samples = N`,
//! and producer-density contract `min_sample_period = P` are enforced exactly —
//! an unsatisfiable `N < floor(H/P) + 2` declaration is rejected (never silently
//! shortened), a sample closer than `P` is rejected and counted
//! (`DensityViolation`), and the ring is strictly sample-time ordered with
//! in-order late arrivals and newer-receive-wins duplicates
//! ([spatial-queries.md] §Invariants `I-Q13`, §Protocols history ingestion;
//! [spatial-core.md] §Protocols history allocation).
//!
//! REQ: REQ-SPATIAL-037, REQ-SPATIAL-033.

// Test fixtures convert between nanosecond integer stamps and `f64` geometry
// exactly for the small magnitudes used.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use drm_spatial::{
    ClockDomain, ClockIdentity, HistorySample, Ingest, SpatialError, TimeHistory, Transform3,
};

/// A host-monotonic [`TimePoint`] at `ns`.
/// REQ: REQ-SPATIAL-035.
fn host(ns: u64) -> drm_spatial::TimePoint {
    drm_spatial::TimePoint::new(ns, ClockIdentity::new(ClockDomain::HostMonotonic))
}

/// A translation-only history sample.
/// REQ: REQ-SPATIAL-038.
fn sample(t_ns: u64, x: f64) -> HistorySample {
    HistorySample::new(
        Transform3 {
            translation: [x, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        },
        host(t_ns),
        host(t_ns),
        0,
    )
}

/// REQ: REQ-SPATIAL-037 — `spatial_history_sizing`: a declaration below
/// `N >= floor(H/P) + 2` is rejected (never silently shortened); `P = 0` is
/// rejected; a valid declaration carries its bound; and faster-than-`P` samples
/// are rejected and counted (`DensityViolation`) so the bound always holds
/// (`I-Q13`).
#[test]
fn spatial_history_sizing() {
    // floor(100/10) + 2 = 12 required: 11 is unsatisfiable, 12 exact, 13 slack.
    assert_eq!(
        TimeHistory::new(11, 100, 10).err(),
        Some(SpatialError::InvalidConfig)
    );
    assert_eq!(
        TimeHistory::new(5, 100, 0).err(),
        Some(SpatialError::InvalidConfig)
    );
    let h = TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(h.capacity(), 12);
    assert_eq!(h.window_ns(), 100);
    assert_eq!(h.min_period_ns(), 10);
    assert_eq!(h.density_violations(), 0);

    // A producer denser than P violates the contract: rejected and counted.
    let mut dense = TimeHistory::new(12, 100, 10).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(dense.push(sample(100, 1.0)), Ingest::Appended);
    assert_eq!(dense.push(sample(105, 2.0)), Ingest::DensityViolation);
    assert_eq!(dense.push(sample(95, 3.0)), Ingest::DensityViolation);
    assert_eq!(dense.density_violations(), 2);
    // A sample exactly P apart is accepted.
    assert_eq!(dense.push(sample(110, 4.0)), Ingest::Appended);
    assert_eq!(dense.len(), 2);
    // The retained window never exceeds N for a valid producer.
    assert!(dense.len() <= dense.capacity());
}

/// REQ: REQ-SPATIAL-037 — `spatial_history_order`: shuffled arrivals insert in
/// sample-time order; a duplicate sample time keeps the newer receive time;
/// samples older than the retained floor are dropped and counted; and the
/// retained set never exceeds `N` (`I-Q13`).
#[test]
fn spatial_history_order() {
    // window H = 50, P = 10 ⇒ N >= 7 required.
    let mut h = TimeHistory::new(7, 50, 10).unwrap_or_else(|e| unreachable!("{e}"));
    // Shuffled arrivals within the window insert in order.
    for (t, v) in [(130_u64, 3.0), (110, 1.0), (120, 2.0)] {
        let outcome = h.push(sample(t, v));
        assert!(matches!(outcome, Ingest::Appended | Ingest::Inserted));
    }
    let times: Vec<u64> = h.samples().iter().map(|s| s.sample.value_ns).collect();
    assert_eq!(times, vec![110, 120, 130], "sample-time ordered");

    // A duplicate time keeps the newer receive time and drops the other.
    let mut newer = sample(120, 9.0);
    newer.receive = host(999);
    assert_eq!(h.push(newer), Ingest::Duplicate);
    assert_eq!(h.exact_at(120).map(|s| s.value.translation[0]), Some(9.0));

    // Older than the retained floor (newest 130 − H 50 = 80): dropped, counted.
    assert_eq!(h.push(sample(70, 5.0)), Ingest::LateDropped);
    assert_eq!(h.late_dropped(), 1);

    // Capacity is a hard structural bound; eviction keeps the retained set
    // within both N and the window H.
    let mut capped = TimeHistory::new(7, 50, 10).unwrap_or_else(|e| unreachable!("{e}"));
    for k in 0..20_u64 {
        capped.push(sample(10 * (k + 1), k as f64));
    }
    assert!(capped.len() <= capped.capacity(), "never exceeds N");
    let oldest = capped.oldest_ns().unwrap_or_default();
    let newest = capped.newest_ns().unwrap_or_default();
    assert!(newest - oldest <= capped.window_ns(), "within H");
}
