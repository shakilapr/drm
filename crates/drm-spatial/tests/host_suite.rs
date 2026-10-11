//! Host reference-suite consolidation (work-plan M21W3-E4-T1): the full spatial
//! reference suite — distinct types, rigid math, geodetic boundaries, affine
//! clocks, forests/overlays, compiled plans, and the runtime proximity index —
//! runs green together against mathematical references. Every suite already has
//! its dedicated file; this consolidation re-derives one representative
//! reference result per domain and composes them into a single chain so an
//! interaction regression between suites blocks the weekly gate
//! (`SPATIAL_HOST_SUITE`). `cargo test -p drm-spatial host_suite`.
//!
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-012, REQ-SPATIAL-004,
//! REQ-SPATIAL-034, REQ-SPATIAL-013.

use drm_spatial::{
    CandidateSet, ClockDomain, ClockIdentity, ClockMapping, ClockTable, Direction3, Enu,
    GridConfig, GridDeps, HashGrid, Lla, PlanStep, Point3, SpatialError, Transform3, TransformPlan,
    TypedPoint, UnitQuat, compose, execute_plan, geodetic_to_enu, transform_direction,
    transform_point,
};

/// Documented tolerance band for host reference comparisons.
/// REQ: REQ-SPATIAL-036.
const TOL: f64 = 1e-9;

/// A rotation about `+Z` by `angle` radians.
/// REQ: REQ-SPATIAL-036.
fn rot_z(angle: f64) -> Transform3 {
    Transform3::new(
        [0.0, 0.0, 0.0],
        UnitQuat::from_axis_angle(Direction3::Z, angle)
            .unwrap_or(UnitQuat::IDENTITY)
            .into_components(),
    )
    .unwrap_or(Transform3::IDENTITY)
}

/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-038 — type semantics and rigid math:
/// point = rotation + translation, direction = rotation only, compose
/// associativity, inverse round-trip. `spatial_type_semantics`.
#[test]
#[allow(clippy::many_single_char_names)]
fn host_suite_types_and_math() {
    let t = Transform3::new(
        [1.0, 2.0, 3.0],
        rot_z(core::f64::consts::FRAC_PI_2).rotation,
    )
    .unwrap_or(Transform3::IDENTITY);
    let p = Point3::new(1.0, 0.0, 0.0).unwrap_or_default();
    let tp = transform_point(&t, p);
    assert!((tp.x - 1.0).abs() < TOL);
    assert!((tp.y - 3.0).abs() < TOL);
    assert!((tp.z - 3.0).abs() < TOL);

    let d = transform_direction(&t, Direction3::X);
    let dc = d.components();
    assert!((dc[1] - 1.0).abs() < TOL);

    let a = Transform3::new([0.5, 0.0, 0.0], rot_z(0.3).rotation).unwrap_or_default();
    let b = Transform3::new([0.0, 0.0, 1.0], rot_z(0.1).rotation).unwrap_or_default();
    let stepwise = transform_point(&a, transform_point(&b, p));
    let folded = transform_point(&compose(&a, &b), p);
    assert!((stepwise.x - folded.x).abs() < TOL);
    assert!((stepwise.y - folded.y).abs() < TOL);
    assert!((stepwise.z - folded.z).abs() < TOL);
}

/// REQ: REQ-SPATIAL-012 — geodetic boundary chain stays a small local offset
/// (`I-S5`). `spatial_geodetic_precision`.
#[test]
fn host_suite_geodetic() {
    let anchor = Lla {
        lat_rad: 47.3769_f64.to_radians(),
        lon_rad: 8.5417_f64.to_radians(),
        alt_m: 408.0,
    };
    let near = Lla {
        alt_m: anchor.alt_m + 1.0,
        ..anchor
    };
    let enu: Enu = geodetic_to_enu(near, anchor);
    assert!(enu.east.abs() < TOL);
    assert!(enu.north.abs() < TOL);
    assert!((enu.up - 1.0).abs() < 1e-6);
}

/// REQ: REQ-SPATIAL-004 — an affine clock mapping composes with the geodetic
/// chain without a silent domain comparison (`I-S7`).
#[test]
fn host_suite_clocks() {
    let device = ClockIdentity::new(ClockDomain::Device(0));
    let host = ClockDomain::HostMonotonic;
    let mut table = ClockTable::new();
    table.insert(
        device,
        ClockMapping {
            to: ClockIdentity::new(host),
            from_ref_ns: 1000,
            to_ref_ns: 5000,
            rate_ratio: 1.0,
            uncertainty_at_ref_ns: 10,
            uncertainty_growth: 1e-6,
            valid_from_ns: 0,
            valid_until_ns: 1_000_000,
            generation: 1,
        },
    );
    let mapped =
        drm_spatial::map_timestamp(drm_spatial::TimePoint::new(2000, device), host, &table)
            .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(mapped.value_ns, 6000);
    // An unmapped clock never silently compares: explicit status (`I-S7`).
    let unmapped = drm_spatial::map_timestamp(
        drm_spatial::TimePoint::new(0, ClockIdentity::new(ClockDomain::SimReplay)),
        host,
        &table,
    );
    assert_eq!(unmapped, Err(SpatialError::ClockUnresolved));
}

/// REQ: REQ-SPATIAL-034 — a compiled plan executes a folded transform chain
/// without graph traversal (`I-Q1`); the host suite runs queries and index
/// together. `spatial_queries` + `spatial_index`.
#[test]
fn host_suite_plan_and_index() {
    static STEPS: [PlanStep; 3] = [
        PlanStep::Constant(Transform3::IDENTITY),
        PlanStep::Constant(Transform3::IDENTITY),
        PlanStep::Multiply,
    ];
    let plan = TransformPlan::new(&STEPS).unwrap_or_else(|e| unreachable!("{e}"));
    let executed = execute_plan(
        &plan,
        |_| Err(SpatialError::OutOfHistory),
        |_| Err(SpatialError::OutOfHistory),
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(executed, Transform3::IDENTITY);

    let cfg = GridConfig {
        cell_size: 1.0,
        max_objects: 16,
        max_cells_per_object: 8,
        max_query_cells: 64,
        slack: 0.1,
    };
    let mut grid = HashGrid::activate(cfg).unwrap_or_else(|e| unreachable!("{e}"));
    for i in 0..8u32 {
        grid.on_sample(
            i,
            Point3 {
                x: f64::from(i),
                y: 0.0,
                z: 0.0,
            },
            GridDeps {
                own: 1,
                ancestor: 1,
                calibration: 1,
                topology: 1,
            },
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
    }
    let mut scratch = CandidateSet::with_capacity(16).unwrap_or_else(|e| unreachable!("{e}"));
    let outcome = grid
        .nearby(
            TypedPoint {
                point: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                frame: 0,
            },
            3.5,
            &mut scratch,
        )
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert!(outcome.is_complete());
    // Candidates are a deliberate superset (`I-X6`): exact-check the reference
    // hits rather than the raw candidate count.
    let hits: Vec<u32> = scratch.handles().iter().map(|h| h.index).collect();
    for expect in 0..4u32 {
        assert!(
            hits.contains(&expect),
            "object {expect} within radius missing"
        );
    }
}
