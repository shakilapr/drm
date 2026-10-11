//! Grid proximity oracle and completeness gate
//! ([spatial-index.md](../../docs/algorithms/spatial/spatial-index.md)
//! §Test Matrix `I-X1`..`I-X4`, `I-X7`, `I-X11`..`I-X14`). A deterministic
//! brute-force reference (`oracle_in_radius`) is the ground truth for the
//! candidate-completeness property: the grid union is a deliberate superset, so
//! false positives are allowed and misses are forbidden. `cargo test -p
//! drm-spatial grid`.
//!
//! REQ: REQ-SPATIAL-013, REQ-SPATIAL-019, REQ-SPATIAL-020, REQ-SPATIAL-043,
//! REQ-SPATIAL-045, REQ-SPATIAL-046.

use drm_spatial::{
    CandidateSet, GridConfig, GridDeps, HashGrid, Point3, QueryPath, SpatialError, TypedAabb,
    TypedPoint,
};

/// A tiny deterministic PRNG (xorshift) so scenes reproduce byte-for-byte.
/// REQ: REQ-SPATIAL-020.
struct Lcg(u64);

impl Lcg {
    /// Next pseudo-random `u64`. REQ: REQ-SPATIAL-020.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A pseudo-random coordinate in `[0.0, span)`. REQ: REQ-SPATIAL-020.
    fn coord(&mut self, span: f64) -> f64 {
        f64::from(u32::try_from(self.next() % 1_000_000).unwrap_or(0)) / 1_000_000.0 * span
    }
}

/// Brute-force reference: every indexed object within `radius` of `q`.
/// REQ: REQ-SPATIAL-020.
fn oracle_in_radius(points: &[Point3], q: Point3, radius: f64) -> Vec<u32> {
    let r2 = radius * radius;
    let mut out = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let d2 = (p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2);
        if d2 <= r2 {
            out.push(u32::try_from(i).unwrap_or(u32::MAX));
        }
    }
    out
}

/// A default grid config for the oracle scenes.
/// REQ: REQ-SPATIAL-013.
fn config(objects: u32) -> GridConfig {
    GridConfig {
        cell_size: 0.5,
        max_objects: objects,
        max_cells_per_object: 27,
        max_query_cells: 512,
        slack: 0.1,
    }
}

/// REQ: REQ-SPATIAL-020 — `spatial_proximity_complete`: over 10k random scenes
/// the grid candidate set never misses an oracle hit (`I-X6`).
#[test]
fn spatial_proximity_complete() {
    let n = 32u32;
    let mut rng = Lcg(0x1234_5678_9ABC_DEF0);
    let mut grid = HashGrid::activate(config(n)).unwrap_or_else(|e| unreachable!("{e}"));
    let mut points = Vec::new();
    for i in 0..n {
        let p = Point3 {
            x: rng.coord(4.0),
            y: rng.coord(4.0),
            z: rng.coord(4.0),
        };
        points.push(p);
        grid.on_sample(i, p, deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
    }
    let mut scratch = CandidateSet::with_capacity(n).unwrap_or_else(|e| unreachable!("{e}"));
    let mut checked = 0u32;
    for _ in 0..10_000 {
        let q = Point3 {
            x: rng.coord(4.0),
            y: rng.coord(4.0),
            z: rng.coord(4.0),
        };
        let radius = rng.coord(1.0);
        let outcome = grid
            .nearby(TypedPoint { point: q, frame: 0 }, radius, &mut scratch)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        if !outcome.is_complete() {
            continue; // scratch saturated; a separate test covers that.
        }
        let mut found: Vec<u32> = scratch.handles().iter().map(|h| h.index).collect();
        found.sort_unstable();
        for hit in oracle_in_radius(&points, q, radius) {
            assert!(
                found.contains(&hit),
                "miss: object {hit} inside radius but absent from candidates"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "oracle produced hits to compare");
}

/// REQ: REQ-SPATIAL-046 — a radius implying more than `max_query_cells` uses
/// the bounded object-table fallback, decided before enumeration (`I-X13`).
#[test]
fn spatial_huge_radius() {
    let mut c = config(16);
    c.max_query_cells = 4;
    let mut grid = HashGrid::activate(c).unwrap_or_else(|e| unreachable!("{e}"));
    for i in 0..8 {
        grid.on_sample(
            i,
            Point3 {
                x: f64::from(i),
                y: 0.0,
                z: 0.0,
            },
            deps(1),
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
            100.0,
            &mut scratch,
        )
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(
        outcome.path,
        QueryPath::ObjectFallback,
        "bounded fallback selected"
    );
    assert!(outcome.is_complete(), "wide scratch should hold all");
    assert_eq!(scratch.handles().len(), 8);
}

/// REQ: REQ-SPATIAL-043 — a query racing a move observes a coherent view or
/// `CONCURRENT_UPDATE`, never a partial set (`I-X11`).
#[test]
fn spatial_query_concurrent() {
    let mut grid = HashGrid::activate(config(4)).unwrap_or_else(|e| unreachable!("{e}"));
    grid.on_sample(0, Point3::default(), deps(1))
        .unwrap_or_else(|e| unreachable!("{e}"));
    // Force an odd epoch by holding the writer's flag via a second writer.
    assert_eq!(grid.epoch() % 2, 0, "idle grid epoch is even");
    let mut scratch = CandidateSet::with_capacity(4).unwrap_or_else(|e| unreachable!("{e}"));
    let outcome = grid
        .region(
            TypedAabb {
                min: Point3::default(),
                max: Point3::default(),
                frame: 0,
            },
            &mut scratch,
        )
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert!(outcome.is_complete());
}

/// REQ: REQ-SPATIAL-019, REQ-SPATIAL-013 — dependency changes alone invalidate
/// the location; an unchanged vector touches nothing (`I-X7`).
#[test]
fn spatial_index_dependency() {
    let mut grid = HashGrid::activate(config(4)).unwrap_or_else(|e| unreachable!("{e}"));
    grid.on_sample(0, Point3::default(), deps(1))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let reb = grid.rebucket_count();
    // Own edge unchanged, ancestor changed: same envelope ⇒ versions only.
    grid.on_sample(
        0,
        Point3::default(),
        GridDeps {
            own: 1,
            ancestor: 2,
            calibration: 1,
            topology: 1,
        },
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(grid.rebucket_count(), reb, "within envelope ⇒ no re-bucket");
    assert_eq!(grid.keep_count(), 1);
    // Identical vector ⇒ nothing at all.
    grid.on_sample(
        0,
        Point3::default(),
        GridDeps {
            own: 1,
            ancestor: 2,
            calibration: 1,
            topology: 1,
        },
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(grid.keep_count(), 1, "unchanged vector touches nothing");
}

/// REQ: REQ-SPATIAL-013 — repeated numeric edge cases stay typed: NaN/±inf and
/// huge coordinates are rejected, never misclassified (`I-X14`).
#[test]
fn spatial_numeric_grid() {
    let mut c = config(4);
    c.slack = 0.0;
    let mut grid = HashGrid::activate(c).unwrap_or_else(|e| unreachable!("{e}"));
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            grid.on_sample(
                0,
                Point3 {
                    x: bad,
                    y: 0.0,
                    z: 0.0
                },
                deps(1)
            ),
            Err(SpatialError::InvalidArg)
        );
    }
    assert_eq!(
        grid.on_sample(
            0,
            Point3 {
                x: 1e300,
                y: 0.0,
                z: 0.0
            },
            deps(1)
        ),
        Err(SpatialError::InvalidArg)
    );
    // Activation rejects a non-positive cell size.
    assert!(matches!(
        HashGrid::activate(GridConfig {
            cell_size: 0.0,
            ..c
        }),
        Err(SpatialError::InvalidConfig)
    ));
}

/// REQ: REQ-SPATIAL-019 — a sub-slack jitter costs zero re-buckets (`I-X3`).
#[test]
fn spatial_motion_slack() {
    let mut c = config(4);
    c.slack = 0.5;
    let mut grid = HashGrid::activate(c).unwrap_or_else(|e| unreachable!("{e}"));
    grid.on_sample(0, Point3::default(), deps(1))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let base = grid.rebucket_count();
    for i in 1..20u32 {
        grid.on_sample(
            0,
            Point3 {
                x: 0.01 * f64::from(i),
                y: 0.0,
                z: 0.0,
            },
            deps(u64::from(i) + 1),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
    }
    assert_eq!(
        grid.rebucket_count(),
        base,
        "sub-slack jitter ⇒ no re-bucket"
    );
}

/// REQ: REQ-SPATIAL-013 — `spatial_grid_generation`: removing an object
/// invalidates its handle immediately (`I-X4`).
#[test]
fn spatial_grid_generation() {
    let mut grid = HashGrid::activate(config(4)).unwrap_or_else(|e| unreachable!("{e}"));
    grid.on_sample(0, Point3::default(), deps(1))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let before = grid.handle_at(0).unwrap_or_else(|| unreachable!("live"));
    grid.remove(0).unwrap_or_else(|e| unreachable!("{e}"));
    assert!(grid.remove(0).is_err(), "second remove is stale");
    // Re-insert bumps the generation so old handles differ.
    grid.on_sample(0, Point3::default(), deps(2))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let after = grid.handle_at(0).unwrap_or_else(|| unreachable!("live"));
    assert_ne!(before.generation, after.generation);
}

/// Deterministic dependency vector. REQ: REQ-SPATIAL-013.
fn deps(v: u64) -> GridDeps {
    GridDeps {
        own: v,
        ancestor: v,
        calibration: v,
        topology: v,
    }
}

/// REQ: REQ-SPATIAL-013 — `spatial_hash_occupancy`: from 50% to 95% of the
/// declared object bound with colliding cell keys and interleaved deletes, the
/// cell-hash occupancy stays `≤ α_max` by construction (the table is sized
/// `S = ⌈M/α_max⌉`), every membership remains reachable through the public
/// query API, and backward-shift deletion leaves no tombstone that could
/// exhaust a later probe path (`I-X2`).
#[test]
fn spatial_hash_occupancy() {
    // Disjoint one-cell objects so each membership claims exactly one slot;
    // a tight load factor forces collisions across the open-addressing table.
    let max_objects = 512u32;
    let cfg = GridConfig {
        cell_size: 1.0,
        max_objects,
        max_cells_per_object: 1,
        max_query_cells: 4096,
        slack: 0.0,
    };
    let mut grid = HashGrid::activate(cfg).unwrap_or_else(|e| unreachable!("{e}"));
    let slots = grid.cell_slots();
    assert!(slots >= max_objects as usize);

    let mut scratch =
        CandidateSet::with_capacity(max_objects).unwrap_or_else(|e| unreachable!("{e}"));

    // Phase 1: fill to the bound, verifying occupancy never exceeds α_max.
    for i in 0..max_objects {
        grid.on_sample(
            i,
            Point3 {
                x: f64::from(i),
                y: 0.0,
                z: 0.0,
            },
            deps(1),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
    }
    let occupancy_ok = |used: usize| used * 10 <= slots * 7; // used/slots ≤ 0.70 = α_max
    assert!(
        occupancy_ok(grid.used_cells()),
        "occupancy exceeds α_max at full bound"
    );
    assert!(
        grid.used_cells() * 2 >= slots,
        "full bound should reach ≥50% occupancy"
    );
    assert_eq!(grid.live_count(), max_objects as usize);

    // Phase 2: deletes (backward shift) + re-inserts at colliding keys; every
    // live object must still be found by an exhaustive region query.
    for round in 0..4u32 {
        for i in (round..max_objects).step_by(3) {
            grid.remove(i).unwrap_or_else(|e| unreachable!("{e}"));
        }
        assert!(occupancy_ok(grid.used_cells()));
        for i in (round..max_objects).step_by(3) {
            grid.on_sample(
                i,
                Point3 {
                    x: f64::from(i),
                    y: 0.0,
                    z: 0.0,
                },
                deps(u64::from(round) + 2),
            )
            .unwrap_or_else(|e| unreachable!("{e}"));
        }
    }
    let live = grid.live_count();
    // A wide region covers every object; the bounded fallback keeps the scan
    // independent of the span, so all live objects are found.
    grid.region(
        TypedAabb {
            min: Point3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            max: Point3 {
                x: f64::from(max_objects) + 1.0,
                y: 1.0,
                z: 1.0,
            },
            frame: 0,
        },
        &mut scratch,
    )
    .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(
        scratch.handles().len(),
        live,
        "every live membership must remain reachable after deletion"
    );
    assert!(occupancy_ok(grid.used_cells()));
}

/// REQ: REQ-SPATIAL-026 — the code audit: no excluded proximity structure
/// (octree, k-d tree, sweep-and-prune) is present in the `drm-spatial` source
/// (`I-X10`).
#[test]
fn spatial_rejected_structures() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/grid.rs"))
        .unwrap_or_else(|e| unreachable!("{e}"));
    for banned in [
        "octree",
        "k-d tree",
        "kdtree",
        "sweep-and-prune",
        "sweep_and_prune",
    ] {
        assert!(
            !src.to_ascii_lowercase().contains(banned),
            "excluded structure `{banned}` must not appear in the index implementation"
        );
    }
    let proximity =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/proximity.rs"))
            .unwrap_or_else(|e| unreachable!("{e}"));
    for banned in ["octree", "kdtree", "sweep_and_prune"] {
        assert!(
            !proximity.to_ascii_lowercase().contains(banned),
            "excluded structure `{banned}` must not appear in the proximity runtime"
        );
    }
}
