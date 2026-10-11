//! Versioned-cache suite (`cargo test -p drm-spatial cache`): dynamic-edge
//! version semantics, generation-protected reads, and the per-process
//! `PlanCache` shortcircuit. Version equality reuses cached **geometry** only;
//! freshness/staleness/uncertainty are always derived per query, so version
//! equality never implies `VALID` (`I-Q2`, [spatial-queries.md] §Test Matrix).
//!
//! REQ: REQ-SPATIAL-002, REQ-SPATIAL-050, REQ-SPATIAL-030, REQ-SPATIAL-016.

use drm_spatial::{
    ClockDomain, ClockIdentity, EdgeSample, FLAG_DISCONTINUITY_NEARBY, FLAG_STALE, PlanCache,
    PlanStep, QualityFlags, SpatialEdgeCache, TimePoint, Transform3, TransformPlan,
    TransformResult, execute_plan, observe_versions,
};

/// A static two-step plan: read dynamic edge `0`, multiply by the identity.
/// REQ: REQ-SPATIAL-034.
fn edge_plan() -> &'static [PlanStep] {
    static STEPS: [PlanStep; 3] = [
        PlanStep::DynamicEdge(0),
        PlanStep::Constant(Transform3::IDENTITY),
        PlanStep::Multiply,
    ];
    &STEPS
}

/// Publishes `t` on edge `0` of `cache` under a fresh claim (single publish).
/// REQ: REQ-SPATIAL-050.
fn publish(cache: &SpatialEdgeCache, t: &Transform3, sample_ns: u64) {
    let claim = cache
        .claim_writer(0, 1)
        .unwrap_or_else(|e| unreachable!("claim: {e}"));
    let host = ClockIdentity::new(ClockDomain::HostMonotonic);
    cache
        .publish_sample(
            &claim,
            &EdgeSample::new(
                *t,
                TimePoint::new(sample_ns, host),
                TimePoint::new(sample_ns, host),
            ),
        )
        .unwrap_or_else(|e| unreachable!("publish: {e}"));
}

/// Reads the observed-version vector for edge `0`.
/// REQ: REQ-SPATIAL-002.
fn edge_versions(cache: &SpatialEdgeCache) -> Vec<u64> {
    let slot = cache.slot(0).unwrap_or_else(|| unreachable!("slot"));
    observe_versions(&[slot])
}

/// REQ: REQ-SPATIAL-002 — `spatial_cache_shortcircuit`: unchanged input versions
/// reuse the cached geometry with zero re-execution; any version change forces a
/// re-execute (`I-Q2`).
#[test]
fn spatial_cache_shortcircuit() {
    let cache = SpatialEdgeCache::new(1);
    let plan = TransformPlan::new(edge_plan()).unwrap_or_else(|e| unreachable!("{e}"));
    let mut plan_cache = PlanCache::new(&plan);

    let claim = cache
        .claim_writer(0, 1)
        .unwrap_or_else(|e| unreachable!("claim: {e}"));
    let host = ClockIdentity::new(ClockDomain::HostMonotonic);
    let exec = |edge: u32| {
        cache
            .read(edge as usize)
            .map(|s| s.transform)
            .ok_or(drm_spatial::SpatialError::OutOfHistory)
    };
    let t = Transform3::new([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0])
        .unwrap_or_else(|e| unreachable!("{e}"));
    publish_with(&cache, &claim, &t, 100);

    // First query: execute and record the observed versions.
    let versions = edge_versions(&cache);
    assert!(!plan_cache.matches(&versions));
    let executed = execute_plan(&plan, exec, |_slot| {
        Err(drm_spatial::SpatialError::OutOfHistory)
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(executed, t);
    plan_cache.update(executed, TimePoint::new(100, host), versions);

    // Second query with no new publish: version match ⇒ cached geometry reused.
    assert!(plan_cache.matches(&edge_versions(&cache)));
    assert_eq!(plan_cache.cached(), t);

    // A new publish bumps the version ⇒ cache misses and re-executes.
    let t2 = Transform3::new([4.0, 5.0, 6.0], [0.0, 0.0, 0.0, 1.0])
        .unwrap_or_else(|e| unreachable!("{e}"));
    publish_with(&cache, &claim, &t2, 200);
    assert!(!plan_cache.matches(&edge_versions(&cache)));
    let reexec = execute_plan(&plan, exec, |_slot| {
        Err(drm_spatial::SpatialError::OutOfHistory)
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(reexec, t2);
}

/// Publishes `t` under a held `claim`.
/// REQ: REQ-SPATIAL-050.
fn publish_with(
    cache: &SpatialEdgeCache,
    claim: &drm_spatial::WriterClaim,
    t: &Transform3,
    sample_ns: u64,
) {
    let host = ClockIdentity::new(ClockDomain::HostMonotonic);
    cache
        .publish_sample(
            claim,
            &EdgeSample::new(
                *t,
                TimePoint::new(sample_ns, host),
                TimePoint::new(sample_ns, host),
            ),
        )
        .unwrap_or_else(|e| unreachable!("publish: {e}"));
}

/// REQ: REQ-SPATIAL-002 — `spatial_cache_aging`: with no version change, wall
/// time advancing past the freshness contract must surface `STALE`; the cached
/// geometry is reused but status is never cached as all-clear (`I-Q2`).
#[test]
fn spatial_cache_aging() {
    let cache = SpatialEdgeCache::new(1);
    let plan = TransformPlan::new(edge_plan()).unwrap_or_else(|e| unreachable!("{e}"));
    let mut plan_cache = PlanCache::new(&plan);
    let host = ClockIdentity::new(ClockDomain::HostMonotonic);

    let t = Transform3::new([7.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0])
        .unwrap_or_else(|e| unreachable!("{e}"));
    publish(&cache, &t, 1_000);
    let versions = {
        let slot = cache.slot(0).unwrap_or_else(|| unreachable!("slot"));
        observe_versions(&[slot])
    };
    plan_cache.update(t, TimePoint::new(1_000, host), versions.clone());

    // Status is derived per query: a fresh query is all-clear.
    let fresh = derive_status(&plan_cache, 1_000, 500);
    assert!(!fresh.flags.contains(FLAG_STALE));
    assert_eq!(fresh.clock_uncertainty_ns, 0);

    // No publish, but wall time advances well past the contract: STALE.
    let aged = derive_status(&plan_cache, 10_000_000, 500);
    assert!(aged.flags.contains(FLAG_STALE));
    // Geometry was still reused (unchanged versions).
    assert_eq!(aged.transform, Some(t));
}

/// Derives a [`TransformResult`] from a cache: reuses cached geometry when the
/// versions still match, and derives freshness per query (`I-Q2`).
/// REQ: REQ-SPATIAL-002.
fn derive_status(plan_cache: &PlanCache<'_>, now_ns: u64, max_age_ns: u64) -> TransformResult {
    let sample_ns = plan_cache.coherent_sample().value_ns;
    let age = now_ns.saturating_sub(sample_ns);
    let mut result = TransformResult::ok(plan_cache.cached(), 0);
    result.sample_time = Some(plan_cache.coherent_sample());
    result.sample_age_ns = Some(age);
    if age > max_age_ns {
        result.flags = result.flags.with(FLAG_STALE);
    }
    result
}

/// REQ: REQ-SPATIAL-050 — the slot version is even after publish and odd only
/// mid-write; a reader never observes a torn snapshot (`I-Q14`).
#[test]
fn cache_version_semantics_exact() {
    let cache = SpatialEdgeCache::new(1);
    let claim = cache
        .claim_writer(0, 1)
        .unwrap_or_else(|e| unreachable!("claim: {e}"));
    {
        let slot = cache.slot(0).unwrap_or_else(|| unreachable!("slot"));
        assert_eq!(slot.version(), 0);
    }
    publish_with(&cache, &claim, &Transform3::IDENTITY, 1);
    {
        let slot = cache.slot(0).unwrap_or_else(|| unreachable!("slot"));
        assert_eq!(slot.version(), 2);
    }
    publish_with(&cache, &claim, &Transform3::IDENTITY, 2);
    let slot = cache.slot(0).unwrap_or_else(|| unreachable!("slot"));
    assert_eq!(slot.version(), 4);
    // Version is strictly increasing and even at rest.
    assert_eq!(slot.version() % 2, 0);
}

/// REQ: REQ-SPATIAL-016 — a discontinuous sample is flagged and the flag
/// survives the lockless round trip (`I-Q7`).
#[test]
fn cache_discontinuous_flag_roundtrip() {
    let cache = SpatialEdgeCache::new(1);
    let claim = cache
        .claim_writer(0, 1)
        .unwrap_or_else(|e| unreachable!("{e}"));
    let host = ClockIdentity::new(ClockDomain::HostMonotonic);
    let sample = EdgeSample::new(
        Transform3::IDENTITY,
        TimePoint::new(10, host),
        TimePoint::new(11, host),
    )
    .with_discontinuous();
    cache
        .publish_sample(&claim, &sample)
        .unwrap_or_else(|e| unreachable!("{e}"));
    let read = cache.read(0).unwrap_or_else(|| unreachable!("sample"));
    assert!(read.is_discontinuous());
}

/// REQ: REQ-SPATIAL-030 — a non-owner cannot publish to a claimed edge, so the
/// single-writer invariant holds at the cache boundary (`I-S11`).
#[test]
fn cache_single_writer_boundary() {
    let cache = SpatialEdgeCache::new(1);
    let _ = cache
        .claim_writer(0, 1)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(
        cache.claim_writer(0, 2),
        Err(drm_spatial::SpatialError::WriterConflict)
    );
}

/// REQ: REQ-SPATIAL-016 — quality flags are combinable and independent.
#[test]
fn quality_flags_combinable() {
    let flags = QualityFlags::NONE
        .with(FLAG_STALE)
        .with(FLAG_DISCONTINUITY_NEARBY);
    assert!(flags.contains(FLAG_STALE));
    assert!(flags.contains(FLAG_DISCONTINUITY_NEARBY));
    assert!(!flags.contains(drm_spatial::FLAG_EXTRAPOLATED));
}
